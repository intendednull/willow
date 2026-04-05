//! # Domain State Types
//!
//! Pure data types for each state domain, designed to be held in
//! [`StateActor<S>`](willow_actor::StateActor). All types are
//! `Clone + Send + Sync + PartialEq`.
//!
//! ## Adding a new domain
//!
//! 1. Define a new type here with `#[derive(Clone, Debug, Default, PartialEq)]`
//! 2. Add a `StateRef<YourType>` field to [`SourceState`]
//! 3. Spawn the actor in `ClientHandle::new()` and register it

use std::collections::{HashMap, HashSet};

use willow_crypto::ChannelKey;
use willow_identity::EndpointId;

// ───── Layer 1: Source state types ──────────────────────────────────────

/// Registry of all servers and their metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerRegistry {
    /// All servers keyed by server ID.
    pub servers: HashMap<String, ServerEntry>,
    /// Currently active server ID.
    pub active_server: Option<String>,
}

impl ServerRegistry {
    /// Get the active server entry (if any).
    pub fn active(&self) -> Option<&ServerEntry> {
        self.active_server
            .as_ref()
            .and_then(|id| self.servers.get(id))
    }

    /// Get the active server entry mutably.
    pub fn active_mut(&mut self) -> Option<&mut ServerEntry> {
        self.active_server
            .as_ref()
            .and_then(|id| self.servers.get_mut(id))
    }

    /// List all server IDs and names.
    pub fn server_list(&self) -> Vec<(String, String)> {
        self.servers
            .iter()
            .map(|(id, entry)| (id.clone(), entry.name.clone()))
            .collect()
    }
}

/// Metadata for a single server.
#[derive(Clone, Debug)]
pub struct ServerEntry {
    /// The channel server instance (stateful — has create_channel/delete_channel methods).
    pub server: willow_channel::Server,
    /// Server display name.
    pub name: String,
    /// Maps gossipsub topic → (channel_name, channel_id).
    pub topic_map: HashMap<String, (String, willow_channel::ChannelId)>,
    /// Per-channel encryption keys, keyed by topic.
    pub keys: HashMap<String, ChannelKey>,
    /// Unread message counts per channel topic.
    pub unread: HashMap<String, usize>,
}

impl PartialEq for ServerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.topic_map == other.topic_map
            && self.keys == other.keys
            && self.unread == other.unread
    }
}

impl ServerEntry {
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

    /// List all channel names in sorted order.
    pub fn channel_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.topic_map.values().map(|(n, _)| n.clone()).collect();
        names.sort();
        names.dedup();
        names
    }
}

/// Chat session metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMeta {
    /// The current channel name (human-readable, e.g. "general").
    pub current_channel: String,
    /// Online peers.
    pub peers: Vec<EndpointId>,
}

impl Default for ChatMeta {
    fn default() -> Self {
        Self {
            current_channel: crate::state::DEFAULT_CHANNEL.to_string(),
            peers: Vec::new(),
        }
    }
}

/// Global profile display names (across all servers).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileState {
    /// EndpointId → display name.
    pub names: HashMap<EndpointId, String>,
}

impl ProfileState {
    /// Look up a display name, falling back to truncated ID.
    pub fn display_name(&self, peer_id: &EndpointId) -> String {
        self.names
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| crate::util::truncate_peer_id(&peer_id.to_string()))
    }
}

/// Network connection metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetworkMeta {
    /// Whether we're connected to the network.
    pub connected: bool,
    /// Peers currently typing: EndpointId → (channel_name, timestamp_ms).
    pub typing_peers: HashMap<EndpointId, (String, u64)>,
    /// Last time we sent a typing indicator (for debouncing).
    pub last_typing_sent_ms: u64,
}

/// Voice call state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoiceState {
    /// Participants per voice channel: channel_id → set of peer IDs.
    pub participants: HashMap<String, HashSet<EndpointId>>,
    /// Currently active voice channel (if in a call).
    pub active_channel: Option<String>,
    /// Local mute state.
    pub muted: bool,
    /// Local deafen state.
    pub deafened: bool,
}

/// Combined EventDag + PendingBuffer, held in a single StateActor.
///
/// The DAG is the source of truth for all events. The PendingBuffer
/// holds events that arrived out of order (missing predecessor).
/// Combining them in one actor ensures insert + buffer operations
/// are atomic without manual lock coordination.
#[derive(Clone, Default)]
pub struct DagState {
    /// The per-author Merkle-DAG of all known events.
    pub dag: willow_state::EventDag,
    /// Buffer for events waiting on missing predecessors.
    pub pending: willow_state::PendingBuffer,
}

// ───── Source state bundle ───────────────────────────────────────────────

/// Bundle of all Layer 1 source state references.
///
/// Passed to derived view constructors. To add a new domain, add a
/// `StateRef<YourType>` field here.
pub struct SourceState {
    /// Event-sourced server state (messages, channels, roles, members, permissions).
    pub event_state: willow_actor::state::StateRef<willow_state::ServerState>,
    /// Server registry (all servers, active server, topic maps, keys).
    pub server_registry: willow_actor::state::StateRef<ServerRegistry>,
    /// Chat session metadata (current channel, peers).
    pub chat_meta: willow_actor::state::StateRef<ChatMeta>,
    /// Global profile display names.
    pub profiles: willow_actor::state::StateRef<ProfileState>,
    /// Network connection metadata (connected, typing peers).
    pub network: willow_actor::state::StateRef<NetworkMeta>,
    /// Voice call state.
    pub voice: willow_actor::state::StateRef<VoiceState>,
}

impl Clone for SourceState {
    fn clone(&self) -> Self {
        Self {
            event_state: self.event_state.clone(),
            server_registry: self.server_registry.clone(),
            chat_meta: self.chat_meta.clone(),
            profiles: self.profiles.clone(),
            network: self.network.clone(),
            voice: self.voice.clone(),
        }
    }
}
