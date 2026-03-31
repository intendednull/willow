//! Lock-free client state actor — owns SharedState directly.
//! Swap this in for client_actor.rs after all methods are converted to async.

use std::any::Any;

use willow_actor::{Actor, Context, Handler, Message, Recipient};

use crate::SharedState;

pub struct ClientStateActor {
    pub(crate) shared: SharedState,
    pub(crate) dirty: bool,
    pub(crate) subscribers: Vec<Recipient<StateChanged>>,
}

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

#[derive(Clone)]
pub struct StateChanged;
impl Message for StateChanged { type Result = (); }

pub struct Subscribe(pub Recipient<StateChanged>);
impl Message for Subscribe { type Result = (); }
impl Handler<Subscribe> for ClientStateActor {
    fn handle(&mut self, msg: Subscribe, _ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        self.subscribers.push(msg.0); async {}
    }
}

#[allow(clippy::type_complexity)]
pub type StateSelector = Box<dyn FnOnce(&SharedState) -> Box<dyn Any + Send> + Send>;
#[allow(clippy::type_complexity)]
pub type StateMutator = Box<dyn FnOnce(&mut SharedState) -> Box<dyn Any + Send> + Send>;

pub struct ReadState(pub StateSelector);
impl Message for ReadState { type Result = Box<dyn Any + Send>; }
impl Handler<ReadState> for ClientStateActor {
    fn handle(&mut self, msg: ReadState, _ctx: &mut Context<Self>) -> impl std::future::Future<Output = Box<dyn Any + Send>> + Send {
        let r = (msg.0)(&self.shared); async move { r }
    }
}

pub struct MutateState(pub StateMutator);
impl Message for MutateState { type Result = Box<dyn Any + Send>; }
impl Handler<MutateState> for ClientStateActor {
    fn handle(&mut self, msg: MutateState, _ctx: &mut Context<Self>) -> impl std::future::Future<Output = Box<dyn Any + Send>> + Send {
        let r = (msg.0)(&mut self.shared); self.dirty = true; async move { r }
    }
}

pub struct NotifyMutation;
impl Message for NotifyMutation { type Result = (); }
impl Handler<NotifyMutation> for ClientStateActor {
    fn handle(&mut self, _msg: NotifyMutation, _ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        self.dirty = true; async {}
    }
}

pub async fn read_state<T: Send + 'static>(addr: &willow_actor::Addr<ClientStateActor>, f: impl FnOnce(&SharedState) -> T + Send + 'static) -> T {
    let r = addr.ask(ReadState(Box::new(move |s| Box::new(f(s)) as Box<dyn Any + Send>))).await.expect("state actor alive");
    *r.downcast::<T>().expect("type mismatch")
}

pub async fn mutate_state<T: Send + 'static>(addr: &willow_actor::Addr<ClientStateActor>, f: impl FnOnce(&mut SharedState) -> T + Send + 'static) -> T {
    let r = addr.ask(MutateState(Box::new(move |s| Box::new(f(s)) as Box<dyn Any + Send>))).await.expect("state actor alive");
    *r.downcast::<T>().expect("type mismatch")
}
