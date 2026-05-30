//! Relay-discovery composition tests (outbox-relay-discovery spec, plan PR 6
//! Task 6.3).
//!
//! Three units under test, all in [`crate::relay_discovery`]:
//!
//! 1. **`SyncProvider` enumeration** — the trust layer (Layer 3). A peer is a
//!    candidate sync source for a server iff it holds a *live, un-revoked*
//!    explicit `SyncProvider` grant in that server's replayed DAG. Revoked
//!    grants are excluded; an admin/owner that never granted *itself* the role
//!    is **not** enumerated (auto-serving from admin status would make the
//!    serving gate — pinned decision 4 — meaningless).
//!
//! 2. **`fetch_relay_info`** — the capability layer (Layer 2). It fetches the
//!    `/.well-known/willow` document, then *reuses PR 1's
//!    [`willow_common::verify_capability_doc`]* to reject a doc whose detached
//!    Ed25519 signature does not verify, and rejects a doc whose
//!    `protocol_versions` do not intersect the client's. The HTTP transport is
//!    injected so the verify/intersect contract is exercised with zero network
//!    and zero `#[cfg]` in the security path.
//!
//! 3. **Ranking** — composes Layers 1+2+3. The ranker latency-orders candidate
//!    relays/workers but *refuses to treat a non-`SyncProvider` endpoint as
//!    authoritative*: a fetched-and-valid capability doc from an endpoint that
//!    is not a live `SyncProvider` is dropped, never ranked.

use std::time::Duration;

use willow_common::relay_info::features;
use willow_common::{sign_capability_doc, WillowRelayInfo};
use willow_identity::{EndpointId, Identity};
use willow_state::Permission;

use crate::relay_discovery::{
    enumerate_sync_providers, fetch_relay_info, rank_relay_candidates, RelayCandidate,
    RelayInfoError,
};
use crate::test_client;

// ───── Helpers ──────────────────────────────────────────────────────────

/// Build and sign a capability doc owned by `id`, advertising the given
/// `protocol_versions`. The signature verifies against the doc's own
/// `pubkey` (= `id`'s endpoint key), exactly as the relay produces it.
fn signed_doc(id: &Identity, protocol_versions: Vec<u16>) -> WillowRelayInfo {
    let mut info = WillowRelayInfo {
        name: Some("Test Relay".into()),
        description: None,
        contact: None,
        admin_pubkey: None,
        pubkey: hex::encode(id.endpoint_id().as_bytes()),
        software: None,
        version: None,
        terms_of_service: None,
        privacy_policy: None,
        icon: None,
        protocol_versions,
        supported_features: vec![features::GOSSIP.into(), features::HISTORY.into()],
        signature: String::new(),
        limitation: None,
        retention: None,
        payments_url: None,
        invites_url: None,
        status: Some("ok".into()),
        status_detail: None,
    };
    sign_capability_doc(&mut info, id).expect("sign capability doc");
    info
}

fn doc_json(id: &Identity, protocol_versions: Vec<u16>) -> String {
    serde_json::to_string(&signed_doc(id, protocol_versions)).expect("serialize doc")
}

// ───── 1. SyncProvider enumeration ────────────────────────────────────────

/// A live, un-revoked `SyncProvider` grant makes the target a candidate; a
/// peer whose grant is later revoked is excluded; the owner (admin, but never
/// explicitly granted) is **not** enumerated.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enumerates_live_grants_excludes_revoked_and_implicit_admin() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (client, _broker) = test_client();
            let owner = client.identity().endpoint_id();

            let provider_a = Identity::generate().endpoint_id();
            let provider_b = Identity::generate().endpoint_id();

            // Grant both A and B SyncProvider.
            client
                .mutations()
                .grant_permission(provider_a, Permission::SyncProvider)
                .await
                .expect("grant A");
            client
                .mutations()
                .grant_permission(provider_b, Permission::SyncProvider)
                .await
                .expect("grant B");
            // Revoke B's grant.
            client
                .mutations()
                .revoke_permission(provider_b, Permission::SyncProvider)
                .await
                .expect("revoke B");

            let providers = client.sync_providers().await;

            assert!(
                providers.contains(&provider_a),
                "a live SyncProvider grant must be enumerated"
            );
            assert!(
                !providers.contains(&provider_b),
                "a revoked SyncProvider grant must be excluded"
            );
            assert!(
                !providers.contains(&owner),
                "the owner is an implicit admin but holds no explicit \
                 SyncProvider grant, so it is not a candidate sync source"
            );
            assert_eq!(
                providers.len(),
                1,
                "exactly one live SyncProvider remains after the revoke"
            );
        })
        .await;
}

/// The pure enumerator over a materialized `ServerState` ignores *other*
/// non-`SyncProvider` permissions (e.g. `ManageChannels`).
#[test]
fn enumerator_ignores_non_syncprovider_grants() {
    use std::collections::{BTreeMap, BTreeSet};

    let sync_peer = Identity::generate().endpoint_id();
    let other_peer = Identity::generate().endpoint_id();

    let mut state = willow_state::ServerState::new(
        "srv".to_string(),
        "srv".to_string(),
        EndpointId::from_bytes(&[1u8; 32]).unwrap(),
    );
    let mut perms_sync = BTreeSet::new();
    perms_sync.insert(Permission::SyncProvider);
    let mut perms_other = BTreeSet::new();
    perms_other.insert(Permission::ManageChannels);
    let mut map = BTreeMap::new();
    map.insert(sync_peer, perms_sync);
    map.insert(other_peer, perms_other);
    state.peer_permissions = map;

    let providers = enumerate_sync_providers(&state);
    assert!(providers.contains(&sync_peer));
    assert!(
        !providers.contains(&other_peer),
        "a peer with only ManageChannels is not a sync provider"
    );
}

// ───── 2. fetch_relay_info: verify + protocol intersection ────────────────

/// A doc whose signature verifies and whose protocol versions intersect the
/// client's is accepted and returned verbatim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_accepts_valid_intersecting_doc() {
    let relay = Identity::generate();
    let body = doc_json(&relay, vec![2]);

    let info = fetch_relay_info("https://relay.test/.well-known/willow", &[2], |_url| {
        let body = body.clone();
        async move { Ok(body) }
    })
    .await
    .expect("a valid, intersecting doc must be accepted");

    assert_eq!(info.pubkey, hex::encode(relay.endpoint_id().as_bytes()));
    assert_eq!(info.protocol_versions, vec![2]);
}

/// A doc whose signature does NOT verify is refused with `BadSignature`,
/// proving `fetch_relay_info` routes through PR 1's `verify_capability_doc`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_refuses_doc_with_bad_signature() {
    let relay = Identity::generate();
    // Sign a valid doc, then tamper a signed field so the detached
    // signature no longer covers the canonical bytes.
    let mut info = signed_doc(&relay, vec![2]);
    info.status = Some("read_only".into()); // attacker flips a field post-signing
    let tampered = serde_json::to_string(&info).expect("serialize tampered doc");

    let err = fetch_relay_info("https://relay.test/.well-known/willow", &[2], |_url| {
        let tampered = tampered.clone();
        async move { Ok(tampered) }
    })
    .await
    .expect_err("a doc whose signature fails must be refused");

    assert!(
        matches!(err, RelayInfoError::BadSignature),
        "expected BadSignature, got {err:?}"
    );
}

/// A correctly-signed doc whose `protocol_versions` do not intersect the
/// client's is refused with `ProtocolMismatch` — the client and relay cannot
/// speak a common wire version.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_refuses_non_intersecting_protocol() {
    let relay = Identity::generate();
    // Relay advertises only v99; client speaks only v2.
    let body = doc_json(&relay, vec![99]);

    let err = fetch_relay_info("https://relay.test/.well-known/willow", &[2], |_url| {
        let body = body.clone();
        async move { Ok(body) }
    })
    .await
    .expect_err("a doc with no shared protocol version must be refused");

    assert!(
        matches!(err, RelayInfoError::ProtocolMismatch { .. }),
        "expected ProtocolMismatch, got {err:?}"
    );
}

/// A transport-level failure surfaces as `Fetch`, distinct from the
/// verification refusals above.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_propagates_transport_error() {
    let err = fetch_relay_info(
        "https://relay.test/.well-known/willow",
        &[2],
        |_url| async move { Err("connection refused".to_string()) },
    )
    .await
    .expect_err("a transport failure must surface");

    assert!(
        matches!(err, RelayInfoError::Fetch(_)),
        "expected Fetch, got {err:?}"
    );
}

// ───── 3. Ranking: refuse non-SyncProvider endpoints ──────────────────────

/// The ranker latency-orders candidates that are live `SyncProvider`s and
/// drops any endpoint that is not — even if its capability doc is valid.
#[test]
fn ranker_refuses_non_syncprovider_then_orders_by_latency() {
    let trusted_slow = Identity::generate().endpoint_id();
    let trusted_fast = Identity::generate().endpoint_id();
    let untrusted = Identity::generate().endpoint_id();

    let mk = |id: EndpointId| signed_doc(&Identity::generate(), vec![2]).tap_pubkey(id);

    let candidates = vec![
        RelayCandidate {
            endpoint: trusted_slow,
            is_sync_provider: true,
            info: mk(trusted_slow),
            latency: Duration::from_millis(200),
        },
        RelayCandidate {
            endpoint: untrusted,
            is_sync_provider: false, // valid doc, but NOT a SyncProvider
            info: mk(untrusted),
            latency: Duration::from_millis(1), // fastest, must still be dropped
        },
        RelayCandidate {
            endpoint: trusted_fast,
            is_sync_provider: true,
            info: mk(trusted_fast),
            latency: Duration::from_millis(50),
        },
    ];

    let ranked = rank_relay_candidates(candidates);

    let ids: Vec<EndpointId> = ranked.iter().map(|r| r.endpoint).collect();
    assert!(
        !ids.contains(&untrusted),
        "a non-SyncProvider endpoint must never be ranked as authoritative, \
         even with the lowest latency"
    );
    assert_eq!(
        ids,
        vec![trusted_fast, trusted_slow],
        "trusted candidates are ordered fastest-first"
    );
}

/// Trivial helper to set a doc's `pubkey` to a specific endpoint for the
/// ranker test (the ranker keys on `endpoint`, not on the doc signature, so an
/// independently-signed sample suffices).
trait TapPubkey {
    fn tap_pubkey(self, id: EndpointId) -> Self;
}
impl TapPubkey for WillowRelayInfo {
    fn tap_pubkey(mut self, id: EndpointId) -> Self {
        self.pubkey = hex::encode(id.as_bytes());
        self
    }
}
