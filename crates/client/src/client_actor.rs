//! # Client State Actor
//!
//! Wraps access to [`SharedState`] through the actor system, providing
//! notification-based derived state. The actor holds an `Arc<RwLock<SharedState>>`
//! (same reference as `ClientHandle::shared`), ensuring a single source of truth.
//!
//! Mutations mark the actor "dirty", and after a batch of mutations drain,
//! the `idle()` hook notifies all subscribers.

use std::any::Any;
use std::sync::{Arc, RwLock};

use willow_actor::{Actor, Context, Handler, Message, Recipient};

use crate::SharedState;

/// The client state actor -- notification hub for state changes.
pub struct ClientStateActor {
    pub(crate) shared: Arc<RwLock<SharedState>>,
    pub(crate) dirty: bool,
    pub(crate) subscribers: Vec<Recipient<StateChanged>>,
}

// Safety: ClientStateActor holds an Arc<RwLock<SharedState>>. The RwLock
// provides synchronized access. On WASM (single-threaded), this is trivially safe.
unsafe impl Send for ClientStateActor {}

impl Actor for ClientStateActor {
    fn idle(
        &mut self,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        if self.dirty {
            self.dirty = false;
            for sub in &self.subscribers {
                let _ = sub.do_send(StateChanged);
            }
        }
        async {}
    }
}

/// Notification sent to subscribers when state has changed.
#[derive(Clone)]
pub struct StateChanged;
impl Message for StateChanged {
    type Result = ();
}

/// Subscribe to state change notifications.
pub struct Subscribe(pub Recipient<StateChanged>);
impl Message for Subscribe {
    type Result = ();
}

impl Handler<Subscribe> for ClientStateActor {
    fn handle(
        &mut self,
        msg: Subscribe,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        self.subscribers.push(msg.0);
        async {}
    }
}

/// Read state via a type-erased selector.
pub struct ReadState(pub Box<dyn FnOnce(&SharedState) -> Box<dyn Any + Send> + Send>);
impl Message for ReadState {
    type Result = Box<dyn Any + Send>;
}

impl Handler<ReadState> for ClientStateActor {
    fn handle(
        &mut self,
        msg: ReadState,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = Box<dyn Any + Send>> + Send {
        let shared = self.shared.read().unwrap();
        let result = (msg.0)(&shared);
        async move { result }
    }
}

/// Mutate state via a closure. Sets dirty flag for subscriber notification.
pub struct MutateState(
    pub Box<dyn FnOnce(&mut SharedState) -> Box<dyn Any + Send> + Send>,
);
impl Message for MutateState {
    type Result = Box<dyn Any + Send>;
}

impl Handler<MutateState> for ClientStateActor {
    fn handle(
        &mut self,
        msg: MutateState,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = Box<dyn Any + Send>> + Send {
        let mut shared = self.shared.write().unwrap();
        let result = (msg.0)(&mut shared);
        self.dirty = true;
        async move { result }
    }
}

/// Notify the actor that state has been mutated externally.
///
/// Call this after mutating state through the `Arc<RwLock<SharedState>>`
/// to trigger subscriber notifications on the next idle cycle.
pub struct NotifyMutation;
impl Message for NotifyMutation {
    type Result = ();
}

impl Handler<NotifyMutation> for ClientStateActor {
    fn handle(
        &mut self,
        _msg: NotifyMutation,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        self.dirty = true;
        async {}
    }
}

/// Typed read: run a closure on `&SharedState` and return `T` directly.
pub async fn read_state<T: Send + 'static>(
    addr: &willow_actor::Addr<ClientStateActor>,
    f: impl FnOnce(&SharedState) -> T + Send + 'static,
) -> T {
    let result = addr
        .ask(ReadState(Box::new(move |s| {
            Box::new(f(s)) as Box<dyn Any + Send>
        })))
        .await
        .expect("state actor is alive");
    *result.downcast::<T>().expect("type mismatch in read_state")
}

/// Typed mutate: run a closure on `&mut SharedState` and return `T` directly.
pub async fn mutate_state<T: Send + 'static>(
    addr: &willow_actor::Addr<ClientStateActor>,
    f: impl FnOnce(&mut SharedState) -> T + Send + 'static,
) -> T {
    let result = addr
        .ask(MutateState(Box::new(move |s| {
            Box::new(f(s)) as Box<dyn Any + Send>
        })))
        .await
        .expect("state actor is alive");
    *result.downcast::<T>().expect("type mismatch in mutate_state")
}
