//! Relay / worker discovery — the explicit composition of three independent
//! layers (outbox-relay-discovery spec,
//! `docs/specs/2026-04-24-outbox-relay-discovery.md`).
//!
//! Willow introduces **no** `EventKind::RelayList`. Discovery is the
//! traversal, in order, of three primitives that already exist:
//!
//! | Layer | Question | Mechanism | Source of truth |
//! |-------|----------|-----------|-----------------|
//! | 1 | `EndpointId` → addresses | iroh pkarr (DHT) | the endpoint's own key |
//! | 2 | `EndpointId` → role / capabilities | `/.well-known/willow` doc | served by the endpoint over HTTPS |
//! | 3 | `EndpointId` → trusted to sync | `SyncProvider` grant in the server DAG | `crates/state/src/event.rs` |
//!
//! This module owns the **Layer 3 enumeration** ([`enumerate_sync_providers`]),
//! the **Layer 2 fetch + verification** ([`fetch_relay_info`], the single
//! definition reused by the web join flow), and the **ranking** step that
//! composes all three ([`rank_relay_candidates`]).
//!
//! ## Why the HTTP transport is injected
//!
//! `fetch_relay_info` is generic over an async fetcher rather than calling a
//! concrete HTTP client. `willow-client` is a dual-target lib crate (native +
//! `wasm32`); a baked-in client would drag a heavy dual-target dependency tree
//! into the crate and couple the *security* logic (signature verification +
//! protocol intersection — the load-bearing part) to a transport, forcing a
//! live server (or a mock-server crate) to test it. With the transport
//! injected, the verify/intersect contract is exercised with canned bytes,
//! zero network, and zero `#[cfg]` in the security path; the web crate supplies
//! a `web_sys::fetch`-backed closure (mirroring the existing
//! `fetch_bootstrap_id` in `crates/web/src/app.rs`), and a native caller can
//! supply any HTTP client. The verification itself is shared, never
//! reimplemented per side — it routes through PR 1's
//! [`willow_common::verify_capability_doc`].

use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

use willow_common::{verify_capability_doc, WillowRelayInfo};
use willow_identity::EndpointId;
use willow_state::{Permission, ServerState};

/// Why a capability document could not be used as an authoritative relay/worker
/// descriptor.
///
/// The three *refusal* variants are deliberately distinct so callers (and the
/// join-flow UI) can tell a transient transport hiccup ([`Self::Fetch`]) apart
/// from a security refusal ([`Self::BadSignature`]) apart from a hard
/// incompatibility ([`Self::ProtocolMismatch`]). A doc that triggers any of the
/// latter two MUST NOT be trusted — the spec treats it as if the endpoint
/// served no doc at all.
#[derive(Debug, thiserror::Error)]
pub enum RelayInfoError {
    /// The injected transport failed to retrieve the document body.
    #[error("capability-doc fetch failed: {0}")]
    Fetch(String),
    /// The retrieved body was not valid JSON for a [`WillowRelayInfo`].
    #[error("capability-doc parse failed: {0}")]
    Parse(String),
    /// The detached Ed25519 signature did not verify against the document's own
    /// `pubkey`. Reuses PR 1's [`willow_common::verify_capability_doc`].
    #[error("capability-doc signature verification failed")]
    BadSignature,
    /// The relay's advertised `protocol_versions` share no value with the
    /// client's — the two cannot speak a common wire version.
    #[error("no shared protocol version (relay {relay:?}, client {client:?})")]
    ProtocolMismatch {
        /// Versions the relay advertised.
        relay: Vec<u16>,
        /// Versions the client supports.
        client: Vec<u16>,
    },
}

/// A discovered endpoint paired with everything the ranker needs to decide
/// whether — and how highly — to rank it.
///
/// `is_sync_provider` is the Layer-3 verdict (computed by the caller from
/// [`enumerate_sync_providers`] over the server DAG); `info` is the verified
/// Layer-2 capability doc; `latency` is the measured Layer-1 round-trip. The
/// ranker keys trust on `endpoint` + `is_sync_provider`, never on the doc
/// alone — a valid signature proves *who served the doc*, not *that the server
/// trusts them to sync*.
#[derive(Debug, Clone)]
pub struct RelayCandidate {
    /// The endpoint this candidate describes.
    pub endpoint: EndpointId,
    /// Whether `endpoint` holds a live `SyncProvider` grant for the server in
    /// question (the Layer-3 trust verdict).
    pub is_sync_provider: bool,
    /// The verified capability document served by `endpoint` (Layer 2).
    pub info: WillowRelayInfo,
    /// Measured round-trip latency to `endpoint` (Layer 1), used to order
    /// trusted candidates.
    pub latency: Duration,
}

/// A candidate that survived the trust filter, retained for connection in
/// latency order.
#[derive(Debug, Clone)]
pub struct RankedRelay {
    /// The trusted endpoint.
    pub endpoint: EndpointId,
    /// Its verified capability document.
    pub info: WillowRelayInfo,
    /// Its measured latency (the ordering key).
    pub latency: Duration,
}

/// Enumerate the live (un-revoked) `SyncProvider` grants in a materialized
/// server state — Layer 3 of relay discovery.
///
/// > An endpoint is a candidate sync source for server S **iff** its
/// > `EndpointId` is the target of a live, un-revoked `SyncProvider` grant in
/// > S's event DAG. — outbox-relay-discovery spec, Layer 3.
///
/// This reads [`ServerState::peer_permissions`], the map that
/// [`materialize::apply_incremental`](willow_state::materialize::apply_incremental)
/// keeps in lockstep with the DAG: a `GrantPermission { SyncProvider }` inserts
/// into it and a `RevokePermission { SyncProvider }` removes from it, so a
/// revoked grant simply isn't present — no separate liveness check is needed.
///
/// Implicit admin/owner authority is deliberately **excluded** *for discovery*:
/// an admin holds `SyncProvider` via [`ServerState::has_permission`]'s "admins
/// have every permission" rule, but is only enumerated here if it was
/// *explicitly* granted the role (its key appears in `peer_permissions`). This
/// is a discovery-ranking choice — a peer is advertised as a *dedicated* sync
/// source only when deliberately designated, so the outbox layer ranks explicit
/// providers rather than every admin.
///
/// Note: this is **not** the serving gate. The SyncRequestV2 responder gates on
/// [`ServerState::is_sync_provider`] (which honors the owner/admins implicitly),
/// so the owner serves its own server's state to joining members even though it
/// is not enumerated here. Discovery (who to *advertise*) and serving (who may
/// *answer*) answer different questions.
pub fn enumerate_sync_providers(state: &ServerState) -> BTreeSet<EndpointId> {
    state
        .peer_permissions
        .iter()
        .filter(|(_, perms)| perms.contains(&Permission::SyncProvider))
        .map(|(peer, _)| *peer)
        .collect()
}

/// Verify a capability-document body and check protocol compatibility — the
/// transport-agnostic core of [`fetch_relay_info`].
///
/// Split out from the fetch so the verify/intersect contract (the security
/// logic) is unit-testable with canned bytes. Routes signature verification
/// through PR 1's [`willow_common::verify_capability_doc`] so producer (relay)
/// and consumer (client) share the *identical* canonicalization — reimplementing
/// it per side would silently break verification.
fn verify_and_intersect(
    body: &str,
    client_versions: &[u16],
) -> Result<WillowRelayInfo, RelayInfoError> {
    let info: WillowRelayInfo =
        serde_json::from_str(body).map_err(|e| RelayInfoError::Parse(e.to_string()))?;

    // Layer 2 trust anchor: the doc must be signed by the key it claims.
    match verify_capability_doc(&info) {
        Ok(true) => {}
        // Either a `false` verdict or a structural error (missing/garbled
        // signature or pubkey) means the doc cannot be trusted.
        _ => return Err(RelayInfoError::BadSignature),
    }

    // The client and relay must share at least one wire-protocol version.
    let intersects = info
        .protocol_versions
        .iter()
        .any(|v| client_versions.contains(v));
    if !intersects {
        return Err(RelayInfoError::ProtocolMismatch {
            relay: info.protocol_versions.clone(),
            client: client_versions.to_vec(),
        });
    }

    Ok(info)
}

/// Fetch, verify, and protocol-check an endpoint's `/.well-known/willow`
/// capability document — Layer 2 of relay discovery, and the **single**
/// definition reused by the web join flow (do not duplicate).
///
/// The HTTP transport is injected as `fetch`: an async fn from the doc URL to
/// either the response body or a transport-error string (see the module-level
/// rationale). This function then:
///
/// 1. awaits `fetch(url)`; a transport failure becomes [`RelayInfoError::Fetch`];
/// 2. parses the body as [`WillowRelayInfo`] ([`RelayInfoError::Parse`] on
///    failure);
/// 3. verifies the detached Ed25519 signature via PR 1's
///    [`willow_common::verify_capability_doc`], refusing
///    ([`RelayInfoError::BadSignature`]) any doc whose signature does not
///    verify against its own `pubkey`;
/// 4. refuses ([`RelayInfoError::ProtocolMismatch`]) any doc whose
///    `protocol_versions` do not intersect `client_versions`.
///
/// Only a doc that clears every gate is returned. The caller is still
/// responsible for the Layer-3 trust check (is this endpoint a live
/// `SyncProvider`?) before treating it as an authoritative sync source — see
/// [`rank_relay_candidates`].
pub async fn fetch_relay_info<F, Fut>(
    url: &str,
    client_versions: &[u16],
    fetch: F,
) -> Result<WillowRelayInfo, RelayInfoError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let body = fetch(url.to_string())
        .await
        .map_err(RelayInfoError::Fetch)?;
    verify_and_intersect(&body, client_versions)
}

/// Latency-rank candidate sync sources, refusing to treat any non-`SyncProvider`
/// endpoint as authoritative — the composition of Layers 1+2+3.
///
/// The trust filter runs **first and unconditionally**: a candidate whose
/// `is_sync_provider` is `false` is dropped no matter how low its latency or how
/// valid its capability doc, because a signed doc proves only *who* served it,
/// never that the server *trusts* them to sync (spec, Layer 3 / test "refuse to
/// use a non-`SyncProvider` endpoint as authoritative"). Survivors are ordered
/// fastest-first; the caller connects to the top N (spec default N=3).
pub fn rank_relay_candidates(candidates: Vec<RelayCandidate>) -> Vec<RankedRelay> {
    let mut ranked: Vec<RankedRelay> = candidates
        .into_iter()
        .filter(|c| c.is_sync_provider)
        .map(|c| RankedRelay {
            endpoint: c.endpoint,
            info: c.info,
            latency: c.latency,
        })
        .collect();
    ranked.sort_by_key(|r| r.latency);
    ranked
}
