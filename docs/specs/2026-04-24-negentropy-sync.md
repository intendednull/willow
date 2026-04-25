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
   *See* [`crates/replay/src/role.rs`][replay-role-sync]
   (`Sync` arm, lines 266–316),
   [`crates/storage/src/role.rs`][storage-role-sync] (lines 78–85),
   [`crates/storage/src/store.rs`][storage-store-sync] (`sync_since`,
   lines 289–381).

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
  sync exchange can span multiple gossip envelopes (the binding cap
  is iroh-gossip's 64 KiB `max_message_size`, **not** transport's
  256 KB `MAX_DESER_SIZE`; see [Wire protocol](#wire-protocol)).
- Removing the 500-event topological-sort fallback.

The DAG already enforces per-author monotonicity — every author's
chain is a strictly increasing sequence enforced in
[`crates/state/src/dag.rs:146-158`][dag-seq-check] (`expected_seq =
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
[dag]: ../../crates/state/src/dag.rs
[dag-seq-check]: ../../crates/state/src/dag.rs

## Algorithm

A single round trip with streaming response.

**Phase 1 — Client request.** The client computes its current
[`HeadsSummary`][heads-summary] from its local DAG (already exposed
via `EventDag::heads_summary()` at
[`crates/state/src/dag.rs:267`][dag]; the `HeadsSummary` /
`AuthorHead` types themselves live in
[`crates/state/src/sync.rs:21-33`][heads-summary]) scoped to a
`SyncFilter`, and sends a `SyncRequestV2 { request_id, heads, filter }`.

**Phase 2 — Responder stream.** The responder, for each author whose
`our_max_seq > requester.heads[author].seq` (or absent from the
requester's `heads`), streams the missing events in `(author, seq)`
ascending order via the existing per-author tail query
([`EventDag::events_since`][dag] at `crates/state/src/dag.rs:282` or
[`StorageEventStore::sync_since`][store-schema] at
`crates/storage/src/store.rs:289-381`). Authors not mentioned in the
requester's `heads` default to `known_max = 0` (the requester has
nothing for that author yet). Authors the requester *does* list but
that we don't know locally are silently ignored — we cannot serve what
we don't have (matches the existing
[`events_since_unknown_author` test][heads-summary] at
`crates/state/src/sync.rs:464-476`). Events are batched into one or
more `SyncBatchV2 { events, more: true }` envelopes, each sized to fit
within the gossip transport's 64 KiB limit (see
[Wire protocol](#wire-protocol)).

**Phase 3 — Termination.** The final envelope carries
`SyncBatchV2 { events: …, more: false }`. The client emits a
`HistorySyncComplete` event for the UI per the EOSE spec (#214); see
[Termination + EOSE](#termination--eose).

Per-author monotonicity (DAG invariant at
[`crates/state/src/dag.rs:146-158`][dag-seq-check]) guarantees that
streaming `seq > known_max` in ascending order delivers a contiguous
chain with no gaps and no duplicates, so **no sort key negotiation is
required**. Authority events (e.g. `GrantPermission`, `CreateChannel`)
are authored just like chat events and ride along on the same
per-author chains.

## Wire protocol

This spec adds two new **additive** variants alongside the existing
`WireMessage::SyncRequest { state_hash, topic }` and
`WireMessage::SyncBatch { events }`. The legacy variants stay defined
and decodable for one release cycle so old peers and new peers can
co-exist on the wire (see [Migration](#migration) for the rationale).
All four variants live inside the `WireMessage` enum in
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

3. **Use the same `request_id` type as the worker path.** The
   existing worker correlation field is
   `WorkerWireMessage::Request { request_id: String, .. }` /
   `Response { request_id: String, .. }` at
   [`crates/common/src/worker_types.rs:73-84`][worker-types]. The new
   gossip variants use `request_id: String` for the same reason —
   shared demux/dispatch helpers stay monomorphic instead of needing a
   `String`/`u64` adapter at every callsite.

```rust
// In crates/common/src/wire.rs — additive variants. The legacy
// SyncRequest / SyncBatch variants stay untouched so old peers can
// continue to decode envelopes from new peers (and vice versa) until
// the legacy path is removed in a later release.
pub enum WireMessage {
    Event(willow_state::Event),

    // Legacy variants kept verbatim for one release cycle so old peers
    // do not see decode failures on the entire envelope:
    SyncRequest { state_hash: willow_state::EventHash, topic: Option<String> },
    SyncBatch   { events: Vec<willow_state::Event> },

    // NEW additive variants. Old peers fail to decode just these
    // variants (the unknown enum tag), not the whole envelope.
    // request_id is `String` to match the worker path's correlation
    // type (`WorkerWireMessage::Request { request_id: String, .. }`
    // in worker_types.rs:73-78); reusing the same type lets a single
    // demux table cover both paths.
    SyncRequestV2 {
        request_id: String,
        heads:      willow_state::HeadsSummary,
        filter:     SyncFilter,
    },
    SyncBatchV2 {
        request_id: String,
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

Each `SyncBatchV2` payload is bounded by the **gossip layer's 64 KiB
`max_message_size`**, not transport's deserialization safety cap.
Concretely:

- iroh-gossip is built with `max_message_size(65536)` at
  [`crates/network/src/iroh.rs:270`][iroh-gossip-cap]. Frames exceeding
  64 KiB are dropped at the gossip layer before they ever reach
  transport.
- Transport's `MAX_DESER_SIZE = 256 KB`
  ([`crates/transport/src/lib.rs:36`][message-type]) is only a
  deserialization-time anti-DoS cap and is **deliberately set above**
  the gossip ceiling so the framing overhead can't trip it. The
  comment at `transport/lib.rs:33-35` makes this explicit.
- Therefore the responder's per-envelope budget is **64 KiB minus a
  small constant** for envelope + signature framing. The constant is
  bounded by: `SignedMessage` adds ~104 B (32 B public key + 64 B
  signature, each carried as `Vec<u8>` with 8 B bincode length prefix);
  `Envelope` adds ~11 B (`u16` version + `u8` `MessageType` + 8 B `Vec<u8>`
  length prefix); the `WireMessage` enum tag is ~4 B; and the
  `SyncBatchV2` payload header (`request_id` String length prefix +
  `events` Vec length prefix + `more` bool) is ~25 B. Total framing
  overhead is well under 200 B, so responders treat the per-envelope
  budget as **64 KiB − ~200 B ≈ 65,300 bytes** available for the
  serialized event sequence. Responders pack events greedily until the
  next event would push the serialized envelope past that budget, emit
  `SyncBatchV2 { request_id, events, more: true }`, and continue. The
  final batch sets `more: false`. Implementers SHOULD measure the actual
  framing overhead in a unit test and tune the constant rather than
  relying on the estimate above.

This aligns the new gossip-side `SyncBatchV2` budget with how the
existing worker `WorkerResponse::SyncBatch` already operates today
(also gossip-bound — see [`crates/worker/src/actors/sync.rs:79-87`][worker-sync]
and the `topic.broadcast(...)` path), and supersedes **two** event-count
caps that exist today:

- Producer-side: `SYNC_BATCH_LIMIT = 10_000` at
  [`crates/storage/src/store.rs:287`][store-schema], which can already
  overflow the gossip cap in practice for non-trivial event sizes.
  Replaced by the per-envelope byte budget above. A storage-side
  per-call cap stays useful as an OOM guard, but the authoritative
  bound for sync streaming is the gossip envelope budget.
- Receiver-side: `MAX_SYNC_BATCH_SIZE = 10_000` at
  [`crates/client/src/listeners.rs:256`][listeners-todo], which today
  rejects oversized inbound `SyncBatch` envelopes. With per-envelope
  byte sizing this cap becomes effectively a no-op (a 64 KiB envelope
  cannot hold 10,000 non-trivial events). Implementers SHOULD retain
  it explicitly as defense-in-depth against a malicious/buggy peer
  serializing 10,000+ tiny events into a single envelope, or remove it
  and document that the gossip cap is the sole bound — pick one and
  call it out in the implementation PR.

[iroh-gossip-cap]: ../../crates/network/src/iroh.rs
[worker-sync]: ../../crates/worker/src/actors/sync.rs

The worker-side `WorkerRequest::Sync { server_id, heads: HeadsSummary }`
in [`crates/common/src/worker_types.rs:88-95`][worker-types] is already
the heads-based protocol; this spec aligns the gossip-level field
shape with it so the same `HeadsSummary` value can drive both paths
unchanged. Where the gossip path needs streaming + filtering, the
worker `WorkerResponse::SyncBatch { events: Vec<Event> }` only needs
to gain a `more: bool` field — request correlation already lives in
the outer envelope as `WorkerWireMessage::Response { request_id:
String, target_peer, payload }` (see
[`crates/common/src/worker_types.rs:79-84`][worker-types]). Adding
`request_id` inside the payload would duplicate it. So:

- **Gossip path** (`WireMessage`): adds *both* `request_id` and `more`
  on the new additive `SyncRequestV2` / `SyncBatchV2` variants, because
  the outer `Envelope` carries no per-exchange correlation.
- **Worker path** (`WorkerResponse::SyncBatch`): adds *only* `more`;
  reuses the outer `WorkerWireMessage::Response.request_id`.

This asymmetry is intentional and avoids duplicating the correlation
ID on the worker path.

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
it forwards `SyncRequestV2` and `SyncBatchV2` envelopes unchanged
(and continues to forward the legacy `SyncRequest` / `SyncBatch`
variants for the duration of the migration window).

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
3. Update `sync_since` to use the new index. The existing
   implementation in [`crates/storage/src/store.rs:289-381`][store-schema]
   has two branches: an empty-heads branch
   (`store.rs:289-319`) that issues `SELECT … WHERE server_id = ?
   ORDER BY seq ASC LIMIT ?` (no author filter, server scan), and a
   non-empty branch (`store.rs:321-349`) that builds an OR-joined
   disjunction `(author = ? AND seq > ?)` per requester-known author
   plus an `author NOT IN (...)` fanout for authors the requester
   never mentioned.

   The new compound `(server_id, author, seq)` index helps **only** the
   per-known-author `(author = ? AND seq > ?)` predicates: each becomes
   a per-(server, author) range scan. The `author NOT IN (...)` half
   of the disjunct still requires a per-server scan with an in-list
   negation filter — the index gives the planner no key prefix to seek
   on, so half the non-empty branch's query stays a server-scan
   regardless of the new index.

   **Recommended fix:** restructure `sync_since` to enumerate
   "authors known locally but absent from `heads`" up-front (using
   `SELECT DISTINCT author FROM events WHERE server_id = ?`, i.e. the
   `known_authors` helper introduced in
   [Per-author tail query helpers](#per-author-tail-query-helpers)) and
   emit explicit `(author = ? AND seq > 0)` predicates for those
   authors instead of `author NOT IN (...)`. After this restructuring,
   *every* disjunct is `(author = ? AND seq > ?)` and the entire query
   is a union of per-(server, author) range scans on the new index.
   Without this restructuring the index addition is a partial win on
   the disjunctive query, not a full one.

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
above query per author and packing into `SyncBatchV2` envelopes.

The in-memory replay worker holds the same `EventDag` clients use:
per-author chains are `HashMap<EndpointId, Vec<EventHash>>` plus an
`events: HashMap<EventHash, Event>` map (see
[`crates/state/src/dag.rs:88-98`][dag]). Position in the per-author
`Vec` is the seq index, so
[`EventDag::events_since`][dag] (`crates/state/src/dag.rs:282-302`)
serves the per-author tail query as a `chain.iter().skip(known_max)`
linear scan rather than a BTreeMap range scan.

### Per-author tail query helpers

Today there is **no `EventStore` trait in `willow-state`** — the
state crate is pure (zero I/O) and the actual stores are concrete
types: `StorageEventStore` (SQLite, in `crates/storage/src/store.rs`)
and the in-memory `EventDag` (in `crates/state/src/dag.rs`) used by
clients and replay workers.

The per-author tail query already exists in both:

- `EventDag::events_since(&BTreeMap<EndpointId, u64>, Option<usize>)
  -> Vec<&Event>` ([`crates/state/src/dag.rs:282`][dag])
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
[`EventDag::heads_summary()`][dag] at `crates/state/src/dag.rs:267`)
to build the request.

## Termination + EOSE

`SyncBatchV2 { request_id, more: false }` is the canonical
end-of-stream marker. Upon receipt the client:

1. Applies the batch via the client's existing per-event entry point
   `try_insert_event(ctx, event)` (defined at
   [`crates/client/src/listeners.rs:120`][listeners-todo]; the existing
   batch loop calls it at `listeners.rs:276-278`), which wraps
   `EventDag::insert(event)` ([`crates/state/src/dag.rs:115`][dag-insert])
   and the `apply_incremental(state, &event)` step
   ([`crates/state/src/materialize.rs:61`][materialize]) through
   `ManagedDag`. Conceptually: validate per-author `seq` and `prev`,
   then advance `ServerState`. The internal `apply_event` (line 130 in
   `materialize.rs`) is private and not part of the public API.
2. Emits a `HistorySyncComplete` client event consumed by the UI per
   the EOSE spec (#214), which owns the user-visible "history loaded"
   signal and the `MessageType` slot 7 reservation.

**Relationship to existing `ClientEvent::SyncCompleted`.** The
`HistorySyncComplete` event is **not yet defined in code**; the EOSE
spec (#214) is currently unmerged. Today,
`ClientEvent::SyncCompleted { ops_applied }`
([`crates/client/src/events.rs:57`][client-events]) is emitted from
`listeners.rs:285-289` after **every** `SyncBatch` whose `count > 0`,
not just at end-of-stream. The "fire only on `more: false`" semantics
this spec proposes are a behavior change, not a no-op rename. Two
possible reconciliations, to be picked when #214 lands:

- **Option A — repurpose:** keep the existing `SyncCompleted` event
  but change its emission point to fire only on `more: false` (i.e.
  rename in spirit, not on the wire). Existing consumers in
  `crates/agent/src/notifications.rs` already gracefully handle a
  single end-of-stream signal.
- **Option B — additive:** introduce `HistorySyncComplete` as a
  separate `ClientEvent` variant emitted on `more: false`, and
  deprecate `SyncCompleted` (which becomes per-batch progress) over
  one release before removing it.

This spec deliberately does not redefine `HistorySyncComplete`; it
only triggers it. Pick A or B in the EOSE spec PR.

[client-events]: ../../crates/client/src/events.rs

[dag-insert]: ../../crates/state/src/dag.rs
[materialize]: ../../crates/state/src/materialize.rs

## Recovery — encrypted channel-key replay

Heads-based exchange recovers the public DAG including
`RotateChannelKey` events, but a late-joining peer still lacks the
**sealed key shares** needed to decrypt historical messages (sealed
shares are unicast, not in the DAG).

After the `SyncBatchV2 { more: false }` arrives, for every channel where
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
   gossip. New peers gain **additive** `WireMessage::SyncRequestV2 {
   request_id, heads, filter }` and `WireMessage::SyncBatchV2 {
   request_id, events, more }` variants. The legacy variants stay
   defined for one release cycle so old peers can still decode the
   envelope of any new message they don't understand and ignore just
   the unknown variant. No new `MessageType` slot is added.

2. **`WorkerRequest::Sync` / `WorkerResponse::SyncBatch`** in
   [`crates/common/src/worker_types.rs:88-125`][worker-types] — for
   client↔worker request/response. The `WorkerRequest::Sync` payload
   is **unchanged** (it already carries `HeadsSummary`).
   `WorkerResponse::SyncBatch` gains a `more: bool` field to support
   multi-envelope streaming; `request_id` already lives on the outer
   `WorkerWireMessage::Response` envelope and is not duplicated inside
   the payload.

**Why additive variants instead of bumping `PROTOCOL_VERSION`.**
`PROTOCOL_VERSION` lives in
[`crates/transport/src/lib.rs:30`][message-type] and is checked by
`Envelope::validate_version` at
[`crates/transport/src/lib.rs:120-128`][message-type]. Any version
mismatch causes the receiver to reject the **entire envelope** with
`UnsupportedVersion` — not just the inner message. Bumping
`PROTOCOL_VERSION` would therefore break **every** message kind
between an upgraded peer and an old peer, not just sync. This
contradicts a soft rollout. By keeping the bump out and instead
adding new `WireMessage` enum variants, the failure mode for an old
peer receiving a new `SyncRequestV2` is a bincode "unknown enum
variant" decode error confined to that one message — the envelope and
all other message kinds (`Event`, `TypingIndicator`, presence, voice,
worker requests, …) keep flowing. New peers handling an inbound
legacy `WireMessage::SyncRequest` either ignore it (if their peer is
new enough to send the V2 variant) or fall back to the legacy
500-event response while the rollout completes.

Cutover, then, is purely additive: ship `SyncRequestV2` /
`SyncBatchV2` together with a new responder, leave the legacy variant
handlers in place, and remove the legacy variants in a follow-up
release once a measured majority of peers have upgraded. Because the
legacy gossip path was already a 500-event heuristic dump, the
user-facing degradation during the overlap window is at most
"bootstrap stays on the legacy 500-event path until both peers are
upgraded," matching the status quo.

## Bandwidth and safety

- `SyncRequestV2.heads` size: `O(authors_known)` × ~72 bytes (32-byte
  `EndpointId` + 8-byte `u64` seq + 32-byte head hash). 1000 authors
  ≈ 72 KB — this **exceeds the 64 KiB gossip cap**. With the
  per-envelope budget of ~65,300 bytes (see [Wire protocol](#wire-protocol)),
  the threshold is ~900 known authors before a single `SyncRequestV2`
  no longer fits. Servers above that will need to chunk the request
  itself or fall back to a non-gossip ALPN. For all expected near-term
  deployments (single- or double-digit author counts per server) this
  is non-binding; the chunking design is deferred to the implementation
  PR and called out as an open question below.
- `SyncBatchV2` is bounded per envelope by the ~65,300-byte
  gossip-usable budget (see [Wire protocol](#wire-protocol)); total
  bytes are bounded by the actual diff, never by `|history|`.
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
| unit | New: `WireMessage::SyncRequestV2 { request_id, heads, filter }` and `SyncBatchV2 { request_id, events, more }` Serde round-trip; serialized envelope ≤ 64 KiB gossip cap; **legacy `SyncRequest` / `SyncBatch` variants still round-trip unchanged** so old peers stay compatible | `crates/common/src/wire.rs` (extend inline `#[cfg(test)]` module that already covers `SyncRequest` / `SyncBatch` round-trip) |
| unit | New: Measure actual framing overhead end-to-end (Envelope + WireMessage tag + SignedMessage) so the per-envelope byte budget constant is empirical, not estimated | `crates/common/src/wire.rs` |
| unit | New: Batching: 5 KB events × 100 authors split correctly across `SyncBatchV2` envelopes (each ≤ ~65,300-byte gossip-usable budget) with `more` flag and consistent `request_id` | `crates/state/src/sync.rs` or a new `crates/network/src/sync.rs` (location TBD by implementer) |
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
2. **Per-author rate-limiting on `SyncRequestV2`.** A malicious or
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
4. **Chunking the request itself.** A single `SyncRequestV2.heads`
   payload of `>~ 900` known authors crosses the 64 KiB gossip cap.
   Options: (a) split the request across multiple envelopes correlated
   by `request_id` and processed atomically by the responder; (b) move
   the entire sync exchange to a dedicated iroh ALPN protocol where
   gossip's framing limit doesn't apply; (c) accept the soft cap and
   defer until production deployments approach it. Option (c) is
   chosen for v1; (b) is the natural escape hatch.
