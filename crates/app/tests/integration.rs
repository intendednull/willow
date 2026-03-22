//! Full integration tests that spin up real network nodes and verify
//! end-to-end message flow through the entire stack:
//!
//! identity → messaging → transport → network (libp2p) → deserialize → verify
//!
//! These tests use localhost TCP with direct dialing for deterministic
//! peer discovery.

use std::time::Duration;

use tokio::time::timeout;

use willow_identity::Identity;
use willow_messaging::hlc::HLC;
use willow_messaging::{ChannelId, Content, Message};
use willow_network::{NetworkConfig, NetworkEvent, NetworkNode};
use willow_transport::{pack_envelope, unpack_envelope, MessageType};

/// Timeout for network operations in tests.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Start a network node with default config (localhost, random port).
async fn start_node(
    identity: &Identity,
) -> (
    NetworkNode,
    tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
) {
    let config = NetworkConfig::default();
    NetworkNode::start(identity.clone(), config)
        .await
        .expect("node should start")
}

/// Wait for a Listening event and dial the address from the other node.
/// Returns after both sides report a connection.
async fn connect_nodes(
    node_a: &NetworkNode,
    events_a: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
    node_b: &NetworkNode,
    events_b: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
) {
    // Get node A's listen address.
    let addr_a = loop {
        let event = timeout(TEST_TIMEOUT, events_a.recv())
            .await
            .expect("timed out waiting for listen addr")
            .expect("channel closed");
        if let NetworkEvent::Listening(addr) = event {
            break addr;
        }
    };

    // B dials A.
    node_b.dial(addr_a).expect("dial should succeed");

    // Wait for connection on both sides.
    wait_for_connect(events_a).await;
    wait_for_connect(events_b).await;

    let _ = (node_a, node_b);
}

/// Wait for a PeerConnected event.
async fn wait_for_connect(events: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>) {
    loop {
        let event = timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("timed out waiting for connection")
            .expect("channel closed");
        if matches!(event, NetworkEvent::PeerConnected(_)) {
            return;
        }
    }
}

/// Wait for a gossipsub message, ignoring other events.
async fn wait_for_message(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
) -> (String, Vec<u8>) {
    loop {
        let event = timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("timed out waiting for message")
            .expect("channel closed");
        if let NetworkEvent::Message { topic, data, .. } = event {
            return (topic, data);
        }
    }
}

// ───── Tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn two_nodes_connect_via_direct_dial() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;

    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;
    // If we get here without timeout, the nodes are connected.
}

#[tokio::test]
async fn raw_message_flows_between_peers() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    // Both subscribe to the same topic.
    let topic = "raw-msg-test";
    node_a.subscribe(topic).expect("subscribe a");
    node_b.subscribe(topic).expect("subscribe b");

    // Give gossipsub time to exchange subscription info.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // A publishes a raw message.
    let payload = b"hello from A".to_vec();
    node_a.publish(topic, payload.clone()).expect("publish");

    // B should receive it.
    let (recv_topic, recv_data) = wait_for_message(&mut events_b).await;
    assert_eq!(recv_topic, topic);
    assert_eq!(recv_data, payload);
}

#[tokio::test]
async fn full_envelope_round_trip_over_network() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    let topic = "envelope-test";
    node_a.subscribe(topic).expect("sub a");
    node_b.subscribe(topic).expect("sub b");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Create a proper Message, pack it in an Envelope, and send.
    let mut hlc = HLC::new();
    let msg = Message::text(
        ChannelId::new(),
        id_a.peer_id(),
        "full stack test!",
        &mut hlc,
    );
    let envelope_bytes = pack_envelope(MessageType::Chat, &msg).expect("pack");

    node_a.publish(topic, envelope_bytes).expect("publish");

    // B receives and deserializes the full envelope.
    let (_topic, data) = wait_for_message(&mut events_b).await;

    let (decoded, msg_type) =
        unpack_envelope::<Message>(&data).expect("should deserialize envelope");
    assert_eq!(msg_type, MessageType::Chat);
    assert_eq!(decoded.id, msg.id);
    assert_eq!(decoded.author, id_a.peer_id());
    assert!(matches!(decoded.content, Content::Text { ref body } if body == "full stack test!"));
    assert_eq!(decoded.hlc, msg.hlc);
}

#[tokio::test]
async fn channels_are_isolated() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    // A subscribes to "alpha", B subscribes to both "alpha" and "beta".
    node_a.subscribe("alpha").expect("sub alpha");
    node_b.subscribe("alpha").expect("sub alpha on b");
    node_b.subscribe("beta").expect("sub beta on b");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // A sends on "alpha" — B should receive it.
    node_a.publish("alpha", b"for alpha".to_vec()).expect("pub");
    let (topic, data) = wait_for_message(&mut events_b).await;
    assert_eq!(topic, "alpha");
    assert_eq!(data, b"for alpha");

    // B sends on "beta" — A is NOT subscribed, so should not receive.
    node_b
        .publish("beta", b"for beta".to_vec())
        .expect("pub beta");

    let result = timeout(Duration::from_millis(500), async {
        loop {
            if let Some(NetworkEvent::Message { topic, .. }) = events_a.recv().await {
                return topic;
            }
        }
    })
    .await;

    assert!(result.is_err(), "A should not receive messages on 'beta'");
}
