# Per-Author Merkle-DAG State Machine

**Date**: 2026-04-01
**Status**: Draft

## Problem

The current `willow-state` crate models shared state as a single linear
event chain. Every event carries a `parent_hash` referencing the state
hash after the previous event — forming a blockchain-like sequence:

```
E1 ← E2 ← E3 ← E4 ← E5
```

This has three structural problems that prevent scaling to mass peers:

### 1. Linearization bottleneck

Two peers producing events concurrently create a fork. The `merge()`
function resolves forks by collecting divergent events, **sorting by
wall-clock `timestamp_ms`**, and replaying onto the common ancestor
state. This means:

- Merge correctness depends on wall-clock time — peers with skewed
  clocks produce different merge orderings.
- The `timestamp_ms` field is a "display hint" that is also load-bearing
  for convergence. This is a contradiction.
- There is no partial order — events are either strictly ordered (same
  chain) or require a full re-sort to merge.

### 2. No per-peer authority

All events from all peers go into one pool. There is no structural
separation between "my events" and "their events." Consequences:

- A peer cannot revise their own history because events are immutable
  once added to the shared pool and marked in `seen_event_ids`.
- Archival disagreements are unresolvable — if the canonical chain says
  X happened, there's no mechanism for the author of X to say "actually
  I retract/revise X."
- Author validation is checked at apply-time but not structurally
  enforced — the data model permits an event with any `author` field.

### 3. O(n) state hashing on every event

`apply()` calls `state.hash()` which serializes the **entire**
`ServerState` (all channels, roles, members, messages, profiles,
permissions, channel keys) via bincode, sorts all HashMaps, then SHA-256
hashes the result. This is called on every `apply()` for the parent hash
check. At 100k messages across 1000 peers, this dominates all costs.

### 4. Sync is coarse-grained

The current sync protocol (`SyncRequest { latest_hlc }`) has no way to
request specific missing events. A peer either has the full chain or
gets a `SyncBatch` of everything newer than an HLC timestamp. There is
no per-author granularity, no gap detection, and no incremental sync.

## Design Goals

1. **Eventual consistency** — all peers that have seen the same set of
   events arrive at the same materialized state, regardless of reception
   order.

2. **No wall-clock dependence** — causal ordering derived from hash
   links, not timestamps. Timestamps may exist as display hints but
   never participate in ordering or merge logic.

3. **Author sovereignty** — each peer's contributions form a
   self-contained, independently verifiable chain. A peer can revise
   their own chain. Other peers accept validly-signed revisions.

4. **Author validation** — structurally enforced via Ed25519 signatures
   on per-author chains. A peer cannot produce events posing as another
   peer.

5. **Scalable sync** — peers exchange compact head summaries and request
   only missing events by author + sequence range. No full-state
   transfer required.

6. **Scalable hashing** — state verification does not require
   serializing the entire state. Incremental or per-author hashing.

7. **Archival tolerance** — current materialized state may diverge from
   historical snapshots. This is by design, not a bug.

8. **Zero I/O** — the state crate remains pure. No networking, no
   persistence, no async. Just `apply(dag, event) -> Result`.

## Prior Art

The design draws from established distributed systems research:

| System | Key idea adopted |
|---|---|
| **Merkle-CRDTs** (Sanjuán et al., 2020) | Merkle-DAGs as logical clocks; DAG union = merge; content-addressing for sync |
| **Secure Scuttlebutt** | Per-peer append-only signed hash chains; sync by sequence number |
| **Automerge** | Per-actor change logs; hash DAG of changes; columnar encoding; Lamport timestamps for op ordering |
| **AT Protocol** (Bluesky) | Per-user signed Merkle repositories; MST for efficient state diff/sync |
| **Delta-state CRDTs** (Almeida et al., 2016) | Idempotent deltas over unreliable channels; no exactly-once requirement |
| **PO-Log compaction** (Bauwens & Gonzalez Boix, 2020) | Causal stability detection for garbage collection of CRDT metadata |

## Overview

The core shift: **state is a deterministic projection of a DAG, not a
mutable object on a chain.**

```
Author A:   A1 ← A2 ← A3 ← A4
                  ↑         ↑
Author B:   B1 ← B2 ← B3   │
                  ↑    ↑    │
Author C:        C1 ← C2 ──┘
```

Each author maintains their own hash-linked chain of events (`←` = prev
pointer within the same author). Cross-author arrows (`↑`) are
dependency pointers — "I had seen event X from another author when I
produced this event." Together, these form a Merkle-DAG that encodes
the full causal history without timestamps or vector clocks.

Materialized state (`ServerState`) is computed by topologically sorting
the DAG and replaying events through a pure `apply()` function.
Concurrent events (no causal relationship) are tie-broken by content
hash — deterministic across all peers.

The following sections define the data model, materialization,
sync protocol, author revision, and compaction in detail.

## Section 1: Core Data Model

### Event

An event is the atomic unit of state mutation. Every event is
self-describing — its identity is its content hash.

```rust
/// A single state mutation, content-addressed and author-signed.
pub struct Event {
    /// Content hash of this event (SHA-256 of all other fields).
    /// This IS the event's identity — no separate UUID needed.
    pub hash: EventHash,

    /// Author's public key (Ed25519). Structurally identifies who
    /// produced this event. Verified via `sig`.
    pub author: EndpointId,

    /// Monotonically increasing sequence number within this author's
    /// chain. Starts at 1. Enables efficient sync: "give me author X's
    /// events after seq N."
    pub seq: u64,

    /// Hash of this author's previous event (`EventHash::ZERO` if this
    /// is the author's first event). Forms the per-author hash chain.
    pub prev: EventHash,

    /// Hashes of events from OTHER authors that this event has "seen."
    /// These are the cross-author causal dependency edges that form
    /// the DAG. May be empty (no cross-author dependencies).
    pub deps: Vec<EventHash>,

    /// The state mutation to apply.
    pub kind: EventKind,

    /// Ed25519 signature over (author, seq, prev, deps, kind).
    /// Proves authorship — cannot be forged without the private key.
    pub sig: Signature,

    /// Wall-clock timestamp hint (milliseconds). Used ONLY for display
    /// purposes (e.g., showing "2 hours ago" in the UI). Never used
    /// for ordering, merge, or any state logic.
    pub timestamp_hint_ms: u64,
}
```

**Key changes from current `Event`:**

| Current | New | Why |
|---|---|---|
| `id: String` (UUID) | `hash: EventHash` (SHA-256) | Content-addressed — identity is derived, not assigned |
| `parent_hash: StateHash` | `prev: EventHash` + `deps: Vec<EventHash>` | Per-author chain + cross-author DAG replaces single-chain parent |
| `timestamp_ms: u64` (used in merge) | `timestamp_hint_ms: u64` (display only) | No wall-clock in ordering logic |
| No signature | `sig: Signature` | Structural author validation |
| No sequence number | `seq: u64` | Efficient per-author sync |

### EventHash

```rust
/// SHA-256 hash of an event's content (excluding the hash itself).
/// Used as the event's identity and for all DAG links.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventHash(pub [u8; 32]);

impl EventHash {
    /// The zero hash — used as `prev` for an author's first event.
    pub const ZERO: EventHash = EventHash([0u8; 32]);

    /// Compute the hash of an event's signable content.
    pub fn compute(
        author: &EndpointId,
        seq: u64,
        prev: &EventHash,
        deps: &[EventHash],
        kind: &EventKind,
        timestamp_hint_ms: u64,
    ) -> Self { /* SHA-256 of canonical serialization */ }
}
```

### Per-Author Chain

Each author's events form a singly-linked hash chain via `prev`:

```
Author A:  [A1] ←prev── [A2] ←prev── [A3] ←prev── [A4]
           seq=1         seq=2         seq=3         seq=4
```

Properties:
- **Unforgeable**: every event is signed by the author's Ed25519 key.
- **Tamper-evident**: changing any event invalidates all subsequent
  `prev` hashes in the chain.
- **Gap-detectable**: if you receive A4 with `prev = hash(A3)` but
  don't have A3, you know you're missing data.
- **Efficient sync**: request "author A, events after seq 2" to get
  exactly {A3, A4}.
- **Author-revisable**: the author can republish their chain (see
  Section 5: Author History Revision).

### Cross-Author Dependencies (the DAG)

The `deps` field links events across authors:

```
A1(seq=1, deps=[])
A2(seq=2, deps=[B1])      ← "when I produced A2, I had seen B1"
B1(seq=1, deps=[])
B2(seq=2, deps=[A1])      ← "when I produced B2, I had seen A1"
B3(seq=3, deps=[A2, C1])  ← "when I produced B3, I had seen A2 and C1"
C1(seq=1, deps=[B1])
```

This creates a partial order:
- `A1 < A2` (same author, seq ordering)
- `B1 < A2` (A2 depends on B1)
- `A1 < B2` (B2 depends on A1)
- `A1 ∥ B1` (concurrent — neither depends on the other)

The `deps` field is **advisory, not exhaustive**. An event only needs to
list the *heads* (latest known events) of other authors at the time of
creation. Transitivity fills in the rest — if A2 depends on B3, and B3
depends on C1, then A2 transitively depends on C1.

### EventDag

The DAG is the central data structure — the source of truth from which
all state is derived.

```rust
/// The Merkle-DAG of all known events across all authors.
pub struct EventDag {
    /// All events indexed by hash.
    events: HashMap<EventHash, Event>,

    /// Per-author chain index: author -> ordered vec of event hashes.
    /// Invariant: chains[author][i].seq == i + 1
    chains: HashMap<EndpointId, Vec<EventHash>>,

    /// Current head (latest event hash) per author.
    heads: HashMap<EndpointId, EventHash>,
}
```

Core operations:

```rust
impl EventDag {
    /// Insert a verified event into the DAG.
    /// Returns error if:
    /// - Signature verification fails
    /// - seq is not exactly prev_seq + 1 for this author
    /// - prev hash doesn't match the author's current head
    /// - Any dep references an unknown event (gap detected)
    pub fn insert(&mut self, event: Event) -> Result<(), InsertError>;

    /// Get all events an author has produced, in seq order.
    pub fn author_events(&self, author: &EndpointId) -> &[EventHash];

    /// Get the current head hash for an author.
    pub fn head(&self, author: &EndpointId) -> Option<&EventHash>;

    /// Get the latest seq number for an author (0 if unknown).
    pub fn latest_seq(&self, author: &EndpointId) -> u64;

    /// Topological sort of all events. Concurrent events are
    /// tie-broken by EventHash (deterministic across all peers).
    pub fn topological_sort(&self) -> Vec<&Event>;

    /// Check if event A is a causal ancestor of event B.
    pub fn is_ancestor(&self, a: &EventHash, b: &EventHash) -> bool;

    /// Return all events not yet seen by a peer, given their
    /// known heads: HashMap<EndpointId, u64> (author -> latest seq).
    pub fn events_since(
        &self,
        their_heads: &HashMap<EndpointId, u64>,
    ) -> Vec<&Event>;
}
```

### InsertError

```rust
pub enum InsertError {
    /// Ed25519 signature does not verify.
    InvalidSignature,
    /// seq is not prev_seq + 1 for this author.
    SeqGap { author: EndpointId, expected: u64, got: u64 },
    /// prev hash doesn't match author's current head.
    PrevMismatch { author: EndpointId, expected: EventHash, got: EventHash },
    /// A dep references an event we don't have yet.
    MissingDep(EventHash),
    /// Event with this hash already exists.
    Duplicate,
}
```

`MissingDep` is not a permanent failure — it means the event arrived
out of order. The caller should buffer it and retry after fetching the
missing dependency. This is standard in Merkle-CRDT systems.

### EventKind (unchanged in structure)

The `EventKind` enum remains the same — it defines *what* mutations are
possible. The change is in *how* events are structured and ordered, not
what they do. All 24 current variants carry over as-is:

```rust
pub enum EventKind {
    CreateChannel { name: String, channel_id: String, kind: String },
    DeleteChannel { channel_id: String },
    RenameChannel { channel_id: String, new_name: String },
    CreateRole { name: String, role_id: String },
    DeleteRole { role_id: String },
    SetPermission { role_id: String, permission: String, granted: bool },
    AssignRole { peer_id: EndpointId, role_id: String },
    GrantPermission { peer_id: EndpointId, permission: Permission },
    RevokePermission { peer_id: EndpointId, permission: Permission },
    KickMember { peer_id: EndpointId },
    Message { channel_id: String, body: String, reply_to: Option<String> },
    EditMessage { message_id: EventHash, new_body: String },
    DeleteMessage { message_id: EventHash },
    Reaction { message_id: EventHash, emoji: String },
    SetProfile { display_name: String },
    RotateChannelKey { channel_id: String, encrypted_keys: Vec<(EndpointId, Vec<u8>)> },
    PinMessage { channel_id: String, message_id: EventHash },
    UnpinMessage { channel_id: String, message_id: EventHash },
    RenameServer { new_name: String },
    SetServerDescription { description: String },
    StateVerification { state_hash: StateHash },
}
```

Note: `message_id` fields change from `String` (UUID) to `EventHash`
since message IDs are now the content hash of the `Message` event that
created them.

## Section 2: State Materialization

`ServerState` is no longer directly mutated. It is a **materialized
view** — a projection computed deterministically from the DAG.

### The Materialization Function

```rust
/// Compute the current server state from the full event DAG.
///
/// This is the ONLY way to derive state. The function is pure,
/// deterministic, and produces identical output on all peers
/// given the same DAG contents.
pub fn materialize(dag: &EventDag, owner: EndpointId) -> ServerState {
    let sorted = dag.topological_sort();
    let mut state = ServerState::new(owner);
    for event in sorted {
        // apply_unchecked: no parent hash check, no dedup check.
        // The DAG structure guarantees no duplicates and the
        // topological sort guarantees causal ordering.
        apply_unchecked(&mut state, event);
    }
    state
}
```

### Topological Sort with Deterministic Tiebreaking

The DAG defines a partial order. To produce a total order for replay,
we topologically sort and break ties deterministically.

**Ordering rules (in priority order):**

1. **Causal order**: if event A is in event B's transitive dependency
   set (via `prev` or `deps`), A comes before B.
2. **Deterministic tiebreaker**: for concurrent events (neither is an
   ancestor of the other), sort by `EventHash` bytes
   (lexicographic comparison of the 32-byte SHA-256 digest).

Hash-based tiebreaking is:
- **Deterministic** — same events produce same order on all peers.
- **Time-independent** — no wall-clock involved.
- **Unpredictable** — no peer can game the ordering (would need to
  find SHA-256 preimages).
- **Stable** — adding new events does not change the relative order
  of existing concurrent events.

```rust
impl EventDag {
    pub fn topological_sort(&self) -> Vec<&Event> {
        // Kahn's algorithm with a BinaryHeap keyed by EventHash
        // for deterministic selection among ready nodes.
        let mut in_degree: HashMap<EventHash, usize> = HashMap::new();
        let mut ready: BTreeSet<&EventHash> = BTreeSet::new();
        let mut result: Vec<&Event> = Vec::new();

        // Compute in-degrees from prev + deps edges.
        for event in self.events.values() {
            let deps = self.causal_parents(event);
            *in_degree.entry(event.hash.clone()).or_default() += 0;
            for dep in deps {
                *in_degree.entry(event.hash.clone()).or_default() += 1;
            }
        }

        // Seed with zero-indegree events.
        for (hash, &degree) in &in_degree {
            if degree == 0 {
                ready.insert(hash);
            }
        }

        // Process in deterministic order (BTreeSet sorts by hash).
        while let Some(hash) = ready.pop_first() {
            let event = &self.events[hash];
            result.push(event);
            for dependent in self.dependents(hash) {
                let d = in_degree.get_mut(dependent).unwrap();
                *d -= 1;
                if *d == 0 {
                    ready.insert(dependent);
                }
            }
        }

        result
    }

    /// Return all causal parents of an event: prev (if not ZERO)
    /// plus all deps.
    fn causal_parents(&self, event: &Event) -> Vec<&EventHash> {
        let mut parents = Vec::new();
        if event.prev != EventHash::ZERO {
            parents.push(&event.prev);
        }
        parents.extend(event.deps.iter());
        parents
    }
}
```

### Incremental Materialization

Full replay from genesis is only needed on first bootstrap. During
normal operation, state is maintained incrementally:

```rust
impl EventDag {
    /// Apply a single new event to an existing materialized state.
    ///
    /// Preconditions:
    /// - All causal parents of `event` are already reflected in `state`
    ///   (i.e., all events in the current DAG that causally precede
    ///   this one have been applied).
    /// - The event has been inserted into the DAG already.
    ///
    /// This is O(1) per event — no re-sort, no full replay.
    pub fn apply_incremental(
        state: &mut ServerState,
        event: &Event,
    ) -> ApplyResult {
        apply_unchecked(state, event)
    }
}
```

When a new event arrives and all its dependencies are already in the
DAG, it can be applied directly without re-materializing. The only
case requiring a partial re-sort is when buffered out-of-order events
become ready after a dependency arrives — those are applied in
topological order among themselves.

### apply_unchecked

The core apply function is simplified — it no longer checks parent
hashes or dedup (the DAG handles both structurally):

```rust
/// Apply an event's mutation to state. No structural validation —
/// the DAG guarantees ordering and dedup. Only permission checks
/// remain.
fn apply_unchecked(state: &mut ServerState, event: &Event) -> ApplyResult {
    // Permission enforcement (same as current apply_inner).
    let required = required_permission(&event.kind);
    if let Some(ref perm) = required {
        if !state.has_permission(&event.author, perm) {
            return ApplyResult::Rejected(format!(
                "author '{}' lacks {:?} permission", event.author, perm
            ));
        }
    }

    // Apply the mutation (same match block as current apply_inner).
    apply_mutation(state, event)
}
```

### ServerState Changes

`ServerState` loses the fields that were artifacts of the linear chain:

```rust
pub struct ServerState {
    // Unchanged fields:
    pub server_id: String,
    pub server_name: String,
    pub owner: EndpointId,
    pub channels: HashMap<String, Channel>,
    pub roles: HashMap<String, Role>,
    pub members: HashMap<EndpointId, Member>,
    pub peer_permissions: HashMap<EndpointId, HashSet<Permission>>,
    pub messages: Vec<ChatMessage>,
    pub profiles: HashMap<EndpointId, Profile>,
    pub description: String,
    pub channel_keys: HashMap<String, Vec<u8>>,

    // REMOVED: seen_event_ids: HashSet<String>
    //   → Dedup is structural (DAG rejects duplicate hashes)
    //
    // REMOVED: hash() method that serializes entire state
    //   → State hashing is optional and computed differently
    //     (see Section 6: Compaction & Snapshots)
}
```

### Concurrency Semantics

Because events from different authors can be concurrent (no causal
relationship), the apply function must handle all EventKind variants
idempotently and commutatively where possible:

| EventKind | Concurrent behavior |
|---|---|
| `CreateChannel` | Two peers creating the same `channel_id` concurrently: first wins (skip if exists). Deterministic because topological sort is deterministic. |
| `DeleteChannel` + `Message` to same channel | If delete sorts first, message targets a missing channel — silently ignored. If message sorts first, it's created then channel + messages deleted. Both are valid eventual states. |
| `EditMessage` + `DeleteMessage` on same message | Last-writer-wins by sort order. Both are acceptable. |
| `GrantPermission` + `KickMember` on same peer | Sort-order determines whether the grant applies before the kick removes it, or the kick happens first and the grant re-adds the member. Both converge. |
| `Reaction` by two peers | Both reactions are added (additive, no conflict). |

The key property: **all peers with the same DAG contents produce the
same materialized state.** The specific outcome of concurrent events
may be arbitrary, but it is *consistently* arbitrary.

## Section 3: Sync Protocol

Sync is the mechanism by which peers bring their DAGs into agreement.
The protocol is per-author and incremental — peers never need to
transfer full state.

### Heads Summary

Each peer maintains a compact summary of their DAG:

```rust
/// Compact representation of what a peer knows.
/// Maps each known author to their latest seq number and head hash.
pub struct HeadsSummary {
    pub heads: HashMap<EndpointId, AuthorHead>,
}

pub struct AuthorHead {
    /// Latest seq number for this author.
    pub seq: u64,
    /// Hash of the latest event from this author.
    pub hash: EventHash,
}
```

This is O(number of authors), not O(number of events). For a server
with 1000 members, each of whom has published events, this is ~1000
entries of ~40 bytes each = ~40KB. Compact enough to broadcast.

### Sync Messages

```rust
pub enum SyncMessage {
    /// "Here's what I have." Sent on connect and periodically.
    Advertise(HeadsSummary),

    /// "I need events from these authors after these seq numbers."
    Request(Vec<AuthorRequest>),

    /// "Here are events you're missing."
    Response(Vec<Event>),
}

pub struct AuthorRequest {
    pub author: EndpointId,
    /// Send me events with seq > after_seq.
    pub after_seq: u64,
}
```

### Sync Flow

```
Peer A                              Peer B
  │                                    │
  │──── Advertise(my_heads) ──────────>│
  │<─── Advertise(their_heads) ────────│
  │                                    │
  │  [compare heads, compute diff]     │  [compare heads, compute diff]
  │                                    │
  │──── Request([{alice, after:5}]) ──>│
  │<─── Request([{bob, after:3}]) ─────│
  │                                    │
  │<─── Response([alice_6, alice_7]) ──│
  │──── Response([bob_4, bob_5]) ─────>│
  │                                    │
  │  [insert into DAG, apply]          │  [insert into DAG, apply]
```

**Step by step:**

1. **Exchange heads.** Both peers broadcast their `HeadsSummary` over
   the gossip topic (or direct connection).

2. **Compute diff.** Each peer compares the received heads against
   their own DAG. For each author:
   - If `their_seq > my_seq`: I need events from that author.
   - If `my_seq > their_seq`: they need events from me (I'll wait for
     their request, or proactively send).
   - If `their_hash != my_hash` at the same seq: the author revised
     their chain (see Section 5).

3. **Request missing events.** Send `Request` listing which authors
   and after which seq number.

4. **Respond with events.** Send the requested events in seq order
   per author. Events include their full content (kind, deps, sig)
   so the receiver can verify independently.

5. **Insert and apply.** Receiver verifies signatures, inserts into
   DAG, applies incrementally to materialized state.

### Gossip Integration

For the common case (real-time chat), events are broadcast eagerly via
iroh-gossip. The sync protocol above is for catch-up after disconnection
or when a new peer joins. Both mechanisms coexist:

- **Eager push**: when a peer creates an event, broadcast it to the
  gossip topic immediately. Other peers insert it as it arrives.
- **Lazy pull**: periodically (or on connect), exchange heads and
  request missing events. This catches anything missed during
  partitions.

This matches the anti-entropy pattern from Merkle-CRDT systems: gossip
for real-time, pull-based reconciliation for consistency.

### Gap Handling

When an event arrives referencing a `prev` or `dep` that the receiver
doesn't have:

1. Buffer the event in a pending queue.
2. Send a `Request` for the missing author's events.
3. When the missing events arrive, insert them first, then retry the
   buffered event.
4. If the gap persists after N retries, log a warning but don't block
   other authors' events. The peer may be offline or the chain may
   have been revised.

```rust
pub struct PendingBuffer {
    /// Events waiting for missing dependencies.
    /// Key: the missing EventHash they're waiting on.
    waiting_on: HashMap<EventHash, Vec<Event>>,
}

impl PendingBuffer {
    /// Call when a new event is inserted into the DAG.
    /// Returns any buffered events that are now ready.
    pub fn resolve(&mut self, inserted: &EventHash) -> Vec<Event> {
        self.waiting_on.remove(inserted).unwrap_or_default()
    }
}
```

### Bandwidth Considerations

| Scenario | Data transferred |
|---|---|
| Real-time message | 1 event (~200-500 bytes) via gossip |
| New peer joins (1000 authors, 100 events each) | Heads summary (~40KB) + 100k events (~20-50MB) |
| Reconnect after 10 minutes offline | Heads summary + delta (~few KB to few MB) |
| Author revises their chain | Full chain for that author only |

For large catch-up scenarios, events can be batched and compressed.
The per-author structure enables parallel fetching — request events
from different authors concurrently.

## Section 4: Author History Revision

A core design goal: peers can modify the history of their own work.
The per-author chain structure makes this possible without breaking
other peers' contributions.

### How Revision Works

An author "revises" their chain by publishing a new chain signed with
the same key. The new chain may have different events, different
ordering, or fewer events than the original.

```
Original chain:  A1 ← A2 ← A3 ← A4  (head seq=4)
Revised chain:   A1 ← A2' ← A3'      (head seq=3)
```

Author A has:
- Kept A1 as-is
- Replaced A2 with A2' (perhaps editing a message)
- Replaced A3 with A3' (perhaps removing an event)
- Dropped A4 entirely

### Detection

During sync, a peer receives a `HeadsSummary` where author A has
`seq=3, hash=X` but the local DAG has author A at `seq=4, hash=Y`.
Even if the seq is the same, if the hash differs, the chain has been
revised.

The general rule:

```rust
enum ChainStatus {
    /// Their chain extends ours (normal: they have newer events).
    Ahead { new_events: u64 },
    /// Our chain extends theirs (we have events they haven't seen).
    Behind { missing_events: u64 },
    /// Same head.
    Synced,
    /// Chains diverge — author has revised their history.
    Revised,
}

fn compare_chains(
    our_head: &AuthorHead,
    their_head: &AuthorHead,
    our_chain: &[EventHash],
) -> ChainStatus {
    if our_head.hash == their_head.hash {
        return ChainStatus::Synced;
    }
    if their_head.seq > our_head.seq {
        // Could be Ahead or Revised — need to verify the chain
        // prefix matches.
        return ChainStatus::Ahead { new_events: their_head.seq - our_head.seq };
    }
    if their_head.seq < our_head.seq {
        return ChainStatus::Behind { missing_events: our_head.seq - their_head.seq };
    }
    // Same seq, different hash — definitely revised.
    ChainStatus::Revised
}
```

### Accepting Revisions

When a revision is detected:

1. **Fetch the full revised chain** from the author (or a peer that
   has it). Since chains are typically small (an author's personal
   events), this is not expensive.

2. **Verify the entire chain**: every event signed by the author's
   key, seq numbers monotonic, prev hashes consistent.

3. **Replace** the author's chain in the local DAG.

4. **Re-materialize state** — remove the old chain's contributions,
   replay with the new chain. In practice this means a full
   re-materialization from the DAG (which is fast for reasonable
   event counts).

5. **Update deps**: other authors' events may have `deps` pointing to
   events that no longer exist in the revised chain. This is handled
   gracefully — a dep pointing to a removed event is treated as
   unresolvable but not fatal. The dependent event still exists and
   is still applied; it just has a broken causal link to the revised
   author. This is the "archival tolerance" design goal in action.

```rust
impl EventDag {
    /// Replace an author's chain with a revised version.
    /// The new chain must be fully signed and internally consistent.
    pub fn replace_chain(
        &mut self,
        author: &EndpointId,
        new_chain: Vec<Event>,
    ) -> Result<(), RevisionError> {
        // Verify all signatures and internal consistency.
        self.verify_chain(author, &new_chain)?;

        // Remove old events for this author.
        if let Some(old_hashes) = self.chains.get(author) {
            for hash in old_hashes {
                self.events.remove(hash);
            }
        }

        // Insert new events.
        let new_hashes: Vec<EventHash> = new_chain.iter()
            .map(|e| e.hash.clone())
            .collect();
        for event in new_chain {
            self.events.insert(event.hash.clone(), event);
        }
        self.chains.insert(*author, new_hashes);

        // Update head.
        if let Some(last) = self.chains[author].last() {
            self.heads.insert(*author, last.clone());
        } else {
            self.heads.remove(author);
        }

        Ok(())
    }
}
```

### What Revision Cannot Do

- **Cannot forge another author's events.** Revision only works for
  chains signed by your own key.
- **Cannot rewrite deps in other authors' events.** If Bob's event B3
  has `deps: [A2]` and Alice revises away A2, Bob's B3 still says
  `deps: [A2]`. The broken dep is a fact of history — Bob really did
  see A2 at the time.
- **Cannot violate seq monotonicity.** A revised chain must still have
  contiguous seq numbers starting from 1.

### Conflict Resolution for Revisions

If two peers have different versions of an author's chain (e.g., the
author sent revision V2 to some peers but not others), the rule is:

**Longest chain wins.** If equal length, the chain with the
lexicographically greater head hash wins. This is deterministic and
converges — eventually all peers will see all versions and pick the
same winner.

In the common case, the author publishes one revision and it propagates
to all peers. The conflict case is rare (partitioned network during
revision) and resolves automatically.

### Archival Implications

Archival nodes may choose to retain old chain versions alongside the
current one. This is outside the scope of the state crate — the state
crate only deals with the current DAG. But the data model supports it:
old events are just events with valid signatures that are no longer in
the active DAG.

## Section 5: Compaction & Snapshots

Without compaction, the DAG grows without bound. Every event ever
produced is retained, and new peers must replay the entire history to
materialize state. This section defines how to bound growth.

### Snapshots

A snapshot captures the materialized state at a point in time, along
with the DAG heads that produced it:

```rust
/// A frozen checkpoint of the DAG and materialized state.
pub struct Snapshot {
    /// The materialized state at this point.
    pub state: ServerState,

    /// The heads (author -> latest seq + hash) at snapshot time.
    /// These define exactly which events are included.
    pub heads: HeadsSummary,

    /// SHA-256 hash of the canonical serialization of (state, heads).
    /// Used for verification.
    pub hash: SnapshotHash,
}
```

Snapshots serve two purposes:

1. **New peer bootstrap**: instead of replaying 100k events, download
   a snapshot + events after it.
2. **Memory reclamation**: events before the snapshot can be discarded
   from the active DAG (archival nodes may retain them).

### Snapshot Creation

Any peer can create a snapshot at any time. Snapshots are **not
consensus** — different peers may create snapshots at different points.
This is fine because:

- Snapshots are just a cache optimization, not a source of truth.
- The DAG is the source of truth. A snapshot can always be verified
  by replaying from genesis (or from an earlier snapshot).
- If a peer receives a snapshot they don't trust, they can request
  the full event history and verify independently.

```rust
impl EventDag {
    /// Create a snapshot of the current state.
    pub fn snapshot(&self, state: &ServerState) -> Snapshot {
        let heads = self.heads_summary();
        let hash = SnapshotHash::compute(state, &heads);
        Snapshot { state: state.clone(), heads, hash }
    }

    /// Discard events at or before the snapshot heads.
    /// Only retains events after the snapshot point.
    pub fn compact(&mut self, snapshot: &Snapshot) {
        for (author, head) in &snapshot.heads.heads {
            if let Some(chain) = self.chains.get_mut(author) {
                // Find the index of the snapshot head in this chain.
                let cutoff = chain.iter()
                    .position(|h| *h == head.hash)
                    .map(|i| i + 1)
                    .unwrap_or(0);

                // Remove events before the cutoff.
                let removed: Vec<_> = chain.drain(..cutoff).collect();
                for hash in removed {
                    self.events.remove(&hash);
                }
            }
        }
    }
}
```

### Snapshot Trust

When a new peer bootstraps from a snapshot, they must decide whether
to trust it. The trust model:

1. **Owner-signed snapshots**: the server owner can sign snapshots,
   providing a root-of-trust endorsement.
2. **Multi-peer agreement**: request the snapshot hash from multiple
   peers. If a majority agree, the snapshot is likely correct.
3. **Full verification**: replay from genesis to verify. Expensive
   but authoritative.

The state crate provides the data structures; the trust decision is
made at the client/network layer.

### Causal Stability (Future Optimization)

An event is **causally stable** when every active peer has seen it —
no more concurrent events can arrive that are causally independent of
it. Causally stable events can have their causal metadata (`deps`)
stripped, reducing per-event storage.

Detecting causal stability requires knowing the active peer set,
which is inherently imprecise in a P2P system. This is deferred to a
future iteration. For now, snapshot-based compaction provides
sufficient memory management.

### Compaction and Author Revision Interaction

If a snapshot includes events from author A at seq 5, and author A
later revises their chain (changing events at seq 3-5), the snapshot
is invalidated for that author. The resolution:

- Re-materialize from the most recent valid snapshot that predates
  the revision, plus all events after it.
- In the worst case (author revises events from before any snapshot),
  re-materialize from genesis.
- Archival nodes that retain full history can always provide this.

This is an intentional tradeoff: author sovereignty over their data
takes priority over snapshot stability.

## Section 6: Migration Path

The new model is a significant change to willow-state's internals but
preserves the same `EventKind` variants and `ServerState` fields. The
migration can be done incrementally.

### Phase 1: EventDag + New Event Type

Add the new data structures alongside the existing ones:

1. Add `EventHash`, `EventDag`, `PendingBuffer` to willow-state.
2. Add new `Event` struct (with `prev`, `deps`, `seq`, `sig`) as
   `DagEvent` to avoid name collision.
3. Add `topological_sort()` and `materialize()`.
4. Add comprehensive tests for the new path.
5. The old `Event`, `apply()`, `merge()` remain untouched.

### Phase 2: Dual-Mode Apply

The client layer can operate in either mode:

- **Legacy mode**: use old `Event` + `apply()` for existing servers.
- **DAG mode**: use `DagEvent` + `EventDag` + `materialize()` for
  new servers or migrated servers.

A server's mode is determined at creation time and stored in metadata.

### Phase 3: Sync Protocol Migration

Update the sync layer to use `HeadsSummary` + per-author requests
instead of `SyncRequest { latest_hlc }`. The new sync protocol can
coexist with the old one (different gossip topic or message type).

### Phase 4: Deprecate Legacy Path

Once all servers have migrated, remove the old `Event`, `apply()`,
`apply_lenient()`, `merge()`, `StateHash`, and `seen_event_ids`.

### What Stays the Same

- `EventKind` enum (all 24 variants)
- `ServerState` struct (minus `seen_event_ids`)
- `Permission` enum and `has_permission()` logic
- All pure data types (`Channel`, `Role`, `Member`, `ChatMessage`,
  `Profile`)
- Permission enforcement in apply
- Zero-I/O crate boundary

### What Changes

| Component | Before | After |
|---|---|---|
| `Event.id` | UUID string | Content hash (`EventHash`) |
| `Event.parent_hash` | `StateHash` (hash of entire state) | `prev: EventHash` (author's previous event) |
| Causal links | Implicit (linear chain) | Explicit (`deps: Vec<EventHash>`) |
| Author validation | Checked at apply time | Structural (Ed25519 signature in event) |
| Deduplication | `seen_event_ids: HashSet` | DAG rejects duplicate hashes |
| Merge | Sort by timestamp, replay | Topological sort of DAG, hash tiebreak |
| Sync | HLC-based, coarse-grained | Per-author seq-based, incremental |
| State hashing | Full serialization + SHA-256 | Optional snapshots, no per-event hash |
| History revision | Not possible | Author can republish their chain |

## Appendix: References

1. Sanjuán, Pöyhtäri, Teixeira. "Merkle-CRDTs: Merkle-DAGs meet CRDTs." arXiv:2004.00107, 2020.
2. Auvolat, Taïani. "Merkle Search Trees: Efficient State-Based CRDTs in Open Networks." SRDS, 2019.
3. Almeida, Shoker, Baquero. "Delta State Replicated Data Types." arXiv:1603.01529, 2016.
4. Bauwens, Gonzalez Boix. "From Causality to Stability: Understanding and Reducing Meta-Data in CRDTs." MPLR, 2020.
5. Secure Scuttlebutt Protocol. https://scuttlebot.io/more/protocols/secure-scuttlebutt.html
6. AT Protocol Repository Spec. https://atproto.com/specs/repository
7. Automerge Binary Document Format. https://automerge.org/automerge-binary-format-spec/
8. iroh by n0. https://github.com/n0-computer/iroh
