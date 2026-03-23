//! Tests for the event-sourced state machine.
//!
//! Covers determinism, idempotency, channel/message lifecycle, trust
//! enforcement, divergence detection, merge, and the in-memory event store.

use crate::hash::StateHash;
use crate::merge::{find_common_ancestor, merge};
use crate::server::ServerState;
use crate::store::{EventStore, InMemoryStore};
use crate::{apply, apply_lenient, ApplyResult, Event, EventKind};

/// Helper: create a server state with a default owner.
fn test_state() -> ServerState {
    ServerState::new("server-1", "Test Server", "owner")
}

/// Helper: create an event with the current state's hash as parent.
fn event(state: &ServerState, id: &str, author: &str, kind: EventKind) -> Event {
    Event {
        id: id.to_string(),
        parent_hash: state.hash(),
        author: author.to_string(),
        timestamp_ms: 1000,
        kind,
    }
}

/// Helper: create an event with an explicit parent hash and timestamp.
fn event_with(id: &str, parent: StateHash, author: &str, ts: u64, kind: EventKind) -> Event {
    Event {
        id: id.to_string(),
        parent_hash: parent,
        author: author.to_string(),
        timestamp_ms: ts,
        kind,
    }
}

// ── Determinism ──────────────────────────────────────────────────────────

#[test]
fn apply_is_deterministic() {
    let events = vec![
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
        EventKind::CreateChannel {
            name: "random".into(),
            channel_id: "ch2".into(),
        },
        EventKind::TrustPeer {
            peer_id: "alice".into(),
        },
    ];

    // Apply the same events to two independent states.
    let mut state_a = test_state();
    let mut state_b = test_state();

    for (i, kind) in events.iter().enumerate() {
        let evt_a = event(&state_a, &format!("e{i}"), "owner", kind.clone());
        assert_eq!(apply(&mut state_a, &evt_a), ApplyResult::Applied);

        let evt_b = event(&state_b, &format!("e{i}"), "owner", kind.clone());
        assert_eq!(apply(&mut state_b, &evt_b), ApplyResult::Applied);
    }

    assert_eq!(state_a.hash(), state_b.hash());
}

// ── Idempotency ──────────────────────────────────────────────────────────

#[test]
fn apply_is_idempotent() {
    let mut state = test_state();
    let evt = event(
        &state,
        "e1",
        "owner",
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    );

    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    // Same event again should be AlreadySeen.
    assert_eq!(apply_lenient(&mut state, &evt), ApplyResult::AlreadySeen);
}

// ── Channel lifecycle ────────────────────────────────────────────────────

#[test]
fn create_and_delete_channel() {
    let mut state = test_state();

    // Create channel.
    let create = event(
        &state,
        "e1",
        "owner",
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);
    assert!(state.channels.contains_key("ch1"));
    assert_eq!(state.channels["ch1"].name, "general");

    // Delete channel.
    let delete = event(
        &state,
        "e2",
        "owner",
        EventKind::DeleteChannel {
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);
    assert!(!state.channels.contains_key("ch1"));
}

#[test]
fn rename_channel() {
    let mut state = test_state();

    let create = event(
        &state,
        "e1",
        "owner",
        EventKind::CreateChannel {
            name: "old-name".into(),
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);

    let rename = event(
        &state,
        "e2",
        "owner",
        EventKind::RenameChannel {
            channel_id: "ch1".into(),
            new_name: "new-name".into(),
        },
    );
    assert_eq!(apply(&mut state, &rename), ApplyResult::Applied);
    assert_eq!(state.channels["ch1"].name, "new-name");
}

// ── Chat lifecycle ───────────────────────────────────────────────────────

#[test]
fn send_and_edit_message() {
    let mut state = test_state();

    // Create a channel first.
    let create_ch = event(
        &state,
        "e0",
        "owner",
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Send a message.
    let msg = event(
        &state,
        "msg1",
        "owner",
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "Hello, world!".into(),
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].body, "Hello, world!");

    // Edit the message.
    let edit = event(
        &state,
        "e2",
        "owner",
        EventKind::EditMessage {
            message_id: "msg1".into(),
            new_body: "Hello, edited!".into(),
        },
    );
    assert_eq!(apply(&mut state, &edit), ApplyResult::Applied);
    assert_eq!(state.messages[0].body, "Hello, edited!");
    assert!(state.messages[0].edited);
}

#[test]
fn delete_message_is_soft() {
    let mut state = test_state();

    let create_ch = event(
        &state,
        "e0",
        "owner",
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1",
        "owner",
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "to be deleted".into(),
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);

    let delete = event(
        &state,
        "e2",
        "owner",
        EventKind::DeleteMessage {
            message_id: "msg1".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);

    // Message still exists but is marked deleted.
    assert_eq!(state.messages.len(), 1);
    assert!(state.messages[0].deleted);
    assert_eq!(state.messages[0].body, "[message deleted]");
}

#[test]
fn reaction_added_to_message() {
    let mut state = test_state();

    let create_ch = event(
        &state,
        "e0",
        "owner",
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1",
        "owner",
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "react to me".into(),
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);

    let reaction = event(
        &state,
        "e2",
        "owner",
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":+1:".into(),
        },
    );
    assert_eq!(apply(&mut state, &reaction), ApplyResult::Applied);

    assert_eq!(state.messages[0].reactions[":+1:"], vec!["owner"]);
}

// ── Trust lifecycle ──────────────────────────────────────────────────────

#[test]
fn trust_and_untrust() {
    let mut state = test_state();

    // Trust a peer.
    let trust = event(
        &state,
        "e1",
        "owner",
        EventKind::TrustPeer {
            peer_id: "alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &trust), ApplyResult::Applied);
    assert!(state.trusted_peers.contains("alice"));
    assert!(state.members.contains_key("alice"));

    // Untrust.
    let untrust = event(
        &state,
        "e2",
        "owner",
        EventKind::UntrustPeer {
            peer_id: "alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &untrust), ApplyResult::Applied);
    assert!(!state.trusted_peers.contains("alice"));
}

// ── Parent hash mismatch ─────────────────────────────────────────────────

#[test]
fn parent_hash_mismatch() {
    let mut state = test_state();

    let evt = Event {
        id: "e1".into(),
        parent_hash: StateHash::from_bytes(b"wrong-hash"),
        author: "owner".into(),
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    };

    assert_eq!(apply(&mut state, &evt), ApplyResult::ParentHashMismatch);
    // Channel should NOT have been created.
    assert!(!state.channels.contains_key("ch1"));
}

// ── Trust enforcement ────────────────────────────────────────────────────

#[test]
fn untrusted_author_rejected() {
    let mut state = test_state();

    // An untrusted peer tries to create a channel.
    let evt = event(
        &state,
        "e1",
        "stranger",
        EventKind::CreateChannel {
            name: "hacked".into(),
            channel_id: "ch1".into(),
        },
    );

    let result = apply(&mut state, &evt);
    assert!(matches!(result, ApplyResult::Rejected(_)));
    assert!(!state.channels.contains_key("ch1"));
}

#[test]
fn untrusted_peer_can_send_messages() {
    let mut state = test_state();

    let create_ch = event(
        &state,
        "e0",
        "owner",
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // An untrusted peer sends a message — this should be accepted.
    let msg = event(
        &state,
        "msg1",
        "stranger",
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hi from stranger".into(),
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);
}

// ── Full replay from genesis ─────────────────────────────────────────────

#[test]
fn full_replay_from_genesis() {
    // Build state incrementally.
    let mut state = test_state();

    let events = vec![event(
        &state,
        "e1",
        "owner",
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    )];
    assert_eq!(apply(&mut state, &events[0]), ApplyResult::Applied);
    let hash_after_e1 = state.hash();

    let e2 = event(
        &state,
        "e2",
        "owner",
        EventKind::TrustPeer {
            peer_id: "alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &e2), ApplyResult::Applied);

    let e3 = event(
        &state,
        "e3",
        "owner",
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "Hello".into(),
        },
    );
    assert_eq!(apply(&mut state, &e3), ApplyResult::Applied);
    let final_hash = state.hash();

    // Now replay all events from genesis on a fresh state.
    let mut replayed = test_state();
    let all_events = vec![events[0].clone(), e2, e3];
    for evt in &all_events {
        assert_eq!(apply_lenient(&mut replayed, evt), ApplyResult::Applied);
    }

    // Hashes should match after full replay.
    assert_eq!(replayed.hash(), final_hash);
    assert_ne!(replayed.hash(), hash_after_e1);

    // Verify state contents.
    assert!(replayed.channels.contains_key("ch1"));
    assert!(replayed.trusted_peers.contains("alice"));
    assert_eq!(replayed.messages.len(), 1);
}

// ── Merge ────────────────────────────────────────────────────────────────

#[test]
fn merge_produces_same_state() {
    let common = test_state();
    let common_hash = common.hash();

    // Peer A creates channel "alpha".
    let evt_a = event_with(
        "ea1",
        common_hash.clone(),
        "owner",
        100,
        EventKind::CreateChannel {
            name: "alpha".into(),
            channel_id: "ch-a".into(),
        },
    );

    // Peer B creates channel "beta".
    let evt_b = event_with(
        "eb1",
        common_hash,
        "owner",
        200,
        EventKind::CreateChannel {
            name: "beta".into(),
            channel_id: "ch-b".into(),
        },
    );

    // Merge from A's perspective.
    let (state_a, events_a) = merge(&[evt_a.clone()], &[evt_b.clone()], &common);

    // Merge from B's perspective.
    let (state_b, events_b) = merge(&[evt_b], &[evt_a], &common);

    // Both should produce the same state and event order.
    assert_eq!(state_a.hash(), state_b.hash());
    assert_eq!(events_a.len(), events_b.len());
    assert!(state_a.channels.contains_key("ch-a"));
    assert!(state_a.channels.contains_key("ch-b"));
}

// ── Event store ──────────────────────────────────────────────────────────

#[test]
fn event_store_in_memory() {
    let mut store = InMemoryStore::new();
    assert_eq!(store.latest_hash(), StateHash::ZERO);
    assert!(store.all_events().is_empty());

    let evt = Event {
        id: "e1".into(),
        parent_hash: StateHash::ZERO,
        author: "owner".into(),
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    };

    store.append(evt);
    assert_eq!(store.all_events().len(), 1);
    assert!(store.contains("e1"));
    assert!(!store.contains("e2"));

    let hash = StateHash::from_bytes(b"new-state");
    store.set_latest_hash(hash.clone());
    assert_eq!(store.latest_hash(), hash);
}

// ── Role lifecycle ───────────────────────────────────────────────────────

#[test]
fn create_role_and_assign() {
    let mut state = test_state();

    // Trust alice first.
    let trust = event(
        &state,
        "e0",
        "owner",
        EventKind::TrustPeer {
            peer_id: "alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &trust), ApplyResult::Applied);

    // Create a role.
    let create_role = event(
        &state,
        "e1",
        "owner",
        EventKind::CreateRole {
            name: "Moderator".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create_role), ApplyResult::Applied);
    assert!(state.roles.contains_key("role1"));

    // Set a permission.
    let set_perm = event(
        &state,
        "e2",
        "owner",
        EventKind::SetPermission {
            role_id: "role1".into(),
            permission: "ManageMessages".into(),
            granted: true,
        },
    );
    assert_eq!(apply(&mut state, &set_perm), ApplyResult::Applied);
    assert!(state.roles["role1"].permissions.contains("ManageMessages"));

    // Assign role to alice.
    let assign = event(
        &state,
        "e3",
        "owner",
        EventKind::AssignRole {
            peer_id: "alice".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &assign), ApplyResult::Applied);
    assert!(state.members["alice"].roles.contains("role1"));
}

#[test]
fn delete_role_removes_from_members() {
    let mut state = test_state();

    let trust = event(
        &state,
        "e0",
        "owner",
        EventKind::TrustPeer {
            peer_id: "alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &trust), ApplyResult::Applied);

    let create = event(
        &state,
        "e1",
        "owner",
        EventKind::CreateRole {
            name: "Temp".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);

    let assign = event(
        &state,
        "e2",
        "owner",
        EventKind::AssignRole {
            peer_id: "alice".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &assign), ApplyResult::Applied);
    assert!(state.members["alice"].roles.contains("role1"));

    let delete = event(
        &state,
        "e3",
        "owner",
        EventKind::DeleteRole {
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);
    assert!(!state.roles.contains_key("role1"));
    assert!(!state.members["alice"].roles.contains("role1"));
}

// ── Kick member ──────────────────────────────────────────────────────────

#[test]
fn kick_member_removes_and_untrusts() {
    let mut state = test_state();

    let trust = event(
        &state,
        "e0",
        "owner",
        EventKind::TrustPeer {
            peer_id: "alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &trust), ApplyResult::Applied);
    assert!(state.members.contains_key("alice"));
    assert!(state.trusted_peers.contains("alice"));

    let kick = event(
        &state,
        "e1",
        "owner",
        EventKind::KickMember {
            peer_id: "alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &kick), ApplyResult::Applied);
    assert!(!state.members.contains_key("alice"));
    assert!(!state.trusted_peers.contains("alice"));
}

#[test]
fn cannot_kick_owner() {
    let mut state = test_state();

    let kick = event(
        &state,
        "e1",
        "owner",
        EventKind::KickMember {
            peer_id: "owner".into(),
        },
    );
    assert_eq!(apply(&mut state, &kick), ApplyResult::Applied);
    // Owner should still be a member.
    assert!(state.members.contains_key("owner"));
}

// ── Profile ──────────────────────────────────────────────────────────────

#[test]
fn set_profile() {
    let mut state = test_state();

    let evt = event(
        &state,
        "e1",
        "owner",
        EventKind::SetProfile {
            display_name: "Alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    assert_eq!(state.profiles["owner"].display_name, "Alice");
    assert_eq!(
        state.members["owner"].display_name.as_deref(),
        Some("Alice")
    );
}

// ── Delete channel also removes messages ─────────────────────────────────

#[test]
fn delete_channel_removes_messages() {
    let mut state = test_state();

    let create = event(
        &state,
        "e0",
        "owner",
        EventKind::CreateChannel {
            name: "temp".into(),
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1",
        "owner",
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "will be removed".into(),
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);

    let delete = event(
        &state,
        "e1",
        "owner",
        EventKind::DeleteChannel {
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);
    assert!(state.messages.is_empty());
}
