//! # Persistence Actor
//!
//! Owns all `!Send` database resources (rusqlite connections) on a
//! single-threaded mailbox.
//!
//! Currently uses fire-and-forget write messages for all persistence.
//! The spec targets auto-persist via `Notify` subscriptions on
//! `StateRef<EventState>`, `StateRef<ServerRegistry>`, `StateRef<ProfileState>`
//! for snapshot persistence, with only `PersistEvent` (event store appends)
//! remaining as an explicit message.

use willow_actor::{Actor, Context, Handler, Message};

use crate::storage;

// ───── Actor ─────────────────────────────────────────────────────────────

/// Persistence actor — owns event log and message databases.
///
/// All database I/O goes through this actor's mailbox so that `!Send`
/// resources (rusqlite) are never shared across threads.
pub struct PersistenceActor {
    events: Vec<willow_state::Event>,
    server_id: Option<String>,
    persistence_enabled: bool,
}

// Note: PersistenceActor fields (Vec, Option, bool) are all Send.
// The unsafe impl Send that was here has been removed as it's no longer needed —
// rusqlite connections are now managed in storage functions, not held in the actor.

impl PersistenceActor {
    /// Create a new persistence actor.
    ///
    /// Pass `persistence_enabled: false` to disable all disk I/O (testing).
    pub fn new(persistence_enabled: bool) -> Self {
        Self {
            events: Vec::new(),
            server_id: None,
            persistence_enabled,
        }
    }
}

impl Actor for PersistenceActor {}

// ───── Messages ──────────────────────────────────────────────────────────

/// Open the event store for a specific server.
pub struct OpenEventStore {
    pub server_id: String,
}
impl Message for OpenEventStore {
    type Result = ();
}

impl Handler<OpenEventStore> for PersistenceActor {
    fn handle(
        &mut self,
        msg: OpenEventStore,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        self.server_id = Some(msg.server_id);
        async {}
    }
}

/// Append an event to the event log.
pub struct PersistEvent {
    pub event: willow_state::Event,
}
impl Message for PersistEvent {
    type Result = ();
}

impl Handler<PersistEvent> for PersistenceActor {
    fn handle(
        &mut self,
        msg: PersistEvent,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        self.events.push(msg.event);
        if self.persistence_enabled {
            if let Some(ref server_id) = self.server_id {
                storage::save_events(server_id, &self.events);
            }
        }
        async {}
    }
}

/// Persist the full server state snapshot to disk.
pub struct PersistServerState {
    pub server_id: String,
    pub state: willow_state::ServerState,
}
impl Message for PersistServerState {
    type Result = ();
}

impl Handler<PersistServerState> for PersistenceActor {
    fn handle(
        &mut self,
        msg: PersistServerState,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        if self.persistence_enabled {
            storage::save_server_state(&msg.server_id, &msg.state);
        }
        async {}
    }
}

/// Persist server config (name, keys) and channel keys.
pub struct PersistServerConfig {
    pub server_id: String,
    pub name: String,
    pub keys: std::collections::HashMap<String, willow_crypto::ChannelKey>,
}
impl Message for PersistServerConfig {
    type Result = ();
}

impl Handler<PersistServerConfig> for PersistenceActor {
    fn handle(
        &mut self,
        msg: PersistServerConfig,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        if self.persistence_enabled {
            let meta = storage::SavedServerMeta {
                server_id: msg.server_id,
                name: msg.name,
            };
            storage::save_server(&meta, &msg.keys);
        }
        async {}
    }
}

/// Persist the server list (all server IDs).
pub struct PersistServerList {
    pub ids: Vec<String>,
}
impl Message for PersistServerList {
    type Result = ();
}

impl Handler<PersistServerList> for PersistenceActor {
    fn handle(
        &mut self,
        msg: PersistServerList,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        if self.persistence_enabled {
            storage::save_server_list(&msg.ids);
        }
        async {}
    }
}

/// Persist per-server config by ID.
pub struct PersistServerById {
    pub server_id: String,
    pub name: String,
    pub keys: std::collections::HashMap<String, willow_crypto::ChannelKey>,
}
impl Message for PersistServerById {
    type Result = ();
}

impl Handler<PersistServerById> for PersistenceActor {
    fn handle(
        &mut self,
        msg: PersistServerById,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        if self.persistence_enabled {
            let meta = storage::SavedServerMeta {
                server_id: msg.server_id.clone(),
                name: msg.name,
            };
            storage::save_server_by_id(&msg.server_id, &meta, &msg.keys);
        }
        async {}
    }
}

/// Persist local user profile.
pub struct PersistProfile {
    pub display_name: String,
}
impl Message for PersistProfile {
    type Result = ();
}

impl Handler<PersistProfile> for PersistenceActor {
    fn handle(
        &mut self,
        msg: PersistProfile,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        if self.persistence_enabled {
            storage::save_profile(&storage::LocalProfile {
                display_name: msg.display_name,
            });
        }
        async {}
    }
}

/// Persist join links for a server.
pub struct PersistJoinLinks {
    pub server_id: String,
    pub links: Vec<crate::ops::JoinLink>,
}
impl Message for PersistJoinLinks {
    type Result = ();
}

impl Handler<PersistJoinLinks> for PersistenceActor {
    fn handle(
        &mut self,
        msg: PersistJoinLinks,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        if self.persistence_enabled {
            storage::save_join_links(&msg.server_id, &msg.links);
        }
        async {}
    }
}

// ───── Read messages (ask-based) ─────────────────────────────────────────

/// Load all events from the internal event log.
pub struct LoadAllEvents;
impl Message for LoadAllEvents {
    type Result = Vec<willow_state::Event>;
}

impl Handler<LoadAllEvents> for PersistenceActor {
    fn handle(
        &mut self,
        _msg: LoadAllEvents,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = Vec<willow_state::Event>> + Send {
        let events = self.events.clone();
        async move { events }
    }
}
