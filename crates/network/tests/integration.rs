//! Integration tests for [`IrohNetwork`].
//!
//! These tests verify gossip delivery, topic isolation, and disconnect
//! behavior using real iroh endpoints connected via in-memory address
//! lookup (no relay, no mDNS).
//!
//! All tests in this file are marked `#[ignore]` because they spin up
//! real iroh UDP endpoints and rely on dual-stack loopback, gossip
//! NeighborUp/Down signaling, and bind timing. GitHub Actions runners
//! have flaky dual-stack networking (see PR #360), and these tests
//! intermittently fail there even though they pass reliably locally.
//!
//! Run them locally with:
//!     cargo test -p willow-network --test integration -- --ignored
//!
//! In CI they are run by a dedicated `network-integration` job that is
//! allowed to fail without blocking PR merges.

use std::time::Duration;

use bytes::Bytes;
use futures_lite::StreamExt;
use iroh::address_lookup::memory::MemoryLookup;
use iroh::endpoint::presets;
use tokio::time::timeout;
use willow_identity::Identity;
use willow_network::iroh::{Config, IrohNetwork};
use willow_network::topics::topic_id;
use willow_network::traits::*;

/// Timeout for waiting on events.
const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

// ───── Helper: raw gossip node for two-node tests ──────────────────────────

/// A test node built from raw iroh components (since `IrohNetwork::new`
/// does not support custom `MemoryLookup` for peer discovery).
struct TestNode {
    id: iroh_base::EndpointId,
    endpoint: iroh::Endpoint,
    gossip: iroh_gossip::Gossip,
    router: iroh::protocol::Router,
}

impl TestNode {
    async fn shutdown(self) {
        let _ = self.router.shutdown().await;
        self.endpoint.close().await;
    }
}

/// Create two connected gossip test nodes.
async fn create_two_nodes() -> (TestNode, TestNode) {
    let discovery = MemoryLookup::new();

    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let ep_a = iroh::Endpoint::builder(presets::Minimal)
        .secret_key(id_a.secret_key().clone())
        .relay_mode(iroh::RelayMode::Disabled)
        .address_lookup(discovery.clone())
        .bind()
        .await
        .unwrap();

    let ep_b = iroh::Endpoint::builder(presets::Minimal)
        .secret_key(id_b.secret_key().clone())
        .relay_mode(iroh::RelayMode::Disabled)
        .address_lookup(discovery.clone())
        .bind()
        .await
        .unwrap();

    // Register each endpoint's address info in the shared discovery
    // so they can find each other without a relay.
    discovery.add_endpoint_info(ep_a.addr());
    discovery.add_endpoint_info(ep_b.addr());

    let gossip_a = iroh_gossip::Gossip::builder()
        .max_message_size(65536)
        .spawn(ep_a.clone());

    let gossip_b = iroh_gossip::Gossip::builder()
        .max_message_size(65536)
        .spawn(ep_b.clone());

    let router_a = iroh::protocol::Router::builder(ep_a.clone())
        .accept(iroh_gossip::ALPN, gossip_a.clone())
        .spawn();

    let router_b = iroh::protocol::Router::builder(ep_b.clone())
        .accept(iroh_gossip::ALPN, gossip_b.clone())
        .spawn();

    let node_a = TestNode {
        id: ep_a.id(),
        endpoint: ep_a,
        gossip: gossip_a,
        router: router_a,
    };

    let node_b = TestNode {
        id: ep_b.id(),
        endpoint: ep_b,
        gossip: gossip_b,
        router: router_b,
    };

    (node_a, node_b)
}

/// Drain events from a [`GossipReceiver`] until a `Received` message arrives.
async fn next_received_gossip(
    receiver: &mut iroh_gossip::api::GossipReceiver,
) -> iroh_gossip::api::Message {
    loop {
        let event = timeout(EVENT_TIMEOUT, receiver.next())
            .await
            .expect("timed out waiting for gossip message")
            .expect("stream ended")
            .expect("event error");
        if let iroh_gossip::api::Event::Received(msg) = event {
            return msg;
        }
    }
}

// ───── Gossip exchange tests ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real iroh UDP — flaky on GHA dual-stack; run with --ignored locally"]
async fn two_nodes_exchange_gossip() {
    let (node_a, node_b) = create_two_nodes().await;
    let topic = topic_id("integration-test");

    // A subscribes first (no bootstrap).
    let topic_a = node_a.gossip.subscribe(topic, vec![]).await.unwrap();
    let (sender_a, mut receiver_a) = topic_a.split();

    // B subscribes with A as bootstrap.
    let topic_b = node_b
        .gossip
        .subscribe(topic, vec![node_a.id])
        .await
        .unwrap();
    let (sender_b, mut receiver_b) = topic_b.split();

    // Wait for B to join (establish connection to A).
    timeout(EVENT_TIMEOUT, receiver_b.joined())
        .await
        .expect("timed out waiting for B to join")
        .unwrap();

    // A sends a message.
    sender_a
        .broadcast(Bytes::from("hello from A"))
        .await
        .unwrap();

    // B should receive it.
    let msg = next_received_gossip(&mut receiver_b).await;
    assert_eq!(msg.content.as_ref(), b"hello from A");

    // B sends a message back.
    sender_b
        .broadcast(Bytes::from("hello from B"))
        .await
        .unwrap();

    // A should receive it.
    let msg = next_received_gossip(&mut receiver_a).await;
    assert_eq!(msg.content.as_ref(), b"hello from B");

    drop(sender_a);
    drop(sender_b);
    drop(receiver_a);
    drop(receiver_b);
    node_a.shutdown().await;
    node_b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real iroh UDP — flaky on GHA dual-stack; run with --ignored locally"]
async fn topic_isolation() {
    let (node_a, node_b) = create_two_nodes().await;

    let topic_x = topic_id("topic-x");
    let topic_y = topic_id("topic-y");

    // A subscribes to topic_x, B subscribes to topic_y.
    let topic_a = node_a.gossip.subscribe(topic_x, vec![]).await.unwrap();
    let (sender_a, _receiver_a) = topic_a.split();

    let topic_b = node_b
        .gossip
        .subscribe(topic_y, vec![node_a.id])
        .await
        .unwrap();
    let (_sender_b, mut receiver_b) = topic_b.split();

    // Give a moment for any connections to attempt.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // A sends on topic_x.
    sender_a
        .broadcast(Bytes::from("wrong topic"))
        .await
        .unwrap();

    // B on topic_y should NOT receive it.
    let result = timeout(
        Duration::from_millis(500),
        next_received_gossip(&mut receiver_b),
    )
    .await;
    assert!(
        result.is_err(),
        "should have timed out — topics are isolated"
    );

    node_a.shutdown().await;
    node_b.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real iroh UDP — flaky on GHA dual-stack; run with --ignored locally"]
async fn node_disconnect_detected() {
    let (node_a, node_b) = create_two_nodes().await;
    let topic = topic_id("disconnect-test");

    // A subscribes.
    let topic_a = node_a.gossip.subscribe(topic, vec![]).await.unwrap();
    let (_sender_a, mut receiver_a) = topic_a.split();

    // B subscribes with A as bootstrap.
    let topic_b = node_b
        .gossip
        .subscribe(topic, vec![node_a.id])
        .await
        .unwrap();
    let (_sender_b, mut receiver_b) = topic_b.split();

    // Wait for connection.
    timeout(EVENT_TIMEOUT, receiver_b.joined())
        .await
        .expect("timed out waiting for join")
        .unwrap();

    // Drain A's initial neighbor events (NeighborUp from B joining).
    let _ = timeout(Duration::from_secs(2), receiver_a.next()).await;

    // B shuts down by dropping all its handles and closing the endpoint.
    drop(_sender_b);
    drop(receiver_b);
    node_b.shutdown().await;

    // After B shuts down, A must eventually observe the disconnect as either
    // a NeighborDown event or a closed stream. Timing out here means the
    // disconnect signaling is broken — that should be a test failure.
    let detected = timeout(Duration::from_secs(10), async {
        loop {
            match receiver_a.next().await {
                Some(Ok(iroh_gossip::api::Event::NeighborDown(_))) => return true,
                Some(_) => continue, // drain other events
                None => return true, // stream closed — also counts as detection
            }
        }
    })
    .await
    .expect("timed out: A did not detect B's disconnect within 10 s");

    assert!(detected, "expected disconnect detection but got false");

    node_a.shutdown().await;
}

// ───── IrohNetwork trait-level tests ──────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real iroh UDP — flaky on GHA dual-stack; run with --ignored locally"]
async fn iroh_network_new_and_shutdown() {
    let identity = Identity::generate();
    let config = Config {
        secret_key: identity.secret_key().clone(),
        relay_url: None,
        bootstrap_peers: vec![],
        mdns: false,
    };
    let network = IrohNetwork::new(config).await.unwrap();
    let _id = network.id();
    network.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "real iroh UDP — flaky on GHA dual-stack; run with --ignored locally"]
async fn iroh_network_blob_store() {
    let identity = Identity::generate();
    let config = Config {
        secret_key: identity.secret_key().clone(),
        relay_url: None,
        bootstrap_peers: vec![],
        mdns: false,
    };
    let network = IrohNetwork::new(config).await.unwrap();

    let data = Bytes::from("blob test data");
    let hash = network.blobs().add(data.clone()).await.unwrap();
    assert!(network.blobs().has(hash).await);
    assert_eq!(network.blobs().get(hash).await.unwrap(), Some(data));

    let removed = network.blobs().remove(hash).await.unwrap();
    assert!(removed);
    assert!(!network.blobs().has(hash).await);

    network.shutdown().await.unwrap();
}
