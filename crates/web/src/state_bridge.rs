//! Selector-based derived state actors for Leptos signals.
//!
//! Bridges `StateRef<T>` into Leptos `ReadSignal<U>` via `Notify`.

use std::sync::{Arc, Mutex};

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use willow_actor::state::{Notify, StateRef};
use willow_actor::{Actor, Context, Handler, Recipient};

/// A derived state actor that watches a `StateRef<T>` and updates a Leptos signal.
///
/// `cached` is wrapped in `Arc<Mutex<_>>` so the async handler body can both
/// read the previous value and write the new one across `.await` points
/// without borrowing `&mut self`. Leptos signals notify on every `set()`
/// regardless of equality, so de-duping here is what prevents spurious
/// re-renders of every subscribing view.
pub struct DerivedStateActor<T: Send + Sync + 'static, U: PartialEq + Clone + Send + Sync + 'static>
{
    source: StateRef<T>,
    selector: Arc<dyn Fn(&T) -> U + Send + Sync>,
    // state: lock-ok — cross-await dedup cache. The actor mailbox
    // serializes `Handler<Notify>::handle` invocations, so handle-vs-
    // handle contention is impossible. The actual race window is
    // `started()` seeding `cached` (its own future) vs the first
    // `Notify` arriving before that seed completes — a one-extra-`set()`
    // worst case that doesn't break correctness. Alternative shapes
    // (Leptos `Resource`, full StateActor) tracked in
    // docs/specs/2026-04-26-state-management-model-design.md § Follow-up F2.
    cached: Arc<Mutex<Option<U>>>,
    write: SendWrapper<WriteSignal<U>>,
}

impl<T: Send + Sync + 'static, U: PartialEq + Clone + Send + Sync + 'static> Actor
    for DerivedStateActor<T, U>
{
    fn started(&mut self, ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        let recipient: Recipient<Notify> = ctx.address().into();
        self.source.subscribe(recipient);
        // Seed the signal immediately with the current value so that state
        // restored from storage during ClientHandle::new() is reflected before
        // any mutation fires a Notify (e.g. after a page reload).
        let selector = self.selector.clone();
        let source = self.source.clone();
        let write = self.write.clone();
        let cached = self.cached.clone();
        async move {
            let snapshot = source.get().await;
            let result = selector(&snapshot);
            // state: lock-ok — see `DerivedStateActor::cached` doc; mailbox serializes handlers, no contention.
            *cached.lock().expect("cached mutex poisoned") = Some(result.clone());
            (*write).set(result);
        }
    }
}

impl<T: Send + Sync + 'static, U: PartialEq + Clone + Send + Sync + 'static> Handler<Notify>
    for DerivedStateActor<T, U>
{
    fn handle(
        &mut self,
        _msg: Notify,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        let selector = self.selector.clone();
        let source = self.source.clone();
        let cached = self.cached.clone();
        let write = self.write.clone();

        async move {
            let snapshot = source.get().await;
            let result = selector(&snapshot);
            // state: lock-ok — see `DerivedStateActor::cached` doc; mailbox serializes handlers, no contention.
            let mut guard = cached.lock().expect("cached mutex poisoned");
            let changed = match &*guard {
                Some(old) => old != &result,
                None => true,
            };
            if changed {
                *guard = Some(result.clone());
                drop(guard);
                (*write).set(result);
            }
        }
    }
}

// SAFETY: All four fields are Send under the `T: Send + Sync` and `U: Send + Sync`
// bounds on the impl: `StateRef<T>` and `Arc<Mutex<Option<U>>>` propagate Send from
// their parameters, `Arc<dyn Fn(&T) -> U + Send + Sync>` is Send by its trait object
// bound, and `SendWrapper<WriteSignal<U>>` is unconditionally Send with a runtime
// panic on cross-thread access. The actor framework requires `Send` for spawning
// across its mailbox, but the actor only ever runs on a single WASM thread (Leptos
// is browser-only), so `SendWrapper`'s runtime check is never tripped. Manual impl
// guards against future field additions silently breaking auto-derive — any new
// `!Send` field must reaffirm or remove this assertion. Tracked alongside the
// `cached` Mutex follow-up in
// docs/specs/2026-04-26-state-management-model-design.md § Follow-up F2.
unsafe impl<T: Send + Sync + 'static, U: PartialEq + Clone + Send + Sync + 'static> Send
    for DerivedStateActor<T, U>
{
}

/// Create a derived Leptos signal from a `StateRef<T>`.
pub fn derived_signal<
    T: Send + Sync + 'static,
    U: PartialEq + Clone + Default + Send + Sync + 'static,
>(
    source: &StateRef<T>,
    system: &willow_actor::SystemHandle,
    selector: impl Fn(&T) -> U + Send + Sync + 'static,
) -> ReadSignal<U> {
    let (read, write) = signal(U::default());

    system.spawn(DerivedStateActor {
        source: source.clone(),
        selector: Arc::new(selector),
        cached: Arc::new(Mutex::new(None)),
        write: SendWrapper::new(write),
    });

    read
}
