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

/// Spec-default reaction shelf used until the per-channel LRU has 5
/// distinct emojis recorded. From `docs/specs/2026-04-19-ui-design/reactions-pins.md`
/// §Quick reactions: "Quick reactions default to `👍 ❤️ 🍃 💚 👀`".
const REACTION_RECENCY_DEFAULT: &[&str] = &["👍", "❤️", "🍃", "💚", "👀"];

/// Cap on the per-channel recency LRU. Spec calls for a 5-slot shelf.
pub const REACTION_RECENCY_CAP: usize = 5;

/// Chat session metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMeta {
    /// The current channel name (human-readable, e.g. "general").
    pub current_channel: String,
    /// Online peers.
    pub peers: Vec<EndpointId>,
    /// Per-channel reaction recency LRU (newest at the back).
    /// Capped at [`REACTION_RECENCY_CAP`] entries per channel; oldest
    /// emoji is dropped when the cap is reached. Drives the row's
    /// quick-react row, the action sheet's quick-react row, and the
    /// emoji picker's "recent" category. In-memory only (does not
    /// persist across app restarts) per phase-3c plan §Ambiguity
    /// decisions §1.
    pub reaction_recency: HashMap<String, VecDeque<String>>,
}

impl Default for ChatMeta {
    fn default() -> Self {
        Self {
            current_channel: crate::state::DEFAULT_CHANNEL.to_string(),
            peers: Vec::new(),
            reaction_recency: HashMap::new(),
        }
    }
}

impl ChatMeta {
    /// Note a successful reaction in `channel`. Moves `emoji` to the
    /// most-recent end of the channel's LRU; if the emoji is already
    /// in the LRU, it's removed first so the new entry shows MRU.
    /// Caps the LRU at [`REACTION_RECENCY_CAP`].
    ///
    /// Spec: `docs/specs/2026-04-19-ui-design/reactions-pins.md`
    /// §Quick reactions — "Override with the five most recent reactions
    /// used in *this channel*".
    pub fn note_reaction(&mut self, channel: &str, emoji: &str) {
        let entry = self
            .reaction_recency
            .entry(channel.to_string())
            .or_default();
        // Dedupe: remove any previous occurrence so the LRU stays
        // unique and the most-recent click wins ordering.
        entry.retain(|e| e != emoji);
        entry.push_back(emoji.to_string());
        while entry.len() > REACTION_RECENCY_CAP {
            entry.pop_front();
        }
    }

    /// Return the 5 most-recent reactions for `channel`, MRU-first.
    /// Falls back to the spec default until the LRU has 5 entries —
    /// the missing slots are filled from `REACTION_RECENCY_DEFAULT`
    /// in spec order, deduped against the LRU.
    pub fn recent_reactions(&self, channel: &str) -> Vec<String> {
        let lru = self.reaction_recency.get(channel);
        let mut out: Vec<String> = lru
            .map(|q| q.iter().rev().cloned().collect())
            .unwrap_or_default();
        for default in REACTION_RECENCY_DEFAULT {
            if out.len() >= REACTION_RECENCY_CAP {
                break;
            }
            let s = (*default).to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out.truncate(REACTION_RECENCY_CAP);
        out
    }
}

/// Maximum number of distinct peers tracked in
/// [`ProfileState::names`]. Caps the total-entry count of the map so a
/// peer churn (or attacker forging many `ProfileAnnounce` envelopes from
/// distinct identities) cannot grow the map without bound. Last-write
/// per-key already kept individual entries fresh; this caps the *count*.
///
/// Followup to [`SEC-V-05`] (issues #234, #429). Eviction is
/// least-recently-touched: entries inserted via
/// [`ProfileState::insert_name`] move to the back of `recency`; on
/// overflow, the front is dropped.
pub const MAX_PROFILE_NAMES: usize = 10_000;

/// Maximum number of distinct peers tracked in
/// [`NetworkMeta::typing_peers`]. Same threat model as
/// [`MAX_PROFILE_NAMES`]. Real users see far fewer concurrent typers; the
/// cap is a defensive ceiling.
pub const MAX_TYPING_PEERS: usize = 10_000;

/// Global profile display names (across all servers).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileState {
    /// EndpointId → display name.
    pub names: HashMap<EndpointId, String>,
    /// Recency queue (least-recently-touched at front). Maintained in
    /// lockstep with `names` by [`ProfileState::insert_name`]. Read-only
    /// for callers; do not mutate directly.
    pub(crate) recency: VecDeque<EndpointId>,
}

impl ProfileState {
    /// Look up a display name, falling back to truncated ID.
    pub fn display_name(&self, peer_id: &EndpointId) -> String {
        self.names
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| crate::util::truncate_peer_id(&peer_id.to_string()))
    }

    /// Insert or update a peer's display name, enforcing the
    /// total-entries cap [`MAX_PROFILE_NAMES`] via least-recently-touched
    /// eviction. Touching an existing entry moves it to the back of the
    /// recency queue.
    pub fn insert_name(&mut self, peer: EndpointId, name: String) {
        // Touch: drop any prior recency entry for this peer, then push to
        // back so most recent writes evict last.
        self.recency.retain(|p| p != &peer);
        self.recency.push_back(peer);
        self.names.insert(peer, name);
        while self.names.len() > MAX_PROFILE_NAMES {
            match self.recency.pop_front() {
                Some(evicted) => {
                    self.names.remove(&evicted);
                }
                None => break,
            }
        }
    }
}

/// Network connection metadata.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetworkMeta {
    /// Whether we're connected to the network.
    pub connected: bool,
    /// Peers currently typing: EndpointId → (channel_name, timestamp_ms).
    pub typing_peers: HashMap<EndpointId, (String, u64)>,
    /// Recency queue for `typing_peers` (least-recently-touched at
    /// front). Maintained in lockstep with `typing_peers` by
    /// [`NetworkMeta::insert_typing`] and [`NetworkMeta::sweep_typing`].
    /// Read-only for callers; do not mutate directly.
    pub(crate) typing_recency: VecDeque<EndpointId>,
    /// Last time we sent a typing indicator (for debouncing).
    pub last_typing_sent_ms: u64,
    /// Dedup set for `HistorySyncComplete` markers, keyed by
    /// `(provider, stream_generation)` (history-sync-eose spec, plan PR 5).
    ///
    /// A provider that restarts and re-streams picks a fresh random
    /// `stream_generation`, so a marker on a new generation re-emits while a
    /// repeat of an already-seen `(provider, generation)` is ignored — this is
    /// the equality-based dedup pinned decision 6 relies on. Read-only for
    /// callers; mutate via [`NetworkMeta::record_history_marker`].
    pub(crate) history_markers_seen: HashSet<(EndpointId, u64)>,
    /// Per-topic set of trusted providers that have sent a completion marker
    /// (history-sync-eose spec, plan PR 5). Keyed by the lowercase-hex of the
    /// marker's `topic_id`. Drives `still_pending` (connected trusted providers
    /// minus the ones that have completed). Read-only for callers; mutate via
    /// [`NetworkMeta::record_history_marker`].
    pub(crate) history_completed_by_topic: HashMap<String, HashSet<EndpointId>>,
}

impl NetworkMeta {
    /// Insert or update a peer's typing indicator, enforcing the
    /// total-entries cap [`MAX_TYPING_PEERS`] via least-recently-touched
    /// eviction. Touching an existing entry moves it to the back of the
    /// recency queue.
    pub fn insert_typing(&mut self, peer: EndpointId, channel: String, ts_ms: u64) {
        self.typing_recency.retain(|p| p != &peer);
        self.typing_recency.push_back(peer);
        self.typing_peers.insert(peer, (channel, ts_ms));
        while self.typing_peers.len() > MAX_TYPING_PEERS {
            match self.typing_recency.pop_front() {
                Some(evicted) => {
                    self.typing_peers.remove(&evicted);
                }
                None => break,
            }
        }
    }

    /// Drop typing entries older than `ttl_ms` (relative to `now_ms`).
    /// Keeps the recency queue in lockstep with the surviving entries.
    /// Called by the presence-tick driver in `connect.rs` so the map
    /// drains on a 1 Hz cadence even when no view is rendering.
    pub fn sweep_typing(&mut self, now_ms: u64, ttl_ms: u64) {
        self.typing_peers
            .retain(|_, (_, ts)| now_ms.saturating_sub(*ts) < ttl_ms);
        self.typing_recency
            .retain(|p| self.typing_peers.contains_key(p));
    }

    /// Record a `HistorySyncComplete` marker from a trusted provider, returning
    /// `true` if this is the first time we have seen this
    /// `(provider, stream_generation)` pair (history-sync-eose spec, plan PR 5).
    ///
    /// On a fresh pair we also note `provider` against `topic` so
    /// [`pending_history_providers`](Self::pending_history_providers) can
    /// exclude it from the still-streaming count. A repeated pair returns
    /// `false` and leaves both tables untouched, so the caller suppresses the
    /// duplicate `HistorySynced` emission. The caller is responsible for the
    /// trust gate — only markers from explicit `SyncProvider` peers reach here.
    pub(crate) fn record_history_marker(
        &mut self,
        provider: EndpointId,
        stream_generation: u64,
        topic: &str,
    ) -> bool {
        let fresh = self
            .history_markers_seen
            .insert((provider, stream_generation));
        if fresh {
            self.history_completed_by_topic
                .entry(topic.to_string())
                .or_default()
                .insert(provider);
        }
        fresh
    }

    /// Count how many of `connected_trusted` providers have **not** yet sent a
    /// completion marker for `topic` — the `still_pending` value for
    /// [`ClientEvent::HistorySynced`](crate::events::ClientEvent::HistorySynced).
    ///
    /// `connected_trusted` is the caller-supplied set of currently-connected
    /// peers holding an explicit `SyncProvider` grant; this method subtracts the
    /// providers already recorded complete for `topic`.
    pub(crate) fn pending_history_providers(
        &self,
        topic: &str,
        connected_trusted: &[EndpointId],
    ) -> usize {
        let completed = self.history_completed_by_topic.get(topic);
        connected_trusted
            .iter()
            .filter(|p| completed.map(|c| !c.contains(*p)).unwrap_or(true))
            .count()
    }
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
    /// Duration (in ticks ≈ seconds) of the most recent completed
    /// offline → online transition. Populated by `set_device_online`
    /// on the offline-to-online flip and exposed verbatim via
    /// `QueueView::last_offline_ticks` so the reconnection toast +
    /// welcome-back banner can gate on "≥ 60 s offline" without having
    /// to observe the pre-clear `offline_since_tick`. `None` until the
    /// first offline window completes.
    pub last_offline_ticks: Option<Tick>,
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
    /// transition to online, capturing the elapsed offline duration in
    /// `last_offline_ticks` so the reconnection toast + welcome-back
    /// banner can gate on "≥ 60 s offline" (spec §Reconnection toast).
    pub fn set_device_online(&mut self, online: bool) {
        if self.device_online && !online {
            self.offline_since_tick = Some(self.now);
        } else if !self.device_online && online {
            if let Some(since) = self.offline_since_tick {
                self.last_offline_ticks = Some(self.now.saturating_sub(since));
            }
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
pub(crate) const MAX_CLIENT_PENDING: usize = 5_000;

/// Maximum number of events sent in a single `WireMessage::SyncRequest`
/// reply (a `WireMessage::SyncBatch`). The first N events of the
/// deterministic topological sort. Receiver dedups via
/// `InsertError::Duplicate`. Long-term migration to heads-based sync is
/// tracked under #65; this cap remains until that lands.
pub(crate) const SYNC_REPLY_LIMIT: usize = 500;

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
    /// Cached materialized first-`SYNC_REPLY_LIMIT` events of the
    /// topological sort, used to answer `WireMessage::SyncRequest`.
    /// `None` = stale; will be recomputed on next read. Set to `None`
    /// by [`DagState::invalidate_sync_reply_cache`] after every
    /// successful DAG insertion (see GEN-08 / issue #268). Lives here
    /// rather than on `EventDag` because `willow-state` is intentionally
    /// pure / zero-I/O / no-interior-mutability — caching is a listener
    /// concern that belongs at the actor-state layer.
    pub(crate) sync_reply_cache: Option<Vec<willow_state::Event>>,
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

    /// Mark the SyncRequest-reply cache as stale. Must be called after
    /// every code path that successfully inserts an event into
    /// `self.managed` (i.e. whenever `topological_sort()` would return a
    /// different prefix). See GEN-08 / issue #268.
    pub(crate) fn invalidate_sync_reply_cache(&mut self) {
        self.sync_reply_cache = None;
    }

    /// Materialize the first [`SYNC_REPLY_LIMIT`] events of the DAG's
    /// topological sort, populating the cache on first call after
    /// invalidation. Returns a fresh `Vec` (cloned from the cache) ready
    /// to ship as a `WireMessage::SyncBatch` payload.
    ///
    /// Cost on cache hit: one `Vec<Event>` clone (~SYNC_REPLY_LIMIT
    /// shallow clones). Cost on miss: one `topological_sort()` over the
    /// whole DAG plus the same clone. Without this cache every
    /// `SyncRequest` paid the full O(N) sort even on a 50k-event DAG —
    /// see GEN-08 / issue #268.
    pub(crate) fn sync_reply_events(&mut self) -> Vec<willow_state::Event> {
        if let Some(cached) = &self.sync_reply_cache {
            return cached.clone();
        }
        let events: Vec<willow_state::Event> = self
            .managed
            .dag()
            .topological_sort()
            .into_iter()
            .take(SYNC_REPLY_LIMIT)
            .cloned()
            .collect();
        self.sync_reply_cache = Some(events.clone());
        events
    }
}

impl Default for DagState {
    fn default() -> Self {
        Self {
            managed: willow_state::ManagedDag::empty(MAX_CLIENT_PENDING),
            stashed: HashMap::new(),
            sync_reply_cache: None,
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

#[cfg(all(test, not(target_arch = "wasm32")))]
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

    // ───── #429 / [SEC-V-05] followup: LRU + TTL sweep ─────────────────

    /// Inserting `MAX_PROFILE_NAMES + 1` distinct peers must cap the map
    /// at `MAX_PROFILE_NAMES` and evict the least-recently-touched
    /// entry. Without the cap, an attacker forging many distinct
    /// `ProfileAnnounce` envelopes could grow `names` without bound.
    #[test]
    fn profile_state_lru_caps_total_entries() {
        let mut p = ProfileState::default();
        // Generate cap + 1 distinct senders. Capture the first one to
        // assert it's the evicted (oldest) entry.
        let first = peer();
        p.insert_name(first, "first".into());
        for i in 1..MAX_PROFILE_NAMES {
            p.insert_name(peer(), format!("name-{i}"));
        }
        assert_eq!(
            p.names.len(),
            MAX_PROFILE_NAMES,
            "exactly cap entries before overflow"
        );
        // One more inserts overflow-by-one — evicts `first`.
        let one_more = peer();
        p.insert_name(one_more, "overflow".into());
        assert_eq!(
            p.names.len(),
            MAX_PROFILE_NAMES,
            "size stays at cap after overflow insert"
        );
        assert!(
            !p.names.contains_key(&first),
            "least-recently-touched (first inserted, never re-touched) must be evicted"
        );
        assert!(
            p.names.contains_key(&one_more),
            "newest insert must be retained"
        );
        assert_eq!(
            p.recency.len(),
            p.names.len(),
            "recency queue stays in lockstep with names map"
        );
    }

    /// Re-inserting an existing peer touches it: the touched entry must
    /// move to the back of the recency queue so a later overflow does
    /// not evict it.
    #[test]
    fn profile_state_lru_touch_on_reinsert() {
        let mut p = ProfileState::default();
        let pinned = peer();
        p.insert_name(pinned, "v1".into());
        // Fill to cap with other peers.
        for i in 1..MAX_PROFILE_NAMES {
            p.insert_name(peer(), format!("name-{i}"));
        }
        // Touch `pinned` — moves it to back of recency queue.
        p.insert_name(pinned, "v2".into());
        // Overflow insert — must not evict `pinned` because it was just
        // touched. The peer at front of recency (next inserted after
        // `pinned`'s original insert) is the LRU now.
        p.insert_name(peer(), "overflow".into());
        assert!(
            p.names.contains_key(&pinned),
            "re-inserting a peer must refresh its LRU position"
        );
        assert_eq!(p.names.get(&pinned), Some(&"v2".to_string()));
    }

    /// `NetworkMeta::insert_typing` enforces the `MAX_TYPING_PEERS` cap
    /// the same way as profiles. Same threat model: attacker forges
    /// many `TypingIndicator` envelopes from distinct identities.
    #[test]
    fn network_meta_typing_lru_caps_total_entries() {
        let mut n = NetworkMeta::default();
        let first = peer();
        n.insert_typing(first, "general".into(), 1_000);
        for i in 1..MAX_TYPING_PEERS {
            n.insert_typing(peer(), "general".into(), 1_000 + i as u64);
        }
        assert_eq!(n.typing_peers.len(), MAX_TYPING_PEERS);
        // Overflow.
        let one_more = peer();
        n.insert_typing(one_more, "general".into(), 99_999);
        assert_eq!(n.typing_peers.len(), MAX_TYPING_PEERS);
        assert!(
            !n.typing_peers.contains_key(&first),
            "least-recently-touched typing entry must be evicted"
        );
        assert!(n.typing_peers.contains_key(&one_more));
        assert_eq!(
            n.typing_recency.len(),
            n.typing_peers.len(),
            "recency queue stays in lockstep with typing_peers"
        );
    }

    /// `sweep_typing(now, ttl)` drops every entry whose timestamp is
    /// older than `now - ttl` and keeps the recency queue in sync. With
    /// `ttl == TYPING_INDICATOR_TTL_MS` and a `now` 5+ s past every
    /// inserted timestamp, the map drains to empty.
    #[test]
    fn network_meta_sweep_drops_stale_entries_past_ttl() {
        let mut n = NetworkMeta::default();
        // Insert several entries at t = 1_000 ms.
        for i in 0..50 {
            n.insert_typing(peer(), format!("ch-{i}"), 1_000);
        }
        assert_eq!(n.typing_peers.len(), 50);
        assert_eq!(n.typing_recency.len(), 50);
        // Sweep at now = 1_000 + TTL + 1 ms. Every entry's age =
        // TTL + 1 ms which is > TTL, so all drop.
        n.sweep_typing(
            1_000 + crate::TYPING_INDICATOR_TTL_MS + 1,
            crate::TYPING_INDICATOR_TTL_MS,
        );
        assert!(
            n.typing_peers.is_empty(),
            "all entries past TTL must be swept"
        );
        assert!(
            n.typing_recency.is_empty(),
            "recency queue must drain alongside the map"
        );
    }

    /// Sweep keeps fresh entries and only drops stale ones — partial
    /// drain semantics. Entries inserted at `now - TTL/2` survive;
    /// entries at `now - TTL - 1` are dropped.
    #[test]
    fn network_meta_sweep_partial_drain() {
        let mut n = NetworkMeta::default();
        let stale = peer();
        let fresh = peer();
        let now = 10_000_u64;
        let ttl = crate::TYPING_INDICATOR_TTL_MS;
        // `stale`: inserted ttl + 1 ms ago — must drop.
        n.insert_typing(stale, "old".into(), now.saturating_sub(ttl + 1));
        // `fresh`: inserted ttl/2 ago — must survive.
        n.insert_typing(fresh, "new".into(), now.saturating_sub(ttl / 2));
        n.sweep_typing(now, ttl);
        assert!(!n.typing_peers.contains_key(&stale));
        assert!(n.typing_peers.contains_key(&fresh));
        assert_eq!(n.typing_recency.len(), 1);
        assert_eq!(n.typing_recency.front(), Some(&fresh));
    }
}
