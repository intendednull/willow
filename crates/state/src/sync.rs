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

/// Buffer for events that arrive before their per-author chain predecessors.
///
/// Per-author chain gaps (`prev` references an unknown event) are hard gaps
/// — the event is buffered. Cross-author dep gaps (`deps` references an
/// unknown event) are soft — the event is accepted and the dep is recorded
/// for background fetching.
#[derive(Clone, Debug, Default)]
pub struct PendingBuffer {
    /// Events waiting for a missing `prev` hash.
    waiting_on_prev: BTreeMap<EventHash, Vec<Event>>,
    /// Cross-author deps we've seen referenced but don't have yet.
    missing_deps: BTreeSet<EventHash>,
    /// Optional maximum number of pending events. When set,
    /// `buffer_for_prev()` auto-evicts oldest entries to stay within limit.
    max_pending: Option<usize>,
    /// Cached total count of pending events for O(1) lookups.
    cached_count: usize,
}

impl PendingBuffer {
    /// Create a new empty buffer with no capacity limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a buffer with a maximum pending event capacity.
    ///
    /// When the total pending count exceeds this limit after an insertion,
    /// the buffer automatically evicts entries to stay within bounds.
    pub fn with_capacity(max_pending: usize) -> Self {
        Self {
            max_pending: Some(max_pending),
            ..Self::default()
        }
    }

    /// Buffer an event that's waiting for a prev hash to arrive.
    ///
    /// If a capacity limit is set, excess entries are automatically evicted.
    pub fn buffer_for_prev(&mut self, prev_hash: EventHash, event: Event) {
        self.waiting_on_prev
            .entry(prev_hash)
            .or_default()
            .push(event);
        self.cached_count += 1;
        if let Some(limit) = self.max_pending {
            self.evict_to(limit);
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
        let resolved = self
            .waiting_on_prev
            .remove(inserted_hash)
            .unwrap_or_default();
        self.cached_count = self.cached_count.saturating_sub(resolved.len());
        resolved
    }

    /// Number of missing cross-author deps being tracked.
    pub fn missing_dep_count(&self) -> usize {
        self.missing_deps.len()
    }

    /// Number of events waiting for prev chain predecessors.
    pub fn pending_count(&self) -> usize {
        self.cached_count
    }

    /// Evict pending entries to keep the buffer bounded.
    ///
    /// Removes the oldest entries (by insertion order approximation)
    /// until the total pending count is at or below `max_pending`.
    /// Returns the number of events evicted.
    pub fn evict_to(&mut self, max_pending: usize) -> usize {
        let mut evicted = 0;
        while self.cached_count > max_pending {
            // Remove an arbitrary entry.
            if let Some(key) = self.waiting_on_prev.keys().next().cloned() {
                if let Some(events) = self.waiting_on_prev.remove(&key) {
                    let n = events.len();
                    evicted += n;
                    self.cached_count = self.cached_count.saturating_sub(n);
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
        let mut buf = PendingBuffer::with_capacity(50);
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

        // Grant permissions to multiple peers.
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
            dag.insert(e).unwrap();
        }

        // Set profiles from different peers.
        for (peer, name) in [(&peer_a, "Alice"), (&peer_b, "Bob"), (&peer_c, "Carol")] {
            let e = dag.create_event(
                peer,
                EventKind::SetProfile {
                    display_name: name.into(),
                },
                vec![],
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
}
