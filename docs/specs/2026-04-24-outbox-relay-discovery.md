# Outbox-style Relay Discovery

> **One-sentence summary:** each peer publishes a signed, replaceable
> `RelayList` event advertising which relays and workers carry its
> data, so clients can discover the rest of the network from a single
> bootstrap URL — no global DHT, no hardcoded relay fleet, and no trust
> promotion beyond the existing `SyncProvider` grant.

## Motivation

Willow's share links today carry a single bootstrap relay URL (see
`docs/specs/2026-03-27-shareable-join-links-design.md`) and
`IrohNetwork` otherwise relies on a hardcoded relay list baked into
`crates/network/src/iroh.rs`. This has three failure modes:

1. **Centralization.** If the hardcoded relay is down, new joiners
   cannot complete the `JoinRequest`/`JoinResponse` handshake.
2. **No graceful migration.** An owner who moves their worker fleet
   (e.g. Linode → Fly.io) must reissue every share link.
3. **No worker discovery.** A server can have a `SyncProvider`-granted
   replay or storage worker, but a joining client has no way to learn
   its network address short of the owner encoding it manually.

Nostr solves an analogous problem with NIP-65 "outbox model" relay
lists: each user publishes a `kind:10002` replaceable event listing
their read/write relays. Willow already has the substrate to do
this — signed events on `_willow_profiles`, an HLC for ordering,
and a cryptographic trust chain rooted at the server owner.

## Relationship to `SyncProvider`

The outbox is a **routing hint, not a capability grant.** Authority
still flows exclusively through `EventKind::GrantPermission { permission: SyncProvider }`
as defined in `crates/state/src/event.rs:22` and enforced by
`apply_event` in `crates/state/src/materialize.rs:130`. A `RelayList`
advertising `wss://evil.example` as a replay worker does not make
that host trusted — it's only a *candidate address* that clients
attempt to connect to. Once connected, the usual authority check
applies: the peer at that address must sign state updates with a key
that has a live `SyncProvider` grant in the server DAG.

| Layer | Who decides | Enforced where |
|-------|-------------|----------------|
| "Which addresses might serve this server?" | Any peer's `RelayList` | Client connection code (advisory) |
| "Is this peer allowed to sync state?" | Owner / admin via `GrantPermission` | `required_permission()` + `apply_event` |
| "Should I prefer this address?" | Client heuristics (weight, latency, trust intersection) | `willow-client` discovery loop |

Clients **SHOULD** prefer outbox entries whose advertised `EndpointId`
is also the target of a current `SyncProvider` grant in the server's
DAG. This is what turns the outbox from a bare hint into a
trust-weighted routing table.

## New event kind

Add a variant to `EventKind` (`crates/state/src/event.rs:70`):

```rust
EventKind::RelayList {
    entries: Vec<RelayEntry>,
    replaces: Option<EventHash>, // previous RelayList by same author
}

pub struct RelayEntry {
    /// Wire address. Accepts `ws://`, `wss://`, `tcp://`, `quic://`,
    /// or a bare iroh `NodeId` ("iroh:<base32-nodeid>").
    pub url: String,
    /// What direction the relay is useful for (NIP-65 analog).
    pub role: RelayRole,
    /// What the remote peer actually runs.
    pub capability: RelayCapability,
    /// Client-side ranking hint. 0 = lowest, 255 = highest. None = default.
    pub weight: Option<u8>,
}

pub enum RelayRole { Read, Write, ReadWrite }
pub enum RelayCapability { Relay, ReplayWorker, StorageWorker }
```

Authority tier: **unrestricted.** Any member may publish their own
`RelayList`. Trust comes from the signature + existing
`SyncProvider` grants, not from membership in a new permission tier.
Add to the intentionally-unrestricted comment block in
`required_permission()` (`crates/state/src/materialize.rs:283`).

## Replaceable semantics

NIP-65 treats `kind:10002` as a replaceable event: the newest
`created_at` from a given author supersedes older ones. Willow's
per-author Merkle DAG already gives us something stronger — a
monotonic `seq` (`crates/state/src/event.rs:191`) plus HLC-ordered
`timestamp_hint_ms`. The rule is:

> For any `(author, EventKind::RelayList)`, the event with the
> **highest `seq`** in that author's chain is authoritative. Older
> `RelayList` events are retained in the DAG for audit but are
> ignored by `apply_mutation`.

This is the first "replaceable kind" in `willow-state`. **Flag for
reviewers:** we either (a) introduce a generic `ReplaceableKind` trait
keyed on `(author, discriminant)` so future additions (e.g.
`SetProfile` arguably belongs here) share the machinery, or (b) just
add a `state.relay_lists: HashMap<EndpointId, RelayList>` and special-
case it. Option (a) is cleaner but a wider change; option (b) is the
minimum viable path and is what this spec recommends shipping first.

Because `seq` is totally ordered within an author chain, replaceable
semantics in Willow are **not** vulnerable to the clock-skew
last-write-wins pathology that NIP-65 inherits.

## Scope: per-peer-global vs per-server

| Axis | Per-peer-global (`_willow_profiles`) | Per-server (inside server DAG) |
|------|--------------------------------------|--------------------------------|
| Discoverable from a share link alone | Yes — single lookup by owner PeerId | No — client must already have genesis event |
| Doxxes your worker fleet to the world | Yes | No — visible only to members |
| Follows key rotations | Yes, tied to peer identity | Yes, but requires DAG access |
| Natural fit for `SyncProvider` nominations | No — grants are per-server | Yes |
| Bootstrap-before-membership usable | Yes | No |

**Recommendation:** publish two `RelayList` streams.

1. A **public per-peer-global** `RelayList` on `_willow_profiles`,
   used by joiners who only have a share link and need to find the
   owner. Entries here are treated as hints about the *peer*, not
   about any specific server.
2. A **per-server** `RelayList` inside the server DAG (so it merges
   with the rest of server state), used to nominate the server's
   `SyncProvider` workers. Members read this after joining.

The per-server stream MAY be published encrypted (see Privacy
below); the per-peer-global stream is inherently public because its
job is to bootstrap strangers.

## Discovery flow

New joiner clicks a share link (`JoinToken` per
`2026-03-27-shareable-join-links-design.md`):

1. Connect to the bootstrap relay embedded in the link.
2. Subscribe to `_willow_profiles` (`crates/network/src/topics.rs:29`)
   and request the owner's latest `RelayList` event by
   `(author = inviter_peer_id, kind = RelayList)`.
3. In parallel, request the server DAG from the bootstrap relay and
   replay to obtain the set of current `SyncProvider` grants.
4. **Intersect:** compute the set of `RelayEntry` whose advertised
   `EndpointId` also holds a live `SyncProvider` grant. These are
   trust-weighted candidates.
5. Rank candidates by `(is_trusted, weight, observed_latency)` and
   connect to the top N (default N=3, following NIP-65's "2–4 each"
   guidance).
6. Subscribe to `_willow_profiles` with a long-lived watch so that
   new `RelayList` events from the owner (key rotation, fleet
   migration) update the candidate set without a restart.
7. When a peer publishes a new message, `willow-network` MUST push to
   its *own* write relays and also opportunistically to the
   recipient's read relays — mirroring NIP-65's "publish to both
   sides" rule so discovery spreads without a central indexer.

## Liveness

Two options, analogous to NIP-66:

- **Client-side probing (ship first).** Clients track connect
  success/failure per entry in memory, decay weights on failure, and
  write nothing back to the network. Simple, private, fully
  distributed. Downside: every joiner re-learns the hard way.
- **`EventKind::RelayLiveness { target: EndpointId, observed_ms: u64, ok: bool }`.**
  Signed attestations from *any* peer about any relay. Consumers
  weight attestations by signer trust. Same trap NIP-66 hit: not
  many people run the checker. Recommended only if client-side
  probing proves insufficient in practice.

This spec recommends **client-side probing in v1** and deferring
`RelayLiveness` until there's evidence we need it.

## Privacy

Publishing a `RelayList` doxxes which relays you use — NIP-65 has
been criticized for this, since public lists enable targeted DoS.
Mitigations, in order of strength:

- **Per-server encryption.** The per-server `RelayList` is encrypted
  to the current channel-key membership set
  (`willow-crypto::seal_content`), so only admitted members see
  worker addresses. Non-members joining via share link never see it.
- **Public outbox is intentionally minimal.** The per-peer-global
  `RelayList` on `_willow_profiles` SHOULD list only public relays
  (the willow.intendednull.com fleet, community relays) — not
  self-hosted workers. Members discover private workers after they
  join.
- **Rotation.** Because replaceable semantics are free, users can
  churn their `RelayList` on any timescale without side effects.

## Bootstrap

The first fetch still needs a starting address. Share links
continue to carry a bootstrap relay URL per
`2026-03-27-shareable-join-links-design.md`. Once any one relay is
reachable, the outbox takes over immediately. Share links MAY carry
a small list (2–3) of bootstrap relays so a single down relay does
not block joining — a one-line extension of `JoinToken`.

## Anti-centralization

NIP-65 converged on three de-facto indexer relays (relay.damus.io,
nos.lol, purplepag.es). Client heuristics to resist this:

- **Never greedy-pick the most common entry.** If 80% of observed
  `RelayList`s cite the same URL, *reduce* its weight on the margin.
- **Weight by observed latency, not popularity.** A nearby relay
  with 20ms RTT outranks a global giant with 200ms RTT.
- **Diversify by operator.** Prefer candidates whose advertised
  `EndpointId` is distinct from those already connected.
- **Rotate periodically.** Every N hours, re-rank and drop the
  lowest-performing connection even if it still works, to give other
  relays a chance.

These heuristics live in `willow-client`, not in the state machine —
the DAG stays neutral.

## Tests

| Layer | Test | Where |
|-------|------|-------|
| State | Two `RelayList` events from same author → only the higher-`seq` one is reflected in `ServerState` / profile view | `crates/state/src/tests.rs` |
| State | `RelayList` from non-member is still applied (unrestricted) and does not fail `required_permission` | `crates/state/src/tests.rs` |
| Client | Discovery flow: stub `_willow_profiles`, feed a `RelayList`, assert top-N connection attempts | `crates/client/src/lib.rs` tests |
| Client | Intersection: `RelayList` entry with no `SyncProvider` grant is ranked below one that has it | `crates/client/src/lib.rs` tests |
| Integration | Share-link join where bootstrap relay serves only `RelayList` + DAG and client completes handshake via discovered worker | `e2e/multi-peer-sync.spec.ts` |

## Open questions

1. **Both scopes or one?** Should we ship only per-peer-global first
   and defer per-server until workers are a real feature, or build
   both simultaneously since the schema is the same?
2. **Public vs encrypted.** Is the privacy cost of a public
   per-peer-global `RelayList` acceptable, or should even the
   bootstrap list be encrypted to a rotating "join-link key"
   carried in the share URL?
3. **Liveness.** Client probing v1, or bite the bullet and design
   `EventKind::RelayLiveness` now so the schema is stable?
4. **Anti-centralization floor.** Should the client *refuse* to
   connect to a relay that appears in >X% of observed `RelayList`s,
   or only demote it?
5. **Mandatory publication.** Should peers with a live
   `SyncProvider` grant be *required* to publish a `RelayList`
   (enforced how? a warning? a soft demotion?), or is publication
   purely opt-in?
6. **Replaceable-event concept.** Is `RelayList` the forcing
   function to introduce a general replaceable-kinds mechanism in
   `willow-state` (retrofit `SetProfile`, anticipate future
   per-author settings), or is a one-off `relay_lists` map on
   `ServerState` enough for now?
