//! # Client State Actor
//!
//! Notification-based derived state for UI frameworks. The actor owns
//! `SharedState` directly and provides subscriber notifications when
//! state is mutated. Derived state actors subscribe and run selectors
//! to efficiently update UI signals.

use std::any::Any;

use willow_actor::{Actor, Context, Handler, Message, Recipient};

use crate::SharedState;

/// Client state actor — notification hub for state changes.
pub struct ClientStateActor {
    pub(crate) shared: SharedState,
    pub(crate) dirty: bool,
    pub(crate) subscribers: Vec<Recipient<StateChanged>>,
    /// Domain actor addresses for automatic sync (transitional).
    pub(crate) domain_sync: Option<DomainSync>,
}

/// Addresses of domain actors for automatic sync from legacy state.
pub(crate) struct DomainSync {
    pub event_state: willow_actor::Addr<willow_actor::StateActor<willow_state::ServerState>>,
    pub server_registry: willow_actor::Addr<willow_actor::StateActor<crate::state_actors::ServerRegistry>>,
    pub chat_meta: willow_actor::Addr<willow_actor::StateActor<crate::state_actors::ChatMeta>>,
    pub profile_state: willow_actor::Addr<willow_actor::StateActor<crate::state_actors::ProfileState>>,
    pub network_meta: willow_actor::Addr<willow_actor::StateActor<crate::state_actors::NetworkMeta>>,
}

// Safety: SharedState is !Send due to rusqlite but the actor runs on a
// single dedicated thread. On WASM, trivially safe.
unsafe impl Send for ClientStateActor {}

impl Actor for ClientStateActor {
    fn idle(&mut self, _ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        if self.dirty {
            self.dirty = false;
            for sub in &self.subscribers {
                let _ = sub.do_send(StateChanged);
            }
            // Sync domain actors with current state.
            if let Some(sync) = &self.domain_sync {
                use willow_actor::state::Set;
                let _ = sync.event_state.do_send(Set(self.shared.state.event_state.clone()));
                let _ = sync.chat_meta.do_send(Set(crate::state_actors::ChatMeta {
                    current_channel: self.shared.state.chat.current_channel.clone(),
                    peers: self.shared.state.chat.peers.clone(),
                    seen_message_ids: self.shared.state.chat.seen_message_ids.clone(),
                }));
                let _ = sync.profile_state.do_send(Set(crate::state_actors::ProfileState {
                    names: self.shared.state.profiles.names.clone(),
                }));
                let _ = sync.network_meta.do_send(Set(crate::state_actors::NetworkMeta {
                    connected: self.shared.connected,
                    typing_peers: self.shared.typing_peers.clone(),
                    last_typing_sent_ms: self.shared.last_typing_sent_ms,
                    state_verification_results: self.shared.state_verification_results.clone(),
                }));
                let mut reg = crate::state_actors::ServerRegistry::default();
                for (id, ctx) in &self.shared.state.servers {
                    reg.servers.insert(id.clone(), crate::state_actors::ServerEntry {
                        server: ctx.server.clone(),
                        name: ctx.server.name.clone(),
                        topic_map: ctx.topic_map.clone(),
                        keys: ctx.keys.clone(),
                        unread: ctx.unread.clone(),
                    });
                }
                reg.active_server = self.shared.state.active_server.clone();
                let _ = sync.server_registry.do_send(Set(reg));
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
        let r = (msg.0)(&self.shared);
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
        let r = (msg.0)(&mut self.shared);
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
