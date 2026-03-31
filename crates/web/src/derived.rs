//! Selector-based derived state actors for Leptos signals.

use std::sync::Arc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use willow_actor::{Actor, Addr, Context, Handler, Recipient};
use willow_client::client_actor::{ClientStateActor, StateChanged, Subscribe};
use willow_client::SharedState;

/// A derived state actor that watches a specific slice of state.
pub struct DerivedStateActor<T: PartialEq + Clone + Send + Sync + 'static> {
    state_addr: Addr<ClientStateActor>,
    selector: Arc<dyn Fn(&SharedState) -> T + Send + Sync>,
    cached: Option<T>,
    write: SendWrapper<WriteSignal<T>>,
}

impl<T: PartialEq + Clone + Send + Sync + 'static> Actor for DerivedStateActor<T> {
    fn started(&mut self, ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        let recipient: Recipient<StateChanged> = ctx.address().into();
        let state_addr = self.state_addr.clone();
        async move {
            let _ = state_addr.do_send(Subscribe(recipient));
        }
    }
}

impl<T: PartialEq + Clone + Send + Sync + 'static> Handler<StateChanged> for DerivedStateActor<T> {
    fn handle(
        &mut self,
        _msg: StateChanged,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        let selector = self.selector.clone();
        let state_addr = self.state_addr.clone();
        let cached = self.cached.clone();
        let write = self.write.clone();

        async move {
            let result =
                willow_client::client_actor::read_state(&state_addr, move |s| selector(s)).await;
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

unsafe impl<T: PartialEq + Clone + Send + Sync + 'static> Send for DerivedStateActor<T> {}

/// Create a derived Leptos signal backed by a state actor selector.
pub fn derived_signal<T: PartialEq + Clone + Default + Send + Sync + 'static>(
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
