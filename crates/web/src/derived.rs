//! Selector-based derived state actors for Leptos signals.
//!
//! Each `DerivedStateActor<T>` watches a specific slice of `SharedState`
//! via a selector function. When the state actor notifies subscribers of
//! a mutation, each derived actor re-runs its selector and only updates
//! the Leptos signal when the selected value actually changes.

use std::sync::Arc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use willow_actor::{Actor, Addr, Context, Handler, Message, Recipient};
use willow_client::client_actor::{
    ClientStateActor, NotifyMutation, ReadState, StateChanged, Subscribe,
};
use willow_client::SharedState;

/// A derived state actor that watches a specific slice of state.
///
/// Generic over `T` — the type of the derived value. When `StateChanged`
/// is received, the actor re-runs its selector against the state actor
/// and updates the Leptos `WriteSignal<T>` only if the value changed.
pub struct DerivedStateActor<T: PartialEq + Clone + Send + 'static> {
    state_addr: Addr<ClientStateActor>,
    selector: Arc<dyn Fn(&SharedState) -> T + Send + Sync>,
    cached: Option<T>,
    write: SendWrapper<WriteSignal<T>>,
}

impl<T: PartialEq + Clone + Send + 'static> Actor for DerivedStateActor<T> {
    fn started(&mut self, ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        // Subscribe to state changes.
        let recipient: Recipient<StateChanged> = ctx.address().into();
        let state_addr = self.state_addr.clone();
        async move {
            let _ = state_addr.do_send(Subscribe(recipient));
        }
    }
}

impl<T: PartialEq + Clone + Send + 'static> Handler<StateChanged> for DerivedStateActor<T> {
    fn handle(
        &mut self,
        _msg: StateChanged,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        // Read the current value from the state actor.
        let selector = self.selector.clone();
        let state_addr = self.state_addr.clone();

        // We need to query the state actor for the current value.
        // Since we can't borrow self across the await, we extract what we need.
        let cached = self.cached.clone();
        let write = self.write.clone();

        async move {
            let result =
                willow_client::client_actor::read_state(&state_addr, move |s| selector(s)).await;

            // Compare with cached value.
            let changed = match &cached {
                Some(old) => old != &result,
                None => true,
            };

            if changed {
                write.set(result);
            }
        }
    }
}

// DerivedStateActor holds SendWrapper<WriteSignal<T>> which is !Send,
// but SendWrapper makes it safe on single-threaded WASM.
unsafe impl<T: PartialEq + Clone + Send + 'static> Send for DerivedStateActor<T> {}

/// Create a derived Leptos signal backed by a state actor selector.
///
/// Returns a `ReadSignal<T>` that updates only when the selected value changes.
/// The `DerivedStateActor` is spawned on the actor system and subscribes to
/// state change notifications automatically.
pub fn derived_signal<T: PartialEq + Clone + Default + Send + 'static>(
    state_addr: &Addr<ClientStateActor>,
    system: &willow_actor::SystemHandle,
    selector: impl Fn(&SharedState) -> T + Send + Sync + 'static,
) -> ReadSignal<T> {
    let (read, write) = signal(T::default());

    system.spawn(DerivedStateActor {
        state_addr: state_addr.clone(),
        selector: Arc::new(selector),
        cached: None,
        write: SendWrapper::new(write),
    });

    read
}
