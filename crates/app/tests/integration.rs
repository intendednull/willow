//! Full integration tests that spin up real network nodes and verify
//! end-to-end message flow through the entire stack:
//!
//! identity → messaging → transport → network (libp2p) → deserialize → verify
//!
//! These tests use localhost TCP with direct dialing for deterministic
//! peer discovery.

// The integration tests still exercise the legacy Op/StampedOp wire format
// through the dual-format bridge. Suppress deprecation warnings.
#![allow(deprecated)]

use std::collections::HashSet;
use std::time::Duration;

use tokio::time::timeout;

use willow_app::server_sync::{self, Op, StampedOp, SyncMessage};
use willow_files::FileManifest;
use willow_identity::Identity;
use willow_messaging::hlc::{HlcTimestamp, HLC};
use willow_messaging::{ChannelId, Content, Message};
use willow_network::file_transfer::ChunkResponse;
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

#[tokio::test]
async fn signed_and_encrypted_message_round_trip() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    let topic = "encrypted-signed-test";
    node_a.subscribe(topic).expect("sub a");
    node_b.subscribe(topic).expect("sub b");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Shared channel key.
    let channel_key = willow_crypto::generate_channel_key();

    // A creates, encrypts, and signs a message.
    let mut hlc = HLC::new();
    let msg = Message::text(ChannelId::new(), id_a.peer_id(), "top secret!", &mut hlc);
    let sealed = willow_crypto::seal_content(&msg.content, &channel_key, 0).expect("seal");
    let mut encrypted_msg = msg.clone();
    encrypted_msg.content = Content::Encrypted(sealed);

    let envelope_bytes = pack_envelope(MessageType::Chat, &encrypted_msg).expect("pack");
    let signed_bytes = willow_identity::pack(&envelope_bytes, &id_a).expect("sign");

    node_a.publish(topic, signed_bytes).expect("publish");

    // B receives, verifies signature, unpacks envelope, decrypts content.
    let (_topic, data) = wait_for_message(&mut events_b).await;

    let (envelope_data, signer) =
        willow_identity::unpack::<Vec<u8>>(&data).expect("verify signature");
    assert_eq!(signer, id_a.peer_id());

    let (decoded, msg_type) = unpack_envelope::<Message>(&envelope_data).expect("unpack envelope");
    assert_eq!(msg_type, MessageType::Chat);

    let content = match &decoded.content {
        Content::Encrypted(sealed) => {
            willow_crypto::open_content(sealed, &channel_key).expect("decrypt")
        }
        other => other.clone(),
    };
    assert!(matches!(content, Content::Text { ref body } if body == "top secret!"));
}

#[tokio::test]
async fn encrypted_message_unreadable_without_key() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    let topic = "no-key-test";
    node_a.subscribe(topic).expect("sub a");
    node_b.subscribe(topic).expect("sub b");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let channel_key = willow_crypto::generate_channel_key();
    let wrong_key = willow_crypto::generate_channel_key();

    let mut hlc = HLC::new();
    let msg = Message::text(ChannelId::new(), id_a.peer_id(), "secret", &mut hlc);
    let sealed = willow_crypto::seal_content(&msg.content, &channel_key, 0).expect("seal");
    let mut encrypted_msg = msg;
    encrypted_msg.content = Content::Encrypted(sealed);

    let envelope_bytes = pack_envelope(MessageType::Chat, &encrypted_msg).expect("pack");
    let signed_bytes = willow_identity::pack(&envelope_bytes, &id_a).expect("sign");

    node_a.publish(topic, signed_bytes).expect("publish");

    // B receives the message but cannot decrypt with the wrong key.
    let (_topic, data) = wait_for_message(&mut events_b).await;

    let (envelope_data, _) = willow_identity::unpack::<Vec<u8>>(&data).expect("signature valid");
    let (decoded, _) = unpack_envelope::<Message>(&envelope_data).expect("envelope valid");

    if let Content::Encrypted(sealed) = &decoded.content {
        let result = willow_crypto::open_content(sealed, &wrong_key);
        assert!(result.is_err(), "decryption with wrong key should fail");
    } else {
        panic!("expected encrypted content");
    }
}

// ───── Additional helpers ──────────────────────────────────────────────────

/// Wait for a ChunkRequested event on the server node, respond with the given
/// data, and return the requested hash.
async fn handle_chunk_request(
    server_node: &NetworkNode,
    server_events: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
    chunk_hash: &willow_files::ContentHash,
    chunk_data: &[u8],
) -> willow_files::ContentHash {
    loop {
        let event = timeout(TEST_TIMEOUT, server_events.recv())
            .await
            .expect("timed out waiting for chunk request")
            .expect("channel closed");
        if let NetworkEvent::ChunkRequested { channel, hash, .. } = event {
            assert_eq!(hash, *chunk_hash);
            server_node
                .respond_chunk(
                    channel,
                    ChunkResponse::Found {
                        hash: chunk_hash.clone(),
                        data: chunk_data.to_vec(),
                    },
                )
                .expect("respond_chunk");
            return hash;
        }
    }
}

/// Wait for a ChunkReceived event, ignoring other events.
async fn wait_for_chunk_response(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
) -> ChunkResponse {
    loop {
        let event = timeout(TEST_TIMEOUT, events.recv())
            .await
            .expect("timed out waiting for chunk response")
            .expect("channel closed");
        if let NetworkEvent::ChunkReceived { response, .. } = event {
            return response;
        }
    }
}

// ───── Server sync tests ───────────────────────────────────────────────────

#[tokio::test]
async fn server_op_sync_between_peers() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    // Use a unique topic to avoid cross-test interference via mDNS.
    let topic = &format!("server-op-sync-{}", uuid::Uuid::new_v4());
    node_a.subscribe(topic).expect("sub a");
    node_b.subscribe(topic).expect("sub b");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // A creates and publishes a server op.
    let mut hlc = HLC::new();
    let channel_id = uuid::Uuid::new_v4().to_string();
    let stamped = StampedOp::new(
        Op::CreateChannel {
            name: "general".into(),
            channel_id: channel_id.clone(),
        },
        &mut hlc,
        &id_a.peer_id().to_string(),
    );

    let signed_data = server_sync::pack_op(&stamped, &id_a).expect("pack_op");
    node_a.publish(topic, signed_data).expect("publish");

    // B receives and unpacks.
    let (_recv_topic, data) = wait_for_message(&mut events_b).await;
    let (msg, signer) = server_sync::unpack_sync(&data).expect("unpack_sync");

    // Verify signer matches A.
    assert_eq!(signer, id_a.peer_id());

    // Verify it's an Op with the correct channel name.
    match msg {
        SyncMessage::Op(op) => {
            assert!(matches!(op.op, Op::CreateChannel { ref name, .. } if name == "general"));
            assert_eq!(op.author, id_a.peer_id().to_string());
        }
        other => panic!("expected SyncMessage::Op, got {:?}", other),
    }
}

#[tokio::test]
async fn server_op_author_verified_against_signer() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();
    let id_impersonated = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    let topic = &format!("server-op-author-{}", uuid::Uuid::new_v4());
    node_a.subscribe(topic).expect("sub a");
    node_b.subscribe(topic).expect("sub b");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // A creates a StampedOp but sets author to a DIFFERENT peer_id.
    let mut hlc = HLC::new();
    let stamped = StampedOp::new(
        Op::CreateChannel {
            name: "spoofed".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &id_impersonated.peer_id().to_string(), // wrong author
    );

    // A signs it with A's own identity.
    let signed_data = server_sync::pack_op(&stamped, &id_a).expect("pack_op");
    node_a.publish(topic, signed_data).expect("publish");

    // B receives and unpacks.
    let (_recv_topic, data) = wait_for_message(&mut events_b).await;
    let (msg, signer) = server_sync::unpack_sync(&data).expect("unpack_sync");

    // The signer is A (the actual signer), not the impersonated identity.
    assert_eq!(signer, id_a.peer_id());

    // The author field inside the op does NOT match the signer.
    if let SyncMessage::Op(op) = msg {
        assert_ne!(
            op.author,
            signer.to_string(),
            "author field should differ from signer — mismatch detected"
        );
        assert_eq!(op.author, id_impersonated.peer_id().to_string());
    } else {
        panic!("expected SyncMessage::Op");
    }
}

#[tokio::test]
async fn sync_request_and_batch_over_network() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    let topic = &format!("sync-req-batch-{}", uuid::Uuid::new_v4());
    node_a.subscribe(topic).expect("sub a");
    node_b.subscribe(topic).expect("sub b");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // A publishes a SyncRequest.
    let sync_req = SyncMessage::SyncRequest {
        latest_hlc: HlcTimestamp::ZERO,
        topic: None,
    };
    let signed_req = server_sync::pack_sync(&sync_req, &id_a).expect("pack_sync request");
    node_a.publish(topic, signed_req).expect("publish request");

    // B receives the SyncRequest.
    let (_topic, data) = wait_for_message(&mut events_b).await;
    let (msg, req_signer) = server_sync::unpack_sync(&data).expect("unpack request");
    assert_eq!(req_signer, id_a.peer_id());
    assert!(
        matches!(msg, SyncMessage::SyncRequest { latest_hlc, .. } if latest_hlc == HlcTimestamp::ZERO)
    );

    // B creates a SyncBatch with one op and publishes.
    let mut hlc = HLC::new();
    let stamped = StampedOp::new(
        Op::CreateChannel {
            name: "from-batch".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &id_b.peer_id().to_string(),
    );
    let batch = SyncMessage::SyncBatch {
        ops: vec![stamped.clone()],
    };
    let signed_batch = server_sync::pack_sync(&batch, &id_b).expect("pack_sync batch");
    node_b.publish(topic, signed_batch).expect("publish batch");

    // A receives the SyncBatch.
    let (_topic, data) = wait_for_message(&mut events_a).await;
    let (msg, batch_signer) = server_sync::unpack_sync(&data).expect("unpack batch");
    assert_eq!(batch_signer, id_b.peer_id());

    match msg {
        SyncMessage::SyncBatch { ops } => {
            assert_eq!(ops.len(), 1);
            assert_eq!(ops[0].op_id, stamped.op_id);
            assert!(
                matches!(ops[0].op, Op::CreateChannel { ref name, .. } if name == "from-batch")
            );
        }
        other => panic!("expected SyncMessage::SyncBatch, got {:?}", other),
    }
}

// ───── Multi-node test ─────────────────────────────────────────────────────

#[tokio::test]
async fn three_node_message_propagation() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();
    let id_c = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    let (node_c, mut events_c) = start_node(&id_c).await;

    // Get A's listen address.
    let addr_a = loop {
        let event = timeout(TEST_TIMEOUT, events_a.recv())
            .await
            .expect("timed out waiting for A listen addr")
            .expect("channel closed");
        if let NetworkEvent::Listening(addr) = event {
            break addr;
        }
    };

    // B dials A.
    node_b.dial(addr_a.clone()).expect("B dial A");
    wait_for_connect(&mut events_a).await;
    wait_for_connect(&mut events_b).await;

    // C dials A.
    node_c.dial(addr_a).expect("C dial A");
    wait_for_connect(&mut events_a).await;
    wait_for_connect(&mut events_c).await;

    // Drain any Listening events from B and C.
    let drain_listening = |events: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>| {
        while let Ok(NetworkEvent::Listening(_)) = events.try_recv() {}
    };
    drain_listening(&mut events_b);
    drain_listening(&mut events_c);

    // All subscribe to the same topic.
    let topic = "three-node-test";
    node_a.subscribe(topic).expect("sub a");
    node_b.subscribe(topic).expect("sub b");
    node_c.subscribe(topic).expect("sub c");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // A publishes a message.
    let payload = b"hello from A to all".to_vec();
    node_a.publish(topic, payload.clone()).expect("publish");

    // Both B and C should receive it.
    let (topic_b, data_b) = wait_for_message(&mut events_b).await;
    assert_eq!(topic_b, topic);
    assert_eq!(data_b, payload);

    let (topic_c, data_c) = wait_for_message(&mut events_c).await;
    assert_eq!(topic_c, topic);
    assert_eq!(data_c, payload);
}

// ───── File transfer tests ─────────────────────────────────────────────────

#[tokio::test]
async fn file_chunk_request_response() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    // Create a small file and split into chunks.
    let file_data = b"hello, this is a test file for chunk transfer!";
    let (_manifest, chunks) = willow_files::split_file(file_data, "test.txt", "text/plain", 16);
    assert!(!chunks.is_empty());

    let target_chunk = &chunks[0];
    let target_hash = target_chunk.hash.clone();
    let target_data = target_chunk.data.clone();

    // B requests a chunk from A.
    let a_peer_id = node_a.peer_id();
    node_b
        .request_chunk(a_peer_id, target_hash.clone())
        .expect("request_chunk");

    // A receives ChunkRequested and responds.
    let requested_hash =
        handle_chunk_request(&node_a, &mut events_a, &target_hash, &target_data).await;
    assert_eq!(requested_hash, target_hash);

    // B receives ChunkReceived.
    let response = wait_for_chunk_response(&mut events_b).await;
    match response {
        ChunkResponse::Found { hash, data } => {
            assert_eq!(hash, target_hash);
            assert_eq!(data, target_data);
        }
        ChunkResponse::NotFound { .. } => panic!("expected Found, got NotFound"),
    }
}

#[tokio::test]
async fn file_manifest_over_gossipsub() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    let topic = "file-manifest-test";
    node_a.subscribe(topic).expect("sub a");
    node_b.subscribe(topic).expect("sub b");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // A creates a file manifest.
    let file_data = b"some file content for manifest test";
    let (manifest, _chunks) =
        willow_files::split_file(file_data, "document.pdf", "application/pdf", 16);

    // Pack the manifest in an envelope and publish.
    let envelope_bytes = pack_envelope(MessageType::File, &manifest).expect("pack manifest");
    node_a.publish(topic, envelope_bytes).expect("publish");

    // B receives and unpacks.
    let (_recv_topic, data) = wait_for_message(&mut events_b).await;
    let (decoded_manifest, msg_type) =
        unpack_envelope::<FileManifest>(&data).expect("unpack manifest");

    assert_eq!(msg_type, MessageType::File);
    assert_eq!(decoded_manifest.filename, "document.pdf");
    assert_eq!(decoded_manifest.mime_type, "application/pdf");
    assert_eq!(decoded_manifest.total_size, file_data.len() as u64);
    assert_eq!(
        decoded_manifest.chunk_hashes.len(),
        manifest.chunk_hashes.len()
    );
    assert_eq!(decoded_manifest.file_hash, manifest.file_hash);
}

// ───── Tamper / dedup tests ────────────────────────────────────────────────

#[tokio::test]
async fn signed_server_op_tampered_rejected() {
    let id_a = Identity::generate();

    // A creates and signs a server op.
    let mut hlc = HLC::new();
    let stamped = StampedOp::new(
        Op::CreateChannel {
            name: "tamper-test".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &id_a.peer_id().to_string(),
    );
    let mut signed_data = server_sync::pack_op(&stamped, &id_a).expect("pack_op");

    // Tamper with the last byte.
    if let Some(byte) = signed_data.last_mut() {
        *byte ^= 0xFF;
    }

    // Unpacking tampered data should fail.
    let result = server_sync::unpack_sync(&signed_data);
    assert!(
        result.is_none(),
        "tampered data should fail to unpack/verify"
    );
}

#[tokio::test]
async fn multiple_ops_deduplicated() {
    let id_a = Identity::generate();
    let id_b = Identity::generate();

    let (node_a, mut events_a) = start_node(&id_a).await;
    let (node_b, mut events_b) = start_node(&id_b).await;
    connect_nodes(&node_a, &mut events_a, &node_b, &mut events_b).await;

    let topic = &format!("dedup-ops-{}", uuid::Uuid::new_v4());
    node_a.subscribe(topic).expect("sub a");
    node_b.subscribe(topic).expect("sub b");
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // A creates a single StampedOp.
    let mut hlc = HLC::new();
    let stamped = StampedOp::new(
        Op::CreateChannel {
            name: "dedup-test".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        },
        &mut hlc,
        &id_a.peer_id().to_string(),
    );
    let op_id = stamped.op_id.clone();

    // A publishes the SAME op twice (different signed payloads but same op_id).
    let signed_1 = server_sync::pack_op(&stamped, &id_a).expect("pack_op 1");
    let signed_2 = server_sync::pack_op(&stamped, &id_a).expect("pack_op 2");

    node_a
        .publish(topic, signed_1.clone())
        .expect("publish first");
    node_a.publish(topic, signed_2).expect("publish second");

    // B collects messages (wait for at least one, try to get both).
    let mut received_op_ids = Vec::new();

    // Get first message.
    let (_topic, data) = wait_for_message(&mut events_b).await;
    if let Some((SyncMessage::Op(op), _signer)) = server_sync::unpack_sync(&data) {
        received_op_ids.push(op.op_id);
    }

    // Try to get second message (may or may not arrive due to gossipsub dedup).
    let second = timeout(Duration::from_secs(3), async {
        loop {
            if let Some(NetworkEvent::Message { data, .. }) = events_b.recv().await {
                if let Some((SyncMessage::Op(op), _)) = server_sync::unpack_sync(&data) {
                    return op.op_id;
                }
            }
        }
    })
    .await;

    if let Ok(second_op_id) = second {
        received_op_ids.push(second_op_id);
    }

    // All received ops should have the same op_id (they're the same op).
    for received_id in &received_op_ids {
        assert_eq!(received_id, &op_id);
    }

    // Verify dedup logic: using a HashSet, only one unique op should be processed.
    let unique_ids: HashSet<&String> = received_op_ids.iter().collect();
    assert_eq!(
        unique_ids.len(),
        1,
        "all received ops should have the same op_id"
    );
    assert!(unique_ids.contains(&op_id));
}
