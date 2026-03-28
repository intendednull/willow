//! # Willow Client
//!
//! UI-agnostic client library for the Willow P2P chat network.
//! Use this crate to build bots, CLIs, TUIs, or alternative frontends.
//!
//! ## Quick start
//!
//! ```no_run
//! use willow_client::{ClientHandle, ClientConfig, ClientEvent};
//!
//! let (client, event_loop) = ClientHandle::new(ClientConfig::default());
//! client.connect();
//!
//! // Spawn the event loop; it sends ClientEvents to the receiver.
//! let (tx, mut rx) = futures::channel::mpsc::unbounded::<ClientEvent>();
//! // event_loop.run(tx) is an async future — spawn it on your runtime.
//!
//! client.send_message("general", "hello!").ok();
//! ```

pub mod base64;
pub mod emoji;
pub mod events;
pub mod files;
pub mod invite;
pub mod network;
pub mod ops;
pub mod state;
pub mod storage;
pub mod util;
pub mod worker_cache;

// Re-export key types at crate root for convenience.
pub use events::ClientEvent;
pub use ops::{pack_wire, unpack_wire, VoiceSignalPayload, WireMessage};
pub use state::{
    ChatState, ClientState, DisplayMessage, PersistentEventStore, ProfileStore, ServerContext,
};

/// Re-export the event-sourced state crate for use by downstream consumers.
pub use willow_state;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use futures::channel::mpsc as futures_mpsc;

use willow_identity::Identity;
use willow_messaging::Content;
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

type DeferredPair = (
    futures_mpsc::UnboundedSender<network::NetworkEvent>,
    futures_mpsc::UnboundedReceiver<network::NetworkCommand>,
);

/// All mutable state shared between ClientHandle and ClientEventLoop.
pub struct SharedState {
    pub state: ClientState,
    pub identity: Identity,
    pub config: ClientConfig,
    pub connected: bool,
    pub connected_subscribed: bool,
    pub typing_peers: HashMap<String, (String, u64)>,
    pub voice_participants: HashMap<String, std::collections::HashSet<String>>,
    pub active_voice_channel: Option<String>,
    pub voice_muted: bool,
    pub voice_deafened: bool,
    pub state_verification_results: HashMap<String, willow_state::StateHash>,
    pub last_typing_sent_ms: u64,
    pub join_links: Vec<ops::JoinLink>,
}

/// Cloneable command interface for UI components.
#[derive(Clone)]
pub struct ClientHandle {
    pub(crate) shared: Rc<RefCell<SharedState>>,
    pub(crate) cmd_tx: futures_mpsc::UnboundedSender<network::NetworkCommand>,
    /// Holds deferred channel halves until connect() consumes them.
    pub(crate) deferred_channels: Option<Rc<RefCell<Option<DeferredPair>>>>,
}

/// Async event processing loop.
///
/// Owns the network event receiver and processes incoming events in a loop,
/// forwarding [`ClientEvent`]s to the provided sender. Created by
/// [`ClientHandle::new()`] and consumed by calling [`ClientEventLoop::run()`].
pub struct ClientEventLoop {
    pub(crate) shared: Rc<RefCell<SharedState>>,
    pub(crate) event_rx: futures_mpsc::UnboundedReceiver<network::NetworkEvent>,
    pub(crate) cmd_tx: futures_mpsc::UnboundedSender<network::NetworkCommand>,
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

/// Called when we first hear from the network (Listening or PeerConnected).
/// Subscribes to all channel topics for ALL servers, profile topic,
/// server ops topic, broadcasts profile, and requests sync.
///
/// Takes explicit parameters rather than `&self` so it can be called from
/// both ClientHandle and ClientEventLoop without borrow conflicts.
fn on_connected(
    state: &ClientState,
    cmd_tx: &futures_mpsc::UnboundedSender<network::NetworkCommand>,
) {
    // Subscribe to all channel topics across all servers.
    for ctx in state.servers.values() {
        for topic in ctx.topic_map.keys() {
            let _ = cmd_tx.unbounded_send(network::NetworkCommand::Subscribe(topic.clone()));
        }
    }

    // Subscribe to the global profile broadcast topic.
    let _ = cmd_tx.unbounded_send(network::NetworkCommand::Subscribe(
        network::PROFILE_TOPIC.to_string(),
    ));

    // Subscribe to server state operations topic.
    let _ = cmd_tx.unbounded_send(network::NetworkCommand::Subscribe(
        ops::SERVER_OPS_TOPIC.to_string(),
    ));

    // Broadcast our profile.
    let saved_profile = storage::load_profile().unwrap_or_default();
    if !saved_profile.display_name.is_empty() {
        let _ = cmd_tx.unbounded_send(network::NetworkCommand::BroadcastProfile {
            display_name: saved_profile.display_name,
        });
    }

    // Request missing events via event-sourced sync.
    let state_hash = state.event_store.latest_hash();
    let _ = cmd_tx.unbounded_send(network::NetworkCommand::RequestSync {
        state_hash: state_hash.clone(),
        topic: None,
    });
    for ctx in state.servers.values() {
        for topic in ctx.topic_map.keys() {
            let _ = cmd_tx.unbounded_send(network::NetworkCommand::RequestSync {
                state_hash: state_hash.clone(),
                topic: Some(topic.clone()),
            });
        }
    }
}

/// Reconcile `topic_map` channel IDs with `event_state.channels`.
///
/// After event state is loaded or synced, the `topic_map` may have stale
/// channel IDs (from invite acceptance or legacy storage). This updates
/// them to match the authoritative IDs in `event_state`.
fn reconcile_topic_map(state: &mut ClientState) {
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
        let owner = ctx.server.owner.to_string();
        state.event_state =
            willow_state::ServerState::new(server_id, ctx.server.name.clone(), owner);
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

impl ClientHandle {
    /// Create a new client. Loads or generates identity, loads or creates
    /// the server with default channels, loads persisted messages.
    ///
    /// Does **not** connect to the network -- call [`ClientHandle::connect()`] for that.
    ///
    /// Returns `(ClientHandle, ClientEventLoop)`.
    pub fn new(config: ClientConfig) -> (Self, ClientEventLoop) {
        let identity = load_identity();

        let (cmd_tx, cmd_rx) = futures_mpsc::unbounded();
        let (event_tx, event_rx) = futures_mpsc::unbounded();

        // We hold cmd_rx and event_tx until connect() is called.
        // Store them in a side channel via Rc<RefCell>.
        let deferred = Rc::new(RefCell::new(Some((event_tx, cmd_rx))));

        let mut state = ClientState::default();

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
                    let owner = ctx.server.owner.to_string();
                    state.event_state =
                        willow_state::ServerState::new(sid.clone(), ctx.server.name.clone(), owner);

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
        let peer_id_str = identity.peer_id().to_string();
        if let Some(ref name) = config.display_name {
            state
                .profiles
                .names
                .insert(peer_id_str.clone(), name.clone());
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
                    .insert(peer_id_str, profile.display_name);
            }
        }

        let shared_state = SharedState {
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

        let shared = Rc::new(RefCell::new(shared_state));

        let handle = ClientHandle {
            shared: Rc::clone(&shared),
            cmd_tx: cmd_tx.clone(),
            deferred_channels: Some(deferred),
        };

        reconcile_topic_map(&mut handle.shared.borrow_mut().state);

        let event_loop = ClientEventLoop {
            shared,
            event_rx,
            cmd_tx,
        };

        (handle, event_loop)
    }

    /// Connect to the P2P network. Spawns the network task in the background.
    ///
    /// After connecting, run [`ClientEventLoop::run()`] to process events.
    pub fn connect(&self) {
        let mut shared = self.shared.borrow_mut();
        if shared.connected {
            return;
        }

        let Some(deferred_rc) = self.deferred_channels.as_ref() else {
            return;
        };
        let Some((event_tx, cmd_rx)) = deferred_rc.borrow_mut().take() else {
            return;
        };

        let config = network::build_network_config(shared.config.relay_addr.as_deref());

        if shared.config.persistence {
            storage::save_settings(&storage::NetworkSettings {
                relay_addr: shared.config.relay_addr.clone(),
            });
        }

        network::spawn_network(shared.identity.clone(), event_tx, cmd_rx, config);
        shared.connected = true;
    }

    // ---- Voice chat ----

    /// Join a voice channel. Leaves the current voice channel first if in one.
    pub fn join_voice(&self, channel_id: &str) {
        // Leave current voice channel if in one.
        if self.shared.borrow().active_voice_channel.is_some() {
            self.leave_voice();
        }
        let mut shared = self.shared.borrow_mut();
        let my_peer_id = shared.identity.peer_id().to_string();
        shared.active_voice_channel = Some(channel_id.to_string());
        // Add ourselves to participants.
        shared
            .voice_participants
            .entry(channel_id.to_string())
            .or_default()
            .insert(my_peer_id);
        // Broadcast join.
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::SendVoiceJoin {
                channel_id: channel_id.to_string(),
            });
    }

    /// Leave the current voice channel, if in one.
    pub fn leave_voice(&self) {
        let mut shared = self.shared.borrow_mut();
        let my_peer_id = shared.identity.peer_id().to_string();
        if let Some(ch) = shared.active_voice_channel.take() {
            // Remove ourselves from participants.
            if let Some(participants) = shared.voice_participants.get_mut(&ch) {
                participants.remove(&my_peer_id);
            }
            let _ = self
                .cmd_tx
                .unbounded_send(network::NetworkCommand::SendVoiceLeave { channel_id: ch });
        }
    }

    /// Toggle mute state. Returns the new muted value.
    pub fn toggle_mute(&self) -> bool {
        let mut shared = self.shared.borrow_mut();
        shared.voice_muted = !shared.voice_muted;
        shared.voice_muted
    }

    /// Toggle deafen state. Returns the new deafened value.
    pub fn toggle_deafen(&self) -> bool {
        let mut shared = self.shared.borrow_mut();
        shared.voice_deafened = !shared.voice_deafened;
        shared.voice_deafened
    }

    /// Returns the list of peer IDs currently in the given voice channel.
    pub fn voice_participants(&self, channel_id: &str) -> Vec<String> {
        let shared = self.shared.borrow();
        shared
            .voice_participants
            .get(channel_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns the voice channel we are currently in, if any.
    pub fn active_voice_channel(&self) -> Option<String> {
        self.shared.borrow().active_voice_channel.clone()
    }

    /// Returns whether we are currently muted.
    pub fn is_voice_muted(&self) -> bool {
        self.shared.borrow().voice_muted
    }

    /// Returns whether we are currently deafened.
    pub fn is_voice_deafened(&self) -> bool {
        self.shared.borrow().voice_deafened
    }

    /// Send a voice signaling message to a specific peer.
    pub fn send_voice_signal(
        &self,
        channel_id: &str,
        target: &str,
        signal: ops::VoiceSignalPayload,
    ) {
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::SendVoiceSignal {
                channel_id: channel_id.to_string(),
                target_peer: target.to_string(),
                signal,
            });
    }

    /// Returns `(channel_name, kind_str)` pairs for the active server's channels.
    /// `kind_str` is `"text"` or `"voice"`.
    pub fn channel_kinds(&self) -> Vec<(String, String)> {
        let shared = self.shared.borrow();
        shared
            .state
            .event_state
            .channels
            .values()
            .map(|ch| (ch.name.clone(), ch.kind.clone()))
            .collect()
    }

    // ---- Server management ----

    /// Switch to a different server by ID.
    pub fn switch_server(&self, server_id: &str) {
        let mut shared = self.shared.borrow_mut();
        if shared.state.servers.contains_key(server_id) {
            shared.state.active_server = Some(server_id.to_string());
            init_event_state_on_shared(&mut shared, server_id);
            reconcile_topic_map(&mut shared.state);
        }
    }

    /// List all servers as (id, name) pairs.
    pub fn server_list(&self) -> Vec<(String, String)> {
        self.shared.borrow().state.server_list()
    }

    /// Get the name of the currently active server.
    pub fn active_server_name(&self) -> String {
        let shared = self.shared.borrow();
        shared
            .state
            .active()
            .map(|ctx| ctx.server.name.clone())
            .unwrap_or_else(|| "No Server".to_string())
    }

    /// Get the ID of the currently active server.
    pub fn active_server_id(&self) -> Option<String> {
        self.shared.borrow().state.active_server.clone()
    }

    /// Check whether any servers exist.
    pub fn has_servers(&self) -> bool {
        !self.shared.borrow().state.servers.is_empty()
    }

    /// Remove a server from the local state and persist the change.
    ///
    /// If the removed server was active, switches to the first remaining
    /// server (or clears the active server if none remain).
    pub fn leave_server(&self, server_id: &str) {
        let mut shared = self.shared.borrow_mut();
        shared.state.servers.remove(server_id);
        if shared.state.active_server.as_deref() == Some(server_id) {
            shared.state.active_server = shared.state.servers.keys().next().cloned();
        }
        // Persist updated server list.
        let ids: Vec<String> = shared.state.servers.keys().cloned().collect();
        storage::save_server_list(&ids);
    }

    /// Create a brand-new server with the local user as owner.
    ///
    /// Automatically creates a "general" text channel, initializes the
    /// event-sourced state, persists everything, and subscribes to the
    /// channel topic on the network.
    ///
    /// Returns the server ID.
    pub fn create_server(&self, name: &str) -> anyhow::Result<String> {
        let mut shared = self.shared.borrow_mut();
        let mut server = willow_channel::Server::new(name, shared.identity.peer_id());
        let server_id = server.id.to_string();

        // Create default "general" channel.
        let ch_id = server
            .create_channel("general", willow_channel::ChannelKind::Text)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let topic = util::make_topic(&server, "general");

        let mut topic_map = HashMap::new();
        let mut keys = HashMap::new();

        if let Some(key) = server.channel_key(&ch_id) {
            keys.insert(topic.clone(), key.clone());
        }
        let ch_id_str = ch_id.to_string();
        topic_map.insert(topic.clone(), ("general".to_string(), ch_id));

        let ctx = ServerContext {
            server,
            topic_map,
            keys,
            unread: HashMap::new(),
        };

        shared.state.servers.insert(server_id.clone(), ctx);
        shared.state.active_server = Some(server_id.clone());
        shared.state.chat.current_channel = "general".to_string();

        // Initialize event-sourced state for this server.
        let peer_id = shared.identity.peer_id().to_string();
        shared.state.event_state =
            willow_state::ServerState::new(server_id.clone(), name.to_string(), peer_id.clone());

        // Open event store.
        if shared.config.persistence {
            if let Some(store) = storage::open_event_store(&server_id) {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    shared.state.event_store = state::PersistentEventStore::Sqlite(store);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    shared.state.event_store = state::PersistentEventStore::LocalStorage(store);
                }
            }
        }

        // Create the general channel via event.
        let create_ch = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: ch_id_str,
                kind: "text".to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &create_ch);

        // Persist.
        if shared.config.persistence {
            persist_servers(&shared.state);
        }

        // Subscribe to channel topic if connected.
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::Subscribe(topic));

        Ok(server_id)
    }

    /// Grant SyncProvider permission to the given worker peer IDs.
    ///
    /// Called during server creation for each worker the user wants to
    /// authorize. Workers need SyncProvider to serve state.
    pub fn authorize_workers(&self, worker_peer_ids: &[String]) {
        let mut events_to_broadcast = Vec::new();
        {
            let mut shared = self.shared.borrow_mut();
            let peer_id = shared.identity.peer_id().to_string();
            for worker_pid in worker_peer_ids {
                let event = willow_state::Event {
                    id: uuid::Uuid::new_v4().to_string(),
                    parent_hash: shared.state.event_state.hash(),
                    author: peer_id.clone(),
                    timestamp_ms: util::current_time_ms(),
                    kind: willow_state::EventKind::GrantPermission {
                        peer_id: worker_pid.clone(),
                        permission: willow_state::Permission::SyncProvider,
                    },
                };
                apply_event_on_shared(&mut shared, &event);
                events_to_broadcast.push(event);
            }
        }
        // Broadcast after releasing the borrow.
        for event in events_to_broadcast {
            let _ = self.cmd_tx.unbounded_send(
                network::NetworkCommand::BroadcastEvent { event, topic: None },
            );
        }
    }

    /// Set display name for the active server via event-sourced state.
    pub fn set_server_display_name(&self, name: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        if shared.state.active_server.is_none() {
            return Err(anyhow::anyhow!("no active server"));
        }
        let peer_id = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::SetProfile {
                display_name: name.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);

        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        // Also update the global profile for backward compat.
        let pid = shared.identity.peer_id().to_string();
        shared.state.profiles.names.insert(pid, name.to_string());

        storage::save_profile(&storage::LocalProfile {
            display_name: name.to_string(),
        });

        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastProfile {
                display_name: name.to_string(),
            });

        Ok(())
    }

    /// Get the display name for the active server (from event-sourced state).
    pub fn server_display_name(&self) -> String {
        let shared = self.shared.borrow();
        let peer_id = shared.identity.peer_id().to_string();
        shared
            .state
            .event_state
            .profiles
            .get(&peer_id)
            .map(|p| p.display_name.clone())
            .unwrap_or_else(|| {
                // Fall back to legacy profile store.
                if let Some(profile) = shared.state.event_state.profiles.get(&peer_id) {
                    return profile.display_name.clone();
                }
                shared.state.profiles.display_name(&peer_id)
            })
    }

    // ---- Action methods ----

    /// Send a text message to the given channel.
    pub fn send_message(&self, channel: &str, body: &str) -> anyhow::Result<()> {
        let content = Content::Text {
            body: body.to_string(),
        };
        self.send_content(channel, content, body, None, None)
    }

    /// Send a reply to a specific message.
    pub fn send_reply(&self, channel: &str, parent_id: &str, body: &str) -> anyhow::Result<()> {
        let parent =
            willow_messaging::MessageId(uuid::Uuid::parse_str(parent_id).unwrap_or_default());
        let content = Content::Reply {
            parent,
            body: body.to_string(),
        };

        // Build reply preview from event_state messages.
        let shared = self.shared.borrow();
        let preview = shared
            .state
            .event_state
            .messages
            .iter()
            .find(|m| m.id == parent_id)
            .map(|m| {
                let text = if m.body.len() > 50 {
                    format!("{}...", &m.body[..50])
                } else {
                    m.body.clone()
                };
                let author_name = shared
                    .state
                    .event_state
                    .profiles
                    .get(&m.author)
                    .map(|p| p.display_name.clone())
                    .unwrap_or_else(|| shared.state.profiles.display_name(&m.author));
                format!("{author_name}: {text}")
            });
        drop(shared);

        self.send_content(channel, content, body, preview, Some(parent_id.to_string()))
    }

    /// Share a small file inline by base64-encoding it into a text message.
    ///
    /// The message body uses the format `[file:filename:base64data]` so the
    /// UI can detect it and render a download card. Files larger than 256 KB
    /// are rejected.
    pub fn share_file_inline(
        &self,
        channel: &str,
        filename: &str,
        data: &[u8],
    ) -> anyhow::Result<()> {
        const MAX_INLINE_SIZE: usize = 256 * 1024;
        if data.len() > MAX_INLINE_SIZE {
            anyhow::bail!("file too large for inline sharing (max 256 KB)");
        }
        let encoded = base64::encode(data);
        let body = format!("[file:{}:{}]", filename, encoded);
        self.send_message(channel, &body)
    }

    /// Edit an existing message.
    pub fn edit_message(
        &self,
        _channel: &str,
        message_id: &str,
        new_body: &str,
    ) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let _ = shared
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let peer_id_str = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::EditMessage {
                message_id: message_id.to_string(),
                new_body: new_body.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Delete a message.
    pub fn delete_message(&self, _channel: &str, message_id: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let _ = shared
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let peer_id_str = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::DeleteMessage {
                message_id: message_id.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Add a reaction to a message.
    pub fn react(&self, _channel: &str, message_id: &str, emoji: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let _ = shared
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let peer_id_str = shared.identity.peer_id().to_string();
        let reaction_event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::Reaction {
                message_id: message_id.to_string(),
                emoji: emoji.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &reaction_event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent {
                event: reaction_event,
                topic: None,
            });

        Ok(())
    }

    /// Pin a message in a channel.
    ///
    /// Creates a `PinMessage` event in the event-sourced state and broadcasts
    /// it to peers.
    pub fn pin_message(&self, channel: &str, message_id: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let channel_id = resolve_channel_id_shared(&shared.state, channel)?;
        let peer_id = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::PinMessage {
                channel_id,
                message_id: message_id.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Unpin a message from a channel.
    ///
    /// Creates an `UnpinMessage` event in the event-sourced state and
    /// broadcasts it to peers.
    pub fn unpin_message(&self, channel: &str, message_id: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let channel_id = resolve_channel_id_shared(&shared.state, channel)?;
        let peer_id = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::UnpinMessage {
                channel_id,
                message_id: message_id.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Get pinned message IDs for a channel from the event-sourced state.
    ///
    /// Returns a sorted `Vec` of message IDs that are pinned in the channel.
    pub fn pinned_message_ids(&self, channel: &str) -> Vec<String> {
        let shared = self.shared.borrow();
        // Find channel_id from event_state by name (authoritative).
        let channel_id = shared
            .state
            .event_state
            .channels
            .iter()
            .find(|(_, ch)| ch.name == channel)
            .map(|(id, _)| id.clone())
            .or_else(|| {
                shared.state.active().and_then(|ctx| {
                    ctx.topic_map
                        .values()
                        .find(|(n, _)| n == channel)
                        .map(|(_, cid)| cid.to_string())
                })
            })
            .unwrap_or_default();

        shared
            .state
            .event_state
            .channels
            .get(&channel_id)
            .map(|ch| {
                let mut ids: Vec<String> = ch.pinned_messages.iter().cloned().collect();
                ids.sort();
                ids
            })
            .unwrap_or_default()
    }

    /// Get pinned messages for a channel.
    ///
    /// Returns messages whose IDs are in the event-sourced pinned set.
    pub fn pinned_messages(&self, channel: &str) -> Vec<state::DisplayMessage> {
        let pinned_ids = self.pinned_message_ids(channel);
        if pinned_ids.is_empty() {
            return vec![];
        }
        let pinned_set: std::collections::HashSet<&str> =
            pinned_ids.iter().map(|s| s.as_str()).collect();
        self.messages(channel)
            .into_iter()
            .filter(|m| pinned_set.contains(m.id.as_str()))
            .collect()
    }

    /// Check if a message is pinned in a channel.
    pub fn is_pinned(&self, channel: &str, message_id: &str) -> bool {
        let pinned_ids = self.pinned_message_ids(channel);
        pinned_ids.iter().any(|id| id == message_id)
    }

    /// Create a new channel.
    pub fn create_channel(&self, name: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let ctx = shared
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let ch_id = ctx
            .server
            .create_channel(name, willow_channel::ChannelKind::Text)?;
        let topic = util::make_topic(&ctx.server, name);

        if let Some(key) = ctx.server.channel_key(&ch_id) {
            ctx.keys.insert(topic.clone(), key.clone());
        }
        storage::save_server(&ctx.server, &ctx.keys);

        let ch_id_str = ch_id.to_string();
        ctx.topic_map
            .insert(topic.clone(), (name.to_string(), ch_id));

        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::Subscribe(topic));

        // Create and apply event, then broadcast it.
        let peer_id_str = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::CreateChannel {
                name: name.to_string(),
                channel_id: ch_id_str,
                kind: "text".to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        shared.state.chat.current_channel = name.to_string();

        Ok(())
    }

    /// Create a voice channel.
    pub fn create_voice_channel(&self, name: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let ctx = shared
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let ch_id = ctx
            .server
            .create_channel(name, willow_channel::ChannelKind::Voice)?;
        let topic = util::make_topic(&ctx.server, name);

        if let Some(key) = ctx.server.channel_key(&ch_id) {
            ctx.keys.insert(topic.clone(), key.clone());
        }
        storage::save_server(&ctx.server, &ctx.keys);

        let ch_id_str = ch_id.to_string();
        ctx.topic_map
            .insert(topic.clone(), (name.to_string(), ch_id));

        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::Subscribe(topic));

        let peer_id_str = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::CreateChannel {
                name: name.to_string(),
                channel_id: ch_id_str,
                kind: "voice".to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Delete a channel.
    pub fn delete_channel(&self, name: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let ctx = shared
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let Some((topic, (_ch_name, ch_id))) = ctx
            .topic_map
            .iter()
            .find(|(_, (n, _))| n == name)
            .map(|(t, v)| (t.clone(), v.clone()))
        else {
            anyhow::bail!("channel not found");
        };

        let ch_id_str = ch_id.to_string();

        ctx.server.delete_channel(&ch_id)?;
        storage::save_server(&ctx.server, &ctx.keys);

        ctx.topic_map.remove(&topic);
        ctx.keys.remove(&topic);

        if shared.state.chat.current_channel == name {
            let names = shared
                .state
                .active()
                .map(|ctx| ctx.channel_names())
                .unwrap_or_default();
            shared.state.chat.current_channel = names.first().cloned().unwrap_or_default();
        }

        // Create and apply event, then broadcast it.
        let peer_id_str = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::DeleteChannel {
                channel_id: ch_id_str,
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Trust a peer for server state operations.
    ///
    /// Applies a `GrantPermission(Administrator)` event to the event-sourced
    /// state and broadcasts the event on the wire.
    pub fn trust_peer(&self, peer_id: &str) {
        let mut shared = self.shared.borrow_mut();
        let author = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::GrantPermission {
                peer_id: peer_id.to_string(),
                permission: willow_state::Permission::Administrator,
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });
    }

    /// Revoke trust from a peer.
    ///
    /// Applies a `RevokePermission(Administrator)` event to the event-sourced
    /// state and broadcasts the event on the wire.
    pub fn untrust_peer(&self, peer_id: &str) {
        let mut shared = self.shared.borrow_mut();
        let author = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::RevokePermission {
                peer_id: peer_id.to_string(),
                permission: willow_state::Permission::Administrator,
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });
    }

    /// Kick a member, rotating channel keys.
    pub fn kick_member(&self, peer_id: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let ctx = shared
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let member_peer = ctx
            .server
            .members()
            .iter()
            .find(|m| m.peer_id.to_string() == peer_id)
            .map(|m| m.peer_id.clone());

        let Some(peer) = member_peer else {
            anyhow::bail!("peer not found in server members");
        };

        let rotated = ctx.server.remove_member(&peer)?;
        storage::save_server(&ctx.server, &ctx.keys);

        // Update key store with rotated keys.
        for (ch_id, key) in &rotated {
            for (topic, (_, tid)) in &ctx.topic_map {
                if tid == ch_id {
                    ctx.keys.insert(topic.clone(), key.clone());
                    break;
                }
            }
        }

        shared.state.chat.peers.retain(|p| p != peer_id);

        // Create and apply event, then broadcast it.
        let author = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::KickMember {
                peer_id: peer_id.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Create a new role.
    pub fn create_role(&self, name: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let role_id = willow_channel::RoleId::new();
        let role = willow_channel::Role::with_id(role_id.clone(), name);

        let ctx = shared
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        ctx.server.create_role(role);
        storage::save_server(&ctx.server, &ctx.keys);

        let peer_id_str = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::CreateRole {
                name: name.to_string(),
                role_id: role_id.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Delete a role by ID.
    pub fn delete_role(&self, role_id: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let rid = willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());

        let ctx = shared
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        ctx.server.delete_role(&rid)?;
        storage::save_server(&ctx.server, &ctx.keys);

        let peer_id_str = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::DeleteRole {
                role_id: role_id.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Set a permission on a role.
    pub fn set_permission(
        &self,
        role_id: &str,
        permission: &str,
        granted: bool,
    ) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let rid = willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
        let perm = parse_permission(permission)?;

        let ctx = shared
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        ctx.server.set_permission(&rid, perm, granted)?;
        storage::save_server(&ctx.server, &ctx.keys);

        let peer_id_str = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::SetPermission {
                role_id: role_id.to_string(),
                permission: permission.to_string(),
                granted,
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Assign a role to a peer.
    pub fn assign_role(&self, peer_id: &str, role_id: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let rid = willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());

        let ctx = shared
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let member_peer = ctx
            .server
            .members()
            .iter()
            .find(|m| m.peer_id.to_string() == peer_id)
            .map(|m| m.peer_id.clone());

        let Some(peer) = member_peer else {
            anyhow::bail!("peer not found");
        };

        ctx.server.assign_role(&peer, &rid)?;
        storage::save_server(&ctx.server, &ctx.keys);

        let peer_id_str_author = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str_author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::AssignRole {
                peer_id: peer_id.to_string(),
                role_id: role_id.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });

        Ok(())
    }

    /// Broadcast a state verification event carrying this peer's current state hash.
    pub fn verify_state(&self) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let author = shared.identity.peer_id().to_string();
        let state_hash = shared.state.event_state.hash();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::StateVerification { state_hash },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });
        Ok(())
    }

    /// Returns (agreeing_peers, total_peers_reporting) based on collected
    /// StateVerification results.
    pub fn state_hash_agreement(&self) -> (usize, usize) {
        let shared = self.shared.borrow();
        let our_hash = shared.state.event_state.hash();
        let total = shared.state_verification_results.len();
        let agreeing = shared
            .state_verification_results
            .values()
            .filter(|h| **h == our_hash)
            .count();
        (agreeing, total)
    }

    /// Rename the server. Only the owner can do this.
    pub fn rename_server(&self, new_name: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let author = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::RenameServer {
                new_name: new_name.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });
        Ok(())
    }

    /// Set the server description. Only the owner can do this.
    pub fn set_server_description(&self, desc: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let author = shared.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::SetServerDescription {
                description: desc.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });
        Ok(())
    }

    /// Generate a secure invite code encrypted for the given recipient.
    pub fn generate_invite(&self, recipient_peer_id: &str) -> anyhow::Result<String> {
        let Some(pub_key) = invite::peer_id_to_ed25519_public(recipient_peer_id) else {
            anyhow::bail!("invalid recipient PeerId");
        };

        let shared = self.shared.borrow();
        let ctx = shared
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        invite::generate_invite(&ctx.server, &ctx.keys, &ctx.topic_map, &pub_key)
            .ok_or_else(|| anyhow::anyhow!("invite generation failed"))
    }

    /// Accept an invite code and join the server.
    pub fn accept_invite(&self, code: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let accepted = invite::accept_invite(code, &shared.identity)
            .ok_or_else(|| anyhow::anyhow!("invalid invite code or not for us"))?;

        let server_id = accepted.server_id.clone();

        // Check if we already have this server.
        if let Some(ctx) = shared.state.servers.get_mut(&server_id) {
            // Merge new channel keys into existing server context.
            for (topic, (name, key)) in &accepted.channel_keys {
                ctx.keys.insert(topic.clone(), key.clone());
                if !ctx.topic_map.contains_key(topic) {
                    ctx.topic_map.insert(
                        topic.clone(),
                        (name.clone(), willow_channel::ChannelId::new()),
                    );
                }
                let _ = self
                    .cmd_tx
                    .unbounded_send(network::NetworkCommand::Subscribe(topic.clone()));
            }
        } else {
            // Create a new server context for this server.
            // Use the ORIGINAL server ID from the invite so topics match.
            // Use the ACTUAL owner from the invite, not the joiner's peer ID.
            // This is persisted and used on reload to initialize event_state —
            // if the owner is wrong, the actual owner's events get rejected.
            let owner_peer_id = willow_identity::PeerId::parse(&accepted.owner)
                .unwrap_or_else(|| shared.identity.peer_id());
            let mut server = willow_channel::Server::new(&accepted.server_name, owner_peer_id);
            server.id = willow_channel::ServerId(
                uuid::Uuid::parse_str(&server_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
            );

            let mut topic_map = HashMap::new();
            let mut keys = HashMap::new();

            for (topic, (name, key)) in &accepted.channel_keys {
                // Create the channel on the server so it appears in channels().
                let ch_id = server
                    .create_channel(name, willow_channel::ChannelKind::Text)
                    .unwrap_or_else(|_| willow_channel::ChannelId::new());

                // Override the generated key with the one from the invite.
                server.set_channel_key(ch_id.clone(), key.clone());

                keys.insert(topic.clone(), key.clone());
                topic_map.insert(topic.clone(), (name.clone(), ch_id));
                let _ = self
                    .cmd_tx
                    .unbounded_send(network::NetworkCommand::Subscribe(topic.clone()));
            }

            let ctx = ServerContext {
                server,
                topic_map,
                keys,
                unread: HashMap::new(),
            };

            shared.state.servers.insert(server_id.clone(), ctx);
        }

        shared.state.active_server = Some(server_id.clone());
        init_event_state_on_shared(&mut shared, &server_id);

        // Fix the event_state owner to be the ACTUAL server owner from the invite,
        // not the joining peer. This is critical for permission checks -- without it,
        // the actual owner's events (CreateChannel, etc.) get rejected.
        shared.state.event_state.owner = accepted.owner.clone();
        // Also add the owner as a member so permissions work.
        if !shared
            .state
            .event_state
            .members
            .contains_key(&accepted.owner)
        {
            shared.state.event_state.members.insert(
                accepted.owner.clone(),
                willow_state::Member {
                    peer_id: accepted.owner.clone(),
                    roles: std::collections::HashSet::new(),
                    display_name: None,
                },
            );
        }

        reconcile_topic_map(&mut shared.state);

        if let Some((_, (name, _))) = accepted.channel_keys.iter().next() {
            shared.state.chat.current_channel = name.clone();
        }

        // Persist all servers so the joined server survives refresh.
        persist_servers(&shared.state);

        // Request sync for the new server -- get all events from peers.
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::RequestSync {
                state_hash: willow_state::StateHash::ZERO,
                topic: None,
            });
        if let Some(ctx) = shared.state.servers.get(&server_id) {
            for topic in ctx.topic_map.keys() {
                let _ = self
                    .cmd_tx
                    .unbounded_send(network::NetworkCommand::RequestSync {
                        state_hash: willow_state::StateHash::ZERO,
                        topic: Some(topic.clone()),
                    });
            }
        }

        Ok(())
    }

    /// Publish raw data on a gossipsub topic.
    pub fn publish(&self, topic: &str, data: Vec<u8>) {
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::Publish {
                topic: topic.to_string(),
                data,
            });
    }

    /// Send a JoinRequest for a link ID on the server ops topic.
    pub fn send_join_request(&self, link_id: &str) {
        let shared = self.shared.borrow();
        let msg = ops::WireMessage::JoinRequest {
            link_id: link_id.to_string(),
            peer_id: shared.identity.peer_id().to_string(),
        };
        if let Some(data) = ops::pack_wire(&msg, &shared.identity) {
            let _ = self
                .cmd_tx
                .unbounded_send(network::NetworkCommand::Publish {
                    topic: ops::SERVER_OPS_TOPIC.to_string(),
                    data,
                });
        }
    }

    /// Create a join link for the active server. Returns the encoded token string.
    /// Requires `CreateInvite` permission (owner has this implicitly).
    pub fn create_join_link(
        &self,
        max_uses: u32,
        expires_at: Option<u64>,
    ) -> anyhow::Result<String> {
        let mut shared = self.shared.borrow_mut();
        let server_id = shared
            .state
            .active_server
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let peer_id = shared.identity.peer_id().to_string();
        if !shared
            .state
            .event_state
            .has_permission(&peer_id, &willow_state::Permission::CreateInvite)
        {
            return Err(anyhow::anyhow!("missing CreateInvite permission"));
        }

        let server_name = shared
            .state
            .active()
            .map(|c| c.server.name.clone())
            .unwrap_or_default();
        let inviter_name = shared
            .state
            .profiles
            .names
            .get(&peer_id)
            .cloned()
            .unwrap_or_default();

        let link = ops::JoinLink {
            link_id: uuid::Uuid::new_v4().to_string(),
            server_id: server_id.clone(),
            max_uses,
            used: 0,
            active: true,
            expires_at,
            created_at: util::current_time_ms(),
        };

        let token = ops::JoinToken {
            inviter_peer_id: peer_id,
            server_id,
            link_id: link.link_id.clone(),
            server_name,
            inviter_name,
        };

        shared.join_links.push(link);
        if shared.config.persistence {
            storage::save_join_links(
                shared.state.active_server.as_deref().unwrap_or(""),
                &shared.join_links,
            );
        }

        Ok(token.encode())
    }

    /// Return all join links for the active server.
    pub fn join_links(&self) -> Vec<ops::JoinLink> {
        self.shared.borrow().join_links.clone()
    }

    /// Delete a join link by ID.
    pub fn delete_join_link(&self, link_id: &str) {
        let mut shared = self.shared.borrow_mut();
        shared.join_links.retain(|l| l.link_id != link_id);
        if shared.config.persistence {
            storage::save_join_links(
                shared.state.active_server.as_deref().unwrap_or(""),
                &shared.join_links,
            );
        }
    }

    /// Set the local display name and broadcast to peers.
    pub fn set_display_name(&self, name: &str) {
        let mut shared = self.shared.borrow_mut();
        let peer_id = shared.identity.peer_id().to_string();
        shared
            .state
            .profiles
            .names
            .insert(peer_id, name.to_string());

        storage::save_profile(&storage::LocalProfile {
            display_name: name.to_string(),
        });

        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastProfile {
                display_name: name.to_string(),
            });
    }

    /// Switch the current channel.
    pub fn switch_channel(&self, name: &str) {
        let mut shared = self.shared.borrow_mut();
        if shared.state.chat.current_channel != name {
            shared.state.chat.current_channel = name.to_string();

            if let Some(ctx) = shared.state.active_mut() {
                if let Some(topic) = ctx.topic_for_name(name) {
                    ctx.unread.remove(&topic);
                }
            }
        }
    }

    // ---- Typing indicator methods ----

    /// Notify peers that we are typing in the current channel.
    ///
    /// Debounced -- will not send more than once per 3 seconds.
    pub fn send_typing(&self) {
        let mut shared = self.shared.borrow_mut();
        let now = util::current_time_ms();
        if now - shared.last_typing_sent_ms < 3000 {
            return; // debounce
        }
        shared.last_typing_sent_ms = now;

        let channel = shared.state.chat.current_channel.clone();
        if !channel.is_empty() {
            let _ = self
                .cmd_tx
                .unbounded_send(network::NetworkCommand::SendTyping { channel });
        }
    }

    /// Get display names of peers currently typing in the given channel.
    ///
    /// Automatically expires entries older than 5 seconds and excludes the
    /// local user.
    pub fn typing_in(&self, channel: &str) -> Vec<String> {
        let mut shared = self.shared.borrow_mut();
        let now = util::current_time_ms();
        // Remove expired entries (older than 5 seconds).
        shared.typing_peers.retain(|_, (_, ts)| now - *ts < 5000);

        let my_id = shared.identity.peer_id().to_string();
        shared
            .typing_peers
            .iter()
            .filter(|(pid, (ch, _))| ch == channel && *pid != &my_id)
            .map(|(pid, _)| peer_display_name_shared(&shared, pid))
            .collect()
    }

    // ---- Accessor methods ----

    /// Get the local PeerId as a string.
    pub fn peer_id(&self) -> String {
        self.shared.borrow().identity.peer_id().to_string()
    }

    /// Get the local display name.
    ///
    /// Checks the event-sourced state profiles first, falling back to the
    /// legacy profile store.
    pub fn display_name(&self) -> String {
        let shared = self.shared.borrow();
        let pid = shared.identity.peer_id().to_string();
        if let Some(profile) = shared.state.event_state.profiles.get(&pid) {
            return profile.display_name.clone();
        }
        shared.state.profiles.display_name(&pid)
    }

    /// Get a peer's display name.
    ///
    /// Checks the event-sourced state profiles first, falling back to the
    /// legacy profile store.
    pub fn peer_display_name(&self, peer_id: &str) -> String {
        let shared = self.shared.borrow();
        peer_display_name_shared(&shared, peer_id)
    }

    /// Get messages for a channel, computed from the event-sourced state.
    pub fn messages(&self, channel: &str) -> Vec<state::DisplayMessage> {
        let shared = self.shared.borrow();
        // Collect ALL channel_ids that map to this channel name.
        // This handles the ID mismatch between owner and joiner.
        let mut channel_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        // From topic_map (legacy).
        if let Some(ctx) = shared.state.active() {
            for (name, cid) in ctx.topic_map.values() {
                if name == channel {
                    channel_ids.insert(cid.to_string());
                }
            }
        }

        // From event_state.channels (authoritative).
        for (id, ch) in &shared.state.event_state.channels {
            if ch.name == channel {
                channel_ids.insert(id.clone());
            }
        }

        if channel_ids.is_empty() {
            return vec![];
        }

        let local_peer_id = shared.identity.peer_id().to_string();

        let mut msgs: Vec<state::DisplayMessage> = shared
            .state
            .event_state
            .messages
            .iter()
            .filter(|m| channel_ids.contains(&m.channel_id))
            .map(|m| {
                let author_name = shared
                    .state
                    .event_state
                    .profiles
                    .get(&m.author)
                    .map(|p| p.display_name.clone())
                    .unwrap_or_else(|| shared.state.profiles.display_name(&m.author));

                let reply_preview = m.reply_to.as_ref().and_then(|parent_id| {
                    shared
                        .state
                        .event_state
                        .messages
                        .iter()
                        .find(|pm| pm.id == *parent_id)
                        .map(|pm| {
                            let parent_name = shared
                                .state
                                .event_state
                                .profiles
                                .get(&pm.author)
                                .map(|p| p.display_name.clone())
                                .unwrap_or_else(|| shared.state.profiles.display_name(&pm.author));
                            let text = if pm.body.len() > 50 {
                                format!("{}...", &pm.body[..50])
                            } else {
                                pm.body.clone()
                            };
                            format!("{parent_name}: {text}")
                        })
                });

                // Resolve reaction author names.
                let reactions = m
                    .reactions
                    .iter()
                    .map(|(emoji, peer_ids)| {
                        let names: Vec<String> = peer_ids
                            .iter()
                            .map(|pid| {
                                shared
                                    .state
                                    .event_state
                                    .profiles
                                    .get(pid)
                                    .map(|p| p.display_name.clone())
                                    .unwrap_or_else(|| shared.state.profiles.display_name(pid))
                            })
                            .collect();
                        (emoji.clone(), names)
                    })
                    .collect();

                state::DisplayMessage {
                    id: m.id.clone(),
                    channel_id: m.channel_id.clone(),
                    author_peer_id: m.author.clone(),
                    author_display_name: author_name,
                    body: m.body.clone(),
                    is_local: m.author == local_peer_id,
                    timestamp_ms: m.timestamp_ms,
                    reactions,
                    edited: m.edited,
                    deleted: m.deleted,
                    reply_to: m.reply_to.clone(),
                    reply_preview,
                }
            })
            .collect();

        msgs.sort_by_key(|m| m.timestamp_ms);
        msgs
    }

    /// List all channel names for the active server.
    ///
    /// Returns the union of channels from the server context and the
    /// event-sourced state, deduplicated and sorted.
    pub fn channels(&self) -> Vec<String> {
        let shared = self.shared.borrow();
        let mut names = shared.state.channel_names();

        // Merge any channels from event_state that aren't yet in the list.
        for ch in shared.state.event_state.channels.values() {
            if !names.contains(&ch.name) {
                names.push(ch.name.clone());
            }
        }

        names.sort();
        names.dedup();
        names
    }

    /// Get messages from the event-sourced state for a channel by ID.
    ///
    /// Returns all non-deleted messages for the given channel in
    /// event-sequence order (owned copies).
    pub fn event_messages(&self, channel_id: &str) -> Vec<willow_state::ChatMessage> {
        let shared = self.shared.borrow();
        shared
            .state
            .event_state
            .messages
            .iter()
            .filter(|m| m.channel_id == channel_id && !m.deleted)
            .cloned()
            .collect()
    }

    /// Get the list of connected peers (libp2p-level connections).
    pub fn peers(&self) -> Vec<String> {
        self.shared.borrow().state.chat.peers.clone()
    }

    /// Get all server members with online/offline status.
    ///
    /// Returns `(peer_id, display_name, is_online)` for each member.
    /// Falls back to `chat.peers` for peers not in the member list
    /// (e.g. connected before event sync completes).
    pub fn server_members(&self) -> Vec<(String, String, bool)> {
        let shared = self.shared.borrow();
        let local_id = shared.identity.peer_id().to_string();
        let online: std::collections::HashSet<&str> =
            shared.state.chat.peers.iter().map(|s| s.as_str()).collect();

        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Members from event-sourced state.
        for (pid, member) in &shared.state.event_state.members {
            let name = member
                .display_name
                .clone()
                .or_else(|| {
                    shared
                        .state
                        .event_state
                        .profiles
                        .get(pid)
                        .map(|p| p.display_name.clone())
                })
                .unwrap_or_else(|| peer_display_name_shared(&shared, pid));
            // Local user is always online.
            let is_online = *pid == local_id || online.contains(pid.as_str());
            result.push((pid.clone(), name, is_online));
            seen.insert(pid.clone());
        }

        // Connected peers not yet in the member list (pre-sync).
        for pid in &shared.state.chat.peers {
            if !seen.contains(pid) {
                let name = peer_display_name_shared(&shared, pid);
                result.push((pid.clone(), name, true));
            }
        }

        result
    }

    /// Whether the network is connected.
    pub fn is_connected(&self) -> bool {
        self.shared.borrow().connected
    }

    /// Returns role data from the event-sourced state as owned values.
    ///
    /// Each entry is `(role_id, role_name, permissions)`, sorted by name.
    pub fn roles_data(&self) -> Vec<(String, String, Vec<String>)> {
        let shared = self.shared.borrow();
        let mut entries: Vec<(String, String, Vec<String>)> = shared
            .state
            .event_state
            .roles
            .values()
            .map(|role| {
                let perms: Vec<String> = role.permissions.iter().cloned().collect();
                (role.id.clone(), role.name.clone(), perms)
            })
            .collect();
        entries.sort_by(|a, b| a.1.cmp(&b.1));
        entries
    }

    /// Returns the PeerId of the server owner.
    pub fn server_owner(&self) -> String {
        self.shared.borrow().state.event_state.owner.clone()
    }

    /// Check whether a peer has a specific permission.
    pub fn has_permission(&self, peer_id: &str, perm: &willow_state::Permission) -> bool {
        self.shared
            .borrow()
            .state
            .event_state
            .has_permission(peer_id, perm)
    }

    /// Returns the current channel name from the chat state.
    pub fn current_channel(&self) -> String {
        self.shared.borrow().state.chat.current_channel.clone()
    }

    /// Returns unread counts keyed by channel name for the active server.
    pub fn unread_counts(&self) -> HashMap<String, usize> {
        let shared = self.shared.borrow();
        let mut unread_map = HashMap::new();
        if let Some(ctx) = shared.state.active() {
            for (topic, count) in &ctx.unread {
                if let Some(name) = ctx.name_for_topic(topic) {
                    unread_map.insert(name.to_string(), *count);
                }
            }
        }
        unread_map
    }

    // ---- Internal helpers ----

    /// Send chat content (text, reply) on a channel.
    fn send_content(
        &self,
        channel: &str,
        _content: Content,
        body: &str,
        _reply_preview: Option<String>,
        reply_to: Option<String>,
    ) -> anyhow::Result<()> {
        let mut shared = self.shared.borrow_mut();
        let ctx = shared
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        let topic = ctx
            .topic_for_name(channel)
            .unwrap_or_else(|| channel.to_string());

        let peer_id_str = shared.identity.peer_id().to_string();

        // Resolve channel_id from topic.
        let channel_id = shared
            .state
            .active()
            .and_then(|ctx| {
                ctx.topic_map
                    .get(&topic)
                    .map(|(_, ch_id)| ch_id.to_string())
            })
            .unwrap_or_else(|| channel.to_string());

        let ts = util::current_time_ms();
        let event_id = uuid::Uuid::new_v4().to_string();
        let msg_event = willow_state::Event {
            id: event_id.clone(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id_str.clone(),
            timestamp_ms: ts,
            kind: willow_state::EventKind::Message {
                channel_id,
                body: body.to_string(),
                reply_to,
            },
        };
        apply_event_on_shared(&mut shared, &msg_event);

        // Broadcast the event on the server ops topic (same as all other events).
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent {
                event: msg_event,
                topic: None,
            });

        // Dedup
        shared.state.chat.seen_message_ids.insert(event_id);

        Ok(())
    }
}

/// Convert a [`willow_state::Event`] into [`ClientEvent`]s for the caller.
///
/// This is a free function so it can be called with an already-borrowed
/// `SharedState` from `ClientEventLoop::process_batch()`.
fn emit_client_events_for(
    shared: &mut SharedState,
    event: &willow_state::Event,
    events: &mut Vec<ClientEvent>,
) {
    match &event.kind {
        willow_state::EventKind::Message { ref channel_id, .. } => {
            let is_local = event.author == shared.identity.peer_id().to_string();

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
            // NOTE: We can't send commands here since we don't have cmd_tx.
            // The subscription will be handled by the caller.
            if let Some(ctx) = shared.state.active_mut() {
                let topic = util::make_topic(&ctx.server, name);
                if !ctx.topic_map.contains_key(&topic) {
                    let cid = willow_channel::ChannelId(
                        uuid::Uuid::parse_str(channel_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
                    );
                    ctx.topic_map.insert(topic.clone(), (name.clone(), cid));
                    // The caller (process_batch) will handle subscribing via cmd_tx if needed.
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
            events.push(ClientEvent::MemberKicked(peer_id.clone()));
        }
        willow_state::EventKind::GrantPermission { peer_id, .. } => {
            events.push(ClientEvent::PeerTrusted(peer_id.clone()));
        }
        willow_state::EventKind::RevokePermission { peer_id, .. } => {
            events.push(ClientEvent::PeerUntrusted(peer_id.clone()));
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
                author: event.author.clone(),
            });
        }
        willow_state::EventKind::StateVerification { state_hash } => {
            let our_hash = shared.state.event_state.hash();
            shared
                .state_verification_results
                .insert(event.author.clone(), state_hash.clone());
            if *state_hash != our_hash {
                events.push(ClientEvent::StateHashMismatch {
                    peer_id: event.author.clone(),
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
    /// Run the event processing loop.
    ///
    /// Awaits network events, processes them, and sends [`ClientEvent`]s to the
    /// provided sender. Returns when the event channel closes.
    ///
    /// Profile re-broadcasts are scheduled at 3 s, 6 s, 10 s, and 20 s after
    /// the first event is received, using real async timers rather than tick
    /// counting.
    pub async fn run(mut self, tx: futures_mpsc::UnboundedSender<ClientEvent>) {
        use futures::FutureExt;
        use futures::StreamExt;

        type TimerFut = std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>;

        #[cfg(target_arch = "wasm32")]
        let make_timer = |ms: u32| -> TimerFut {
            Box::pin(async move {
                gloo_timers::future::TimeoutFuture::new(ms).await;
            })
        };
        #[cfg(not(target_arch = "wasm32"))]
        let make_timer = |ms: u32| -> TimerFut {
            Box::pin(async move {
                tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
            })
        };

        // Profile re-broadcast schedule: 3s, 6s, 10s, 20s after start.
        let profile_delays = [3000u32, 3000, 4000, 10000];
        let mut profile_idx = 0;
        let mut profile_timer = make_timer(profile_delays[0]).fuse();

        loop {
            futures::select! {
                net_event = self.event_rx.next() => {
                    let Some(net_event) = net_event else {
                        break; // Channel closed
                    };

                    // Drain additional ready events for batching.
                    let mut batch = vec![net_event];
                    while let Ok(more) = self.event_rx.try_recv() {
                        batch.push(more);
                    }

                    // Process the batch and check if we need to verify state.
                    let (client_events, needs_verify) = self.process_batch(batch);
                    for event in client_events {
                        if tx.unbounded_send(event).is_err() {
                            return; // Receiver dropped
                        }
                    }

                    // State verification must happen outside process_batch
                    // because it needs its own shared borrow.
                    if needs_verify {
                        self.verify_state_after_sync();
                    }
                }
                _ = profile_timer => {
                    self.broadcast_profile();
                    profile_idx += 1;
                    if profile_idx < profile_delays.len() {
                        profile_timer = make_timer(profile_delays[profile_idx]).fuse();
                    } else {
                        // No more scheduled re-broadcasts; wait forever.
                        let pending: TimerFut = Box::pin(futures::future::pending::<()>());
                        profile_timer = pending.fuse();
                    }
                }
                complete => break,
            }
        }
    }

    /// Process a batch of network events, returning the resulting client events
    /// and a flag indicating whether state verification should be triggered.
    fn process_batch(&self, net_events: Vec<network::NetworkEvent>) -> (Vec<ClientEvent>, bool) {
        let mut events = Vec::new();
        let mut needs_verify = false;

        for net_event in net_events {
            match net_event {
                // -- EventReceived --
                network::NetworkEvent::EventReceived { event, from } => {
                    tracing::info!(
                        kind = ?std::mem::discriminant(&event.kind),
                        from = %from,
                        event_id = %event.id,
                        "received event"
                    );

                    // Verify author matches signer.
                    if event.author != from {
                        tracing::warn!("event author mismatch: {} != {}", event.author, from);
                        continue;
                    }

                    let mut shared = self.shared.borrow_mut();
                    // Apply to event-sourced state.
                    let result = willow_state::apply_lenient(&mut shared.state.event_state, &event);
                    if matches!(result, willow_state::ApplyResult::Applied) {
                        shared.state.event_store.append(event.clone());
                        let hash = shared.state.event_state.hash();
                        shared.state.event_store.set_latest_hash(hash);

                        // Persist full state.
                        if shared.config.persistence {
                            if let Some(sid) = &shared.state.active_server {
                                storage::save_server_state(sid, &shared.state.event_state);
                            }
                        }

                        // Emit ClientEvents based on event kind.
                        emit_client_events_for(&mut shared, &event, &mut events);
                    }
                }

                // -- SyncRequested --
                network::NetworkEvent::SyncRequested {
                    state_hash,
                    from,
                    topic,
                } => {
                    tracing::info!(%from, ?topic, "sync requested");
                    let shared = self.shared.borrow();
                    let missing = shared.state.event_store.events_since(&state_hash);
                    if !missing.is_empty() {
                        let count = missing.len();
                        tracing::info!(count, "sending event sync batch");
                        let _ =
                            self.cmd_tx
                                .unbounded_send(network::NetworkCommand::SendSyncBatch {
                                    events: missing,
                                });
                    }
                }

                // -- SyncBatchReceived --
                network::NetworkEvent::SyncBatchReceived {
                    events: batch_events,
                    from,
                } => {
                    let mut shared = self.shared.borrow_mut();
                    tracing::info!(count = batch_events.len(), %from, "received event sync batch");
                    // Track the sender as online.
                    if !from.is_empty() && !shared.state.chat.peers.contains(&from) {
                        shared.state.chat.peers.push(from.clone());
                        events.push(ClientEvent::PeerConnected(from.clone()));
                    }
                    let mut sorted = batch_events;
                    sorted.sort_by_key(|e| e.timestamp_ms);
                    let count = sorted.len();
                    for event in &sorted {
                        let result =
                            willow_state::apply_lenient(&mut shared.state.event_state, event);
                        if matches!(result, willow_state::ApplyResult::Applied) {
                            shared.state.event_store.append(event.clone());
                            let hash = shared.state.event_state.hash();
                            shared.state.event_store.set_latest_hash(hash);

                            emit_client_events_for(&mut shared, event, &mut events);
                        }
                    }
                    if count > 0 {
                        // Persist full state after sync batch.
                        if shared.config.persistence {
                            if let Some(sid) = &shared.state.active_server {
                                storage::save_server_state(sid, &shared.state.event_state);
                            }
                        }
                        // Reconcile topic_map with synced event_state channels.
                        reconcile_topic_map(&mut shared.state);
                        events.push(ClientEvent::SyncCompleted { ops_applied: count });
                        // Signal that state verification is needed (done outside borrow).
                        needs_verify = true;
                    }
                }

                // -- Common events --
                network::NetworkEvent::PeerConnected(peer) => {
                    let mut shared = self.shared.borrow_mut();
                    if !shared.state.chat.peers.contains(&peer) {
                        shared.state.chat.peers.push(peer.clone());
                    }
                    // On first peer connect, subscribe to channels.
                    if !shared.connected_subscribed {
                        on_connected(&shared.state, &self.cmd_tx);
                        shared.connected_subscribed = true;
                    } else {
                        // Re-broadcast profile so the new peer learns our name.
                        let saved = storage::load_profile().unwrap_or_default();
                        if !saved.display_name.is_empty() {
                            let _ = self.cmd_tx.unbounded_send(
                                network::NetworkCommand::BroadcastProfile {
                                    display_name: saved.display_name,
                                },
                            );
                        }
                    }
                    events.push(ClientEvent::PeerConnected(peer));
                }
                network::NetworkEvent::PeerDisconnected(peer) => {
                    let mut shared = self.shared.borrow_mut();
                    shared.state.chat.peers.retain(|p| p != &peer);
                    events.push(ClientEvent::PeerDisconnected(peer));
                }
                network::NetworkEvent::ProfileReceived {
                    peer_id,
                    display_name,
                } => {
                    let mut shared = self.shared.borrow_mut();
                    shared
                        .state
                        .profiles
                        .names
                        .insert(peer_id.clone(), display_name.clone());
                    events.push(ClientEvent::ProfileUpdated {
                        peer_id,
                        display_name,
                    });
                }
                network::NetworkEvent::Listening(addr) => {
                    let mut shared = self.shared.borrow_mut();
                    // Reset subscription flag on reconnect so on_connected re-runs.
                    if addr == "reconnecting" {
                        shared.connected_subscribed = false;
                    }
                    if !shared.connected_subscribed {
                        on_connected(&shared.state, &self.cmd_tx);
                        shared.connected_subscribed = true;
                    }
                    events.push(ClientEvent::Listening(addr));
                }
                network::NetworkEvent::FileAnnounced {
                    filename,
                    size,
                    from,
                    topic,
                    ..
                } => {
                    let shared = self.shared.borrow();
                    let channel = shared
                        .state
                        .active()
                        .and_then(|ctx| ctx.name_for_topic(&topic))
                        .unwrap_or("unknown")
                        .to_string();
                    events.push(ClientEvent::FileAnnounced {
                        channel,
                        filename,
                        size,
                        from,
                    });
                }
                network::NetworkEvent::FileDownloaded { .. } => {
                    // no-op for now
                }
                network::NetworkEvent::TypingReceived { peer_id, channel } => {
                    let mut shared = self.shared.borrow_mut();
                    let now = util::current_time_ms();
                    shared.typing_peers.insert(peer_id.clone(), (channel, now));
                    // Also track as online peer.
                    if !shared.state.chat.peers.contains(&peer_id) {
                        shared.state.chat.peers.push(peer_id.clone());
                        events.push(ClientEvent::PeerConnected(peer_id));
                    }
                }
                network::NetworkEvent::MessageReceived { .. } => {
                    // All messages now go through EventReceived.
                }
                network::NetworkEvent::VoiceJoinReceived {
                    channel_id,
                    peer_id,
                } => {
                    let mut shared = self.shared.borrow_mut();
                    shared
                        .voice_participants
                        .entry(channel_id.clone())
                        .or_default()
                        .insert(peer_id.clone());
                    events.push(ClientEvent::VoiceJoined {
                        channel_id,
                        peer_id,
                    });
                }
                network::NetworkEvent::VoiceLeaveReceived {
                    channel_id,
                    peer_id,
                } => {
                    let mut shared = self.shared.borrow_mut();
                    if let Some(participants) = shared.voice_participants.get_mut(&channel_id) {
                        participants.remove(&peer_id);
                    }
                    events.push(ClientEvent::VoiceLeft {
                        channel_id,
                        peer_id,
                    });
                }
                network::NetworkEvent::VoiceSignalReceived {
                    channel_id,
                    from_peer,
                    signal,
                } => {
                    events.push(ClientEvent::VoiceSignal {
                        channel_id,
                        from_peer,
                        signal,
                    });
                }
                network::NetworkEvent::JoinLinkRequested { link_id, peer_id } => {
                    let mut shared = self.shared.borrow_mut();
                    let link = shared.join_links.iter_mut().find(|l| l.link_id == link_id);
                    match link {
                        Some(link) if link.is_valid() => {
                            link.used += 1;
                            tracing::info!(%link_id, used = link.used, max = link.max_uses, %peer_id, "processing join link request");
                            if shared.config.persistence {
                                storage::save_join_links(
                                    shared.state.active_server.as_deref().unwrap_or(""),
                                    &shared.join_links,
                                );
                            }
                            let identity = shared.identity.clone();
                            drop(shared);
                            let handle = ClientHandle {
                                shared: Rc::clone(&self.shared),
                                cmd_tx: self.cmd_tx.clone(),
                                deferred_channels: None,
                            };
                            match handle.generate_invite(&peer_id) {
                                Ok(invite_data) => {
                                    let msg = ops::WireMessage::JoinResponse {
                                        target_peer: peer_id,
                                        invite_data,
                                    };
                                    if let Some(data) = ops::pack_wire(&msg, &identity) {
                                        let _ = self.cmd_tx.unbounded_send(
                                            network::NetworkCommand::Publish {
                                                topic: ops::SERVER_OPS_TOPIC.to_string(),
                                                data,
                                            },
                                        );
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(%e, "failed to generate invite for join link")
                                }
                            }
                        }
                        Some(_) => {
                            let shared = self.shared.borrow();
                            let msg = ops::WireMessage::JoinDenied {
                                target_peer: peer_id,
                                reason: "link_expired".to_string(),
                            };
                            if let Some(data) = ops::pack_wire(&msg, &shared.identity) {
                                let _ =
                                    self.cmd_tx
                                        .unbounded_send(network::NetworkCommand::Publish {
                                            topic: ops::SERVER_OPS_TOPIC.to_string(),
                                            data,
                                        });
                            }
                        }
                        None => {}
                    }
                }
                network::NetworkEvent::JoinLinkResponseReceived { invite_data } => {
                    events.push(ClientEvent::JoinLinkResponse { invite_data });
                }
                network::NetworkEvent::JoinLinkDenied { reason } => {
                    events.push(ClientEvent::JoinLinkDenied { reason });
                }
            }
        }

        (events, needs_verify)
    }

    /// Broadcast our profile to the network if connected and profile is set.
    fn broadcast_profile(&self) {
        let shared = self.shared.borrow();
        if !shared.connected_subscribed {
            return;
        }
        let saved = storage::load_profile().unwrap_or_default();
        if !saved.display_name.is_empty() {
            let _ = self
                .cmd_tx
                .unbounded_send(network::NetworkCommand::BroadcastProfile {
                    display_name: saved.display_name,
                });
        }
    }

    /// Trigger state verification after a sync batch completes.
    ///
    /// This is separated from [`process_batch`] because it needs its own
    /// mutable borrow of shared state, which cannot overlap with the borrow
    /// inside `process_batch`.
    fn verify_state_after_sync(&self) {
        let mut shared = self.shared.borrow_mut();
        let author = shared.identity.peer_id().to_string();
        let state_hash = shared.state.event_state.hash();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::StateVerification { state_hash },
        };
        apply_event_on_shared(&mut shared, &event);
        let _ = self
            .cmd_tx
            .unbounded_send(network::NetworkCommand::BroadcastEvent { event, topic: None });
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

/// Get a peer's display name from a borrowed SharedState.
fn peer_display_name_shared(shared: &SharedState, peer_id: &str) -> String {
    if let Some(profile) = shared.state.event_state.profiles.get(peer_id) {
        return profile.display_name.clone();
    }
    shared.state.profiles.display_name(peer_id)
}

// ---- Identity persistence ----

fn load_identity() -> Identity {
    if let Some(bytes) = storage::load_identity_bytes() {
        if let Some(id) = Identity::from_ed25519_bytes(&bytes) {
            return id;
        }
    }

    let identity = Identity::generate();
    if let Some(bytes) = identity.to_ed25519_bytes() {
        storage::save_identity_bytes(&bytes);
    }
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
/// The returned client has mpsc channels wired up but no background task.
#[cfg(test)]
pub(crate) fn test_client() -> (
    ClientHandle,
    futures_mpsc::UnboundedReceiver<network::NetworkCommand>,
) {
    let identity = Identity::generate();
    let (cmd_tx, cmd_rx) = futures_mpsc::unbounded();
    let (_event_tx, event_rx) = futures_mpsc::unbounded::<network::NetworkEvent>();

    let mut state = ClientState::default();

    // Create a minimal server.
    let mut server = willow_channel::Server::new("Test Server", identity.peer_id());
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
        willow_state::ServerState::new(&server_id, "Test Server", identity.peer_id().to_string());

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

    let shared = Rc::new(RefCell::new(shared_state));

    drop(event_rx); // Consumed by ClientEventLoop in production; not needed in tests.
    let client = ClientHandle {
        shared,
        cmd_tx,
        deferred_channels: None,
    };

    (client, cmd_rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_new_has_default_channels() {
        let (client, _rx) = test_client();
        let names = client.channels();
        assert!(names.contains(&"general".to_string()));
    }

    #[test]
    fn client_peer_id_is_stable() {
        let (client, _rx) = test_client();
        let id1 = client.peer_id();
        let id2 = client.peer_id();
        assert_eq!(id1, id2);
        assert!(!id1.is_empty());
    }

    #[test]
    fn send_message_adds_to_state() {
        let (client, _rx) = test_client();
        client.send_message("general", "hello").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "hello");
        assert!(msgs[0].is_local);
    }

    #[test]
    fn send_message_broadcasts_event() {
        let (client, mut rx) = test_client();
        client.send_message("general", "test").unwrap();

        // Should broadcast a BroadcastEvent command.
        let cmd1 = rx.try_recv().unwrap();
        assert!(
            matches!(cmd1, network::NetworkCommand::BroadcastEvent { .. }),
            "expected BroadcastEvent, got {:?}",
            std::mem::discriminant(&cmd1),
        );
    }

    #[test]
    fn send_reply_has_preview() {
        let (client, _rx) = test_client();
        client.send_message("general", "original").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.send_reply("general", &msg_id, "reply").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].body, "reply");
        assert!(msgs[1].reply_preview.is_some());
    }

    #[test]
    fn edit_message_updates_state() {
        let (client, _rx) = test_client();
        client.send_message("general", "original").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.edit_message("general", &msg_id, "edited").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs[0].body, "edited");
        assert!(msgs[0].edited);
    }

    #[test]
    fn delete_message_marks_deleted() {
        let (client, _rx) = test_client();
        client.send_message("general", "to delete").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.delete_message("general", &msg_id).unwrap();

        let msgs = client.messages("general");
        assert!(msgs[0].deleted);
        assert_eq!(msgs[0].body, "[message deleted]");
    }

    #[test]
    fn react_adds_reaction() {
        let (client, _rx) = test_client();
        client.send_message("general", "react to me").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.react("general", &msg_id, "thumbsup").unwrap();

        let msgs = client.messages("general");
        assert!(msgs[0].reactions.contains_key("thumbsup"));
    }

    #[test]
    fn create_channel_adds_to_server() {
        let (client, _rx) = test_client();
        client.create_channel("new-channel").unwrap();

        let names = client.channels();
        assert!(names.contains(&"new-channel".to_string()));
        assert_eq!(
            client.shared.borrow().state.chat.current_channel,
            "new-channel"
        );
    }

    #[test]
    fn delete_channel_removes_from_server() {
        let (client, _rx) = test_client();
        client.create_channel("temp").unwrap();
        assert!(client.channels().contains(&"temp".to_string()));

        client.delete_channel("temp").unwrap();
        assert!(!client.channels().contains(&"temp".to_string()));
    }

    #[test]
    fn switch_channel_updates_current() {
        let (client, _rx) = test_client();
        client.create_channel("other").unwrap();
        client.switch_channel("general");

        assert_eq!(client.shared.borrow().state.chat.current_channel, "general");
    }

    #[test]
    fn trust_untrust_broadcasts_events() {
        let (client, mut rx) = test_client();
        client.trust_peer("some-peer");

        // Should broadcast BroadcastEvent.
        let cmd1 = rx.try_recv().unwrap();
        assert!(matches!(
            cmd1,
            network::NetworkCommand::BroadcastEvent { .. }
        ));

        client.untrust_peer("some-peer");
        // Should broadcast BroadcastEvent.
        let cmd2 = rx.try_recv().unwrap();
        assert!(matches!(
            cmd2,
            network::NetworkCommand::BroadcastEvent { .. }
        ));
    }

    #[test]
    fn display_name_default_truncated() {
        let (client, _rx) = test_client();
        let name = client.display_name();
        // Default name should be truncated peer ID.
        assert!(!name.is_empty());
    }

    #[test]
    fn set_display_name_updates_profile() {
        let (client, mut rx) = test_client();
        client.set_display_name("Alice");

        assert_eq!(client.display_name(), "Alice");

        // Should broadcast profile.
        let cmd = rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            network::NetworkCommand::BroadcastProfile { .. }
        ));
    }

    #[test]
    fn messages_filters_by_channel() {
        let (client, _rx) = test_client();
        client.send_message("general", "msg1").unwrap();
        client.create_channel("other").unwrap();
        client.send_message("other", "msg2").unwrap();

        assert_eq!(client.messages("general").len(), 1);
        assert_eq!(client.messages("other").len(), 1);
        assert_eq!(client.messages("general")[0].body, "msg1");
        assert_eq!(client.messages("other")[0].body, "msg2");
    }

    #[test]
    fn generate_accept_invite_round_trip() {
        let (client, _rx) = test_client();
        let recipient = Identity::generate();
        let recipient_peer_id = recipient.peer_id().to_string();

        let code = client.generate_invite(&recipient_peer_id).unwrap();
        assert!(!code.is_empty());

        // Verify recipient can accept.
        let accepted = invite::accept_invite(&code, &recipient).unwrap();
        assert!(!accepted.channel_keys.is_empty());
    }

    // ---- Multi-server tests ----

    #[test]
    fn test_switch_server() {
        let (client, _rx) = test_client();

        // Create a second server context.
        let server2 =
            willow_channel::Server::new("Second Server", client.shared.borrow().identity.peer_id());
        let server2_id = server2.id.to_string();
        let ctx2 = ServerContext {
            server: server2,
            topic_map: HashMap::new(),
            keys: HashMap::new(),
            unread: HashMap::new(),
        };
        client
            .shared
            .borrow_mut()
            .state
            .servers
            .insert(server2_id.clone(), ctx2);

        let original_id = client.shared.borrow().state.active_server.clone().unwrap();
        assert_ne!(original_id, server2_id);

        client.switch_server(&server2_id);
        assert_eq!(
            client.shared.borrow().state.active_server.as_deref(),
            Some(server2_id.as_str())
        );

        // Switch back.
        client.switch_server(&original_id);
        assert_eq!(
            client.shared.borrow().state.active_server.as_deref(),
            Some(original_id.as_str())
        );

        // Switch to non-existent server does nothing.
        client.switch_server("non-existent");
        assert_eq!(
            client.shared.borrow().state.active_server.as_deref(),
            Some(original_id.as_str())
        );
    }

    #[test]
    fn test_accept_invite_creates_new_server() {
        let (client, _rx) = test_client();
        let initial_count = client.shared.borrow().state.servers.len();
        assert_eq!(initial_count, 1);

        // Create a second identity (the "owner" of the other server).
        let owner = Identity::generate();
        let mut owner_server = willow_channel::Server::new("Other Server", owner.peer_id());
        let ch_id = owner_server
            .create_channel("lobby", willow_channel::ChannelKind::Text)
            .unwrap();

        let mut keys = HashMap::new();
        let mut topic_map = HashMap::new();
        let topic = format!("{}/lobby", owner_server.id);
        if let Some(key) = owner_server.channel_key(&ch_id) {
            keys.insert(topic.clone(), key.clone());
        }
        topic_map.insert(topic, ("lobby".into(), ch_id));

        // Generate invite for our client.
        let our_pub = {
            let shared = client.shared.borrow();
            let ed_kp = shared
                .identity
                .keypair()
                .clone()
                .try_into_ed25519()
                .unwrap();
            let full = ed_kp.to_bytes();
            let mut pub_bytes = [0u8; 32];
            pub_bytes.copy_from_slice(&full[32..]);
            pub_bytes
        };
        let code = invite::generate_invite(&owner_server, &keys, &topic_map, &our_pub).unwrap();

        // Accept the invite.
        client.accept_invite(&code).unwrap();

        // Should now have 2 servers.
        assert_eq!(client.shared.borrow().state.servers.len(), 2);

        // Active server should be the new one.
        let shared = client.shared.borrow();
        let active_id = shared.state.active_server.clone().unwrap();
        let new_ctx = shared.state.servers.get(&active_id).unwrap();
        assert!(!new_ctx.keys.is_empty());
    }

    #[test]
    fn invite_joiner_event_state_has_correct_owner() {
        let (client, _rx) = test_client();

        // Create a second identity (the "owner" of the other server).
        let owner = Identity::generate();
        let owner_peer_id = owner.peer_id().to_string();
        let mut owner_server = willow_channel::Server::new("Other Server", owner.peer_id());
        let ch_id = owner_server
            .create_channel("lobby", willow_channel::ChannelKind::Text)
            .unwrap();

        let mut keys = HashMap::new();
        let mut topic_map = HashMap::new();
        let topic = format!("{}/lobby", owner_server.id);
        if let Some(key) = owner_server.channel_key(&ch_id) {
            keys.insert(topic.clone(), key.clone());
        }
        topic_map.insert(topic, ("lobby".into(), ch_id));

        let our_pub = {
            let shared = client.shared.borrow();
            let ed_kp = shared
                .identity
                .keypair()
                .clone()
                .try_into_ed25519()
                .unwrap();
            let full = ed_kp.to_bytes();
            let mut pub_bytes = [0u8; 32];
            pub_bytes.copy_from_slice(&full[32..]);
            pub_bytes
        };
        let code = invite::generate_invite(&owner_server, &keys, &topic_map, &our_pub).unwrap();

        client.accept_invite(&code).unwrap();

        // The event_state owner should be the ACTUAL owner, not the joiner.
        assert_eq!(
            client.shared.borrow().state.event_state.owner,
            owner_peer_id,
            "event_state.owner should be the server owner, not the joining peer"
        );
    }

    #[test]
    fn invite_joiner_can_see_owner_messages() {
        let (client, _rx) = test_client();

        // Create a second identity (the "owner" of the other server).
        let owner = Identity::generate();
        let owner_peer_id = owner.peer_id().to_string();
        let mut owner_server = willow_channel::Server::new("Other Server", owner.peer_id());
        let ch_id = owner_server
            .create_channel("lobby", willow_channel::ChannelKind::Text)
            .unwrap();

        let mut keys = HashMap::new();
        let mut topic_map = HashMap::new();
        let topic = format!("{}/lobby", owner_server.id);
        let ch_id_str = ch_id.to_string();
        if let Some(key) = owner_server.channel_key(&ch_id) {
            keys.insert(topic.clone(), key.clone());
        }
        topic_map.insert(topic, ("lobby".into(), ch_id));

        let our_pub = {
            let shared = client.shared.borrow();
            let ed_kp = shared
                .identity
                .keypair()
                .clone()
                .try_into_ed25519()
                .unwrap();
            let full = ed_kp.to_bytes();
            let mut pub_bytes = [0u8; 32];
            pub_bytes.copy_from_slice(&full[32..]);
            pub_bytes
        };
        let code = invite::generate_invite(&owner_server, &keys, &topic_map, &our_pub).unwrap();

        client.accept_invite(&code).unwrap();

        // Simulate sync: apply the owner's CreateChannel event first.
        let create_ch_event = willow_state::Event {
            id: "create-lobby".to_string(),
            parent_hash: willow_state::StateHash::ZERO,
            author: owner_peer_id.clone(),
            timestamp_ms: 500,
            kind: willow_state::EventKind::CreateChannel {
                name: "lobby".to_string(),
                channel_id: ch_id_str.clone(),
                kind: "text".to_string(),
            },
        };
        willow_state::apply_lenient(
            &mut client.shared.borrow_mut().state.event_state,
            &create_ch_event,
        );

        // Simulate receiving a message from the owner.
        let msg_event = willow_state::Event {
            id: "msg-from-owner".to_string(),
            parent_hash: willow_state::StateHash::ZERO,
            author: owner_peer_id.clone(),
            timestamp_ms: 1000,
            kind: willow_state::EventKind::Message {
                channel_id: ch_id_str.clone(),
                body: "hello from owner".to_string(),
                reply_to: None,
            },
        };
        willow_state::apply_lenient(
            &mut client.shared.borrow_mut().state.event_state,
            &msg_event,
        );

        // The joiner should be able to see this message via messages("lobby").
        let msgs = client.messages("lobby");
        assert_eq!(
            msgs.len(),
            1,
            "joiner should see the owner's message -- channel_id must match"
        );
        assert_eq!(msgs[0].body, "hello from owner");
    }

    #[test]
    fn test_messages_filtered_by_server() {
        let (client, _rx) = test_client();

        // Send a message on server 1.
        client.send_message("general", "server1 msg").unwrap();

        let server1_id = client.shared.borrow().state.active_server.clone().unwrap();

        // When viewing server 1, see server 1 messages.
        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "server1 msg");

        // Create a second server and switch to it.
        let server2_id = client.create_server("Server 2").unwrap();
        assert_ne!(server1_id, server2_id);

        // Server 2 should have no messages yet.
        let msgs = client.messages("general");
        assert!(msgs.is_empty());

        // Switch back to server 1. Without persistence enabled, event_state
        // is re-initialized empty so in-memory messages from the session are lost.
        // This is expected behavior when persistence = false.
        client.switch_server(&server1_id);
        assert_eq!(client.active_server_id(), Some(server1_id));
    }

    #[test]
    fn test_server_list() {
        let (client, _rx) = test_client();

        let list = client.server_list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].1, "Test Server");

        // Add a second server.
        let server2 =
            willow_channel::Server::new("Second", client.shared.borrow().identity.peer_id());
        let server2_id = server2.id.to_string();
        client.shared.borrow_mut().state.servers.insert(
            server2_id,
            ServerContext {
                server: server2,
                topic_map: HashMap::new(),
                keys: HashMap::new(),
                unread: HashMap::new(),
            },
        );

        let list = client.server_list();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_active_server_name() {
        let (client, _rx) = test_client();
        assert_eq!(client.active_server_name(), "Test Server");
    }

    // ---- Multi-peer scenario tests ----

    #[test]
    fn client_create_multiple_channels_and_verify() {
        let (client, _rx) = test_client();

        // Create 5 channels via client API.
        let names = ["alpha", "beta", "gamma", "delta", "epsilon"];
        for name in &names {
            client.create_channel(name).unwrap();
        }

        // Verify all appear in channels() list.
        let channels = client.channels();
        for name in &names {
            assert!(
                channels.contains(&name.to_string()),
                "channel '{}' should be in channels list",
                name
            );
        }

        // Verify event_state has all 5 channels.
        let shared = client.shared.borrow();
        let event_channels: Vec<String> = shared
            .state
            .event_state
            .channels
            .values()
            .map(|c| c.name.clone())
            .collect();
        for name in &names {
            assert!(
                event_channels.contains(&name.to_string()),
                "channel '{}' should be in event_state.channels",
                name
            );
        }
    }

    #[test]
    fn client_send_messages_to_different_channels() {
        let (client, _rx) = test_client();

        // Create 3 channels.
        let ch_names = ["dev", "design", "random"];
        for name in &ch_names {
            client.create_channel(name).unwrap();
        }

        // Send 2 messages to each channel.
        for name in &ch_names {
            client.send_message(name, &format!("{name} msg 1")).unwrap();
            client.send_message(name, &format!("{name} msg 2")).unwrap();
        }

        // Verify messages() returns correct messages per channel.
        for name in &ch_names {
            let msgs = client.messages(name);
            assert_eq!(
                msgs.len(),
                2,
                "channel '{}' should have 2 messages, got {}",
                name,
                msgs.len()
            );
            assert_eq!(msgs[0].body, format!("{name} msg 1"));
            assert_eq!(msgs[1].body, format!("{name} msg 2"));
        }

        // Verify event_state.messages has all 6 messages.
        assert_eq!(
            client.shared.borrow().state.event_state.messages.len(),
            6,
            "event_state should have 6 messages total"
        );
    }

    #[test]
    fn client_trust_and_permission_flow() {
        let (client, _rx) = test_client();

        // Trust a peer.
        client.trust_peer("some-peer");

        // Verify they appear in event_state.peer_permissions with Administrator.
        let shared = client.shared.borrow();
        assert!(
            shared
                .state
                .event_state
                .has_permission("some-peer", &willow_state::Permission::Administrator),
            "trusted peer should have Administrator permission"
        );
        assert!(
            shared.state.event_state.members.contains_key("some-peer"),
            "trusted peer should be a member"
        );
        drop(shared);

        // Untrust them.
        client.untrust_peer("some-peer");

        // Verify Administrator permission removed.
        let shared = client.shared.borrow();
        assert!(
            !shared
                .state
                .event_state
                .has_permission("some-peer", &willow_state::Permission::Administrator),
            "untrusted peer should not have Administrator permission"
        );
    }

    #[test]
    fn client_event_store_persists_events() {
        use willow_state::EventStore as _;

        let (client, _rx) = test_client();

        // Perform several actions.
        client.create_channel("test-channel").unwrap();
        client.send_message("test-channel", "hello").unwrap();
        client.trust_peer("peer-x");

        // Check event_store has the corresponding events.
        let shared = client.shared.borrow();
        let events = shared.state.event_store.all_events();
        assert!(
            events.len() >= 3,
            "event store should have at least 3 events, got {}",
            events.len()
        );
    }

    #[test]
    fn client_channels_from_event_state() {
        let (client, _rx) = test_client();

        // Create channels via client.
        client.create_channel("forum").unwrap();
        client.create_channel("help").unwrap();

        // Verify channels() returns names from event_state.
        let channels = client.channels();
        assert!(
            channels.contains(&"forum".to_string()),
            "should contain 'forum'"
        );
        assert!(
            channels.contains(&"help".to_string()),
            "should contain 'help'"
        );

        // Also verify the event_state has them.
        let shared = client.shared.borrow();
        let es_names: Vec<String> = shared
            .state
            .event_state
            .channels
            .values()
            .map(|c| c.name.clone())
            .collect();
        assert!(es_names.contains(&"forum".to_string()));
        assert!(es_names.contains(&"help".to_string()));
    }

    #[test]
    fn client_display_name_from_event_state() {
        let (client, _rx) = test_client();

        // Set display name via SetProfile event on the event_state.
        let peer_id = client.peer_id();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: client.shared.borrow().state.event_state.hash(),
            author: peer_id.clone(),
            timestamp_ms: 1000,
            kind: willow_state::EventKind::SetProfile {
                display_name: "EventAlice".into(),
            },
        };
        willow_state::apply_lenient(&mut client.shared.borrow_mut().state.event_state, &event);

        // Verify display_name() reads from event_state.profiles.
        assert_eq!(
            client.display_name(),
            "EventAlice",
            "display_name should come from event_state profile"
        );
    }

    // ---- State verification tests ----

    #[test]
    fn verify_state_broadcasts_event() {
        let (client, mut rx) = test_client();
        client.verify_state().unwrap();

        // Should broadcast a BroadcastEvent command.
        let cmd = rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            network::NetworkCommand::BroadcastEvent { .. }
        ));
    }

    #[test]
    fn state_hash_agreement_empty_initially() {
        let (client, _rx) = test_client();
        let (agreeing, total) = client.state_hash_agreement();
        assert_eq!(agreeing, 0);
        assert_eq!(total, 0);
    }

    #[test]
    fn state_hash_agreement_tracks_matching_peer() {
        let (client, _rx) = test_client();

        // Simulate a remote peer's StateVerification event with a matching hash.
        let our_hash = client.shared.borrow().state.event_state.hash();
        client
            .shared
            .borrow_mut()
            .state_verification_results
            .insert("peer-a".to_string(), our_hash);

        let (agreeing, total) = client.state_hash_agreement();
        assert_eq!(agreeing, 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn state_hash_agreement_tracks_mismatched_peer() {
        let (client, _rx) = test_client();

        // Insert a different hash for a peer.
        let wrong_hash = willow_state::StateHash::from_bytes(b"wrong");
        client
            .shared
            .borrow_mut()
            .state_verification_results
            .insert("peer-b".to_string(), wrong_hash);

        let (agreeing, total) = client.state_hash_agreement();
        assert_eq!(agreeing, 0);
        assert_eq!(total, 1);
    }

    #[test]
    fn state_hash_agreement_mixed() {
        let (client, _rx) = test_client();
        let our_hash = client.shared.borrow().state.event_state.hash();

        // One matching, one mismatched.
        client
            .shared
            .borrow_mut()
            .state_verification_results
            .insert("peer-a".to_string(), our_hash);
        client
            .shared
            .borrow_mut()
            .state_verification_results
            .insert(
                "peer-b".to_string(),
                willow_state::StateHash::from_bytes(b"different"),
            );

        let (agreeing, total) = client.state_hash_agreement();
        assert_eq!(agreeing, 1);
        assert_eq!(total, 2);
    }

    // ---- Server rename/description tests ----

    #[test]
    fn rename_server_updates_event_state() {
        let (client, _rx) = test_client();
        client.rename_server("New Server Name").unwrap();
        assert_eq!(
            client.shared.borrow().state.event_state.server_name,
            "New Server Name"
        );
    }

    #[test]
    fn rename_server_broadcasts_event() {
        let (client, mut rx) = test_client();
        client.rename_server("Another Name").unwrap();

        let cmd = rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            network::NetworkCommand::BroadcastEvent { .. }
        ));
    }

    #[test]
    fn set_server_description_updates_event_state() {
        let (client, _rx) = test_client();
        client.set_server_description("A cool server").unwrap();
        assert_eq!(
            client.shared.borrow().state.event_state.description,
            "A cool server"
        );
    }

    #[test]
    fn set_server_description_broadcasts_event() {
        let (client, mut rx) = test_client();
        client.set_server_description("Hello world").unwrap();

        let cmd = rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            network::NetworkCommand::BroadcastEvent { .. }
        ));
    }

    // ---- No-server and create_server tests ----

    #[test]
    fn has_servers_returns_true_for_test_client() {
        let (client, _rx) = test_client();
        assert!(client.has_servers());
    }

    #[test]
    fn no_servers_returns_empty_channels() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
            shared.state.event_state.channels.clear();
        }

        let channels = client.channels();
        assert!(channels.is_empty());
    }

    #[test]
    fn no_servers_returns_empty_messages() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        let msgs = client.messages("general");
        assert!(msgs.is_empty());
    }

    #[test]
    fn no_servers_active_server_name_returns_no_server() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        assert_eq!(client.active_server_name(), "No Server");
    }

    #[test]
    fn no_servers_send_message_returns_error() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        let result = client.send_message("general", "hello");
        assert!(result.is_err());
    }

    #[test]
    fn no_servers_create_channel_returns_error() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        let result = client.create_channel("test");
        assert!(result.is_err());
    }

    #[test]
    fn create_server_adds_server() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }
        assert!(!client.has_servers());

        let server_id = client.create_server("My New Server").unwrap();
        assert!(client.has_servers());
        assert_eq!(
            client.shared.borrow().state.active_server.as_deref(),
            Some(server_id.as_str())
        );
        assert_eq!(client.active_server_name(), "My New Server");
    }

    #[test]
    fn create_server_has_general_channel() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        client.create_server("Test").unwrap();
        let channels = client.channels();
        assert!(
            channels.contains(&"general".to_string()),
            "created server should have a 'general' channel"
        );
    }

    #[test]
    fn create_server_sets_current_channel() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        client.create_server("Test").unwrap();
        assert_eq!(client.shared.borrow().state.chat.current_channel, "general");
    }

    #[test]
    fn create_server_initializes_event_state() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        let server_id = client.create_server("Event Test").unwrap();
        let shared = client.shared.borrow();
        assert_eq!(shared.state.event_state.server_id, server_id);
        assert_eq!(shared.state.event_state.server_name, "Event Test");
        assert_eq!(
            shared.state.event_state.owner,
            shared.identity.peer_id().to_string()
        );

        // Event state should have the general channel.
        assert!(
            shared
                .state
                .event_state
                .channels
                .values()
                .any(|ch| ch.name == "general"),
            "event_state should have 'general' channel"
        );
    }

    #[test]
    fn create_server_subscribes_to_topic() {
        let (client, mut rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        client.create_server("Sub Test").unwrap();

        // Should have sent a Subscribe command for the general channel topic.
        let cmd = rx.try_recv().unwrap();
        assert!(
            matches!(cmd, network::NetworkCommand::Subscribe(_)),
            "expected Subscribe, got {:?}",
            std::mem::discriminant(&cmd),
        );
    }

    #[test]
    fn create_server_allows_sending_messages() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        client.create_server("Msg Test").unwrap();
        client.send_message("general", "hello new server").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "hello new server");
    }

    #[test]
    fn create_multiple_servers() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        let id1 = client.create_server("Server One").unwrap();
        let id2 = client.create_server("Server Two").unwrap();

        assert_eq!(client.shared.borrow().state.servers.len(), 2);
        assert_ne!(id1, id2);

        // Active server should be the last created.
        assert_eq!(
            client.shared.borrow().state.active_server.as_deref(),
            Some(id2.as_str())
        );
        assert_eq!(client.active_server_name(), "Server Two");

        // Can switch back to first server.
        client.switch_server(&id1);
        assert_eq!(client.active_server_name(), "Server One");
    }

    // ---- Per-server profile tests ----

    #[test]
    fn set_server_display_name_updates_event_state() {
        let (client, _rx) = test_client();
        client.set_server_display_name("ServerAlice").unwrap();

        assert_eq!(client.server_display_name(), "ServerAlice");
    }

    #[test]
    fn set_server_display_name_no_server_returns_error() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        let result = client.set_server_display_name("test");
        assert!(result.is_err());
    }

    #[test]
    fn server_display_name_falls_back_to_global() {
        let (client, _rx) = test_client();
        client.set_display_name("GlobalName");
        // No server profile set, so should fall back to global.
        assert_eq!(client.server_display_name(), "GlobalName");
    }

    // ---- Typing indicator tests ----

    #[test]
    fn send_typing_debounces() {
        let (client, _rx) = test_client();
        client.send_typing();
        client.send_typing();
    }

    #[test]
    fn typing_in_returns_empty_when_no_typers() {
        let (client, _rx) = test_client();
        let typers = client.typing_in("general");
        assert!(typers.is_empty());
    }

    // ---- Server members tests ----

    #[test]
    fn server_members_includes_owner() {
        let (client, _rx) = test_client();
        let members = client.server_members();
        assert!(!members.is_empty());
        let owner = members.iter().find(|(_, _, online)| *online);
        assert!(owner.is_some());
    }

    // ---- Message dedup tests ----

    #[test]
    fn seen_message_ids_prevents_duplicates() {
        let (client, _rx) = test_client();
        assert!(client
            .shared
            .borrow()
            .state
            .chat
            .seen_message_ids
            .is_empty());

        client.send_message("general", "test").unwrap();
        let _ = client.shared.borrow().state.chat.seen_message_ids.len();
    }

    // ---- Per-server display name isolation ----

    #[test]
    fn server_display_name_per_server() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        let _id1 = client.create_server("Server A").unwrap();
        let _ = client.set_server_display_name("Alice on A");
        assert_eq!(client.server_display_name(), "Alice on A");

        let _id2 = client.create_server("Server B").unwrap();
        let name = client.server_display_name();
        assert_eq!(name, "Alice on A");

        let _ = client.set_server_display_name("Alice on B");
        assert_eq!(client.server_display_name(), "Alice on B");
    }

    // ---- Server switching & event state isolation ----

    #[test]
    fn switch_server_updates_event_state() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        let id1 = client.create_server("Server A").unwrap();
        assert!(client
            .shared
            .borrow()
            .state
            .event_state
            .channels
            .values()
            .any(|c| c.name == "general"));

        let _id2 = client.create_server("Server B").unwrap();
        assert_eq!(
            client.shared.borrow().state.event_state.server_name,
            "Server B"
        );

        client.switch_server(&id1);
        assert_eq!(
            client.shared.borrow().state.event_state.server_name,
            "Server A"
        );
    }

    #[test]
    fn switch_server_isolates_legacy_channels() {
        let (client, _rx) = test_client();
        {
            let mut shared = client.shared.borrow_mut();
            shared.state.servers.clear();
            shared.state.active_server = None;
        }

        let id1 = client.create_server("Server A").unwrap();
        client.create_channel("alpha").unwrap();

        let _id2 = client.create_server("Server B").unwrap();
        client.create_channel("beta").unwrap();

        // On Server B, channel_names should include "beta" but not "alpha".
        let names_b = client.shared.borrow().state.channel_names();
        assert!(names_b.contains(&"general".to_string()));
        assert!(names_b.contains(&"beta".to_string()));
        assert!(!names_b.contains(&"alpha".to_string()));

        // Switch to Server A, channel_names should include "alpha" but not "beta".
        client.switch_server(&id1);
        let names_a = client.shared.borrow().state.channel_names();
        assert!(names_a.contains(&"general".to_string()));
        assert!(names_a.contains(&"alpha".to_string()));
        assert!(!names_a.contains(&"beta".to_string()));
    }

    // ---- DisplayMessage computation tests ----

    #[test]
    fn messages_returns_display_messages_with_resolved_names() {
        let (client, _rx) = test_client();
        client.send_message("general", "hello").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "hello");
        assert!(!msgs[0].author_display_name.is_empty());
        assert!(msgs[0].is_local);
        assert!(!msgs[0].edited);
        assert!(!msgs[0].deleted);
    }

    #[test]
    fn messages_returns_empty_for_unknown_channel() {
        let (client, _rx) = test_client();
        let msgs = client.messages("nonexistent");
        assert!(msgs.is_empty());
    }

    #[test]
    fn display_message_reply_has_parent_id() {
        let (client, _rx) = test_client();
        client.send_message("general", "parent message").unwrap();
        let parent_id = client.messages("general")[0].id.clone();

        client
            .send_reply("general", &parent_id, "reply body")
            .unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 2);
        let reply = &msgs[1];
        assert_eq!(reply.body, "reply body");
        assert_eq!(reply.reply_to, Some(parent_id));
        assert!(reply.reply_preview.is_some());
        assert!(reply
            .reply_preview
            .as_ref()
            .unwrap()
            .contains("parent message"));
    }

    #[test]
    fn display_message_is_local_false_for_remote() {
        let (client, _rx) = test_client();

        // Find the channel_id for "general" in event_state.
        let channel_id = client
            .shared
            .borrow()
            .state
            .event_state
            .channels
            .values()
            .find(|c| c.name == "general")
            .unwrap()
            .id
            .clone();

        // Manually insert a message from a "remote" peer into event_state.
        let remote_event = willow_state::Event {
            id: "remote-msg-1".to_string(),
            parent_hash: client.shared.borrow().state.event_state.hash(),
            author: "remote-peer-id".to_string(),
            timestamp_ms: 1000,
            kind: willow_state::EventKind::Message {
                channel_id,
                body: "from remote".to_string(),
                reply_to: None,
            },
        };
        willow_state::apply_lenient(
            &mut client.shared.borrow_mut().state.event_state,
            &remote_event,
        );

        let msgs = client.messages("general");
        let remote_msg = msgs.iter().find(|m| m.body == "from remote").unwrap();
        assert!(!remote_msg.is_local);
        assert_eq!(remote_msg.author_peer_id, "remote-peer-id");
    }

    #[test]
    fn messages_sorted_by_timestamp() {
        let (client, _rx) = test_client();
        client.send_message("general", "first").unwrap();
        client.send_message("general", "second").unwrap();
        client.send_message("general", "third").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 3);
        assert!(msgs[0].timestamp_ms <= msgs[1].timestamp_ms);
        assert!(msgs[1].timestamp_ms <= msgs[2].timestamp_ms);
    }

    // ---- Typing indicator timeout test ----

    #[test]
    fn typing_in_expires_after_timeout() {
        let (client, _rx) = test_client();

        // Insert a typing entry with timestamp 0 (very old, > 5 seconds ago).
        client
            .shared
            .borrow_mut()
            .typing_peers
            .insert("peer-1".to_string(), ("general".to_string(), 0));

        // Should be expired (> 5 seconds old).
        let typers = client.typing_in("general");
        assert!(typers.is_empty());
    }

    // ---- Pin persistence test ----

    #[test]
    fn pin_reflected_in_pinned_messages() {
        let (client, _rx) = test_client();
        client.send_message("general", "pin this").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.pin_message("general", &msg_id).unwrap();

        let pinned = client.pinned_messages("general");
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].id, msg_id);
        assert_eq!(pinned[0].body, "pin this");
    }

    #[test]
    fn create_join_link_returns_token_with_server_info() {
        let (client, _rx) = test_client();
        let token_str = client.create_join_link(5, None).unwrap();
        let token = crate::ops::JoinToken::decode(&token_str).unwrap();
        assert_eq!(token.inviter_peer_id, client.peer_id());
        assert!(!token.link_id.is_empty());
        assert!(!token.server_name.is_empty());
    }

    #[test]
    fn join_links_returns_created_links() {
        let (client, _rx) = test_client();
        assert!(client.join_links().is_empty());
        client.create_join_link(3, None).unwrap();
        let links = client.join_links();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].max_uses, 3);
        assert_eq!(links[0].used, 0);
        assert!(links[0].active);
    }

    #[test]
    fn delete_join_link_removes_it() {
        let (client, _rx) = test_client();
        client.create_join_link(5, None).unwrap();
        let link_id = client.join_links()[0].link_id.clone();
        client.delete_join_link(&link_id);
        assert!(client.join_links().is_empty());
    }

    #[test]
    fn authorize_workers_grants_sync_provider() {
        let (client, _rx) = test_client();
        client.create_server("Worker Test").unwrap();

        client.authorize_workers(&[
            "worker-peer-1".to_string(),
            "worker-peer-2".to_string(),
        ]);

        let shared = client.shared.borrow();
        assert!(shared
            .state
            .event_state
            .has_permission("worker-peer-1", &willow_state::Permission::SyncProvider));
        assert!(shared
            .state
            .event_state
            .has_permission("worker-peer-2", &willow_state::Permission::SyncProvider));
    }

    #[test]
    fn authorize_workers_empty_list_is_noop() {
        let (client, _rx) = test_client();
        client.create_server("No Workers").unwrap();

        let hash_before = client.shared.borrow().state.event_state.hash();
        client.authorize_workers(&[]);
        let hash_after = client.shared.borrow().state.event_state.hash();

        assert_eq!(hash_before, hash_after);
    }
}
