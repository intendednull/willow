# Per-Author Merkle-DAG State Machine

**Date**: 2026-04-01
**Status**: Ready for implementation

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
    /// Content hash of this event — SHA-256 of the signable fields
    /// (author, seq, prev, deps, kind, timestamp_hint_ms).
    /// Excludes `hash` itself and `sig`. This IS the event's identity.
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

    /// Ed25519 signature over (author, seq, prev, deps, kind, timestamp_hint_ms).
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
///
/// Defined in `hash.rs`. Does not depend on `EventKind` — it is a
/// pure hash wrapper. The computation that serializes event fields
/// and produces the hash lives in `Event::new()` in `event.rs`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct EventHash(pub [u8; 32]);

impl EventHash {
    /// The zero hash — used as `prev` for an author's first event.
    pub const ZERO: EventHash = EventHash([0u8; 32]);

    /// Hash arbitrary bytes with SHA-256.
    pub fn from_bytes(data: &[u8]) -> Self { /* SHA-256 */ }
}
```

`Ord` is derived from lexicographic byte comparison — used by
`BTreeSet` in topological sort for deterministic tiebreaking.

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
  Section 4: Author History Revision).

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

### Server Identity and Genesis Event

A server begins with a **genesis event** — the first event in the
DAG, published by the server's creator. The genesis event
is a `CreateServer` event:

```rust
EventKind::CreateServer { name: String }
```

The `server_id` is derived from the genesis event:

```
server_id = hex(genesis_event.hash)
```

This cryptographically binds the server's identity to its creator and
creation moment. Properties:

- **Content-addressed**: the same creator making a server with the same
  name at different times produces different IDs (different timestamps,
  different hashes).
- **Unforgeable**: the genesis event is signed by the creator's key.
  No one else can produce an event that hashes to this server_id.
- **Permanent**: changing the genesis event would change the server_id,
  making it a different server.

### Governance Model

There is no permanent "owner." The genesis author starts as the sole
admin, but has no special permanent status beyond that. All privileged
operations (granting/revoking admin, kicking members, changing the vote
threshold) go through a **propose → vote → auto-apply** process.

Admin status is structurally separate from permissions. The `Permission`
enum does not contain `Administrator` — admin status is tracked in a
dedicated `admins: HashSet<EndpointId>` field on `ServerState`. This
makes it impossible for any peer (malicious or otherwise) to grant
admin via a direct `GrantPermission` event. The type system enforces
the governance path.

#### No single point of failure

- The genesis author can be outvoted by other admins.
- If the genesis author leaves, the server continues with remaining admins.
- A community's governance is emergent from the admin set, not tied to
  a single key.

#### Vote threshold

The default threshold is **unanimous** — all admins must approve. The
threshold itself can be changed via a `SetVoteThreshold` proposal,
which must pass under the *current* threshold. This means moving from
unanimous to majority requires unanimous agreement first.

```rust
pub enum VoteThreshold {
    /// All admins must approve (default).
    Unanimous,
    /// More than half of admins must approve.
    Majority,
    /// A specific count of admins must approve (capped at admin count).
    Count(u32),
}
```

#### Auto-apply on threshold

There is no `Resolve` event. When a `Vote` event brings the yes-count
to the vote threshold during materialization, the proposed action is
applied immediately. No single admin "declares" the result — it is a
deterministic consequence of the DAG state.

```
Admin A:  Propose(GrantAdmin{bob})     ← proposer = implicit yes (1/3)
                    ↑
Admin B:  Vote(yes) │                  ← 2/3 votes
                    ↑
Admin C:  Vote(yes) │                  ← 3/3, threshold met → applied
```

This eliminates:
- Conflicting resolution events (no resolution event exists)
- Race conditions between admins claiming different outcomes
- Action substitution attacks (action comes only from the Propose)
- Need for resolver ≠ proposer rules

The "proof" that a vote passed is the Vote events themselves, signed
and positioned in each voter's chain. Any peer can verify by replaying
the DAG.

#### Bootstrap sequence

1. Genesis `CreateServer` event. Genesis author is sole admin.
2. Sole admin proposes `GrantAdmin{alice}`. Quorum = 1 (unanimous of 1).
   Proposer is implicit "yes." Auto-applies immediately during
   materialization when the Propose event is processed.
3. Now 2 admins. Propose `GrantAdmin{bob}`. Both must vote (unanimous).
4. Now 3 admins. They can propose `SetVoteThreshold(Majority)`. All 3
   must agree. Once applied, future proposals need only 2 of 3.

#### Edge cases

- **Last admin revokes themselves**: passes with quorum of 1. Server
  has 0 admins. No more privileged ops. Members can still chat. The
  server is effectively locked.
- **Two admins revoke each other concurrently**: topological sort
  determines which lands first. The first revocation removes the second
  admin's power before their revocation applies. Deterministic on all
  peers.
- **`Count(n)` where n > admin count**: effectively unanimous — capped
  at the actual admin count.

#### Known tradeoffs

**Unanimous deadlock.** With the default unanimous threshold and 3
admins, if one admin loses their key, goes offline permanently, or
refuses to vote, no governance action can ever pass — including
changing the threshold. The server is frozen for admin changes.
Servers that want resilience should proactively lower the threshold
while all admins are active. This is an accepted tradeoff: unanimous
is safe but brittle, majority is flexible but can be exploited by
colluding admins. There is no free lunch. See intendednull/willow#22
for future hardening work including community-based fallback voting.

**Vote retraction.** A voter can revise their chain to remove a vote.
On re-materialization, the yes-count drops below threshold and the
action is undone. This is author sovereignty working as designed — the
retraction is visible (the Vote event used to exist at a known seq in
the voter's chain) and other admins can re-propose. If a peer
repeatedly retracts, other admins can vote to revoke their admin.

**Majority collusion.** With majority threshold, a majority of admins
can collude to kick the minority and seize control. This is inherent
in any majority-rule system. The default unanimous threshold protects
against this at the cost of the deadlock risk above.

#### What requires a vote

The `ProposedAction` enum defines exactly which actions require a vote.
These actions can ONLY be applied through the vote path — the data
model makes any other path structurally impossible.

| Action | Mechanism |
|---|---|
| Grant admin status | `ProposedAction::GrantAdmin` via Propose → Vote |
| Revoke admin status | `ProposedAction::RevokeAdmin` via Propose → Vote |
| Kick a member | `ProposedAction::KickMember` via Propose → Vote |
| Change vote threshold | `ProposedAction::SetVoteThreshold` via Propose → Vote |
| Grant non-admin permissions (`ManageChannels`, etc.) | Direct `GrantPermission` event by any admin |
| Revoke non-admin permissions | Direct `RevokePermission` event by any admin |
| Create/delete/rename channels | Direct event by authorized peer |
| Send messages, reactions, profiles | Direct event by any member |
| Rename server, set description | Direct event by any admin |

`materialize()` derives the initial admin from the genesis event:

```rust
pub fn materialize(dag: &EventDag) -> ServerState {
    let genesis = dag.genesis().expect("DAG must have a genesis event");
    let server_id = genesis.hash.to_string();
    let name = match &genesis.kind {
        EventKind::CreateServer { name } => name.clone(),
        _ => panic!("genesis event must be CreateServer"),
    };

    let sorted = dag.topological_sort();
    let mut state = ServerState::new(&server_id, &name, genesis.author);
    for event in sorted {
        apply_unchecked(&mut state, event);
    }
    state
}
```

The genesis author is passed to `ServerState::new()` which grants them
admin status in the initial state. From that point, all admin changes
go through the voting process.

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

    /// Hash of the genesis event (CreateServer). Set on first insert.
    /// Also serves as the server_id.
    genesis_hash: Option<EventHash>,
}
```

Core operations:

```rust
impl EventDag {
    /// Insert a verified event into the DAG.
    ///
    /// The first event inserted must be `EventKind::CreateServer`.
    /// It becomes the genesis event and its hash is the server_id.
    ///
    /// Returns error if:
    /// - Signature verification fails
    /// - seq is not exactly prev_seq + 1 for this author
    /// - prev hash doesn't match the author's current head
    /// - First event is not CreateServer
    /// Unknown deps are accepted (soft-accept) and recorded for
    /// background resolution.
    pub fn insert(&mut self, event: Event) -> Result<(), InsertError>;

    /// The genesis event. None if the DAG is empty.
    pub fn genesis(&self) -> Option<&Event>;

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
    /// First event in DAG must be EventKind::CreateServer.
    NotGenesis,
    /// seq is not prev_seq + 1 for this author.
    SeqGap { author: EndpointId, expected: u64, got: u64 },
    /// prev hash doesn't match author's current head.
    PrevMismatch { author: EndpointId, expected: EventHash, got: EventHash },
    /// Event with this hash already exists.
    Duplicate,
}
```

Note: there is no `MissingDep` error. The `deps` field is advisory —
if a dep references an event we don't have, the event is still
accepted and inserted. The missing dep is recorded for background
resolution (the caller can fetch it lazily). This is a deliberate
choice: soft-accept makes the system robust to partitions, author
revisions that remove dep targets, and out-of-order delivery. The
causal information in `deps` is best-effort, not a hard constraint.

### EventKind (unchanged in structure)

The `EventKind` enum remains the same — it defines *what* mutations are
possible. The change is in *how* events are structured and ordered, not
what they do. The remaining variants carry over as-is:

```rust
pub enum EventKind {
    // -- Server lifecycle --
    CreateServer { name: String },

    // -- Governance (vote-based, auto-apply on threshold) --
    Propose { action: ProposedAction },
    Vote { proposal: EventHash, accept: bool },

    // -- Permissions (direct, by any admin) --
    /// Grants non-admin permissions only. Admin status is managed
    /// exclusively via ProposedAction::GrantAdmin.
    GrantPermission { peer_id: EndpointId, permission: Permission },
    RevokePermission { peer_id: EndpointId, permission: Permission },

    // -- Server structure --
    CreateChannel { name: String, channel_id: String, kind: String },
    DeleteChannel { channel_id: String },
    RenameChannel { channel_id: String, new_name: String },
    CreateRole { name: String, role_id: String },
    DeleteRole { role_id: String },
    SetPermission { role_id: String, permission: String, granted: bool },
    AssignRole { peer_id: EndpointId, role_id: String },

    // -- Chat --
    Message { channel_id: String, body: String, reply_to: Option<EventHash> },
    EditMessage { message_id: EventHash, new_body: String },
    DeleteMessage { message_id: EventHash },
    Reaction { message_id: EventHash, emoji: String },

    // -- Identity --
    SetProfile { display_name: String },

    // -- Encryption --
    RotateChannelKey { channel_id: String, encrypted_keys: Vec<(EndpointId, Vec<u8>)> },

    // -- Pinning --
    PinMessage { channel_id: String, message_id: EventHash },
    UnpinMessage { channel_id: String, message_id: EventHash },

    // -- Server metadata (any admin) --
    RenameServer { new_name: String },
    SetServerDescription { description: String },
}

/// Actions that require admin vote to take effect. This enum
/// defines EXACTLY which actions must go through the vote path.
/// These actions cannot be triggered any other way — the data
/// model makes direct execution structurally impossible.
pub enum ProposedAction {
    GrantAdmin { peer_id: EndpointId },
    RevokeAdmin { peer_id: EndpointId },
    KickMember { peer_id: EndpointId },
    SetVoteThreshold { threshold: VoteThreshold },
}

/// Permission types that can be granted directly by any admin.
/// Does NOT include admin status — that is managed exclusively
/// through ProposedAction and the vote path.
pub enum Permission {
    SyncProvider,
    ManageChannels,
    ManageRoles,
    SendMessages,
    CreateInvite,
    // Administrator is NOT here — admin status is in ServerState.admins
}
```

**Changes from current EventKind:**

- **Removed**: `StateVerification` (legacy), `KickMember` (now a
  `ProposedAction`), `Resolve` (replaced by auto-apply on threshold)
- **Added**: `CreateServer`, `Propose`, `Vote`
- **Kept as direct**: `GrantPermission` / `RevokePermission` — these
  grant non-admin permissions only. The `Permission` enum does not
  contain `Administrator`, making it structurally impossible to grant
  admin status via direct event. Admin changes go through
  `ProposedAction::GrantAdmin` / `RevokeAdmin` exclusively.

Note: `message_id` fields change from `String` to `EventHash`.
Total variant count: 22 (21 original - `StateVerification` -
`KickMember` + `CreateServer` + `Propose` + `Vote` = 22).

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
///
/// The genesis author and server_id are derived from the genesis
/// event — no external parameters needed.
pub fn materialize(dag: &EventDag) -> ServerState {
    let genesis = dag.genesis().expect("DAG must have a genesis event");
    let server_id = genesis.hash.to_string();
    let genesis_author = genesis.author;
    let name = match &genesis.kind {
        EventKind::CreateServer { name } => name.clone(),
        _ => panic!("genesis event must be CreateServer"),
    };

    let sorted = dag.topological_sort();
    let mut state = ServerState::new(&server_id, &name, genesis_author);
    for event in sorted {
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
        // Kahn's algorithm with BTreeSet for deterministic tie-breaking.
        let mut in_degree: HashMap<&EventHash, usize> = HashMap::new();
        let mut dependents: HashMap<&EventHash, Vec<&EventHash>> = HashMap::new();

        // Initialize all nodes with in-degree 0.
        for hash in self.events.keys() {
            in_degree.insert(hash, 0);
        }

        // Compute in-degrees from prev + deps edges.
        // Only count edges to events that exist in this DAG
        // (soft-accept means deps may reference absent events).
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
        let mut result: Vec<&Event> = Vec::new();
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
/// the DAG guarantees ordering and dedup. Permission checks and
/// governance logic are enforced here.
fn apply_unchecked(state: &mut ServerState, event: &Event) -> ApplyResult {
    match &event.kind {
        // Governance events — handled specially.
        EventKind::CreateServer { .. } => {
            // No-op during replay — genesis data already extracted
            // by materialize() before the replay loop.
            return ApplyResult::Applied;
        }
        EventKind::Propose { action } => {
            // Only admins can propose.
            if !state.is_admin(&event.author) {
                return ApplyResult::Rejected("not an admin".into());
            }
            state.pending_proposals.insert(event.hash.clone(), PendingProposal {
                action: action.clone(),
                proposer: event.author,
                votes: HashMap::from([(event.author, true)]),  // proposer is implicit yes
            });
            // Check if threshold is already met (sole admin case).
            check_and_apply_proposal(state, &event.hash.clone());
            return ApplyResult::Applied;
        }
        EventKind::Vote { proposal, accept } => {
            if !state.is_admin(&event.author) {
                return ApplyResult::Rejected("not an admin".into());
            }
            if let Some(prop) = state.pending_proposals.get_mut(proposal) {
                prop.votes.insert(event.author, *accept);
            }
            // Check if threshold is now met.
            check_and_apply_proposal(state, proposal);
            return ApplyResult::Applied;
        }
        _ => {}
    }

    // Non-governance events — standard permission check.
    let required = required_permission(&event.kind);
    if let Some(ref perm) = required {
        if !state.has_permission(&event.author, perm) {
            return ApplyResult::Rejected(format!(
                "author '{}' lacks {:?} permission", event.author, perm
            ));
        }
    }

    apply_mutation(state, event)
}

/// Check if a pending proposal has met the vote threshold.
/// If so, remove from pending and apply the action.
fn check_and_apply_proposal(state: &mut ServerState, proposal: &EventHash) {
    let should_apply = state.pending_proposals.get(proposal)
        .map(|prop| {
            let yes_count = prop.votes.values().filter(|v| **v).count();
            state.meets_threshold(yes_count)
        })
        .unwrap_or(false);

    if should_apply {
        let prop = state.pending_proposals.remove(proposal).unwrap();
        apply_proposed_action(state, &prop.action);
    }
}

/// Apply a voted-on action to state.
fn apply_proposed_action(state: &mut ServerState, action: &ProposedAction) {
    match action {
        ProposedAction::GrantAdmin { peer_id } => {
            state.admins.insert(*peer_id);
            state.members.entry(*peer_id).or_insert_with(|| Member {
                peer_id: *peer_id,
                roles: HashSet::new(),
                display_name: None,
            });
        }
        ProposedAction::RevokeAdmin { peer_id } => {
            state.admins.remove(peer_id);
        }
        ProposedAction::KickMember { peer_id } => {
            state.members.remove(peer_id);
            state.peer_permissions.remove(peer_id);
            state.admins.remove(peer_id);
        }
        ProposedAction::SetVoteThreshold { threshold } => {
            state.vote_threshold = threshold.clone();
        }
    }
}
```

### ChatMessage Type Changes

`ChatMessage` in `types.rs` changes two fields to reflect
content-addressed message identity:

- `id: String` → `id: EventHash` (message ID is the hash of the
  `Message` event that created it)
- `reply_to: Option<String>` → `reply_to: Option<EventHash>`

All other types (`Channel`, `Role`, `Member`, `Profile`, `Permission`)
are unchanged. `Channel.id` and `Role.id` remain `String` (user-
provided UUIDs, not content hashes).

### ServerState Changes

`ServerState` gains governance state and loses the single-owner concept.
`new()` takes `(server_id, name, genesis_author)` — the genesis author
is added to the `admins` set. From there, the admin set evolves through
votes.

```rust
pub struct ServerState {
    pub server_id: String,
    pub server_name: String,
    pub channels: HashMap<String, Channel>,
    pub roles: HashMap<String, Role>,
    pub members: HashMap<EndpointId, Member>,
    /// Non-admin permissions (ManageChannels, SendMessages, etc.).
    /// Does not control admin status — that's in `admins`.
    pub peer_permissions: HashMap<EndpointId, HashSet<Permission>>,
    pub messages: Vec<ChatMessage>,
    pub profiles: HashMap<EndpointId, Profile>,
    pub description: String,
    pub channel_keys: HashMap<String, Vec<u8>>,

    // -- Governance state --
    /// The set of peers with admin status. Separate from Permission
    /// enum to make the governance boundary structurally enforced.
    pub admins: HashSet<EndpointId>,
    /// Current vote threshold for admin actions.
    pub vote_threshold: VoteThreshold,
    /// Pending proposals awaiting votes.
    pub pending_proposals: HashMap<EventHash, PendingProposal>,

    // REMOVED: owner: EndpointId
    //   → No single owner. Admin set managed by votes.
    //
    // REMOVED: seen_event_ids: HashSet<String>
    //   → Dedup is structural (DAG rejects duplicate hashes)
    //
    // REMOVED: hash() method that serializes entire state
    //   → State hashing is optional and computed differently
    //     (see Section 5: Compaction & Snapshots)
}

/// A proposal awaiting votes.
pub struct PendingProposal {
    pub action: ProposedAction,
    pub proposer: EndpointId,
    pub votes: HashMap<EndpointId, bool>,  // voter -> accept/reject
}
```

Key methods:

```rust
impl ServerState {
    /// Check if a peer is an admin.
    pub fn is_admin(&self, peer_id: &EndpointId) -> bool {
        self.admins.contains(peer_id)
    }

    /// Check if a peer has a specific non-admin permission.
    /// Admins implicitly have all permissions.
    pub fn has_permission(&self, peer_id: &EndpointId, perm: &Permission) -> bool {
        if self.admins.contains(peer_id) {
            return true;
        }
        self.peer_permissions
            .get(peer_id)
            .map(|perms| perms.contains(perm))
            .unwrap_or(false)
    }

    /// Check if a yes-vote count meets the current threshold.
    pub fn meets_threshold(&self, yes_count: usize) -> bool {
        let admin_count = self.admins.len();
        if admin_count == 0 { return false; }
        match self.vote_threshold {
            VoteThreshold::Unanimous => yes_count >= admin_count,
            VoteThreshold::Majority => yes_count > admin_count / 2,
            VoteThreshold::Count(n) => yes_count >= (n as usize).min(admin_count),
        }
    }
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
| `Reaction` by two peers | Both reactions are added (additive, no conflict). |
| Two concurrent `Propose` events | Both recorded as pending. Each passes independently when threshold is met. |
| `Vote` on a proposal that already passed | Vote is ignored (proposal no longer pending). |

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
     their chain (see Section 4).

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

There are two kinds of gaps, handled differently:

**Per-author chain gap** (`prev` references an unknown event): This is
a hard gap — the event cannot be inserted because the author's chain
must be contiguous. The event is buffered until the missing predecessor
arrives.

**Cross-author dep gap** (`deps` references an unknown event): This is
a soft gap — the event is accepted and inserted immediately. The
missing dep is recorded for background fetching. This keeps the system
responsive: a missing dep from a peer who is offline or has revised
their chain does not block events from other authors.

```rust
pub struct PendingBuffer {
    /// Events waiting for missing prev (per-author chain gap).
    /// Key: the missing EventHash they're waiting on.
    waiting_on_prev: HashMap<EventHash, Vec<Event>>,

    /// Deps we've seen referenced but don't have yet.
    /// Background task fetches these lazily.
    missing_deps: HashSet<EventHash>,
}

impl PendingBuffer {
    /// Call when a new event is inserted into the DAG.
    /// Returns any buffered events whose prev is now satisfied.
    pub fn resolve(&mut self, inserted: &EventHash) -> Vec<Event> {
        self.missing_deps.remove(inserted);
        self.waiting_on_prev.remove(inserted).unwrap_or_default()
    }

    /// Record a dep that we don't have yet.
    pub fn record_missing_dep(&mut self, hash: EventHash) {
        self.missing_deps.insert(hash);
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

The author is sovereign over their chain. There is no arbitration
rule — whichever version of the chain was signed by the author is
accepted as truth from that author.

If two peers have different versions of an author's chain (e.g., the
author sent revision V2 to some peers but not others during a
partition), convergence happens naturally: the author is the only
entity that can produce a validly-signed chain, so they will
eventually publish one definitive version and it will propagate. In
the interim, peers may have different views of that author's
contributions — this is acceptable per the archival tolerance goal.

There is no need for tie-breaking rules. The author's key is the
authority. If you receive a new chain signed by the author, you
accept it. If you receive two conflicting chains both signed by the
author, you accept whichever you received most recently and it will
converge as the author continues publishing events (which will
extend one chain and not the other).

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

1. **Admin-signed snapshots**: an admin can sign snapshots,
   providing a trust endorsement.
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

## Section 6: Materialized View — Client Consumption Map

The `ServerState` materialized view is consumed by multiple layers.
Understanding these access patterns is critical — the DAG redesign
must produce a `ServerState` that satisfies all of them unchanged.

### View Architecture (Current)

```
EventDag (new source of truth)
     │
     │  materialize() / apply_incremental()
     ▼
ServerState  ◄── held in StateActor<ServerState>
     │
     │  DerivedActor subscriptions (reactive)
     ├──────────► MessagesView    (messages + profiles + channels)
     ├──────────► ChannelsView    (channels)
     ├──────────► MembersView     (members + profiles + online peers)
     ├──────────► RolesView       (roles)
     ├──────────► UnreadView      (server registry)
     └──────────► ConnectionView  (network meta)
                       │
                       ▼
                  ClientView  (terminal composite)
                       │
              ┌────────┼────────┐
              ▼        ▼        ▼
           Bevy UI   Leptos   Accessors
```

### Field-by-Field Consumption

Every `ServerState` field and where it is read:

| Field | Consumers | Access Pattern |
|---|---|---|
| `server_name` | `ClientView.server_name`, join page, app title | Read on server join, display |
| `admins` | Admin badge in member list, permission checks, governance UI | Frequent read |
| `channels` | `compute_channels_view()`, channel name→ID resolution for message sending, message filtering | Hot path — every message send resolves channel |
| `roles` | `compute_roles_view()`, role management UI | Infrequent read |
| `members` | `compute_members_view()`, member list, online status merge | Read on membership change |
| `peer_permissions` | `has_permission()` checks before privileged ops (kick, role grant, channel create) | Read before every privileged mutation |
| `messages` | `compute_messages_view()` — filter by channel, sort by timestamp, resolve author names, compute reactions, reply previews | **Hottest path** — read on every new message |
| `profiles` | `resolve_display_name()` — called per-message per-member | Hot path — N times per messages view compute |
| `description` | Settings panel, `server_description()` accessor | Infrequent read |
| `channel_keys` | Join flow — extract encrypted keys for channel decryption | Read on server join |

### Mutation Entry Points

All writes to `ServerState` flow through exactly two paths:

1. **Local mutations** (`crates/client/src/mutations.rs`):
   - User action → build `EventKind` → build `Event` → `apply_lenient()` on `StateActor<ServerState>`
   - Covers: send message, create channel, grant permission, kick, etc.
   - All 16 active EventKind variants are constructed here.

2. **Remote events** (`crates/client/src/listeners.rs`):
   - Gossip received → verify → `apply_lenient()` on `StateActor<ServerState>`
   - Same apply path, different origin.

Both paths will change from `apply_lenient(state, event)` to
`dag.insert(event)` + `apply_incremental(state, event)`.

### Persistence

- **Full state snapshots**: `storage::save_server_state()` serializes
  entire `ServerState` via serde. Called after events are applied.
- **Event store**: `PersistentEventStore` (SQLite/LocalStorage) stores
  events + tracks `latest_hash`. This becomes the `EventDag` store.
- **Replay node** (`crates/replay/src/role.rs`): holds `ServerState`
  in memory, applies events via `apply_lenient()`, serves state hashes
  and event diffs to syncing peers.

### What This Means for the Redesign

The materialized `ServerState` struct itself is almost untouched — the
same fields serve the same views. The changes are all upstream of it:

- **Source of truth** shifts from "linear event chain + state" to
  "DAG + materialized state"
- **Mutation path** shifts from `apply(state, event)` to
  `dag.insert(event)` + `apply_incremental(state, event)`
- **Sync path** shifts from HLC-based SyncRequest to per-author heads
- **Persistence** shifts from event store + state snapshots to
  DAG store + state snapshots
- **Dedup** moves from `seen_event_ids` in state to structural
  uniqueness in the DAG

## Section 7: Complete Replacement

This is a clean break, not an incremental migration. The old linear
chain model (`Event`, `apply()`, `apply_lenient()`, `merge()`,
`StateHash`, `seen_event_ids`) is removed entirely and replaced with
the per-author Merkle-DAG.

### Rationale

An incremental migration would require maintaining two code paths,
dual-mode servers, and compatibility shims — more total work than a
clean replacement, with worse architecture. The old and new models are
fundamentally different data structures. There is no meaningful shared
code path between "single linear chain with parent state hash" and
"per-author DAG with content-addressed events."

### What Is Deleted

| Module / Symbol | Reason |
|---|---|
| `Event` (old struct) | Replaced by new `Event` with `prev`, `deps`, `seq`, `sig` |
| `apply()` | Parent-hash check is structurally unnecessary in DAG model |
| `apply_lenient()` | Replaced by `apply_unchecked()` (DAG guarantees ordering) |
| `merge()` | DAG union + topological sort replaces timestamp-sorted merge |
| `find_common_ancestor()` | Per-author seq comparison replaces state-hash walking |
| `StateHash` | No per-event state hash; snapshots use `SnapshotHash` |
| `ServerState.owner` | No single owner. Admin set managed by votes via `admins: HashSet` |
| `Permission::Administrator` | Admin status separated from Permission enum into `ServerState.admins` |
| `ServerState.seen_event_ids` | Dedup is structural (DAG rejects duplicate hashes) |
| `ServerState.hash()` | Full-state hashing removed from hot path |
| `InMemoryStore` | Replaced by `EventDag` (which subsumes store + ordering) |
| `EventStore` trait | Replaced by `DagStore` trait for persistent DAG backends |
| `KickMember` EventKind | Now a `ProposedAction`, applied via vote |

### What Is Preserved

| Symbol | Status |
|---|---|
| `EventKind` (22 variants) | 19 carried over + `CreateServer` + `Propose` + `Vote` |
| `ServerState` (struct) | Kept, minus `owner`/`seen_event_ids`/`hash()`, plus `admins`/`vote_threshold`/`pending_proposals` |
| `Permission` enum | `Administrator` removed — admin status tracked separately in `ServerState.admins` |
| `has_permission()` | Admins have all permissions; no owner concept |
| `Channel`, `Role`, `Member`, `Profile` | Unchanged |
| `ChatMessage` | `id` and `reply_to` change from `String` to `EventHash` |
| Permission enforcement in apply | Moved to `apply_unchecked()`, governance events handled specially |
| Zero-I/O crate boundary | Preserved — state crate remains pure |

### New Modules

| Module | Contents |
|---|---|
| `hash.rs` | `EventHash` (32-byte SHA-256 wrapper, `Ord`, `Display`) |
| `event.rs` | `Event`, `EventKind`, `ProposedAction`, `VoteThreshold` |
| `dag.rs` | `EventDag`, `InsertError`, `ChainStatus`, `RevisionError`, topological sort |
| `materialize.rs` | `materialize()`, `apply_unchecked()`, `apply_incremental()` |
| `sync.rs` | `HeadsSummary`, `AuthorHead`, `SyncMessage`, `AuthorRequest`, `PendingBuffer` |
| `snapshot.rs` | `Snapshot`, `SnapshotHash`, compaction |
| `types.rs` | Unchanged — `Channel`, `Role`, `Member`, etc. |
| `server.rs` | `ServerState` (simplified — no `hash()`, no `seen_event_ids`) |
| `tests.rs` | Rewritten from scratch for DAG model |

### New Public API Surface

```rust
// Core types
pub use event::{Event, EventKind, ProposedAction, VoteThreshold};
pub use hash::EventHash;
pub use dag::{EventDag, InsertError, ChainStatus, RevisionError};
pub use materialize::{materialize, apply_incremental, ApplyResult};
pub use sync::{HeadsSummary, AuthorHead, SyncMessage, AuthorRequest, PendingBuffer};
pub use server::{ServerState, PendingProposal};
pub use types::{Channel, ChatMessage, Member, Permission, Profile, Role};
// Deferred: pub use snapshot::{Snapshot, SnapshotHash};
```

### Client-Side Changes

The client crate (`willow-client`) changes at the mutation and
listener boundaries. View computation is unaffected.

**mutations.rs** — before:
```rust
let event = willow_state::Event {
    id: uuid(),
    parent_hash: state.hash(),
    author: my_id,
    timestamp_ms: now(),
    kind: EventKind::Message { ... },
};
willow_state::apply_lenient(&mut state, &event);
broadcast(event);
```

**mutations.rs** — after:
```rust
let event = dag.create_event(
    &my_identity,   // signs automatically
    EventKind::Message { ... },
);
dag.insert(event.clone())?;
apply_incremental(&mut state, &event);
broadcast(event);
```

**listeners.rs** — before:
```rust
let event = receive_from_gossip();
willow_state::apply_lenient(&mut state, &event);
```

**listeners.rs** — after:
```rust
let event = receive_from_gossip();
match dag.insert(event.clone()) {
    Ok(()) => {
        apply_incremental(&mut state, &event);
        // Resolve any events that were waiting on this one's prev.
        for ready in pending.resolve(&event.hash) {
            if let Ok(()) = dag.insert(ready.clone()) {
                apply_incremental(&mut state, &ready);
            }
        }
    }
    Err(InsertError::SeqGap { .. }) | Err(InsertError::PrevMismatch { .. }) => {
        // Per-author chain gap — buffer until predecessor arrives.
        pending.waiting_on_prev.entry(event.prev.clone())
            .or_default().push(event);
    }
    Err(InsertError::Duplicate) => { /* already have it */ }
    Err(e) => log::warn!("rejected event: {e:?}"),
}
```

**storage** — the `EventStore` trait is replaced by a `DagStore`
trait that persists the per-author chains:

```rust
pub trait DagStore {
    /// Append an event to an author's chain.
    fn append(&mut self, event: &Event);

    /// Get an author's events after a given seq number.
    fn author_events_since(&self, author: &EndpointId, after_seq: u64) -> Vec<Event>;

    /// Get the latest seq for an author.
    fn latest_seq(&self, author: &EndpointId) -> u64;

    /// Get all known author heads.
    fn heads(&self) -> HeadsSummary;

    /// Replace an author's chain (for revisions).
    fn replace_chain(&mut self, author: &EndpointId, chain: &[Event]);

    /// Load the full DAG into memory.
    fn load_dag(&self) -> EventDag;

    /// Save/load snapshots.
    fn save_snapshot(&mut self, snapshot: &Snapshot);
    fn load_latest_snapshot(&self) -> Option<Snapshot>;
}
```

### Sync-Layer Changes

The wire protocol changes from:

```rust
// Old
enum SyncMessage {
    Op(StampedOp),
    SyncRequest { latest_hlc: u64 },
    SyncBatch { ops: Vec<StampedOp> },
}
```

To the new per-author protocol defined in Section 3:

```rust
// New
enum SyncMessage {
    Advertise(HeadsSummary),
    Request(Vec<AuthorRequest>),
    Response(Vec<Event>),
}
```

The gossip topic for real-time event broadcast remains the same —
only the event format changes. The sync protocol uses a separate
topic or direct connection for catch-up.

### Test Rewrite

All 85 existing state tests are rewritten against the new API. The
test structure maps directly:

| Old test pattern | New test pattern |
|---|---|
| `let mut state = ServerState::new(...)` | `let mut dag = EventDag::new(); let mut state = ServerState::new(...)` |
| `let event = Event { id, parent_hash, ... }` | `let event = dag.create_event(&identity, kind)` |
| `apply(&mut state, &event)` | `dag.insert(event)?; apply_incremental(&mut state, &event)` |
| `merge(our, their, common)` | DAG union via `dag.insert()` for each event + `materialize()` |
| `state.hash() == other.hash()` | `materialize(&dag_a) == materialize(&dag_b)` |

The determinism, idempotency, permission, and stress tests all carry
over — same properties, different API.

### Summary of Changes

| Component | Before | After |
|---|---|---|
| `Event.id` | UUID string | Content hash (`EventHash`) |
| `Event.parent_hash` | `StateHash` (hash of entire state) | `prev: EventHash` + `deps: Vec<EventHash>` |
| Author validation | Checked at apply time | Structural (Ed25519 `sig` in event) |
| Deduplication | `seen_event_ids: HashSet` | DAG rejects duplicate hashes |
| Merge | Sort by timestamp, replay | DAG union + topological sort, hash tiebreak |
| Sync | HLC-based `SyncRequest` | Per-author `HeadsSummary` + seq-based requests |
| State hashing | Full serialization + SHA-256 on every `apply()` | None on hot path; optional via snapshots |
| History revision | Not possible | Author republishes their chain |
| Event store | Flat append-only log | Per-author chains in `DagStore` |
| Materialized view | `ServerState` mutated directly | `ServerState` projected from DAG |

## Section 8: Replay Workers

The replay node (`crates/replay/src/role.rs`) is the always-online
peer that buffers recent events and serves catch-up data to peers that
reconnect after being offline. Its role changes substantially.

### Current Design

```
ReplayRole {
    servers: HashMap<String, ServerData>,
}

ServerData {
    state: ServerState,              // full materialized state
    events: VecDeque<Event>,         // bounded FIFO (max 1000)
}
```

The replay node:
1. Receives events via `on_event()` → `apply_lenient()` to state
2. Buffers events in a bounded VecDeque (oldest evicted first)
3. On `WorkerRequest::Sync { state_hash }`:
   - Walks the buffer looking for an event whose `parent_hash` matches
   - If found, returns events from that point (delta sync)
   - If not found (peer too far behind), returns full `ServerState` snapshot

**Problems with current replay design:**
- Sync is keyed on `StateHash` — finding the divergence point requires
  linear scan of the buffer matching `parent_hash` fields
- Evicting oldest events from a linear buffer destroys the ability to
  serve those events to any peer that needs them
- Full state snapshot is O(entire state) when a delta would suffice
- No per-author granularity — can't serve "just Alice's events"

### New Design

```rust
struct ServerData {
    /// Per-author DAG (same EventDag from willow-state).
    dag: EventDag,

    /// Materialized state (cached, recomputed on revision).
    state: ServerState,

    /// Per-author bounded event retention.
    /// Each author's chain is capped independently.
    max_events_per_author: usize,
}
```

The replay node becomes a DAG-aware sync peer:

1. **Receives events** via `on_event()` → `dag.insert()` +
   `apply_incremental()` to cached state.

2. **Per-author buffering.** Instead of a single FIFO, each author's
   chain is bounded independently. This prevents a chatty peer from
   evicting a quiet peer's entire history.

3. **Sync via heads comparison.** On receiving a `SyncMessage::Advertise`:
   - Compare the requester's heads against local heads
   - Compute per-author deltas (events with seq > their_seq)
   - Respond with exactly the missing events

4. **Snapshot fallback.** If a peer is so far behind that the replay
   node has compacted away their events (per-author chain was capped),
   fall back to sending a snapshot + post-snapshot events.

5. **Author revision handling.** When the replay node receives a
   revised chain from an author, it replaces that author's chain in
   the DAG and re-materializes state.

```rust
impl WorkerRole for ReplayRole {
    fn on_event(&mut self, event: &Event) {
        let data = self.servers.entry(server_id).or_insert_with(..);

        match data.dag.insert(event.clone()) {
            Ok(()) => {
                apply_incremental(&mut data.state, event);
                // Per-author compaction if chain exceeds limit.
                data.compact_author(&event.author);
                // Resolve any buffered events waiting on this prev.
                for ready in data.pending.resolve(&event.hash) {
                    let _ = data.dag.insert(ready);
                }
            }
            Err(InsertError::SeqGap { .. }) | Err(InsertError::PrevMismatch { .. }) => {
                data.pending.waiting_on_prev.entry(event.prev.clone())
                    .or_default().push(event.clone());
            }
            Err(InsertError::Duplicate) => { /* already have it */ }
            Err(e) => tracing::warn!("replay rejected event: {e:?}"),
        }
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::Sync(SyncMessage::Advertise(their_heads)) => {
                let data = self.servers.get(&server_id);
                let missing = data.dag.events_since(&their_heads);
                if missing.is_empty() {
                    WorkerResponse::SyncBatch { events: vec![] }
                } else {
                    WorkerResponse::SyncBatch {
                        events: missing.into_iter().cloned().collect(),
                    }
                }
            }
            // ... snapshot fallback, history, etc.
        }
    }
}
```

### WorkerRequest / WorkerResponse Changes

The wire protocol (`crates/common/src/worker_types.rs`) changes:

```rust
// Old
pub enum WorkerRequest {
    Sync { server_id: String, state_hash: StateHash },
    History { server_id: String, channel: String, before_timestamp: Option<u64>, limit: u32 },
}

// New
pub enum WorkerRequest {
    Sync {
        server_id: String,
        heads: HeadsSummary,
    },
    History {
        server_id: String,
        channel: String,
        before_seq: Option<AuthorSeq>,  // cursor is now author+seq, not timestamp
        limit: u32,
    },
}

/// Cursor for paginated history.
pub struct AuthorSeq {
    pub author: EndpointId,
    pub seq: u64,
}
```

```rust
// Old
pub enum WorkerResponse {
    SyncBatch { events: Vec<Event> },
    Snapshot { state: Box<ServerState> },
    HistoryPage { events: Vec<Event>, has_more: bool },
    Denied { reason: String },
}

// New
pub enum WorkerResponse {
    /// Per-author delta events for sync catch-up.
    SyncBatch { events: Vec<Event> },
    /// Full DAG snapshot for far-behind peers.
    Snapshot {
        snapshot: Box<Snapshot>,         // state + heads
        post_snapshot_events: Vec<Event>, // events after snapshot
    },
    /// Paginated history.
    HistoryPage { events: Vec<Event>, has_more: bool },
    Denied { reason: String },
}
```

### SyncActor Changes

The sync actor (`crates/worker/src/actors/sync.rs`) currently
broadcasts `WorkerRequest::Sync { state_hash }` periodically. This
changes to broadcasting `SyncMessage::Advertise(heads)`:

```rust
// Old: state_hashes() → Vec<(String, StateHash)>
// Broadcasts one Sync request per server with full state hash

// New: heads_summary() → Vec<(String, HeadsSummary)>
// Broadcasts one Advertise per server with per-author heads
```

The per-author heads are much more informative than a single state
hash — they tell the receiver exactly what's needed per-author without
requiring the receiver to guess or send everything.

## Section 9: History & Archival

History (paginated access to old events) and archival (long-term
storage) change significantly with the DAG model.

### Current History Model

The current `WorkerRequest::History` uses timestamp-based pagination:

```rust
History {
    server_id: String,
    channel: String,
    before_timestamp: Option<u64>,  // cursor
    limit: u32,
}
```

This has problems:
- Timestamps are unreliable in a P2P system (clock skew)
- Pagination cursor is fragile — if events are reordered, the cursor
  breaks
- No way to request "events by author X" or "events since I last
  synced"
- History and sync are separate protocols with different wire types

### New History Model

History uses the same DAG-based addressing as sync. The cursor is
an author+seq pair (or a set of them), and channels are filtered
at the application layer:

```rust
/// Request historical events from a storage node.
pub struct HistoryRequest {
    pub server_id: String,
    /// Filter: only return events in this channel.
    /// None = all channels.
    pub channel: Option<String>,
    /// Cursor: return events whose topological position is
    /// before this point. None = start from latest.
    pub before: Option<HeadsSummary>,
    /// Maximum events to return.
    pub limit: u32,
}
```

**Key changes:**
- Cursor is a `HeadsSummary` (set of author+seq heads) instead of a
  timestamp. This is stable across re-orderings.
- Channel filtering is an application-level concern — the storage node
  stores DAG events and filters by `EventKind::Message { channel_id }`
  when serving.
- The same `HeadsSummary` structure is used for both sync and history
  pagination — unified addressing model.

### Storage Node Role

Storage nodes are the archival tier. Unlike replay nodes (bounded
in-memory buffer), storage nodes persist the full DAG to disk:

```rust
struct StorageData {
    /// Persistent DAG store (SQLite-backed).
    store: Box<dyn DagStore>,

    /// Cached materialized state (optional — can recompute).
    state: Option<ServerState>,
}
```

Storage nodes:
1. Ingest all events into persistent `DagStore`
2. Never evict events (archival)
3. Serve `HistoryRequest` with paginated results
4. Serve `SyncMessage::Advertise` with full DAG coverage
5. Can provide full-replay verification for new peers that don't
   trust snapshots

### Archival and Author Revision

When an author revises their chain, the storage node faces a choice:

1. **Replace only** (default): store the latest chain version per
   author. Old events are deleted. This matches the design goal
   that "current state may diverge from historical snapshots."

2. **Retain history** (opt-in): store both old and new chain versions,
   tagged by revision number. This enables audit trails but is
   outside the scope of the state crate. The `DagStore` trait can
   support this:

```rust
pub trait DagStore {
    // ... existing methods ...

    /// Store a superseded chain version for archival.
    /// revision_id is a monotonic counter per author.
    fn archive_chain(
        &mut self,
        author: &EndpointId,
        revision_id: u64,
        chain: &[Event],
    );

    /// Retrieve archived chain versions for an author.
    fn archived_chains(
        &self,
        author: &EndpointId,
    ) -> Vec<(u64, Vec<Event>)>;
}
```

### History and Current State Divergence

A critical consequence of author revision: history served by a storage
node may not match the current materialized state. Example:

1. Alice sends message "hello" (event A3, seq=3)
2. Storage node archives A3
3. Alice revises her chain, removing A3 (her new head is seq=2)
4. Current materialized state has no "hello" message
5. Storage node's archive still has A3

This is explicitly acceptable per the design goals. The application
layer must decide how to present this — possible approaches:

- Show archived messages with a "revised" indicator
- Only show messages from current chains (drop archived)
- Let users toggle "show history including revisions"

The state crate does not prescribe a policy. It provides the data
structures; the UI layer decides presentation.

### Unified Sync and History

With the DAG model, sync and history are the same operation at
different scales:

| Operation | Scope | Served by |
|---|---|---|
| Real-time sync | Latest events (gossip push) | All peers |
| Catch-up sync | Events since last seen heads | Replay nodes |
| Recent history | Last N events in a channel | Replay nodes |
| Deep history | Full event archive | Storage nodes |
| Full verification | Complete DAG from genesis | Storage nodes |

All use the same `HeadsSummary` → `Vec<Event>` pattern. The
difference is just the depth of the query and which node serves it.

## Section 10: Implementation Scope

This section defines what is in scope for the initial implementation
and what is deferred.

### In Scope (Core Foundation)

| Section | What ships |
|---|---|
| Section 1 | `Event`, `EventHash`, `EventKind` (22 variants), `ProposedAction`, `VoteThreshold`, `EventDag`, `InsertError`, `PendingBuffer` |
| Section 2 | `materialize()`, `apply_unchecked()`, `apply_incremental()`, topological sort, `ServerState` (simplified) |
| Section 3 | `HeadsSummary`, `SyncMessage`, `AuthorRequest`, sync flow |
| Section 4 | `replace_chain()`, chain verification, revision detection (`ChainStatus`) |
| Section 7 | Full deletion of legacy types, new module layout, new public API |
| Section 10 | Test suite (below) |

### Deferred

| Section | What is deferred | Why |
|---|---|---|
| Section 5 | Snapshots, compaction, `SnapshotHash` | Optimization — system works without it, just grows |
| Section 8 | Replay worker changes | Depends on core; separate PR |
| Section 9 | History/archival, storage node, `DagStore` archival methods | Depends on core + replay |
| Section 6 | Client-side wiring (mutations.rs, listeners.rs, persistence) | Depends on core; separate PR |

The first implementation delivers a fully tested `willow-state` crate
with the new DAG model. Downstream crates (client, worker, relay)
adapt in follow-up work.

## Section 11: Test Specification

Tests are the primary deliverable alongside the implementation. Every
property of the system must be covered. Tests are organized by the
concept they verify, not by module.

### Test Helpers

```rust
/// Create an EventDag with a genesis event and initial admin.
fn test_dag() -> (EventDag, Identity) {
    let admin = Identity::generate();
    let mut dag = EventDag::new();
    let genesis = dag.create_event(
        &admin,
        EventKind::CreateServer { name: "Test Server".into() },
        vec![],
        0,
    );
    dag.insert(genesis).unwrap();
    (dag, admin)
}

/// Create and insert a signed event into the DAG.
fn emit(
    dag: &mut EventDag,
    identity: &Identity,
    kind: EventKind,
) -> Event {
    let event = dag.create_event(identity, kind);
    dag.insert(event.clone()).unwrap();
    event
}

/// Create N identities.
fn identities(n: usize) -> Vec<Identity> {
    (0..n).map(|_| Identity::generate()).collect()
}

/// Materialize state from a DAG.
fn state(dag: &EventDag) -> ServerState {
    materialize(dag)
}
```

### Event & EventHash Tests

```
test event_hash_is_deterministic
    Same (author, seq, prev, deps, kind, timestamp) → same hash.

test event_hash_changes_with_any_field
    Changing any single field produces a different hash.

test event_signature_verifies
    A signed event passes signature verification.

test event_signature_rejects_tampered
    Modifying any field after signing fails verification.

test event_signature_rejects_wrong_key
    Signing with key A, verifying with key B fails.
```

### EventDag Insert Tests

```
test insert_genesis_event
    Insert a CreateServer event as first event. Succeeds.
    DAG has one event. genesis() returns it. server_id() and
    genesis_author() return correct values.

test insert_rejects_non_genesis_first
    Insert a non-CreateServer event into empty DAG.
    Returns NotGenesis.

test insert_sequential_events
    Insert seq=1, then seq=2 with prev=hash(seq=1). Both succeed.
    Chain is [e1, e2]. Head is e2.

test insert_rejects_duplicate
    Insert same event twice. Second returns Duplicate.

test insert_rejects_invalid_signature
    Tamper with event after signing. Returns InvalidSignature.

test insert_rejects_seq_gap
    Insert seq=1, then seq=3. Returns SeqGap { expected: 2, got: 3 }.

test insert_rejects_prev_mismatch
    Insert seq=1, then seq=2 with wrong prev hash.
    Returns PrevMismatch.

test insert_accepts_unknown_deps
    Insert event with deps=[nonexistent_hash]. Succeeds.
    The unknown dep is recorded but does not block insertion.

test insert_multiple_authors
    Two authors each insert their own chains. Both succeed.
    DAG has two independent chains.

test insert_with_cross_author_deps
    Author A inserts A1. Author B inserts B1 with deps=[A1.hash].
    Both succeed. B1's dep is resolved.
```

### Topological Sort Tests

```
test sort_single_author_is_seq_order
    Author A: [A1, A2, A3]. Sort order is [A1, A2, A3].

test sort_independent_authors_by_hash
    A1 and B1 have no deps on each other.
    Sort order is determined by EventHash comparison.
    Deterministic across runs (same hashes → same order).

test sort_respects_cross_author_deps
    A1, B1(deps=[A1]). A1 always comes before B1.

test sort_complex_dag
    A1, A2(deps=[B1]), B1, B2(deps=[A1]), C1(deps=[B1]).
    Verify all causal constraints are respected.
    Verify concurrent events are tiebroken by hash.

test sort_is_deterministic
    Same DAG contents → same sort order, every time.

test sort_is_stable_under_insertion_order
    Insert events in different orders. Same DAG → same sort.
```

### Materialization Tests

```
test materialize_empty_dag
    Just genesis → fresh ServerState with genesis author as sole admin.

test materialize_create_channel
    One CreateChannel event → state has one channel.

test materialize_is_deterministic
    Same DAG → same ServerState, every time.

test materialize_two_dags_same_events_same_state
    Build two DAGs with the same events (different insertion order).
    materialize() produces identical states.

test materialize_concurrent_channel_creates
    Two authors concurrently create channels. Both appear in state.

test materialize_permission_enforcement
    Unpermitted author tries CreateChannel. Event is rejected
    (ApplyResult::Rejected). Channel does not appear in state.

test materialize_genesis_author_is_admin
    Genesis author is in admins set and can do anything.

test materialize_admin_has_all_permissions
    Admin peer (in admins set) can do anything without explicit
    permission grants.

test materialize_kick_via_vote
    KickMember via Propose + Vote removes the member and permissions.

test materialize_kick_admin_via_vote
    An admin can be kicked via the vote process like any member.

test materialize_message_in_channel
    Message event → ChatMessage appears in state.messages.

test materialize_edit_message
    Message then EditMessage → message body updated, edited=true.

test materialize_delete_message
    Message then DeleteMessage → message marked deleted.

test materialize_reaction
    Message then Reaction → reaction appears on message.

test materialize_set_profile
    SetProfile → profile and member display_name updated.

test materialize_rename_server_admin_only
    Admin can rename. Non-admin is rejected.

test materialize_server_description_admin_only
    Admin can set description. Non-admin is rejected.

test materialize_delete_channel_cascades_messages
    DeleteChannel removes the channel and all its messages.

test materialize_delete_role_cascades_members
    DeleteRole removes the role from all members.

test materialize_grant_permission_adds_member
    GrantPermission to unknown peer also adds them as a member.
```

### Governance Tests

```
test propose_requires_admin
    Non-admin tries to Propose. Rejected.

test vote_requires_admin
    Non-admin tries to Vote. Rejected.

test vote_auto_applies_on_threshold
    With 3 admins and unanimous threshold, 3rd vote auto-applies
    the action during materialization.

test vote_does_not_apply_below_threshold
    With 3 admins and unanimous, 2 yes votes → action not applied.
    Proposal remains pending.

test sole_admin_propose_auto_applies
    Sole admin proposes GrantAdmin. Proposer is implicit yes.
    Quorum = 1. Action auto-applies immediately on Propose.

test propose_grant_admin_full_flow
    Propose GrantAdmin, all admins vote yes, peer is added to
    admins set and members.

test propose_revoke_admin_full_flow
    Propose RevokeAdmin, all admins vote yes, peer is removed
    from admins set.

test propose_kick_member_full_flow
    Propose KickMember, all admins vote yes, peer is removed
    from members, peer_permissions, and admins.

test propose_set_vote_threshold
    Propose SetVoteThreshold(Majority), all admins vote yes,
    vote_threshold changes. Subsequent proposals need only majority.

test threshold_change_requires_current_threshold
    With unanimous threshold, changing to Majority requires
    unanimous vote. With Majority, changing back requires majority.

test vote_on_passed_proposal_ignored
    Proposal passes (action applied, removed from pending).
    Late vote on that proposal is a no-op.

test concurrent_proposals_apply_independently
    Two proposals pending at the same time. Each accumulates
    votes independently. Both can pass.

test grant_permission_cannot_grant_admin
    GrantPermission event can only carry Permission variants
    (SyncProvider, ManageChannels, etc.). Admin status is
    structurally separate — enforced by the type system.
```

### Incremental Apply Tests

```
test incremental_matches_full_materialize
    Apply events incrementally one by one. Compare result to
    full materialize(). States are identical.

test incremental_concurrent_events
    Two authors produce concurrent events. Apply in topo order.
    Result matches full materialize().
```

### Author Revision Tests

```
test replace_chain_basic
    Author A has [A1, A2, A3]. Replace with [A1, A2'].
    DAG now has A1 and A2'. A3 is gone.

test replace_chain_re_materializes_correctly
    After revision, materialize() reflects the new chain.

test replace_chain_rejects_invalid_signature
    Attempt to replace with a chain signed by a different key.
    Returns error.

test replace_chain_rejects_broken_prev
    Chain with inconsistent prev hashes is rejected.

test replace_chain_broken_dep_is_tolerated
    After revision, another author's event has deps pointing to
    a now-removed event. The dep is broken but the event still
    applies. Materialization succeeds.

test revision_detection
    Build DAG with author A at seq=3. Receive HeadsSummary with
    author A at seq=3 but different hash. Detected as Revised.
```

### Sync Protocol Tests

```
test heads_summary_reflects_dag
    Insert events for 3 authors. HeadsSummary has 3 entries
    with correct seq and hash for each.

test events_since_returns_delta
    DAG has authors A(seq=5), B(seq=3). Request with
    their_heads={A:3, B:3}. Returns A's events 4 and 5.

test events_since_unknown_author
    Request includes an author not in the DAG. That author
    is simply skipped (no error).

test events_since_new_author
    DAG has author C that the requester doesn't know about.
    Returns all of C's events.

test sync_round_trip
    Two DAGs with overlapping events. Exchange heads, compute
    deltas, apply. Both DAGs converge to the same state.
```

### PendingBuffer Tests

```
test buffer_and_resolve
    Buffer an event waiting on prev hash X. Insert an event
    with hash X. resolve() returns the buffered event.

test missing_deps_recorded
    Insert event with unknown dep. Missing dep is recorded
    in the buffer. Does not block the event.

test resolve_cascading
    Event C waits on B which waits on A. Insert A → resolves B.
    Insert B → resolves C. All three end up in the DAG.
```

### Stress Tests

```
test stress_1000_events_single_author
    One author inserts 1000 sequential events.
    Materialize produces correct state.

test stress_100_authors_10_events_each
    100 authors, each with 10 events with cross-author deps.
    Topological sort completes. Materialization is deterministic.

test stress_sort_performance
    10000 events across 50 authors. Topological sort completes
    in reasonable time (benchmark, not hard assertion).

test stress_concurrent_channel_creates
    50 authors all create a channel concurrently.
    Materialized state has 50 channels. Deterministic.
```

### Tests Removed from Current Suite

The following current test patterns do not apply to the new design
and are not carried over:

| Old test | Why removed |
|---|---|
| `parent_hash_mismatch` | No per-event state hash in new model |
| `apply_is_idempotent` (via `seen_event_ids`) | Dedup is structural in DAG (Duplicate error) |
| `full_replay_from_genesis` (via state hash comparison) | Replaced by `materialize_is_deterministic` |
| `merge_*` tests (timestamp-based merge) | No merge function — DAG union + topo sort replaces it |
| `state_hash_*` tests | `StateHash` removed |
| `state_verification_*` tests | `StateVerification` event kind removed |
| `event_store_*` tests (`InMemoryStore`) | `EventStore` replaced by `EventDag` |

The properties these tests covered (determinism, idempotency,
convergence, replay correctness) are all still tested — just
through the new API.

## Appendix: References

1. Sanjuán, Pöyhtäri, Teixeira. "Merkle-CRDTs: Merkle-DAGs meet CRDTs." arXiv:2004.00107, 2020.
2. Auvolat, Taïani. "Merkle Search Trees: Efficient State-Based CRDTs in Open Networks." SRDS, 2019.
3. Almeida, Shoker, Baquero. "Delta State Replicated Data Types." arXiv:1603.01529, 2016.
4. Bauwens, Gonzalez Boix. "From Causality to Stability: Understanding and Reducing Meta-Data in CRDTs." MPLR, 2020.
5. Secure Scuttlebutt Protocol. https://scuttlebot.io/more/protocols/secure-scuttlebutt.html
6. AT Protocol Repository Spec. https://atproto.com/specs/repository
7. Automerge Binary Document Format. https://automerge.org/automerge-binary-format-spec/
8. iroh by n0. https://github.com/n0-computer/iroh
