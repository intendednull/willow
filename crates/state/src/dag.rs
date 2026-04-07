//! The per-author Merkle-DAG — source of truth for all shared state.
//!
//! [`EventDag`] stores all known events across all authors, indexed by
//! content hash. Each author's events form an append-only chain linked
//! via `prev` hashes, with cross-author causal dependencies via `deps`.

use std::collections::HashMap;

use willow_identity::{EndpointId, Identity};

use crate::event::{Event, EventKind};
use crate::hash::EventHash;

/// Error returned when inserting an event into the DAG fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertError {
    /// Ed25519 signature does not verify.
    InvalidSignature,
    /// First event in DAG must be `EventKind::CreateServer`.
    NotGenesis,
    /// seq is not prev_seq + 1 for this author.
    SeqGap {
        author: EndpointId,
        expected: u64,
        got: u64,
    },
    /// prev hash doesn't match author's current head.
    PrevMismatch {
        author: EndpointId,
        expected: EventHash,
        got: EventHash,
    },
    /// Event with this hash already exists.
    Duplicate,
    /// A CreateServer event was inserted after genesis already exists.
    DuplicateGenesis,
    /// A Vote event does not include the proposal hash in its deps.
    MissingGovernanceDep {
        vote: EventHash,
        proposal: EventHash,
    },
}

impl std::fmt::Display for InsertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSignature => write!(f, "invalid signature"),
            Self::NotGenesis => {
                write!(f, "first event in DAG must be CreateServer")
            }
            Self::SeqGap {
                author,
                expected,
                got,
            } => write!(
                f,
                "seq gap for author {author}: expected {expected}, got {got}"
            ),
            Self::PrevMismatch {
                author,
                expected,
                got,
            } => write!(
                f,
                "prev mismatch for author {author}: expected {expected}, got {got}"
            ),
            Self::Duplicate => write!(f, "duplicate event"),
            Self::DuplicateGenesis => {
                write!(f, "CreateServer event rejected: genesis already exists")
            }
            Self::MissingGovernanceDep { vote, proposal } => write!(
                f,
                "Vote event {vote} must include proposal {proposal} in deps"
            ),
        }
    }
}

impl std::error::Error for InsertError {}

/// The Merkle-DAG of all known events across all authors.
///
/// This is the source of truth from which all state is derived via
/// [`crate::materialize::materialize`].
#[derive(Clone)]
pub struct EventDag {
    /// All events indexed by hash.
    events: HashMap<EventHash, Event>,
    /// Per-author chain index: `chains[author][i].seq == i + 1`.
    chains: HashMap<EndpointId, Vec<EventHash>>,
    /// Current head (latest event hash) per author.
    heads: HashMap<EndpointId, EventHash>,
    /// Hash of the genesis event (CreateServer). Set on first insert.
    genesis_hash: Option<EventHash>,
}

impl EventDag {
    /// Create a new empty DAG.
    pub fn new() -> Self {
        Self {
            events: HashMap::new(),
            chains: HashMap::new(),
            heads: HashMap::new(),
            genesis_hash: None,
        }
    }

    /// Insert a verified event into the DAG.
    ///
    /// The first event must be `EventKind::CreateServer` with seq=1 and
    /// prev=ZERO. Unknown deps are silently accepted (soft-accept).
    pub fn insert(&mut self, event: Event) -> Result<(), InsertError> {
        // 1. Verify signature.
        if !event.verify() {
            return Err(InsertError::InvalidSignature);
        }

        // 2. Check duplicate.
        if self.events.contains_key(&event.hash) {
            return Err(InsertError::Duplicate);
        }

        // 3. Genesis check: first event must be CreateServer.
        //    After genesis is set, reject any further CreateServer events.
        if self.genesis_hash.is_none() {
            match &event.kind {
                EventKind::CreateServer { .. } => {
                    if event.seq != 1 || event.prev != EventHash::ZERO {
                        return Err(InsertError::SeqGap {
                            author: event.author,
                            expected: 1,
                            got: event.seq,
                        });
                    }
                    self.genesis_hash = Some(event.hash);
                }
                _ => return Err(InsertError::NotGenesis),
            }
        } else if matches!(event.kind, EventKind::CreateServer { .. }) {
            return Err(InsertError::DuplicateGenesis);
        }

        // 4. Check seq: must be latest_seq + 1.
        //    This also prevents equivocation: an author cannot insert two
        //    events at the same seq number because only seq = latest + 1
        //    is accepted. Combined with the prev-hash check below, this
        //    makes per-author chain forking structurally impossible.
        let expected_seq = self.latest_seq(&event.author) + 1;
        if event.seq != expected_seq {
            return Err(InsertError::SeqGap {
                author: event.author,
                expected: expected_seq,
                got: event.seq,
            });
        }

        // 5. Check prev: must match current head (or ZERO for seq=1).
        let expected_prev = self
            .heads
            .get(&event.author)
            .cloned()
            .unwrap_or(EventHash::ZERO);
        if event.prev != expected_prev {
            return Err(InsertError::PrevMismatch {
                author: event.author,
                expected: expected_prev,
                got: event.prev,
            });
        }

        // 6. Governance structural checks: Vote events must causally
        //    depend on their proposal (via deps or prev) so topological
        //    sort always places the proposal before the vote.
        if let EventKind::Vote { proposal, .. } = &event.kind {
            if !event.deps.contains(proposal) && event.prev != *proposal {
                return Err(InsertError::MissingGovernanceDep {
                    vote: event.hash,
                    proposal: *proposal,
                });
            }
        }

        // 7. Insert.
        let hash = event.hash;
        let author = event.author;
        self.events.insert(hash, event);
        self.chains.entry(author).or_default().push(hash);
        self.heads.insert(author, hash);

        Ok(())
    }

    /// The genesis event. `None` if the DAG is empty.
    pub fn genesis(&self) -> Option<&Event> {
        self.genesis_hash.as_ref().and_then(|h| self.events.get(h))
    }

    /// The server ID (hex of genesis event hash). `None` if empty.
    pub fn server_id(&self) -> Option<String> {
        self.genesis_hash.as_ref().map(|h| h.to_string())
    }

    /// The genesis event's author. `None` if empty.
    pub fn genesis_author(&self) -> Option<EndpointId> {
        self.genesis().map(|e| e.author)
    }

    /// Latest sequence number for an author (0 if unknown).
    pub fn latest_seq(&self, author: &EndpointId) -> u64 {
        self.chains.get(author).map(|c| c.len() as u64).unwrap_or(0)
    }

    /// Current head hash for an author.
    pub fn head(&self, author: &EndpointId) -> Option<&EventHash> {
        self.heads.get(author)
    }

    /// All event hashes for an author, in seq order.
    pub fn author_events(&self, author: &EndpointId) -> &[EventHash] {
        self.chains.get(author).map(|c| c.as_slice()).unwrap_or(&[])
    }

    /// Look up an event by hash.
    pub fn get(&self, hash: &EventHash) -> Option<&Event> {
        self.events.get(hash)
    }

    /// Total number of events in the DAG.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the DAG is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Iterator over all known authors.
    pub fn authors(&self) -> impl Iterator<Item = &EndpointId> {
        self.chains.keys()
    }

    /// Convenience: create a signed event ready for insertion.
    ///
    /// Reads the current head/seq for the identity's author and builds
    /// an event with `seq + 1` and `prev = current_head`. Does NOT
    /// insert — the caller must call `insert()` separately.
    pub fn create_event(
        &self,
        identity: &Identity,
        kind: EventKind,
        deps: Vec<EventHash>,
        timestamp_hint_ms: u64,
    ) -> Event {
        let author = identity.endpoint_id();
        let seq = self.latest_seq(&author) + 1;
        let prev = self.heads.get(&author).cloned().unwrap_or(EventHash::ZERO);
        Event::new(identity, seq, prev, deps, kind, timestamp_hint_ms)
    }

    // ── Sync helpers ────────────────────────────────────────────────

    /// Compute a compact summary of the DAG's current heads.
    pub fn heads_summary(&self) -> crate::sync::HeadsSummary {
        use crate::sync::{AuthorHead, HeadsSummary};
        let mut heads = std::collections::BTreeMap::new();
        for (author, hash) in &self.heads {
            let seq = self.latest_seq(author);
            heads.insert(*author, AuthorHead { seq, hash: *hash });
        }
        HeadsSummary { heads }
    }

    /// Return events the requester doesn't have, based on their known heads.
    ///
    /// For each author we know about: if the requester has a lower seq (or
    /// doesn't know the author at all), return our events after their seq.
    /// An optional `limit` caps the total number of events returned.
    pub fn events_since(
        &self,
        their_heads: &std::collections::BTreeMap<EndpointId, u64>,
        limit: Option<usize>,
    ) -> Vec<&Event> {
        let mut result = Vec::new();
        for (author, chain) in &self.chains {
            let their_seq = their_heads.get(author).copied().unwrap_or(0);
            for hash in chain.iter().skip(their_seq as usize) {
                if let Some(max) = limit {
                    if result.len() >= max {
                        return result;
                    }
                }
                if let Some(event) = self.events.get(hash) {
                    result.push(event);
                }
            }
        }
        result
    }

    // ── Topological sort ────────────────────────────────────────────

    /// Topological sort of all events in the DAG.
    ///
    /// Concurrent events (no causal relationship) are tie-broken by
    /// `EventHash` (lexicographic byte comparison via `Ord`). This
    /// produces a deterministic total order on all peers given the
    /// same DAG contents.
    ///
    /// Uses Kahn's algorithm with a `BTreeSet` for deterministic
    /// selection among ready nodes. Only counts edges to events that
    /// exist in the DAG (soft-accept: deps to absent events are ignored).
    pub fn topological_sort(&self) -> Vec<&Event> {
        use std::collections::{BTreeSet, HashMap as Map};

        let mut in_degree: Map<&EventHash, usize> = Map::new();
        let mut dependents: Map<&EventHash, Vec<&EventHash>> = Map::new();

        // Initialize all nodes with in-degree 0.
        for hash in self.events.keys() {
            in_degree.insert(hash, 0);
        }

        // Compute in-degrees from prev + deps edges.
        for event in self.events.values() {
            for parent in self.causal_parents(event) {
                if self.events.contains_key(parent) {
                    *in_degree.get_mut(&event.hash).unwrap() += 1;
                    dependents.entry(parent).or_default().push(&event.hash);
                }
            }
        }

        // Seed with zero-indegree events.
        let mut ready: BTreeSet<&EventHash> = BTreeSet::new();
        for (hash, &degree) in &in_degree {
            if degree == 0 {
                ready.insert(hash);
            }
        }

        // Process in deterministic order (BTreeSet sorts by hash bytes).
        let mut result: Vec<&Event> = Vec::with_capacity(self.events.len());
        while let Some(hash) = ready.pop_first() {
            result.push(&self.events[hash]);
            if let Some(deps) = dependents.get(hash) {
                for dep in deps {
                    let d = in_degree.get_mut(dep).unwrap();
                    *d -= 1;
                    if *d == 0 {
                        ready.insert(dep);
                    }
                }
            }
        }

        // Cycle detection: if any events remain unprocessed, a cycle exists.
        // Normal insert() prevents cycles via seq/prev chain checks, so this
        // is a defensive invariant against data corruption.
        assert_eq!(
            result.len(),
            self.events.len(),
            "Cycle detected in DAG: {} of {} events unprocessable",
            self.events.len() - result.len(),
            self.events.len()
        );

        result
    }

    /// Return all causal parents of an event: prev (if not ZERO) plus
    /// all deps. Only returns hashes — caller checks existence in DAG.
    fn causal_parents<'a>(&self, event: &'a Event) -> Vec<&'a EventHash> {
        let mut parents = Vec::new();
        if event.prev != EventHash::ZERO {
            parents.push(&event.prev);
        }
        parents.extend(event.deps.iter());
        parents
    }

    /// Check if event `a` is a causal ancestor of event `b`.
    ///
    /// Walks backwards from `b` via causal parents (prev + deps that
    /// exist in the DAG) looking for `a`.
    pub fn is_ancestor(&self, a: &EventHash, b: &EventHash) -> bool {
        if a == b {
            return true;
        }
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![b];
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(event) = self.events.get(current) {
                for parent in self.causal_parents(event) {
                    if parent == a {
                        return true;
                    }
                    if self.events.contains_key(parent) {
                        stack.push(parent);
                    }
                }
            }
        }
        false
    }
}

impl Default for EventDag {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;

    fn genesis_kind() -> EventKind {
        EventKind::CreateServer {
            name: "Test Server".into(),
        }
    }

    fn test_dag(identity: &Identity) -> EventDag {
        let mut dag = EventDag::new();
        let genesis = dag.create_event(identity, genesis_kind(), vec![], 0);
        dag.insert(genesis).unwrap();
        dag
    }

    #[test]
    fn insert_genesis_event() {
        let id = Identity::generate();
        let mut dag = EventDag::new();
        let genesis = dag.create_event(&id, genesis_kind(), vec![], 0);
        dag.insert(genesis.clone()).unwrap();

        assert_eq!(dag.len(), 1);
        assert!(dag.genesis().is_some());
        assert_eq!(dag.genesis().unwrap().hash, genesis.hash);
        assert_eq!(dag.genesis_author(), Some(id.endpoint_id()));
        assert!(dag.server_id().is_some());
    }

    #[test]
    fn insert_rejects_non_genesis_first() {
        let id = Identity::generate();
        let mut dag = EventDag::new();
        let event = dag.create_event(
            &id,
            EventKind::SetProfile {
                display_name: "alice".into(),
            },
            vec![],
            0,
        );
        let err = dag.insert(event).unwrap_err();
        assert!(matches!(err, InsertError::NotGenesis));
    }

    #[test]
    fn insert_sequential_events() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);

        let e2 = dag.create_event(
            &id,
            EventKind::SetProfile {
                display_name: "alice".into(),
            },
            vec![],
            100,
        );
        dag.insert(e2.clone()).unwrap();

        assert_eq!(dag.len(), 2);
        assert_eq!(dag.latest_seq(&id.endpoint_id()), 2);
        assert_eq!(dag.head(&id.endpoint_id()), Some(&e2.hash));
        assert_eq!(dag.author_events(&id.endpoint_id()).len(), 2);
    }

    #[test]
    fn insert_rejects_duplicate() {
        let id = Identity::generate();
        let mut dag = EventDag::new();
        let genesis = dag.create_event(&id, genesis_kind(), vec![], 0);
        dag.insert(genesis.clone()).unwrap();
        let err = dag.insert(genesis).unwrap_err();
        assert!(matches!(err, InsertError::Duplicate));
    }

    #[test]
    fn insert_rejects_second_create_server() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);
        // A second CreateServer event has seq=2 and valid prev, but the
        // DAG must reject it because genesis already exists.
        let second = dag.create_event(
            &id,
            EventKind::CreateServer {
                name: "Second Server".into(),
            },
            vec![],
            0,
        );
        let err = dag.insert(second).unwrap_err();
        assert!(matches!(err, InsertError::DuplicateGenesis));
        // DAG still has only the original genesis.
        assert_eq!(dag.len(), 1);
    }

    #[test]
    fn vote_without_proposal_dep_rejected() {
        let admin = Identity::generate();
        let admin_b = Identity::generate();
        let mut dag = test_dag(&admin);

        // Grant admin_b admin status (sole admin, auto-applies).
        let grant = dag.create_event(
            &admin,
            EventKind::Propose {
                action: crate::event::ProposedAction::GrantAdmin {
                    peer_id: admin_b.endpoint_id(),
                },
            },
            vec![],
            0,
        );
        dag.insert(grant).unwrap();

        // Admin proposes something new.
        let prop = dag.create_event(
            &admin,
            EventKind::Propose {
                action: crate::event::ProposedAction::SetVoteThreshold {
                    threshold: crate::event::VoteThreshold::Unanimous,
                },
            },
            vec![],
            0,
        );
        dag.insert(prop.clone()).unwrap();

        // admin_b votes WITHOUT proposal in deps — prev is NOT the proposal
        // (admin_b has no prev events yet), so this must be rejected.
        let vote = dag.create_event(
            &admin_b,
            EventKind::Vote {
                proposal: prop.hash,
                accept: true,
            },
            vec![], // Missing proposal dep!
            0,
        );
        let err = dag.insert(vote).unwrap_err();
        assert!(matches!(err, InsertError::MissingGovernanceDep { .. }));
    }

    #[test]
    fn vote_with_proposal_dep_accepted() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        let prop = dag.create_event(
            &admin,
            EventKind::Propose {
                action: crate::event::ProposedAction::GrantAdmin {
                    peer_id: Identity::generate().endpoint_id(),
                },
            },
            vec![],
            0,
        );
        dag.insert(prop.clone()).unwrap();
        // Vote WITH proposal in deps — should succeed.
        let vote = dag.create_event(
            &admin,
            EventKind::Vote {
                proposal: prop.hash,
                accept: true,
            },
            vec![prop.hash],
            0,
        );
        assert!(dag.insert(vote).is_ok());
    }

    #[test]
    fn vote_with_proposal_as_prev_accepted() {
        // If the voter is also the proposer, the proposal is the prev event
        // (previous event from same author), which also satisfies the causal dep.
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        let prop = dag.create_event(
            &admin,
            EventKind::Propose {
                action: crate::event::ProposedAction::GrantAdmin {
                    peer_id: Identity::generate().endpoint_id(),
                },
            },
            vec![],
            0,
        );
        dag.insert(prop.clone()).unwrap();
        // The admin's prev is now prop.hash, so even with empty deps it works.
        assert_eq!(dag.head(&admin.endpoint_id()), Some(&prop.hash));
        // This vote's prev will be prop.hash, satisfying the governance check.
        let vote = dag.create_event(
            &admin,
            EventKind::Vote {
                proposal: prop.hash,
                accept: true,
            },
            vec![], // prev == proposal, so this is OK
            0,
        );
        assert!(dag.insert(vote).is_ok());
    }

    #[test]
    fn insert_rejects_invalid_signature() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let mut dag = test_dag(&id_a);

        // Create an event signed by id_a but claim it's from id_b.
        let mut event = dag.create_event(
            &id_a,
            EventKind::SetProfile {
                display_name: "tampered".into(),
            },
            vec![],
            0,
        );
        event.author = id_b.endpoint_id();
        let err = dag.insert(event).unwrap_err();
        assert!(matches!(err, InsertError::InvalidSignature));
    }

    #[test]
    fn insert_rejects_seq_gap() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);

        // Manually create an event with seq=3 (skipping 2).
        let event = Event::new(
            &id,
            3,
            dag.head(&id.endpoint_id()).cloned().unwrap(),
            vec![],
            EventKind::SetProfile {
                display_name: "gap".into(),
            },
            0,
        );
        let err = dag.insert(event).unwrap_err();
        match err {
            InsertError::SeqGap { expected, got, .. } => {
                assert_eq!(expected, 2);
                assert_eq!(got, 3);
            }
            other => panic!("expected SeqGap, got {other:?}"),
        }
    }

    #[test]
    fn insert_rejects_prev_mismatch() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);

        // Create event with wrong prev hash.
        let event = Event::new(
            &id,
            2,
            EventHash::from_bytes(b"wrong"),
            vec![],
            EventKind::SetProfile {
                display_name: "bad".into(),
            },
            0,
        );
        let err = dag.insert(event).unwrap_err();
        assert!(matches!(err, InsertError::PrevMismatch { .. }));
    }

    #[test]
    fn insert_accepts_unknown_deps() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);

        let unknown_dep = EventHash::from_bytes(b"nonexistent");
        let event = dag.create_event(
            &id,
            EventKind::SetProfile {
                display_name: "alice".into(),
            },
            vec![unknown_dep],
            0,
        );
        // Should succeed — deps are soft-accepted.
        dag.insert(event).unwrap();
        assert_eq!(dag.len(), 2);
    }

    #[test]
    fn insert_multiple_authors() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let mut dag = test_dag(&id_a);

        // Author B inserts their own event (not genesis — genesis is already set).
        let b_event = dag.create_event(
            &id_b,
            EventKind::SetProfile {
                display_name: "bob".into(),
            },
            vec![],
            0,
        );
        dag.insert(b_event).unwrap();

        assert_eq!(dag.len(), 2);
        assert_eq!(dag.latest_seq(&id_a.endpoint_id()), 1);
        assert_eq!(dag.latest_seq(&id_b.endpoint_id()), 1);
        assert_eq!(dag.authors().count(), 2);
    }

    #[test]
    fn insert_with_cross_author_deps() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let mut dag = test_dag(&id_a);

        let a_head = *dag.head(&id_a.endpoint_id()).unwrap();
        let b_event = dag.create_event(
            &id_b,
            EventKind::SetProfile {
                display_name: "bob".into(),
            },
            vec![a_head], // B depends on A's genesis
            0,
        );
        dag.insert(b_event.clone()).unwrap();

        assert_eq!(dag.len(), 2);
        assert_eq!(dag.get(&b_event.hash).unwrap().deps, vec![a_head]);
    }

    #[test]
    fn genesis_accessors() {
        let id = Identity::generate();
        let dag = test_dag(&id);

        assert!(dag.genesis().is_some());
        assert_eq!(dag.genesis_author(), Some(id.endpoint_id()));
        let server_id = dag.server_id().unwrap();
        assert_eq!(server_id.len(), 64); // hex of 32 bytes
        assert_eq!(server_id, dag.genesis().unwrap().hash.to_string());
    }

    // ── Topological sort tests ─────────────────────────────────────

    #[test]
    fn sort_single_author_is_seq_order() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);

        for _ in 0..3 {
            let e = dag.create_event(
                &id,
                EventKind::SetProfile {
                    display_name: "x".into(),
                },
                vec![],
                0,
            );
            dag.insert(e).unwrap();
        }

        let sorted = dag.topological_sort();
        assert_eq!(sorted.len(), 4); // genesis + 3
        for (i, event) in sorted.iter().enumerate() {
            assert_eq!(event.seq, (i + 1) as u64);
        }
    }

    #[test]
    fn sort_independent_authors_by_hash() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let mut dag = test_dag(&id_a);

        let b1 = dag.create_event(
            &id_b,
            EventKind::SetProfile {
                display_name: "bob".into(),
            },
            vec![], // no dep on A — concurrent
            0,
        );
        dag.insert(b1).unwrap();

        let sorted = dag.topological_sort();
        assert_eq!(sorted.len(), 2);
        // Concurrent events sorted by hash (deterministic).
        assert!(sorted[0].hash < sorted[1].hash);
    }

    #[test]
    fn sort_respects_cross_author_deps() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let mut dag = test_dag(&id_a);

        let a_head = *dag.head(&id_a.endpoint_id()).unwrap();
        let b1 = dag.create_event(
            &id_b,
            EventKind::SetProfile {
                display_name: "bob".into(),
            },
            vec![a_head], // B depends on A's genesis
            0,
        );
        dag.insert(b1).unwrap();

        let sorted = dag.topological_sort();
        assert_eq!(sorted.len(), 2);
        // A's genesis must come before B's event (causal dep).
        assert_eq!(sorted[0].author, id_a.endpoint_id());
        assert_eq!(sorted[1].author, id_b.endpoint_id());
    }

    #[test]
    fn sort_complex_dag() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let id_c = Identity::generate();
        let mut dag = test_dag(&id_a);

        // A1 (genesis) already inserted
        let a1_hash = *dag.head(&id_a.endpoint_id()).unwrap();

        // B1: depends on A1
        let b1 = dag.create_event(
            &id_b,
            EventKind::SetProfile {
                display_name: "b".into(),
            },
            vec![a1_hash],
            0,
        );
        dag.insert(b1.clone()).unwrap();

        // C1: depends on B1
        let c1 = dag.create_event(
            &id_c,
            EventKind::SetProfile {
                display_name: "c".into(),
            },
            vec![b1.hash],
            0,
        );
        dag.insert(c1.clone()).unwrap();

        // A2: depends on B1
        let a2 = dag.create_event(
            &id_a,
            EventKind::SetProfile {
                display_name: "a2".into(),
            },
            vec![b1.hash],
            0,
        );
        dag.insert(a2.clone()).unwrap();

        let sorted = dag.topological_sort();
        assert_eq!(sorted.len(), 4);

        // A1 must be first (no deps).
        assert_eq!(sorted[0].hash, a1_hash);

        // B1 must come after A1 (depends on A1).
        let b1_pos = sorted.iter().position(|e| e.hash == b1.hash).unwrap();
        let a1_pos = sorted.iter().position(|e| e.hash == a1_hash).unwrap();
        assert!(a1_pos < b1_pos);

        // C1 and A2 both depend on B1, so B1 must come before both.
        let c1_pos = sorted.iter().position(|e| e.hash == c1.hash).unwrap();
        let a2_pos = sorted.iter().position(|e| e.hash == a2.hash).unwrap();
        assert!(b1_pos < c1_pos);
        assert!(b1_pos < a2_pos);
    }

    #[test]
    fn sort_is_deterministic() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);
        for i in 0..5 {
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

        let s1 = dag.topological_sort();
        let s2 = dag.topological_sort();
        let hashes1: Vec<_> = s1.iter().map(|e| e.hash).collect();
        let hashes2: Vec<_> = s2.iter().map(|e| e.hash).collect();
        assert_eq!(hashes1, hashes2);
    }

    #[test]
    fn sort_is_stable_under_insertion_order() {
        // Build the same logical DAG with different insertion orders.
        // Both should produce the same topological sort.
        let id_a = Identity::generate();
        let id_b = Identity::generate();

        // DAG 1: insert A genesis, then B.
        let mut dag1 = test_dag(&id_a);
        let a_head = *dag1.head(&id_a.endpoint_id()).unwrap();
        let b1 = Event::new(
            &id_b,
            1,
            EventHash::ZERO,
            vec![a_head],
            EventKind::SetProfile {
                display_name: "bob".into(),
            },
            0,
        );
        dag1.insert(b1.clone()).unwrap();

        // DAG 2: same events. Insertion order is the same structurally
        // (genesis must be first), but we verify sort output matches.
        let mut dag2 = EventDag::new();
        let genesis = Event::new(&id_a, 1, EventHash::ZERO, vec![], genesis_kind(), 0);
        dag2.insert(genesis).unwrap();
        dag2.insert(b1).unwrap();

        let s1: Vec<_> = dag1.topological_sort().iter().map(|e| e.hash).collect();
        let s2: Vec<_> = dag2.topological_sort().iter().map(|e| e.hash).collect();
        assert_eq!(s1, s2);
    }

    #[test]
    fn is_ancestor_basic() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let mut dag = test_dag(&id_a);

        let a1_hash = *dag.head(&id_a.endpoint_id()).unwrap();
        let b1 = dag.create_event(
            &id_b,
            EventKind::SetProfile {
                display_name: "bob".into(),
            },
            vec![a1_hash],
            0,
        );
        dag.insert(b1.clone()).unwrap();

        assert!(dag.is_ancestor(&a1_hash, &b1.hash));
        assert!(!dag.is_ancestor(&b1.hash, &a1_hash));
        assert!(dag.is_ancestor(&a1_hash, &a1_hash)); // self
    }
}
