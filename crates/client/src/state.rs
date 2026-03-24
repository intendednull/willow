//! # Client State
//!
//! Pure state types for the Willow client, extracted from the Bevy UI resources.
//! These types hold the client's runtime state without any UI framework dependency.

use std::collections::HashMap;

use willow_channel::Server;
use willow_crypto::ChannelKey;
use willow_messaging::hlc::HLC;

/// Maximum messages kept in memory per topic to avoid unbounded growth.
pub const MAX_MESSAGES_IN_MEMORY: usize = 1000;

/// Platform-aware event store that delegates to the appropriate backend.
///
/// Uses SQLite on native and localStorage on WASM. Falls back to the
/// in-memory store when persistence is disabled or unavailable.
pub enum PersistentEventStore {
    /// In-memory store for testing and ephemeral use.
    InMemory(willow_state::InMemoryStore),
    /// SQLite-backed store (native only).
    #[cfg(not(target_arch = "wasm32"))]
    Sqlite(crate::storage::SqliteEventStore),
    /// localStorage-backed store (WASM only).
    #[cfg(target_arch = "wasm32")]
    LocalStorage(crate::storage::LocalStorageEventStore),
}

impl Default for PersistentEventStore {
    fn default() -> Self {
        Self::InMemory(willow_state::InMemoryStore::new())
    }
}

impl willow_state::EventStore for PersistentEventStore {
    fn append(&mut self, event: willow_state::Event) {
        match self {
            Self::InMemory(s) => s.append(event),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Sqlite(s) => s.append(event),
            #[cfg(target_arch = "wasm32")]
            Self::LocalStorage(s) => s.append(event),
        }
    }

    fn events_since(&self, hash: &willow_state::StateHash) -> Vec<willow_state::Event> {
        match self {
            Self::InMemory(s) => s.events_since(hash),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Sqlite(s) => s.events_since(hash),
            #[cfg(target_arch = "wasm32")]
            Self::LocalStorage(s) => s.events_since(hash),
        }
    }

    fn all_events(&self) -> Vec<willow_state::Event> {
        match self {
            Self::InMemory(s) => s.all_events(),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Sqlite(s) => s.all_events(),
            #[cfg(target_arch = "wasm32")]
            Self::LocalStorage(s) => s.all_events(),
        }
    }

    fn latest_hash(&self) -> willow_state::StateHash {
        match self {
            Self::InMemory(s) => s.latest_hash(),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Sqlite(s) => s.latest_hash(),
            #[cfg(target_arch = "wasm32")]
            Self::LocalStorage(s) => s.latest_hash(),
        }
    }

    fn set_latest_hash(&mut self, hash: willow_state::StateHash) {
        match self {
            Self::InMemory(s) => s.set_latest_hash(hash),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Sqlite(s) => s.set_latest_hash(hash),
            #[cfg(target_arch = "wasm32")]
            Self::LocalStorage(s) => s.set_latest_hash(hash),
        }
    }

    fn contains(&self, event_id: &str) -> bool {
        match self {
            Self::InMemory(s) => s.contains(event_id),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Sqlite(s) => s.contains(event_id),
            #[cfg(target_arch = "wasm32")]
            Self::LocalStorage(s) => s.contains(event_id),
        }
    }
}

/// The default channel name used when no channels exist.
pub const DEFAULT_CHANNEL: &str = "general";

/// All state for a single server.
pub struct ServerContext {
    /// The channel server instance.
    pub server: Server,
    /// Maps gossipsub topic -> (channel_name, channel_id) for display + key lookup.
    pub topic_map: HashMap<String, (String, willow_channel::ChannelId)>,
    /// Per-channel encryption keys, keyed by topic.
    pub keys: HashMap<String, ChannelKey>,
    /// Unread message counts per channel topic.
    pub unread: HashMap<String, usize>,
}

impl ServerContext {
    /// Get the gossipsub topic for a channel by name.
    pub fn topic_for_name(&self, name: &str) -> Option<String> {
        self.topic_map
            .iter()
            .find(|(_, (n, _))| n == name)
            .map(|(topic, _)| topic.clone())
    }

    /// Get the channel name for a gossipsub topic.
    pub fn name_for_topic(&self, topic: &str) -> Option<&str> {
        self.topic_map.get(topic).map(|(name, _)| name.as_str())
    }

    /// List all channel names in sidebar order.
    pub fn channel_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .server
            .channels()
            .iter()
            .map(|ch| ch.name.clone())
            .collect();
        names.sort();
        names
    }
}

/// Chat state holding messages, current channel, peers, and the HLC clock.
pub struct ChatState {
    pub messages: Vec<ChatMessage>,
    /// The current channel *name* (human-readable, e.g. "general").
    pub current_channel: String,
    pub peers: Vec<String>,
    pub hlc: HLC,
    pub messages_dirty: bool,
    /// Seen message/op IDs for deduplication.
    pub seen_message_ids: std::collections::HashSet<String>,
    /// Pinned message IDs per channel name.
    pub pinned: HashMap<String, Vec<String>>,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            current_channel: DEFAULT_CHANNEL.to_string(),
            peers: Vec::new(),
            hlc: HLC::new(),
            messages_dirty: true,
            seen_message_ids: std::collections::HashSet::new(),
            pinned: HashMap::new(),
        }
    }
}

impl ChatState {
    /// Prune old messages if total count exceeds the limit.
    pub fn prune_if_needed(&mut self) {
        if self.messages.len() > MAX_MESSAGES_IN_MEMORY {
            let excess = self.messages.len() - MAX_MESSAGES_IN_MEMORY;
            self.messages.drain(..excess);
        }
    }
}

/// A single chat message with metadata.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// The server this message belongs to.
    pub server_id: String,
    /// The gossipsub topic this message belongs to.
    pub topic: String,
    /// Unique ID for this message (for reactions/edit/delete to target).
    pub id: String,
    pub author: String,
    pub body: String,
    pub is_local: bool,
    /// HLC timestamp in milliseconds (for display).
    pub timestamp_ms: u64,
    /// Reactions: emoji -> list of author names.
    pub reactions: HashMap<String, Vec<String>>,
    /// Whether this message has been edited.
    pub edited: bool,
    /// Whether this message has been deleted (shows "[deleted]").
    pub deleted: bool,
    /// If this is a reply, the parent message preview ("Author: text...").
    pub reply_preview: Option<String>,
    /// If this is a reply, the parent message ID (for jump-to-message).
    pub reply_to: Option<String>,
}

impl ChatMessage {
    pub fn new(
        server_id: String,
        topic: String,
        author: String,
        body: String,
        is_local: bool,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            server_id,
            topic,
            id: uuid::Uuid::new_v4().to_string(),
            author,
            body,
            is_local,
            timestamp_ms,
            reactions: HashMap::new(),
            edited: false,
            deleted: false,
            reply_preview: None,
            reply_to: None,
        }
    }
}

/// Maps PeerId strings -> display names. Updated from profile broadcasts.
#[derive(Default, Clone)]
pub struct ProfileStore {
    pub names: HashMap<String, String>,
}

impl ProfileStore {
    /// Look up a display name for a peer, falling back to truncated ID.
    pub fn display_name(&self, peer_id: &str) -> String {
        self.names
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| crate::util::truncate_peer_id(peer_id))
    }
}

/// Aggregate client state bundle. Holds all runtime state for the client
/// without any UI framework dependency.
pub struct ClientState {
    /// Chat messages, current channel, peers, and HLC clock.
    pub chat: ChatState,
    /// All servers, keyed by ServerId string.
    pub servers: HashMap<String, ServerContext>,
    /// Currently active server ID.
    pub active_server: Option<String>,
    /// Peer display names (global across all servers).
    pub profiles: ProfileStore,
    /// Emoji shortcode expansion registry.
    pub emoji: crate::emoji::EmojiRegistry,
    /// Persistent message database (native-only SQLite, WASM localStorage).
    pub message_db: Option<std::sync::Arc<std::sync::Mutex<crate::storage::MessageDb>>>,

    // --- Event-sourced state (willow-state) ---
    /// Event-sourced server state — the single source of truth.
    pub event_state: willow_state::ServerState,
    /// Persistent event store for the event-sourced model.
    pub event_store: PersistentEventStore,
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            chat: ChatState::default(),
            servers: HashMap::new(),
            active_server: None,
            profiles: ProfileStore::default(),
            emoji: crate::emoji::EmojiRegistry::new(),
            message_db: None,
            event_state: willow_state::ServerState::default(),
            event_store: PersistentEventStore::default(),
        }
    }
}

impl ClientState {
    /// Get the active server context (if any).
    pub fn active(&self) -> Option<&ServerContext> {
        self.active_server
            .as_ref()
            .and_then(|id| self.servers.get(id))
    }

    /// Get the active server context mutably.
    pub fn active_mut(&mut self) -> Option<&mut ServerContext> {
        self.active_server
            .as_ref()
            .and_then(|id| self.servers.get_mut(id))
    }

    /// Channel names for the active server.
    pub fn channel_names(&self) -> Vec<String> {
        self.active()
            .map(|ctx| ctx.channel_names())
            .unwrap_or_default()
    }

    /// List all server IDs and names.
    pub fn server_list(&self) -> Vec<(String, String)> {
        self.servers
            .iter()
            .map(|(id, ctx)| (id.clone(), ctx.server.name.clone()))
            .collect()
    }

    /// Find which server owns a given topic.
    pub fn find_server_for_topic(&self, topic: &str) -> Option<&str> {
        for (id, ctx) in &self.servers {
            if ctx.topic_map.contains_key(topic) {
                return Some(id);
            }
        }
        None
    }
}
