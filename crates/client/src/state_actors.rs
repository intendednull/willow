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

use crate::presence::{PresenceOverride, Tick, DEFAULT_GONE_TICKS, DEFAULT_IDLE_TICKS};

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
#[derive(Clone, Debug, PartialEq)]
pub struct ServerEntry {
    /// Server ID (UUID string).
    pub server_id: String,
    /// Server display name.
    pub name: String,
    /// Per-channel encryption keys, keyed by topic.
    pub keys: HashMap<String, ChannelKey>,
    /// Unread message counts per channel topic.
    pub unread: HashMap<String, usize>,
}

impl ServerEntry {
    /// Get the gossipsub topic for a channel by name.
    pub fn topic_for_name(&self, name: &str) -> Option<String> {
        Some(crate::util::make_topic(&self.server_id, name))
    }

    /// Get the channel name for a gossipsub topic.
    pub fn name_for_topic<'a>(&self, topic: &'a str) -> Option<&'a str> {
        let prefix = format!("{}/", self.server_id);
        topic.strip_prefix(&prefix)
    }

    /// Derive channel topic strings from event state channels.
    ///
    /// Iterates over channels in the given `ServerState` and returns
    /// topic strings of the form `"{server_id}/{channel_name}"`.
    pub fn channel_topics(&self, event_state: &willow_state::ServerState) -> Vec<String> {
        event_state
            .channels
            .values()
            .map(|ch| crate::util::make_topic(&self.server_id, &ch.name))
            .collect()
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

/// Presence metadata — holds the tick counter, last-seen map, queue
/// depth, whisper / invisibility hints, and the local self-override.
///
/// Derived view [`PresenceView`](crate::views::PresenceView) converts
/// this snapshot plus [`ChatMeta`] reachability + [`VoiceState`] into
/// per-peer [`PresenceState`](crate::presence::PresenceState) values.
#[derive(Clone, Debug, PartialEq)]
pub struct PresenceMeta {
    /// Monotonic tick counter. Advanced by the connect.rs tick driver.
    pub now: Tick,
    /// Last observed tick per peer (heartbeat or reachability probe).
    pub last_seen: HashMap<EndpointId, Tick>,
    /// Queued-outbound message depth per peer (stubbed — real queue in a
    /// later phase).
    pub queue_depth: HashMap<EndpointId, u32>,
    /// Peers currently in a whisper session we know about (stub).
    pub whispering_with: HashSet<EndpointId>,
    /// Peers invisible to us (stub — stays empty in phase 1e).
    pub invisible_to_me: HashSet<EndpointId>,
    /// Local user's self-presence override. Sticky, per-device; resets
    /// to [`PresenceOverride::Auto`] on browser reload.
    pub self_override: PresenceOverride,
    /// Idle threshold in ticks (default 6 min = 360).
    pub idle_ticks: Tick,
    /// Gone threshold in ticks (default 48 h = 172_800).
    pub gone_ticks: Tick,
}

impl Default for PresenceMeta {
    fn default() -> Self {
        Self {
            now: 0,
            last_seen: HashMap::new(),
            queue_depth: HashMap::new(),
            whispering_with: HashSet::new(),
            invisible_to_me: HashSet::new(),
            self_override: PresenceOverride::Auto,
            idle_ticks: DEFAULT_IDLE_TICKS,
            gone_ticks: DEFAULT_GONE_TICKS,
        }
    }
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

/// Maximum number of events the client will buffer while waiting for
/// missing predecessors. Prevents unbounded memory growth from malicious
/// or misbehaving peers sending events with chain gaps.
pub(crate) const MAX_CLIENT_PENDING: usize = 5_000;

/// Combined EventDag + ServerState + PendingBuffer, held in a single
/// StateActor via [`ManagedDag`](willow_state::ManagedDag).
///
/// Using `ManagedDag` ensures that DAG insertions and state
/// materialization are always atomic — it's structurally impossible
/// for the DAG and ServerState to diverge.
#[derive(Clone)]
pub struct DagState {
    /// The managed DAG that keeps EventDag, ServerState, and
    /// PendingBuffer in sync atomically.
    pub managed: willow_state::ManagedDag,
    /// Stashed DAGs for inactive servers, keyed by server ID.
    /// When switching servers, the current DAG is stashed and the
    /// target server's DAG is restored (or a fresh one is created).
    pub stashed: HashMap<String, willow_state::ManagedDag>,
}

impl DagState {
    // Convenience accessors for backward compatibility with code that
    // accessed the old `dag`, `pending`, and `synced` fields directly.

    /// Read-only access to the underlying EventDag.
    pub fn dag(&self) -> &willow_state::EventDag {
        self.managed.dag()
    }

    /// Read-only access to the pending buffer.
    pub fn pending(&self) -> &willow_state::PendingBuffer {
        self.managed.pending()
    }

    /// Whether the DAG has been populated (via genesis seed or sync batch).
    pub fn synced(&self) -> bool {
        self.managed.is_synced()
    }
}

impl Default for DagState {
    fn default() -> Self {
        Self {
            managed: willow_state::ManagedDag::empty(MAX_CLIENT_PENDING),
            stashed: HashMap::new(),
        }
    }
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
