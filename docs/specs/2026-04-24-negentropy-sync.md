# History sync — consolidating on heads-based delta exchange

> **One-sentence summary:** Replace the legacy gossip-level
> `WireMessage::SyncRequest { state_hash, topic }` "first 500 events
> from topological sort" dump with the worker's already-existing
> `HeadsSummary`-based delta protocol, hoisting it to the gossip path
> so client↔client sync uses the same per-author seq cursor as
> client↔worker sync.

## Motivation

Willow already has two sync code paths in production today, and they
do not agree:

1. **Worker path (`WorkerRequest::Sync { server_id, heads: HeadsSummary }`)**
   — clients exchange `HeadsSummary` (a `BTreeMap<EndpointId,
   AuthorHead { seq, hash }>`) with replay and storage workers, and the
   worker streams a per-author delta via `EventDag::events_since`
   (replay) or `StorageEventStore::sync_since` (storage). This path
   already does what we want: it transmits only events the requester is
   missing, scoped per author.
   *See* [`crates/replay/src/role.rs`][replay-role-sync] (lines 264–316),
   [`crates/storage/src/role.rs`][storage-role-sync] (lines 78–85),
   [`crates/storage/src/store.rs`][storage-store-sync] (`sync_since`,
   lines 281–381).

2. **Gossip path (`WireMessage::SyncRequest { state_hash, topic }`)** —
   peer A asks peer B for "events I'm missing relative to state hash X."
   The DAG model has no efficient way to answer that question, so the
   responder dumps the first 500 events from a topological sort and
   relies on `InsertError::Duplicate` to dedupe on the receiver. There
   is an in-tree TODO acknowledging this:

   > ```rust
   > // Legacy field — can't filter by state hash in DAG model.
   > // TODO: Migrate clients to worker's heads-based sync protocol
   > // (WorkerRequest::Sync { heads }) for efficient delta sync.
   > // For now, send the first 500 events from topological sort.
   > // Receiver will dedup via InsertError::Duplicate.
   > ```
   > — [`crates/client/src/listeners.rs:292-297`][listeners-todo]

This spec resolves the TODO. The novelty is **not** introducing a new
per-author cursor — `HeadsSummary` already exists in
[`crates/state/src/sync.rs:21-33`][heads-summary] and is already
serialized over the wire by the worker protocol. The novelty is:

- Replacing the gossip `SyncRequest { state_hash, topic }` payload with
  a `HeadsSummary`-based payload so the client↔client gossip path uses
  the same protocol as the client↔worker request path.
- Adding an explicit `SyncFilter` so callers can scope a sync to a
  specific server / channel set / author set / event-kind set.
- Defining streaming termination semantics (`more: bool`) so a single
  sync exchange can span multiple `MAX_DESER_SIZE` envelopes.
- Removing the 500-event topological-sort fallback.

The DAG already enforces per-author monotonicity — every author's
chain is a strictly increasing sequence enforced in
[`crates/state/src/dag.rs:146-160`][dag-seq-check] (`expected_seq =
self.latest_seq(&event.author) + 1`). Combined with the prev-hash
check at lines 161–172, this makes streaming `seq > known_max` in
ascending order delivers a contiguous chain with no gaps and no
duplicates, so **no fingerprint negotiation is required**.

This unlocks:

- Clients rejoining after downtime transfer only new events (1 RTT)
  even when peering directly (not through a worker).
- Storage↔storage replication for geographic redundancy uses the same
  protocol as everything else.
- Replay workers backfill from storage on boot via the same protocol.
- Relay-mediated sync stays bounded in envelope count because the
  responder knows where to stop.

[replay-role-sync]: ../../crates/replay/src/role.rs
[storage-role-sync]: ../../crates/storage/src/role.rs
[storage-store-sync]: ../../crates/storage/src/store.rs
[listeners-todo]: ../../crates/client/src/listeners.rs
[heads-summary]: ../../crates/state/src/sync.rs
[dag-seq-check]: ../../crates/state/src/dag.rs

## Algorithm

A single round trip with streaming response.

**Phase 1 — Client request.** The client computes its current
[`HeadsSummary`][heads-summary] from its local DAG (already exposed
via `EventDag::heads_summary()` in
[`crates/state/src/sync.rs`][heads-summary]) scoped to a `SyncFilter`,
and sends a `SyncRequest`.

**Phase 2 — Responder stream.** The responder, for each author whose
`our_max_seq > requester.heads[author].seq` (or absent from the
requester's `heads`), streams the missing events in `(author, seq)`
ascending order via the existing per-author tail query
(`EventDag::events_since` or `StorageEventStore::sync_since`). Authors
not mentioned in the requester's `heads` default to `known_max = 0`
(the requester has nothing for that author yet). Events are batched
into one or more `SyncBatch { events, more: true }` envelopes, each
fitting `MAX_DESER_SIZE = 256 KB`.

**Phase 3 — Termination.** The final envelope carries
`SyncBatch { events: …, more: false }`. The client emits a
`HistorySyncComplete` event for the UI per the EOSE spec (#214); see
[Termination + EOSE](#termination--eose).

Per-author monotonicity (DAG invariant at
[`crates/state/src/dag.rs:146-160`][dag-seq-check]) guarantees that
streaming `seq > known_max` in ascending order delivers a contiguous
chain with no gaps and no duplicates, so **no sort key negotiation is
required**. Authority events (e.g. `GrantPermission`, `CreateChannel`)
are authored just like chat events and ride along on the same
per-author chains.

## Wire protocol

This spec replaces the existing `WireMessage::SyncRequest { state_hash,
topic }` and clarifies the semantics of `WireMessage::SyncBatch`. Both
are already variants of the `WireMessage` enum in
[`crates/common/src/wire.rs:13-28`][wire-msg], wrapped in
`MessageType::Channel` envelopes (see [`crates/transport/src/lib.rs:62-79`][message-type]
for the current `MessageType` allocation: `Chat=0` through `Ping=6`,
with slots 7+ unallocated in current code).

[wire-msg]: ../../crates/common/src/wire.rs
[message-type]: ../../crates/transport/src/lib.rs

Two design choices to call out explicitly:

1. **Do these stay inside `WireMessage` (envelope kind `Channel`) or
   get promoted to top-level `MessageType` slots?** Today the worker
   path uses `WireMessage::Worker(WorkerWireMessage::Request { …
   payload: WorkerRequest::Sync … })` and the gossip path uses
   `WireMessage::SyncRequest`. Either is workable; this spec keeps
   both inside `WireMessage` for now (no new `MessageType` variant) so
   the transport-level envelope shape is unchanged. Hoisting to a
   dedicated `MessageType::Sync` slot is a future option once the
   worker and client paths are demonstrably interchangeable.

2. **Reuse `HeadsSummary` directly, do not invent a new
   `HashMap<EndpointId, u64>` shape.** `HeadsSummary` already carries
   `AuthorHead { seq, hash }`. The hash field powers
   `compare_chains(...)` ([`crates/state/src/sync.rs:118`][heads-summary])
   for fork detection — dropping it would lose that capability for free
   on every gossip-level sync. We keep the hash.

```rust
// In crates/common/src/wire.rs — replaces today's SyncRequest variant:
pub enum WireMessage {
    Event(willow_state::Event),

    // REPLACES the legacy `SyncRequest { state_hash, topic }`. The
    // payload is the requester's HeadsSummary plus an optional filter.
    SyncRequest {
        heads:  willow_state::HeadsSummary,
        filter: SyncFilter,
    },

    // Existing variant; gains a `more` flag and a `request_id` so a
    // multi-envelope response can be correlated and terminated.
    SyncBatch {
        request_id: u64,
        events:     Vec<willow_state::Event>,
        more:       bool,
    },

    // … other variants unchanged …
}

pub struct SyncFilter {
    /// Required. Event-DAG genesis hash hex (matches the existing
    /// `String` server_id used in EventKind, e.g. EventKind::Message
    /// { channel_id: String, ... }). This is NOT a newtype today.
    pub server_id:   String,

    /// Narrows chat-shaped kinds only; structural events ignore this.
    /// Plain `String` channel IDs to match `EventKind::Message
    /// { channel_id: String, ... }`. The `ChannelId` newtype in
    /// `willow-messaging` is unrelated.
    pub channels:    Option<Vec<String>>,

    pub authors:     Option<Vec<willow_identity::EndpointId>>,
    pub event_kinds: Option<Vec<u8>>,
    pub since_ms:    Option<u64>,
}
```

Each `SyncBatch` payload (post-Serde, post-`Envelope`) is bounded by
`MAX_DESER_SIZE = 256 KB`
([`crates/transport/src/lib.rs:36`][message-type]). Responders pack
events greedily until the next event would overflow, emit
`SyncBatch { request_id, events, more: true }`, and continue. The
final batch sets `more: false`.

The worker-side `WorkerRequest::Sync { server_id, heads: HeadsSummary }`
in [`crates/common/src/worker_types.rs:88-95`][worker-types] is already
the heads-based protocol; this spec aligns the gossip-level field
shape with it so the same `HeadsSummary` value can drive both paths
unchanged. Where the gossip path needs streaming + filtering, the
worker `WorkerResponse::SyncBatch { events: Vec<Event> }` will need to
gain matching `request_id` and `more` fields. This is a coordinated
change to both `WireMessage` and `WorkerResponse`.

[worker-types]: ../../crates/common/src/worker_types.rs

## Filter semantics

```rust
pub struct SyncFilter {
    pub server_id:   String,                        // required
    pub channels:    Option<Vec<String>>,           // narrows chat-shaped kinds only
    pub authors:     Option<Vec<EndpointId>>,       // restrict to these authors
    pub event_kinds: Option<Vec<u8>>,               // EventKind tag whitelist
    pub since_ms:    Option<u64>,                   // soft floor; see below
}
```

- Empty `Option`s = no restriction on that axis.
- `channels` only narrows chat-shaped `EventKind`s. Structural events
  (`GrantPermission`, `CreateChannel`, `RotateChannelKey`, …) ignore
  the channel filter so structure always reconciles fully.
- `since_ms` is **advisory** — the per-event `timestamp_hint_ms` is
  display-only (see the [timestamp note](#a-note-on-timestamp_hint_ms)).
  The authoritative bound is the per-author `seq` cursor in
  `HeadsSummary`. `since_ms` is intended only as a coarse pre-filter
  to reduce DB scan width on the responder.
- `event_kinds` uses the stable `EventKind` discriminant byte; see
  "Adding a new EventKind" in `CLAUDE.md`.

## A note on `timestamp_hint_ms`

The `timestamp_hint_ms` field on `Event` is **display-only** per
[`crates/state/src/event.rs:216-217`][ts-hint] and intentionally not
part of the sync protocol's correctness guarantees. It is not used to
order, dedupe, or terminate sync. The per-author `seq` carried in
`HeadsSummary` is the sole authoritative cursor.

[ts-hint]: ../../crates/state/src/event.rs

## Integration points

| Pair | Direction | Notes |
|---|---|---|
| client ↔ replay worker | client initiates on connect | **Already** uses `WorkerRequest::Sync { heads: HeadsSummary }`. This spec layers the optional `SyncFilter` on top and standardizes streaming termination. |
| client ↔ storage worker | client initiates on connect / scrollback | **Already** uses `WorkerRequest::Sync` against `StorageEventStore::sync_since`. Same delta. |
| client ↔ client (gossip) | initiator on join | **Replaces** the legacy `WireMessage::SyncRequest { state_hash, topic }` "first 500 events" path with the heads-based payload. This is the load-bearing change. |
| replay ↔ storage | replay initiates on boot | Warm-start; replay worker streams missing chains from storage using the same protocol it serves to clients. |
| storage ↔ storage | either side | Geographic redundancy. Both peers SHOULD hold `SyncProvider` permission once the gate is enforced (see [Bandwidth and safety](#bandwidth-and-safety)). |

The [Relay](../../crates/relay/src/lib.rs) remains a stateless bridge:
it forwards `SyncRequest` and `SyncBatch` envelopes unchanged.

## Storage requirements

The hot query is "events for `(server_id, author)` with `seq > N`,
ordered ascending, capped at a limit." Today's storage worker schema
already has:

```sql
CREATE INDEX idx_events_author_seq ON events(author, seq);
```

defined in [`crates/storage/src/store.rs:41`][store-schema] (migration
1). This index is **not** server-prefixed, which is fine for a
single-server deployment but suboptimal once one storage worker tracks
multiple servers. The migration plan:

1. Add a new migration appending
   `CREATE INDEX idx_events_server_author_seq ON events(server_id, author, seq);`
2. Drop the old `idx_events_author_seq` after the new index is
   verified in production (a separate migration so the rollout is
   reversible).
3. Update `sync_since` to use the new index (the existing
   implementation in [`crates/storage/src/store.rs:289-381`][store-schema]
   already filters by `server_id` and `author`, just over a less
   selective index today).

[store-schema]: ../../crates/storage/src/store.rs

Hot query (unchanged shape, better-indexed plan):

```sql
SELECT * FROM events
WHERE server_id = ?
  AND author    = ?
  AND seq       > ?
ORDER BY seq ASC
LIMIT ?;
```

The responder iterates the union of `(authors in requester.heads ∪
authors known locally)` filtered by `SyncFilter.authors`, paging the
above query per author and packing into `SyncBatch` envelopes.

The in-memory replay worker maintains
`HashMap<EndpointId, BTreeMap<u64, Arc<Event>>>` (effectively, via
`EventDag::events_since` in [`crates/state/src/sync.rs`][heads-summary])
to support the same query shape with a `range((known_max + 1)..)` scan.

### Per-author tail query helpers

Today there is **no `EventStore` trait in `willow-state`** — the
state crate is pure (zero I/O) and the actual stores are concrete
types: `StorageEventStore` (SQLite, in `crates/storage/src/store.rs`)
and the in-memory `EventDag` (in `crates/state/src/dag.rs`) used by
clients and replay workers.

The per-author tail query already exists in both:

- `EventDag::events_since(&BTreeMap<EndpointId, u64>, Option<usize>)
  -> Vec<&Event>` ([`crates/state/src/sync.rs`][heads-summary])
- `StorageEventStore::sync_since(&str, &HeadsSummary) ->
  anyhow::Result<Vec<Event>>`
  ([`crates/storage/src/store.rs:289-381`][store-schema])

This spec does **not** introduce an `EventStore` trait. It only
requires that both stores expose:

```rust
// Equivalent of the existing methods, but reachable from both sides
// of the sync protocol via a small adapter rather than a trait. If a
// future worker needs to plug in a third backend, *that* is when we
// extract the trait.

// Pseudocode shape — actual signatures match the existing functions:
fn events_since(server: &str, requester_heads: &HeadsSummary, limit: usize)
    -> impl Iterator<Item = Event>;

fn known_authors(server: &str) -> Vec<EndpointId>;
```

`known_authors` is a small new helper for the responder to pick up
authors the requester didn't mention. Both backends can implement it
trivially (`EventDag` from its `chains` map; `StorageEventStore` from
`SELECT DISTINCT author FROM events WHERE server_id = ?`).

Browser-only clients implement these against IndexedDB but only need
to *serve* if they ever respond to peer requests; pure leaf clients
just need their own `HeadsSummary` (already produced by
`EventDag::heads_summary()`) to build the request.

## Termination + EOSE

`SyncBatch { request_id, more: false }` is the canonical end-of-stream
marker. Upon receipt the client:

1. Applies the batch via the public materialize entry point. The
   primary path is `EventDag::insert(event)`
   ([`crates/state/src/dag.rs`][dag-insert]), which validates per-author
   `seq` and `prev`, followed by `apply_incremental(state, &event)`
   ([`crates/state/src/materialize.rs:61`][materialize]) to update
   `ServerState`. The internal `apply_event` (line 130 in
   `materialize.rs`) is private and not part of the public API.
2. Emits a `HistorySyncComplete` client event consumed by the UI per
   the EOSE spec (#214), which owns the user-visible "history loaded"
   signal and the `MessageType` slot 7 reservation.

This spec deliberately does not redefine `HistorySyncComplete`; it
only triggers it.

[dag-insert]: ../../crates/state/src/dag.rs
[materialize]: ../../crates/state/src/materialize.rs

## Recovery — encrypted channel-key replay

Heads-based exchange recovers the public DAG including
`RotateChannelKey` events, but a late-joining peer still lacks the
**sealed key shares** needed to decrypt historical messages (sealed
shares are unicast, not in the DAG).

After the `SyncBatch { more: false }` arrives, for every channel where
the client now sees a `RotateChannelKey` epoch it cannot decrypt, it
emits the `RequestEpochKey { channel_id, epoch }` message defined by
spec #220. Any current channel member with the unwrapped epoch key
responds with a directed re-wrap addressed to the requester's
endpoint.

This is **out-of-band** to the heads-based exchange — it rides on the
existing unicast envelope path. Heads-based sync surfaces the gap;
#220 fills it. See open question on placement.

## Migration

This spec supersedes the prior `(timestamp, hash)` Negentropy sketch
in this same file and the per-author seq-vector sketch from the
preceding revision. The naming aligns with both the existing wire
variants in [`crates/common/src/wire.rs`][wire-msg] and the worker
design doc
[`docs/specs/2026-03-27-worker-nodes-design.md`](2026-03-27-worker-nodes-design.md).

There are **two distinct existing code paths** that both happen to use
the names `SyncRequest` / `SyncBatch`, and the spec touches both:

1. **`WireMessage::SyncRequest` / `WireMessage::SyncBatch`** in
   [`crates/common/src/wire.rs:13-28`][wire-msg] — for client↔client
   gossip. **Payload shape changes** from `{ state_hash, topic }` to
   `{ heads, filter }`, and `SyncBatch` gains `request_id` + `more`.
   This is a wire-incompatible change to the gossip protocol — the
   structural change is contained inside the existing `WireMessage`
   enum; no new `MessageType` slot is added.

2. **`WorkerRequest::Sync` / `WorkerResponse::SyncBatch`** in
   [`crates/common/src/worker_types.rs:88-125`][worker-types] — for
   client↔worker request/response. The `WorkerRequest::Sync` payload
   is **unchanged** (it already carries `HeadsSummary`).
   `WorkerResponse::SyncBatch` gains `request_id` + `more` to match
   the gossip-side `SyncBatch` and support multi-envelope streaming.

Cutover: bump `PROTOCOL_VERSION` in
[`crates/transport/src/lib.rs:30`][message-type] together with the
wire change. Old clients see the new `SyncRequest` payload as a Serde
decode failure and ignore it; new clients ignore old `SyncRequest`
variants. Because the legacy gossip path was already a 500-event
heuristic dump, the user-facing degradation during rollout is at most
"slower bootstrap until both peers are upgraded," matching the status
quo.

## Bandwidth and safety

- `SyncRequest.heads` size: `O(authors_known)` × ~72 bytes (32-byte
  `EndpointId` + 8-byte `u64` seq + 32-byte head hash). 1000 authors
  ≈ 72 KB; well within `MAX_DESER_SIZE`.
- `SyncBatch` is bounded per envelope; total bytes are bounded by the
  actual diff, never by `|history|`.
- Responders enforce a per-peer concurrency cap (e.g. 2 in-flight
  responses) and a per-session wall-clock budget to bound memory.
- **Serving SHOULD be gated by `SyncProvider`**
  ([`crates/state/src/event.rs:23`][permission-enum]) once the gate
  is wired up. Today, neither the worker code paths
  ([`crates/replay/src/role.rs:264`][replay-role-sync],
  [`crates/storage/src/role.rs:78`][storage-role-sync]) nor the gossip
  path checks this permission — any peer can request a delta. Adding
  the gate is **proposed by this spec** as part of the cutover; peers
  without `SyncProvider` MAY initiate but MUST refuse to serve once
  the gate lands.

[permission-enum]: ../../crates/state/src/event.rs

## Testing

| Tier | Test | Location |
|---|---|---|
| unit | `EventDag::events_since` returns contiguous `(author, seq)` ranges, empty when up-to-date (already covered) | `crates/state/src/sync.rs` (existing tests at lines 418–501) |
| unit | `StorageEventStore::sync_since` for known and unknown server IDs (already covered) | `crates/storage/src/store.rs` (existing tests at lines 998–1085) |
| unit | New: `events_since` accepts a `SyncFilter` and respects `channels` / `authors` / `event_kinds` / `since_ms` | `crates/state/src/sync.rs` (extend existing module) |
| unit | New: `WireMessage::SyncRequest { heads, filter }` and `SyncBatch { request_id, events, more }` Serde round-trip; envelope size bound | `crates/common/src/wire.rs` (extend inline `#[cfg(test)]` module that already covers `SyncRequest` / `SyncBatch` round-trip) |
| unit | New: Batching: 5 KB events × 100 authors split correctly across `SyncBatch` envelopes with `more` flag and consistent `request_id` | `crates/state/src/sync.rs` or a new `crates/network/src/sync.rs` (location TBD by implementer) |
| integration | Three-peer convergence: A has authors {x:1..100}, B has {y:1..100}, C empty; C syncs from A then B and ends with both chains complete | `crates/client/src/tests/` against `willow_network::mem::MemNetwork` |
| integration | Edge cases: empty store, requester already up-to-date (zero-event response with `more: false`), single missing event, author entirely unknown to requester | same crate |
| integration | Authority events sync identically (server-create, grant, kick reach client without special-casing) | same crate |
| E2E | Client offline reconnect transfers only the diff (byte-count assertion); `HistorySyncComplete` fires | `e2e/history-sync.spec.ts` |

The testing tier follows the project rule "default to the lowest tier
that can cover the behaviour" (see `CLAUDE.md`). Wire round-trips and
sync-algorithm correctness are unit tests; multi-peer convergence
prefers `MemNetwork` over Playwright.

## Future work / Appendix A — Negentropy fallback

The heads-based approach is optimal when divergence is per-author-tail
(the common case: a peer was offline, missed the last K events from
each active author). It is **not** optimal for *cross-author* set
divergence — e.g. two storage replicas that each independently dropped
a different middle slice of history. In that pathological case the
heads exchange would re-send full author tails when only an interior
gap is missing.

A future v2 may layer Negentropy / RBSR (NIP-77, Hoyte) over a
secondary `(author, seq)` keyspace as a fallback for replicas that
detect tail divergence (e.g. via `compare_chains` returning `Forked`
in [`crates/state/src/sync.rs:118`][heads-summary]). Implementation
path: reuse iroh-docs' existing range-based reconciliation primitives
rather than porting `rust-nostr/negentropy`. This is deferred until a
concrete operational need arises; for the `client ↔ worker`,
`worker ↔ worker`, and steady-state `client ↔ client` cases the
heads-based approach is strictly sufficient given the DAG's per-author
monotonicity invariant.

## Open questions

1. **Where does `RequestEpochKey` live?** Spec #220 defines the
   message; this spec triggers it. Should the trigger logic live in
   `willow-client` (pull-based, after `HistorySyncComplete`) or in
   the channel decryption path (lazy, on first failed decrypt)?
   Pull-based is simpler; lazy is more bandwidth-friendly.
2. **Per-author rate-limiting on `SyncRequest`.** A malicious or
   buggy client could open many sessions with disjoint author
   subsets to amplify responder DB work. Should the responder
   maintain a per-peer token bucket keyed on
   `(peer, requested_author_count)`, or rely solely on the proposed
   `SyncProvider` admission gate plus the concurrency cap?
3. **Promote to a top-level `MessageType` slot?** This spec keeps
   sync inside `WireMessage::Channel` for now (matching the existing
   shape). Once worker and gossip paths share enough code, hoisting
   to `MessageType::Sync = 8` (slot 7 reserved by EOSE spec #214)
   would let middleboxes route sync traffic without parsing the
   inner envelope. Defer until the consolidation lands.
