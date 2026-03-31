//! # Client State Actor
//!
//! Notification-based derived state for UI frameworks. The actor wraps
//! `Arc<RwLock<SharedState>>` and provides subscriber notifications when
//! state is mutated. Derived state actors subscribe and run selectors
//! to efficiently update UI signals.

use std::any::Any;
use std::sync::{Arc, RwLock};

use willow_actor::{Actor, Context, Handler, Message, Recipient};

use crate::SharedState;

/// Client state actor — notification hub for state changes.
pub struct ClientStateActor {
    pub(crate) shared: Arc<RwLock<SharedState>>,
    pub(crate) dirty: bool,
    pub(crate) subscribers: Vec<Recipient<StateChanged>>,
}

// Safety: Arc<RwLock> provides synchronized access. SharedState is !Send
// due to rusqlite but is safe behind the lock. On WASM, trivially safe.
unsafe impl Send for ClientStateActor {}

impl Actor for ClientStateActor {
    fn idle(&mut self, _ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
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

/// Type-erased selector closure.
#[allow(clippy::type_complexity)]
pub type StateSelector = Box<dyn FnOnce(&SharedState) -> Box<dyn Any + Send> + Send>;

/// Read state via a type-erased selector.
pub struct ReadState(pub StateSelector);
impl Message for ReadState {
    type Result = Box<dyn Any + Send>;
}

impl Handler<ReadState> for ClientStateActor {
    fn handle(
        &mut self,
        msg: ReadState,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = Box<dyn Any + Send>> + Send {
        let s = self.shared.read().unwrap();
        let r = (msg.0)(&s);
        async move { r }
    }
}

/// Notify the actor that state has been mutated externally.
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

/// Typed read: run a selector on `&SharedState` and return `T`.
pub async fn read_state<T: Send + 'static>(
    addr: &willow_actor::Addr<ClientStateActor>,
    f: impl FnOnce(&SharedState) -> T + Send + 'static,
) -> T {
    let r = addr
        .ask(ReadState(Box::new(move |s| {
            Box::new(f(s)) as Box<dyn Any + Send>
        })))
        .await
        .expect("state actor alive");
    *r.downcast::<T>().expect("type mismatch")
}

/// Type-erased mutator closure.
#[allow(clippy::type_complexity)]
pub type StateMutator = Box<dyn FnOnce(&mut SharedState) -> Box<dyn Any + Send> + Send>;

/// Mutate state via a closure. Sets dirty flag for subscriber notification.
pub struct MutateState(pub StateMutator);
impl Message for MutateState {
    type Result = Box<dyn Any + Send>;
}

impl Handler<MutateState> for ClientStateActor {
    fn handle(
        &mut self,
        msg: MutateState,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = Box<dyn Any + Send>> + Send {
        let mut s = self.shared.write().unwrap();
        let r = (msg.0)(&mut s);
        drop(s);
        self.dirty = true;
        async move { r }
    }
}

/// Typed mutate: run a closure on `&mut SharedState` and return `T`.
pub async fn mutate_state<T: Send + 'static>(
    addr: &willow_actor::Addr<ClientStateActor>,
    f: impl FnOnce(&mut SharedState) -> T + Send + 'static,
) -> T {
    let r = addr
        .ask(MutateState(Box::new(move |s| {
            Box::new(f(s)) as Box<dyn Any + Send>
        })))
        .await
        .expect("state actor alive");
    *r.downcast::<T>().expect("type mismatch")
}
