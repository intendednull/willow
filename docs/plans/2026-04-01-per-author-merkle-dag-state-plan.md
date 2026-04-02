# Per-Author Merkle-DAG State Machine — Implementation Plan

**Date**: 2026-04-01
**Spec**: `docs/specs/2026-04-01-per-author-merkle-dag-state-design.md`
**Scope**: Sections 1-4, 7, 11 (core foundation)

## Approach

Replace the entire `willow-state` crate contents in place. Same
Cargo.toml, same crate name, new internals. Downstream crates will
have compile errors — that's expected and handled in follow-up work.

All work happens in `crates/state/src/`. Every step ends with
`cargo test -p willow-state` passing.

**Delete old files first** (Step 1). Build up from clean slate.

## Dependency Changes

The `Cargo.toml` for willow-state needs:
- **Keep**: `bincode`, `serde`, `sha2`, `willow-identity`, `iroh-base`
- **Remove**: `uuid` (event IDs are content hashes now, not UUIDs)
- **No new deps**: `willow-identity` already re-exports `Signature`,
  `PublicKey`, `SecretKey` from `iroh-base` and provides
  `Identity::sign(&[u8]) -> Signature` and
  `verify(&PublicKey, &[u8], &Signature) -> bool`.

## Step 1: Delete old code, EventHash type

**Files**:
- Delete: `crates/state/src/merge.rs`, `crates/state/src/store.rs`
- Gut: `crates/state/src/lib.rs` (empty except module decls added as
  each step progresses)
- Rewrite: `crates/state/src/hash.rs`

Replace `StateHash` with `EventHash`. Pure hash wrapper — no
dependency on event types.

- `EventHash(pub [u8; 32])`
- `EventHash::ZERO` — used as `prev` for an author's first event
- `EventHash::from_bytes(data: &[u8]) -> Self` — SHA-256 of raw bytes
- `Ord` / `PartialOrd` — lexicographic byte comparison, needed for
  `BTreeSet` in topological sort tiebreaking
- `Display` — hex format
- `Serialize` / `Deserialize`
- `Default` → `ZERO`

**Tests** (in `hash.rs`):
- `zero_hash_is_all_zeros`
- `same_input_same_hash`
- `different_input_different_hash`
- `display_is_hex`
- `ord_is_lexicographic`

## Step 2: EventKind, ProposedAction, VoteThreshold, Event

**Files**: `crates/state/src/event.rs` (new)

### EventKind

22 variants total. Changes from current:
- **Removed**: `StateVerification` (legacy), `KickMember` (now a
  `ProposedAction`), `Resolve` (votes auto-apply on threshold)
- **Added**: `CreateServer`, `Propose`, `Vote`
- **Kept as direct**: `GrantPermission` / `RevokePermission` for
  non-admin permissions only (ManageChannels, SendMessages, etc.)

`Permission` enum loses `Administrator` — admin status is tracked
separately in `ServerState.admins: HashSet<EndpointId>`. This makes
it structurally impossible to grant admin via `GrantPermission`.

```rust
pub enum Permission {
    SyncProvider,
    ManageChannels,
    ManageRoles,
    SendMessages,
    CreateInvite,
}
```

Field type changes:
- `message_id: String` → `message_id: EventHash` in `EditMessage`,
  `DeleteMessage`, `Reaction`, `PinMessage`, `UnpinMessage`
- `reply_to: Option<String>` → `reply_to: Option<EventHash>` in
  `Message`

Keep `default_create_channel_kind()` serde helper for `CreateChannel`.

### ProposedAction and VoteThreshold

```rust
pub enum ProposedAction {
    GrantAdmin { peer_id: EndpointId },
    RevokeAdmin { peer_id: EndpointId },
    KickMember { peer_id: EndpointId },
    SetVoteThreshold { threshold: VoteThreshold },
}

pub enum VoteThreshold {
    Unanimous,
    Majority,
    Count(u32),
}
```

Both derive `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`.

### Event struct

```rust
pub struct Event {
    pub hash: EventHash,
    pub author: EndpointId,
    pub seq: u64,
    pub prev: EventHash,
    pub deps: Vec<EventHash>,
    pub kind: EventKind,
    pub sig: Signature,
    pub timestamp_hint_ms: u64,
}
```

`Event::new(identity, seq, prev, deps, kind, timestamp_hint_ms)`:
1. Serialize `(author, seq, prev, deps, kind, timestamp_hint_ms)` via
   bincode (canonical — same as EventHash computation)
2. `hash = EventHash::from_bytes(&serialized)`
3. `sig = identity.sign(&serialized)`
4. Return Event with all fields

`Event::verify(&self) -> bool`:
1. Re-serialize `(author, seq, prev, deps, kind, timestamp_hint_ms)`
2. `willow_identity::verify(&self.author.into(), &serialized, &self.sig)`

### ChatMessage.id type change

`ChatMessage.id` in `types.rs`: `String` → `EventHash`.
`ChatMessage.reply_to`: `Option<String>` → `Option<EventHash>`.

**Tests**:
- `event_hash_is_deterministic`
- `event_hash_changes_with_any_field`
- `event_signature_verifies`
- `event_signature_rejects_tampered`
- `event_signature_rejects_wrong_key`

## Step 3: EventDag — core structure and insertion

**Files**: `crates/state/src/dag.rs` (new)

### Struct

```rust
pub struct EventDag {
    events: HashMap<EventHash, Event>,
    chains: HashMap<EndpointId, Vec<EventHash>>,
    heads: HashMap<EndpointId, EventHash>,
    genesis_hash: Option<EventHash>,
}
```

### InsertError

```rust
pub enum InsertError {
    InvalidSignature,
    NotGenesis,
    SeqGap { author: EndpointId, expected: u64, got: u64 },
    PrevMismatch { author: EndpointId, expected: EventHash, got: EventHash },
    Duplicate,
}
```

### insert()

1. `event.verify()` → `InvalidSignature`
2. `events.contains_key(&event.hash)` → `Duplicate`
3. If DAG is empty (`genesis_hash.is_none()`): event must be
   `EventKind::CreateServer` with seq=1 and prev=ZERO. Set
   `genesis_hash`. Otherwise: reject non-CreateServer as first event
   → `NotGenesis`.
4. Check seq: must be `latest_seq(author) + 1` → `SeqGap`
5. Check prev: must match `head(author)` or `ZERO` for seq=1 → `PrevMismatch`
6. Insert into `events`, push hash to `chains[author]`, set `heads[author]`
7. Return `Ok(())`. Unknown deps in `event.deps` are silently accepted.

### Accessors

- `new() -> Self`
- `genesis(&self) -> Option<&Event>`
- `server_id(&self) -> Option<String>` (hex of genesis hash)
- `genesis_author(&self) -> Option<EndpointId>`
- `latest_seq(&self, author) -> u64` (0 if unknown)
- `head(&self, author) -> Option<&EventHash>`
- `author_events(&self, author) -> &[EventHash]` (empty slice if unknown)
- `get(&self, hash) -> Option<&Event>`
- `len(&self) -> usize` (total events)
- `is_empty(&self) -> bool`
- `authors(&self) -> impl Iterator<Item = &EndpointId>`

### create_event() convenience

```rust
pub fn create_event(
    &self,
    identity: &Identity,
    kind: EventKind,
    deps: Vec<EventHash>,
    timestamp_hint_ms: u64,
) -> Event
```

Reads current head/seq for `identity.endpoint_id()`, builds Event
with `seq + 1`, `prev = current_head` (or ZERO). Does NOT insert.

**Tests**:
- `insert_genesis_event`
- `insert_rejects_non_genesis_first`
- `insert_sequential_events`
- `insert_rejects_duplicate`
- `insert_rejects_invalid_signature`
- `insert_rejects_seq_gap`
- `insert_rejects_prev_mismatch`
- `insert_accepts_unknown_deps`
- `insert_multiple_authors`
- `insert_with_cross_author_deps`
- `genesis_accessors`

## Step 4: Topological sort

**Files**: `crates/state/src/dag.rs` (extend)

### topological_sort()

Kahn's algorithm:
1. Build in-degree map and adjacency (dependents) map from `prev` + `deps`
2. Only count edges to events that exist in the DAG (soft-accept)
3. Seed `BTreeSet<&EventHash>` with zero-indegree nodes
4. Pop smallest hash (lexicographic via `Ord`), emit, decrement dependents
5. Return `Vec<&Event>`

### causal_parents() helper

Returns `prev` (if not ZERO) + all `deps` that exist in the DAG.

### is_ancestor()

Walk backwards from B via causal_parents to find A. Memoize with a
visited set.

**Tests**:
- `sort_single_author_is_seq_order`
- `sort_independent_authors_by_hash`
- `sort_respects_cross_author_deps`
- `sort_complex_dag`
- `sort_is_deterministic`
- `sort_is_stable_under_insertion_order`

## Step 5: ServerState and types

**Files**: `crates/state/src/server.rs` (rewrite), `crates/state/src/types.rs` (modify)

### ServerState changes

- Remove `owner: EndpointId` — no single owner
- Remove `seen_event_ids: HashSet<String>`
- Remove `hash()` method
- Remove `is_trusted()` (legacy backward-compat bridge)
- Remove `Administrator` from `Permission` enum
- Add `admins: HashSet<EndpointId>` — admin set, separate from permissions
- Add `vote_threshold: VoteThreshold` (default: `Unanimous`)
- Add `pending_proposals: HashMap<EventHash, PendingProposal>`
- Add `is_admin(&self, peer_id) -> bool`
- Keep `has_permission()` — admins have all permissions implicitly
- Keep `is_sync_provider()`
- Add `meets_threshold(&self, yes_count: usize) -> bool`

### PendingProposal

```rust
pub struct PendingProposal {
    pub action: ProposedAction,
    pub proposer: EndpointId,
    pub votes: HashMap<EndpointId, bool>,
}
```

### ServerState.new() signature

```rust
pub fn new(
    id: impl Into<String>,
    name: impl Into<String>,
    genesis_author: EndpointId,
) -> Self
```

Genesis author is added as member and added to `admins` set. No
special "owner" field — they're just the first admin.

### Key methods

```rust
pub fn is_admin(&self, peer_id: &EndpointId) -> bool {
    self.admins.contains(peer_id)
}

pub fn has_permission(&self, peer_id: &EndpointId, perm: &Permission) -> bool {
    if self.admins.contains(peer_id) {
        return true;  // admins have all permissions implicitly
    }
    self.peer_permissions
        .get(peer_id)
        .map(|perms| perms.contains(perm))
        .unwrap_or(false)
}

pub fn meets_threshold(&self, yes_count: usize) -> bool {
    let admin_count = self.admins.len();
    if admin_count == 0 { return false; }
    match self.vote_threshold {
        VoteThreshold::Unanimous => yes_count >= admin_count,
        VoteThreshold::Majority => yes_count > admin_count / 2,
        VoteThreshold::Count(n) => yes_count >= (n as usize).min(admin_count),
    }
}
```

### types.rs changes

- `ChatMessage.id`: `String` → `EventHash`
- `ChatMessage.reply_to`: `Option<String>` → `Option<EventHash>`
- All other types unchanged: `Channel`, `Role`, `Member`, `Profile`,
  `Permission`

**Tests**:
- `new_server_has_genesis_author_as_admin`
- `admin_has_all_permissions`
- `peer_without_permissions`
- `meets_threshold_unanimous`
- `meets_threshold_majority`
- `meets_threshold_count`

## Step 6: Materialization and apply

**Files**: `crates/state/src/materialize.rs` (new)

### Public API

- `materialize(dag) -> ServerState` — full replay (genesis author +
  server_id derived from genesis event)
- `apply_incremental(state, event) -> ApplyResult` — single event
- `ApplyResult { Applied, Rejected(String) }`

### Internal

- `apply_unchecked(state, event) -> ApplyResult` — governance +
  permission check + mutation
- `apply_mutation(state, event) -> ApplyResult` — the match block
  for non-governance events
- `apply_proposed_action(state, action)` — applies a voted-on action
- `required_permission(kind) -> Option<Permission>`

### Governance handling in apply_unchecked

```
CreateServer → no-op (genesis data extracted by materialize)
Propose      → check is_admin, record in pending_proposals
               (proposer = implicit yes), check_and_apply_proposal
               (handles sole admin auto-apply)
Vote         → check is_admin, record vote, check_and_apply_proposal
               (auto-applies when threshold met)
```

No Resolve event — votes auto-apply during materialization.

### check_and_apply_proposal helper

After recording a Propose or Vote, check if the pending proposal's
yes-count meets `state.meets_threshold()`. If so, remove from pending
and call `apply_proposed_action(state, &prop.action)`.

### The mutation match block

Ported from current `apply_inner`. Changes:
- Remove `StateVerification` arm
- Remove `KickMember` arm (now in `apply_proposed_action`)
- Add `CreateServer` no-op arm
- Add `Propose` / `Vote` governance arms (no Resolve)
- `message_id` fields are `EventHash` not `String`
- `event.id` references become `event.hash`
- No `seen_event_ids` insertion
- `ChatMessage.id` is `event.hash.clone()`
- `RenameServer` / `SetServerDescription` require admin (via `is_admin`)
- Permission checks use `has_permission` (admins pass implicitly)
- `apply_proposed_action` modifies `state.admins` not `peer_permissions`

Total: 22 match arms in apply_unchecked (3 governance + 19 standard).

**Tests**:
- `materialize_empty_dag` — just genesis → fresh state with genesis
  author as sole admin
- `materialize_create_channel`
- `materialize_is_deterministic`
- `materialize_two_dags_same_events_same_state`
- `materialize_concurrent_channel_creates`
- `materialize_permission_enforcement`
- `materialize_genesis_author_is_admin`
- `materialize_admin_has_all_permissions`
- `materialize_kick_via_vote`
- `materialize_kick_admin_via_vote`
- `materialize_message_in_channel`
- `materialize_edit_message`
- `materialize_delete_message`
- `materialize_reaction`
- `materialize_set_profile`
- `materialize_rename_server_admin_only`
- `materialize_server_description_admin_only`
- `materialize_delete_channel_cascades_messages`
- `materialize_delete_role_cascades_members`
- `materialize_grant_permission_adds_member`
- `incremental_matches_full_materialize`
- `incremental_concurrent_events`
- **Governance tests**:
- `propose_requires_admin`
- `vote_requires_admin`
- `vote_auto_applies_on_threshold`
- `vote_does_not_apply_below_threshold`
- `sole_admin_propose_auto_applies`
- `propose_grant_admin_full_flow`
- `propose_revoke_admin_full_flow`
- `propose_kick_member_full_flow`
- `propose_set_vote_threshold`
- `threshold_change_requires_current_threshold`
- `vote_on_passed_proposal_ignored`
- `concurrent_proposals_apply_independently`
- `grant_permission_cannot_grant_admin` (structurally impossible via type system)

## Step 7: Sync types and PendingBuffer

**Files**: `crates/state/src/sync.rs` (new)

`HeadsSummary` is used by `EventDag::heads_summary()` and the sync
protocol. No circular dependency — `sync.rs` depends on `hash.rs`
for `EventHash`, and `dag.rs` depends on `sync.rs` for `HeadsSummary`.

### Types

```rust
pub struct HeadsSummary {
    pub heads: HashMap<EndpointId, AuthorHead>,
}

pub struct AuthorHead {
    pub seq: u64,
    pub hash: EventHash,
}

pub enum SyncMessage {
    Advertise(HeadsSummary),
    Request(Vec<AuthorRequest>),
    Response(Vec<Event>),
}

pub struct AuthorRequest {
    pub author: EndpointId,
    pub after_seq: u64,
}
```

All derive `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`.

### EventDag methods (in dag.rs)

- `heads_summary(&self) -> HeadsSummary`
- `events_since(&self, their_heads: &HashMap<EndpointId, u64>) -> Vec<&Event>`

### PendingBuffer

```rust
pub struct PendingBuffer {
    waiting_on_prev: HashMap<EventHash, Vec<Event>>,
    missing_deps: HashSet<EventHash>,
}
```

- `new() -> Self`
- `buffer_for_prev(&mut self, prev_hash: EventHash, event: Event)`
- `record_missing_dep(&mut self, hash: EventHash)`
- `resolve(&mut self, inserted_hash: &EventHash) -> Vec<Event>`
- `missing_dep_count(&self) -> usize`

**Tests**:
- `heads_summary_reflects_dag`
- `events_since_returns_delta`
- `events_since_unknown_author`
- `events_since_new_author`
- `sync_round_trip`
- `buffer_and_resolve`
- `missing_deps_recorded`
- `resolve_cascading`

## Step 8: Author revision

**Files**: `crates/state/src/dag.rs` (extend)

### ChainStatus and compare_chains

```rust
pub enum ChainStatus {
    Ahead { new_events: u64 },
    Behind { missing_events: u64 },
    Synced,
    Revised,
}

pub fn compare_chains(
    our_head: &AuthorHead,
    their_head: &AuthorHead,
    our_chain: &[EventHash],
) -> ChainStatus
```

### replace_chain

```rust
pub fn replace_chain(
    &mut self,
    author: &EndpointId,
    new_chain: Vec<Event>,
) -> Result<(), RevisionError>
```

1. `verify_chain(author, &new_chain)?` — check all sigs, seq
   monotonicity, prev consistency, all events have matching author
2. Cannot replace the genesis author's chain if it removes the
   `CreateServer` event (genesis event is immutable)
3. Remove old events for this author from `events` map
4. Insert new events
5. Update `chains[author]` and `heads[author]`

### RevisionError

```rust
pub enum RevisionError {
    InvalidSignature { seq: u64 },
    BrokenPrevChain { seq: u64 },
    WrongAuthor { seq: u64, expected: EndpointId, got: EndpointId },
    EmptyChain,
    SeqDoesNotStartAtOne,
    GenesisEventMissing,
}
```

**Tests**:
- `replace_chain_basic`
- `replace_chain_re_materializes_correctly`
- `replace_chain_rejects_invalid_signature`
- `replace_chain_rejects_broken_prev`
- `replace_chain_broken_dep_is_tolerated`
- `replace_chain_preserves_genesis`
- `revision_detection`

## Step 9: lib.rs and cleanup

**Files**: `crates/state/src/lib.rs` (rewrite)

New contents:

```rust
pub mod dag;
pub mod event;
pub mod hash;
pub mod materialize;
pub mod server;
pub mod sync;
pub mod types;

#[cfg(test)]
mod tests;

pub use event::{Event, EventKind, ProposedAction, VoteThreshold};
pub use hash::EventHash;
pub use dag::{ChainStatus, EventDag, InsertError, RevisionError};
pub use materialize::{apply_incremental, materialize, ApplyResult};
pub use server::{PendingProposal, ServerState};
pub use sync::{AuthorHead, AuthorRequest, HeadsSummary, PendingBuffer, SyncMessage};
pub use types::{Channel, ChatMessage, Member, Permission, Profile, Role};
```

Note: `HeadsSummary` is defined in `sync.rs` and re-exported.
`dag.rs` imports it for `heads_summary()` method.

**Validation**: `cargo test -p willow-state` — all tests pass.

## Step 10: Stress tests

**Files**: `crates/state/src/tests.rs`

- `stress_1000_events_single_author`
- `stress_100_authors_10_events_each`
- `stress_sort_performance`
- `stress_concurrent_channel_creates`
- `stress_governance_many_proposals`

## Step 11: WASM check and clippy

Run:
- `cargo clippy -p willow-state -- -D warnings`
- `just check-wasm` (or equivalent wasm32 target check)

Fix any issues. Should be clean — no platform-specific code added.

## Order of Operations

```
Step 1  → Delete old, EventHash        (clean slate, hash primitive)
Step 2  → EventKind + Event            (core types, governance types, signing)
Step 3  → EventDag insert             (DAG structure, validation)
Step 4  → Topological sort            (ordering algorithm)
Step 5  → ServerState + types         (governance state, threshold, pending proposals)
Step 6  → Materialization + apply     (projection, governance handling)
Step 7  → Sync types + PendingBuffer  (protocol types, HeadsSummary)
Step 8  → Author revision             (chain replacement)
Step 9  → lib.rs + cleanup            (public API)
Step 10 → Stress tests                (scale validation)
Step 11 → WASM + clippy               (platform compat, lint)
```

**Module dependencies**: `hash.rs` has no deps. `event.rs` depends
on `hash.rs`. `dag.rs` depends on `event.rs` and `sync.rs` (for
`HeadsSummary`). `materialize.rs` depends on `event.rs`, `dag.rs`,
`server.rs`. `sync.rs` depends on `hash.rs` and `event.rs`. No
circular deps. `sync.rs` structs (`HeadsSummary`, `AuthorHead`) can
be created early (Step 7 types, but file exists from then on).

## What Breaks Downstream

| Crate | What breaks | Fix (follow-up) |
|---|---|---|
| `willow-client` | `apply_lenient()`, old `Event`, `EventStore`, `StateHash`, `ServerState.owner`, `ChatMessage.id` type | Adapt mutations.rs, listeners.rs, views.rs per spec Section 7 |
| `willow-common` | `WorkerRequest::Sync { state_hash }`, `WorkerResponse`, `Event` type | Update wire types per spec Section 8 |
| `willow-worker` | `WorkerRole::on_event(&Event)`, state hash methods | Update to new Event type |
| `willow-replay` | `apply_lenient()`, `StateHash`, VecDeque buffer | Rewrite per spec Section 8 |
| `willow-app` | Old `Event` in test helpers, `StateHash`, `merge()`, `ServerState.owner` | Update test helpers and e2e tests |
| `willow-web` | Indirect via client | Follows client fixes |

These are expected and intentional. The state crate is the foundation;
downstream adapts to it.
