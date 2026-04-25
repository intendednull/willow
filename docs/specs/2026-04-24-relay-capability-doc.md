# Relay Capability Document

> **One-sentence summary:** Willow relays expose a plain-HTTP
> `/.well-known/willow` JSON document — a NIP-11-style capability
> sidecar — that clients fetch *before* connecting so they can discover
> the relay's protocol versions, limits, auth/payment requirements, and
> operator metadata without a failed-connection round-trip.

## Motivation

Today a Willow client opens a connection to the relay listener bound
in `crates/relay/src/main.rs:129` and spawned via `run_proxy_listener`
at `crates/relay/src/main.rs:202` — a single public TCP port (default
`3340`) that multiplexes `/bootstrap-id` plus an HTTP/WebSocket-upgrade
proxy to the loopback iroh-relay — and *only then* discovers whether
the relay supports its wire version, whether it happens to be
storage-degraded, or whether its topic cap has been reached. Failure
is silent or shows up as a confusing disconnect — exactly the "why
did that connection fail?" problem that NIP-11 was designed to solve
for Nostr. The capability document is served on the **same** port as
the relay handshake, not a sidecar port.

A sidecar capability document lets clients pick the right wire
version before the handshake (see `PROTOCOL_VERSION` in
`crates/transport/src/lib.rs:30`); decide whether the user needs an
invite or payment proof before dialling; surface a "degraded / full"
banner; display operator name, contact, and ToS in a settings sheet;
and filter a relay directory without connecting to each candidate.

## Dispatch surgery (relay)

Today `dispatch_connection` in `crates/relay/src/lib.rs` carves out
exactly one path — `BOOTSTRAP_ID_PATH = "/bootstrap-id"` — and
forwards everything else to the loopback iroh-relay. The active
production handler is `handle_bootstrap_request_after_line`
(`crates/relay/src/lib.rs:266-314`), reached through
`run_proxy_listener` → `dispatch_connection`; the older
`handle_bootstrap_connection` (`crates/relay/src/lib.rs:102`) is now
only exercised by the test-only `run_bootstrap_listener` path used in
`crates/relay/tests/bootstrap_endpoint.rs`. As written, a naive
`GET /.well-known/willow` would land in the upstream iroh-relay and
404 (or be misread as a relay handshake and dropped). Implementation
MUST add:

1. An explicit branch in `dispatch_connection` matching
   `GET /.well-known/willow` and `OPTIONS /.well-known/willow`
   *before* the iroh-relay fallthrough.
2. A new handler analogous to `handle_bootstrap_request_after_line`
   that emits the JSON body, ETag, and CORS headers (GET) or returns
   `204` with full ACAO/ACAM/ACAH (OPTIONS preflight).
3. Reuse of `BOOTSTRAP_IO_TIMEOUT` for socket reads/writes and the
   existing `MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS` semaphore for
   admission control. No new tuning knobs in v1. Note: the constant
   name is already stale — it gates the public proxy semaphore in
   `crates/relay/src/main.rs:201-207`, not just bootstrap-id traffic
   — and SHOULD be renamed (e.g. `MAX_CONCURRENT_PROXY_CONNECTIONS`)
   in the same change that introduces this endpoint.

This is an **extension** of the existing proxy-handler pattern (both
`handle_bootstrap_request_after_line` in production and
`handle_bootstrap_connection` in tests), not a mirror: both handlers
currently emit ACAO only (no ACAM/ACAH) and neither responds to
`OPTIONS` preflights. Both gaps are closed here.

## Endpoint

| Property | Value |
|---|---|
| Path | `/.well-known/willow` |
| Method | `GET` (plus `OPTIONS` for preflight) |
| Response Content-Type | `application/willow+json; charset=utf-8` |
| CORS | required — `Access-Control-Allow-Origin: *` |
| Served on | the public relay HTTP port (default `3340`, configurable via `--relay-port`; see `crates/relay/src/main.rs:87`) |

**Why `/.well-known/willow` over `/willow-info`?** The relay proxy in
`crates/relay/src/lib.rs:186` already dispatches on request path.
`/.well-known/*` (RFC 8615) gives a stable namespace for future
sidecars (e.g. `/.well-known/willow-payment`) without top-level
collisions.

**Why a dedicated content type over `application/json`?** Nostr chose
`Accept: application/nostr+json` because relay and info share a path.
Willow's proxy multiplexes by path, so a distinct path plus a distinct
media type beats Accept-based disambiguation; the `+json` structured
suffix still opts us into generic JSON tooling.

## Field schema

The document is a single JSON object. All fields are optional except
`protocol_versions`; a minimal compliant document is
`{"protocol_versions":[1]}`. **Clients MUST ignore unknown top-level
fields** so additions remain forward-compatible.

```rust
/// Capability document served at GET /.well-known/willow.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WillowRelayInfo {
    // Operator metadata (all optional, display-only).
    pub name: Option<String>,             // ≤ 60 UTF-8 bytes
    pub description: Option<String>,      // plain text, no markup
    pub contact: Option<String>,          // mailto: / https: / matrix:
    pub admin_pubkey: Option<String>,     // hex Ed25519, operator DM key
    pub pubkey: Option<String>,           // hex Ed25519, relay's own key
    pub software: Option<String>,         // project name; operators MAY omit
    pub version: Option<String>,          // coarse semver (e.g. "0.3.x");
                                          // operators MAY omit; never a git SHA
    pub terms_of_service: Option<String>,
    pub privacy_policy: Option<String>,
    pub icon: Option<String>,             // square, ≥ 64×64

    /// REQUIRED. Wire-protocol versions the relay accepts, sorted
    /// highest-first, no duplicates. Mirrors
    /// `willow_transport::PROTOCOL_VERSION`.
    pub protocol_versions: Vec<u16>,

    /// Short string feature tags. Initial set: "gossip", "history",
    /// "blobs", "voice-signal", "invite-gate", "payment-gate". See
    /// "Cross-spec coordination" below for the canonical tag table.
    #[serde(default)]
    pub supported_features: Vec<String>,

    /// REQUIRED. Detached Ed25519 signature over the canonical JSON
    /// (RFC 8785 JCS) of this object with the `signature` field
    /// removed. Encoded as lowercase hex. Signed with the relay's
    /// own Ed25519 key (the same `identity` constructed in
    /// `crates/relay/src/main.rs:104`); the public half is published
    /// in `pubkey`. Closes the "Clients MUST NOT cache across
    /// `pubkey` changes" gap and prevents on-path rewrites of
    /// `payment_required`, `min_client_version`, etc.
    pub signature: String,

    pub limitation: Option<Limitation>,
    pub retention: Option<Retention>,

    pub payments_url: Option<String>,  // required iff payment_required
    pub invites_url: Option<String>,   // required iff invite_required

    /// "ok" | "degraded" | "read_only". "degraded" = up but a worker
    /// (e.g. storage) is offline; clients SHOULD still connect.
    pub status: Option<String>,
    pub status_detail: Option<String>, // human-readable, plain text
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Limitation {
    pub max_message_bytes: Option<u32>,   // mirrors MAX_DESER_SIZE,
                                          // `crates/transport/src/lib.rs:36`
    pub max_topic_len: Option<u16>,       // MAX_TOPIC_LEN,
                                          // `crates/relay/src/lib.rs:84`
    pub max_topics: Option<u32>,          // MAX_TOPICS,
                                          // `crates/relay/src/lib.rs:80`
    pub max_connections: Option<u32>,     // MAX_CONCURRENT_BOOTSTRAP_
                                          // CONNECTIONS, lib.rs:59 —
                                          // misnamed; gates the
                                          // public proxy semaphore.
                                          // Rename in the same change
                                          // (see "Dispatch surgery").
    pub max_blob_bytes: Option<u64>,      // 0 = blob pinning off
    #[serde(default)] pub invite_required: bool,
    #[serde(default)] pub payment_required: bool,
    pub hlc_lower_limit: Option<u64>,     // oldest accepted HLC ms
    pub min_client_version: Option<u16>,  // reject older handshakes
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Retention {
    /// "replay" (in-memory; per-author cap, default 1000 events per
    /// author per server — see `ReplayConfig::max_events_per_author`
    /// in `crates/replay/src/role.rs:49,64`) or "storage" (SQLite).
    /// See `docs/specs/2026-03-27-worker-nodes-design.md`.
    pub mode: String,
    pub max_events_per_author: Option<u32>,  // null = unbounded;
                                             // mirrors the replay
                                             // role's per-author cap
    pub max_age_seconds: Option<u64>,        // null = keep everything
    /// Willow default: false. Sealed channel keys stay peer-to-peer.
    #[serde(default)] pub channel_key_escrow: bool,
}
```

## Versioning

`protocol_versions: Vec<u16>` is the sole negotiated axis in v1.
Client picks the highest integer in both its list and the relay's.
Empty intersection → refuse to connect, surface a "version mismatch"
error. The list MUST be sorted highest-first and MUST NOT contain
duplicates so the negotiation rule is unambiguous.

**WebSocket clients SHOULD also send `Sec-WebSocket-Protocol`** (e.g.
`willow.v2, willow.v1`) in the WS opening handshake. The JSON
document is *advisory* — useful for pre-connect filtering and
directory listings — but version selection at handshake time via
RFC 6455 subprotocol negotiation is authoritative, and gracefully
handles the case where a sidecar doc and the relay binary drift
(operator forgot to redeploy the JSON).

Event-schema versioning is **deferred**: `willow-state` defines
`EventKind` as a Rust enum with no numeric tag and no
`EVENT_SCHEMA_VERSION` constant. Advertising a range here would be
vapor. Listed as future work below.

The capability document itself is unversioned: compatibility comes from
"add fields, never repurpose" plus the mandatory ignore-unknown-fields
rule. A deprecated field graduates to a fresh name; peers that only
understand the old name keep working.

## Signing

The capability document MUST be signed. Without a signature, an
on-path attacker (or hostile CDN/reverse proxy fronting the relay)
could flip `payment_required: true`, downgrade `protocol_versions`
to `[1]`, or rewrite `pubkey` to a key the attacker controls. The
relay already has an Ed25519 key (`identity` in
`crates/relay/src/main.rs:104`); a detached signature is ~15 lines
of code and ~88 hex characters in the document.

- **Algorithm:** Ed25519 over the canonical JSON serialisation of
  the document with the `signature` field removed.
- **Canonicalisation:** RFC 8785 JSON Canonicalization Scheme (JCS).
  Two relays running the same software with the same metadata MUST
  produce byte-identical canonical bytes — this is what makes
  cross-relay caching possible (see "Caching" below).
- **Two canonical forms.** Be careful: the bytes the *signature*
  covers exclude the `signature` field (`CANON_SIGNED`), but the
  bytes the `ETag` covers include it (`CANON_ETAG`). Both subsections
  must be implemented in lockstep — sign first, then re-canonicalise
  the document with `signature` populated to derive the ETag. A
  shared helper that takes a `include_signature: bool` parameter
  keeps the two paths from drifting.
- **Encoding:** lowercase hex in the `signature` field.
- **Verification:** clients verify against the `pubkey` field. A
  document whose signature does not verify MUST be treated as if
  the endpoint returned `404`. Clients MUST NOT cache an
  unverified document — this preserves the XEP-0115 → XEP-0390
  lesson that an unverified capability blob can poison cache
  entries keyed by content hash.
- **Key rotation:** clients MUST NOT cache a document across a
  `pubkey` change. The signature plus the publication of
  `pubkey` together let clients pin a relay's identity across
  CDN proxies and infrastructure migrations.

## CORS

WASM clients in `crates/web` fetch this endpoint cross-origin, so the
relay MUST respond with the following on both `GET` and `OPTIONS`
preflight:

```
Access-Control-Allow-Origin: *
Access-Control-Allow-Methods: GET, OPTIONS
Access-Control-Allow-Headers: Accept, Content-Type, If-None-Match
```

This **extends** the pattern used by both proxy handlers — the
production-active `handle_bootstrap_request_after_line`
(`crates/relay/src/lib.rs:266-314`, ACAO emitted at line 298) and the
test-only `handle_bootstrap_connection`
(`crates/relay/src/lib.rs:102`, ACAO emitted at line 116). Both
currently send ACAO only and neither responds to `OPTIONS` preflights,
so the new dispatch branch must add ACAM/ACAH *and* an explicit
`OPTIONS → 204` path in `dispatch_connection`.

## Caching

- Response SHOULD carry a strong `ETag` derived from SHA-256 over
  the `CANON_ETAG` form: the RFC 8785 canonical JSON serialisation
  with `signature` **included**. (The signature itself covers
  `CANON_SIGNED`, the same canonicalisation with `signature`
  **removed**; see "Signing".) The ETag is strong, not weak,
  because canonical JSON gives byte-equality semantics — this is
  what enables cross-relay caching keyed by content hash, XEP-0115
  / 0390 style. Honour `If-None-Match` with `304 Not Modified`.
- **Two-tier `Cache-Control` keyed on `status`:**
  - Steady-state (`status == "ok"` or absent):
    `Cache-Control: public, max-age=300` — directories and clients
    appreciate the longer TTL.
  - Transitional (`status == "degraded"` or `"read_only"`):
    `Cache-Control: public, max-age=5, must-revalidate` — clients
    see the recovery quickly. The relay knows its own status and
    varies the header per response.
- Clients MUST NOT cache across `pubkey` changes; the relay's key
  is part of the cache identity. Combined with the mandatory
  signature this lets clients pin a relay across CDN proxies.

## Error modes

| Condition | Status | Body |
|---|---|---|
| OK | `200` | Full `WillowRelayInfo` JSON |
| Storage worker offline | `200` | `status: "degraded"` + `status_detail` |
| Relay shutting down | `503` | `{"status":"read_only","status_detail":"..."}` |
| Older relay without the sidecar | `404` | plain text |
| CORS preflight | `204` | empty, with ACAO/ACAM/ACAH |

Clients MUST treat `404` as "older relay; assume
`protocol_versions:[1]`, no advertised limits, proceed at your own
risk" so the endpoint is purely additive.

## Security considerations

- Every field is operator-controlled. The relay MUST NOT populate
  fields from peer-supplied metadata; otherwise a hostile peer can
  rewrite the sidecar via injection.
- `admin_pubkey` and `pubkey` are *hints*. Trust is established by
  the owner via `GrantPermission { permission: SyncProvider }` (see
  `docs/specs/2026-04-12-state-authority-and-mutations.md`). Merely
  appearing in the sidecar grants no authority.
- The endpoint is unauthenticated and MUST NOT expose connected-peer
  lists, traffic counts, or anything that fingerprints users.
- `status_detail` and `description` are rendered in the web UI — the
  client MUST escape them as text, never HTML.
- Rate-limit the endpoint by reusing the existing
  `MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS` semaphore
  (`crates/relay/src/lib.rs:59`), which already gates every
  connection accepted by `run_proxy_listener`. The constant's name
  is stale — see "Dispatch surgery" — and SHOULD be renamed
  alongside the new endpoint.

## Cross-spec coordination

This document is the natural advertising surface for sibling specs in
the #214–#221 set. To prevent tag-name drift (`hist-eose` here vs.
`history_eose` somewhere else), the canonical `supported_features`
strings are pinned here:

| Sibling | Feature tag | Notes |
|---|---|---|
| #214 (history EOSE) | `history-eose` | Set when relay emits an end-of-stored-events sentinel. |
| #216 (machine-readable rejections) | `rejection-codes-v1` | Bumped tag if/when codes evolve. May also bump `protocol_versions`. |
| #217 (bech32 pubkey HRP) | bech32-pubkey-format (no tag yet) | Coordinate `pubkey` / `admin_pubkey` encoding with #217; v1 here keeps hex but #217 may extend the schema. |
| #218 (gift-wrap DM) | `gift-wrap-dm` | Informational only — relays cannot tell whether traffic is gift-wrapped, so the tag advertises operator intent rather than a checked capability. |
| #219 (sync algorithm) | `negentropy` or `seq-vector-sync` | One tag per algorithm the relay implements; client picks. |
| #220 (epoch key rotation) | (none) | No relay impact — omit. |
| #221 (outbox / no `EventKind::RelayList`) | (none) | No relay impact in this doc; `suggested_relays` (future) overlaps and should be resolved jointly with #221. |

## Tests

**Unit (serde, in `crates/relay/src/`):** (1) round-trip a fully
populated `WillowRelayInfo` through `serde_json` with byte-for-byte
equality after canonical re-serialisation; (2) parse the minimum
document `{"protocol_versions":[1]}` and assert all other fields are
`None`/empty; (3) parse a document with an unknown top-level key and
assert it is ignored; (4) reject a document missing
`protocol_versions` with a typed error.

**Integration** (new `crates/relay/tests/capability_endpoint.rs`,
alongside `bootstrap_endpoint.rs`): (1) `GET /.well-known/willow` →
`200`, `Content-Type: application/willow+json`, ACAO/ACAM/ACAH;
(2) `OPTIONS` preflight → `204` + CORS headers; (3) simulate
storage-worker offline → `status == "degraded"`; (4) replay `GET` with
the previous `ETag` via `If-None-Match` → `304`.

**Browser (`crates/web/tests/browser.rs`):** stub `fetch` with a
document whose `protocol_versions` does not intersect the client's;
mount the connect flow; assert the "connect" button is disabled and a
mismatch banner is rendered.

## Resolved during review

- **Signing.** Resolved in favour of MUST-sign in v1 with an inline
  `signature` field excluded from the canonical bytes. See "Signing"
  above.
- **Multi-tenant relays.** Resolved in favour of **one shared
  document per host**. The relay in this codebase is topic-agnostic
  (`crates/relay/src/lib.rs:8-22`: "All routines in this crate
  operate at the transport layer", line 10) — it does not know what servers
  it relays for, only `TopicAnnounce` strings. Per-server
  `/.well-known/willow/{server_id}` would require teaching the
  relay to enumerate servers it has no semantic knowledge of, which
  contradicts the trust-model layering.
- **Operator-metadata leakage.** `version` and `software` softened to
  coarse semver and project name respectively; both MAY be omitted.
- **`sync_provider_only`.** Dropped from v1. The relay has no DAG
  and the field reduces to "operator vibes" without a concrete
  pre-handshake check. If resurrected later, it must be tied to a
  typed-error pre-handshake rejection so a client reading the field
  can do something actionable.
- **`event_schema_range`.** Dropped from v1; see "Future work".

## Open questions

1. **Payment proof format.** Willow has no payments primitive yet.
   `payment_required` ships as a boolean hint with no token format
   — clients can surface it but cannot satisfy it. Either spec the
   token format in a sibling doc or gate the field behind a build
   flag in a follow-up.
2. **Utilisation telemetry.** Advertise current load (e.g.
   `served_topics: u32`, "9998 / 10000 topics used") for client
   load balancing, or omit to avoid fingerprinting? A counted
   number (not list) avoids leaking server IDs but still gives
   clients something to balance on. Worth a follow-up spec either
   way.
3. **Relay discovery / `suggested_relays`.** Advertise sibling
   relays so clients discover alternates after connecting, à la
   Nostr relay exchange? Resolve jointly with #221 (outbox), since
   the shapes overlap.
4. **`supported_features` registry.** The cross-spec table above
   pins the v1 tags, but should the set be promoted to a Rust enum
   in `crates/transport` so unknown tags fail to compile, or stay
   free-form to allow out-of-tree operators to advertise local
   features?

## Future work

- **Event schema versioning.** Once `willow-state` introduces an
  `EVENT_SCHEMA_VERSION: u16` constant and a documented bump rule
  (additive variants vs. breaking changes), add an
  `event_schema_range: [min, max]` field to this document. Until
  then, advertising a range would be vapor.
- **DNS SVCB/HTTPS hints (RFC 9460).** A `willow-versions=1,2`
  SvcParam would let clients decide whether to dial at all with
  zero HTTP round-trips. Complementary to this document, not a
  replacement: SVCB cannot carry `terms_of_service`,
  `description`, or `status_detail`.
- **Per-peer capabilities.** Matrix split `/versions` (public,
  cacheable) from `/capabilities` (authenticated, per-user) at
  v1.10. If/when Willow grows per-peer capability answers, route
  them at a *new* path (e.g. `/willow/peer-capabilities`) rather
  than overloading `/.well-known/willow` — Matrix's v1.10 retrofit
  is a cautionary tale.
