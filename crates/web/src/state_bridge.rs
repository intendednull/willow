//! Selector-based derived state actors for Leptos signals.
//!
//! Bridges `StateRef<T>` into Leptos `ReadSignal<U>` via `Notify`.

use std::sync::Arc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use willow_actor::{Actor, Context, Handler, Recipient};
use willow_actor::state::{Notify, StateRef};

/// A derived state actor that watches a `StateRef<T>` and updates a Leptos signal.
pub struct DerivedStateActor<T: Send + Sync + 'static, U: PartialEq + Clone + Send + Sync + 'static>
{
    source: StateRef<T>,
    selector: Arc<dyn Fn(&T) -> U + Send + Sync>,
    cached: Option<U>,
    write: SendWrapper<WriteSignal<U>>,
}

impl<T: Send + Sync + 'static, U: PartialEq + Clone + Send + Sync + 'static> Actor
    for DerivedStateActor<T, U>
{
    fn started(&mut self, ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        let recipient: Recipient<Notify> = ctx.address().into();
        self.source.subscribe(recipient);
        async {}
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
            let changed = match &cached {
                Some(old) => old != &result,
                None => true,
            };
            if changed {
                (*write).set(result);
            }
        }
    }
}

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
        cached: None,
        write: SendWrapper::new(write),
    });

    read
}
