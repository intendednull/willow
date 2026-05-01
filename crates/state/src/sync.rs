//! Sync protocol types and pending event buffer.
//!
//! [`HeadsSummary`] is the compact representation of a peer's DAG state,
//! used for efficient sync. [`Snapshot`] is a frozen checkpoint of the DAG
//! and materialized state. [`PendingBuffer`] buffers events that arrive
//! before their per-author chain predecessors.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;

use crate::event::Event;
use crate::hash::EventHash;
use crate::server::ServerState;

// ───── Sync types ──────────────────────────────────────────────────────────

/// Compact representation of what a peer knows about the DAG.
/// Maps each known author to their latest seq number and head hash.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadsSummary {
    pub heads: BTreeMap<EndpointId, AuthorHead>,
}

/// A single author's head state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorHead {
    /// Latest seq number for this author.
    pub seq: u64,
    /// Hash of the latest event from this author.
    pub hash: EventHash,
}

/// Wire-level sync message.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SyncMessage {
    /// "Here's what I have."
    Advertise(HeadsSummary),
    /// "I need events from these authors after these seq numbers."
    Request(Vec<AuthorRequest>),
    /// "Here are events you're missing."
    Response(Vec<Event>),
}

/// A request for events from a specific author.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthorRequest {
    /// The author to request events from.
    pub author: EndpointId,
    /// Send events with seq > after_seq.
    pub after_seq: u64,
}

// ───── Snapshot ───────────────────────────────────────────────────────────

/// A frozen checkpoint of the DAG and materialized state.
///
/// Used for bootstrapping far-behind peers: instead of replaying the full
/// event history, send a snapshot plus any events created after it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// The materialized state at this point.
    pub state: ServerState,
    /// The heads (author → latest seq + hash) at snapshot time.
    /// These define exactly which events are included.
    pub heads: HeadsSummary,
    /// SHA-256 hash of the canonical serialization of (state, heads).
    /// Used for verification.
    pub hash: EventHash,
}

/// Helper for computing the snapshot hash with deterministic ordering.
/// Uses sorted vectors for heads to ensure consistent hashing.
#[derive(Serialize)]
struct SnapshotHashInput<'a> {
    state: &'a ServerState,
    /// Sorted by author to ensure deterministic serialization.
    heads: Vec<(&'a EndpointId, &'a AuthorHead)>,
}

impl Snapshot {
    /// Create a new snapshot, computing the verification hash.
    ///
    /// The hash is computed from a canonical serialization of (state, heads).
    /// All collection types use `BTreeMap`/`BTreeSet` for deterministic
    /// iteration order, so the hash is consistent across processes.
    pub fn new(state: ServerState, heads: HeadsSummary) -> Self {
        let mut sorted_heads: Vec<_> = heads.heads.iter().collect();
        sorted_heads.sort_by_key(|(id, _)| id.as_bytes());
        let input = SnapshotHashInput {
            state: &state,
            heads: sorted_heads,
        };
        let bytes = bincode::serialize(&input).expect("snapshot serialization should not fail");
        let hash = EventHash::from_bytes(&bytes);
        Self { state, heads, hash }
    }
}

// ───── Chain comparison ────────────────────────────────────────────────────

/// Result of comparing our head for an author against a peer's head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainStatus {
    /// Their chain extends ours (they have newer events).
    Ahead { new_events: u64 },
    /// Our chain extends theirs (we have events they haven't seen).
    Behind { missing_events: u64 },
    /// Same head — fully synced for this author.
    Synced,
    /// Same seq, different hash — equivocation detected.
    /// The author signed two different events at the same seq number.
    Forked,
}

/// Compare our head for an author against a peer's head.
pub fn compare_chains(our_head: &AuthorHead, their_head: &AuthorHead) -> ChainStatus {
    if our_head.hash == their_head.hash {
        return ChainStatus::Synced;
    }
    if their_head.seq > our_head.seq {
        return ChainStatus::Ahead {
            new_events: their_head.seq - our_head.seq,
        };
    }
    if their_head.seq < our_head.seq {
        return ChainStatus::Behind {
            missing_events: our_head.seq - their_head.seq,
        };
    }
    // Same seq, different hash — equivocation.
    ChainStatus::Forked
}

// ───── Pending buffer ──────────────────────────────────────────────────────

/// Default maximum age for pending entries before they are evicted (1 hour).
pub const DEFAULT_PENDING_MAX_AGE_MS: u64 = 60 * 60 * 1000;

/// Default maximum number of pending events across all prev-hash buckets.
pub const DEFAULT_PENDING_MAX_ENTRIES: usize = 10_000;

/// Default divisor used to derive a per-author sub-cap from the total
/// `max_entries`. With the default total of 10_000 and a divisor of 50,
/// each author may hold at most 200 pending entries, so a single
/// misbehaving signer cannot consume the whole buffer (SEC-V-08).
pub const DEFAULT_PENDING_PER_AUTHOR_DIVISOR: usize = 50;

/// Compute the default per-author sub-cap from a total `max_entries`.
///
/// Returns at least `1` so the buffer remains usable when callers
/// configure absurdly small totals (e.g. in tests).
fn default_per_author_cap(max_entries: usize) -> usize {
    (max_entries / DEFAULT_PENDING_PER_AUTHOR_DIVISOR).max(1)
}

/// A single pending entry — the event plus the time it was buffered.
///
/// `inserted_at_ms` is `None` when the caller did not supply a timestamp
/// (via legacy [`PendingBuffer::buffer_for_prev`]). Such entries are
/// immune to age-based eviction but still subject to capacity eviction.
#[derive(Clone, Debug)]
struct PendingEntry {
    event: Event,
    /// Wall-clock time (ms since epoch) the entry was buffered. `None`
    /// means the caller did not record a time and age eviction is skipped.
    inserted_at_ms: Option<u64>,
}

/// Buffer for events that arrive before their per-author chain predecessors.
///
/// Per-author chain gaps (`prev` references an unknown event) are hard gaps
/// — the event is buffered. Cross-author dep gaps (`deps` references an
/// unknown event) are soft — the event is accepted and the dep is recorded
/// for background fetching.
///
/// Two independent eviction policies keep the buffer bounded:
///
/// * **Age-based** — entries older than `max_age_ms` are dropped when any
///   method that carries a clock is called ([`buffer_for_prev_at`] or
///   [`evict_expired`]).
/// * **Capacity-based** — after any insertion, if the total pending count
///   exceeds `max_entries`, the oldest entries are evicted.
///
/// Legacy callers that use [`buffer_for_prev`] (without a timestamp) get
/// capacity-based eviction only. Each eviction is logged at `warn!` level
/// with the event hash and (for age eviction) the entry's age.
#[derive(Clone, Debug, Default)]
pub struct PendingBuffer {
    /// Events waiting for a missing `prev` hash.
    waiting_on_prev: BTreeMap<EventHash, Vec<PendingEntry>>,
    /// Cross-author deps we've seen referenced but don't have yet.
    missing_deps: BTreeSet<EventHash>,
    /// Optional maximum number of pending events. When set,
    /// inserts auto-evict the oldest entries to stay within limit.
    max_entries: Option<usize>,
    /// Optional per-author sub-cap. When set, a single author may hold at
    /// most this many pending entries; further inserts from that author
    /// are dropped without disturbing other authors' buckets (SEC-V-08).
    /// Derived from `max_entries` by default — see
    /// [`DEFAULT_PENDING_PER_AUTHOR_DIVISOR`].
    max_per_author: Option<usize>,
    /// Optional maximum age in ms before an entry is evicted. Only applies
    /// to entries inserted with a timestamp via [`buffer_for_prev_at`].
    max_age_ms: Option<u64>,
    /// Cached total count of pending events for O(1) lookups.
    cached_count: usize,
    /// Per-author pending counts, used to enforce `max_per_author`.
    /// Kept in sync with `waiting_on_prev` on every insert/resolve/evict.
    per_author_count: BTreeMap<EndpointId, usize>,
    /// Authors we have already warned about hitting their sub-cap. Used to
    /// rate-limit the per-author drop warning so one chatty offender does
    /// not flood logs (mirrors the `warned_full` pattern in the relay).
    warned_full_authors: BTreeSet<EndpointId>,
}

impl PendingBuffer {
    /// Create a new empty buffer with no capacity or age limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a buffer with a maximum pending event capacity.
    ///
    /// When the total pending count exceeds this limit after an insertion,
    /// the buffer automatically evicts entries to stay within bounds. No
    /// age-based eviction is applied — use [`PendingBuffer::with_limits`]
    /// if you also want timeout-based eviction.
    ///
    /// A per-author sub-cap is derived automatically from `max_entries`
    /// (see [`DEFAULT_PENDING_PER_AUTHOR_DIVISOR`]) so a single misbehaving
    /// signer cannot consume the whole buffer (SEC-V-08).
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            max_entries: Some(max_entries),
            max_per_author: Some(default_per_author_cap(max_entries)),
            ..Self::default()
        }
    }

    /// Create a buffer with both a maximum pending event count and a
    /// maximum age (in ms) before entries are evicted.
    ///
    /// Sensible defaults are [`DEFAULT_PENDING_MAX_ENTRIES`] and
    /// [`DEFAULT_PENDING_MAX_AGE_MS`].
    ///
    /// A per-author sub-cap is derived automatically from `max_entries`
    /// — see [`PendingBuffer::with_capacity`].
    pub fn with_limits(max_entries: usize, max_age_ms: u64) -> Self {
        Self {
            max_entries: Some(max_entries),
            max_per_author: Some(default_per_author_cap(max_entries)),
            max_age_ms: Some(max_age_ms),
            ..Self::default()
        }
    }

    /// Override the per-author sub-cap. Mainly intended for tests; the
    /// default derived by [`PendingBuffer::with_capacity`] /
    /// [`PendingBuffer::with_limits`] is correct for production callers.
    pub fn with_per_author_cap(mut self, max_per_author: usize) -> Self {
        self.max_per_author = Some(max_per_author);
        self
    }

    /// Buffer an event that's waiting for a prev hash to arrive.
    ///
    /// Legacy entry point: entries inserted this way have no timestamp and
    /// so are exempt from age-based eviction (but still subject to capacity
    /// eviction). Use [`PendingBuffer::buffer_for_prev_at`] when the caller
    /// can supply a monotonic wall-clock.
    pub fn buffer_for_prev(&mut self, prev_hash: EventHash, event: Event) {
        self.insert_entry(prev_hash, event, None);
    }

    /// Buffer an event with a wall-clock timestamp. Evicts expired entries
    /// first, then enforces the capacity limit.
    pub fn buffer_for_prev_at(&mut self, prev_hash: EventHash, event: Event, now_ms: u64) {
        // Sweep age-expired entries before inserting the new one so the
        // capacity check only considers live pending entries.
        self.evict_expired(now_ms);
        self.insert_entry(prev_hash, event, Some(now_ms));
    }

    fn insert_entry(&mut self, prev_hash: EventHash, event: Event, inserted_at_ms: Option<u64>) {
        let author = event.author;

        // Per-author sub-cap (SEC-V-08): one signer cannot fill the
        // whole buffer with unresolved events. Drop the new event when
        // its author is already at the sub-cap so other authors retain
        // room. Warn at most once per author per buffer instance.
        if let Some(per_author_limit) = self.max_per_author {
            let current = self.per_author_count.get(&author).copied().unwrap_or(0);
            if current >= per_author_limit {
                if self.warned_full_authors.insert(author) {
                    tracing::warn!(
                        author = %author,
                        per_author_cap = per_author_limit,
                        event_hash = %event.hash,
                        "pending buffer: per-author sub-cap reached; dropping further events from this author"
                    );
                }
                return;
            }
        }

        self.waiting_on_prev
            .entry(prev_hash)
            .or_default()
            .push(PendingEntry {
                event,
                inserted_at_ms,
            });
        self.cached_count += 1;
        *self.per_author_count.entry(author).or_insert(0) += 1;

        if let Some(limit) = self.max_entries {
            let evicted = self.evict_to(limit);
            if evicted > 0 {
                tracing::warn!(
                    evicted,
                    buffer_size = self.cached_count,
                    "pending buffer at capacity; dropped oldest events"
                );
            }
        }
    }

    /// Decrement the per-author count for `author`, removing the entry
    /// when it hits zero. Saturating subtraction keeps the bookkeeping
    /// safe even if a caller double-resolves the same event.
    fn decrement_author(&mut self, author: &EndpointId) {
        if let Some(count) = self.per_author_count.get_mut(author) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.per_author_count.remove(author);
            }
        }
    }

    /// Record a cross-author dep that we don't have yet.
    pub fn record_missing_dep(&mut self, hash: EventHash) {
        self.missing_deps.insert(hash);
    }

    /// Called when a new event is inserted into the DAG.
    /// Returns any buffered events whose `prev` is now satisfied.
    pub fn resolve(&mut self, inserted_hash: &EventHash) -> Vec<Event> {
        self.missing_deps.remove(inserted_hash);
        let entries = self
            .waiting_on_prev
            .remove(inserted_hash)
            .unwrap_or_default();
        self.cached_count = self.cached_count.saturating_sub(entries.len());
        for entry in &entries {
            self.decrement_author(&entry.event.author);
        }
        entries.into_iter().map(|e| e.event).collect()
    }

    /// Number of missing cross-author deps being tracked.
    pub fn missing_dep_count(&self) -> usize {
        self.missing_deps.len()
    }

    /// Number of events waiting for prev chain predecessors.
    pub fn pending_count(&self) -> usize {
        self.cached_count
    }

    /// Evict all pending entries whose age exceeds `max_age_ms` (if set)
    /// relative to `now_ms`. Entries inserted without a timestamp are
    /// never evicted by age. Returns the number of events evicted.
    pub fn evict_expired(&mut self, now_ms: u64) -> usize {
        let max_age = match self.max_age_ms {
            Some(v) => v,
            None => return 0,
        };
        let mut evicted = 0usize;
        let mut evicted_authors: Vec<EndpointId> = Vec::new();
        let mut empty_keys: Vec<EventHash> = Vec::new();
        for (prev_hash, entries) in self.waiting_on_prev.iter_mut() {
            entries.retain(|entry| {
                let Some(inserted_at) = entry.inserted_at_ms else {
                    return true; // no timestamp → immune to age eviction
                };
                let age = now_ms.saturating_sub(inserted_at);
                if age > max_age {
                    tracing::warn!(
                        event_hash = %entry.event.hash,
                        age_ms = age,
                        max_age_ms = max_age,
                        "pending buffer: evicting aged-out event"
                    );
                    evicted += 1;
                    evicted_authors.push(entry.event.author);
                    false
                } else {
                    true
                }
            });
            if entries.is_empty() {
                empty_keys.push(*prev_hash);
            }
        }
        for k in empty_keys {
            self.waiting_on_prev.remove(&k);
        }
        self.cached_count = self.cached_count.saturating_sub(evicted);
        for author in evicted_authors {
            self.decrement_author(&author);
        }
        evicted
    }

    /// Evict pending entries to keep the buffer bounded.
    ///
    /// Removes the oldest entries (by insertion order approximation)
    /// until the total pending count is at or below `max_entries`.
    /// Returns the number of events evicted.
    pub fn evict_to(&mut self, max_entries: usize) -> usize {
        let mut evicted = 0;
        while self.cached_count > max_entries {
            // Remove an arbitrary entry.
            if let Some(key) = self.waiting_on_prev.keys().next().cloned() {
                if let Some(entries) = self.waiting_on_prev.remove(&key) {
                    for entry in &entries {
                        tracing::warn!(
                            event_hash = %entry.event.hash,
                            "pending buffer: evicting event to enforce capacity"
                        );
                    }
                    let n = entries.len();
                    evicted += n;
                    self.cached_count = self.cached_count.saturating_sub(n);
                    // Keep per-author bookkeeping in sync with the
                    // bucket we just removed (SEC-V-08).
                    let evicted_authors: Vec<EndpointId> =
                        entries.iter().map(|e| e.event.author).collect();
                    for author in evicted_authors {
                        self.decrement_author(&author);
                    }
                }
            } else {
                break;
            }
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::EventDag;
    use crate::event::EventKind;
    use crate::materialize::materialize;
    use willow_identity::Identity;

    fn test_dag(id: &Identity) -> EventDag {
        let mut dag = EventDag::new();
        let genesis = dag.create_event(
            id,
            EventKind::CreateServer {
                name: "Test".into(),
            },
            vec![],
            0,
        );
        dag.insert(genesis).unwrap();
        dag
    }

    // ── HeadsSummary tests ─────────────────────────────────────────

    #[test]
    fn heads_summary_reflects_dag() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let id_c = Identity::generate();
        let mut dag = test_dag(&id_a);

        let b1 = dag.create_event(
            &id_b,
            EventKind::SetProfile {
                display_name: "b".into(),
            },
            vec![],
            0,
        );
        dag.insert(b1).unwrap();

        let c1 = dag.create_event(
            &id_c,
            EventKind::SetProfile {
                display_name: "c".into(),
            },
            vec![],
            0,
        );
        dag.insert(c1).unwrap();

        let summary = dag.heads_summary();
        assert_eq!(summary.heads.len(), 3);
        assert_eq!(summary.heads[&id_a.endpoint_id()].seq, 1);
        assert_eq!(summary.heads[&id_b.endpoint_id()].seq, 1);
        assert_eq!(summary.heads[&id_c.endpoint_id()].seq, 1);
    }

    #[test]
    fn events_since_returns_delta() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);

        for i in 0..4 {
            let e = dag.create_event(
                &id,
                EventKind::SetProfile {
                    display_name: format!("n{i}"),
                },
                vec![],
                0,
            );
            dag.insert(e).unwrap();
        }
        // Author has seq 1-5 (genesis + 4). Request since seq 3.
        let mut their_heads = BTreeMap::new();
        their_heads.insert(id.endpoint_id(), 3);

        let delta = dag.events_since(&their_heads, None);
        assert_eq!(delta.len(), 2); // seq 4 and 5
    }

    #[test]
    fn events_since_respects_limit() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);

        for i in 0..10 {
            let e = dag.create_event(
                &id,
                EventKind::SetProfile {
                    display_name: format!("n{i}"),
                },
                vec![],
                0,
            );
            dag.insert(e).unwrap();
        }
        // Author has seq 1-11 (genesis + 10). Request all with limit 5.
        let their_heads = BTreeMap::new();
        let delta = dag.events_since(&their_heads, Some(5));
        assert_eq!(delta.len(), 5, "should be capped at limit");
    }

    #[test]
    fn events_since_unknown_author() {
        let id = Identity::generate();
        let dag = test_dag(&id);

        let unknown = Identity::generate().endpoint_id();
        let mut their_heads = BTreeMap::new();
        their_heads.insert(unknown, 5);

        let delta = dag.events_since(&their_heads, None);
        // Unknown author is skipped. We return our events they don't have.
        // Since they didn't mention our author, we return all events for our author.
        assert_eq!(delta.len(), 1); // genesis
    }

    #[test]
    fn events_since_new_author() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let mut dag = test_dag(&id_a);

        let b1 = dag.create_event(
            &id_b,
            EventKind::SetProfile {
                display_name: "b".into(),
            },
            vec![],
            0,
        );
        dag.insert(b1).unwrap();

        // Requester only knows about id_a at seq 1.
        let mut their_heads = BTreeMap::new();
        their_heads.insert(id_a.endpoint_id(), 1);

        let delta = dag.events_since(&their_heads, None);
        // They're missing id_b entirely.
        assert_eq!(delta.len(), 1); // b1
    }

    #[test]
    fn sync_round_trip() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();

        // DAG 1: has genesis + a2
        let mut dag1 = test_dag(&id_a);
        let a2 = dag1.create_event(
            &id_a,
            EventKind::SetProfile {
                display_name: "a2".into(),
            },
            vec![],
            0,
        );
        dag1.insert(a2).unwrap();

        // DAG 2: has genesis + b1
        let mut dag2 = test_dag(&id_a);
        let b1 = dag2.create_event(
            &id_b,
            EventKind::SetProfile {
                display_name: "b1".into(),
            },
            vec![],
            0,
        );
        dag2.insert(b1.clone()).unwrap();

        // Exchange heads.
        let heads1 = dag1.heads_summary();
        let heads2 = dag2.heads_summary();

        // DAG 1 gets events from DAG 2.
        let their_heads_map: BTreeMap<_, _> =
            heads1.heads.iter().map(|(k, v)| (*k, v.seq)).collect();
        let for_dag1 = dag2.events_since(&their_heads_map, None);
        for event in for_dag1 {
            let _ = dag1.insert(event.clone());
        }

        // DAG 2 gets events from DAG 1.
        let their_heads_map: BTreeMap<_, _> =
            heads2.heads.iter().map(|(k, v)| (*k, v.seq)).collect();
        let for_dag2 = dag1.events_since(&their_heads_map, None);
        for event in for_dag2 {
            let _ = dag2.insert(event.clone());
        }

        // Both DAGs should now have the same events.
        let s1 = materialize(&dag1);
        let s2 = materialize(&dag2);
        assert_eq!(s1.profiles, s2.profiles);
    }

    // ── Chain comparison tests ─────────────────────────────────────

    #[test]
    fn compare_chains_synced() {
        let hash = EventHash::from_bytes(b"same");
        let a = AuthorHead { seq: 5, hash };
        let b = AuthorHead { seq: 5, hash };
        assert_eq!(compare_chains(&a, &b), ChainStatus::Synced);
    }

    #[test]
    fn compare_chains_ahead() {
        let a = AuthorHead {
            seq: 3,
            hash: EventHash::from_bytes(b"a"),
        };
        let b = AuthorHead {
            seq: 5,
            hash: EventHash::from_bytes(b"b"),
        };
        assert_eq!(compare_chains(&a, &b), ChainStatus::Ahead { new_events: 2 });
    }

    #[test]
    fn compare_chains_behind() {
        let a = AuthorHead {
            seq: 5,
            hash: EventHash::from_bytes(b"a"),
        };
        let b = AuthorHead {
            seq: 3,
            hash: EventHash::from_bytes(b"b"),
        };
        assert_eq!(
            compare_chains(&a, &b),
            ChainStatus::Behind { missing_events: 2 }
        );
    }

    #[test]
    fn compare_chains_forked() {
        let a = AuthorHead {
            seq: 5,
            hash: EventHash::from_bytes(b"version_a"),
        };
        let b = AuthorHead {
            seq: 5,
            hash: EventHash::from_bytes(b"version_b"),
        };
        assert_eq!(compare_chains(&a, &b), ChainStatus::Forked);
    }

    // ── PendingBuffer tests ────────────────────────────────────────

    #[test]
    fn buffer_and_resolve() {
        let mut buf = PendingBuffer::new();
        let id = Identity::generate();
        let event = Event::new(
            &id,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer {
                name: "test".into(),
            },
            0,
        );
        let waiting_on = EventHash::from_bytes(b"predecessor");
        buf.buffer_for_prev(waiting_on, event.clone());
        assert_eq!(buf.pending_count(), 1);

        let resolved = buf.resolve(&waiting_on);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].hash, event.hash);
        assert_eq!(buf.pending_count(), 0);
    }

    #[test]
    fn missing_deps_recorded() {
        let mut buf = PendingBuffer::new();
        let dep = EventHash::from_bytes(b"missing");
        buf.record_missing_dep(dep);
        assert_eq!(buf.missing_dep_count(), 1);

        buf.resolve(&dep);
        assert_eq!(buf.missing_dep_count(), 0);
    }

    #[test]
    fn resolve_cascading() {
        let mut buf = PendingBuffer::new();
        let id = Identity::generate();

        let hash_a = EventHash::from_bytes(b"a");
        let hash_b = EventHash::from_bytes(b"b");

        // Event C waits on B, event B waits on A.
        let event_b = Event::new(
            &id,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer { name: "b".into() },
            0,
        );
        let event_c = Event::new(
            &id,
            2,
            event_b.hash,
            vec![],
            EventKind::SetProfile {
                display_name: "c".into(),
            },
            0,
        );

        buf.buffer_for_prev(hash_a, event_b.clone());
        buf.buffer_for_prev(hash_b, event_c.clone());

        // Resolve A → gets B.
        let resolved_a = buf.resolve(&hash_a);
        assert_eq!(resolved_a.len(), 1);
        assert_eq!(resolved_a[0].hash, event_b.hash);

        // Resolve B → gets C.
        let resolved_b = buf.resolve(&hash_b);
        assert_eq!(resolved_b.len(), 1);
        assert_eq!(resolved_b[0].hash, event_c.hash);
    }

    // ── Issue #76: Self-enforcing PendingBuffer capacity ────────────

    #[test]
    fn pending_buffer_auto_evicts_when_limit_exceeded() {
        // Use a generous per-author cap so this test exercises the
        // global capacity-eviction path rather than the SEC-V-08
        // per-author sub-cap.
        let mut buf = PendingBuffer::with_capacity(50).with_per_author_cap(1_000);
        let id = Identity::generate();
        // Buffer 100 events with unique prev hashes (simulating gaps).
        for i in 0u64..100 {
            let prev = EventHash::from_bytes(&i.to_le_bytes());
            let event = Event::new(
                &id,
                i + 1,
                prev,
                vec![],
                EventKind::SetProfile {
                    display_name: format!("n{i}"),
                },
                0,
            );
            let unique_prev = EventHash::from_bytes(&(i + 1000).to_le_bytes());
            buf.buffer_for_prev(unique_prev, event);
        }
        // Buffer should auto-evict to stay within capacity.
        assert!(
            buf.pending_count() <= 50,
            "pending_count {} should be <= 50",
            buf.pending_count()
        );
    }

    #[test]
    fn pending_buffer_unlimited_when_no_capacity_set() {
        let mut buf = PendingBuffer::new();
        let id = Identity::generate();
        for i in 0u64..200 {
            let event = Event::new(
                &id,
                i + 1,
                EventHash::from_bytes(&i.to_le_bytes()),
                vec![],
                EventKind::SetProfile {
                    display_name: format!("n{i}"),
                },
                0,
            );
            buf.buffer_for_prev(EventHash::from_bytes(&(i + 500).to_le_bytes()), event);
        }
        // Without capacity, buffer grows unbounded.
        assert_eq!(buf.pending_count(), 200);
    }

    // ── Issue #41: Snapshot hash determinism ────────────────────────

    #[test]
    fn snapshot_hash_deterministic_regardless_of_insertion_order() {
        use crate::event::Permission;

        // Build two DAGs with identical events but different author ordering.
        // If ServerState used HashMap, the snapshot hashes could differ
        // because iteration order is non-deterministic. With BTreeMap,
        // serialization is deterministic and hashes always match.
        let owner = Identity::generate();
        let peer_a = Identity::generate();
        let peer_b = Identity::generate();
        let peer_c = Identity::generate();

        let mut dag = test_dag(&owner);

        // Add channels in specific order.
        for (ch_id, ch_name) in [("ch-1", "alpha"), ("ch-2", "beta"), ("ch-3", "gamma")] {
            let e = dag.create_event(
                &owner,
                EventKind::CreateChannel {
                    channel_id: ch_id.into(),
                    name: ch_name.into(),
                    kind: crate::types::ChannelKind::Text,
                    ephemeral: None,
                },
                vec![],
                0,
            );
            dag.insert(e).unwrap();
        }

        // Add roles.
        for (role_id, role_name) in [("r-1", "Mod"), ("r-2", "VIP")] {
            let e = dag.create_event(
                &owner,
                EventKind::CreateRole {
                    role_id: role_id.into(),
                    name: role_name.into(),
                },
                vec![],
                0,
            );
            dag.insert(e).unwrap();
        }

        // Grant permissions to multiple peers, recording each grant
        // hash so peer SetProfile events can causally depend on it.
        // (SetProfile requires membership — issue #177 — so the grant
        // must topologically precede the SetProfile.)
        let mut grant_hashes: std::collections::BTreeMap<_, _> = std::collections::BTreeMap::new();
        for peer in [&peer_a, &peer_b, &peer_c] {
            let e = dag.create_event(
                &owner,
                EventKind::GrantPermission {
                    peer_id: peer.endpoint_id(),
                    permission: Permission::SendMessages,
                },
                vec![],
                0,
            );
            grant_hashes.insert(peer.endpoint_id(), e.hash);
            dag.insert(e).unwrap();
        }

        // Set profiles from different peers, each depending on the
        // grant that made them a member.
        for (peer, name) in [(&peer_a, "Alice"), (&peer_b, "Bob"), (&peer_c, "Carol")] {
            let dep = grant_hashes[&peer.endpoint_id()];
            let e = dag.create_event(
                peer,
                EventKind::SetProfile {
                    display_name: name.into(),
                },
                vec![dep],
                0,
            );
            dag.insert(e).unwrap();
        }

        // Materialize twice — both should produce identical snapshot hashes
        // because BTreeMap iteration is deterministic.
        let state1 = materialize(&dag);
        let heads1 = dag.heads_summary();
        let snap1 = Snapshot::new(state1, heads1);

        let state2 = materialize(&dag);
        let heads2 = dag.heads_summary();
        let snap2 = Snapshot::new(state2, heads2);

        assert_eq!(
            snap1.hash, snap2.hash,
            "snapshot hashes must be deterministic across materializations"
        );

        // Verify the state has meaningful content (not empty defaults).
        assert_eq!(snap1.state.channels.len(), 3);
        assert_eq!(snap1.state.roles.len(), 2);
        assert_eq!(snap1.state.profiles.len(), 3);
        assert!(snap1.state.peer_permissions.len() >= 3);
    }

    // ── Issue #51: ZERO-buffered events must resolve after genesis ──

    #[test]
    fn resolve_zero_drains_pre_genesis_events() {
        let mut buf = PendingBuffer::new();
        let id = Identity::generate();

        // Simulate a second author's first event (prev=ZERO) arriving
        // before genesis. It gets buffered under EventHash::ZERO.
        let event = Event::new(
            &id,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::SetProfile {
                display_name: "newcomer".into(),
            },
            0,
        );
        buf.buffer_for_prev(EventHash::ZERO, event.clone());
        assert_eq!(buf.pending_count(), 1);

        // Resolving with a genesis hash (not ZERO) should NOT return it.
        let genesis_hash = EventHash::from_bytes(b"genesis-hash");
        let resolved = buf.resolve(&genesis_hash);
        assert_eq!(resolved.len(), 0, "should not resolve under genesis hash");
        assert_eq!(buf.pending_count(), 1, "event should still be pending");

        // Resolving with ZERO should return it.
        let resolved = buf.resolve(&EventHash::ZERO);
        assert_eq!(resolved.len(), 1, "should resolve under ZERO");
        assert_eq!(resolved[0].hash, event.hash);
        assert_eq!(buf.pending_count(), 0);
    }

    // ── Issue #50: NotGenesis events can be buffered and recovered ──

    #[test]
    fn buffer_not_genesis_then_resolve_after_genesis() {
        let owner = Identity::generate();
        let member = Identity::generate();
        let mut dag = EventDag::new();
        let mut buf = PendingBuffer::new();

        // A non-CreateServer event arrives first → NotGenesis.
        let member_event = Event::new(
            &member,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::SetProfile {
                display_name: "member".into(),
            },
            0,
        );
        let err = dag.insert(member_event.clone()).unwrap_err();
        assert!(
            matches!(err, crate::dag::InsertError::NotGenesis),
            "should get NotGenesis error"
        );

        // Buffer under prev (ZERO), same as the fix will do.
        buf.buffer_for_prev(member_event.prev, member_event.clone());

        // Now genesis arrives and succeeds.
        let genesis = Event::new(
            &owner,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer { name: "srv".into() },
            0,
        );
        dag.insert(genesis.clone()).unwrap();

        // Resolve events buffered under the genesis hash — should be empty.
        let resolved = buf.resolve(&genesis.hash);
        assert_eq!(resolved.len(), 0);

        // Resolve ZERO — should return the member event.
        let resolved = buf.resolve(&EventHash::ZERO);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].hash, member_event.hash);

        // The resolved event should now insert successfully.
        dag.insert(resolved[0].clone()).unwrap();
        assert_eq!(dag.len(), 2);
    }

    // ── Issue #52: PrevMismatch is equivocation, not a chain gap ──

    #[test]
    fn prev_mismatch_indicates_equivocation_not_gap() {
        let owner = Identity::generate();
        let mut dag = test_dag(&owner);

        // Create a legitimate event to establish the chain.
        let e1 = dag.create_event(
            &owner,
            EventKind::SetProfile {
                display_name: "v1".into(),
            },
            vec![],
            0,
        );
        dag.insert(e1.clone()).unwrap();

        // Create a competing event with correct seq (3) but pointing
        // to genesis as prev instead of e1 — this is equivocation.
        let genesis_hash = dag.genesis().unwrap().hash;
        let competing = Event::new(
            &owner,
            3,
            genesis_hash, // wrong prev — should be e1.hash
            vec![],
            EventKind::SetProfile {
                display_name: "equivocating".into(),
            },
            0,
        );
        let err = dag.insert(competing).unwrap_err();
        assert!(
            matches!(err, crate::dag::InsertError::PrevMismatch { .. }),
            "should get PrevMismatch, not SeqGap: got {err:?}"
        );
        // PrevMismatch events should be DROPPED (not buffered) because
        // the predecessor they reference will never become the head.
    }

    // ── Corrective events test ─────────────────────────────────────

    #[test]
    fn corrective_events() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);

        // Send a message.
        let msg = dag.create_event(
            &admin,
            EventKind::Message {
                channel_id: "ch".into(),
                body: "oops".into(),
                reply_to: None,
            },
            vec![],
            0,
        );
        dag.insert(msg.clone()).unwrap();

        // Delete it (corrective event).
        let del = dag.create_event(
            &admin,
            EventKind::DeleteMessage {
                message_id: msg.hash,
            },
            vec![],
            0,
        );
        dag.insert(del).unwrap();

        let state = materialize(&dag);
        assert_eq!(state.messages.len(), 1);
        assert!(state.messages[0].deleted);
        assert_eq!(state.messages[0].body, "[message deleted]");
        // Original event is still in the DAG.
        assert!(dag.get(&msg.hash).is_some());
    }

    #[test]
    fn evict_to_completes_in_linear_time() {
        let mut buf = PendingBuffer::new();
        let id = Identity::generate();

        // Insert 10,000 pending events under unique prev hashes.
        for i in 0..10_000u64 {
            let mut hash_bytes = [0u8; 32];
            hash_bytes[..8].copy_from_slice(&i.to_le_bytes());
            let prev = EventHash(hash_bytes);
            let e = Event::new(
                &id,
                i + 2,
                prev,
                vec![],
                EventKind::SetProfile {
                    display_name: format!("n{i}"),
                },
                0,
            );
            buf.buffer_for_prev(prev, e);
        }
        assert_eq!(buf.pending_count(), 10_000);

        let start = std::time::Instant::now();
        let evicted = buf.evict_to(100);
        let elapsed = start.elapsed();

        assert!(evicted >= 9_900, "should evict most entries, got {evicted}");
        assert!(
            buf.pending_count() <= 100,
            "should have at most 100 pending, got {}",
            buf.pending_count()
        );
        // With cached count: should be near-instant. Without: quadratic and slow.
        assert!(
            elapsed.as_millis() < 500,
            "evict_to took too long: {elapsed:?}"
        );
    }

    #[test]
    fn cached_count_stays_consistent() {
        let mut buf = PendingBuffer::new();
        let id = Identity::generate();

        assert_eq!(buf.pending_count(), 0);

        // Buffer some events.
        for i in 0..5u64 {
            let mut hash_bytes = [0u8; 32];
            hash_bytes[..8].copy_from_slice(&i.to_le_bytes());
            let prev = EventHash(hash_bytes);
            let e = Event::new(
                &id,
                i + 2,
                prev,
                vec![],
                EventKind::SetProfile {
                    display_name: format!("n{i}"),
                },
                0,
            );
            buf.buffer_for_prev(prev, e);
        }
        assert_eq!(buf.pending_count(), 5);

        // Resolve one entry.
        let mut hash_bytes = [0u8; 32];
        hash_bytes[..8].copy_from_slice(&2u64.to_le_bytes());
        let resolved = buf.resolve(&EventHash(hash_bytes));
        assert_eq!(resolved.len(), 1);
        assert_eq!(buf.pending_count(), 4);

        // Resolve nonexistent — count unchanged.
        let _ = buf.resolve(&EventHash([0xFF; 32]));
        assert_eq!(buf.pending_count(), 4);

        // Evict to 2.
        buf.evict_to(2);
        assert_eq!(buf.pending_count(), 2);
    }

    // ── Issue #40: Age + capacity eviction for pending events ──────

    fn make_pending_event(id: &Identity, seed: u64) -> (EventHash, Event) {
        let mut hash_bytes = [0u8; 32];
        hash_bytes[..8].copy_from_slice(&seed.to_le_bytes());
        let prev = EventHash(hash_bytes);
        let event = Event::new(
            id,
            seed + 2,
            prev,
            vec![],
            EventKind::SetProfile {
                display_name: format!("n{seed}"),
            },
            0,
        );
        (prev, event)
    }

    /// An entry older than `max_age_ms` is evicted when a later insert
    /// carries a timestamp that advances past the age threshold.
    #[test]
    fn age_eviction_drops_stale_entries_on_insert() {
        let id = Identity::generate();
        let max_age_ms = 1_000u64;
        let mut buf = PendingBuffer::with_limits(10_000, max_age_ms);

        // Insert one entry at t=0.
        let (prev_a, event_a) = make_pending_event(&id, 1);
        buf.buffer_for_prev_at(prev_a, event_a, 0);
        assert_eq!(buf.pending_count(), 1);

        // Advance past max_age (t = max_age + 100). Inserting a new entry
        // at that time must first evict the stale entry.
        let (prev_b, event_b) = make_pending_event(&id, 2);
        buf.buffer_for_prev_at(prev_b, event_b, max_age_ms + 100);

        assert_eq!(
            buf.pending_count(),
            1,
            "stale entry should have been evicted, leaving only the fresh one"
        );
    }

    /// An entry within `max_age_ms` is retained across subsequent inserts.
    #[test]
    fn age_eviction_retains_fresh_entries() {
        let id = Identity::generate();
        let max_age_ms = 60_000u64;
        let mut buf = PendingBuffer::with_limits(10_000, max_age_ms);

        let (prev_a, event_a) = make_pending_event(&id, 1);
        buf.buffer_for_prev_at(prev_a, event_a, 0);

        // Only 500 ms later — still fresh.
        let (prev_b, event_b) = make_pending_event(&id, 2);
        buf.buffer_for_prev_at(prev_b, event_b, 500);

        assert_eq!(
            buf.pending_count(),
            2,
            "both entries should still be pending within max_age_ms"
        );
    }

    /// Entries inserted via the legacy timestamp-less entry point are
    /// immune to age eviction.
    #[test]
    fn age_eviction_ignores_entries_without_timestamp() {
        let id = Identity::generate();
        let mut buf = PendingBuffer::with_limits(10_000, 1_000);

        let (prev_a, event_a) = make_pending_event(&id, 1);
        buf.buffer_for_prev(prev_a, event_a); // no timestamp

        let evicted = buf.evict_expired(u64::MAX);
        assert_eq!(evicted, 0, "timestamp-less entries must not be evicted");
        assert_eq!(buf.pending_count(), 1);
    }

    /// After filling to `max_entries + 1` the oldest entry is evicted
    /// so the count stays at the configured capacity.
    ///
    /// The per-author sub-cap is widened so this test exercises the
    /// global capacity path independently of SEC-V-08.
    #[test]
    fn capacity_eviction_drops_oldest_when_exceeded() {
        let id = Identity::generate();
        let max_entries = 5usize;
        let mut buf =
            PendingBuffer::with_limits(max_entries, 60_000).with_per_author_cap(max_entries + 10);

        for i in 0..max_entries as u64 {
            let (prev, event) = make_pending_event(&id, i);
            buf.buffer_for_prev_at(prev, event, 0);
        }
        assert_eq!(buf.pending_count(), max_entries);

        // Insert one more — capacity is exceeded, so eviction kicks in.
        let (prev, event) = make_pending_event(&id, 999);
        buf.buffer_for_prev_at(prev, event, 100);
        assert_eq!(
            buf.pending_count(),
            max_entries,
            "count must stay at max_entries after overfill"
        );
    }

    /// `pending_count()` accurately reflects eviction activity.
    ///
    /// Uses an explicit per-author cap so the test focuses on the
    /// interaction between age- and capacity-based eviction (not
    /// SEC-V-08 sub-cap behaviour, which is covered separately).
    #[test]
    fn pending_count_reflects_both_eviction_policies() {
        let id = Identity::generate();
        let max_age_ms = 1_000u64;
        let mut buf = PendingBuffer::with_limits(4, max_age_ms).with_per_author_cap(100);

        // Four fresh entries at t=0 → fills capacity exactly.
        for i in 0..4u64 {
            let (prev, event) = make_pending_event(&id, i);
            buf.buffer_for_prev_at(prev, event, 0);
        }
        assert_eq!(buf.pending_count(), 4);

        // Add one at t = max_age + 50. All old entries should age out;
        // the new entry is the only survivor.
        let (prev, event) = make_pending_event(&id, 100);
        buf.buffer_for_prev_at(prev, event, max_age_ms + 50);
        assert_eq!(
            buf.pending_count(),
            1,
            "age eviction must sweep the four stale entries before the new insert"
        );
    }

    /// `evict_expired` on a buffer without a configured `max_age_ms` is a
    /// no-op (returns 0).
    #[test]
    fn evict_expired_without_age_limit_is_noop() {
        let id = Identity::generate();
        let mut buf = PendingBuffer::with_capacity(1_000); // no max_age_ms

        let (prev, event) = make_pending_event(&id, 1);
        buf.buffer_for_prev_at(prev, event, 0);

        let evicted = buf.evict_expired(u64::MAX);
        assert_eq!(evicted, 0);
        assert_eq!(buf.pending_count(), 1);
    }

    /// Defaults (`with_limits(DEFAULT_PENDING_MAX_ENTRIES, DEFAULT_PENDING_MAX_AGE_MS)`)
    /// compile cleanly and behave as expected for a tiny workload.
    #[test]
    fn default_limits_construct_and_work() {
        let id = Identity::generate();
        let mut buf =
            PendingBuffer::with_limits(DEFAULT_PENDING_MAX_ENTRIES, DEFAULT_PENDING_MAX_AGE_MS);
        let (prev, event) = make_pending_event(&id, 1);
        buf.buffer_for_prev_at(prev, event, 42);
        assert_eq!(buf.pending_count(), 1);
    }

    // ── SEC-V-08: per-author sub-cap on pending buffer ──────────────

    /// Helper: build a pending event whose author is `id` and whose
    /// `prev` hash is unique (so each event lands in its own bucket).
    fn make_pending_event_for(id: &Identity, seed: u64) -> (EventHash, Event) {
        let mut hash_bytes = [0u8; 32];
        hash_bytes[..8].copy_from_slice(&seed.to_le_bytes());
        let prev = EventHash(hash_bytes);
        let event = Event::new(
            id,
            seed + 2,
            prev,
            vec![],
            EventKind::SetProfile {
                display_name: format!("a-{seed}"),
            },
            0,
        );
        (prev, event)
    }

    /// One author cannot fill the whole buffer: events past the
    /// per-author sub-cap from a single signer are dropped while other
    /// authors keep their slots.
    #[test]
    fn per_author_cap_isolates_one_signer() {
        let attacker = Identity::generate();
        let victim = Identity::generate();

        let per_author = 3usize;
        let mut buf = PendingBuffer::with_capacity(1_000).with_per_author_cap(per_author);

        // Attacker tries to insert 10 events — only `per_author` survive.
        for i in 0..10u64 {
            let (prev, event) = make_pending_event_for(&attacker, i);
            buf.buffer_for_prev(prev, event);
        }
        assert_eq!(
            buf.pending_count(),
            per_author,
            "attacker must be capped to the per-author limit"
        );

        // Victim can still insert events — their bucket is untouched.
        for i in 100..103u64 {
            let (prev, event) = make_pending_event_for(&victim, i);
            buf.buffer_for_prev(prev, event);
        }
        assert_eq!(
            buf.pending_count(),
            per_author + 3,
            "victim's slots must not be consumed by the attacker"
        );
    }

    /// `with_capacity` and `with_limits` derive the per-author cap from
    /// the total `max_entries` using `DEFAULT_PENDING_PER_AUTHOR_DIVISOR`.
    #[test]
    fn per_author_cap_defaults_to_total_over_divisor() {
        let id = Identity::generate();
        let total = 5_000usize;
        let expected_cap = total / DEFAULT_PENDING_PER_AUTHOR_DIVISOR; // 100

        let mut buf = PendingBuffer::with_capacity(total);

        // Fill well past the expected cap from one author.
        for i in 0..(expected_cap as u64 + 50) {
            let (prev, event) = make_pending_event_for(&id, i);
            buf.buffer_for_prev(prev, event);
        }

        // Per-author count must clamp at the derived sub-cap, not the
        // 5000-event global cap.
        assert_eq!(
            buf.pending_count(),
            expected_cap,
            "default per-author cap should be max_entries / DEFAULT_PENDING_PER_AUTHOR_DIVISOR"
        );
    }

    /// Tiny totals still yield a usable per-author cap (>= 1) — the
    /// `(max / divisor).max(1)` floor keeps tests configurable.
    #[test]
    fn per_author_cap_floor_is_one() {
        let id = Identity::generate();
        // 5 / 50 == 0; floor brings it back to 1.
        let mut buf = PendingBuffer::with_capacity(5);
        let (prev_a, event_a) = make_pending_event_for(&id, 0);
        buf.buffer_for_prev(prev_a, event_a);

        let (prev_b, event_b) = make_pending_event_for(&id, 1);
        buf.buffer_for_prev(prev_b, event_b);

        assert_eq!(
            buf.pending_count(),
            1,
            "per-author cap should never collapse to zero"
        );
    }

    /// The first time an author hits the sub-cap, we record it in
    /// `warned_full_authors` so subsequent drops from the same author
    /// don't re-emit a warning. Other authors still get their own first
    /// warning.
    #[test]
    fn per_author_cap_warns_once_per_author() {
        let attacker = Identity::generate();
        let other = Identity::generate();

        let mut buf = PendingBuffer::with_capacity(1_000).with_per_author_cap(1);

        // First insert succeeds.
        let (prev_a, event_a) = make_pending_event_for(&attacker, 0);
        buf.buffer_for_prev(prev_a, event_a);
        assert!(buf.warned_full_authors.is_empty());

        // Second insert from same author hits the cap → warn once.
        let (prev_b, event_b) = make_pending_event_for(&attacker, 1);
        buf.buffer_for_prev(prev_b, event_b);
        assert!(buf.warned_full_authors.contains(&attacker.endpoint_id()));
        assert_eq!(buf.warned_full_authors.len(), 1);

        // Third insert from same author also drops, but the bookkeeping
        // set still has exactly one entry for that author.
        let (prev_c, event_c) = make_pending_event_for(&attacker, 2);
        buf.buffer_for_prev(prev_c, event_c);
        assert_eq!(buf.warned_full_authors.len(), 1);

        // Different author → independent warning lifecycle.
        let (prev_d, event_d) = make_pending_event_for(&other, 0);
        buf.buffer_for_prev(prev_d, event_d);
        let (prev_e, event_e) = make_pending_event_for(&other, 1);
        buf.buffer_for_prev(prev_e, event_e);
        assert!(buf.warned_full_authors.contains(&other.endpoint_id()));
        assert_eq!(buf.warned_full_authors.len(), 2);
    }

    /// Resolving an author's events frees their slot so subsequent
    /// inserts from that author are accepted again.
    #[test]
    fn per_author_cap_frees_on_resolve() {
        let id = Identity::generate();
        let mut buf = PendingBuffer::with_capacity(1_000).with_per_author_cap(2);

        let (prev_a, event_a) = make_pending_event_for(&id, 0);
        buf.buffer_for_prev(prev_a, event_a);
        let (prev_b, event_b) = make_pending_event_for(&id, 1);
        buf.buffer_for_prev(prev_b, event_b);
        assert_eq!(buf.pending_count(), 2);

        // Cap reached — extra is dropped.
        let (prev_c, event_c) = make_pending_event_for(&id, 2);
        buf.buffer_for_prev(prev_c, event_c);
        assert_eq!(buf.pending_count(), 2);

        // Resolve one of the existing entries — frees a slot.
        let _ = buf.resolve(&prev_a);
        assert_eq!(buf.pending_count(), 1);

        // Now a new event from the same author fits again.
        let (prev_d, event_d) = make_pending_event_for(&id, 3);
        buf.buffer_for_prev(prev_d, event_d);
        assert_eq!(buf.pending_count(), 2);
    }
}
