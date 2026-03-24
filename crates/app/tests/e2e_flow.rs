//! End-to-end integration tests for the full invite-chat-sync flow.
//!
//! Two tiers:
//! 1. **State machine E2E** — pure deterministic events covering server
//!    creation, permission grants, messaging, channel management,
//!    divergence, and merge convergence across 3 simulated peers.
//! 2. **Network E2E** — real libp2p nodes on localhost verifying message
//!    delivery and replies across a 3-node topology.

#![allow(deprecated)]

use std::time::Duration;

use tokio::time::timeout;

use willow_identity::Identity;
use willow_network::{NetworkConfig, NetworkEvent, NetworkNode};

// ────────────────────────────────────────────────────────────────────────────
// Helpers (network tier)
// ────────────────────────────────────────────────────────────────────────────

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Wait for a `Listening` event and dial from the given nodes.
///
/// Returns after the hub's listen address has been extracted and all
/// `dialers` have called `dial()`.  The caller must still wait for
/// the corresponding `PeerConnected` events.
async fn wait_and_dial(
    hub_events: &mut tokio::sync::mpsc::UnboundedReceiver<NetworkEvent>,
    dialers: &[&NetworkNode],
) {
    let addr = loop {
        let event = timeout(TEST_TIMEOUT, hub_events.recv())
            .await
            .expect("timed out waiting for listen addr")
            .expect("channel closed");
        if let NetworkEvent::Listening(addr) = event {
            break addr;
        }
    };
    for dialer in dialers {
        dialer.dial(addr.clone()).expect("dial should succeed");
    }
}

/// Wait for a `PeerConnected` event.
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

// ────────────────────────────────────────────────────────────────────────────
// 1.  State-machine E2E: invite → chat → sync
// ────────────────────────────────────────────────────────────────────────────

#[test]
fn e2e_invite_chat_sync_state_machine() {
    use willow_app::willow_state::*;

    // ── Step 1: Alice creates a server ──────────────────────────────────
    let alice_id = "alice-peer-id";
    let bob_id = "bob-peer-id";
    let carol_id = "carol-peer-id";

    let mut alice_state = ServerState::new("server-1", "Alice's Server", alice_id);
    assert!(alice_state.members.contains_key(alice_id));
    assert_eq!(alice_state.members.len(), 1);

    // ── Step 2: Alice creates general channel ───────────────────────────
    let create_general = Event {
        id: "evt-create-general".to_string(),
        parent_hash: alice_state.hash(),
        author: alice_id.to_string(),
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-general".to_string(),
        },
    };
    let result = apply(&mut alice_state, &create_general);
    assert_eq!(result, ApplyResult::Applied);
    assert!(alice_state.channels.contains_key("ch-general"));

    // ── Step 3: Alice grants permissions to Bob and Carol ("invite") ────
    let grant_bob = Event {
        id: "evt-grant-bob".to_string(),
        parent_hash: alice_state.hash(),
        author: alice_id.to_string(),
        timestamp_ms: 2000,
        kind: EventKind::GrantPermission {
            peer_id: bob_id.to_string(),
            permission: Permission::SendMessages,
        },
    };
    let result = apply(&mut alice_state, &grant_bob);
    assert_eq!(result, ApplyResult::Applied);
    assert!(alice_state.members.contains_key(bob_id));

    let grant_carol = Event {
        id: "evt-grant-carol".to_string(),
        parent_hash: alice_state.hash(),
        author: alice_id.to_string(),
        timestamp_ms: 2001,
        kind: EventKind::GrantPermission {
            peer_id: carol_id.to_string(),
            permission: Permission::SendMessages,
        },
    };
    let result = apply(&mut alice_state, &grant_carol);
    assert_eq!(result, ApplyResult::Applied);
    assert!(alice_state.members.contains_key(carol_id));
    assert_eq!(alice_state.members.len(), 3); // Alice + Bob + Carol

    // ── Step 4: Bob and Carol sync — replay same events ─────────────────
    let mut bob_state = ServerState::new("server-1", "Alice's Server", alice_id);
    apply_lenient(&mut bob_state, &create_general);
    apply_lenient(&mut bob_state, &grant_bob);
    apply_lenient(&mut bob_state, &grant_carol);

    let mut carol_state = ServerState::new("server-1", "Alice's Server", alice_id);
    apply_lenient(&mut carol_state, &create_general);
    apply_lenient(&mut carol_state, &grant_bob);
    apply_lenient(&mut carol_state, &grant_carol);

    // All three peers now agree on state.
    assert_eq!(alice_state.hash(), bob_state.hash());
    assert_eq!(alice_state.hash(), carol_state.hash());

    // ── Step 5: Alice sends a message ───────────────────────────────────
    let alice_msg = Event {
        id: "msg-alice-1".to_string(),
        parent_hash: StateHash::ZERO, // lenient from here on (simulating async)
        author: alice_id.to_string(),
        timestamp_ms: 3000,
        kind: EventKind::Message {
            channel_id: "ch-general".to_string(),
            body: "Hello from Alice!".to_string(),
            reply_to: None,
        },
    };
    apply_lenient(&mut alice_state, &alice_msg);
    apply_lenient(&mut bob_state, &alice_msg);
    apply_lenient(&mut carol_state, &alice_msg);

    // ── Step 6: Verify Bob and Carol received the message ───────────────
    assert_eq!(bob_state.messages.len(), 1);
    assert_eq!(bob_state.messages[0].body, "Hello from Alice!");
    assert_eq!(bob_state.messages[0].author, alice_id);
    assert_eq!(carol_state.messages.len(), 1);
    assert_eq!(carol_state.messages[0].body, "Hello from Alice!");

    // ── Step 7: Bob sends a reply ───────────────────────────────────────
    let bob_msg = Event {
        id: "msg-bob-1".to_string(),
        parent_hash: StateHash::ZERO,
        author: bob_id.to_string(),
        timestamp_ms: 3500,
        kind: EventKind::Message {
            channel_id: "ch-general".to_string(),
            body: "Hi Alice!".to_string(),
            reply_to: None,
        },
    };
    apply_lenient(&mut alice_state, &bob_msg);
    apply_lenient(&mut bob_state, &bob_msg);
    apply_lenient(&mut carol_state, &bob_msg);

    assert_eq!(alice_state.messages.len(), 2);
    assert_eq!(alice_state.messages[1].body, "Hi Alice!");
    assert_eq!(alice_state.messages[1].author, bob_id);

    // ── Step 8: Alice creates "random" channel ──────────────────────────
    let create_random = Event {
        id: "evt-create-random".to_string(),
        parent_hash: StateHash::ZERO,
        author: alice_id.to_string(),
        timestamp_ms: 4000,
        kind: EventKind::CreateChannel {
            name: "random".to_string(),
            channel_id: "ch-random".to_string(),
        },
    };
    apply_lenient(&mut alice_state, &create_random);
    apply_lenient(&mut bob_state, &create_random);
    apply_lenient(&mut carol_state, &create_random);

    // ── Step 9: All peers see 2 channels ────────────────────────────────
    assert_eq!(alice_state.channels.len(), 2);
    assert_eq!(bob_state.channels.len(), 2);
    assert_eq!(carol_state.channels.len(), 2);

    assert!(alice_state.channels.contains_key("ch-random"));
    assert!(bob_state.channels.contains_key("ch-random"));
    assert!(carol_state.channels.contains_key("ch-random"));

    // ── Step 10: State hashes match after full sync ─────────────────────
    assert_eq!(alice_state.hash(), bob_state.hash());
    assert_eq!(alice_state.hash(), carol_state.hash());

    // ── Step 11: Test divergence + merge ────────────────────────────────
    // Save the common ancestor before diverging.
    let common_ancestor = alice_state.clone();
    let common_hash = common_ancestor.hash();

    // Alice and Bob both send messages independently (no sync).
    let alice_offline = Event {
        id: "msg-alice-offline".to_string(),
        parent_hash: common_hash.clone(),
        author: alice_id.to_string(),
        timestamp_ms: 5000,
        kind: EventKind::Message {
            channel_id: "ch-general".to_string(),
            body: "Alice offline msg".to_string(),
            reply_to: None,
        },
    };
    let bob_offline = Event {
        id: "msg-bob-offline".to_string(),
        parent_hash: common_hash,
        author: bob_id.to_string(),
        timestamp_ms: 5001,
        kind: EventKind::Message {
            channel_id: "ch-general".to_string(),
            body: "Bob offline msg".to_string(),
            reply_to: None,
        },
    };

    apply_lenient(&mut alice_state, &alice_offline);
    apply_lenient(&mut bob_state, &bob_offline);

    // States diverge.
    assert_ne!(alice_state.hash(), bob_state.hash());

    // Merge resolves — both messages present, deterministic order.
    let (merged, canonical) = merge(
        &[alice_offline.clone()],
        &[bob_offline.clone()],
        &common_ancestor,
    );

    assert_eq!(canonical.len(), 2);
    assert_eq!(merged.messages.len(), 4); // 2 earlier + 2 offline msgs

    // Ordering: alice_offline (ts=5000) before bob_offline (ts=5001).
    let merged_offline_msgs: Vec<&ChatMessage> = merged
        .messages
        .iter()
        .filter(|m| m.body.contains("offline"))
        .collect();
    assert_eq!(merged_offline_msgs.len(), 2);
    assert_eq!(merged_offline_msgs[0].body, "Alice offline msg");
    assert_eq!(merged_offline_msgs[1].body, "Bob offline msg");

    // Both sides converge to the same state after replaying the merge.
    let mut alice_converged = common_ancestor.clone();
    for event in &canonical {
        apply_lenient(&mut alice_converged, event);
    }
    let mut bob_converged = common_ancestor;
    for event in &canonical {
        apply_lenient(&mut bob_converged, event);
    }
    assert_eq!(alice_converged.hash(), bob_converged.hash());
}

#[test]
fn e2e_permission_enforcement() {
    use willow_app::willow_state::*;

    let owner = "owner-peer";
    let unprivileged = "stranger-peer";

    let mut state = ServerState::new("s1", "Test", owner);

    // Owner creates a channel (should succeed).
    let create_ch = Event {
        id: "create-ch".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-1".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Unprivileged peer tries to create a channel (should be rejected).
    let bad_create = Event {
        id: "bad-create".to_string(),
        parent_hash: state.hash(),
        author: unprivileged.to_string(),
        timestamp_ms: 2000,
        kind: EventKind::CreateChannel {
            name: "hacker-room".to_string(),
            channel_id: "ch-bad".to_string(),
        },
    };
    let result = apply(&mut state, &bad_create);
    assert!(matches!(result, ApplyResult::Rejected(_)));
    assert!(!state.channels.contains_key("ch-bad"));

    // Unprivileged peer CAN still send messages (no permission required).
    let chat_msg = Event {
        id: "chat-1".to_string(),
        parent_hash: state.hash(),
        author: unprivileged.to_string(),
        timestamp_ms: 3000,
        kind: EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "hello".to_string(),
            reply_to: None,
        },
    };
    assert_eq!(apply(&mut state, &chat_msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);

    // Owner grants ManageChannels to the peer, then they can create.
    let grant = Event {
        id: "grant-manage".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 4000,
        kind: EventKind::GrantPermission {
            peer_id: unprivileged.to_string(),
            permission: Permission::ManageChannels,
        },
    };
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    let ok_create = Event {
        id: "ok-create".to_string(),
        parent_hash: state.hash(),
        author: unprivileged.to_string(),
        timestamp_ms: 5000,
        kind: EventKind::CreateChannel {
            name: "allowed-room".to_string(),
            channel_id: "ch-allowed".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &ok_create), ApplyResult::Applied);
    assert!(state.channels.contains_key("ch-allowed"));
}

#[test]
fn e2e_dedup_and_idempotency() {
    use willow_app::willow_state::*;

    let mut state = ServerState::new("s1", "Test", "owner");

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

    // First application succeeds.
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);
    assert_eq!(state.channels.len(), 1);

    // Duplicate event is detected.
    let result = apply(&mut state, &create_ch);
    assert_eq!(result, ApplyResult::AlreadySeen);
    assert_eq!(state.channels.len(), 1); // no duplicate channel

    // Also deduped in lenient mode.
    let result = apply_lenient(&mut state, &create_ch);
    assert_eq!(result, ApplyResult::AlreadySeen);
}

#[test]
fn e2e_kick_member_and_permission_revoke() {
    use willow_app::willow_state::*;

    let owner = "owner";
    let target = "target-peer";

    let mut state = ServerState::new("s1", "Test", owner);

    // Grant permission to target.
    let grant = Event {
        id: "grant".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 1000,
        kind: EventKind::GrantPermission {
            peer_id: target.to_string(),
            permission: Permission::SendMessages,
        },
    };
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.members.contains_key(target));
    assert!(state.is_trusted(target));

    // Kick the target.
    let kick = Event {
        id: "kick".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 2000,
        kind: EventKind::KickMember {
            peer_id: target.to_string(),
        },
    };
    assert_eq!(apply(&mut state, &kick), ApplyResult::Applied);
    assert!(!state.members.contains_key(target));
    assert!(!state.is_trusted(target));

    // Cannot kick the owner.
    let kick_owner = Event {
        id: "kick-owner".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 3000,
        kind: EventKind::KickMember {
            peer_id: owner.to_string(),
        },
    };
    assert_eq!(apply(&mut state, &kick_owner), ApplyResult::Applied);
    assert!(state.members.contains_key(owner)); // owner cannot be removed
}

#[test]
fn e2e_roles_and_channel_management() {
    use willow_app::willow_state::*;

    let owner = "owner";
    let mut state = ServerState::new("s1", "Test", owner);

    // Create a role.
    let create_role = Event {
        id: "role-create".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 1000,
        kind: EventKind::CreateRole {
            name: "Moderator".to_string(),
            role_id: "role-mod".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &create_role), ApplyResult::Applied);
    assert!(state.roles.contains_key("role-mod"));

    // Create two channels.
    let create_ch1 = Event {
        id: "ch1-create".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 2000,
        kind: EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-1".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &create_ch1), ApplyResult::Applied);

    let create_ch2 = Event {
        id: "ch2-create".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 2001,
        kind: EventKind::CreateChannel {
            name: "random".to_string(),
            channel_id: "ch-2".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &create_ch2), ApplyResult::Applied);
    assert_eq!(state.channels.len(), 2);

    // Rename a channel.
    let rename = Event {
        id: "rename-ch2".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 3000,
        kind: EventKind::RenameChannel {
            channel_id: "ch-2".to_string(),
            new_name: "off-topic".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &rename), ApplyResult::Applied);
    assert_eq!(state.channels["ch-2"].name, "off-topic");

    // Delete a channel — messages in it are also removed.
    let msg_in_ch2 = Event {
        id: "msg-ch2".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 3500,
        kind: EventKind::Message {
            channel_id: "ch-2".to_string(),
            body: "will be deleted".to_string(),
            reply_to: None,
        },
    };
    assert_eq!(apply(&mut state, &msg_in_ch2), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);

    let delete_ch2 = Event {
        id: "delete-ch2".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 4000,
        kind: EventKind::DeleteChannel {
            channel_id: "ch-2".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &delete_ch2), ApplyResult::Applied);
    assert_eq!(state.channels.len(), 1);
    assert!(!state.channels.contains_key("ch-2"));
    assert!(state.messages.is_empty()); // message in deleted channel removed

    // Delete the role.
    let delete_role = Event {
        id: "role-delete".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 5000,
        kind: EventKind::DeleteRole {
            role_id: "role-mod".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &delete_role), ApplyResult::Applied);
    assert!(!state.roles.contains_key("role-mod"));
}

#[test]
fn e2e_profile_and_reactions() {
    use willow_app::willow_state::*;

    let owner = "owner";
    let peer = "peer-a";
    let mut state = ServerState::new("s1", "Test", owner);

    // Create channel and grant peer access.
    let create_ch = Event {
        id: "ch-create".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-1".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    let grant = Event {
        id: "grant-peer".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 1500,
        kind: EventKind::GrantPermission {
            peer_id: peer.to_string(),
            permission: Permission::SendMessages,
        },
    };
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Peer sets display name.
    let set_profile = Event {
        id: "profile-set".to_string(),
        parent_hash: state.hash(),
        author: peer.to_string(),
        timestamp_ms: 2000,
        kind: EventKind::SetProfile {
            display_name: "AlphaUser".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &set_profile), ApplyResult::Applied);
    assert_eq!(state.profiles[peer].display_name, "AlphaUser");
    assert_eq!(
        state.members[peer].display_name.as_deref(),
        Some("AlphaUser")
    );

    // Peer sends a message.
    let msg = Event {
        id: "msg-1".to_string(),
        parent_hash: state.hash(),
        author: peer.to_string(),
        timestamp_ms: 3000,
        kind: EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "hello world".to_string(),
            reply_to: None,
        },
    };
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);

    // Owner reacts to the message.
    let reaction = Event {
        id: "react-1".to_string(),
        parent_hash: state.hash(),
        author: owner.to_string(),
        timestamp_ms: 3500,
        kind: EventKind::Reaction {
            message_id: "msg-1".to_string(),
            emoji: "thumbsup".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &reaction), ApplyResult::Applied);
    let chat_msg = &state.messages[0];
    assert!(chat_msg.reactions.contains_key("thumbsup"));
    assert_eq!(chat_msg.reactions["thumbsup"], vec![owner.to_string()]);

    // Edit the message.
    let edit = Event {
        id: "edit-1".to_string(),
        parent_hash: state.hash(),
        author: peer.to_string(),
        timestamp_ms: 4000,
        kind: EventKind::EditMessage {
            message_id: "msg-1".to_string(),
            new_body: "hello world (edited)".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &edit), ApplyResult::Applied);
    assert!(state.messages[0].edited);
    assert_eq!(state.messages[0].body, "hello world (edited)");

    // Delete the message.
    let delete = Event {
        id: "delete-1".to_string(),
        parent_hash: state.hash(),
        author: peer.to_string(),
        timestamp_ms: 5000,
        kind: EventKind::DeleteMessage {
            message_id: "msg-1".to_string(),
        },
    };
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);
    assert!(state.messages[0].deleted);
    assert_eq!(state.messages[0].body, "[message deleted]");
    assert!(state.messages[0].reactions.is_empty());
}

// ────────────────────────────────────────────────────────────────────────────
// 2.  Network-level E2E: 3-node message delivery
// ────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn e2e_network_message_delivery() {
    // Start 3 nodes.
    let mut alice = start_test_node().await;
    let mut bob = start_test_node().await;
    let mut carol = start_test_node().await;

    // Bob and Carol dial Alice (star topology).
    wait_and_dial(&mut alice.events, &[&bob.node, &carol.node]).await;

    // Wait for connections (Alice sees 2 peers; Bob and Carol see 1 each).
    wait_for_connect(&mut alice.events).await;
    wait_for_connect(&mut alice.events).await;
    wait_for_connect(&mut bob.events).await;
    wait_for_connect(&mut carol.events).await;

    // All subscribe to the same topic.
    let topic = &format!("e2e-msg-{}", uuid::Uuid::new_v4());
    alice.node.subscribe(topic).expect("sub alice");
    bob.node.subscribe(topic).expect("sub bob");
    carol.node.subscribe(topic).expect("sub carol");
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Alice publishes a message.
    let payload = b"Hello from Alice!".to_vec();
    alice
        .node
        .publish(topic, payload.clone())
        .expect("alice publish");

    // Bob and Carol should both receive it.
    let (topic_b, data_b) = wait_for_message(&mut bob.events).await;
    assert_eq!(topic_b, *topic);
    assert_eq!(data_b, payload);

    let (topic_c, data_c) = wait_for_message(&mut carol.events).await;
    assert_eq!(topic_c, *topic);
    assert_eq!(data_c, payload);

    // Bob publishes a reply.
    let reply = b"Hi Alice, from Bob!".to_vec();
    bob.node.publish(topic, reply.clone()).expect("bob publish");

    // Alice and Carol should receive Bob's reply.
    let (topic_a, data_a) = wait_for_message(&mut alice.events).await;
    assert_eq!(topic_a, *topic);
    assert_eq!(data_a, reply);

    let (topic_c2, data_c2) = wait_for_message(&mut carol.events).await;
    assert_eq!(topic_c2, *topic);
    assert_eq!(data_c2, reply);
}

#[tokio::test]
async fn e2e_network_full_envelope_three_nodes() {
    use willow_messaging::hlc::HLC;
    use willow_messaging::{ChannelId, Content, Message};
    use willow_transport::{pack_envelope, unpack_envelope, MessageType};

    // Start 3 nodes.
    let mut alice = start_test_node().await;
    let mut bob = start_test_node().await;
    let mut carol = start_test_node().await;

    // Connect in a star around Alice.
    let alice_addr = loop {
        let event = timeout(TEST_TIMEOUT, alice.events.recv())
            .await
            .expect("timed out")
            .expect("closed");
        if let NetworkEvent::Listening(addr) = event {
            break addr;
        }
    };
    bob.node.dial(alice_addr.clone()).expect("bob dial");
    carol.node.dial(alice_addr).expect("carol dial");

    wait_for_connect(&mut alice.events).await;
    wait_for_connect(&mut alice.events).await;
    wait_for_connect(&mut bob.events).await;
    wait_for_connect(&mut carol.events).await;

    let topic = &format!("e2e-envelope-{}", uuid::Uuid::new_v4());
    alice.node.subscribe(topic).expect("sub alice");
    bob.node.subscribe(topic).expect("sub bob");
    carol.node.subscribe(topic).expect("sub carol");
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Alice sends a signed message through the full transport stack.
    let mut hlc = HLC::new();
    let channel_id = ChannelId::new();
    let msg = Message::text(
        channel_id,
        alice.identity.peer_id(),
        "Full envelope E2E!",
        &mut hlc,
    );
    let envelope_bytes = pack_envelope(MessageType::Chat, &msg).expect("pack");
    let signed = willow_identity::pack(&envelope_bytes, &alice.identity).expect("sign");
    alice.node.publish(topic, signed).expect("publish");

    // Both Bob and Carol receive, verify signature, and decode.
    for (name, events) in [("Bob", &mut bob.events), ("Carol", &mut carol.events)] {
        let (_topic, data) = wait_for_message(events).await;

        let (envelope_data, signer) =
            willow_identity::unpack::<Vec<u8>>(&data).expect("verify sig");
        assert_eq!(
            signer,
            alice.identity.peer_id(),
            "{name} should verify Alice's signature"
        );

        let (decoded, msg_type) =
            unpack_envelope::<Message>(&envelope_data).expect("unpack envelope");
        assert_eq!(msg_type, MessageType::Chat);
        assert_eq!(decoded.id, msg.id);
        assert!(
            matches!(decoded.content, Content::Text { ref body } if body == "Full envelope E2E!")
        );
    }
}

#[tokio::test]
async fn e2e_network_server_op_sync_three_nodes() {
    use willow_app::server_sync::{self, Op, StampedOp, SyncMessage};
    use willow_messaging::hlc::HLC;

    // Start 3 nodes.
    let mut alice = start_test_node().await;
    let mut bob = start_test_node().await;
    let mut carol = start_test_node().await;

    let alice_addr = loop {
        let event = timeout(TEST_TIMEOUT, alice.events.recv())
            .await
            .expect("timed out")
            .expect("closed");
        if let NetworkEvent::Listening(addr) = event {
            break addr;
        }
    };
    bob.node.dial(alice_addr.clone()).expect("bob dial");
    carol.node.dial(alice_addr).expect("carol dial");

    wait_for_connect(&mut alice.events).await;
    wait_for_connect(&mut alice.events).await;
    wait_for_connect(&mut bob.events).await;
    wait_for_connect(&mut carol.events).await;

    let topic = &format!("e2e-server-ops-{}", uuid::Uuid::new_v4());
    alice.node.subscribe(topic).expect("sub alice");
    bob.node.subscribe(topic).expect("sub bob");
    carol.node.subscribe(topic).expect("sub carol");
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Alice broadcasts a CreateChannel server op.
    let mut hlc = HLC::new();
    let channel_id = uuid::Uuid::new_v4().to_string();
    let stamped = StampedOp::new(
        Op::CreateChannel {
            name: "general".into(),
            channel_id: channel_id.clone(),
        },
        &mut hlc,
        &alice.identity.peer_id().to_string(),
    );
    let signed = server_sync::pack_op(&stamped, &alice.identity).expect("pack_op");
    alice.node.publish(topic, signed).expect("publish");

    // Both Bob and Carol receive, unpack, and verify.
    for (name, events) in [("Bob", &mut bob.events), ("Carol", &mut carol.events)] {
        let (_topic, data) = wait_for_message(events).await;
        let (msg, signer) = server_sync::unpack_sync(&data).expect("unpack_sync");
        assert_eq!(
            signer,
            alice.identity.peer_id(),
            "{name} should see Alice as signer"
        );

        match msg {
            SyncMessage::Op(op) => {
                assert!(
                    matches!(op.op, Op::CreateChannel { ref name, .. } if name == "general"),
                    "{name} should see CreateChannel(general)"
                );
            }
            other => panic!("{name}: expected SyncMessage::Op, got {other:?}"),
        }
    }
}
