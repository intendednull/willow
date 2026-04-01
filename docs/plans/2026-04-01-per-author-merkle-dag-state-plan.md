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

## Dependency Changes

The `Cargo.toml` for willow-state needs:
- **Keep**: `bincode`, `serde`, `sha2`, `willow-identity`, `iroh-base`
- **Remove**: `uuid` (event IDs are content hashes now, not UUIDs)
- **No new deps**: `willow-identity` already re-exports `Signature`,
  `PublicKey`, `SecretKey` from `iroh-base` and provides
  `Identity::sign(&[u8]) -> Signature` and
  `verify(&PublicKey, &[u8], &Signature) -> bool`.

## Step 1: EventHash type

**Files**: `crates/state/src/hash.rs` (rewrite)

Replace `StateHash` with `EventHash`. Same 32-byte SHA-256 wrapper,
different semantics (content hash of an event, not hash of full state).

- `EventHash(pub [u8; 32])`
- `EventHash::ZERO` — used as `prev` for an author's first event
- `EventHash::from_bytes(data: &[u8]) -> Self` — SHA-256 of raw bytes
- `Ord` / `PartialOrd` — lexicographic byte comparison, needed for
  `BTreeSet` in topological sort tiebreaking
- `Display` — hex format (same as current `StateHash`)
- `Serialize` / `Deserialize`
- `Default` → `ZERO`

`hash.rs` is a pure hash wrapper. It does not depend on `EventKind`
or any event structure. The serialization of event fields into bytes
(which are then hashed) happens in `Event::new()` in `event.rs`.

**Tests** (in `hash.rs`):
- `zero_hash_is_all_zeros`
- `same_input_same_hash`
- `different_input_different_hash`
- `display_is_hex`
- `ord_is_lexicographic`

## Step 2: EventKind and Event

**Files**: `crates/state/src/event.rs` (new)

### EventKind

Move from `lib.rs`. 20 variants (drop `StateVerification` from original 21).

Field type changes:
- `message_id: String` → `message_id: EventHash` in `EditMessage`,
  `DeleteMessage`, `Reaction`, `PinMessage`, `UnpinMessage`
- `reply_to: Option<String>` → `reply_to: Option<EventHash>` in
  `Message`

Keep `default_create_channel_kind()` serde helper for `CreateChannel`.

`Channel.id`, `Role.id` remain `String` (user-provided UUIDs).

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

Note: `author` is `EndpointId` which is `PublicKey`. The `verify()`
function in `willow-identity` takes `&PublicKey`.

### ChatMessage.id type change

`ChatMessage.id` in `types.rs` changes from `String` to `EventHash` —
message identity is the event hash of the `Message` event that created
it. `ChatMessage.reply_to` changes from `Option<String>` to
`Option<EventHash>`.

**Tests**:
- `event_hash_is_deterministic` — same fields → same hash
- `event_hash_changes_with_any_field` — change each field, hash differs
- `event_signature_verifies` — round-trip sign + verify
- `event_signature_rejects_tampered` — modify field, verify fails
- `event_signature_rejects_wrong_key` — sign with A, verify with B

## Step 3: EventDag — core structure and insertion

**Files**: `crates/state/src/dag.rs` (new)

### Struct

```rust
pub struct EventDag {
    events: HashMap<EventHash, Event>,
    chains: HashMap<EndpointId, Vec<EventHash>>,
    heads: HashMap<EndpointId, EventHash>,
}
```

### InsertError

```rust
pub enum InsertError {
    InvalidSignature,
    SeqGap { author: EndpointId, expected: u64, got: u64 },
    PrevMismatch { author: EndpointId, expected: EventHash, got: EventHash },
    Duplicate,
}
```

### insert()

1. `event.verify()` → `InvalidSignature`
2. `events.contains_key(&event.hash)` → `Duplicate`
3. Check seq: must be `latest_seq(author) + 1` → `SeqGap`
4. Check prev: must match `head(author)` or `ZERO` for seq=1 → `PrevMismatch`
5. Insert into `events`, push hash to `chains[author]`, set `heads[author]`
6. Return `Ok(())`. Unknown deps in `event.deps` are silently accepted.

### Accessors

- `new() -> Self`
- `latest_seq(&self, author) -> u64` (0 if unknown)
- `head(&self, author) -> Option<&EventHash>`
- `author_events(&self, author) -> &[EventHash]` (empty slice if unknown)
- `get(&self, hash) -> Option<&Event>`
- `len(&self) -> usize` (total events)
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
- `insert_first_event`
- `insert_sequential_events`
- `insert_rejects_duplicate`
- `insert_rejects_invalid_signature`
- `insert_rejects_seq_gap`
- `insert_rejects_prev_mismatch`
- `insert_accepts_unknown_deps`
- `insert_multiple_authors`
- `insert_with_cross_author_deps`

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

- Remove `seen_event_ids: HashSet<String>`
- Remove `hash()` method
- Remove `is_trusted()` (legacy backward-compat bridge)
- Keep `has_permission()`, `is_sync_provider()`
- `new(server_id, owner)` — takes 2 params, not 3. `server_name`
  starts empty (set later by `RenameServer` event from owner).

### types.rs changes

- `ChatMessage.id`: `String` → `EventHash`
- `ChatMessage.reply_to`: `Option<String>` → `Option<EventHash>`
- All other types unchanged: `Channel`, `Role`, `Member`, `Profile`,
  `Permission`

### ServerState.new() signature

```rust
pub fn new(id: impl Into<String>, owner: EndpointId) -> Self
```

`server_name` defaults to empty string. Owner is added as member.

**Tests**:
- `new_server_has_owner_as_member`
- `owner_has_all_permissions`
- `peer_without_permissions`
- `admin_has_all_permissions`

## Step 6: Materialization and apply

**Files**: `crates/state/src/materialize.rs` (new)

### Public API

- `materialize(dag, server_id, owner) -> ServerState` — full replay
- `apply_incremental(state, event) -> ApplyResult` — single event
- `ApplyResult { Applied, Rejected(String) }`

### Internal

- `apply_unchecked(state, event) -> ApplyResult` — permission check + mutation
- `apply_mutation(state, event) -> ApplyResult` — the big match block
- `required_permission(kind) -> Option<Permission>`

### The match block

Ported from current `apply_inner` in `lib.rs:341-571`. Changes:
- Remove `StateVerification` arm
- `message_id` fields are `EventHash` not `String`
- `event.id` references become `event.hash`
- No `seen_event_ids` insertion (removed from state)
- `ChatMessage.id` is `event.hash.clone()` not `event.id.clone()`

All 20 remaining match arms carry over with these substitutions.

**Tests**:
- `materialize_empty_dag`
- `materialize_create_channel`
- `materialize_is_deterministic`
- `materialize_two_dags_same_events_same_state`
- `materialize_concurrent_channel_creates`
- `materialize_permission_enforcement`
- `materialize_owner_has_all_permissions`
- `materialize_admin_has_all_permissions`
- `materialize_kick_removes_member_and_permissions`
- `materialize_cannot_kick_owner`
- `materialize_message_in_channel`
- `materialize_edit_message`
- `materialize_delete_message`
- `materialize_reaction`
- `materialize_set_profile`
- `materialize_rename_server_owner_only`
- `materialize_server_description_owner_only`
- `materialize_delete_channel_cascades_messages`
- `materialize_delete_role_cascades_members`
- `materialize_grant_permission_adds_member`
- `incremental_matches_full_materialize`
- `incremental_concurrent_events`

## Step 7: Sync types and PendingBuffer

**Files**: `crates/state/src/sync.rs` (new)

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

All derive `Serialize, Deserialize` for wire transport.

### EventDag methods (in dag.rs, not sync.rs)

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

1. `verify_chain(author, &new_chain)?` — check all sigs, seq monotonicity, prev consistency, all events have matching author
2. Remove old events for this author from `events` map
3. Insert new events
4. Update `chains[author]` and `heads[author]`

### RevisionError

```rust
pub enum RevisionError {
    InvalidSignature { seq: u64 },
    BrokenPrevChain { seq: u64 },
    WrongAuthor { seq: u64, expected: EndpointId, got: EndpointId },
    EmptyChain,
    SeqDoesNotStartAtOne,
}
```

**Tests**:
- `replace_chain_basic`
- `replace_chain_re_materializes_correctly`
- `replace_chain_rejects_invalid_signature`
- `replace_chain_rejects_broken_prev`
- `replace_chain_broken_dep_is_tolerated`
- `revision_detection`

## Step 9: lib.rs and cleanup

**Files**: `crates/state/src/lib.rs` (rewrite)

Delete all old code from lib.rs. New contents:

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

pub use dag::{ChainStatus, EventDag, InsertError, RevisionError};
pub use event::{Event, EventKind};
pub use hash::EventHash;
pub use materialize::{apply_incremental, materialize, ApplyResult};
pub use server::ServerState;
pub use sync::{AuthorHead, AuthorRequest, HeadsSummary, PendingBuffer, SyncMessage};
pub use types::{Channel, ChatMessage, Member, Permission, Profile, Role};
```

Delete old files:
- `merge.rs` — replaced by DAG union + topological sort
- `store.rs` — replaced by `EventDag` (and `DagStore` trait in follow-up)

Note: `EventHash` is defined in `hash.rs` and re-exported from lib.rs.
`event.rs` imports it via `use crate::hash::EventHash`.

**Validation**: `cargo test -p willow-state` — all tests pass.

## Step 10: Stress tests

**Files**: `crates/state/src/tests.rs`

- `stress_1000_events_single_author`
- `stress_100_authors_10_events_each`
- `stress_sort_performance`
- `stress_concurrent_channel_creates`

## Step 11: WASM check and clippy

Run:
- `cargo clippy -p willow-state -- -D warnings`
- `just check-wasm` (or equivalent wasm32 target check)

Fix any issues. Should be clean — no platform-specific code added.

## Order of Operations

```
Step 1  → EventHash                    (hash primitive)
Step 2  → EventKind + Event            (core types, signing)
Step 3  → EventDag insert             (DAG structure, validation)
Step 4  → Topological sort            (ordering algorithm)
Step 5  → ServerState + types         (materialized view structure)
Step 6  → Materialization + apply     (projection from DAG to state)
Step 7  → Sync types + PendingBuffer  (protocol types)
Step 8  → Author revision             (chain replacement)
Step 9  → lib.rs + cleanup            (public API, delete old files)
Step 10 → Stress tests                (scale validation)
Step 11 → WASM + clippy               (platform compat, lint)
```

Steps 1-2 are foundation types. Steps 3-4 are the DAG engine. Steps
5-6 are materialization. Steps 7-8 are sync/revision. Steps 9-11 are
polish.

During Steps 1-8, old code in `lib.rs` may coexist temporarily
(it won't compile but the new modules are tested independently via
`#[cfg(test)]` in each file). Step 9 does the final cleanup.

Alternatively, Step 9 (delete old code) can happen first, making the
crate empty before building up. This is cleaner — no dead code
confusion — but means `cargo test` only passes after enough new code
exists.

**Recommended**: delete old files in Step 1 (they're being fully
replaced anyway), then build up from there. Each step's tests validate
independently.

## What Breaks Downstream

| Crate | What breaks | Fix (follow-up) |
|---|---|---|
| `willow-client` | `apply_lenient()`, old `Event` struct, `EventStore`, `StateHash`, `ChatMessage.id` is now `EventHash` | Adapt mutations.rs, listeners.rs, views.rs per spec Section 7 |
| `willow-common` | `WorkerRequest::Sync { state_hash }`, `WorkerResponse`, `Event` type | Update wire types per spec Section 8 |
| `willow-worker` | `WorkerRole::on_event(&Event)`, state hash methods | Update to new Event type |
| `willow-replay` | `apply_lenient()`, `StateHash`, VecDeque buffer | Rewrite per spec Section 8 |
| `willow-app` | Old `Event` in test helpers, `StateHash`, `merge()` | Update test helpers and e2e tests |
| `willow-web` | Indirect via client | Follows client fixes |

These are expected and intentional. The state crate is the foundation;
downstream adapts to it.
