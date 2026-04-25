# Relay Discovery — composing iroh pkarr with capability negotiation

> **One-sentence summary:** Willow does **not** introduce a new
> `EventKind::RelayList`. NodeId → addresses is solved by iroh's
> existing pkarr discovery, EndpointId → role/capability is solved by
> PR #215's `/.well-known/willow` capability document, and EndpointId
> → trust is solved by the `SyncProvider` grants already in the server
> event DAG. This spec is the explicit composition of those three
> layers, not a new event kind.

## Status — what changed

This spec originally proposed an `EventKind::RelayList` event with
replaceable per-author semantics, modelled on Nostr's NIP-65 outbox.
Two rounds of review (recorded on intendednull/willow#221) converged
on the conclusion that the proposed event is unnecessary and would
introduce a new state-machine concept (replaceable kinds, breaking
the current single-pass `apply_event` replay) for capabilities iroh
and PR #215 already provide.

**This revision drops `EventKind::RelayList` entirely.** The spec
now describes the composition of existing primitives. Sections that
previously specified the event have been replaced; sections that
remain useful (privacy, share-link bootstrap, anti-centralization
realism) have been retained and rewritten to fit.

## Motivation

Willow's share links today carry a single bootstrap relay URL (see
`docs/specs/2026-03-27-shareable-join-links-design.md`) and
`IrohNetwork` otherwise relies on a hardcoded relay list baked into
`crates/network/src/iroh.rs`. The failure modes:

1. **No graceful migration.** An owner who moves their worker fleet
   (e.g. Linode → Fly.io) must reissue every share link if the URL is
   the only addressing mechanism.
2. **No worker discovery.** A server can have a `SyncProvider`-granted
   replay or storage worker, but a joining client has no way to learn
   its network address short of the owner encoding it manually.
3. **Bootstrap fragility.** A single hardcoded relay is a single
   point of failure for new joiners.

These are real problems, but the solutions already exist in our
dependency graph. We just need to compose them.

## Architecture: three independent layers

| Layer | Mechanism | Source of truth |
|-------|-----------|-----------------|
| `NodeId` → addresses | iroh pkarr (signed DNS packets over BitTorrent mainline DHT) | DHT, signed by the node's own key |
| `EndpointId` → role / capabilities | PR #215 `/.well-known/willow` capability document | served by the endpoint itself over HTTPS |
| `EndpointId` → trusted to sync | `SyncProvider` grant in the server event DAG | `crates/state/src/event.rs` + `apply_event` |

These three layers are independent: each has its own trust anchor,
its own update cadence, and its own failure mode. The job of this
spec is to nail down how a Willow client traverses them in order.

## Layer 1: pkarr for NodeId → addresses

iroh ships [pkarr-based node discovery](https://www.iroh.computer/blog/iroh-global-node-discovery)
out of the box (see `iroh-pkarr-node-discovery`). A Willow node's
`NodeId` is its long-term Ed25519 public key; the node publishes a
signed DNS packet to the BitTorrent mainline DHT containing its
current relay URL and direct addresses, and any client that knows
the `NodeId` can resolve those addresses by querying the DHT.

Implications for Willow:

- **Willow does not need to encode addresses anywhere durable.**
  Every `EndpointId` in the server DAG (peers with `SyncProvider`
  grants, the inviter, ordinary members) is already a `NodeId`.
  Clients resolve those `NodeId`s on demand.
- **Owner relay migration is automatic.** Moving from one hosting
  provider to another causes the operator's iroh node to republish
  its pkarr packet. Existing clients pick up the new addresses on
  their next discovery cycle. No share-link reissue.
- **No new wire format, no new state, no new replay semantics.**
- **Failure mode.** DHT queries can fail in restricted networks. The
  fallback is the same set of hardcoded bootstrap relays already
  baked into `IrohNetwork` — clients try direct pkarr first, fall
  back to the bootstrap fleet for relay-mediated connection.

## Layer 2: PR #215 capability doc for role/capability

PR #215 ([capability negotiation](https://github.com/intendednull/willow/pull/215))
introduces a `/.well-known/willow` document served by every endpoint
that runs Willow infrastructure. It describes what the endpoint
runs: `Relay`, `ReplayWorker`, `StorageWorker`, supported protocol
versions, rate-limit hints, and operator metadata.

This is the right place — and the only place — for role/capability
information:

- It's served by the endpoint itself, so an endpoint can change its
  role at any time without rewriting state held by other peers.
- It's not signed into the DAG, so a relay rebooting in `Relay`-only
  mode after running as `StorageWorker` doesn't require a state
  mutation.
- It composes naturally with HTTPS-backed PKI for transport trust;
  Willow-specific authority still flows through `SyncProvider`.

This is structurally identical to what Matrix's
`/.well-known/matrix/server` does for federation discovery, which is
the most-deployed federated-discovery cascade on the internet. We
should copy the shape.

## Layer 3: `SyncProvider` for trust

Authority is unchanged. `EventKind::GrantPermission { permission: SyncProvider }`
in `crates/state/src/event.rs` and the enforcement site in
`apply_event` (`crates/state/src/materialize.rs`) remain the only
mechanism for declaring "this `EndpointId` may serve this server's
state." The composition rule is straightforward:

> An endpoint is a candidate sync source for server S if and only if
> its `EndpointId` is the target of a live (un-revoked)
> `SyncProvider` grant in S's event DAG.

Resolution of that `EndpointId` to addresses happens via Layer 1.
Resolution of "what does it run" happens via Layer 2. Trust is
strictly Layer 3.

## Per-server "trusted workers" hint — already present

The original spec proposed a per-server `RelayList` to nominate the
workers a server prefers. This is **already encoded** by the existing
set of `SyncProvider` grants in the server DAG. A client computing
"who carries this server's state" enumerates the grants — the answer
is the list of `EndpointId`s authorized to sync. No new event kind
required, no list of URLs, no per-entry weight.

If we ever want a softer hint (e.g. "prefer this worker over that
one"), the right place to add it is on the existing `GrantPermission`
record (an optional `priority: u8`), not as a parallel event stream.
That extension is out of scope for this spec.

## Bootstrap flow for share links

`JoinToken` (per `docs/specs/2026-03-27-shareable-join-links-design.md`)
currently carries `inviter_peer_id`, `server_id`, `link_id`,
`server_name`, `inviter_name`. **It carries no addressing
information**, implicitly relying on the hardcoded relay fleet.

This spec proposes a one-line extension: add a small list of
bootstrap **`NodeId`s** (not URLs):

```rust
JoinToken {
    inviter_peer_id: String,
    server_id: String,
    link_id: String,
    server_name: String,
    inviter_name: String,
    bootstrap_node_ids: Vec<NodeId>, // 2-3 entries; resolved via pkarr
}
```

Joining flow:

1. Client decodes `JoinToken` and obtains `bootstrap_node_ids` and
   `inviter_peer_id`.
2. For each bootstrap `NodeId` (and the inviter's `NodeId`), resolve
   addresses via iroh pkarr.
3. Connect to the first responsive endpoint and complete the
   `JoinRequest` / `JoinResponse` handshake from the share-link spec.
4. Replay the server DAG and read the set of `SyncProvider` grants.
5. Resolve those `EndpointId`s via pkarr; fetch their capability
   docs (Layer 2) to learn which run replay vs storage roles; connect
   to the top N (default N=3) prioritised by latency.

Steps 1-3 use only Layer 1. Step 4 uses Layer 3. Step 5 uses Layers
1 + 2 + 3 in combination. **At no point does the client read a
"relay list event."**

## Privacy

The original concern — that a public outbox doxxes a server's worker
fleet — does not arise in this design. Worker `EndpointId`s appear
only inside the server's own event DAG (via `SyncProvider` grants),
which is encrypted to members under PR #220's epoch keys. Public
discovery resolves `NodeId`s via pkarr packets the operator chooses
to publish, and only members ever see the server-membership list.

## Composition notes

- **PR #220 (epoch rotation).** No interaction. Per-server worker
  lists are simply the existing `SyncProvider` grants inside the DAG;
  they ride whatever encryption already protects server events.
- **PR #217 (bech32 identifiers).** Pkarr packets contain raw
  `NodeId` bytes, not bech32. Display surfaces (UI, logs, share-link
  text) use the `wpeer` / `wrelay` HRPs from #217 for human-readable
  rendering. There is no wire-format collision.
- **PR #215 (capability negotiation).** This spec depends on #215.
  It is consumed, not duplicated.

## Anti-centralization — honest scope

The original spec claimed protocol-level mitigations would prevent
relay centralization. The empirical record from comparable systems
contradicts that:

- **Matrix:** matrix.org dominates federation traffic despite ~10K
  federateable servers (TWIM 2025-10-03: only 28.6% of servers
  publish room directories).
- **ActivityPub:** mastodon.social retains years-running dominance
  discussion.
- **Nostr:** damus.relay and a handful of others dominate; the
  Nostrify outbox docs explicitly cite "everyone picking the same
  few relays" as the unsolved problem.
- **SSB:** Pubs centralized; the project moved to Rooms (no feed
  storage) precisely because the load model invited centralization.

Users pick the most-available endpoint regardless of protocol
affordances. **This spec drops anti-centralization as a claim.** The
honest scope is "graceful migration + worker discovery", which is
what the iroh-pkarr + #215 composition delivers.

Client-side ranking heuristics (latency-weighted retry, dead-server
backoff, periodic re-ranking) belong in `willow-client` as ordinary
quality-of-service code. They are not protocol features.

## Liveness — not a Willow concern

Liveness is a TCP/QUIC connection-attempt concern (the OS gives us
connect timeouts) plus a client retry-and-decay concern. Encoding
liveness into a signed DAG event would be a category error: it
would write transient network state into the permanent log, with no
authoritative prober to serve as ground truth. **No
`EventKind::RelayLiveness` will be introduced.** The original open
question is closed.

## Mandatory publication — voluntary

Should peers with a live `SyncProvider` grant be required to
participate in any discovery mechanism? **No.** Operators may
legitimately run private/mesh deployments. Discovery participation
remains voluntary; pkarr publication is opt-in (and is iroh's
default for nodes that have a relay configured). The original open
question is closed.

## Tests

| Layer | Test | Where |
|-------|------|-------|
| Client | Resolve owner `NodeId` via stub pkarr, connect via discovered address | `crates/client/src/lib.rs` tests |
| Client | Enumerate `SyncProvider` grants from a replayed DAG; resolve each via pkarr; rank by latency | `crates/client/src/lib.rs` tests |
| Client | Capability-doc fetch is gated by trust check (refuse to use a non-`SyncProvider` endpoint as authoritative) | `crates/client/src/lib.rs` tests |
| Integration | Share-link join with extended `JoinToken` carrying `bootstrap_node_ids`, fallback when first bootstrap is unreachable | `e2e/multi-peer-sync.spec.ts` |
| Integration | Owner migrates relay provider mid-session; existing clients re-resolve via pkarr without restart | `e2e/multi-peer-sync.spec.ts` |

State-machine tests are deliberately absent: no new `EventKind` is
introduced, so the state machine is unchanged.

## Open questions

Most of the original open questions are resolved by dropping the
event kind. What remains:

1. **Bootstrap NodeId signing.** Should the `bootstrap_node_ids`
   carried in `JoinToken` be signed by the inviter's key (so a
   tampered share link can be detected at the time of join, before
   any peer connection)? The token itself is base64-packed; we
   could either sign the whole token with the inviter's key or rely
   on first-connection identity verification. Concrete design
   pending input on `JoinToken`'s threat model.
2. **Pkarr fallback policy.** When the DHT is unreachable (e.g.
   restrictive corporate network), should the client fall back to
   resolving via the hardcoded bootstrap relays' caches, or fail
   fast and surface the error to the user? The behaviour matters
   for share-link UX and is genuinely unsettled.

These are the only questions worth keeping open. Replaceable-event
semantics, multi-device writes to the same author chain, anti-
centralization algorithms, and `RelayLiveness` are all rendered
moot by the layering above.

## References

- iroh pkarr: <https://www.iroh.computer/blog/iroh-global-node-discovery>
- iroh DNS dial-by-NodeID: <https://www.iroh.computer/blog/iroh-dns>
- PR #215: capability negotiation
- PR #217: bech32 identifiers
- PR #220: epoch key rotation
- Matrix Server-Server API: <https://spec.matrix.org/latest/server-server-api/>
- did:plc spec: <https://github.com/did-method-plc/did-method-plc>
- SSB Rooms: <https://www.manyver.se/blog/announcing-ssb-rooms/>
- Existing share-link design: `docs/specs/2026-03-27-shareable-join-links-design.md`
