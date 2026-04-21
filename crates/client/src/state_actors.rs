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

use std::collections::{HashMap, HashSet, VecDeque};

use willow_crypto::ChannelKey;
use willow_identity::EndpointId;
use willow_messaging::MessageId;

use crate::presence::{PresenceOverride, Tick, DEFAULT_GONE_TICKS, DEFAULT_IDLE_TICKS};
use crate::queue::{ArrivedSummary, RelayStatus};

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

/// Maximum number of peer-presence-history entries retained by
/// [`QueueMeta`]. Drop-oldest on overflow; cap enforced by
/// [`QueueMeta::record_presence`].
pub const QUEUE_HISTORY_CAP: usize = 2048;

/// Maximum number of `recent_arrivals` entries retained by
/// [`QueueMeta`]. Drop-oldest on overflow.
pub const QUEUE_ARRIVALS_CAP: usize = 512;

/// A single pending outbound message destined for a specific recipient.
///
/// Keyed by `(MessageId, EndpointId)` inside
/// [`QueueMeta::outbound`] so a fan-out message (one `MessageId`, N
/// recipients) occupies N entries — one per recipient awaiting
/// acknowledgement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueEntry {
    /// Identifier of the outbound message.
    pub message_id: MessageId,
    /// Recipient we are still waiting to ack the message.
    pub recipient: EndpointId,
    /// Wall-clock milliseconds at which the message was authored (HLC
    /// wall component).
    pub authored_at: u64,
    /// Tick at which the last retry attempt happened, if any.
    pub last_attempt_at: Option<Tick>,
    /// Human-readable last-attempt error, if any.
    pub last_attempt_error: Option<String>,
}

/// Central sync-queue state — owned by a dedicated state actor so the
/// message-row projection, offline strip, sync-queue screen, and queue
/// pill all read from one truth.
///
/// `PresenceMeta::queue_depth` delegates to this actor via
/// [`ClientHandle::_set_queue_depth`](crate::ClientHandle::_set_queue_depth)
/// so the two signals stay in lockstep without duplicate tracking.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueMeta {
    /// Monotonic tick counter shared with [`PresenceMeta`]. The
    /// tick-driver increments both.
    pub now: Tick,
    /// Pending outbound messages keyed by `(MessageId, Recipient)`.
    pub outbound: HashMap<(MessageId, EndpointId), QueueEntry>,
    /// Best-effort inbound-queue hints per peer — populated from peer
    /// heartbeat payloads when the heartbeat extension lands (`§Open
    /// questions §1`). Stays empty until then.
    pub inbound_hint_per_peer: HashMap<EndpointId, u32>,
    /// Rolling 24 h window of `ArrivedSummary` rows for the sync-queue
    /// screen's Recent section.
    pub recent_arrivals: VecDeque<ArrivedSummary>,
    /// Relay reachability snapshot fed from `Network::relay_status`.
    pub relay_status: RelayStatus,
    /// Device connectivity snapshot fed from `Network::device_online`
    /// + (on WASM) `window.addEventListener('online'/'offline')`.
    pub device_online: bool,
    /// Bounded `(peer, tick, reachable)` log used by
    /// [`derive_late_arrival`](crate::queue::derive_late_arrival).
    pub peer_presence_history: VecDeque<(EndpointId, Tick, bool)>,
    /// Per-peer `mark as read locally` markers. Keyed by peer ID and
    /// stamped with the tick at which the user pressed the button.
    pub marks: HashMap<EndpointId, Tick>,
    /// Tick at which the device last transitioned to offline. Drives
    /// the reconnection toast + welcome-back banner (Phase 2b Task 16):
    /// both gate on `offline_since_tick >= 60 s`. `None` while online.
    pub offline_since_tick: Option<Tick>,
}

impl QueueMeta {
    /// Enqueue a new outbound entry. No-op if the `(message_id,
    /// recipient)` pair already exists — duplicate enqueue is benign,
    /// the first wins and its retry bookkeeping is preserved.
    pub fn enqueue(&mut self, entry: QueueEntry) {
        let key = (entry.message_id.clone(), entry.recipient);
        self.outbound.entry(key).or_insert(entry);
    }

    /// Mark `peer` as having acknowledged `message_id`. Drops the entry
    /// from `outbound` if present.
    pub fn ack(&mut self, message_id: &MessageId, peer: EndpointId) {
        self.outbound.remove(&(message_id.clone(), peer));
    }

    /// Stamp the most-recent retry attempt for `(message_id, peer)`.
    /// Updates `last_attempt_at` + `last_attempt_error`. No-op if the
    /// entry no longer exists.
    pub fn mark_attempt(
        &mut self,
        message_id: &MessageId,
        peer: EndpointId,
        error: Option<String>,
    ) {
        if let Some(entry) = self.outbound.get_mut(&(message_id.clone(), peer)) {
            entry.last_attempt_at = Some(self.now);
            entry.last_attempt_error = error;
        }
    }

    /// Record an arrival bucket. Enforces [`QUEUE_ARRIVALS_CAP`] by
    /// dropping the oldest entry when full.
    pub fn record_arrival(&mut self, arrival: ArrivedSummary) {
        self.recent_arrivals.push_back(arrival);
        while self.recent_arrivals.len() > QUEUE_ARRIVALS_CAP {
            self.recent_arrivals.pop_front();
        }
    }

    /// Log a peer presence transition used by
    /// [`derive_late_arrival`](crate::queue::derive_late_arrival).
    /// Enforces [`QUEUE_HISTORY_CAP`] by dropping the oldest entry when
    /// full.
    pub fn record_presence(&mut self, peer: EndpointId, reachable: bool) {
        self.peer_presence_history
            .push_back((peer, self.now, reachable));
        while self.peer_presence_history.len() > QUEUE_HISTORY_CAP {
            self.peer_presence_history.pop_front();
        }
    }

    /// Decay arrivals older than `older_than_ticks` (default: 24 h).
    /// Called by the tick driver once per tick.
    pub fn decay_arrivals(&mut self, older_than_ticks: Tick) {
        self.recent_arrivals
            .retain(|a| self.now.saturating_sub(a.at_tick) < older_than_ticks);
    }

    /// Mark a peer's inbound queue as "read locally" at the current
    /// tick.
    pub fn mark_read(&mut self, peer: EndpointId) {
        self.marks.insert(peer, self.now);
    }

    /// Update the relay-reachability snapshot. Plain setter.
    pub fn set_relay_status(&mut self, status: RelayStatus) {
        self.relay_status = status;
    }

    /// Update the device-online snapshot. Stamps
    /// `offline_since_tick` on a transition to offline and clears it on
    /// transition to online.
    pub fn set_device_online(&mut self, online: bool) {
        if self.device_online && !online {
            self.offline_since_tick = Some(self.now);
        } else if !self.device_online && online {
            self.offline_since_tick = None;
        }
        self.device_online = online;
    }

    /// Aggregate per-peer outbound count — helper used by the queue
    /// view projection.
    pub fn peer_outbound_counts(&self) -> HashMap<EndpointId, u32> {
        let mut out: HashMap<EndpointId, u32> = HashMap::new();
        for (_, entry) in self.outbound.iter() {
            *out.entry(entry.recipient).or_insert(0) += 1;
        }
        out
    }

    /// Derive a [`DeliveryState`](willow_messaging::DeliveryState) for
    /// the given message-id string from the in-memory `outbound` map.
    ///
    /// This is the projection-facing shim used by
    /// [`compute_messages_view`](crate::views::compute_messages_view)
    /// while the real `MessageStore::delivery_state` plumbing is
    /// deferred (see plan §Open questions §3). Implements the
    /// `MessageStore::delivery_state` *contract* using QueueMeta's
    /// outbound tracking:
    ///
    /// - No entries for `message_id` → `Delivered` (permissive default).
    /// - One or more entries → `PendingAllRecipients` keyed on the
    ///   recipient set.
    pub fn delivery_state_by_id_str(
        &self,
        message_id_str: &str,
    ) -> willow_messaging::DeliveryState {
        let mut pending: HashSet<EndpointId> = HashSet::new();
        for ((mid, _), entry) in self.outbound.iter() {
            if mid.to_string() == message_id_str {
                pending.insert(entry.recipient);
            }
        }
        if pending.is_empty() {
            willow_messaging::DeliveryState::Delivered
        } else {
            willow_messaging::DeliveryState::PendingAllRecipients(pending)
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
const MAX_CLIENT_PENDING: usize = 5_000;

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
    /// Sync-queue state (Phase 2b).
    pub queue_meta: willow_actor::state::StateRef<QueueMeta>,
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
            queue_meta: self.queue_meta.clone(),
        }
    }
}

// ───── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    fn peer() -> EndpointId {
        Identity::generate().endpoint_id()
    }

    #[test]
    fn queue_meta_enqueue_and_ack_drain() {
        let mut qm = QueueMeta::default();
        let alice = peer();
        let bob = peer();
        let msg = MessageId::new();
        qm.enqueue(QueueEntry {
            message_id: msg.clone(),
            recipient: alice,
            authored_at: 1_000,
            last_attempt_at: None,
            last_attempt_error: None,
        });
        qm.enqueue(QueueEntry {
            message_id: msg.clone(),
            recipient: bob,
            authored_at: 1_000,
            last_attempt_at: None,
            last_attempt_error: None,
        });
        assert_eq!(qm.outbound.len(), 2);
        qm.ack(&msg, alice);
        assert_eq!(qm.outbound.len(), 1);
        qm.ack(&msg, bob);
        assert!(qm.outbound.is_empty());
    }

    #[test]
    fn queue_meta_history_cap_enforced() {
        let mut qm = QueueMeta::default();
        for _ in 0..QUEUE_HISTORY_CAP + 5 {
            qm.record_presence(peer(), true);
        }
        assert_eq!(qm.peer_presence_history.len(), QUEUE_HISTORY_CAP);
    }

    #[test]
    fn queue_meta_arrivals_cap_enforced() {
        let mut qm = QueueMeta::default();
        for _ in 0..QUEUE_ARRIVALS_CAP + 5 {
            qm.record_arrival(ArrivedSummary {
                peer_id: peer(),
                at_tick: 0,
                count: 1,
                preview: None,
            });
        }
        assert_eq!(qm.recent_arrivals.len(), QUEUE_ARRIVALS_CAP);
    }

    #[test]
    fn queue_meta_set_device_online_stamps_offline_since() {
        let mut qm = QueueMeta {
            now: 10,
            ..QueueMeta::default()
        };
        qm.set_device_online(true); // idempotent: already online by default
        assert_eq!(qm.offline_since_tick, None);
        qm.set_device_online(false);
        assert_eq!(qm.offline_since_tick, Some(10));
        qm.now = 42;
        qm.set_device_online(true);
        assert_eq!(qm.offline_since_tick, None);
    }

    #[test]
    fn queue_meta_peer_outbound_counts() {
        let mut qm = QueueMeta::default();
        let alice = peer();
        let bob = peer();
        let m1 = MessageId::new();
        let m2 = MessageId::new();
        qm.enqueue(QueueEntry {
            message_id: m1,
            recipient: alice,
            authored_at: 1,
            last_attempt_at: None,
            last_attempt_error: None,
        });
        qm.enqueue(QueueEntry {
            message_id: m2.clone(),
            recipient: alice,
            authored_at: 2,
            last_attempt_at: None,
            last_attempt_error: None,
        });
        qm.enqueue(QueueEntry {
            message_id: m2,
            recipient: bob,
            authored_at: 2,
            last_attempt_at: None,
            last_attempt_error: None,
        });
        let counts = qm.peer_outbound_counts();
        assert_eq!(counts.get(&alice), Some(&2));
        assert_eq!(counts.get(&bob), Some(&1));
    }
}
