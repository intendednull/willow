# Negentropy Range-Based Set Reconciliation for History Sync

> **One-sentence summary:** Replace Willow's "replay last N events" bulk
> fetch with a Negentropy-style range-based reconciliation protocol so
> sync cost scales with the symmetric difference between two peers'
> event sets, not the total history size.

## Motivation

Clients connecting to a replay worker today receive the entire
1000-event ring buffer; clients connecting to a storage worker
receive a bounded `SyncRequest` page. Both re-transmit events the
client already has, and neither supports worker↔worker replication
without rebuilding the transfer path.

Negentropy (NIP-77, Doug Hoyte) reconciles two sets in
`O(log(|A ⊕ B|))` round-trips by exchanging Merkle-style fingerprints
over recursively-bisected sorted ranges. This unlocks:

- Clients rejoining after downtime transfer only new events.
- Storage↔storage replication for geographic redundancy.
- Replay workers backfill from storage on boot without a full dump.
- Relay-mediated sync stays bounded in envelope count.

References: [NIP-77](https://github.com/nostr-protocol/nips/blob/master/77.md),
[hoytech/negentropy](https://github.com/hoytech/negentropy),
[rust-nostr/negentropy](https://github.com/rust-nostr/negentropy).

## Algorithm summary

Both sides sort their items by `(timestamp, id)` and exchange ranges
tagged with one of three modes — **Skip**, **Fingerprint** (a 16-byte
digest), or **IdList** (explicit IDs, used once the range is small).
Matching fingerprints become Skip; mismatched ones split at the
midpoint and recurse with finer fingerprints; IdLists diff directly
into "need" / "have" ID sets. Each range only transmits its upper
bound (lower bound is implicit). Convergence is logarithmic in the
symmetric difference. See
[NIP-77](https://github.com/nostr-protocol/nips/blob/master/77.md) for
the encoding and [negentropy-protocol-v1](https://github.com/hoytech/negentropy/blob/master/docs/negentropy-protocol-v1.md)
for the state machine.

## Sort key (primary design decision)

Willow's `Event` ([`crates/state/src/event.rs:185-210`](../../crates/state/src/event.rs))
carries a per-author `seq`, a `prev` hash, an application-assigned
`timestamp_hint_ms`, and a content-addressed `hash`. The sort-key
choice governs what queries the range index must answer.

| Sort key | Pros | Cons |
|---|---|---|
| `(timestamp_hint_ms, hash)` | Matches Negentropy's uint64+32-byte model verbatim; works for cross-author ranges; compatible with time-window filters | `timestamp_hint_ms` is advisory and attacker-controllable — a malicious author can place events at `t=0` to force excessive range recursion |
| `(author_pubkey, seq)` | Per-author chains are monotonic and authoritative; enables trivial vector-clock sync ("your last seq per author") | Breaks the logarithmic property — we'd reconcile one chain at a time, not one mixed stream, and cross-author ordering is lost |
| `(hlc_timestamp, hash)` | HLCs (see [`crates/messaging/src/hlc.rs`](../../crates/messaging/src/hlc.rs)) give monotonic causal order across authors; resilient to clock skew | HLCs only stamp `Message` events today; non-message `EventKind` variants would need HLC adoption first |
| `(author_pubkey, seq)` primary with `(ts, hash)` fallback | Cheap fast-path for peers that share most chains | Two protocols to implement and reason about |

**Recommendation: `(timestamp_hint_ms, hash)`** for the initial
implementation, matching NIP-77's `(uint64, 32-byte id)` item shape so
we can reuse [rust-nostr/negentropy](https://github.com/rust-nostr/negentropy)
with minimal glue. Two mitigations for adversarial timestamps:

1. Bucket by epoch-day at the top of the range tree so a flood of
   `t=0` events only harms reconciliation within one bucket.
2. Gate serving behind `SyncProvider`; abusive authors are kickable
   via governance.

A future v2 may layer an `(author, seq)` fast-path as a pre-filter.
**Flagged for reviewer:** see Open Questions.

## Fingerprint

Mirror Negentropy v1 exactly so we can interop with existing Rust
crates and reason by reference to the upstream proof:

```
fingerprint(ids) = truncate16( sha256( xor_sum(ids) || count_le ) )
```

- `ids` — 32-byte `EventHash` of every event in the range.
- `xor_sum` — byte-wise XOR over all `ids`, initial zero. Order-
  independent, which is why ranges can split cheaply.
- `count_le` — event count as a little-endian `u64`.
- `truncate16` — first 16 bytes of the SHA-256 output.

`EventHash` bytes are taken verbatim (big-endian as already hashed in
[`crates/state/src/hash.rs`](../../crates/state/src/hash.rs)); the
count is little-endian to match NIP-77 reference vectors.

## Wire protocol

Add four variants to `MessageType` in
[`crates/transport/src/lib.rs:62-79`](../../crates/transport/src/lib.rs):

```rust
MessageType::NegOpen  = 7,
MessageType::NegMsg   = 8,
MessageType::NegClose = 9,
MessageType::NegErr   = 10,
```

Payloads (Serde-encoded, wrapped in the existing `Envelope`):

| Variant | Fields |
|---|---|
| `NegOpen`  | `session_id: [u8; 16]`, `filter: SyncFilter`, `initial_msg: Vec<u8>` |
| `NegMsg`   | `session_id: [u8; 16]`, `msg: Vec<u8>` |
| `NegClose` | `session_id: [u8; 16]` |
| `NegErr`   | `session_id: [u8; 16]`, `reason: NegErrReason` |

`session_id` is a 16-byte random nonce chosen by the initiator.
`msg` bytes are the Negentropy v1 binary frame (protocol byte `0x61`
+ ranges). Each envelope must fit the 256 KB `MAX_DESER_SIZE` limit
([`crates/transport/src/lib.rs:36`](../../crates/transport/src/lib.rs));
responders split replies into multiple `NegMsg` envelopes as needed.
Hoyte's reference library already exposes `frameSizeLimit` for this.

`NegErrReason` variants:

| Reason | Meaning |
|---|---|
| `Blocked`     | Responder refuses the filter (too broad, rate-limited, missing `SyncProvider`). |
| `Closed`      | Session timed out server-side. |
| `Unsupported` | Protocol byte not recognised. |
| `BadMessage`  | Decoding error. |

## Filter semantics

A `SyncFilter` selects which events participate in the session:

```rust
pub struct SyncFilter {
    pub server_id:   ServerId,            // required
    pub authors:     Option<Vec<EndpointId>>,
    pub since_ms:    Option<u64>,          // inclusive
    pub until_ms:    Option<u64>,          // exclusive
    pub channels:    Option<Vec<ChannelId>>, // applies to Chat kinds
    pub kinds:       Option<Vec<EventKindTag>>,
}
```

- Empty `Option`s = no restriction on that axis.
- Responders MAY cap `since_ms`/`until_ms` and reply `Blocked` if the
  requested window exceeds policy.
- `channels` only narrows chat-shaped `EventKind`s; structural events
  (`GrantPermission`, `CreateChannel`, …) ignore the channel filter so
  that structure is always fully reconciled.
- `kinds` is a stable tag enum parallel to `EventKind`; see "Adding a
  new EventKind" in `CLAUDE.md`.

## Integration points

| Pair | Direction | Filter | Notes |
|---|---|---|---|
| client ↔ replay worker | client initiates on connect | `server_id`, `since_ms = client.last_seen` | Replaces the current "dump 1000 events" path; replay worker's ring buffer bounds the set. |
| client ↔ storage worker | client initiates on page/scroll | `server_id`, `since_ms`, `until_ms`, optional `channels` | Replaces paged `SyncRequest`; lets the client backfill a specific time window efficiently. |
| replay ↔ storage | replay initiates on boot | `server_id`, `since_ms = max(now - 24h, last_known)` | Warm-start so a fresh replay worker doesn't begin empty. |
| storage ↔ storage | either side | full `server_id`, `since_ms = last_replication_cursor` | Geographic redundancy; both peers must hold `SyncProvider` permission for the server. |

The [Relay](../../crates/relay/src/lib.rs) remains a stateless bridge:
it forwards `NegOpen`/`NegMsg`/`NegClose`/`NegErr` envelopes unchanged.
Reconciliation state lives in the participating peers.

## Storage requirements

Workers must expose a range-scannable index over the chosen sort key.
The SQLite-backed storage worker can add:

```sql
CREATE INDEX events_by_ts ON events (server_id, timestamp_hint_ms, hash);
```

and a streaming iterator bounded by `(ts_lo, hash_lo)..(ts_hi, hash_hi)`.
The in-memory replay worker keeps a `BTreeMap<(u64, EventHash), Arc<Event>>`
alongside its ring buffer. The `EventStore` trait (see `willow-state`)
gains:

```rust
fn range_scan(
    &self,
    server: ServerId,
    lo: (u64, EventHash),
    hi: (u64, EventHash),
) -> Box<dyn Iterator<Item = Event>>;
```

Clients running purely in-browser do not need to implement range scan
to act as initiators — they only need it to *serve* a session.

## Completion signalling

The pending "history sync EOSE" spec defines a single `SyncComplete`
signal that tells the UI "you have everything the peer intends to
send". A Negentropy session satisfies that contract naturally: once
both sides have emptied their outbound range queue, the initiator
sends `NegClose` and the client emits `SyncComplete`. No additional
end-of-stored-events marker is required.

## Bandwidth and safety

- Each `NegMsg` is capped by `MAX_DESER_SIZE` (256 KB); a single round
  trip carries at most ~16 000 fingerprints or ~8 000 IDs.
- Responders enforce a per-session time budget (~10s) and idle timeout
  (~30s), responding `NegErr(Closed)` on expiry.
- Responders enforce a per-peer concurrency cap (e.g. 4 open sessions)
  to bound memory; excess `NegOpen` gets `NegErr(Blocked)`.
- After reconciliation, missing events are fetched via the existing
  event-fetch path, not inline — the negentropy session only produces
  the "need" ID set.

## Interaction with SyncProvider

Serving a reconciliation session is gated by the `SyncProvider`
permission from [`crates/state/src/event.rs:21-33`](../../crates/state/src/event.rs).
A peer without `SyncProvider` MAY still *initiate* sessions (pulling
history is a right) but MUST refuse incoming `NegOpen` with
`NegErr(Blocked)`. Workers are granted `SyncProvider` the same way any
peer is — via a `GrantPermission` event from an admin. This keeps the
trust model unchanged.

## Encrypted channel keys

Channel-key events (`RotateChannelKey`) live in the same DAG as every
other event and therefore ride along inside negentropy sessions
automatically, subject to the filter. Per-recipient sealed key shares
are NOT part of the DAG and remain on their own point-to-point path.

## Testing

| Tier | Test | Location |
|---|---|---|
| unit | Fingerprint matches Hoyte reference vectors | `crates/network/src/negentropy/fingerprint.rs` |
| unit | `range_scan` iterator bounds (empty, single, inclusive/exclusive) | `crates/state/src/store.rs` |
| unit | Round-trip: two sets with known diff converge in ≤ expected rounds | `crates/network/src/negentropy/session.rs` |
| integration | Three-peer: A has {1..1000}, B has {500..1500}, C has {1..1500}, A↔C then B↔C converge | `crates/network/tests/negentropy_sync.rs` |
| integration | Edge cases: both empty, identical sets, fully disjoint, one side empty, diff = 1 | same file |
| E2E | Client reconnect after offline period transfers only new events (byte-count assertion) | `e2e/negentropy-sync.spec.ts` |

## Open questions

1. **Rust crate vs port.** `rust-nostr/negentropy` exists and targets
   NIP-77; does its API fit our `Event` type, and is its licence
   compatible? If not, do we fork or port from C++?
2. **Sort key.** `(timestamp_hint_ms, hash)` is proposed. Does the
   reviewer prefer `(author, seq)` vector sync for the
   client↔replay-worker path given per-author monotonicity?
3. **Per-author fast path.** Can we short-circuit with a single
   `max_seq_per_author` vector exchange *before* opening a negentropy
   session, falling back to negentropy only when seq gaps exist?
4. **Encrypted channel keys.** Do per-recipient sealed key shares
   belong inside the same DAG (and thus the same session) or in a
   parallel unicast flow? Current design keeps them unicast.
5. **SyncProvider as guard.** Should *initiating* a session also
   require a permission (e.g. member of the server) to prevent a
   stranger from probing existence? Today any peer can initiate.
6. **Timestamp adversary.** If an author stuffs events at `t=0`, the
   first range bucket balloons. Is the per-epoch bucketing in §Sort
   key sufficient, or do we need a secondary keying scheme for
   pathological timestamp distributions?
