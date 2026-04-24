# Machine-Readable Wire-Rejection Reasons

> **One-sentence summary:** introduce a typed `WireRejectReason` enum,
> carried in a new `MessageType::Reject` envelope, so peers can react
> programmatically to rejections — retry on rate-limit, re-auth on
> `AuthRequired`, drop silently on `Duplicate`, surface a permission
> prompt on `PermissionDenied` — instead of matching on free-form error
> strings that are only fit for logs.

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
([`transport/lib.rs:99`](../../crates/transport/src/lib.rs)), so we can
carry a **typed enum** — same machine-readability as Nostr, plus
compile-time exhaustive matching and structured payloads like
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
- `Identity::verify` fail on a forged envelope — re-sign
  ([`identity/lib.rs:51`](../../crates/identity/src/lib.rs)).

## Proposed format

A new enum lives in `willow-transport` so every crate above it can
produce the type without depending on `willow-state`:

```rust
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireRejectReason {
    Duplicate,
    Invalid(String),
    RateLimited { retry_after_ms: u64 },
    PermissionDenied(Permission),     // re-exported from willow-state
    ParentHashMismatch { expected: EventHash, actual: EventHash },
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

## Wire envelope

Add a new variant to `MessageType` in
[`crates/transport/src/lib.rs:64`](../../crates/transport/src/lib.rs):

```rust
#[repr(u8)]
pub enum MessageType {
    Chat = 0, Channel = 1, Identity = 2, File = 3,
    Signal = 4, Presence = 5, Ping = 6,
    Reject = 7,
}
```

The payload is a new `RejectPayload` carrying the reason plus enough
context for the receiver to correlate the rejection with the event it
sent:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectPayload {
    pub reason: WireRejectReason,
    pub context: RejectContext,
    pub human: Option<String>,        // logs/UI only; never parsed
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RejectContext {
    Event(EventHash),
    Topic(TopicId),
    Envelope,                         // predates any event hash
}
```

`human` is the existing free-form `Display` output of the underlying
error (e.g. the current `InsertError` message); the reason variant is
canonical, the string is never matched on.

## Mapping table

| Current source | Site | New variant |
|---|---|---|
| `InsertError::InvalidSignature` | [`dag.rs:18`](../../crates/state/src/dag.rs) | `SignatureInvalid` |
| `InsertError::Duplicate` | [`dag.rs:34`](../../crates/state/src/dag.rs) | `Duplicate` |
| `InsertError::DuplicateGenesis` | [`dag.rs:36`](../../crates/state/src/dag.rs) | `Invalid("duplicate genesis")` |
| `InsertError::NotGenesis` | [`dag.rs:19`](../../crates/state/src/dag.rs) | `Invalid("first event must be CreateServer")` |
| `InsertError::SeqGap` | [`dag.rs:22`](../../crates/state/src/dag.rs) | `SeqGap { expected, actual }` |
| `InsertError::PrevMismatch` | [`dag.rs:28`](../../crates/state/src/dag.rs) | `ParentHashMismatch { expected, actual }` |
| `InsertError::MissingGovernanceDep` | [`dag.rs:37`](../../crates/state/src/dag.rs) | `Invalid("vote missing proposal dep")` |
| `InsertError::PermissionDenied(_)` | [`dag.rs:43`](../../crates/state/src/dag.rs) | `PermissionDenied(perm)` |
| `check_permission` "not an admin" | [`materialize.rs:94`](../../crates/state/src/materialize.rs) | `Restricted("admin required")` |
| `check_permission` lacks `Permission::X` | [`materialize.rs:117`](../../crates/state/src/materialize.rs) | `PermissionDenied(X)` |
| `ApplyResult::Rejected(String)` | [`materialize.rs:24`](../../crates/state/src/materialize.rs) | `PermissionDenied(_)` / `Restricted(_)` depending on cause |
| `IdentityError::InvalidSignature` | [`identity/lib.rs:51`](../../crates/identity/src/lib.rs) | `SignatureInvalid` |
| `IdentityError::PeerMismatch` | [`identity/lib.rs:79`](../../crates/identity/src/lib.rs) | `Invalid("peer_id mismatch")` |
| `IdentityError::Serde` | [`identity/lib.rs:48`](../../crates/identity/src/lib.rs) | `Invalid("serde: …")` |
| `TransportError::UnsupportedVersion` | [`transport/lib.rs:53`](../../crates/transport/src/lib.rs) | `UnsupportedVersion { expected, actual }` |
| `TransportError::Deserialize` (size) | [`transport/lib.rs:155`](../../crates/transport/src/lib.rs) | `PayloadTooLarge { limit, actual }` |
| `TransportError::Deserialize` (shape) | [`transport/lib.rs:162`](../../crates/transport/src/lib.rs) | `Invalid("deser: …")` |
| Relay `topic_str_is_valid` fails | [`relay/lib.rs:388`](../../crates/relay/src/lib.rs) | `TopicInvalid(topic)` |
| Relay `MAX_TOPICS` cap reached | [`relay/lib.rs:398`](../../crates/relay/src/lib.rs) | `Capacity` |
| Relay connection cap reached | [`relay/lib.rs:155`](../../crates/relay/src/lib.rs) | `RateLimited { retry_after_ms }` |
| Relay not granted `SyncProvider` | (future history-serve guard) | `NotSyncProvider` |
| iroh gossip receive error | [`network/iroh.rs:164`](../../crates/network/src/iroh.rs) | `ServerError` (local-only; not sent) |

Items marked "local-only" in the last column feed structured logs and
metrics but are never serialised onto the wire, because the peer we
would be telling is precisely the peer we failed to decode bytes
from.

## Client consumption pattern

The client event loop gains one arm. This is the payoff:

```rust
ClientEvent::Rejected(RejectPayload { reason, context, .. }) => match reason {
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
    _                                      => log::warn!(?reason, ?context, "rejected"),
}
```

## Extensibility & versioning

- `#[non_exhaustive]` forces downstream `match` to carry a wildcard,
  so adding a variant is never a SemVer break.
- bincode encodes enums with a `u32` discriminant. A receiver that
  hits an unknown discriminant fails `unpack_envelope` cleanly; the
  client treats the failed decode as a local `ServerError` with
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

- **Round-trip** (`transport/src/lib.rs` tests): every variant
  survives `pack` → `unpack` equality, driven by a macro that
  iterates a representative value per variant.
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
