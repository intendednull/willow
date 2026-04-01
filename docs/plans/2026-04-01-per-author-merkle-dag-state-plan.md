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

## Step 1: EventHash and Signature types

**Files**: `crates/state/src/hash.rs` (rewrite)

Replace `StateHash` with `EventHash`. Keep the same 32-byte SHA-256
wrapper pattern but change the semantics (event content hash, not
state hash).

- `EventHash` struct: `pub struct EventHash(pub [u8; 32])`
- `EventHash::ZERO` constant
- `EventHash::compute(author, seq, prev, deps, kind, timestamp_hint_ms) -> Self`
- `Ord` / `PartialOrd` impl for BTreeSet usage in topo sort
- `Display` impl (hex)
- `Serialize` / `Deserialize`

Add `Signature` type (thin wrapper around `[u8; 64]` for Ed25519).

**Tests**:
- `event_hash_is_deterministic`
- `event_hash_changes_with_any_field`
- `event_hash_ord_is_lexicographic`

## Step 2: EventKind

**Files**: `crates/state/src/event.rs` (new)

Move `EventKind` here from `lib.rs`. Drop `StateVerification` variant.
Change `message_id: String` fields to `message_id: EventHash`.
Keep the `default_create_channel_kind()` serde helper.

23 variants, same structure as current minus `StateVerification`.

**Tests**: Serialization round-trip for each variant (quick sanity).

## Step 3: Event struct

**Files**: `crates/state/src/event.rs` (extend)

Define the new `Event` struct with: `hash`, `author`, `seq`, `prev`,
`deps`, `kind`, `sig`, `timestamp_hint_ms`.

Add `Event::new()` which:
1. Takes identity, seq, prev, deps, kind, timestamp_hint_ms
2. Computes hash via `EventHash::compute()`
3. Signs (author, seq, prev, deps, kind, timestamp_hint_ms) with the identity's private key
4. Returns the complete Event

Add `Event::verify(&self) -> bool` which verifies the signature.

**Tests**:
- `event_signature_verifies`
- `event_signature_rejects_tampered`
- `event_signature_rejects_wrong_key`

## Step 4: EventDag — insertion

**Files**: `crates/state/src/dag.rs` (new)

Define `EventDag` struct with `events`, `chains`, `heads` fields.
Define `InsertError` enum (InvalidSignature, SeqGap, PrevMismatch, Duplicate).

Implement `EventDag::insert()`:
1. Verify signature → InvalidSignature
2. Check duplicate (hash already in events) → Duplicate
3. Check seq = latest_seq + 1 → SeqGap
4. Check prev = current head hash (or ZERO for seq=1) → PrevMismatch
5. Insert into events map, append to chains vec, update head
6. Record any unknown deps (return them for caller to track)

Implement `EventDag::new()`, `latest_seq()`, `head()`, `author_events()`.

Add `EventDag::create_event()` convenience:
1. Reads current head/seq for the identity's author
2. Builds and signs Event with seq+1, prev=current head
3. Does NOT insert (caller does that)

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

## Step 5: Topological sort

**Files**: `crates/state/src/dag.rs` (extend)

Implement `EventDag::topological_sort()`:
- Kahn's algorithm
- BTreeSet for deterministic tie-breaking by EventHash
- Skip deps pointing to absent events (soft-accept)

Implement `EventDag::causal_parents()` helper.

**Tests**:
- `sort_single_author_is_seq_order`
- `sort_independent_authors_by_hash`
- `sort_respects_cross_author_deps`
- `sort_complex_dag`
- `sort_is_deterministic`
- `sort_is_stable_under_insertion_order`

## Step 6: ServerState and types

**Files**: `crates/state/src/server.rs` (rewrite), `crates/state/src/types.rs` (keep)

Simplify `ServerState`:
- Remove `seen_event_ids` field
- Remove `hash()` method
- Remove `is_trusted()` (legacy bridge)
- Keep `has_permission()`, `is_sync_provider()`
- Change `new()` to take `(server_id, owner)` — no name param
  (name is set by a `RenameServer` event)

`types.rs` is unchanged: `Channel`, `Role`, `Member`, `ChatMessage`,
`Profile`, `Permission`.

**Tests**:
- `new_server_has_owner_as_member`
- `owner_has_all_permissions`
- `peer_without_permissions`

## Step 7: Materialization and apply

**Files**: `crates/state/src/materialize.rs` (new)

Implement:
- `materialize(dag, server_id, owner) -> ServerState`
- `apply_incremental(state, event) -> ApplyResult`
- `apply_unchecked(state, event) -> ApplyResult` (internal)
- `apply_mutation(state, event) -> ApplyResult` (the big match block)
- `required_permission(kind) -> Option<Permission>`

The match block in `apply_mutation` is ported directly from the current
`apply_inner`, minus the `StateVerification` arm and with `EventHash`
for message_id fields.

`ApplyResult` enum: `Applied`, `Rejected(String)`.

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

## Step 8: Sync types

**Files**: `crates/state/src/sync.rs` (new)

Define:
- `HeadsSummary` struct
- `AuthorHead` struct
- `SyncMessage` enum (Advertise, Request, Response)
- `AuthorRequest` struct
- `PendingBuffer` struct with `waiting_on_prev` and `missing_deps`

Implement `EventDag::heads_summary()` and `EventDag::events_since()`.
Implement `PendingBuffer::resolve()` and `record_missing_dep()`.

**Tests**:
- `heads_summary_reflects_dag`
- `events_since_returns_delta`
- `events_since_unknown_author`
- `events_since_new_author`
- `sync_round_trip`
- `buffer_and_resolve`
- `missing_deps_recorded`
- `resolve_cascading`

## Step 9: Author revision

**Files**: `crates/state/src/dag.rs` (extend)

Implement:
- `EventDag::replace_chain(author, new_chain) -> Result<(), RevisionError>`
- `EventDag::verify_chain(author, chain) -> Result<(), RevisionError>`
- `ChainStatus` enum and `compare_chains()` function

**Tests**:
- `replace_chain_basic`
- `replace_chain_re_materializes_correctly`
- `replace_chain_rejects_invalid_signature`
- `replace_chain_rejects_broken_prev`
- `replace_chain_broken_dep_is_tolerated`
- `revision_detection`

## Step 10: lib.rs and public API

**Files**: `crates/state/src/lib.rs` (rewrite)

Delete all old code. New lib.rs is just module declarations and
re-exports:

```rust
pub mod dag;
pub mod event;
pub mod materialize;
pub mod server;
pub mod sync;
pub mod types;

pub use dag::{EventDag, InsertError, ChainStatus};
pub use event::{Event, EventHash, EventKind, Signature};
pub use materialize::{materialize, apply_incremental, ApplyResult};
pub use server::ServerState;
pub use sync::{HeadsSummary, AuthorHead, SyncMessage, AuthorRequest, PendingBuffer};
pub use types::{Channel, ChatMessage, Member, Permission, Profile, Role};
```

Delete old files: `merge.rs`, `store.rs`.
`hash.rs` has been rewritten in Step 1.

**Tests**: `cargo test -p willow-state` — all tests pass.

## Step 11: Stress tests

**Files**: `crates/state/src/tests.rs` (extend)

- `stress_1000_events_single_author`
- `stress_100_authors_10_events_each`
- `stress_sort_performance`
- `stress_concurrent_channel_creates`

## Step 12: WASM check

Run `just check-wasm` to verify the crate still compiles for
`wasm32-unknown-unknown`. Fix any issues (should be none — we're
not adding any platform-specific code).

## Dependency Changes

The `Cargo.toml` for willow-state needs:
- Keep: `bincode`, `serde`, `sha2`, `willow-identity`
- Add: `ed25519-dalek` (or use willow-identity's signing — check
  what's available)
- Remove: `uuid` (event IDs are content hashes, not UUIDs)
- Keep: `iroh-base` (for `EndpointId` via willow-identity)

## Order of Operations

```
Step 1  → EventHash, Signature         (foundation types)
Step 2  → EventKind                     (mutation variants)
Step 3  → Event                         (core struct)
Step 4  → EventDag insert              (DAG operations)
Step 5  → Topological sort             (ordering)
Step 6  → ServerState, types           (materialized view)
Step 7  → Materialization, apply       (projection)
Step 8  → Sync types                   (protocol)
Step 9  → Author revision              (sovereignty)
Step 10 → lib.rs, cleanup              (public API)
Step 11 → Stress tests                 (scale validation)
Step 12 → WASM check                   (platform compat)
```

Each step builds on the previous. Steps 1-3 are pure data types with
no dependencies on each other except sequential (Event needs EventHash,
EventHash needs EventKind for hashing). Steps 4-5 build the DAG.
Steps 6-7 build materialization on top of the DAG. Steps 8-9 add
sync and revision. Step 10 wires it all together.

## What Breaks Downstream

After this work, the following crates will have compile errors:

| Crate | What breaks | Fix (follow-up) |
|---|---|---|
| `willow-client` | `apply_lenient()`, old `Event` struct, `EventStore`, `StateHash` | Adapt mutations.rs and listeners.rs per spec Section 7 |
| `willow-common` | `WorkerRequest::Sync { state_hash }`, `WorkerResponse` types | Update wire types per spec Section 8 |
| `willow-worker` | `WorkerRole::on_event(&Event)`, state hashes | Update to new Event type |
| `willow-replay` | `apply_lenient()`, `StateHash`, event buffer | Rewrite per spec Section 8 |
| `willow-app` | Old `Event` in tests, `StateHash` | Update test helpers |
| `willow-web` | Indirect via client | Follows client fixes |

These are expected and intentional. The state crate is the foundation;
downstream adapts to it.
