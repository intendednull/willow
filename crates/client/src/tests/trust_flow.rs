//! Trust-flow client tests.
//!
//! These tests cover the behaviours that used to live in
//! `e2e/permissions.spec.ts` as `owner trusts peer`,
//! `trusted peer messages are visible`, `owner untrusts peer`, and
//! `untrusted messages rejected after untrust`.
//!
//! The local `PeerTrust` belief is a per-device UI concern, so tests
//! 1 and 3 exercise [`ClientHandle::verify_peer`] and
//! [`ClientHandle::mark_unverified`] directly on the in-memory
//! [`InMemoryTrustStore`]. The paired "messages visible / rejected"
//! behaviours (2, 4) need a real two-peer gossip path and so wire two
//! `ClientHandle`s through a shared `MemHub`. Rejection specifically
//! uses [`ClientMutations::revoke_permission`] of
//! [`willow_state::Permission::SendMessages`], matching the
//! `propose_revoke_admin` path the UI's "Untrust" button takes.
//!
//! DOM-side rendering of the verified / unverified trust badge lives in
//! `crates/web/tests/browser.rs` — these tests stay focused on client
//! state transitions.

use std::sync::Arc;
use std::time::Duration;

use crate::trust::{InMemoryTrustStore, PeerTrust, TrustStoreHandle, UnverifiedReason};
use crate::{test_client, ClientEvent, ClientHandle, EventReceiver};

use willow_actor::Addr;
use willow_actor::Broker;
use willow_network::mem::{MemHub, MemNetwork};

/// Attach a fresh [`InMemoryTrustStore`] to a test client and return the
/// store handle so the test can observe its state.
fn attach_trust_store<N: willow_network::Network>(
    client: ClientHandle<N>,
) -> (ClientHandle<N>, TrustStoreHandle) {
    let store: TrustStoreHandle = Arc::new(InMemoryTrustStore::new());
    let client = client.with_trust_store(Arc::clone(&store));
    (client, store)
}

/// Build two connected clients on a shared [`MemHub`].
///
/// * Alice owns the server (`test_client()` genesis author).
/// * Bob's DAG is replayed from Alice's events and includes a
///   `GrantPermission(SendMessages)` so Bob can author messages that
///   Alice accepts.
/// * Both clients connect to the same `MemHub` so their gossip topics
///   meet.
async fn connected_pair() -> (
    ClientHandle<MemNetwork>,
    Addr<Broker<ClientEvent>>,
    ClientHandle<MemNetwork>,
    Addr<Broker<ClientEvent>>,
) {
    let hub = Arc::new(MemHub::new());

    // Alice — server owner.
    let (mut alice, alice_broker) = test_client();
    let alice_net = MemNetwork::new(&hub);
    alice.connect(alice_net).await;

    // Bob — fresh identity, no server state yet.
    let (mut bob, bob_broker) = test_client();
    let bob_peer = bob.identity.endpoint_id();

    // Alice grants Bob SendMessages so messages Bob authors after the
    // DAG replay are accepted when gossiped back to Alice.
    alice
        .mutations()
        .grant_permission(bob_peer, willow_state::Permission::SendMessages)
        .await
        .expect("grant SendMessages");

    // Snapshot Alice's full DAG (genesis + CreateChannel + GrantPermission).
    let alice_events: Vec<willow_state::Event> =
        willow_actor::state::select(&alice.dag_addr, |ds| {
            ds.managed
                .dag()
                .topological_sort()
                .into_iter()
                .cloned()
                .collect()
        })
        .await;

    // Replay into Bob so the two peers start from an identical state.
    let events_for_bob = alice_events.clone();
    willow_actor::state::mutate(&bob.dag_addr, move |ds| {
        ds.managed = willow_state::ManagedDag::empty(5000);
        for event in events_for_bob {
            ds.managed.insert_and_apply(event).ok();
        }
        ds.invalidate_sync_reply_cache();
    })
    .await;
    let bob_state =
        willow_actor::state::select(&bob.dag_addr, |ds| ds.managed.state().clone()).await;
    willow_actor::state::mutate(&bob.event_state_addr, move |es| {
        *es = bob_state;
    })
    .await;

    let bob_net = MemNetwork::new(&hub);
    bob.connect(bob_net).await;

    (alice, alice_broker, bob, bob_broker)
}

/// Drain any pending events from `rx` for a short deadline so the next
/// assertion starts from an empty queue. Events are discarded.
async fn drain_events(rx: &mut EventReceiver, deadline: Duration) {
    let _ = tokio::time::timeout(deadline, async {
        loop {
            match rx.try_recv() {
                Some(_) => {}
                None => tokio::task::yield_now().await,
            }
        }
    })
    .await;
}

/// Poll Alice's event store until a message with `body` appears on the
/// `general` channel or `deadline` elapses.
async fn wait_for_message<N: willow_network::Network>(
    client: &ClientHandle<N>,
    body: &str,
    deadline: Duration,
) -> bool {
    tokio::time::timeout(deadline, async {
        loop {
            let general_id = willow_actor::state::select(&client.event_state_addr, |es| {
                es.channels
                    .iter()
                    .find(|(_, ch)| ch.name == "general")
                    .map(|(id, _)| id.clone())
                    .unwrap_or_default()
            })
            .await;
            if !general_id.is_empty()
                && client
                    .event_messages(&general_id)
                    .await
                    .iter()
                    .any(|m| m.body == body)
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

// ── 1. owner trusts peer — trust state transitions Unknown → Verified ──

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_peer_transitions_unknown_to_verified() {
    let (client, _rx) = test_client();
    let (client, store) = attach_trust_store(client);
    let bob = willow_identity::Identity::generate().endpoint_id();
    let bob_str = bob.to_string();

    assert_eq!(
        store.get(&bob_str),
        PeerTrust::Unknown,
        "peer should start as Unknown before any verify call"
    );

    client.verify_peer(&bob_str);

    assert!(
        store.get(&bob_str).is_verified(),
        "verify_peer must transition the store to Verified"
    );
    match store.get(&bob_str) {
        PeerTrust::Verified { pinned_key, .. } => {
            assert_eq!(
                &pinned_key,
                bob.as_bytes(),
                "pinned key must match the peer's Ed25519 public key bytes"
            );
        }
        other => panic!("unexpected trust state after verify_peer: {other:?}"),
    }
    assert!(
        client.trust_state(&bob_str).is_verified(),
        "client.trust_state must reflect the store mutation"
    );
}

// ── 2. trusted peer messages are visible (two-peer MemNetwork path) ──

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trusted_peer_messages_reach_owner() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, alice_broker, bob, _bob_broker) = connected_pair().await;
            let (alice, _alice_store) = attach_trust_store(alice);

            // Alice marks Bob verified locally — the spec's trust flow is
            // independent of the DAG, but this mirrors the e2e scenario.
            let bob_peer = bob.identity.endpoint_id();
            alice.verify_peer(&bob_peer.to_string());
            assert!(
                alice.trust_state(&bob_peer.to_string()).is_verified(),
                "Alice should see Bob as Verified locally"
            );

            // Bob authors a message in the shared general channel.
            bob.send_message("general", "trusted message")
                .await
                .expect("Bob can send after GrantPermission(SendMessages)");

            // Alice observes it via gossip within a generous deadline.
            let arrived = wait_for_message(&alice, "trusted message", Duration::from_secs(5)).await;
            assert!(
                arrived,
                "Alice must receive Bob's trusted message via gossip"
            );
            // Silence unused-variable lint on the broker we deliberately
            // don't subscribe to in this test — it stays live so the
            // actor system doesn't tear down under us.
            let _ = alice_broker;
        })
        .await;
}

// ── 3. owner untrusts peer — Verified → Unverified ──

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mark_unverified_transitions_verified_to_unverified() {
    let (client, _rx) = test_client();
    let (client, store) = attach_trust_store(client);
    let bob = willow_identity::Identity::generate().endpoint_id();
    let bob_str = bob.to_string();

    // Start by verifying Bob, then mark him unverified.
    client.verify_peer(&bob_str);
    assert!(
        store.get(&bob_str).is_verified(),
        "precondition: peer starts Verified"
    );

    client.mark_unverified(&bob_str, UnverifiedReason::SasMismatch);

    match store.get(&bob_str) {
        PeerTrust::Unverified { reason } => {
            assert_eq!(reason, UnverifiedReason::SasMismatch);
        }
        other => panic!("mark_unverified should leave peer Unverified, got {other:?}"),
    }
    assert!(
        !client.trust_state(&bob_str).is_verified(),
        "trust_state must no longer report Verified"
    );
    assert!(
        client.trust_state(&bob_str).is_unverified(),
        "trust_state must report the amber (unverified) bucket"
    );
}

// ── 4. revoke SendMessages rejects future messages (two-peer) ──

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_peer_messages_are_dropped() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let (alice, alice_broker, bob, _bob_broker) = connected_pair().await;

            let mut alice_rx = EventReceiver::subscribe(&alice_broker, &alice.system).await;

            // Baseline: Bob can send, Alice receives.
            bob.send_message("general", "before untrust")
                .await
                .expect("Bob still has SendMessages at this point");

            let saw_baseline =
                wait_for_message(&alice, "before untrust", Duration::from_secs(5)).await;
            assert!(
                saw_baseline,
                "baseline: Bob's first message must arrive before untrust"
            );

            // Drain the broker so follow-up assertions start fresh.
            drain_events(&mut alice_rx, Duration::from_millis(100)).await;

            // Alice revokes Bob's SendMessages — the "full untrust" path.
            // This matches what `propose_revoke_admin` does in the UI.
            let bob_peer = bob.identity.endpoint_id();
            alice
                .mutations()
                .revoke_permission(bob_peer, willow_state::Permission::SendMessages)
                .await
                .expect("revoke_permission as owner");

            // Give the revoke event time to propagate into Bob's DAG via
            // the shared gossip topic so his next send_message applies
            // against the post-revoke state.
            let revoke_landed = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let has = willow_actor::state::select(&bob.event_state_addr, move |es| {
                        es.has_permission(&bob_peer, &willow_state::Permission::SendMessages)
                    })
                    .await;
                    if !has {
                        return;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .is_ok();
            assert!(
                revoke_landed,
                "Bob's state must reflect the revoke before he tries to send again"
            );

            // Bob tries to send — must fail at the author-side permission
            // check and therefore never reach Alice's message store.
            let send_result = bob.send_message("general", "after untrust secret").await;
            assert!(
                send_result.is_err(),
                "revoked peer must get PermissionDenied on send, got {send_result:?}"
            );

            // Sentinel: Alice sends her own message. The owner always has
            // rights; observing this proves real time has passed so the
            // following negative assertion isn't racing an un-flushed
            // network path.
            alice
                .send_message("general", "alice sentinel")
                .await
                .expect("owner can always send");

            let sentinel = wait_for_message(&alice, "alice sentinel", Duration::from_secs(5)).await;
            assert!(sentinel, "Alice's own sentinel must land in her store");

            // Assert Bob's post-revoke message never appeared on Alice's side.
            let general_id = willow_actor::state::select(&alice.event_state_addr, |es| {
                es.channels
                    .iter()
                    .find(|(_, ch)| ch.name == "general")
                    .map(|(id, _)| id.clone())
                    .unwrap_or_default()
            })
            .await;
            assert!(!general_id.is_empty(), "general channel must exist");
            let bodies: Vec<String> = alice
                .event_messages(&general_id)
                .await
                .into_iter()
                .map(|m| m.body)
                .collect();
            assert!(
                !bodies.contains(&"after untrust secret".to_string()),
                "revoked message must not appear in Alice's store, got: {bodies:?}"
            );
        })
        .await;
}
