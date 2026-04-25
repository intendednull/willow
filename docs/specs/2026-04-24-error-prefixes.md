# Machine-Readable Wire-Rejection Reasons

> **One-sentence summary:** introduce a typed `WireRejectReason` enum,
> carried in a new `WireMessage::Reject(RejectPayload)` variant in
> `willow-common`, so peers can react programmatically to rejections —
> retry on rate-limit, re-auth on `AuthRequired`, drop silently on
> `Duplicate`, surface a permission prompt on `PermissionDenied` —
> instead of matching on free-form error strings that are only fit
> for logs.

## Motivation

Every rejection path in Willow today ends as a log line
([`relay/lib.rs:388`](../../crates/relay/src/lib.rs)), a stringly
reason ([`materialize.rs:111`](../../crates/state/src/materialize.rs)),
or a typed error that never leaves the rejecting node
([`dag.rs:16`](../../crates/state/src/dag.rs),
 [`identity/lib.rs:45`](../../crates/identity/src/lib.rs)). Senders
learn nothing, so clients treat every failure identically — no
auto-retry, no re-auth, no duplicate back-pressure.

NIP-01 solves this for Nostr by prefixing OK/CLOSED messages with a
single-word machine-readable tag (`duplicate:`, `pow:`, `blocked:`,
`rate-limited:`, `invalid:`, `restricted:`, `mute:`, `auth-required:`,
`error:`). Willow's wire format is binary bincode
([`pack` at `transport/lib.rs:138`](../../crates/transport/src/lib.rs)),
so we can carry a **typed enum** — same machine-readability as Nostr,
plus compile-time exhaustive matching and structured payloads like
`retry_after_ms` or the violated `Permission`.

Concrete cases the new reason must cover, each a real rejection site:

- Relay topic-announce with an invalid string — dropped silently today,
  so the sender keeps republishing
  ([`relay/lib.rs:388`](../../crates/relay/src/lib.rs)).
- `InsertError::Duplicate` on a re-gossiped event — sender should stop
  retransmitting ([`dag.rs:34`](../../crates/state/src/dag.rs)).
- `check_permission` rejects for lack of `SendMessages` — UI should
  surface the block, not spin on "sending…"
  ([`materialize.rs:117`](../../crates/state/src/materialize.rs)).
- `validate_version` mismatch — prompt an upgrade
  ([`transport/lib.rs:120`](../../crates/transport/src/lib.rs)).
- `unpack` above `MAX_DESER_SIZE` — chunk, don't retry
  ([`transport/lib.rs:155`](../../crates/transport/src/lib.rs)).
- `SignedMessage::verify` fail (i.e. `IdentityError::InvalidSignature`
  surfaced through `unpack` / `unpack_profile`) on a forged envelope —
  re-sign
  ([`identity/lib.rs:391`](../../crates/identity/src/lib.rs)).

## Proposed format

The enum lives in `willow-common` rather than `willow-transport`. The
fields require `Permission` and `EventHash` (defined in
`willow-state`) and `TopicId` (re-exported from iroh-gossip via
`willow-network`); putting the type in `willow-transport` would
introduce a `transport → state` cycle that the existing dependency
graph forbids (`state → transport`, `client → state`,
`common → state`). `willow-common` already depends on `willow-state`,
`willow-identity`, and `willow-transport`
([`crates/common/Cargo.toml`](../../crates/common/Cargo.toml)) and is
the natural home for any type that mixes state-layer and transport-
layer references — exactly the rationale PR #214 used to land
`WireMessage::HistorySyncComplete` in this same crate
(see [`2026-04-24-history-sync-eose.md`](2026-04-24-history-sync-eose.md)).

The new types are added to `crates/common/src/wire.rs`:

```rust
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireRejectReason {
    Duplicate,
    Invalid(String),
    RateLimited { retry_after_ms: u64 },
    PermissionDenied(willow_state::Permission),
    ParentHashMismatch { expected: willow_state::EventHash, actual: willow_state::EventHash },
    SeqGap { expected: u64, actual: u64 },
    SignatureInvalid,
    PayloadTooLarge { limit: u64, actual: u64 },
    UnsupportedVersion { expected: u16, actual: u16 },
    AuthRequired,
    Restricted(String),               // authed but not authorized
    NotSyncProvider,                  // relay-specific: not granted SyncProvider
    UnknownTopic,
    TopicInvalid(String),
    Capacity,                         // relay MAX_TOPICS or similar cap hit
    ServerError,                      // generic last-resort
}
```

Each variant maps to exactly one code path that exists today; the
spec is a rename-and-surface exercise, not a behavior change.

Note that the existing `TransportError::UnsupportedVersion` and
`InsertError::SeqGap` / `InsertError::PrevMismatch` use a `got` field
([`transport/lib.rs:53`](../../crates/transport/src/lib.rs),
[`dag.rs:22-32`](../../crates/state/src/dag.rs)). The new
`WireRejectReason` variants rename `got` → `actual` to match the
`{ expected, actual }` convention used in the rest of the
proposed enum (e.g. `PayloadTooLarge { limit, actual }`). The
existing internal types may either be renamed in the same change
or kept as-is and translated at the boundary.

## Wire envelope

Adding `MessageType::Reject = 7` to
[`crates/transport/src/lib.rs:64`](../../crates/transport/src/lib.rs)
would not surface to any consumer: every gossipsub frame in production
is packed under `MessageType::Channel` and dispatched through the
single `WireMessage` enum at
[`crates/common/src/wire.rs:13`](../../crates/common/src/wire.rs).
`pack_wire` / `unpack_wire`
([`crates/common/src/wire.rs:105-120`](../../crates/common/src/wire.rs))
hard-code `MessageType::Channel` on both sides; receive paths
(e.g. `topic_announce_listener` at
[`crates/relay/src/lib.rs:382-386`](../../crates/relay/src/lib.rs))
match on `WireMessage` variants after `unpack_wire`, never on the
underlying `MessageType` discriminant. The same constraint forced
PR #214 to land `HistorySyncComplete` as a `WireMessage` variant
rather than a new `MessageType`; we follow that precedent here.

We therefore add a new variant to `WireMessage` in
`crates/common/src/wire.rs`:

```rust
pub enum WireMessage {
    // ... existing variants ...

    /// A peer is informing the sender that one of their previously
    /// gossiped events or envelopes has been rejected, with a
    /// machine-readable reason.
    Reject(RejectPayload),
}
```

The payload carries the reason plus enough context for the receiver
to correlate the rejection with the event it sent:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectPayload {
    pub reason: WireRejectReason,
    pub context: RejectContext,
    pub human: Option<String>,        // logs/UI only; never parsed
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RejectContext {
    Event(willow_state::EventHash),
    Topic([u8; 32]),                  // raw TopicId bytes — same shape
                                      // used by HistorySyncComplete
                                      // to avoid an iroh-gossip dep
                                      // leaking into willow-common
    Envelope,                         // predates any event hash
}
```

`human` is the existing free-form `Display` output of the underlying
error (e.g. the current `InsertError` message); the reason variant is
canonical, the string is never matched on.

The rejecting peer's identity is **not** carried in the payload —
exactly like every other `WireMessage`, the reject rides the same
Ed25519-signed envelope built by `pack_wire` / verified by
`unpack_wire`, so the receiver derives the rejector's `EndpointId`
from the verified signer at unpack time. This forecloses MITM
attribution attacks and matches the rationale documented for
`HistorySyncComplete` in
[`2026-04-24-history-sync-eose.md`](2026-04-24-history-sync-eose.md).

## Receiver-side wiring

A `WireMessage::Reject` is delivered through the same
`unpack_wire` → match pipeline that already dispatches every other
`WireMessage` variant. The receiver-side changes are confined to
the gossip listeners in each crate:

- **Client** (`crates/client/src/...`): the gossip listener that
  matches on `WireMessage` variants gains a new arm that maps
  `WireMessage::Reject(payload)` (with the verified signer from
  `unpack_wire`) to a new `ClientEvent::Rejected { from: EndpointId,
  payload: RejectPayload }` variant on `ClientEvent` in
  [`crates/client/src/events.rs`](../../crates/client/src/events.rs).
  The UI subscribes the same way it already subscribes to
  `MessageReceived` / `SyncCompleted`.
- **Relay** ([`crates/relay/src/lib.rs`](../../crates/relay/src/lib.rs)):
  the relay is content-agnostic (see the `Scope: transport only`
  module-level docs at `relay/src/lib.rs:9-22`) and forwards
  `WireMessage::Reject` envelopes verbatim by its existing topic-
  routing rules. The relay also **emits** rejects directly for the
  cases it owns: invalid `TopicAnnounce` strings (currently dropped
  silently at `relay/src/lib.rs:388`), `MAX_TOPICS` cap hits at
  `relay/src/lib.rs:398`, and connection-cap saturation at
  `relay/src/lib.rs:156` — each replaces today's silent-drop or
  log-only path with a `pack_wire`-encoded `WireMessage::Reject` sent
  back to the offending peer on the same topic.
- **Replay**
  ([`crates/replay/src/role.rs`](../../crates/replay/src/role.rs)):
  emits rejects from the same code paths that surface
  `InsertError` today (DAG-insert failures during sync streaming),
  carrying the resulting `WireRejectReason` back to the upstream
  source on `_willow_server_ops`.
- **Storage**
  ([`crates/storage/src/role.rs`](../../crates/storage/src/role.rs)):
  emits rejects when a streamed event fails to apply or fails the
  archival-write path; mirrors the replay-side wiring above.

In all four crates the receive path stays the same — a
`WireMessage::Reject` arriving for the local peer is forwarded to the
client's event stream as `ClientEvent::Rejected`. The only crate that
gains a new producer code path is whichever one owns the rejecting
decision; every other crate's only change is the new `match` arm.

## Mapping table

| Current source | Site | New variant |
|---|---|---|
| `InsertError::InvalidSignature` | [`dag.rs:18`](../../crates/state/src/dag.rs) | `SignatureInvalid` |
| `InsertError::Duplicate` | [`dag.rs:34`](../../crates/state/src/dag.rs) | `Duplicate` |
| `InsertError::DuplicateGenesis` | [`dag.rs:36`](../../crates/state/src/dag.rs) | `Invalid("duplicate genesis")` |
| `InsertError::NotGenesis` | [`dag.rs:19`](../../crates/state/src/dag.rs) | `Invalid("first event must be CreateServer")` |
| `InsertError::SeqGap` | [`dag.rs:22`](../../crates/state/src/dag.rs) | `SeqGap { expected, actual }` |
| `InsertError::PrevMismatch` | [`dag.rs:28`](../../crates/state/src/dag.rs) | `ParentHashMismatch { expected, actual }` |
| `InsertError::MissingGovernanceDep` | [`dag.rs:38`](../../crates/state/src/dag.rs) | `Invalid("vote missing proposal dep")` |
| `InsertError::PermissionDenied(_)` | [`dag.rs:43`](../../crates/state/src/dag.rs) | `PermissionDenied(perm)` |
| `check_permission` "not an admin" | [`materialize.rs:94`](../../crates/state/src/materialize.rs) | `Restricted("admin required")` |
| `check_permission` lacks `Permission::X` | [`materialize.rs:117`](../../crates/state/src/materialize.rs) | `PermissionDenied(X)` |
| `ApplyResult::Rejected(String)` | [`materialize.rs:24`](../../crates/state/src/materialize.rs) | `PermissionDenied(_)` / `Restricted(_)` depending on cause |
| `IdentityError::InvalidSignature` | [`identity/lib.rs:52`](../../crates/identity/src/lib.rs) | `SignatureInvalid` |
| `IdentityError::PeerMismatch` | [`identity/lib.rs:79`](../../crates/identity/src/lib.rs) | `Invalid("peer_id mismatch")` |
| `IdentityError::Serde` | [`identity/lib.rs:48`](../../crates/identity/src/lib.rs) | `Invalid("serde: …")` |
| `TransportError::UnsupportedVersion` | [`transport/lib.rs:53`](../../crates/transport/src/lib.rs) | `UnsupportedVersion { expected, actual }` |
| `TransportError::Deserialize` (size) | [`transport/lib.rs:155`](../../crates/transport/src/lib.rs) | `PayloadTooLarge { limit, actual }` |
| `TransportError::Deserialize` (shape) | [`transport/lib.rs:162`](../../crates/transport/src/lib.rs) | `Invalid("deser: …")` |
| Relay `topic_str_is_valid` fails | [`relay/lib.rs:388`](../../crates/relay/src/lib.rs) | `TopicInvalid(topic)` |
| Relay `MAX_TOPICS` cap reached | [`relay/lib.rs:398`](../../crates/relay/src/lib.rs) | `Capacity` |
| Relay connection cap reached | [`relay/lib.rs:156`](../../crates/relay/src/lib.rs) (`Err(_)` arm of `try_acquire_owned`) | `RateLimited { retry_after_ms }` |
| `check_permission` admin-only block | [`materialize.rs:111`](../../crates/state/src/materialize.rs) | `Restricted("admin required")` |
| Vote on missing proposal | [`materialize.rs:161`](../../crates/state/src/materialize.rs) | `Invalid("proposal not found")` |
| `RotateChannelKey` non-member | [`materialize.rs:497`](../../crates/state/src/materialize.rs) | `Restricted("not a member")` |
| Relay not granted `SyncProvider` | (future history-serve guard) | `NotSyncProvider` |
| iroh gossip receive error | [`network/iroh.rs:164`](../../crates/network/src/iroh.rs) | `ServerError` (local-only; not sent) |

The mapping table above is illustrative of the major rejection
categories rather than exhaustive — additional defense-in-depth and
governance branches in `materialize.rs` map onto the same
`Restricted(_)` / `Invalid(_)` shapes shown for their cousins.

The `iroh gossip receive error` row is local-only: it feeds
structured logs and metrics but is never serialised onto the wire,
because the peer we would be telling is precisely the peer we failed
to decode bytes from.

## Client consumption pattern

The client event loop gains one arm, dispatching the new
`ClientEvent::Rejected` introduced in "Receiver-side wiring" above.
This is the payoff:

```rust
ClientEvent::Rejected { from, payload: RejectPayload { reason, context, .. } } => match reason {
    WireRejectReason::Duplicate            => { /* peer caught up, drop */ }
    WireRejectReason::RateLimited { retry_after_ms }
                                           => backoff.schedule(retry_after_ms),
    WireRejectReason::AuthRequired         => ui.prompt_reauth(),
    WireRejectReason::PermissionDenied(p)  => ui.surface_permission_error(p, context),
    WireRejectReason::ParentHashMismatch { .. }
    | WireRejectReason::SeqGap { .. }      => sync.request_history(context),
    WireRejectReason::UnsupportedVersion { .. }
                                           => ui.prompt_upgrade(),
    WireRejectReason::PayloadTooLarge { .. }
                                           => outbox.chunk_and_retry(context),
    WireRejectReason::SignatureInvalid     => log::error!("own signature failed — bug"),
    _                                      => log::warn!(?from, ?reason, ?context, "rejected"),
}
```

## Extensibility & versioning

- `#[non_exhaustive]` forces downstream `match` to carry a wildcard,
  so adding a variant is never a SemVer break.
- bincode encodes enums with a `u32` discriminant. A receiver that
  hits an unknown discriminant fails `unpack_wire` cleanly (the
  `unpack_envelope` call inside it returns `Err`); the client treats
  the failed decode as a local `ServerError` with
  `human = Some("unknown reason variant")`, logs the raw bytes at
  `debug!`, and leaves the original outbound event in its retry
  queue — safe default, because a newer peer that accepts is still
  reachable elsewhere in the mesh.
- String payloads (`Invalid`, `Restricted`, `TopicInvalid`) absorb
  future sub-categories without a variant bump, mirroring Nostr's
  "prefix + human text" discipline.
- `PROTOCOL_VERSION`
  ([`transport/lib.rs:30`](../../crates/transport/src/lib.rs)) is
  reserved for **breaking** wire changes; adding a reject variant is
  not one.

## Logging

The same enum feeds structured logs via
`tracing::warn!(reason = ?r, context = ?c, human = %h)`. The
`WireRejectReason` is `Debug`-derived, so each variant's field values
appear verbatim in the log event and can be filtered in `tracing-
subscriber`. Metrics counters key on the enum's `discriminant()` for
a flat histogram of rejection causes over time. Operators get
"how many `RateLimited` per minute?" for free, with no string
parsing.

## Tests

Coverage hits the three places a reason can go wrong — produced,
serialised, consumed.

- **Round-trip** (`crates/common/src/wire.rs` test module, alongside
  the existing `pack_unpack_*_round_trip` cases): every
  `WireRejectReason` variant survives `pack_wire` → `unpack_wire`
  equality wrapped in `WireMessage::Reject(RejectPayload)`, driven by
  a macro that iterates a representative value per variant.
- **Exhaustive mapping** (`state/src/tests.rs`): for each
  `InsertError` variant, build a DAG that triggers it and assert the
  expected `WireRejectReason`.
- **check_permission**: for each `Permission`, reject and assert
  `PermissionDenied(p)` — retires the stringly reasons.
- **Transport**: oversized payload → `PayloadTooLarge`; version 0
  and 999 → `UnsupportedVersion`.
- **Relay**: invalid topic → `TopicInvalid`; topic cap full →
  `Capacity`.
- **Forward compat**: encode a synthetic unknown discriminant, assert
  receiver logs, preserves outbox, does not panic.
- **Browser** (`crates/web/tests/browser.rs`): a
  `PermissionDenied(SendMessages)` reject flips the just-sent bubble
  to its "cannot-send" state within one `tick().await`.

## Open questions

1. Should `RejectPayload` be authenticated? A relay that forges
   `PermissionDenied` against a peer's legitimate event could
   suppress that peer's messaging UI. Probably yes — sign the
   payload with the rejecting peer's identity so clients can decide
   whether to trust it.
2. How do we correlate `RejectContext::Envelope` with the offending
   send when the envelope never carried a hash? Option: stamp an
   outbound `send_id` (u64) in every `Envelope` and echo it in the
   reject.
3. Is `PermissionDenied` leaking too much to an untrusted relay?
   Telling the rejector which `Permission` they lack is fine;
   telling a third party could help an attacker enumerate roles.
   Route: relays forward rejections verbatim, clients filter.
4. Should `RateLimited.retry_after_ms` be advisory (client may
   ignore) or enforced (peer drops earlier retries)? Nostr leaves
   this implementation-defined; we probably should too.
5. Do we need a separate `MessageType::Ack` for the positive case,
   or is "no reject within N seconds" enough? Nostr requires both
   OK (accept) and OK (reject); we currently rely on gossip delivery
   as implicit ACK. Worth revisiting once `Reject` ships.
6. Does the state machine itself grow a new `EventKind` to record
   rejections for audit, or do they stay ephemeral? Audit is
   tempting but contradicts the "rejected events never enter the
   DAG" rule from
   [`2026-04-12-state-authority-and-mutations.md`](2026-04-12-state-authority-and-mutations.md).
