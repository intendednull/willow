# History sync — per-author sequence vector exchange

> **One-sentence summary:** Replace Willow's "replay last N events" bulk
> fetch with a 1-RTT per-author `(author → max seq)` vector exchange so
> sync transmits only events the client is missing, leveraging the
> per-author monotonic `seq` invariant already enforced by the DAG.

## Motivation

Clients connecting to a replay worker today receive the entire
1000-event ring buffer; clients connecting to a storage worker receive
a bounded `SyncRequest` page. Both re-transmit events the client
already has, and neither supports worker↔worker replication without
rebuilding the transfer path.

Willow's `Event` already carries a per-author monotonic `seq` enforced
by [`crates/state/src/event.rs:190-194`](../../crates/state/src/event.rs)
(every author maintains a strictly-increasing chain). That invariant
makes a Secure Scuttlebutt / EBT-style vector exchange trivially
correct: a client that knows it has events `1..=N` from author `A` only
needs `N+1..` to be complete, with no fingerprint negotiation.

This unlocks:

- Clients rejoining after downtime transfer only new events (1 RTT).
- Storage↔storage replication for geographic redundancy.
- Replay workers backfill from storage on boot without a full dump.
- Relay-mediated sync stays bounded in envelope count.

## Algorithm

A single round trip with streaming response.

**Phase 1 — Client request.** The client computes its current
`HashMap<EndpointId, u64>` of `(author → max seq)` across its local
event store, scoped to a `SyncFilter`, and sends a `SyncRequest`.

**Phase 2 — Responder stream.** The responder, for each author in its
own store whose `max seq > client.vector[author]` (or absent from
`client.vector`), streams the missing events in `(author, seq)`
ascending order. Authors not mentioned in the client vector default to
`known_max = 0` (the client has nothing for that author yet). Events
are batched into one or more `SyncBatch { events, more: true }`
envelopes, each fitting `MAX_DESER_SIZE = 256 KB`.

**Phase 3 — Termination.** The final envelope carries
`SyncBatch { events: …, more: false }`. The client emits a
`HistorySyncComplete` event for the UI per the EOSE spec (#214); see
[Termination + EOSE](#termination--eose).

Per-author monotonicity (DAG invariant at
[`crates/state/src/event.rs:190-194`](../../crates/state/src/event.rs))
guarantees that streaming `seq > known_max` in ascending order delivers
a contiguous chain with no gaps and no duplicates, so **no sort key
negotiation is required**. Authority events (e.g. `GrantPermission`,
`CreateChannel`) are authored just like chat events and ride along on
the same vector.

## Wire protocol

Add two variants to `MessageType` in
[`crates/transport/src/lib.rs:62-79`](../../crates/transport/src/lib.rs).
Slot 7 is reserved for `HistorySyncComplete` by the EOSE spec (#214),
so this spec claims slots 8 and 9:

```rust
MessageType::SyncRequest = 8,
MessageType::SyncBatch   = 9,
```

These names align with the existing worker design in
[`docs/specs/2026-03-27-worker-nodes-design.md`](2026-03-27-worker-nodes-design.md).

```rust
pub struct SyncRequest {
    pub vector: HashMap<EndpointId, u64>,
    pub filter: SyncFilter,
}

pub struct SyncBatch {
    pub events: Vec<Event>,
    pub more:   bool,
}

pub struct SyncFilter {
    pub server_id:   ServerId,
    pub channels:    Option<Vec<ChannelId>>,
    pub authors:     Option<Vec<EndpointId>>,
    pub event_kinds: Option<Vec<u8>>,
    pub since_ms:    Option<u64>,
}
```

Each `SyncBatch` payload (post-Serde, post-`Envelope`) is bounded by
`MAX_DESER_SIZE = 256 KB`
([`crates/transport/src/lib.rs:36`](../../crates/transport/src/lib.rs)).
Responders pack events greedily until the next event would overflow,
emit `SyncBatch { events, more: true }`, and continue. The final batch
sets `more: false`.

## Filter semantics

```rust
pub struct SyncFilter {
    pub server_id:   ServerId,                  // required
    pub channels:    Option<Vec<ChannelId>>,    // narrows chat-shaped kinds only
    pub authors:     Option<Vec<EndpointId>>,   // restrict to these authors
    pub event_kinds: Option<Vec<u8>>,           // EventKind tag whitelist
    pub since_ms:    Option<u64>,               // soft floor; see below
}
```

- Empty `Option`s = no restriction on that axis.
- `channels` only narrows chat-shaped `EventKind`s. Structural events
  (`GrantPermission`, `CreateChannel`, `RotateChannelKey`, …) ignore
  the channel filter so structure always reconciles fully.
- `since_ms` is **advisory** — server timestamps are the wall-clock
  hint embedded in the `Event` envelope and are display-only (see the
  [timestamp note](#a-note-on-timestamp_hint_ms)). The authoritative
  bound is the per-author `seq` vector. `since_ms` is intended only as
  a coarse pre-filter to reduce DB scan width on the responder.
- `event_kinds` uses the stable `EventKind` discriminant byte; see
  "Adding a new EventKind" in `CLAUDE.md`.

## A note on `timestamp_hint_ms`

The `timestamp_hint_ms` field on `Event` is **display-only** per
[`crates/state/src/event.rs:202-203`](../../crates/state/src/event.rs)
and intentionally not part of the sync protocol's correctness
guarantees. It is not used to order, dedupe, or terminate sync. The
per-author `seq` is the sole authoritative cursor.

## Integration points

| Pair | Direction | Notes |
|---|---|---|
| client ↔ replay worker | client initiates on connect | Replaces the "dump 1000 events" path; replay worker's ring buffer bounds the served set per author. |
| client ↔ storage worker | client initiates on connect / scrollback | Replaces paged `SyncRequest`; client's `(author → max seq)` vector skips already-known authors entirely. |
| replay ↔ storage | replay initiates on boot | Warm-start; replay worker streams missing chains from storage. |
| storage ↔ storage | either side | Geographic redundancy. Both peers MUST hold `SyncProvider` permission. |

The [Relay](../../crates/relay/src/lib.rs) remains a stateless bridge:
it forwards `SyncRequest` and `SyncBatch` envelopes unchanged.

## Storage requirements

Workers serve `SyncRequest` by querying per-author tails. SQLite
storage worker schema gains:

```sql
CREATE INDEX events_by_author_seq
    ON events (server_id, author, seq);
```

Hot query:

```sql
SELECT * FROM events
WHERE server_id = ?
  AND author    = ?
  AND seq       > ?
ORDER BY seq ASC
LIMIT ?;
```

The responder iterates the union of `(authors in client.vector ∪
authors known locally)` filtered by `SyncFilter.authors`, paging the
above query per author and packing into `SyncBatch` envelopes.

The in-memory replay worker maintains
`HashMap<EndpointId, BTreeMap<u64, Arc<Event>>>` to support the same
query shape with a `range((known_max + 1)..)` scan.

The `EventStore` trait (in `willow-state`) gains:

```rust
fn events_after(
    &self,
    server: ServerId,
    author: EndpointId,
    after_seq: u64,
    limit: usize,
) -> Vec<Event>;

fn author_max_seq(&self, server: ServerId, author: EndpointId) -> u64;

fn known_authors(&self, server: ServerId) -> Vec<EndpointId>;
```

Browser-only clients implement these against IndexedDB but only need
to *serve* if they ever respond to peer requests; pure leaf clients
just need `author_max_seq` to build their request vector.

## Termination + EOSE

`SyncBatch { more: false }` is the canonical end-of-stream marker.
Upon receipt the client:

1. Applies the final batch via `apply_event` per
   [`crates/state/src/materialize.rs`](../../crates/state/src/materialize.rs).
2. Emits a `HistorySyncComplete` client event consumed by the UI per
   the EOSE spec (#214), which owns the user-visible "history loaded"
   signal and the `MessageType` slot 7 reservation.

This spec deliberately does not redefine `HistorySyncComplete`; it
only triggers it.

## Recovery — encrypted channel-key replay

Per-author `seq` exchange recovers the public DAG including
`RotateChannelKey` events, but a late-joining peer still lacks the
**sealed key shares** needed to decrypt historical messages (sealed
shares are unicast, not in the DAG).

After the `SyncBatch { more: false }` arrives, for every channel where
the client now sees a `RotateChannelKey` epoch it cannot decrypt, it
emits the `RequestEpochKey { channel_id, epoch }` message defined by
spec #220. Any current channel member with the unwrapped epoch key
responds with a directed re-wrap addressed to the requester's
endpoint.

This is **out-of-band** to the vector exchange protocol — it rides on
the existing unicast envelope path. Vector sync surfaces the gap;
#220 fills it. See open question on placement.

## Migration

This spec supersedes the prior `(timestamp, hash)` Negentropy sketch
in this same file. The naming aligns with the worker design doc
[`docs/specs/2026-03-27-worker-nodes-design.md`](2026-03-27-worker-nodes-design.md),
so existing worker code paths using `SyncRequest`/`SyncBatch` remain
the integration target — only the payload shape changes.

Cutover: bump `MessageType` slot allocations together with #214's slot
7 reservation in a single transport release. There is no deployed
prior implementation to migrate from on the wire.

## Bandwidth and safety

- `SyncRequest.vector` size: `O(authors_known)` × 40 bytes
  (32-byte `EndpointId` + 8-byte `u64`). 1000 authors ≈ 40 KB; well
  within `MAX_DESER_SIZE`.
- `SyncBatch` is bounded per envelope; total bytes are bounded by the
  actual diff, never by `|history|`.
- Responders enforce a per-peer concurrency cap (e.g. 2 in-flight
  responses) and a per-session wall-clock budget to bound memory.
- Serving is gated by `SyncProvider`
  ([`crates/state/src/event.rs:21-33`](../../crates/state/src/event.rs)).
  Peers without `SyncProvider` MAY initiate but MUST refuse to serve.

## Testing

| Tier | Test | Location |
|---|---|---|
| unit | `events_after` returns contiguous `(author, seq)` ranges, empty when up-to-date | `crates/state/src/store.rs` |
| unit | `SyncRequest`/`SyncBatch` Serde round-trip; envelope size bound | `crates/transport/src/tests.rs` |
| unit | Batching: 5 KB events × 100 authors split correctly across `SyncBatch` envelopes with `more` flag | `crates/network/src/sync.rs` |
| integration | Three-peer convergence: A has authors {x:1..100}, B has {y:1..100}, C empty; C syncs from A then B and ends with both chains complete | `crates/network/tests/vector_sync.rs` |
| integration | Edge cases: empty store, client already up-to-date (zero-event response), single missing event, author entirely unknown to client | same file |
| integration | Authority events sync identically (server-create, grant, kick reach client without special-casing) | same file |
| E2E | Client offline reconnect transfers only the diff (byte-count assertion); `HistorySyncComplete` fires | `e2e/history-sync.spec.ts` |

## Future work / Appendix A — Negentropy fallback

The vector approach is optimal when divergence is per-author-tail
(the common case: a peer was offline, missed the last K events from
each active author). It is **not** optimal for *cross-author* set
divergence — e.g. two storage replicas that each independently dropped
a different middle slice of history. In that pathological case the
vector exchange would re-send full author tails when only an interior
gap is missing.

A future v2 may layer Negentropy / RBSR (NIP-77, Hoyte) over a
secondary `(author, seq)` keyspace as a fallback for replicas that
detect tail divergence. Implementation path: reuse iroh-docs'
existing range-based reconciliation primitives rather than porting
`rust-nostr/negentropy`. This is deferred until a concrete operational
need arises; for the `client ↔ worker` and `worker ↔ worker` cases
the vector approach is strictly sufficient given the DAG's per-author
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
   `(peer, requested_author_count)`, or rely solely on the
   `SyncProvider` admission gate plus the concurrency cap?
