//! # Willow Client
//!
//! UI-agnostic client library for the Willow P2P chat network.
//! Use this crate to build bots, CLIs, TUIs, or alternative frontends.
//!
//! ## Quick start
//!
//! ```no_run
//! use willow_client::{Client, ClientConfig};
//!
//! let mut client = Client::new(ClientConfig::default());
//! client.connect();
//! loop {
//!     for event in client.poll() {
//!         // handle events
//!     }
//!     client.send_message("general", "hello!").ok();
//! }
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

// Re-export key types at crate root for convenience.
pub use events::{ClientEvent, ClientNotification};
pub use ops::{pack_wire, unpack_wire, WireMessage};
pub use state::{
    ChatMessage, ChatState, ClientState, PersistentEventStore, ProfileStore, ServerContext,
};

/// Re-export the event-sourced state crate for use by downstream consumers.
pub use willow_state;

use std::collections::HashMap;
use std::sync::mpsc as std_mpsc;

use willow_identity::Identity;
use willow_messaging::Content;
use willow_state::EventStore as _;

/// Configuration for creating a [`Client`].
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
    std_mpsc::Sender<network::NetworkEvent>,
    std_mpsc::Receiver<network::NetworkCommand>,
);

/// UI-agnostic P2P chat client.
///
/// Wraps identity, networking, and all chat state. Call [`Client::poll()`]
/// each frame (or in a loop) to drain network events and receive
/// [`ClientEvent`]s.
///
/// Optionally accepts a push notification channel via
/// [`Client::with_notifications`] for reactive UIs that prefer push over
/// poll.
pub struct Client {
    pub(crate) state: ClientState,
    pub(crate) identity: Identity,
    pub(crate) cmd_tx: std_mpsc::Sender<network::NetworkCommand>,
    pub(crate) event_rx: std_mpsc::Receiver<network::NetworkEvent>,
    pub(crate) connected: bool,
    pub(crate) config: ClientConfig,
    /// Holds the event_tx and cmd_rx until connect() consumes them.
    pub(crate) deferred_channels: Option<std::sync::Arc<std::sync::Mutex<Option<DeferredPair>>>>,
    /// Whether we have performed initial channel subscriptions.
    pub(crate) connected_subscribed: bool,
    /// Counter for throttled profile re-broadcasts.
    pub(crate) profile_broadcast_counter: u32,
    /// Optional push notification sender for reactive UIs.
    pub(crate) notification_tx: Option<std_mpsc::Sender<events::ClientNotification>>,
    /// Tracks what state hash each peer has reported via StateVerification events.
    pub(crate) state_verification_results: HashMap<String, willow_state::StateHash>,
    /// Timestamp (ms) when we last broadcast a typing indicator (for debouncing).
    pub(crate) last_typing_sent_ms: u64,
    /// Maps peer_id -> (channel_name, timestamp_ms) for typing indicators.
    pub(crate) typing_peers: HashMap<String, (String, u64)>,
}

impl Client {
    /// Create a new client. Loads or generates identity, loads or creates
    /// the server with default channels, loads persisted messages.
    ///
    /// Does **not** connect to the network -- call [`Client::connect()`] for that.
    pub fn new(config: ClientConfig) -> Self {
        let identity = load_identity();

        let (cmd_tx, cmd_rx) = std_mpsc::channel();
        let (event_tx, event_rx) = std_mpsc::channel();

        // We hold cmd_rx and event_tx until connect() is called.
        // Store them in a side channel via Mutex.
        let deferred = std::sync::Arc::new(std::sync::Mutex::new(Some((event_tx, cmd_rx))));

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
            // If no legacy server found, servers stays empty — user must create or join.
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

                    // No saved state — seed from legacy channels.
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
            Self::persist_servers(&state);
        }

        // Load messages from event_state (authoritative source with reactions/edits).
        {
            let peer_id_for_msgs = identity.peer_id().to_string();
            let active_sid = state.active_server.clone().unwrap_or_default();
            for es_msg in &state.event_state.messages {
                if state.chat.seen_message_ids.insert(es_msg.id.clone()) {
                    // Resolve topic from channel_id.
                    let topic = state
                        .active()
                        .and_then(|ctx| {
                            ctx.topic_map
                                .iter()
                                .find(|(_, (_, cid))| cid.to_string() == es_msg.channel_id)
                                .map(|(t, _)| t.clone())
                        })
                        .unwrap_or_default();
                    let is_local = es_msg.author == peer_id_for_msgs;
                    let author = state.profiles.display_name(&es_msg.author);
                    let mut msg = ChatMessage::new(
                        active_sid.clone(),
                        topic,
                        author,
                        es_msg.body.clone(),
                        is_local,
                        es_msg.timestamp_ms,
                    );
                    msg.id = es_msg.id.clone();
                    msg.edited = es_msg.edited;
                    msg.deleted = es_msg.deleted;
                    msg.reply_to = es_msg.reply_to.clone();
                    // Reconstruct reply preview from parent message.
                    if let Some(ref parent_id) = es_msg.reply_to {
                        msg.reply_preview = state
                            .event_state
                            .messages
                            .iter()
                            .find(|m| m.id == *parent_id)
                            .map(|m| {
                                let text = if m.body.len() > 50 {
                                    format!("{}...", &m.body[..50])
                                } else {
                                    m.body.clone()
                                };
                                let author_name = state.profiles.display_name(&m.author);
                                format!("{author_name}: {text}")
                            });
                    }
                    // Copy reactions directly.
                    for (emoji, authors) in &es_msg.reactions {
                        msg.reactions.insert(emoji.clone(), authors.clone());
                    }
                    state.chat.messages.push(msg);
                }
            }
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

        let mut client = Self {
            state,
            identity,
            cmd_tx,
            event_rx,
            connected: false,
            config,
            deferred_channels: Some(deferred),
            connected_subscribed: false,
            profile_broadcast_counter: 0,
            notification_tx: None,
            state_verification_results: HashMap::new(),
            last_typing_sent_ms: 0,
            typing_peers: HashMap::new(),
        };

        // Merge reaction/edit/delete data from saved state into chat messages.
        client.merge_event_state_into_chat();
        client
    }

    /// Attach a push notification channel. Notifications are sent whenever
    /// an event is applied to the event-sourced state, a peer connects or
    /// disconnects, etc.
    ///
    /// This is an **addition** to the existing `poll()` model -- both work
    /// simultaneously.
    pub fn with_notifications(mut self, tx: std_mpsc::Sender<events::ClientNotification>) -> Self {
        self.notification_tx = Some(tx);
        self
    }

    /// Helper: send a push notification if the channel is configured.
    fn notify(&self, notification: events::ClientNotification) {
        if let Some(ref tx) = self.notification_tx {
            let _ = tx.send(notification);
        }
    }

    /// Helper: apply an event to the event-sourced state and store it.
    /// Sends a push notification if the channel is configured.
    /// Persists the full state after every mutation.
    fn apply_event(&mut self, event: &willow_state::Event) {
        willow_state::apply_lenient(&mut self.state.event_state, event);
        self.state.event_store.append(event.clone());
        self.state
            .event_store
            .set_latest_hash(self.state.event_state.hash());
        self.notify(events::ClientNotification::EventApplied(event.clone()));

        // Persist full state after every mutation.
        if self.config.persistence {
            if let Some(sid) = &self.state.active_server {
                storage::save_server_state(sid, &self.state.event_state);
            }
        }
    }

    /// Persist all servers to storage.
    fn persist_servers(state: &ClientState) {
        let ids: Vec<String> = state.servers.keys().cloned().collect();
        storage::save_server_list(&ids);
        for (id, ctx) in &state.servers {
            storage::save_server_by_id(id, &ctx.server, &ctx.keys);
        }
    }

    /// Connect to the P2P network. Spawns the network task in the background.
    ///
    /// After connecting, call [`Client::poll()`] regularly to process events.
    pub fn connect(&mut self) {
        if self.connected {
            return;
        }

        let Some(deferred) = self.deferred_channels.take() else {
            return;
        };
        let Ok(mut guard) = deferred.lock() else {
            return;
        };
        let Some((event_tx, cmd_rx)) = guard.take() else {
            return;
        };

        let config = network::build_network_config(self.config.relay_addr.as_deref());

        if self.config.persistence {
            storage::save_settings(&storage::NetworkSettings {
                relay_addr: self.config.relay_addr.clone(),
            });
        }

        network::spawn_network(self.identity.clone(), event_tx, cmd_rx, config);
        self.connected = true;
    }

    /// Drain network events, apply them to state, and return a list of
    /// [`ClientEvent`]s for the caller to handle.
    pub fn poll(&mut self) -> Vec<ClientEvent> {
        let mut events = Vec::new();

        // Re-broadcast our profile periodically during startup so peers
        // learn our display name even if the initial broadcast was missed
        // (gossipsub mesh not yet formed).
        if self.connected_subscribed && self.profile_broadcast_counter < 600 {
            self.profile_broadcast_counter += 1;
            // Broadcast at ticks 60, 120, 200, 400 (~3s, 6s, 10s, 20s at 50ms poll)
            if matches!(self.profile_broadcast_counter, 60 | 120 | 200 | 400) {
                let saved = storage::load_profile().unwrap_or_default();
                if !saved.display_name.is_empty() {
                    let _ = self.cmd_tx.send(network::NetworkCommand::BroadcastProfile {
                        display_name: saved.display_name,
                    });
                }
            }
        }

        while let Ok(net_event) = self.event_rx.try_recv() {
            match net_event {
                // ── EventReceived ────────────────────────────────────
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

                    // Apply to event-sourced state.
                    let result = willow_state::apply_lenient(&mut self.state.event_state, &event);
                    if matches!(result, willow_state::ApplyResult::Applied) {
                        self.state.event_store.append(event.clone());
                        self.state
                            .event_store
                            .set_latest_hash(self.state.event_state.hash());
                        self.notify(events::ClientNotification::EventApplied(event.clone()));

                        // Persist full state.
                        if self.config.persistence {
                            if let Some(sid) = &self.state.active_server {
                                storage::save_server_state(sid, &self.state.event_state);
                            }
                        }

                        // Process chat messages into ChatState.
                        self.process_event_message(&event, &mut events);

                        // Emit ClientEvents based on event kind.
                        self.emit_client_events_for(&event, &mut events);
                    }
                }

                // ── SyncRequested ────────────────────────────────────
                network::NetworkEvent::SyncRequested {
                    state_hash,
                    from,
                    topic,
                } => {
                    tracing::info!(%from, ?topic, "sync requested");
                    let missing = self.state.event_store.events_since(&state_hash);
                    if !missing.is_empty() {
                        let count = missing.len();
                        tracing::info!(count, "sending event sync batch");
                        let _ = self
                            .cmd_tx
                            .send(network::NetworkCommand::SendSyncBatch { events: missing });
                    }
                }

                // ── SyncBatchReceived ────────────────────────────────
                network::NetworkEvent::SyncBatchReceived {
                    events: batch_events,
                    from,
                } => {
                    tracing::info!(count = batch_events.len(), %from, "received event sync batch");
                    // Track the sender as online.
                    if !from.is_empty() && !self.state.chat.peers.contains(&from) {
                        self.state.chat.peers.push(from.clone());
                        events.push(ClientEvent::PeerConnected(from.clone()));
                    }
                    let mut sorted = batch_events;
                    sorted.sort_by_key(|e| e.timestamp_ms);
                    let count = sorted.len();
                    for event in &sorted {
                        let result =
                            willow_state::apply_lenient(&mut self.state.event_state, event);
                        if matches!(result, willow_state::ApplyResult::Applied) {
                            self.state.event_store.append(event.clone());
                            self.state
                                .event_store
                                .set_latest_hash(self.state.event_state.hash());
                            self.notify(events::ClientNotification::EventApplied(event.clone()));

                            // Process chat messages into ChatState.
                            self.process_event_message(event, &mut events);

                            self.emit_client_events_for(event, &mut events);
                        }
                    }
                    if count > 0 {
                        // Persist full state after sync batch.
                        if self.config.persistence {
                            if let Some(sid) = &self.state.active_server {
                                storage::save_server_state(sid, &self.state.event_state);
                            }
                        }
                        events.push(ClientEvent::SyncCompleted { ops_applied: count });
                        // Trigger state verification after sync completion.
                        let _ = self.verify_state();
                    }
                }

                // ── Common events ───────────────────────────────────
                network::NetworkEvent::PeerConnected(peer) => {
                    if !self.state.chat.peers.contains(&peer) {
                        self.state.chat.peers.push(peer.clone());
                    }
                    // On first peer connect, subscribe to channels.
                    if !self.connected_subscribed {
                        self.on_connected();
                        self.connected_subscribed = true;
                    } else {
                        // Re-broadcast profile so the new peer learns our name.
                        let saved = storage::load_profile().unwrap_or_default();
                        if !saved.display_name.is_empty() {
                            let _ = self.cmd_tx.send(network::NetworkCommand::BroadcastProfile {
                                display_name: saved.display_name,
                            });
                        }
                    }
                    self.notify(events::ClientNotification::PeerConnected(peer.clone()));
                    events.push(ClientEvent::PeerConnected(peer));
                }
                network::NetworkEvent::PeerDisconnected(peer) => {
                    self.state.chat.peers.retain(|p| p != &peer);
                    self.notify(events::ClientNotification::PeerDisconnected(peer.clone()));
                    events.push(ClientEvent::PeerDisconnected(peer));
                }
                network::NetworkEvent::ProfileReceived {
                    peer_id,
                    display_name,
                } => {
                    self.state
                        .profiles
                        .names
                        .insert(peer_id.clone(), display_name.clone());
                    events.push(ClientEvent::ProfileUpdated {
                        peer_id,
                        display_name,
                    });
                }
                network::NetworkEvent::Listening(addr) => {
                    // On receiving a listening event, do initial subscriptions.
                    if !self.connected_subscribed {
                        self.on_connected();
                        self.connected_subscribed = true;
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
                    let author = self.state.profiles.display_name(&from);
                    let size_kb = size / 1024;
                    let body = format!("[shared file: {filename} ({size_kb} KB)]");
                    let ts = self.state.chat.hlc.latest().millis;
                    let server_id = self
                        .state
                        .find_server_for_topic(&topic)
                        .map(|s| s.to_string())
                        .or_else(|| self.state.active_server.clone())
                        .unwrap_or_default();
                    self.state.chat.messages.push(ChatMessage::new(
                        server_id,
                        topic.clone(),
                        author,
                        body,
                        false,
                        ts,
                    ));
                    self.state.chat.messages_dirty = true;

                    let channel = self
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
                    let now = util::current_time_ms();
                    self.typing_peers.insert(peer_id.clone(), (channel, now));
                    // Also track as online peer.
                    if !self.state.chat.peers.contains(&peer_id) {
                        self.state.chat.peers.push(peer_id.clone());
                        events.push(ClientEvent::PeerConnected(peer_id));
                    }
                }
                network::NetworkEvent::MessageReceived { .. } => {
                    // All messages now go through EventReceived.
                }
            }
        }

        events
    }

    /// Process an incoming event that represents a chat message, edit, delete,
    /// or reaction. Updates `ChatState` and persists to `MessageDb`.
    fn process_event_message(
        &mut self,
        event: &willow_state::Event,
        events: &mut Vec<ClientEvent>,
    ) {
        match &event.kind {
            willow_state::EventKind::Message {
                ref channel_id,
                ref body,
            } => {
                let server_id = self.state.active_server.clone().unwrap_or_default();
                let topic = self
                    .state
                    .active()
                    .and_then(|ctx| {
                        ctx.topic_map
                            .iter()
                            .find(|(_, (_, cid))| cid.to_string() == *channel_id)
                            .map(|(t, _)| t.clone())
                    })
                    .unwrap_or_default();
                let channel_name = self
                    .state
                    .active()
                    .and_then(|ctx| ctx.name_for_topic(&topic))
                    .unwrap_or("unknown")
                    .to_string();

                let author = self.state.profiles.display_name(&event.author);
                let is_local = event.author == self.identity.peer_id().to_string();

                let mut msg = ChatMessage::new(
                    server_id,
                    topic.clone(),
                    author.clone(),
                    body.clone(),
                    is_local,
                    event.timestamp_ms,
                );
                msg.id = event.id.clone();

                // Dedup
                if !self.state.chat.seen_message_ids.insert(event.id.clone()) {
                    return;
                }

                // Store in message DB
                if let Some(ref db_arc) = self.state.message_db {
                    if let Ok(db_lock) = db_arc.lock() {
                        db_lock.insert(&storage::StoredMessage {
                            topic: msg.topic.clone(),
                            author,
                            body: body.clone(),
                            is_local,
                            timestamp_ms: event.timestamp_ms,
                            msg_id: event.id.clone(),
                        });
                    }
                }

                // Update unread counts.
                let current_topic = self
                    .state
                    .active()
                    .and_then(|ctx| ctx.topic_for_name(&self.state.chat.current_channel))
                    .unwrap_or_default();
                if !is_local && msg.topic != current_topic {
                    let sid = self.state.active_server.clone().unwrap_or_default();
                    if let Some(ctx) = self.state.servers.get_mut(&sid) {
                        *ctx.unread.entry(msg.topic.clone()).or_insert(0) += 1;
                    }
                }

                self.state.chat.messages.push(msg.clone());
                self.state.chat.messages_dirty = true;

                events.push(ClientEvent::MessageReceived {
                    channel: channel_name,
                    message: msg,
                });
            }
            willow_state::EventKind::EditMessage {
                ref message_id,
                ref new_body,
            } => {
                for m in &mut self.state.chat.messages {
                    if m.id == *message_id {
                        m.body = new_body.clone();
                        m.edited = true;
                        self.state.chat.messages_dirty = true;
                        break;
                    }
                }
            }
            willow_state::EventKind::DeleteMessage { ref message_id } => {
                for m in &mut self.state.chat.messages {
                    if m.id == *message_id {
                        m.body = "[message deleted]".to_string();
                        m.deleted = true;
                        m.reactions.clear();
                        self.state.chat.messages_dirty = true;
                        break;
                    }
                }
            }
            willow_state::EventKind::Reaction {
                ref message_id,
                ref emoji,
            } => {
                let author = self.state.profiles.display_name(&event.author);
                for m in &mut self.state.chat.messages {
                    if m.id == *message_id {
                        m.reactions.entry(emoji.clone()).or_default().push(author);
                        self.state.chat.messages_dirty = true;
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    /// Convert a [`willow_state::Event`] into [`ClientEvent`]s for the caller.
    fn emit_client_events_for(
        &mut self,
        event: &willow_state::Event,
        events: &mut Vec<ClientEvent>,
    ) {
        match &event.kind {
            willow_state::EventKind::Message { .. } => {
                // Already handled by process_event_message.
            }
            willow_state::EventKind::CreateChannel { name, .. } => {
                events.push(ClientEvent::ChannelCreated(name.clone()));
            }
            willow_state::EventKind::DeleteChannel { channel_id } => {
                // Look up channel name from state before deletion.
                let name = self
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
                let channel = self
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
                let channel = self
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
            willow_state::EventKind::StateVerification { state_hash } => {
                let our_hash = self.state.event_state.hash();
                self.state_verification_results
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

    // ───── Server management ──────────────────────────────────────────────────

    /// Switch to a different server by ID.
    pub fn switch_server(&mut self, server_id: &str) {
        if self.state.servers.contains_key(server_id) {
            self.state.active_server = Some(server_id.to_string());
            self.init_event_state_for_server(server_id);
            self.merge_event_state_into_chat();
        }
    }

    /// Merge reaction/edit/delete data from event_state.messages into chat.messages.
    fn merge_event_state_into_chat(&mut self) {
        for es_msg in &self.state.event_state.messages {
            if let Some(chat_msg) = self
                .state
                .chat
                .messages
                .iter_mut()
                .find(|m| m.id == es_msg.id)
            {
                for (emoji, authors) in &es_msg.reactions {
                    let entry = chat_msg.reactions.entry(emoji.clone()).or_default();
                    for author in authors {
                        if !entry.contains(author) {
                            entry.push(author.clone());
                        }
                    }
                }
                if es_msg.edited {
                    chat_msg.body = es_msg.body.clone();
                    chat_msg.edited = true;
                }
                if es_msg.deleted {
                    chat_msg.body = "[message deleted]".to_string();
                    chat_msg.deleted = true;
                }
            }
        }
    }

    /// Initialize (or re-initialize) the event-sourced state for a specific server.
    /// Tries loading saved state first, then falls back to replaying events.
    fn init_event_state_for_server(&mut self, server_id: &str) {
        let Some(ctx) = self.state.servers.get(server_id) else {
            return;
        };

        // Try loading saved state first.
        if let Some(saved_state) = storage::load_server_state(server_id) {
            self.state.event_state = saved_state;
        } else {
            let owner = ctx.server.owner.to_string();
            self.state.event_state =
                willow_state::ServerState::new(server_id, ctx.server.name.clone(), owner);
        }

        // Open persistent event store for this server.
        if self.config.persistence {
            if let Some(store) = storage::open_event_store(server_id) {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.state.event_store = state::PersistentEventStore::Sqlite(store);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    self.state.event_store = state::PersistentEventStore::LocalStorage(store);
                }
            }
        }

        // If we didn't load a saved state, replay persisted events.
        if storage::load_server_state(server_id).is_none() {
            let stored_events = self.state.event_store.all_events();
            if !stored_events.is_empty() {
                for event in &stored_events {
                    willow_state::apply_lenient(&mut self.state.event_state, event);
                }
                // Merge reaction data from replayed event_state into chat messages.
                for es_msg in &self.state.event_state.messages {
                    if !es_msg.reactions.is_empty() {
                        if let Some(chat_msg) = self
                            .state
                            .chat
                            .messages
                            .iter_mut()
                            .find(|m| m.id == es_msg.id)
                        {
                            for (emoji, authors) in &es_msg.reactions {
                                let entry = chat_msg.reactions.entry(emoji.clone()).or_default();
                                for author in authors {
                                    if !entry.contains(author) {
                                        entry.push(author.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                // Seed event_state with channels from legacy storage.
                let ctx = self.state.servers.get(server_id).unwrap();
                for (name, ch_id) in ctx.topic_map.values() {
                    self.state.event_state.channels.insert(
                        ch_id.to_string(),
                        willow_state::Channel {
                            id: ch_id.to_string(),
                            name: name.clone(),
                            pinned_messages: std::collections::HashSet::new(),
                        },
                    );
                }
            }
        }
    }

    /// List all servers as (id, name) pairs.
    pub fn server_list(&self) -> Vec<(String, String)> {
        self.state.server_list()
    }

    /// Get the name of the currently active server.
    pub fn active_server_name(&self) -> String {
        self.state
            .active()
            .map(|ctx| ctx.server.name.clone())
            .unwrap_or_else(|| "No Server".to_string())
    }

    /// Get the ID of the currently active server.
    pub fn active_server_id(&self) -> Option<&str> {
        self.state.active_server.as_deref()
    }

    /// Check whether any servers exist.
    pub fn has_servers(&self) -> bool {
        !self.state.servers.is_empty()
    }

    /// Create a brand-new server with the local user as owner.
    ///
    /// Automatically creates a "general" text channel, initializes the
    /// event-sourced state, persists everything, and subscribes to the
    /// channel topic on the network.
    ///
    /// Returns the server ID.
    pub fn create_server(&mut self, name: &str) -> anyhow::Result<String> {
        let mut server = willow_channel::Server::new(name, self.identity.peer_id());
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

        self.state.servers.insert(server_id.clone(), ctx);
        self.state.active_server = Some(server_id.clone());
        self.state.chat.current_channel = "general".to_string();

        // Initialize event-sourced state for this server.
        let peer_id = self.identity.peer_id().to_string();
        self.state.event_state =
            willow_state::ServerState::new(server_id.clone(), name.to_string(), peer_id.clone());

        // Open event store.
        if self.config.persistence {
            if let Some(store) = storage::open_event_store(&server_id) {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.state.event_store = state::PersistentEventStore::Sqlite(store);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    self.state.event_store = state::PersistentEventStore::LocalStorage(store);
                }
            }
        }

        // Create the general channel via event.
        let create_ch = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: ch_id_str,
            },
        };
        self.apply_event(&create_ch);

        // Persist.
        if self.config.persistence {
            Self::persist_servers(&self.state);
        }

        // Subscribe to channel topic if connected.
        let _ = self.cmd_tx.send(network::NetworkCommand::Subscribe(topic));

        Ok(server_id)
    }

    /// Set display name for the active server via event-sourced state.
    pub fn set_server_display_name(&mut self, name: &str) -> anyhow::Result<()> {
        if self.state.active_server.is_none() {
            return Err(anyhow::anyhow!("no active server"));
        }
        let peer_id = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::SetProfile {
                display_name: name.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        // Also update the global profile for backward compat.
        self.set_display_name(name);

        Ok(())
    }

    /// Get the display name for the active server (from event-sourced state).
    pub fn server_display_name(&self) -> String {
        let peer_id = self.identity.peer_id().to_string();
        self.state
            .event_state
            .profiles
            .get(&peer_id)
            .map(|p| p.display_name.clone())
            .unwrap_or_else(|| self.display_name())
    }

    // ───── Action methods ───────────────────────────────────────────────────

    /// Send a text message to the given channel.
    pub fn send_message(&mut self, channel: &str, body: &str) -> anyhow::Result<()> {
        let content = Content::Text {
            body: body.to_string(),
        };
        self.send_content(channel, content, body, None, None)
    }

    /// Send a reply to a specific message.
    pub fn send_reply(&mut self, channel: &str, parent_id: &str, body: &str) -> anyhow::Result<()> {
        let parent =
            willow_messaging::MessageId(uuid::Uuid::parse_str(parent_id).unwrap_or_default());
        let content = Content::Reply {
            parent,
            body: body.to_string(),
        };

        // Build reply preview from existing messages.
        let preview = self
            .state
            .chat
            .messages
            .iter()
            .find(|m| m.id == parent_id)
            .map(|m| {
                let text = if m.body.len() > 50 {
                    format!("{}...", &m.body[..50])
                } else {
                    m.body.clone()
                };
                format!("{}: {text}", m.author)
            });

        self.send_content(channel, content, body, preview, Some(parent_id.to_string()))
    }

    /// Share a small file inline by base64-encoding it into a text message.
    ///
    /// The message body uses the format `[file:filename:base64data]` so the
    /// UI can detect it and render a download card. Files larger than 256 KB
    /// are rejected.
    pub fn share_file_inline(
        &mut self,
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
        &mut self,
        _channel: &str,
        message_id: &str,
        new_body: &str,
    ) -> anyhow::Result<()> {
        let _ = self
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let peer_id_str = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::EditMessage {
                message_id: message_id.to_string(),
                new_body: new_body.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        // Apply locally.
        for m in &mut self.state.chat.messages {
            if m.id == message_id {
                m.body = new_body.to_string();
                m.edited = true;
                self.state.chat.messages_dirty = true;
                break;
            }
        }

        Ok(())
    }

    /// Delete a message.
    pub fn delete_message(&mut self, _channel: &str, message_id: &str) -> anyhow::Result<()> {
        let _ = self
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let peer_id_str = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::DeleteMessage {
                message_id: message_id.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        // Apply locally.
        for m in &mut self.state.chat.messages {
            if m.id == message_id {
                m.body = "[message deleted]".to_string();
                m.deleted = true;
                m.reactions.clear();
                self.state.chat.messages_dirty = true;
                break;
            }
        }

        Ok(())
    }

    /// Add a reaction to a message.
    pub fn react(&mut self, _channel: &str, message_id: &str, emoji: &str) -> anyhow::Result<()> {
        let _ = self
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let peer_id_str = self.identity.peer_id().to_string();
        let reaction_event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::Reaction {
                message_id: message_id.to_string(),
                emoji: emoji.to_string(),
            },
        };
        self.apply_event(&reaction_event);
        self.broadcast_event(reaction_event, None);

        // Apply locally to chat messages.
        let author = self.display_name();
        for m in &mut self.state.chat.messages {
            if m.id == message_id {
                m.reactions
                    .entry(emoji.to_string())
                    .or_default()
                    .push(author);
                self.state.chat.messages_dirty = true;
                break;
            }
        }

        Ok(())
    }

    /// Pin a message in a channel.
    ///
    /// Creates a `PinMessage` event in the event-sourced state and broadcasts
    /// it to peers.
    pub fn pin_message(&mut self, channel: &str, message_id: &str) -> anyhow::Result<()> {
        let channel_id = self.resolve_channel_id(channel)?;
        let peer_id = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::PinMessage {
                channel_id,
                message_id: message_id.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        // Also update legacy pinned list for backward compat.
        let pinned = self
            .state
            .chat
            .pinned
            .entry(channel.to_string())
            .or_default();
        if !pinned.contains(&message_id.to_string()) {
            pinned.push(message_id.to_string());
        }
        Ok(())
    }

    /// Unpin a message from a channel.
    ///
    /// Creates an `UnpinMessage` event in the event-sourced state and
    /// broadcasts it to peers.
    pub fn unpin_message(&mut self, channel: &str, message_id: &str) -> anyhow::Result<()> {
        let channel_id = self.resolve_channel_id(channel)?;
        let peer_id = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::UnpinMessage {
                channel_id,
                message_id: message_id.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        // Also update legacy pinned list for backward compat.
        if let Some(pinned) = self.state.chat.pinned.get_mut(channel) {
            pinned.retain(|id| id != message_id);
        }
        Ok(())
    }

    /// Get pinned message IDs for a channel from the event-sourced state.
    ///
    /// Returns a sorted `Vec` of message IDs that are pinned in the channel.
    pub fn pinned_message_ids(&self, channel: &str) -> Vec<String> {
        let channel_id = self
            .state
            .active()
            .and_then(|ctx| {
                ctx.topic_map
                    .values()
                    .find(|(n, _)| n == channel)
                    .map(|(_, cid)| cid.to_string())
            })
            .unwrap_or_default();

        self.state
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
    /// Returns messages whose IDs are in the event-sourced pinned set,
    /// falling back to the legacy in-memory list if the event-sourced
    /// state has no pinned messages.
    pub fn pinned_messages(&self, channel: &str) -> Vec<&ChatMessage> {
        let pinned_ids = self.pinned_message_ids(channel);
        if !pinned_ids.is_empty() {
            let pinned_set: std::collections::HashSet<&str> =
                pinned_ids.iter().map(|s| s.as_str()).collect();
            return self
                .messages(channel)
                .into_iter()
                .filter(|m| pinned_set.contains(m.id.as_str()))
                .collect();
        }

        // Fall back to legacy pinned list.
        let Some(legacy_ids) = self.state.chat.pinned.get(channel) else {
            return vec![];
        };
        self.state
            .chat
            .messages
            .iter()
            .filter(|m| legacy_ids.contains(&m.id))
            .collect()
    }

    /// Check if a message is pinned in a channel.
    pub fn is_pinned(&self, channel: &str, message_id: &str) -> bool {
        let pinned_ids = self.pinned_message_ids(channel);
        if !pinned_ids.is_empty() {
            return pinned_ids.iter().any(|id| id == message_id);
        }
        // Fall back to legacy.
        self.state
            .chat
            .pinned
            .get(channel)
            .map(|ids| ids.contains(&message_id.to_string()))
            .unwrap_or(false)
    }

    /// Resolve a channel name to its channel ID via the active server context.
    fn resolve_channel_id(&self, channel: &str) -> anyhow::Result<String> {
        let ctx = self
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        ctx.topic_map
            .values()
            .find(|(n, _)| n == channel)
            .map(|(_, cid)| cid.to_string())
            .ok_or_else(|| anyhow::anyhow!("channel not found: {}", channel))
    }

    /// Create a new channel.
    pub fn create_channel(&mut self, name: &str) -> anyhow::Result<()> {
        let ctx = self
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

        let _ = self.cmd_tx.send(network::NetworkCommand::Subscribe(topic));

        // Create and apply event, then broadcast it.
        let peer_id_str = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::CreateChannel {
                name: name.to_string(),
                channel_id: ch_id_str,
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        self.state.chat.current_channel = name.to_string();
        self.state.chat.messages_dirty = true;

        Ok(())
    }

    /// Delete a channel.
    pub fn delete_channel(&mut self, name: &str) -> anyhow::Result<()> {
        let ctx = self
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

        if self.state.chat.current_channel == name {
            let names = self
                .state
                .active()
                .map(|ctx| ctx.channel_names())
                .unwrap_or_default();
            self.state.chat.current_channel = names.first().cloned().unwrap_or_default();
            self.state.chat.messages_dirty = true;
        }

        // Create and apply event, then broadcast it.
        let peer_id_str = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::DeleteChannel {
                channel_id: ch_id_str,
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        Ok(())
    }

    /// Trust a peer for server state operations.
    ///
    /// Applies a `GrantPermission(Administrator)` event to the event-sourced
    /// state and broadcasts the event on the wire.
    pub fn trust_peer(&mut self, peer_id: &str) {
        let author = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::GrantPermission {
                peer_id: peer_id.to_string(),
                permission: willow_state::Permission::Administrator,
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);
    }

    /// Revoke trust from a peer.
    ///
    /// Applies a `RevokePermission(Administrator)` event to the event-sourced
    /// state and broadcasts the event on the wire.
    pub fn untrust_peer(&mut self, peer_id: &str) {
        let author = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::RevokePermission {
                peer_id: peer_id.to_string(),
                permission: willow_state::Permission::Administrator,
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);
    }

    /// Kick a member, rotating channel keys.
    pub fn kick_member(&mut self, peer_id: &str) -> anyhow::Result<()> {
        let ctx = self
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

        self.state.chat.peers.retain(|p| p != peer_id);

        // Create and apply event, then broadcast it.
        let author = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::KickMember {
                peer_id: peer_id.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        Ok(())
    }

    /// Create a new role.
    pub fn create_role(&mut self, name: &str) -> anyhow::Result<()> {
        let role_id = willow_channel::RoleId::new();
        let role = willow_channel::Role::with_id(role_id.clone(), name);

        let ctx = self
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        ctx.server.create_role(role);
        storage::save_server(&ctx.server, &ctx.keys);

        let peer_id_str = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::CreateRole {
                name: name.to_string(),
                role_id: role_id.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        Ok(())
    }

    /// Delete a role by ID.
    pub fn delete_role(&mut self, role_id: &str) -> anyhow::Result<()> {
        let rid = willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());

        let ctx = self
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        ctx.server.delete_role(&rid)?;
        storage::save_server(&ctx.server, &ctx.keys);

        let peer_id_str = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::DeleteRole {
                role_id: role_id.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        Ok(())
    }

    /// Set a permission on a role.
    pub fn set_permission(
        &mut self,
        role_id: &str,
        permission: &str,
        granted: bool,
    ) -> anyhow::Result<()> {
        let rid = willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
        let perm = parse_permission(permission)?;

        let ctx = self
            .state
            .active_mut()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        ctx.server.set_permission(&rid, perm, granted)?;
        storage::save_server(&ctx.server, &ctx.keys);

        let peer_id_str = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::SetPermission {
                role_id: role_id.to_string(),
                permission: permission.to_string(),
                granted,
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        Ok(())
    }

    /// Assign a role to a peer.
    pub fn assign_role(&mut self, peer_id: &str, role_id: &str) -> anyhow::Result<()> {
        let rid = willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());

        let ctx = self
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

        let peer_id_str_author = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str_author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::AssignRole {
                peer_id: peer_id.to_string(),
                role_id: role_id.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);

        Ok(())
    }

    /// Broadcast a state verification event carrying this peer's current state hash.
    pub fn verify_state(&mut self) -> anyhow::Result<()> {
        let author = self.identity.peer_id().to_string();
        let state_hash = self.state.event_state.hash();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::StateVerification { state_hash },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);
        Ok(())
    }

    /// Returns (agreeing_peers, total_peers_reporting) based on collected
    /// StateVerification results.
    pub fn state_hash_agreement(&self) -> (usize, usize) {
        let our_hash = self.state.event_state.hash();
        let total = self.state_verification_results.len();
        let agreeing = self
            .state_verification_results
            .values()
            .filter(|h| **h == our_hash)
            .count();
        (agreeing, total)
    }

    /// Rename the server. Only the owner can do this.
    pub fn rename_server(&mut self, new_name: &str) -> anyhow::Result<()> {
        let author = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::RenameServer {
                new_name: new_name.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);
        Ok(())
    }

    /// Set the server description. Only the owner can do this.
    pub fn set_server_description(&mut self, desc: &str) -> anyhow::Result<()> {
        let author = self.identity.peer_id().to_string();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: self.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::SetServerDescription {
                description: desc.to_string(),
            },
        };
        self.apply_event(&event);
        self.broadcast_event(event, None);
        Ok(())
    }

    /// Generate a secure invite code encrypted for the given recipient.
    pub fn generate_invite(&self, recipient_peer_id: &str) -> anyhow::Result<String> {
        let Some(pub_key) = invite::peer_id_to_ed25519_public(recipient_peer_id) else {
            anyhow::bail!("invalid recipient PeerId");
        };

        let ctx = self
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        invite::generate_invite(&ctx.server, &ctx.keys, &ctx.topic_map, &pub_key)
            .ok_or_else(|| anyhow::anyhow!("invite generation failed"))
    }

    /// Accept an invite code and join the server.
    pub fn accept_invite(&mut self, code: &str) -> anyhow::Result<()> {
        let accepted = invite::accept_invite(code, &self.identity)
            .ok_or_else(|| anyhow::anyhow!("invalid invite code or not for us"))?;

        let server_id = accepted.server_id.clone();

        // Check if we already have this server.
        if let Some(ctx) = self.state.servers.get_mut(&server_id) {
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
                    .send(network::NetworkCommand::Subscribe(topic.clone()));
            }
        } else {
            // Create a new server context for this server.
            // Use the ORIGINAL server ID from the invite so topics match.
            let mut server =
                willow_channel::Server::new(&accepted.server_name, self.identity.peer_id());
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
                    .send(network::NetworkCommand::Subscribe(topic.clone()));
            }

            let ctx = ServerContext {
                server,
                topic_map,
                keys,
                unread: HashMap::new(),
            };

            self.state.servers.insert(server_id.clone(), ctx);
        }

        self.state.active_server = Some(server_id.clone());
        self.init_event_state_for_server(&server_id);

        if let Some((_, (name, _))) = accepted.channel_keys.iter().next() {
            self.state.chat.current_channel = name.clone();
            self.state.chat.messages_dirty = true;
        }

        // Persist all servers so the joined server survives refresh.
        Self::persist_servers(&self.state);

        // Request sync for the new server — get all events from peers.
        let _ = self.cmd_tx.send(network::NetworkCommand::RequestSync {
            state_hash: willow_state::StateHash::ZERO,
            topic: None,
        });
        if let Some(ctx) = self.state.servers.get(&server_id) {
            for topic in ctx.topic_map.keys() {
                let _ = self.cmd_tx.send(network::NetworkCommand::RequestSync {
                    state_hash: willow_state::StateHash::ZERO,
                    topic: Some(topic.clone()),
                });
            }
        }

        Ok(())
    }

    /// Set the local display name and broadcast to peers.
    pub fn set_display_name(&mut self, name: &str) {
        let peer_id = self.identity.peer_id().to_string();
        self.state.profiles.names.insert(peer_id, name.to_string());

        storage::save_profile(&storage::LocalProfile {
            display_name: name.to_string(),
        });

        let _ = self.cmd_tx.send(network::NetworkCommand::BroadcastProfile {
            display_name: name.to_string(),
        });
    }

    /// Switch the current channel.
    pub fn switch_channel(&mut self, name: &str) {
        if self.state.chat.current_channel != name {
            self.state.chat.current_channel = name.to_string();
            self.state.chat.messages_dirty = true;

            if let Some(ctx) = self.state.active_mut() {
                if let Some(topic) = ctx.topic_for_name(name) {
                    ctx.unread.remove(&topic);
                }
            }
        }
    }

    // ───── Typing indicator methods ──────────────────────────────────────────

    /// Notify peers that we are typing in the current channel.
    ///
    /// Debounced — will not send more than once per 3 seconds.
    pub fn send_typing(&mut self) {
        let now = util::current_time_ms();
        if now - self.last_typing_sent_ms < 3000 {
            return; // debounce
        }
        self.last_typing_sent_ms = now;

        let channel = self.state.chat.current_channel.clone();
        if !channel.is_empty() {
            let _ = self
                .cmd_tx
                .send(network::NetworkCommand::SendTyping { channel });
        }
    }

    /// Get display names of peers currently typing in the given channel.
    ///
    /// Automatically expires entries older than 5 seconds and excludes the
    /// local user.
    pub fn typing_in(&mut self, channel: &str) -> Vec<String> {
        let now = util::current_time_ms();
        // Remove expired entries (older than 5 seconds).
        self.typing_peers.retain(|_, (_, ts)| now - *ts < 5000);

        let my_id = self.identity.peer_id().to_string();
        self.typing_peers
            .iter()
            .filter(|(pid, (ch, _))| ch == channel && *pid != &my_id)
            .map(|(pid, _)| self.peer_display_name(pid))
            .collect()
    }

    // ───── Accessor methods ─────────────────────────────────────────────────

    /// Get a reference to the full client state.
    pub fn state(&self) -> &ClientState {
        &self.state
    }

    /// Mutable access to the client state.
    pub fn state_mut(&mut self) -> &mut ClientState {
        &mut self.state
    }

    /// Get the local PeerId as a string.
    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }

    /// Get the local display name.
    ///
    /// Checks the event-sourced state profiles first, falling back to the
    /// legacy profile store.
    pub fn display_name(&self) -> String {
        let pid = self.peer_id();
        if let Some(profile) = self.state.event_state.profiles.get(&pid) {
            return profile.display_name.clone();
        }
        self.state.profiles.display_name(&pid)
    }

    /// Get a peer's display name.
    ///
    /// Checks the event-sourced state profiles first, falling back to the
    /// legacy profile store.
    pub fn peer_display_name(&self, peer_id: &str) -> String {
        if let Some(profile) = self.state.event_state.profiles.get(peer_id) {
            return profile.display_name.clone();
        }
        self.state.profiles.display_name(peer_id)
    }

    /// Get messages for a channel, filtered by active server and topic.
    pub fn messages(&self, channel: &str) -> Vec<&ChatMessage> {
        let Some(server_id) = &self.state.active_server else {
            return vec![];
        };
        let Some(ctx) = self.state.servers.get(server_id) else {
            return vec![];
        };
        let topic = ctx.topic_for_name(channel).unwrap_or_default();
        let mut msgs: Vec<&ChatMessage> = self
            .state
            .chat
            .messages
            .iter()
            .filter(|m| m.server_id == *server_id && m.topic == topic)
            .collect();
        msgs.sort_by_key(|m| m.timestamp_ms);
        msgs
    }

    /// List all channel names for the active server.
    ///
    /// Returns the union of channels from the server context and the
    /// event-sourced state, deduplicated and sorted.
    pub fn channels(&self) -> Vec<String> {
        let mut names = self.state.channel_names();

        // Merge any channels from event_state that aren't yet in the list.
        for ch in self.state.event_state.channels.values() {
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
    /// event-sequence order.
    pub fn event_messages(&self, channel_id: &str) -> Vec<&willow_state::ChatMessage> {
        self.state
            .event_state
            .messages
            .iter()
            .filter(|m| m.channel_id == channel_id && !m.deleted)
            .collect()
    }

    /// Get the list of connected peers (libp2p-level connections).
    pub fn peers(&self) -> &[String] {
        &self.state.chat.peers
    }

    /// Get all server members with online/offline status.
    ///
    /// Returns `(peer_id, display_name, is_online)` for each member.
    /// Falls back to `chat.peers` for peers not in the member list
    /// (e.g. connected before event sync completes).
    pub fn server_members(&self) -> Vec<(String, String, bool)> {
        let local_id = self.identity.peer_id().to_string();
        let online: std::collections::HashSet<&str> =
            self.state.chat.peers.iter().map(|s| s.as_str()).collect();

        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Members from event-sourced state.
        for (pid, member) in &self.state.event_state.members {
            let name = member
                .display_name
                .clone()
                .or_else(|| {
                    self.state
                        .event_state
                        .profiles
                        .get(pid)
                        .map(|p| p.display_name.clone())
                })
                .unwrap_or_else(|| self.peer_display_name(pid));
            // Local user is always online.
            let is_online = *pid == local_id || online.contains(pid.as_str());
            result.push((pid.clone(), name, is_online));
            seen.insert(pid.clone());
        }

        // Connected peers not yet in the member list (pre-sync).
        for pid in &self.state.chat.peers {
            if !seen.contains(pid) {
                let name = self.peer_display_name(pid);
                result.push((pid.clone(), name, true));
            }
        }

        result
    }

    /// Whether the network is connected.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    // ───── Internal helpers ─────────────────────────────────────────────────

    /// Broadcast a `willow_state::Event` to the network.
    ///
    /// If `topic` is `Some`, the event is published on that specific topic
    /// (used for chat messages). If `None`, it goes on the server ops topic.
    fn broadcast_event(&self, event: willow_state::Event, topic: Option<String>) {
        let _ = self
            .cmd_tx
            .send(network::NetworkCommand::BroadcastEvent { event, topic });
    }

    /// Send chat content (text, reply) on a channel.
    fn send_content(
        &mut self,
        channel: &str,
        _content: Content,
        body: &str,
        reply_preview: Option<String>,
        reply_to: Option<String>,
    ) -> anyhow::Result<()> {
        let ctx = self
            .state
            .active()
            .ok_or_else(|| anyhow::anyhow!("no active server"))?;
        let topic = ctx
            .topic_for_name(channel)
            .unwrap_or_else(|| channel.to_string());
        let server_id = self.state.active_server.clone().unwrap_or_default();

        let peer_id_str = self.identity.peer_id().to_string();

        // Resolve channel_id from topic.
        let channel_id = self
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
            parent_hash: self.state.event_state.hash(),
            author: peer_id_str.clone(),
            timestamp_ms: ts,
            kind: willow_state::EventKind::Message {
                channel_id,
                body: body.to_string(),
            },
        };
        self.apply_event(&msg_event);

        // Broadcast the event on the channel topic.
        self.broadcast_event(msg_event, Some(topic.clone()));

        // Add to local display.
        let author = self.state.profiles.display_name(&peer_id_str);
        let mut chat_msg = ChatMessage::new(
            server_id,
            topic.clone(),
            author.clone(),
            body.to_string(),
            true,
            ts,
        );
        chat_msg.id = event_id.clone();
        chat_msg.reply_preview = reply_preview;
        chat_msg.reply_to = reply_to;

        // Dedup
        self.state.chat.seen_message_ids.insert(event_id.clone());

        // Persist to MessageDb.
        if let Some(ref db_arc) = self.state.message_db {
            if let Ok(db_lock) = db_arc.lock() {
                db_lock.insert(&storage::StoredMessage {
                    topic: topic.clone(),
                    author,
                    body: body.to_string(),
                    is_local: true,
                    timestamp_ms: ts,
                    msg_id: event_id,
                });
            }
        }

        self.state.chat.messages.push(chat_msg);
        self.state.chat.messages_dirty = true;

        Ok(())
    }

    /// Called when we first hear from the network (Listening or PeerConnected).
    /// Subscribes to all channel topics for ALL servers, profile topic,
    /// server ops topic, broadcasts profile, and requests sync.
    fn on_connected(&self) {
        // Subscribe to all channel topics across all servers.
        for ctx in self.state.servers.values() {
            for topic in ctx.topic_map.keys() {
                let _ = self
                    .cmd_tx
                    .send(network::NetworkCommand::Subscribe(topic.clone()));
            }
        }

        // Subscribe to the global profile broadcast topic.
        let _ = self.cmd_tx.send(network::NetworkCommand::Subscribe(
            network::PROFILE_TOPIC.to_string(),
        ));

        // Subscribe to server state operations topic.
        let _ = self.cmd_tx.send(network::NetworkCommand::Subscribe(
            ops::SERVER_OPS_TOPIC.to_string(),
        ));

        // Broadcast our profile.
        let saved_profile = storage::load_profile().unwrap_or_default();
        if !saved_profile.display_name.is_empty() {
            let _ = self.cmd_tx.send(network::NetworkCommand::BroadcastProfile {
                display_name: saved_profile.display_name,
            });
        }

        // Request missing events via event-sourced sync.
        let state_hash = self.state.event_store.latest_hash();
        let _ = self.cmd_tx.send(network::NetworkCommand::RequestSync {
            state_hash: state_hash.clone(),
            topic: None,
        });
        for ctx in self.state.servers.values() {
            for topic in ctx.topic_map.keys() {
                let _ = self.cmd_tx.send(network::NetworkCommand::RequestSync {
                    state_hash: state_hash.clone(),
                    topic: Some(topic.clone()),
                });
            }
        }
    }
}

// ───── Identity persistence ──────────────────────────────────────────────────

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

/// Create a test-only Client without connecting to the network.
/// The returned client has mpsc channels wired up but no background task.
#[cfg(test)]
pub(crate) fn test_client() -> (Client, std::sync::mpsc::Receiver<network::NetworkCommand>) {
    let identity = Identity::generate();
    let (cmd_tx, cmd_rx) = std_mpsc::channel();
    let (_event_tx, event_rx) = std_mpsc::channel();

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
    // mirroring what Client::new() does.
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
        },
    );

    let client = Client {
        state,
        identity,
        cmd_tx,
        event_rx,
        connected: false,
        config: ClientConfig {
            persistence: false,
            ..ClientConfig::default()
        },
        deferred_channels: None,
        connected_subscribed: false,
        profile_broadcast_counter: 0,
        notification_tx: None,
        state_verification_results: HashMap::new(),
        last_typing_sent_ms: 0,
        typing_peers: HashMap::new(),
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
        let (mut client, _rx) = test_client();
        client.send_message("general", "hello").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "hello");
        assert!(msgs[0].is_local);
    }

    #[test]
    fn send_message_broadcasts_event() {
        let (mut client, rx) = test_client();
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
        let (mut client, _rx) = test_client();
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
        let (mut client, _rx) = test_client();
        client.send_message("general", "original").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.edit_message("general", &msg_id, "edited").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs[0].body, "edited");
        assert!(msgs[0].edited);
    }

    #[test]
    fn delete_message_marks_deleted() {
        let (mut client, _rx) = test_client();
        client.send_message("general", "to delete").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.delete_message("general", &msg_id).unwrap();

        let msgs = client.messages("general");
        assert!(msgs[0].deleted);
        assert_eq!(msgs[0].body, "[message deleted]");
    }

    #[test]
    fn react_adds_reaction() {
        let (mut client, _rx) = test_client();
        client.send_message("general", "react to me").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.react("general", &msg_id, "thumbsup").unwrap();

        let msgs = client.messages("general");
        assert!(msgs[0].reactions.contains_key("thumbsup"));
    }

    #[test]
    fn create_channel_adds_to_server() {
        let (mut client, _rx) = test_client();
        client.create_channel("new-channel").unwrap();

        let names = client.channels();
        assert!(names.contains(&"new-channel".to_string()));
        assert_eq!(client.state.chat.current_channel, "new-channel");
    }

    #[test]
    fn delete_channel_removes_from_server() {
        let (mut client, _rx) = test_client();
        client.create_channel("temp").unwrap();
        assert!(client.channels().contains(&"temp".to_string()));

        client.delete_channel("temp").unwrap();
        assert!(!client.channels().contains(&"temp".to_string()));
    }

    #[test]
    fn switch_channel_updates_current() {
        let (mut client, _rx) = test_client();
        client.create_channel("other").unwrap();
        client.switch_channel("general");

        assert_eq!(client.state.chat.current_channel, "general");
    }

    #[test]
    fn trust_untrust_broadcasts_events() {
        let (mut client, rx) = test_client();
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
        let (mut client, rx) = test_client();
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
        let (mut client, _rx) = test_client();
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

    // ───── Multi-server tests ─────────────────────────────────────────────

    #[test]
    fn test_switch_server() {
        let (mut client, _rx) = test_client();

        // Create a second server context.
        let server2 = willow_channel::Server::new("Second Server", client.identity.peer_id());
        let server2_id = server2.id.to_string();
        let ctx2 = ServerContext {
            server: server2,
            topic_map: HashMap::new(),
            keys: HashMap::new(),
            unread: HashMap::new(),
        };
        client.state.servers.insert(server2_id.clone(), ctx2);

        let original_id = client.state.active_server.clone().unwrap();
        assert_ne!(original_id, server2_id);

        client.switch_server(&server2_id);
        assert_eq!(
            client.state.active_server.as_deref(),
            Some(server2_id.as_str())
        );

        // Switch back.
        client.switch_server(&original_id);
        assert_eq!(
            client.state.active_server.as_deref(),
            Some(original_id.as_str())
        );

        // Switch to non-existent server does nothing.
        client.switch_server("non-existent");
        assert_eq!(
            client.state.active_server.as_deref(),
            Some(original_id.as_str())
        );
    }

    #[test]
    fn test_accept_invite_creates_new_server() {
        let (mut client, _rx) = test_client();
        let initial_count = client.state.servers.len();
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
            let ed_kp = client
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
        assert_eq!(client.state.servers.len(), 2);

        // Active server should be the new one.
        let active_id = client.state.active_server.clone().unwrap();
        let new_ctx = client.state.servers.get(&active_id).unwrap();
        assert!(!new_ctx.keys.is_empty());
    }

    #[test]
    fn test_messages_filtered_by_server() {
        let (mut client, _rx) = test_client();

        // Send a message on server 1.
        client.send_message("general", "server1 msg").unwrap();

        let server1_id = client.state.active_server.clone().unwrap();

        // Create a second server context with a "general" channel.
        let server2 = willow_channel::Server::new("Server 2", client.identity.peer_id());
        let server2_id = server2.id.to_string();
        let topic2 = util::make_topic(&server2, "general");
        let mut topic_map2 = HashMap::new();
        topic_map2.insert(
            topic2.clone(),
            ("general".to_string(), willow_channel::ChannelId::new()),
        );
        let ctx2 = ServerContext {
            server: server2,
            topic_map: topic_map2,
            keys: HashMap::new(),
            unread: HashMap::new(),
        };
        client.state.servers.insert(server2_id.clone(), ctx2);

        // Add a message that belongs to server 2.
        client.state.chat.messages.push(ChatMessage::new(
            server2_id.clone(),
            topic2,
            "Bob".to_string(),
            "server2 msg".to_string(),
            false,
            1000,
        ));

        // When viewing server 1, only see server 1 messages.
        client.switch_server(&server1_id);
        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "server1 msg");

        // When viewing server 2, only see server 2 messages.
        client.switch_server(&server2_id);
        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "server2 msg");
    }

    #[test]
    fn test_server_list() {
        let (mut client, _rx) = test_client();

        let list = client.server_list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].1, "Test Server");

        // Add a second server.
        let server2 = willow_channel::Server::new("Second", client.identity.peer_id());
        let server2_id = server2.id.to_string();
        client.state.servers.insert(
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

    // ───── Multi-peer scenario tests ─────────────────────────────────────

    #[test]
    fn client_create_multiple_channels_and_verify() {
        let (mut client, _rx) = test_client();

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
        let event_channels: Vec<String> = client
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
        let (mut client, _rx) = test_client();

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
            client.state.event_state.messages.len(),
            6,
            "event_state should have 6 messages total"
        );
    }

    #[test]
    fn client_trust_and_permission_flow() {
        let (mut client, _rx) = test_client();

        // Trust a peer.
        client.trust_peer("some-peer");

        // Verify they appear in event_state.peer_permissions with Administrator.
        assert!(
            client
                .state
                .event_state
                .has_permission("some-peer", &willow_state::Permission::Administrator),
            "trusted peer should have Administrator permission"
        );
        assert!(
            client.state.event_state.members.contains_key("some-peer"),
            "trusted peer should be a member"
        );

        // Untrust them.
        client.untrust_peer("some-peer");

        // Verify Administrator permission removed.
        assert!(
            !client
                .state
                .event_state
                .has_permission("some-peer", &willow_state::Permission::Administrator),
            "untrusted peer should not have Administrator permission"
        );
    }

    #[test]
    fn client_event_store_persists_events() {
        use willow_state::EventStore as _;

        let (mut client, _rx) = test_client();

        // Perform several actions.
        client.create_channel("test-channel").unwrap();
        client.send_message("test-channel", "hello").unwrap();
        client.trust_peer("peer-x");

        // Check event_store has the corresponding events.
        let events = client.state.event_store.all_events();
        assert!(
            events.len() >= 3,
            "event store should have at least 3 events, got {}",
            events.len()
        );
    }

    #[test]
    fn client_channels_from_event_state() {
        let (mut client, _rx) = test_client();

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
        let es_names: Vec<String> = client
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
        let (mut client, _rx) = test_client();

        // Set display name via SetProfile event on the event_state.
        let peer_id = client.peer_id();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: client.state.event_state.hash(),
            author: peer_id.clone(),
            timestamp_ms: 1000,
            kind: willow_state::EventKind::SetProfile {
                display_name: "EventAlice".into(),
            },
        };
        willow_state::apply_lenient(&mut client.state.event_state, &event);

        // Verify display_name() reads from event_state.profiles.
        assert_eq!(
            client.display_name(),
            "EventAlice",
            "display_name should come from event_state profile"
        );
    }

    // ───── State verification tests ─────────────────────────────────────

    #[test]
    fn verify_state_broadcasts_event() {
        let (mut client, rx) = test_client();
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
        let (mut client, _rx) = test_client();

        // Simulate a remote peer's StateVerification event with a matching hash.
        let our_hash = client.state.event_state.hash();
        client
            .state_verification_results
            .insert("peer-a".to_string(), our_hash);

        let (agreeing, total) = client.state_hash_agreement();
        assert_eq!(agreeing, 1);
        assert_eq!(total, 1);
    }

    #[test]
    fn state_hash_agreement_tracks_mismatched_peer() {
        let (mut client, _rx) = test_client();

        // Insert a different hash for a peer.
        let wrong_hash = willow_state::StateHash::from_bytes(b"wrong");
        client
            .state_verification_results
            .insert("peer-b".to_string(), wrong_hash);

        let (agreeing, total) = client.state_hash_agreement();
        assert_eq!(agreeing, 0);
        assert_eq!(total, 1);
    }

    #[test]
    fn state_hash_agreement_mixed() {
        let (mut client, _rx) = test_client();
        let our_hash = client.state.event_state.hash();

        // One matching, one mismatched.
        client
            .state_verification_results
            .insert("peer-a".to_string(), our_hash);
        client.state_verification_results.insert(
            "peer-b".to_string(),
            willow_state::StateHash::from_bytes(b"different"),
        );

        let (agreeing, total) = client.state_hash_agreement();
        assert_eq!(agreeing, 1);
        assert_eq!(total, 2);
    }

    // ───── Server rename/description tests ──────────────────────────────

    #[test]
    fn rename_server_updates_event_state() {
        let (mut client, _rx) = test_client();
        client.rename_server("New Server Name").unwrap();
        assert_eq!(client.state.event_state.server_name, "New Server Name");
    }

    #[test]
    fn rename_server_broadcasts_event() {
        let (mut client, rx) = test_client();
        client.rename_server("Another Name").unwrap();

        let cmd = rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            network::NetworkCommand::BroadcastEvent { .. }
        ));
    }

    #[test]
    fn set_server_description_updates_event_state() {
        let (mut client, _rx) = test_client();
        client.set_server_description("A cool server").unwrap();
        assert_eq!(client.state.event_state.description, "A cool server");
    }

    #[test]
    fn set_server_description_broadcasts_event() {
        let (mut client, rx) = test_client();
        client.set_server_description("Hello world").unwrap();

        let cmd = rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            network::NetworkCommand::BroadcastEvent { .. }
        ));
    }

    // ───── No-server and create_server tests ─────────────────────────────

    #[test]
    fn has_servers_returns_true_for_test_client() {
        let (client, _rx) = test_client();
        assert!(client.has_servers());
    }

    #[test]
    fn no_servers_returns_empty_channels() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;
        client.state.event_state.channels.clear();

        let channels = client.channels();
        assert!(channels.is_empty());
    }

    #[test]
    fn no_servers_returns_empty_messages() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        let msgs = client.messages("general");
        assert!(msgs.is_empty());
    }

    #[test]
    fn no_servers_active_server_name_returns_no_server() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        assert_eq!(client.active_server_name(), "No Server");
    }

    #[test]
    fn no_servers_send_message_returns_error() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        let result = client.send_message("general", "hello");
        assert!(result.is_err());
    }

    #[test]
    fn no_servers_create_channel_returns_error() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        let result = client.create_channel("test");
        assert!(result.is_err());
    }

    #[test]
    fn create_server_adds_server() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;
        assert!(!client.has_servers());

        let server_id = client.create_server("My New Server").unwrap();
        assert!(client.has_servers());
        assert_eq!(
            client.state.active_server.as_deref(),
            Some(server_id.as_str())
        );
        assert_eq!(client.active_server_name(), "My New Server");
    }

    #[test]
    fn create_server_has_general_channel() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        client.create_server("Test").unwrap();
        let channels = client.channels();
        assert!(
            channels.contains(&"general".to_string()),
            "created server should have a 'general' channel"
        );
    }

    #[test]
    fn create_server_sets_current_channel() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        client.create_server("Test").unwrap();
        assert_eq!(client.state.chat.current_channel, "general");
    }

    #[test]
    fn create_server_initializes_event_state() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        let server_id = client.create_server("Event Test").unwrap();
        assert_eq!(client.state.event_state.server_id, server_id);
        assert_eq!(client.state.event_state.server_name, "Event Test");
        assert_eq!(
            client.state.event_state.owner,
            client.identity.peer_id().to_string()
        );

        // Event state should have the general channel.
        assert!(
            client
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
        let (mut client, rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

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
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        client.create_server("Msg Test").unwrap();
        client.send_message("general", "hello new server").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "hello new server");
    }

    #[test]
    fn create_multiple_servers() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        let id1 = client.create_server("Server One").unwrap();
        let id2 = client.create_server("Server Two").unwrap();

        assert_eq!(client.state.servers.len(), 2);
        assert_ne!(id1, id2);

        // Active server should be the last created.
        assert_eq!(client.state.active_server.as_deref(), Some(id2.as_str()));
        assert_eq!(client.active_server_name(), "Server Two");

        // Can switch back to first server.
        client.switch_server(&id1);
        assert_eq!(client.active_server_name(), "Server One");
    }

    // ───── Per-server profile tests ──────────────────────────────────────

    #[test]
    fn set_server_display_name_updates_event_state() {
        let (mut client, _rx) = test_client();
        client.set_server_display_name("ServerAlice").unwrap();

        assert_eq!(client.server_display_name(), "ServerAlice");
    }

    #[test]
    fn set_server_display_name_no_server_returns_error() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        let result = client.set_server_display_name("test");
        assert!(result.is_err());
    }

    #[test]
    fn server_display_name_falls_back_to_global() {
        let (mut client, _rx) = test_client();
        client.set_display_name("GlobalName");
        // No server profile set, so should fall back to global.
        assert_eq!(client.server_display_name(), "GlobalName");
    }

    // ───── Typing indicator tests ──────────────────────────────────────────

    #[test]
    fn send_typing_debounces() {
        let (mut client, _rx) = test_client();
        client.send_typing();
        client.send_typing();
    }

    #[test]
    fn typing_in_returns_empty_when_no_typers() {
        let (mut client, _rx) = test_client();
        let typers = client.typing_in("general");
        assert!(typers.is_empty());
    }

    // ───── Server members tests ────────────────────────────────────────────

    #[test]
    fn server_members_includes_owner() {
        let (client, _rx) = test_client();
        let members = client.server_members();
        assert!(!members.is_empty());
        let owner = members.iter().find(|(_, _, online)| *online);
        assert!(owner.is_some());
    }

    // ───── Message dedup tests ─────────────────────────────────────────────

    #[test]
    fn seen_message_ids_prevents_duplicates() {
        let (mut client, _rx) = test_client();
        assert!(client.state().chat.seen_message_ids.is_empty());

        client.send_message("general", "test").unwrap();
        let _ = client.state().chat.seen_message_ids.len();
    }

    // ───── Per-server display name isolation ───────────────────────────────

    #[test]
    fn server_display_name_per_server() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        let _id1 = client.create_server("Server A").unwrap();
        let _ = client.set_server_display_name("Alice on A");
        assert_eq!(client.server_display_name(), "Alice on A");

        let _id2 = client.create_server("Server B").unwrap();
        let name = client.server_display_name();
        assert_eq!(name, "Alice on A");

        let _ = client.set_server_display_name("Alice on B");
        assert_eq!(client.server_display_name(), "Alice on B");
    }

    // ───── Server switching & event state isolation ──────────────────────

    #[test]
    fn switch_server_updates_event_state() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        let id1 = client.create_server("Server A").unwrap();
        assert!(client
            .state
            .event_state
            .channels
            .values()
            .any(|c| c.name == "general"));

        let _id2 = client.create_server("Server B").unwrap();
        assert_eq!(client.state.event_state.server_name, "Server B");

        client.switch_server(&id1);
        assert_eq!(client.state.event_state.server_name, "Server A");
    }

    #[test]
    fn switch_server_isolates_legacy_channels() {
        let (mut client, _rx) = test_client();
        client.state.servers.clear();
        client.state.active_server = None;

        let id1 = client.create_server("Server A").unwrap();
        client.create_channel("alpha").unwrap();

        let _id2 = client.create_server("Server B").unwrap();
        client.create_channel("beta").unwrap();

        // On Server B, channel_names should include "beta" but not "alpha".
        let names_b = client.state.channel_names();
        assert!(names_b.contains(&"general".to_string()));
        assert!(names_b.contains(&"beta".to_string()));
        assert!(!names_b.contains(&"alpha".to_string()));

        // Switch to Server A, channel_names should include "alpha" but not "beta".
        client.switch_server(&id1);
        let names_a = client.state.channel_names();
        assert!(names_a.contains(&"general".to_string()));
        assert!(names_a.contains(&"alpha".to_string()));
        assert!(!names_a.contains(&"beta".to_string()));
    }
}
