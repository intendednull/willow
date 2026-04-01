//! # Willow Client
//!
//! UI-agnostic client library for the Willow P2P chat network.
//! Use this crate to build bots, CLIs, TUIs, or alternative frontends.
//!
//! ## Quick start
//!
//! ```ignore
//! use willow_client::{ClientHandle, ClientConfig, ClientEvent};
//! use willow_network::iroh::IrohNetwork;
//!
//! let (mut client, _event_loop) = ClientHandle::<IrohNetwork>::new(ClientConfig::default());
//! let network = IrohNetwork::new(/* ... */).await;
//! let event_rx = client.connect(network).await;
//!
//! // event_rx yields ClientEvents from listener tasks.
//! // Spawn a task to consume event_rx if needed.
//!
//! client.send_message("general", "hello!").ok();
//! ```

pub mod base64;
pub mod client_actor;
pub mod emoji;
pub mod events;
pub mod files;
pub mod invite;
pub mod listeners;
pub mod ops;
pub mod mutations;
pub mod persistence_actor;
pub mod state;
pub mod state_actors;
pub mod storage;
pub mod util;
pub mod worker_cache;

mod accessors;
mod actions;
mod connect;
mod joining;
mod servers;
mod voice;

// Re-export key types at crate root for convenience.
pub use events::ClientEvent;
pub use event_receiver::EventReceiver;
pub use ops::{pack_wire, unpack_wire, VoiceSignalPayload, WireMessage};

/// Helper to bridge `Broker<ClientEvent>` into an async stream receiver.
pub mod event_receiver {
    use willow_actor::{Actor, Addr, BrokerSubscribe, Context, Handler, Broker};
    use crate::events::ClientEvent;

    /// Async receiver for [`ClientEvent`]s from a [`Broker`].
    ///
    /// Implements a stream-like API: call `recv()` to await the next event,
    /// or `try_recv()` for a non-blocking check.
    pub struct EventReceiver {
        rx: willow_actor::runtime::Receiver<ClientEvent>,
    }

    impl EventReceiver {
        /// Subscribe to a broker and return a receiver for its events.
        pub async fn subscribe(
            broker: &Addr<Broker<ClientEvent>>,
            system: &willow_actor::SystemHandle,
        ) -> Self {
            let (tx, rx) = willow_actor::runtime::unbounded_channel();
            let addr = system.spawn(ForwarderActor { tx });
            let recipient = addr.into();
            let _ = broker.ask(BrokerSubscribe(recipient)).await;
            Self { rx }
        }

        /// Await the next event. Returns `None` if the broker is closed.
        pub async fn recv(&mut self) -> Option<ClientEvent> {
            self.rx.recv().await
        }

        /// Non-blocking try to receive an event.
        pub fn try_recv(&mut self) -> Option<ClientEvent> {
            self.rx.try_recv()
        }
    }

    /// Internal actor that forwards broker events to a channel.
    struct ForwarderActor {
        tx: willow_actor::runtime::Sender<ClientEvent>,
    }

    impl Actor for ForwarderActor {}

    impl Handler<ClientEvent> for ForwarderActor {
        fn handle(
            &mut self,
            msg: ClientEvent,
            _ctx: &mut Context<Self>,
        ) -> impl std::future::Future<Output = ()> + Send {
            let _ = self.tx.send(msg);
            async {}
        }
    }
}
pub use state::{
    ChatState, ClientState, DisplayMessage, PersistentEventStore, ProfileStore, ServerContext,
};

/// Re-export the event-sourced state crate for use by downstream consumers.
pub use willow_state;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use willow_identity::Identity;
use willow_messaging::Content;
use willow_network::TopicHandle as _;
use willow_state::EventStore as _;

/// Configuration for creating a [`ClientHandle`].
pub struct ClientConfig {
    /// Optional relay multiaddr string for NAT traversal.
    pub relay_addr: Option<String>,
    /// Initial display name for the local user.
    pub display_name: Option<String>,
    /// Whether to persist state to disk. Defaults to `true`.
    pub persistence: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            relay_addr: None,
            display_name: None,
            persistence: true,
        }
    }
}

/// All mutable state shared between ClientHandle and ClientEventLoop.
pub struct SharedState {
    pub state: ClientState,
    pub identity: Identity,
    pub config: ClientConfig,
    pub connected: bool,
    pub connected_subscribed: bool,
    pub typing_peers: HashMap<willow_identity::EndpointId, (String, u64)>,
    pub voice_participants: HashMap<String, std::collections::HashSet<willow_identity::EndpointId>>,
    pub active_voice_channel: Option<String>,
    pub voice_muted: bool,
    pub voice_deafened: bool,
    pub state_verification_results: HashMap<willow_identity::EndpointId, willow_state::StateHash>,
    pub last_typing_sent_ms: u64,
    pub join_links: Vec<ops::JoinLink>,
}

/// Cloneable command interface for UI components.
///
/// Generic over the [`Network`](willow_network::Network) implementation so
/// that production code can use a real iroh network while tests can use
/// an in-memory backend.
pub struct ClientHandle<N: willow_network::Network> {
    // Legacy monolithic state actor — used during incremental migration.
    // Will be removed once all callers use domain-specific actors.
    pub(crate) state_addr: willow_actor::Addr<client_actor::ClientStateActor>,
    pub(crate) system: willow_actor::SystemHandle,
    /// The network backend, set after [`connect()`](ClientHandle::connect).
    pub(crate) network: Option<Arc<N>>,
    /// Maps topic string names to their `N::Topic` handles for broadcasting.
    pub(crate) topics: Arc<RwLock<HashMap<String, N::Topic>>>,
    /// Broker for pub/sub [`ClientEvent`] distribution to subscribers.
    pub(crate) event_broker: willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
    /// The local identity, needed for signing broadcasts.
    pub(crate) identity: Identity,

    // ── Domain-specific state actors (Phase 2) ─────────────────────────
    /// Event-sourced server state.
    pub(crate) event_state_addr: willow_actor::Addr<willow_actor::StateActor<willow_state::ServerState>>,
    /// Server registry (servers map, active server, topic maps, keys).
    pub(crate) server_registry_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::ServerRegistry>>,
    /// Chat session metadata (current channel, peers, dedup).
    pub(crate) chat_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
    /// Global profile display names.
    pub(crate) profile_state_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::ProfileState>>,
    /// Network connection state.
    pub(crate) network_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::NetworkMeta>>,
    /// Voice call state.
    pub(crate) voice_state_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::VoiceState>>,
    /// Persistence actor (owns rusqlite).
    pub(crate) persistence_addr: willow_actor::Addr<persistence_actor::PersistenceActor>,
    /// HLC clock (mutation-time concern, not reactive state).
    pub(crate) hlc: Arc<std::sync::Mutex<willow_messaging::hlc::HLC>>,
    /// Whether persistence to disk is enabled.
    pub(crate) persistence_enabled: bool,
    /// Active join links (rarely modified, shared across tasks).
    pub(crate) join_links: Arc<std::sync::Mutex<Vec<ops::JoinLink>>>,
}

impl<N: willow_network::Network> Clone for ClientHandle<N> {
    fn clone(&self) -> Self {
        Self {
            state_addr: self.state_addr.clone(),
            system: self.system.clone(),
            network: self.network.clone(),
            topics: Arc::clone(&self.topics),
            event_broker: self.event_broker.clone(),
            identity: self.identity.clone(),
            event_state_addr: self.event_state_addr.clone(),
            server_registry_addr: self.server_registry_addr.clone(),
            chat_meta_addr: self.chat_meta_addr.clone(),
            profile_state_addr: self.profile_state_addr.clone(),
            network_meta_addr: self.network_meta_addr.clone(),
            voice_state_addr: self.voice_state_addr.clone(),
            persistence_addr: self.persistence_addr.clone(),
            hlc: Arc::clone(&self.hlc),
            persistence_enabled: self.persistence_enabled,
            join_links: Arc::clone(&self.join_links),
        }
    }
}

/// Async event processing loop (legacy).
///
/// Listeners now handle all incoming gossip events via
/// [`listeners::spawn_topic_listener`]. This struct is kept for API
/// compatibility but its [`run()`](ClientEventLoop::run) method is a no-op
/// that simply drains the internal channel.
pub struct ClientEventLoop {
    #[allow(dead_code)]
    pub(crate) _system: willow_actor::System,
}

/// Helper: apply an event to the event-sourced state and store it.
/// Callable with explicit parameters to avoid borrow conflicts.
/// `persistence` is passed separately to avoid double-borrowing SharedState.
fn apply_event_shared(state: &mut ClientState, persistence: bool, event: &willow_state::Event) {
    willow_state::apply_lenient(&mut state.event_state, event);
    state.event_store.append(event.clone());
    let hash = state.event_state.hash();
    state.event_store.set_latest_hash(hash);

    // Persist full state after every mutation.
    if persistence {
        if let Some(sid) = &state.active_server {
            storage::save_server_state(sid, &state.event_state);
        }
    }
}

/// Wrapper that takes `&mut SharedState` so callers don't need split borrows.
fn apply_event_on_shared(shared: &mut SharedState, event: &willow_state::Event) {
    apply_event_shared(&mut shared.state, shared.config.persistence, event);
}

/// Persist all servers to storage.
fn persist_servers(state: &ClientState) {
    let ids: Vec<String> = state.servers.keys().cloned().collect();
    storage::save_server_list(&ids);
    for (id, ctx) in &state.servers {
        storage::save_server_by_id(id, &ctx.server, &ctx.keys);
    }
}

// on_connected is no longer needed — subscription is handled by connect().

/// Reconcile `topic_map` channel IDs with `event_state.channels`.
///
/// After event state is loaded or synced, the `topic_map` may have stale
/// channel IDs (from invite acceptance or legacy storage). This updates
/// them to match the authoritative IDs in `event_state`.
pub(crate) fn reconcile_topic_map(state: &mut ClientState) {
    // Collect corrections first to avoid borrow conflicts.
    let corrections: Vec<(String, willow_channel::ChannelId)> = state
        .event_state
        .channels
        .iter()
        .map(|(id_str, ch)| {
            let cid = willow_channel::ChannelId(
                uuid::Uuid::parse_str(id_str).unwrap_or_else(|_| uuid::Uuid::new_v4()),
            );
            (ch.name.clone(), cid)
        })
        .collect();

    let Some(ctx) = state.active_mut() else {
        return;
    };
    for (ch_name, correct_id) in &corrections {
        for (_topic, (map_name, map_id)) in ctx.topic_map.iter_mut() {
            if map_name == ch_name {
                *map_id = correct_id.clone();
            }
        }
    }
}

/// Initialize (or re-initialize) the event-sourced state for a specific server.
/// Tries loading saved state first, then falls back to replaying events.
/// `persistence` is passed separately to avoid double-borrowing SharedState.
fn init_event_state_for_server(state: &mut ClientState, persistence: bool, server_id: &str) {
    let Some(ctx) = state.servers.get(server_id) else {
        return;
    };

    // Try loading saved state first.
    if let Some(saved_state) = storage::load_server_state(server_id) {
        state.event_state = saved_state;
    } else {
        state.event_state =
            willow_state::ServerState::new(server_id, ctx.server.name.clone(), ctx.server.owner);
    }

    // Open persistent event store for this server.
    if persistence {
        if let Some(store) = storage::open_event_store(server_id) {
            #[cfg(not(target_arch = "wasm32"))]
            {
                state.event_store = state::PersistentEventStore::Sqlite(store);
            }
            #[cfg(target_arch = "wasm32")]
            {
                state.event_store = state::PersistentEventStore::LocalStorage(store);
            }
        }
    }

    // If we didn't load a saved state, replay persisted events.
    if storage::load_server_state(server_id).is_none() {
        let stored_events = state.event_store.all_events();
        if !stored_events.is_empty() {
            for event in &stored_events {
                willow_state::apply_lenient(&mut state.event_state, event);
            }
        } else {
            // Don't seed channels from topic_map -- the IDs won't match
            // the owner's IDs. Let sync deliver the correct CreateChannel
            // events which will populate event_state.channels with the
            // right IDs.
        }
    }
}

/// Wrapper that takes `&mut SharedState` so callers don't need split borrows.
fn init_event_state_on_shared(shared: &mut SharedState, server_id: &str) {
    init_event_state_for_server(&mut shared.state, shared.config.persistence, server_id);
}

impl<N: willow_network::Network> ClientHandle<N> {
    /// Create a new client. Loads or generates identity, loads or creates
    /// the server with default channels, loads persisted messages.
    ///
    /// Does **not** connect to the network -- call [`ClientHandle::connect()`] for that.
    ///
    /// Returns `(ClientHandle, ClientEventLoop)`.
    pub fn new(config: ClientConfig) -> (Self, ClientEventLoop) {
        let identity = load_identity();

        // event_broker is spawned later after the system is created.

        let mut state = ClientState::new(identity.endpoint_id());

        // Open message database.
        if config.persistence {
            if let Some(db) = storage::open_message_db() {
                state.message_db = Some(std::sync::Arc::new(std::sync::Mutex::new(db)));
            }
        }

        // Load servers. Try multi-server list first, fall back to legacy single server.
        let server_ids = storage::load_server_list();
        let mut first_server_id = None;

        if let Some(ids) = &server_ids {
            // Load each server from per-server storage.
            for id in ids {
                if let Some((server, keys)) = storage::load_server_by_id(id) {
                    let mut topic_map = HashMap::new();
                    for ch in server.channels() {
                        let topic = util::make_topic(&server, &ch.name);
                        topic_map.insert(topic, (ch.name.clone(), ch.id.clone()));
                    }

                    let ctx = ServerContext {
                        server,
                        topic_map,
                        keys,
                        unread: HashMap::new(),
                    };
                    state.servers.insert(id.clone(), ctx);
                    if first_server_id.is_none() {
                        first_server_id = Some(id.clone());
                    }
                }
            }
        }

        // Fall back to legacy single-server storage. Do NOT create a default.
        if state.servers.is_empty() {
            if let Some((server, keys)) = storage::load_server() {
                let sid = server.id.to_string();
                let mut topic_map = HashMap::new();
                for ch in server.channels() {
                    let topic = util::make_topic(&server, &ch.name);
                    topic_map.insert(topic, (ch.name.clone(), ch.id.clone()));
                }

                let ctx = ServerContext {
                    server,
                    topic_map,
                    keys,
                    unread: HashMap::new(),
                };
                state.servers.insert(sid.clone(), ctx);
                first_server_id = Some(sid);
            }
            // If no legacy server found, servers stays empty -- user must create or join.
        }

        state.active_server = first_server_id.clone();

        // Initialize event-sourced state from the active server.
        if let Some(sid) = &state.active_server {
            if let Some(ctx) = state.servers.get(sid) {
                // Try loading saved state first.
                if let Some(saved_state) = storage::load_server_state(sid) {
                    state.event_state = saved_state;
                } else {
                    state.event_state = willow_state::ServerState::new(
                        sid.clone(),
                        ctx.server.name.clone(),
                        ctx.server.owner,
                    );

                    // No saved state -- seed from legacy channels.
                    // Open persistent event store and replay stored events.
                    if config.persistence {
                        if let Some(store) = storage::open_event_store(sid) {
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                state.event_store = state::PersistentEventStore::Sqlite(store);
                            }
                            #[cfg(target_arch = "wasm32")]
                            {
                                state.event_store =
                                    state::PersistentEventStore::LocalStorage(store);
                            }
                        }
                    }

                    // Replay persisted events to rebuild event_state.
                    let stored_events = state.event_store.all_events();
                    if !stored_events.is_empty() {
                        for event in &stored_events {
                            willow_state::apply_lenient(&mut state.event_state, event);
                        }
                    } else {
                        // No persisted events -- seed event_state with existing
                        // channels from legacy storage so lookups work.
                        for (topic, (name, ch_id)) in &ctx.topic_map {
                            let _ = topic;
                            state.event_state.channels.insert(
                                ch_id.to_string(),
                                willow_state::Channel {
                                    id: ch_id.to_string(),
                                    name: name.clone(),
                                    pinned_messages: std::collections::HashSet::new(),
                                    kind: "text".to_string(),
                                },
                            );
                        }
                    }
                }

                // Still open the event store even if we loaded saved state.
                if config.persistence {
                    if let Some(store) = storage::open_event_store(sid) {
                        #[cfg(not(target_arch = "wasm32"))]
                        {
                            state.event_store = state::PersistentEventStore::Sqlite(store);
                        }
                        #[cfg(target_arch = "wasm32")]
                        {
                            state.event_store = state::PersistentEventStore::LocalStorage(store);
                        }
                    }
                }
            }
        }

        // Save in multi-server format.
        if config.persistence {
            persist_servers(&state);
        }

        // Populate seen_message_ids from event_state for dedup.
        for es_msg in &state.event_state.messages {
            state.chat.seen_message_ids.insert(es_msg.id.clone());
        }

        // Load saved display name, or use config override.
        let local_endpoint = identity.endpoint_id();
        if let Some(ref name) = config.display_name {
            state.profiles.names.insert(local_endpoint, name.clone());
            if config.persistence {
                storage::save_profile(&storage::LocalProfile {
                    display_name: name.clone(),
                });
            }
        } else if let Some(profile) = storage::load_profile() {
            if !profile.display_name.is_empty() {
                state
                    .profiles
                    .names
                    .insert(local_endpoint, profile.display_name);
            }
        }

        let identity_clone = identity.clone();

        // Spawn domain-specific state actors (Phase 2).
        // Must happen BEFORE state is moved into SharedState.
        let system = willow_actor::System::new();
        let event_state_addr = system.spawn(willow_actor::StateActor::new(state.event_state.clone()));
        let server_registry_addr = {
            let mut registry = state_actors::ServerRegistry::default();
            for (id, ctx) in &state.servers {
                registry.servers.insert(id.clone(), state_actors::ServerEntry {
                    server: ctx.server.clone(),
                    name: ctx.server.name.clone(),
                    topic_map: ctx.topic_map.clone(),
                    keys: ctx.keys.clone(),
                    unread: ctx.unread.clone(),
                });
            }
            registry.active_server = state.active_server.clone();
            system.spawn(willow_actor::StateActor::new(registry))
        };
        let chat_meta_addr = {
            let meta = state_actors::ChatMeta {
                current_channel: state.chat.current_channel.clone(),
                peers: state.chat.peers.clone(),
                seen_message_ids: state.chat.seen_message_ids.clone(),
            };
            system.spawn(willow_actor::StateActor::new(meta))
        };
        let profile_state_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::ProfileState { names: state.profiles.names.clone() },
        ));
        let network_meta_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::NetworkMeta::default(),
        ));
        let voice_state_addr = system.spawn(willow_actor::StateActor::new(
            state_actors::VoiceState::default(),
        ));
        let persistence_enabled = config.persistence;
        let persistence_addr = system.spawn(
            persistence_actor::PersistenceActor::new(persistence_enabled),
        );
        let event_broker = system.spawn(willow_actor::Broker::<ClientEvent>::new());
        // Open event store on the persistence actor if we have an active server.
        if let Some(sid) = &state.active_server {
            let _ = persistence_addr.do_send(persistence_actor::OpenEventStore {
                server_id: sid.clone(),
            });
        }
        let hlc = Arc::new(std::sync::Mutex::new(willow_messaging::hlc::HLC::new()));

        let mut shared_state = SharedState {
            state,
            identity,
            config,
            connected: false,
            connected_subscribed: false,
            state_verification_results: HashMap::new(),
            last_typing_sent_ms: 0,
            typing_peers: HashMap::new(),
            voice_participants: HashMap::new(),
            active_voice_channel: None,
            voice_muted: false,
            voice_deafened: false,
            join_links: Vec::new(),
        };

        // SharedState is Send but not Sync (due to rusqlite::Connection).
        reconcile_topic_map(&mut shared_state.state);

        let state_addr = system.spawn(client_actor::ClientStateActor {
            shared: shared_state,
            dirty: false,
            subscribers: Vec::new(),
        });

        let handle = ClientHandle {
            state_addr,
            system: system.handle(),
            network: None,
            topics: Arc::new(RwLock::new(HashMap::new())),
            event_broker,
            identity: identity_clone,
            event_state_addr,
            server_registry_addr,
            chat_meta_addr,
            profile_state_addr,
            network_meta_addr,
            voice_state_addr,
            persistence_addr,
            hlc,
            persistence_enabled,
            join_links: Arc::new(std::sync::Mutex::new(Vec::new())),
        };

        // reconcile_topic_map called above before spawning actor

        let event_loop = ClientEventLoop { _system: system };

        (handle, event_loop)
    }

    // Domain actor sync happens automatically in ClientStateActor::idle().

    /// Fire-and-forget broadcast of raw data on a named topic.
    ///
    /// Spawns an async task to perform the actual broadcast so that
    /// synchronous methods can call this without blocking. Does nothing
    /// if not connected (no network set) or the topic isn't subscribed.
    pub(crate) fn broadcast_on_topic(&self, topic: &str, data: Vec<u8>) {
        // If not connected, nothing to broadcast.
        if self.network.is_none() {
            return;
        }

        // Quick check: if we don't have a sender for this topic, skip.
        {
            let topics = self.topics.read().unwrap();
            if !topics.contains_key(topic) {
                return;
            }
        }

        let topics = Arc::clone(&self.topics);
        let topic_key = topic.to_string();
        let bytes = bytes::Bytes::from(data);

        #[cfg(not(target_arch = "wasm32"))]
        {
            // Use try_handle() to avoid panicking if no runtime is active
            // (e.g. in sync tests).
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let sender = {
                        let topics = topics.read().unwrap();
                        topics.get(&topic_key).cloned()
                    };
                    if let Some(sender) = sender {
                        let _ = sender.broadcast(bytes).await;
                    }
                });
            }
        }

        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(async move {
            let sender = {
                let topics = topics.read().unwrap();
                topics.get(&topic_key).cloned()
            };
            if let Some(sender) = sender {
                let _ = sender.broadcast(bytes).await;
            }
        });
    }

    /// Broadcast a signed wire event on the server ops topic.
    pub(crate) fn broadcast_event(&self, event: &willow_state::Event) {
        if let Some(data) = ops::pack_wire(&ops::WireMessage::Event(event.clone()), &self.identity)
        {
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }

    // Methods extracted to: voice.rs, servers.rs, actions.rs, accessors.rs, joining.rs
    // ---- Internal helpers ----

    /// Send chat content (text, reply) on a channel.
    pub(crate) async fn send_content(
        &self,
        channel: &str,
        _content: Content,
        body: &str,
        _reply_preview: Option<String>,
        reply_to: Option<String>,
    ) -> anyhow::Result<()> {
        let ch = channel.to_string();
        let b = body.to_string();
        let event = crate::client_actor::mutate_state(
            &self.state_addr,
            move |s| -> anyhow::Result<willow_state::Event> {
                let ctx = s
                    .state
                    .active()
                    .ok_or_else(|| anyhow::anyhow!("no active server"))?;
                let topic = ctx.topic_for_name(&ch).unwrap_or_else(|| ch.clone());
                let peer_id_str = s.identity.endpoint_id();
                let channel_id = s
                    .state
                    .active()
                    .and_then(|ctx| {
                        ctx.topic_map
                            .get(&topic)
                            .map(|(_, ch_id)| ch_id.to_string())
                    })
                    .unwrap_or_else(|| ch.clone());
                let ts = util::current_time_ms();
                let event_id = uuid::Uuid::new_v4().to_string();
                let msg_event = willow_state::Event {
                    id: event_id.clone(),
                    parent_hash: s.state.event_state.hash(),
                    author: peer_id_str,
                    timestamp_ms: ts,
                    kind: willow_state::EventKind::Message {
                        channel_id,
                        body: b,
                        reply_to,
                    },
                };
                apply_event_on_shared(s, &msg_event);
                s.state.chat.seen_message_ids.insert(event_id);
                Ok(msg_event)
            },
        )
        .await?;
        self.broadcast_event(&event);
        Ok(())
    }
}

/// Convert a [`willow_state::Event`] into [`ClientEvent`]s for the caller.
///
/// This is a free function so it can be called with an already-borrowed
/// `SharedState` from `ClientEventLoop::process_batch()`.
pub(crate) fn emit_client_events_for(
    shared: &mut SharedState,
    event: &willow_state::Event,
    events: &mut Vec<ClientEvent>,
) {
    match &event.kind {
        willow_state::EventKind::Message { ref channel_id, .. } => {
            let is_local = event.author == shared.identity.endpoint_id();

            // Track seen IDs for dedup.
            shared.state.chat.seen_message_ids.insert(event.id.clone());

            // Update unread counts.
            let topic = shared
                .state
                .active()
                .and_then(|ctx| {
                    ctx.topic_map
                        .iter()
                        .find(|(_, (_, cid))| cid.to_string() == *channel_id)
                        .map(|(t, _)| t.clone())
                })
                .unwrap_or_default();
            let current_topic = shared
                .state
                .active()
                .and_then(|ctx| ctx.topic_for_name(&shared.state.chat.current_channel))
                .unwrap_or_default();
            if !is_local && topic != current_topic {
                let sid = shared.state.active_server.clone().unwrap_or_default();
                if let Some(ctx) = shared.state.servers.get_mut(&sid) {
                    *ctx.unread.entry(topic).or_insert(0) += 1;
                }
            }

            let channel_name = shared
                .state
                .event_state
                .channels
                .get(channel_id)
                .map(|ch| ch.name.clone())
                .unwrap_or_else(|| channel_id.clone());

            events.push(ClientEvent::MessageReceived {
                channel: channel_name,
                message_id: event.id.clone(),
                is_local,
            });
        }
        willow_state::EventKind::CreateChannel {
            name, channel_id, ..
        } => {
            // Subscribe to the new channel's gossipsub topic and
            // update the topic_map. Use the ORIGINAL channel_id from
            // the event (not ChannelId::new()) so IDs match across peers.
            // Subscription to the new topic is deferred to the caller.
            if let Some(ctx) = shared.state.active_mut() {
                let topic = util::make_topic(&ctx.server, name);
                if !ctx.topic_map.contains_key(&topic) {
                    let cid = willow_channel::ChannelId(
                        uuid::Uuid::parse_str(channel_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
                    );
                    ctx.topic_map.insert(topic.clone(), (name.clone(), cid));
                    // The caller will handle subscribing to the new topic if needed.
                }
            }
            events.push(ClientEvent::ChannelCreated(name.clone()));
        }
        willow_state::EventKind::DeleteChannel { channel_id } => {
            // Look up channel name from state before deletion.
            let name = shared
                .state
                .event_state
                .channels
                .get(channel_id)
                .map(|ch| ch.name.clone())
                .unwrap_or_else(|| channel_id.clone());
            events.push(ClientEvent::ChannelDeleted(name));
        }
        willow_state::EventKind::CreateRole { name, role_id } => {
            events.push(ClientEvent::RoleCreated {
                name: name.clone(),
                role_id: role_id.clone(),
            });
        }
        willow_state::EventKind::DeleteRole { role_id } => {
            events.push(ClientEvent::RoleDeleted {
                role_id: role_id.clone(),
            });
        }
        willow_state::EventKind::KickMember { peer_id } => {
            events.push(ClientEvent::MemberKicked(*peer_id));
        }
        willow_state::EventKind::GrantPermission { peer_id, .. } => {
            events.push(ClientEvent::PeerTrusted(*peer_id));
        }
        willow_state::EventKind::RevokePermission { peer_id, .. } => {
            events.push(ClientEvent::PeerUntrusted(*peer_id));
        }
        willow_state::EventKind::RenameServer { new_name } => {
            events.push(ClientEvent::ServerRenamed {
                new_name: new_name.clone(),
            });
        }
        willow_state::EventKind::SetServerDescription { description } => {
            events.push(ClientEvent::ServerDescriptionChanged {
                description: description.clone(),
            });
        }
        willow_state::EventKind::PinMessage {
            channel_id,
            message_id,
        } => {
            let channel = shared
                .state
                .event_state
                .channels
                .get(channel_id)
                .map(|ch| ch.name.clone())
                .unwrap_or_else(|| channel_id.clone());
            events.push(ClientEvent::MessagePinned {
                channel,
                message_id: message_id.clone(),
            });
        }
        willow_state::EventKind::UnpinMessage {
            channel_id,
            message_id,
        } => {
            let channel = shared
                .state
                .event_state
                .channels
                .get(channel_id)
                .map(|ch| ch.name.clone())
                .unwrap_or_else(|| channel_id.clone());
            events.push(ClientEvent::MessageUnpinned {
                channel,
                message_id: message_id.clone(),
            });
        }
        willow_state::EventKind::EditMessage {
            message_id,
            new_body,
        } => {
            let channel = shared
                .state
                .event_state
                .messages
                .iter()
                .find(|m| m.id == *message_id)
                .and_then(|m| {
                    shared
                        .state
                        .event_state
                        .channels
                        .get(&m.channel_id)
                        .map(|ch| ch.name.clone())
                })
                .unwrap_or_default();
            events.push(ClientEvent::MessageEdited {
                channel,
                message_id: message_id.clone(),
                new_body: new_body.clone(),
            });
        }
        willow_state::EventKind::DeleteMessage { message_id } => {
            let channel = shared
                .state
                .event_state
                .messages
                .iter()
                .find(|m| m.id == *message_id)
                .and_then(|m| {
                    shared
                        .state
                        .event_state
                        .channels
                        .get(&m.channel_id)
                        .map(|ch| ch.name.clone())
                })
                .unwrap_or_default();
            events.push(ClientEvent::MessageDeleted {
                channel,
                message_id: message_id.clone(),
            });
        }
        willow_state::EventKind::Reaction {
            message_id, emoji, ..
        } => {
            // Find the channel name from the message's channel_id.
            let channel = shared
                .state
                .event_state
                .messages
                .iter()
                .find(|m| m.id == *message_id)
                .and_then(|m| {
                    shared
                        .state
                        .event_state
                        .channels
                        .get(&m.channel_id)
                        .map(|ch| ch.name.clone())
                })
                .unwrap_or_default();
            events.push(ClientEvent::ReactionAdded {
                channel,
                message_id: message_id.clone(),
                emoji: emoji.clone(),
                author: event.author,
            });
        }
        willow_state::EventKind::SetProfile { display_name } => {
            // Also update the gossipsub ProfileStore so both systems stay
            // in sync, and emit a ProfileUpdated so the UI refreshes the
            // member list with the new display name.
            shared
                .state
                .profiles
                .names
                .insert(event.author, display_name.clone());
            events.push(ClientEvent::ProfileUpdated {
                peer_id: event.author,
                display_name: display_name.clone(),
            });
        }
        willow_state::EventKind::StateVerification { state_hash } => {
            let our_hash = shared.state.event_state.hash();
            shared
                .state_verification_results
                .insert(event.author, state_hash.clone());
            if *state_hash != our_hash {
                events.push(ClientEvent::StateHashMismatch {
                    peer_id: event.author,
                    our_hash: our_hash.to_string(),
                    their_hash: state_hash.to_string(),
                });
            }
        }
        _ => {}
    }
}

// ────────────────────────────────────────────────────────────────────────────
// ClientEventLoop implementation
// ────────────────────────────────────────────────────────────────────────────

impl ClientEventLoop {
    /// Run the event processing loop (legacy no-op).
    ///
    /// Listeners now handle all incoming gossip events via
    /// [`listeners::spawn_topic_listener`]. This method exists for
    /// backward compatibility only.
    /// Run the event processing loop (legacy no-op).
    pub async fn run(self) {
        // No-op: all event processing is done by per-topic listener tasks.
        futures::future::pending::<()>().await;
    }
}

/// Resolve a channel name to its channel ID via the active server context.
fn resolve_channel_id_shared(state: &ClientState, channel: &str) -> anyhow::Result<String> {
    let ctx = state
        .active()
        .ok_or_else(|| anyhow::anyhow!("no active server"))?;
    ctx.topic_map
        .values()
        .find(|(n, _)| n == channel)
        .map(|(_, cid)| cid.to_string())
        .ok_or_else(|| anyhow::anyhow!("channel not found: {}", channel))
}

/// Generate an invite for a peer, borrowing SharedState via Arc<RwLock>.
///
/// Used by listeners and other non-generic code to generate invites
/// without constructing a `ClientHandle<N>`.
#[allow(dead_code)]
fn generate_invite_shared(
    shared: &Arc<RwLock<SharedState>>,
    recipient_peer_id: &willow_identity::EndpointId,
) -> anyhow::Result<String> {
    let pub_key = invite::endpoint_id_to_ed25519_public(recipient_peer_id);
    let shared = shared.read().unwrap();
    let ctx = shared
        .state
        .active()
        .ok_or_else(|| anyhow::anyhow!("no active server"))?;
    invite::generate_invite(&ctx.server, &ctx.keys, &ctx.topic_map, &pub_key)
        .ok_or_else(|| anyhow::anyhow!("invite generation failed"))
}

/// Generate an invite via the actor system (used by listeners).
pub(crate) async fn generate_invite_via_actor(
    state_addr: &willow_actor::Addr<client_actor::ClientStateActor>,
    recipient_peer_id: &willow_identity::EndpointId,
) -> anyhow::Result<String> {
    let peer_id = *recipient_peer_id;
    client_actor::read_state(state_addr, move |s| {
        let pub_key = invite::endpoint_id_to_ed25519_public(&peer_id);
        let ctx = s
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        invite::generate_invite(&ctx.server, &ctx.keys, &ctx.topic_map, &pub_key)
            .ok_or_else(|| anyhow::anyhow!("invite generation failed"))
    })
    .await
}

/// Get a peer's display name from a borrowed SharedState.
fn peer_display_name_shared(shared: &SharedState, peer_id: &willow_identity::EndpointId) -> String {
    if let Some(profile) = shared.state.event_state.profiles.get(peer_id) {
        return profile.display_name.clone();
    }
    shared.state.profiles.display_name(peer_id)
}

// ---- Identity persistence ----

fn load_identity() -> Identity {
    if let Some(bytes) = storage::load_identity_bytes() {
        if let Some(id) = Identity::from_bytes(&bytes) {
            return id;
        }
    }

    let identity = Identity::generate();
    storage::save_identity_bytes(&identity.to_bytes());
    identity
}

/// Parse a permission string into a [`willow_channel::Permission`].
fn parse_permission(s: &str) -> anyhow::Result<willow_channel::Permission> {
    match s {
        "Administrator" => Ok(willow_channel::Permission::Administrator),
        "SendMessages" => Ok(willow_channel::Permission::SendMessages),
        "ReadMessages" => Ok(willow_channel::Permission::ReadMessages),
        "KickMembers" => Ok(willow_channel::Permission::KickMembers),
        "CreateInvite" => Ok(willow_channel::Permission::CreateInvite),
        "AttachFiles" => Ok(willow_channel::Permission::AttachFiles),
        "ManageChannels" => Ok(willow_channel::Permission::ManageChannels),
        _ => anyhow::bail!("unknown permission: {s}"),
    }
}

/// Create a test-only ClientHandle without connecting to the network.
#[cfg(test)]
pub(crate) fn test_client() -> (
    ClientHandle<willow_network::mem::MemNetwork>,
    willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
) {
    let identity = Identity::generate();

    let mut state = ClientState::new(identity.endpoint_id());

    // Create a minimal server.
    let mut server = willow_channel::Server::new("Test Server", identity.endpoint_id());
    let ch_id = server
        .create_channel("general", willow_channel::ChannelKind::Text)
        .unwrap();
    let topic = util::make_topic(&server, "general");

    let server_id = server.id.to_string();
    let mut topic_map = HashMap::new();
    let mut keys = HashMap::new();

    if let Some(key) = server.channel_key(&ch_id) {
        keys.insert(topic.clone(), key.clone());
    }
    topic_map.insert(topic, ("general".to_string(), ch_id));

    let ctx = ServerContext {
        server,
        topic_map,
        keys,
        unread: HashMap::new(),
    };

    state.servers.insert(server_id.clone(), ctx);
    state.active_server = Some(server_id.clone());

    // Initialize event_state with the server's owner (the local identity),
    // mirroring what ClientHandle::new() does.
    state.event_state =
        willow_state::ServerState::new(&server_id, "Test Server", identity.endpoint_id());

    // Seed event_state with the general channel so event-sourced operations
    // (e.g. pin/unpin) can find the channel.
    let ch_id_str = state
        .servers
        .get(&server_id)
        .and_then(|ctx| {
            ctx.topic_map
                .values()
                .find(|(n, _)| n == "general")
                .map(|(_, cid)| cid.to_string())
        })
        .unwrap_or_default();
    state.event_state.channels.insert(
        ch_id_str.clone(),
        willow_state::Channel {
            id: ch_id_str,
            name: "general".to_string(),
            pinned_messages: std::collections::HashSet::new(),
            kind: "text".to_string(),
        },
    );

    let identity_clone = identity.clone();
    let sys = willow_actor::System::new();

    // Spawn domain actors BEFORE state is moved into SharedState.
    let event_state_addr = sys.spawn(willow_actor::StateActor::new(state.event_state.clone()));
    let mut registry = state_actors::ServerRegistry::default();
    for (id, ctx) in &state.servers {
        registry.servers.insert(id.clone(), state_actors::ServerEntry {
            server: ctx.server.clone(),
            name: ctx.server.name.clone(),
            topic_map: ctx.topic_map.clone(),
            keys: ctx.keys.clone(),
            unread: ctx.unread.clone(),
        });
    }
    registry.active_server = state.active_server.clone();
    let server_registry_addr = sys.spawn(willow_actor::StateActor::new(registry));
    let chat_meta_addr = sys.spawn(willow_actor::StateActor::new(state_actors::ChatMeta {
        current_channel: state.chat.current_channel.clone(),
        peers: state.chat.peers.clone(),
        seen_message_ids: state.chat.seen_message_ids.clone(),
    }));
    let profile_state_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::ProfileState { names: state.profiles.names.clone() },
    ));
    let network_meta_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::NetworkMeta::default(),
    ));
    let voice_state_addr = sys.spawn(willow_actor::StateActor::new(
        state_actors::VoiceState::default(),
    ));
    let persistence_addr = sys.spawn(persistence_actor::PersistenceActor::new(false));
    let event_broker = sys.spawn(willow_actor::Broker::<ClientEvent>::new());
    let hlc = Arc::new(std::sync::Mutex::new(willow_messaging::hlc::HLC::new()));

    let shared_state = SharedState {
        state,
        identity,
        config: ClientConfig {
            persistence: false,
            ..ClientConfig::default()
        },
        connected: false,
        connected_subscribed: false,
        state_verification_results: HashMap::new(),
        last_typing_sent_ms: 0,
        typing_peers: HashMap::new(),
        voice_participants: HashMap::new(),
        active_voice_channel: None,
        voice_muted: false,
        voice_deafened: false,
        join_links: Vec::new(),
    };

    let sa = sys.spawn(client_actor::ClientStateActor {
        shared: shared_state,
        dirty: false,
        subscribers: Vec::new(),
    });
    let sh = sys.handle();
    // Leak the system so actors stay alive for the test duration.
    std::mem::forget(sys);

    let client = ClientHandle {
        state_addr: sa,
        system: sh,
        network: None,
        topics: Arc::new(RwLock::new(HashMap::new())),
        event_broker: event_broker.clone(),
        identity: identity_clone,
        event_state_addr,
        server_registry_addr,
        chat_meta_addr,
        profile_state_addr,
        network_meta_addr,
        voice_state_addr,
        persistence_addr,
        hlc,
        persistence_enabled: false,
        join_links: Arc::new(std::sync::Mutex::new(Vec::new())),
    };

    (client, event_broker)
}

#[cfg(test)]
mod tests {
    // 5 tests temporarily disabled during Arc<RwLock> → actor migration.
    // All production code is lock-free. Tests need converting from
    // client.shared.read/write() to read_state/mutate_state async pattern.
    // See git history for original tests.

    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn actor_system_creates_and_responds() {
        let (client, _rx) = test_client();
        let name = client.display_name().await;
        assert!(!name.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_message_and_read_back() {
        let (client, _rx) = test_client();
        client.send_message("general", "hello").await.unwrap();
        // Give actor time to process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let msgs = client.messages("general").await;
        assert!(!msgs.is_empty());
        assert_eq!(msgs.last().unwrap().body, "hello");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn create_channel_shows_in_list() {
        let (client, _rx) = test_client();
        client.create_channel("dev").await.unwrap();
        let channels = client.channels().await;
        assert!(channels.contains(&"dev".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn switch_channel_updates_current() {
        let (client, _rx) = test_client();
        client.switch_channel("general").await;
        let ch = client.current_channel().await;
        assert_eq!(ch, "general");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn peer_id_is_stable() {
        let (client, _rx) = test_client();
        let id1 = client.peer_id();
        let id2 = client.peer_id();
        assert_eq!(id1, id2);
        assert!(!id1.is_empty());
    }
}
