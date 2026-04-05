//! # Client State
//!
//! Pure state types for the Willow client, extracted from the Bevy UI resources.
//! These types hold the client's runtime state without any UI framework dependency.

use std::collections::HashMap;

use willow_channel::Server;
use willow_crypto::ChannelKey;
use willow_identity::EndpointId;
use willow_messaging::hlc::HLC;

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

/// Chat state holding current channel, peers, and the HLC clock.
pub struct ChatState {
    /// The current channel *name* (human-readable, e.g. "general").
    pub current_channel: String,
    pub peers: Vec<EndpointId>,
    pub hlc: HLC,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            current_channel: DEFAULT_CHANNEL.to_string(),
            peers: Vec::new(),
            hlc: HLC::new(),
        }
    }
}

/// A message prepared for display. Computed on-the-fly from
/// event_state, never stored. Display names are resolved at
/// construction time so they're never stale.
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayMessage {
    pub id: String,
    pub channel_id: String,
    pub author_peer_id: EndpointId,
    pub author_display_name: String,
    pub body: String,
    pub is_local: bool,
    pub timestamp_ms: u64,
    pub reactions: HashMap<String, Vec<String>>,
    pub edited: bool,
    pub deleted: bool,
    pub reply_to: Option<String>,
    pub reply_preview: Option<String>,
}

/// Maps EndpointId -> display names. Updated from profile broadcasts.
#[derive(Default, Clone)]
pub struct ProfileStore {
    pub names: HashMap<EndpointId, String>,
}

impl ProfileStore {
    /// Look up a display name for a peer, falling back to truncated ID.
    pub fn display_name(&self, peer_id: &EndpointId) -> String {
        self.names
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| crate::util::truncate_peer_id(&peer_id.to_string()))
    }
}

/// Aggregate client state bundle. Holds all runtime state for the client
/// without any UI framework dependency.
pub struct ClientState {
    /// Current channel, peers, and HLC clock.
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
}

impl ClientState {
    /// Create with a placeholder event state. The real owner will be set
    /// when a server is loaded or created.
    pub fn new(owner: EndpointId) -> Self {
        Self {
            chat: ChatState::default(),
            servers: HashMap::new(),
            active_server: None,
            profiles: ProfileStore::default(),
            emoji: crate::emoji::EmojiRegistry::new(),
            message_db: None,
            event_state: willow_state::ServerState::new("", "", owner),
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
