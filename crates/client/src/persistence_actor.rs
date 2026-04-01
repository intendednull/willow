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

use crate::state::PersistentEventStore;
use crate::storage;

// ───── Actor ─────────────────────────────────────────────────────────────

/// Persistence actor — owns event stores and message databases.
///
/// All database I/O goes through this actor's mailbox so that `!Send`
/// resources (rusqlite) are never shared across threads.
pub struct PersistenceActor {
    event_store: PersistentEventStore,
    server_id: Option<String>,
    persistence_enabled: bool,
}

// Safety: PersistenceActor owns !Send resources (rusqlite::Connection inside
// PersistentEventStore) but the actor mailbox guarantees single-threaded
// execution — messages are processed sequentially on one thread.
unsafe impl Send for PersistenceActor {}

impl PersistenceActor {
    /// Create a new persistence actor.
    ///
    /// Pass `persistence_enabled: false` to disable all disk I/O (testing).
    pub fn new(persistence_enabled: bool) -> Self {
        Self {
            event_store: PersistentEventStore::default(),
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
        if self.persistence_enabled {
            if let Some(store) = storage::open_event_store(&msg.server_id) {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.event_store = PersistentEventStore::Sqlite(store);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    self.event_store = PersistentEventStore::LocalStorage(store);
                }
            }
        }
        self.server_id = Some(msg.server_id);
        async {}
    }
}

/// Append an event to the event store and update the latest hash.
pub struct PersistEvent {
    pub event: willow_state::Event,
    pub new_hash: willow_state::StateHash,
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
        use willow_state::EventStore as _;
        self.event_store.append(msg.event);
        self.event_store.set_latest_hash(msg.new_hash);
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

/// Persist server config (name, channels) and channel keys.
pub struct PersistServerConfig {
    pub server: willow_channel::Server,
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
            storage::save_server(&msg.server, &msg.keys);
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
    pub server: willow_channel::Server,
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
            storage::save_server_by_id(&msg.server_id, &msg.server, &msg.keys);
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

/// Load all events from the event store.
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
        use willow_state::EventStore as _;
        let events = self.event_store.all_events();
        async move { events }
    }
}

/// Get the latest state hash from the event store.
pub struct GetLatestHash;
impl Message for GetLatestHash {
    type Result = willow_state::StateHash;
}

impl Handler<GetLatestHash> for PersistenceActor {
    fn handle(
        &mut self,
        _msg: GetLatestHash,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = willow_state::StateHash> + Send {
        use willow_state::EventStore as _;
        let hash = self.event_store.latest_hash();
        async move { hash }
    }
}

/// Load events since a given state hash.
pub struct LoadEventsSince {
    pub hash: willow_state::StateHash,
}
impl Message for LoadEventsSince {
    type Result = Vec<willow_state::Event>;
}

impl Handler<LoadEventsSince> for PersistenceActor {
    fn handle(
        &mut self,
        msg: LoadEventsSince,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = Vec<willow_state::Event>> + Send {
        use willow_state::EventStore as _;
        let events = self.event_store.events_since(&msg.hash);
        async move { events }
    }
}

/// Check if an event exists in the store.
pub struct ContainsEvent {
    pub event_id: String,
}
impl Message for ContainsEvent {
    type Result = bool;
}

impl Handler<ContainsEvent> for PersistenceActor {
    fn handle(
        &mut self,
        msg: ContainsEvent,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = bool> + Send {
        use willow_state::EventStore as _;
        let result = self.event_store.contains(&msg.event_id);
        async move { result }
    }
}
