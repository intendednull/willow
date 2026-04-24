# Relay Capability Document

> **One-sentence summary:** Willow relays expose a plain-HTTP
> `/.well-known/willow` JSON document — a NIP-11-style capability
> sidecar — that clients fetch *before* connecting so they can discover
> the relay's protocol versions, limits, auth/payment requirements, and
> operator metadata without a failed-connection round-trip.

## Motivation

Today a Willow client opens a TCP or WebSocket connection to the relay
listener in `crates/relay/src/main.rs:128` and *only then* discovers
whether the relay supports its wire version, whether it gates access on
`SyncProvider` permission, whether it happens to be storage-degraded,
or whether its topic cap has been reached. Failure is silent or shows
up as a confusing disconnect — exactly the "why did that connection
fail?" problem that NIP-11 was designed to solve for Nostr.

A sidecar capability document lets clients pick the right wire
version before the handshake (see `PROTOCOL_VERSION` in
`crates/transport/src/lib.rs:30`); decide whether the user needs an
invite or payment proof before dialling; surface a "degraded / full"
banner; display operator name, contact, and ToS in a settings sheet;
and filter a relay directory without connecting to each candidate.

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
    pub software: Option<String>,         // project URL
    pub version: Option<String>,          // semver or git SHA
    pub terms_of_service: Option<String>,
    pub privacy_policy: Option<String>,
    pub icon: Option<String>,             // square, ≥ 64×64

    /// REQUIRED. Wire-protocol versions the relay accepts, highest-
    /// preference first. Mirrors `willow_transport::PROTOCOL_VERSION`.
    pub protocol_versions: Vec<u16>,

    /// Short string feature tags. Initial set: "gossip", "history",
    /// "blobs", "voice-signal", "invite-gate", "payment-gate".
    #[serde(default)]
    pub supported_features: Vec<String>,

    /// Supported `EventKind` schema range `[min, max]` from
    /// `crates/state/src/event.rs`. Absent = assume `[1, 1]`.
    pub event_schema_range: Option<[u16; 2]>,

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
                                          // CONNECTIONS, lib.rs:59
    pub max_blob_bytes: Option<u64>,      // 0 = blob pinning off
    #[serde(default)] pub invite_required: bool,
    #[serde(default)] pub payment_required: bool,
    /// Relay drops traffic whose author isn't in its SyncProvider
    /// allowlist. The relay CAN'T enforce the state-level grant (it
    /// has no DAG), so this is a best-effort operator allowlist.
    #[serde(default)] pub sync_provider_only: bool,
    pub hlc_lower_limit: Option<u64>,     // oldest accepted HLC ms
    pub min_client_version: Option<u16>,  // reject older handshakes
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Retention {
    /// "replay" (in-memory, 1000-event cap per server) or "storage"
    /// (SQLite). See `docs/specs/2026-03-27-worker-nodes-design.md`.
    pub mode: String,
    pub max_events_per_server: Option<u32>,  // null = unbounded
    pub max_age_seconds: Option<u64>,        // null = keep everything
    /// Willow default: false. Sealed channel keys stay peer-to-peer.
    #[serde(default)] pub channel_key_escrow: bool,
}
```

## Versioning

Two independent version axes travel in this document:

| Axis | Field | Negotiation rule |
|---|---|---|
| Wire framing (`Envelope`) | `protocol_versions: Vec<u16>` | Client picks the highest integer in both its list and the relay's. Empty intersection → refuse to connect, surface a "version mismatch" error. |
| Event schema | `event_schema_range: [min, max]` | Client's active schema MUST lie within `[min, max]`. Outside the range the client either upgrades or treats the relay as a byte-forwarder and disables state replay through it. |

The capability document itself is unversioned: compatibility comes from
"add fields, never repurpose" plus the mandatory ignore-unknown-fields
rule. A deprecated field graduates to a fresh name; peers that only
understand the old name keep working.

## CORS

WASM clients in `crates/web` fetch this endpoint cross-origin, so the
relay MUST respond with the following on both `GET` and `OPTIONS`
preflight:

```
Access-Control-Allow-Origin: *
Access-Control-Allow-Methods: GET, OPTIONS
Access-Control-Allow-Headers: Accept, Content-Type, If-None-Match
```

This mirrors the pattern already in `handle_bootstrap_connection`
(`crates/relay/src/lib.rs:114`).

## Caching

- Response SHOULD carry a weak `ETag` derived from SHA-256 over the
  canonical JSON serialisation, and SHOULD honour `If-None-Match`
  with `304 Not Modified`.
- Default `Cache-Control: public, max-age=60` — low enough that
  operational-status transitions propagate within a minute, high
  enough for a relay directory to fan out cheaply.
- Clients MUST NOT cache across `pubkey` changes; the relay's key
  is part of the cache identity.

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
  (`crates/relay/src/lib.rs:59`).

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

## Open questions

1. **Signed documents.** Ship an Ed25519 signature over the canonical
   JSON so clients can pin a relay by key across CDN proxies? Sibling
   `/.well-known/willow.sig`, HTTP header, or inline `signature` field
   over a canonicalised hash?
2. **Multi-tenant relays.** One host can serve many Willow servers;
   per-server document at `/.well-known/willow/{server_id}`, or one
   shared document since the relay is topic-agnostic?
3. **Relay discovery.** Advertise `suggested_relays: Vec<Url>` so
   clients discover siblings after connecting, à la Nostr relay
   exchange?
4. **Payment proof format.** Willow has no payments primitive yet — if
   we keep `payment_required`, either spec the token format now or
   gate the field behind a build flag so operators don't advertise
   something no client can satisfy.
5. **`supported_features` registry.** Strings are friendlier than
   NIP-11 integers but drift; pin the tag set in `crates/transport`
   as an enum, or keep it free-form?
6. **Utilisation signalling.** Advertise current load (e.g. "9 998 /
   10 000 topics used") for client load balancing, or omit to avoid
   telemetry leakage? Worth a follow-up spec either way.
