//! Peer scaling tests — measure performance as peer count increases.
//!
//! These tests spin up N real libp2p nodes on localhost, connect them
//! in a star topology (all dial node 0), and measure:
//! - Connection time
//! - Message delivery time (gossipsub flood to all peers)
//! - State event application throughput

#![allow(deprecated)]

use std::time::{Duration, Instant};

use tokio::time::timeout;

use willow_identity::Identity;
use willow_network::{NetworkConfig, NetworkEvent, NetworkNode};

const TEST_TIMEOUT: Duration = Duration::from_secs(30);

struct TestNode {
    node: NetworkNode,
    events: tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
    #[allow(dead_code)]
    identity: Identity,
}

async fn start_test_node() -> TestNode {
    let identity = Identity::generate();
    let config = NetworkConfig::default();
    let (node, events) = NetworkNode::start(identity.clone(), config)
        .await
        .expect("node should start");
    TestNode {
        node,
        events,
        identity,
    }
}

/// Wait for a Listening event and return the address.
/// Wait for at least `count` PeerConnected events.
async fn wait_for_peers(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
    count: usize,
) {
    let mut connected = 0;
    while connected < count {
        let event = timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("timed out waiting for peers")
            .expect("closed");
        if matches!(event, NetworkEvent::PeerConnected(_)) {
            connected += 1;
        }
    }
}

/// Wait for a gossipsub message.
async fn wait_for_message(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
) -> Option<(String, Vec<u8>)> {
    loop {
        match timeout(Duration::from_secs(10), events.recv()).await {
            Ok(Some(NetworkEvent::Message { topic, data, .. })) => {
                return Some((topic, data));
            }
            Ok(Some(_)) => continue,
            _ => return None,
        }
    }
}

/// Spin up N peers in a star topology (all connect to peer 0).
/// Returns (nodes, hub_addr, connection_time_ms).
async fn create_star_topology(n: usize) -> (Vec<TestNode>, Duration) {
    let start = Instant::now();

    // Start all nodes.
    let mut nodes = Vec::new();
    for _ in 0..n {
        nodes.push(start_test_node().await);
    }

    // Get hub's listen address.
    let hub_addr = loop {
        let event = timeout(TEST_TIMEOUT, nodes[0].events.recv())
            .await
            .expect("timed out")
            .expect("closed");
        if let NetworkEvent::Listening(addr) = event {
            break addr;
        }
    };

    // All other nodes dial the hub.
    for i in 1..n {
        nodes[i].node.dial(hub_addr.clone()).expect("dial failed");
    }

    // Wait for hub to see all peers.
    wait_for_peers(&mut nodes[0].events, n - 1).await;

    let connect_time = start.elapsed();
    (nodes, connect_time)
}

#[tokio::test]
async fn scale_5_peers_connect() {
    let (nodes, connect_time) = create_star_topology(5).await;
    println!(
        "5 peers connected in {:?} ({:.0}ms per peer)",
        connect_time,
        connect_time.as_millis() as f64 / 5.0
    );
    assert!(connect_time < Duration::from_secs(10));
    drop(nodes);
}

#[tokio::test]
async fn scale_10_peers_connect() {
    let (nodes, connect_time) = create_star_topology(10).await;
    println!(
        "10 peers connected in {:?} ({:.0}ms per peer)",
        connect_time,
        connect_time.as_millis() as f64 / 10.0
    );
    assert!(connect_time < Duration::from_secs(15));
    drop(nodes);
}

#[tokio::test]
async fn scale_20_peers_connect() {
    let (nodes, connect_time) = create_star_topology(20).await;
    println!(
        "20 peers connected in {:?} ({:.0}ms per peer)",
        connect_time,
        connect_time.as_millis() as f64 / 20.0
    );
    assert!(connect_time < Duration::from_secs(20));
    drop(nodes);
}

#[tokio::test]
async fn scale_5_peers_message_flood() {
    let (mut nodes, _) = create_star_topology(5).await;
    let topic = format!("flood-5-{}", uuid::Uuid::new_v4());

    // All subscribe.
    for node in &nodes {
        node.node.subscribe(&topic).expect("subscribe");
    }
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Peer 0 sends a message.
    let start = Instant::now();
    let payload = b"hello from hub".to_vec();
    nodes[0]
        .node
        .publish(&topic, payload.clone())
        .expect("publish");

    // Count how many peers receive it within 5 seconds.
    let mut received = 0;
    for i in 1..5 {
        if wait_for_message(&mut nodes[i].events).await.is_some() {
            received += 1;
        }
    }
    let deliver_time = start.elapsed();

    println!(
        "5 peers: {received}/4 received in {:?} ({:.0}ms avg)",
        deliver_time,
        deliver_time.as_millis() as f64 / received.max(1) as f64
    );
    assert!(
        received >= 3,
        "at least 3/4 peers should receive the message"
    );
    drop(nodes);
}

#[tokio::test]
async fn scale_10_peers_message_flood() {
    let (mut nodes, _) = create_star_topology(10).await;
    let topic = format!("flood-10-{}", uuid::Uuid::new_v4());

    for node in &nodes {
        node.node.subscribe(&topic).expect("subscribe");
    }
    tokio::time::sleep(Duration::from_millis(2000)).await;

    let start = Instant::now();
    nodes[0]
        .node
        .publish(&topic, b"hello 10".to_vec())
        .expect("publish");

    let mut received = 0;
    for i in 1..10 {
        if wait_for_message(&mut nodes[i].events).await.is_some() {
            received += 1;
        }
    }
    let deliver_time = start.elapsed();

    println!(
        "10 peers: {received}/9 received in {:?} ({:.0}ms avg)",
        deliver_time,
        deliver_time.as_millis() as f64 / received.max(1) as f64
    );
    assert!(
        received >= 7,
        "at least 7/9 peers should receive the message"
    );
    drop(nodes);
}

#[tokio::test]
async fn scale_event_application_throughput() {
    use willow_app::willow_state::*;

    // Pure state machine throughput — no networking.
    let mut state = ServerState::new("server-1", "Test Server", "owner");

    // Create a channel first.
    let create_ch = Event {
        id: "ch-create".to_string(),
        parent_hash: state.hash(),
        author: "owner".to_string(),
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-1".to_string(),
        },
    };
    apply_lenient(&mut state, &create_ch);

    // Measure applying 10,000 message events.
    let start = Instant::now();
    let count = 10_000;
    for i in 0..count {
        let event = Event {
            id: format!("msg-{i}"),
            parent_hash: StateHash::ZERO, // lenient mode
            author: format!("peer-{}", i % 20),
            timestamp_ms: 1000 + i as u64,
            kind: EventKind::Message {
                channel_id: "ch-1".to_string(),
                body: format!("Message number {i}"),
            },
        };
        apply_lenient(&mut state, &event);
    }
    let elapsed = start.elapsed();

    let events_per_sec = count as f64 / elapsed.as_secs_f64();
    println!(
        "Applied {count} events in {:?} ({:.0} events/sec)",
        elapsed, events_per_sec
    );

    assert_eq!(state.messages.len(), count);
    assert!(
        events_per_sec > 10_000.0,
        "should handle at least 10k events/sec"
    );
}

#[tokio::test]
async fn scale_merge_throughput() {
    use willow_app::willow_state::*;

    let mut state = ServerState::new("server-1", "Merge Test", "owner");
    let create_ch = Event {
        id: "ch-create".to_string(),
        parent_hash: state.hash(),
        author: "owner".to_string(),
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-1".to_string(),
        },
    };
    apply_lenient(&mut state, &create_ch);

    // Create two divergent histories with 500 events each.
    let mut events_a: Vec<Event> = Vec::new();
    let mut events_b: Vec<Event> = Vec::new();

    for i in 0..500 {
        events_a.push(Event {
            id: format!("a-{i}"),
            parent_hash: state.hash(),
            author: "peer-a".to_string(),
            timestamp_ms: 2000 + i as u64,
            kind: EventKind::Message {
                channel_id: "ch-1".to_string(),
                body: format!("From A: {i}"),
            },
        });
        events_b.push(Event {
            id: format!("b-{i}"),
            parent_hash: state.hash(),
            author: "peer-b".to_string(),
            timestamp_ms: 2000 + i as u64,
            kind: EventKind::Message {
                channel_id: "ch-1".to_string(),
                body: format!("From B: {i}"),
            },
        });
    }

    let start = Instant::now();
    let (merged, canonical) = willow_app::willow_state::merge(&events_a, &events_b, &state);
    let elapsed = start.elapsed();

    println!(
        "Merged 500+500 events in {:?} ({} canonical events, {} messages)",
        elapsed,
        canonical.len(),
        merged.messages.len()
    );

    assert_eq!(merged.messages.len(), 1000);
    assert!(elapsed < Duration::from_secs(2), "merge should be fast");
}
