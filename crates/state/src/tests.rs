//! Tests for the event-sourced state machine.
//!
//! Covers determinism, idempotency, channel/message lifecycle, permission
//! enforcement, divergence detection, merge, and the in-memory event store.

use crate::hash::StateHash;
use crate::merge::merge;
use crate::server::ServerState;
use crate::store::{EventStore, InMemoryStore};
use crate::types::Permission;
use crate::{apply, apply_lenient, ApplyResult, Event, EventKind};
use willow_identity::{EndpointId, Identity};

/// Helper: create a server state with a generated owner.
fn test_state() -> (ServerState, EndpointId) {
    let owner = Identity::generate().endpoint_id();
    (ServerState::new("server-1", "Test Server", owner), owner)
}

/// Helper: create an event with the current state's hash as parent.
fn event(state: &ServerState, id: &str, author: EndpointId, kind: EventKind) -> Event {
    Event {
        id: id.to_string(),
        parent_hash: state.hash(),
        author,
        timestamp_ms: 1000,
        kind,
    }
}

/// Helper: create an event with an explicit parent hash and timestamp.
fn event_with(id: &str, parent: StateHash, author: EndpointId, ts: u64, kind: EventKind) -> Event {
    Event {
        id: id.to_string(),
        parent_hash: parent,
        author,
        timestamp_ms: ts,
        kind,
    }
}

// ── Determinism ──────────────────────────────────────────────────────────

#[test]
fn apply_is_deterministic() {
    let alice = Identity::generate().endpoint_id();
    let events = [
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
        EventKind::CreateChannel {
            name: "random".into(),
            channel_id: "ch2".into(),
            kind: "text".to_string(),
        },
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    ];

    // Apply the same events to two independent states.
    let owner = Identity::generate().endpoint_id();
    let mut state_a = ServerState::new("server-1", "Test Server", owner);
    let mut state_b = ServerState::new("server-1", "Test Server", owner);

    for (i, kind) in events.iter().enumerate() {
        let evt_a = event(&state_a, &format!("e{i}"), owner, kind.clone());
        assert_eq!(apply(&mut state_a, &evt_a), ApplyResult::Applied);

        let evt_b = event(&state_b, &format!("e{i}"), owner, kind.clone());
        assert_eq!(apply(&mut state_b, &evt_b), ApplyResult::Applied);
    }

    assert_eq!(state_a.hash(), state_b.hash());
}

// ── Idempotency ──────────────────────────────────────────────────────────

#[test]
fn apply_is_idempotent() {
    let (mut state, owner) = test_state();
    let evt = event(
        &state,
        "e1",
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );

    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    // Same event again should be AlreadySeen.
    assert_eq!(apply_lenient(&mut state, &evt), ApplyResult::AlreadySeen);
}

// ── Channel lifecycle ────────────────────────────────────────────────────

#[test]
fn create_and_delete_channel() {
    let (mut state, owner) = test_state();

    // Create channel.
    let create = event(
        &state,
        "e1",
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);
    assert!(state.channels.contains_key("ch1"));
    assert_eq!(state.channels["ch1"].name, "general");

    // Delete channel.
    let delete = event(
        &state,
        "e2",
        owner,
        EventKind::DeleteChannel {
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);
    assert!(!state.channels.contains_key("ch1"));
}

#[test]
fn rename_channel() {
    let (mut state, owner) = test_state();

    let create = event(
        &state,
        "e1",
        owner,
        EventKind::CreateChannel {
            name: "old-name".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);

    let rename = event(
        &state,
        "e2",
        owner,
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
    let (mut state, owner) = test_state();

    // Create a channel first.
    let create_ch = event(
        &state,
        "e0", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Send a message.
    let msg = event(
        &state,
        "msg1", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "Hello, world!".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].body, "Hello, world!");

    // Edit the message.
    let edit = event(
        &state,
        "e2", owner,
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
    let (mut state, owner) = test_state();
    let create_ch = event(
        &state,
        "e0", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "to be deleted".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);

    let delete = event(
        &state,
        "e2", owner,
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
    let (mut state, owner) = test_state();
    let create_ch = event(
        &state,
        "e0", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "react to me".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);

    let reaction = event(
        &state,
        "e2", owner,
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":+1:".into(),
        },
    );
    assert_eq!(apply(&mut state, &reaction), ApplyResult::Applied);

    assert_eq!(state.messages[0].reactions[":+1:"], vec![owner]);
}

// ── Permission lifecycle ─────────────────────────────────────────────────

#[test]
fn grant_and_revoke_permission() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    // Grant a permission.
    let grant = event(
        &state,
        "e1", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.has_permission(&alice, &Permission::ManageChannels));
    assert!(state.members.contains_key(&alice));

    // Revoke.
    let revoke = event(
        &state,
        "e2", owner,
        EventKind::RevokePermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &revoke), ApplyResult::Applied);
    assert!(!state.has_permission(&alice, &Permission::ManageChannels));
}

// ── Parent hash mismatch ─────────────────────────────────────────────────

#[test]
fn parent_hash_mismatch() {
    let (mut state, owner) = test_state();
    let evt = Event {
        id: "e1".into(),
        parent_hash: StateHash::from_bytes(b"wrong-hash"),
        author: owner,
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    };

    assert_eq!(apply(&mut state, &evt), ApplyResult::ParentHashMismatch);
    // Channel should NOT have been created.
    assert!(!state.channels.contains_key("ch1"));
}

// ── Permission enforcement ──────────────────────────────────────────────

#[test]
fn unpermitted_author_rejected() {
    let (mut state, _owner) = test_state();
    let stranger = Identity::generate().endpoint_id();

    // An unpermitted peer tries to create a channel.
    let evt = event(
        &state,
        "e1", stranger,
        EventKind::CreateChannel {
            name: "hacked".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );

    let result = apply(&mut state, &evt);
    assert!(matches!(result, ApplyResult::Rejected(_)));
    assert!(!state.channels.contains_key("ch1"));
}

#[test]
fn unpermitted_peer_can_send_messages() {
    let (mut state, owner) = test_state();
    let stranger = Identity::generate().endpoint_id();

    let create_ch = event(
        &state,
        "e0", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // A peer with no permissions sends a message -- this should be accepted.
    let msg = event(
        &state,
        "msg1", stranger,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hi from stranger".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);
}

#[test]
fn permission_enforcement() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    // Grant alice only SendMessages.
    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Alice should NOT be able to create channels (requires ManageChannels).
    let create = event(
        &state,
        "e1", alice,
        EventKind::CreateChannel {
            name: "unauthorized".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    let result = apply(&mut state, &create);
    assert!(matches!(result, ApplyResult::Rejected(_)));
    assert!(!state.channels.contains_key("ch1"));
}

#[test]
fn admin_permission_grants_all() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();

    // Grant alice Administrator.
    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::Administrator,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Admin can create channels.
    let create = event(
        &state,
        "e1", alice,
        EventKind::CreateChannel {
            name: "admin-channel".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);
    assert!(state.channels.contains_key("ch1"));

    // Admin can create roles.
    let create_role = event(
        &state,
        "e2", alice,
        EventKind::CreateRole {
            name: "Moderator".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create_role), ApplyResult::Applied);
    assert!(state.roles.contains_key("role1"));

    // Admin can kick members.
    // First add bob as a member.
    let grant_bob = event(
        &state,
        "e3", alice,
        EventKind::GrantPermission {
            peer_id: bob,
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant_bob), ApplyResult::Applied);
    assert!(state.members.contains_key(&bob));

    let kick = event(
        &state,
        "e4", alice,
        EventKind::KickMember {
            peer_id: bob,
        },
    );
    assert_eq!(apply(&mut state, &kick), ApplyResult::Applied);
    assert!(!state.members.contains_key(&bob));
}

#[test]
fn owner_always_has_permission() {
    let (state, owner) = test_state();
    // Owner has every permission without any explicit grants.
    assert!(state.has_permission(&owner, &Permission::ManageChannels));
    assert!(state.has_permission(&owner, &Permission::ManageRoles));
    assert!(state.has_permission(&owner, &Permission::KickMembers));
    assert!(state.has_permission(&owner, &Permission::SendMessages));
    assert!(state.has_permission(&owner, &Permission::SyncProvider));
    assert!(state.has_permission(&owner, &Permission::CreateInvite));
    assert!(state.has_permission(&owner, &Permission::Administrator));
}

#[test]
fn fine_grained_permissions() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();

    // Grant alice only SendMessages.
    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Create a channel (as owner) so alice can send messages.
    let create_ch = event(
        &state,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Alice can send messages (messages don't require permissions).
    let msg = event(
        &state,
        "msg1", alice,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello from alice".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);

    // Alice cannot create channels.
    let create = event(
        &state,
        "e2", alice,
        EventKind::CreateChannel {
            name: "unauthorized".into(),
            channel_id: "ch2".into(),
            kind: "text".to_string(),
        },
    );
    assert!(matches!(
        apply(&mut state, &create),
        ApplyResult::Rejected(_)
    ));

    // Alice cannot kick members.
    let kick = event(
        &state,
        "e3", alice,
        EventKind::KickMember {
            peer_id: owner,
        },
    );
    assert!(matches!(apply(&mut state, &kick), ApplyResult::Rejected(_)));

    // Alice cannot create roles.
    let create_role = event(
        &state,
        "e4", alice,
        EventKind::CreateRole {
            name: "Admin".into(),
            role_id: "role1".into(),
        },
    );
    assert!(matches!(
        apply(&mut state, &create_role),
        ApplyResult::Rejected(_)
    ));

    // Alice cannot grant permissions.
    let grant_perm = event(
        &state,
        "e5", alice,
        EventKind::GrantPermission {
            peer_id: bob,
            permission: Permission::Administrator,
        },
    );
    assert!(matches!(
        apply(&mut state, &grant_perm),
        ApplyResult::Rejected(_)
    ));
}

// ── Full replay from genesis ─────────────────────────────────────────────

#[test]
fn full_replay_from_genesis() {
    // Build state incrementally.
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    let e1 = event(
        &state,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &e1), ApplyResult::Applied);
    let hash_after_e1 = state.hash();

    let e2 = event(
        &state,
        "e2", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &e2), ApplyResult::Applied);

    let e3 = event(
        &state,
        "e3", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "Hello".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &e3), ApplyResult::Applied);
    let final_hash = state.hash();

    // Now replay all events from genesis on a fresh state with the SAME owner.
    let mut replayed = ServerState::new("server-1", "Test Server", owner);
    let all_events = vec![e1, e2, e3];
    for evt in &all_events {
        assert_eq!(apply_lenient(&mut replayed, evt), ApplyResult::Applied);
    }

    // Hashes should match after full replay.
    assert_eq!(replayed.hash(), final_hash);
    assert_ne!(replayed.hash(), hash_after_e1);

    // Verify state contents.
    assert!(replayed.channels.contains_key("ch1"));
    assert!(replayed.has_permission(&alice, &Permission::ManageChannels));
    assert_eq!(replayed.messages.len(), 1);
}

// ── Merge ────────────────────────────────────────────────────────────────

#[test]
fn merge_produces_same_state() {
    let (common, owner) = test_state();
    let common_hash = common.hash();

    // Peer A creates channel "alpha".
    let evt_a = event_with(
        "ea1",
        common_hash.clone(),
        owner,
        100,
        EventKind::CreateChannel {
            name: "alpha".into(),
            channel_id: "ch-a".into(),
            kind: "text".to_string(),
        },
    );

    // Peer B creates channel "beta".
    let evt_b = event_with(
        "eb1",
        common_hash, owner,
        200,
        EventKind::CreateChannel {
            name: "beta".into(),
            channel_id: "ch-b".into(),
            kind: "text".to_string(),
        },
    );

    // Merge from A's perspective.
    let (state_a, events_a) = merge(std::slice::from_ref(&evt_a), std::slice::from_ref(&evt_b), &common);

    // Merge from B's perspective.
    let (state_b, events_b) = merge(std::slice::from_ref(&evt_b), std::slice::from_ref(&evt_a), &common);

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

    let owner = Identity::generate().endpoint_id();
    let evt = Event {
        id: "e1".into(),
        parent_hash: StateHash::ZERO,
        author: owner,
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
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
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    // Grant alice ManageRoles so she becomes a member.
    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageRoles,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Create a role.
    let create_role = event(
        &state,
        "e1", owner,
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
        "e2", owner,
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
        "e3", owner,
        EventKind::AssignRole {
            peer_id: alice,
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &assign), ApplyResult::Applied);
    assert!(state.members[&alice].roles.contains("role1"));
}

#[test]
fn delete_role_removes_from_members() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageRoles,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    let create = event(
        &state,
        "e1", owner,
        EventKind::CreateRole {
            name: "Temp".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);

    let assign = event(
        &state,
        "e2", owner,
        EventKind::AssignRole {
            peer_id: alice,
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &assign), ApplyResult::Applied);
    assert!(state.members[&alice].roles.contains("role1"));

    let delete = event(
        &state,
        "e3", owner,
        EventKind::DeleteRole {
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);
    assert!(!state.roles.contains_key("role1"));
    assert!(!state.members[&alice].roles.contains("role1"));
}

// ── Kick member ──────────────────────────────────────────────────────────

#[test]
fn kick_member_removes_and_revokes_permissions() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.members.contains_key(&alice));
    assert!(state.has_permission(&alice, &Permission::ManageChannels));

    let kick = event(
        &state,
        "e1", owner,
        EventKind::KickMember {
            peer_id: alice,
        },
    );
    assert_eq!(apply(&mut state, &kick), ApplyResult::Applied);
    assert!(!state.members.contains_key(&alice));
    assert!(!state.has_permission(&alice, &Permission::ManageChannels));
}

#[test]
fn cannot_kick_owner() {
    let (mut state, owner) = test_state();
    let kick = event(
        &state,
        "e1", owner,
        EventKind::KickMember {
            peer_id: owner,
        },
    );
    assert_eq!(apply(&mut state, &kick), ApplyResult::Applied);
    // Owner should still be a member.
    assert!(state.members.contains_key(&owner));
}

// ── Profile ──────────────────────────────────────────────────────────────

#[test]
fn set_profile() {
    let (mut state, owner) = test_state();
    let evt = event(
        &state,
        "e1", owner,
        EventKind::SetProfile {
            display_name: "Alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    assert_eq!(state.profiles[&owner].display_name, "Alice");
    assert_eq!(
        state.members[&owner].display_name.as_deref(),
        Some("Alice")
    );
}

// ── Delete channel also removes messages ─────────────────────────────────

#[test]
fn delete_channel_removes_messages() {
    let (mut state, owner) = test_state();
    let create = event(
        &state,
        "e0", owner,
        EventKind::CreateChannel {
            name: "temp".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "will be removed".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);

    let delete = event(
        &state,
        "e1", owner,
        EventKind::DeleteChannel {
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);
    assert!(state.messages.is_empty());
}

// ── Sync provider ────────────────────────────────────────────────────────

#[test]
fn sync_provider_permission() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    // Alice is not a sync provider by default.
    assert!(!state.is_sync_provider(&alice));

    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::SyncProvider,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.is_sync_provider(&alice));

    // Owner is always a sync provider.
    assert!(state.is_sync_provider(&owner));
}

// ── Backward compat: is_trusted ──────────────────────────────────────────

#[test]
fn is_trusted_compat() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let stranger = Identity::generate().endpoint_id();

    // Owner is always trusted.
    assert!(state.is_trusted(&owner));
    // Stranger with no permissions is not trusted.
    assert!(!state.is_trusted(&stranger));

    // Grant any permission makes a peer "trusted".
    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.is_trusted(&alice));

    // Revoke all permissions makes peer untrusted again.
    let revoke = event(
        &state,
        "e1", owner,
        EventKind::RevokePermission {
            peer_id: alice,
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &revoke), ApplyResult::Applied);
    assert!(!state.is_trusted(&alice));
}

// ── Multiple permissions per peer ────────────────────────────────────────

#[test]
fn multiple_permissions_per_peer() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let g1 = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &g1), ApplyResult::Applied);

    let g2 = event(
        &state,
        "e1", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::KickMembers,
        },
    );
    assert_eq!(apply(&mut state, &g2), ApplyResult::Applied);

    assert!(state.has_permission(&alice, &Permission::ManageChannels));
    assert!(state.has_permission(&alice, &Permission::KickMembers));
    assert!(!state.has_permission(&alice, &Permission::ManageRoles));

    // Revoke one, keep the other.
    let r1 = event(
        &state,
        "e2", owner,
        EventKind::RevokePermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &r1), ApplyResult::Applied);

    assert!(!state.has_permission(&alice, &Permission::ManageChannels));
    assert!(state.has_permission(&alice, &Permission::KickMembers));
    // Alice is still "trusted" because she has at least one permission.
    assert!(state.is_trusted(&alice));
}

// ── Multi-peer scenario tests ────────────────────────────────────────────

/// Helper: create an event with a UUID and the current state hash as parent.
fn make_event(state: &ServerState, author: EndpointId, kind: EventKind) -> Event {
    Event {
        id: uuid::Uuid::new_v4().to_string(),
        parent_hash: state.hash(),
        author,
        timestamp_ms: 1000,
        kind,
    }
}

#[test]
fn five_peers_concurrent_messages() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();
    let dave = Identity::generate().endpoint_id();
    let carol = Identity::generate().endpoint_id();

    // Create a channel first.
    let create_ch = make_event(
        &state,
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // 5 peers all send messages.
    let eve = Identity::generate().endpoint_id();
    let peers = [alice, bob, carol, dave, eve];
    for (i, peer) in peers.iter().enumerate() {
        let msg = make_event(
            &state,
            *peer,
            EventKind::Message {
                channel_id: "ch1".into(),
                body: format!("Hello from peer #{i}"),
                reply_to: None,
            },
        );
        assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    }

    // All 5 messages should be in the state.
    assert_eq!(state.messages.len(), 5);
    for peer in &peers {
        assert!(state.messages.iter().any(|m| m.author == *peer));
    }
}

#[test]
fn permission_cascade() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();

    // Owner grants Admin to peer A.
    let grant_admin = make_event(
        &state,
        owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::Administrator,
        },
    );
    assert_eq!(apply(&mut state, &grant_admin), ApplyResult::Applied);

    // Peer A grants ManageChannels to peer B (Admin can do this).
    let grant_manage = make_event(
        &state,
        alice,
        EventKind::GrantPermission {
            peer_id: bob,
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &grant_manage), ApplyResult::Applied);

    // Peer B creates a channel (they have ManageChannels).
    let create_ch = make_event(
        &state,
        bob,
        EventKind::CreateChannel {
            name: "dev".into(),
            channel_id: "ch-dev".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Verify the channel exists and the permission chain holds.
    assert!(state.channels.contains_key("ch-dev"));
    assert!(state.has_permission(&alice, &Permission::Administrator));
    assert!(state.has_permission(&bob, &Permission::ManageChannels));
}

#[test]
fn kick_revokes_all_permissions() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    // Owner grants multiple permissions to a peer.
    for perm in [
        Permission::ManageChannels,
        Permission::KickMembers,
        Permission::SendMessages,
    ] {
        let grant = make_event(
            &state,
            owner,
            EventKind::GrantPermission {
                peer_id: alice,
                permission: perm,
            },
        );
        assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    }
    assert!(state.members.contains_key(&alice));
    assert!(state.has_permission(&alice, &Permission::ManageChannels));
    assert!(state.has_permission(&alice, &Permission::KickMembers));

    // Owner kicks the peer.
    let kick = make_event(
        &state,
        owner,
        EventKind::KickMember {
            peer_id: alice,
        },
    );
    assert_eq!(apply(&mut state, &kick), ApplyResult::Applied);

    // All permissions and membership should be revoked.
    assert!(!state.members.contains_key(&alice));
    assert!(!state.has_permission(&alice, &Permission::ManageChannels));
    assert!(!state.has_permission(&alice, &Permission::KickMembers));
    assert!(!state.has_permission(&alice, &Permission::SendMessages));
    assert!(!state.is_trusted(&alice));

    // Kicked peer can still send messages (messages are open to all),
    // but cannot perform privileged operations.
    let create_ch = make_event(
        &state,
        alice,
        EventKind::CreateChannel {
            name: "sneaky".into(),
            channel_id: "ch-sneaky".into(),
            kind: "text".to_string(),
        },
    );
    assert!(matches!(
        apply(&mut state, &create_ch),
        ApplyResult::Rejected(_)
    ));
}

#[test]
fn concurrent_channel_create_same_name() {
    // Two events creating a channel with the same channel_id concurrently.
    // The first (by timestamp) should succeed, the second should be a no-op.
    let (common, owner) = test_state();
    let common_hash = common.hash();

    let evt_a = event_with(
        "ea1",
        common_hash.clone(),
        owner,
        100,
        EventKind::CreateChannel {
            name: "dev".into(),
            channel_id: "ch-dev".into(),
            kind: "text".to_string(),
        },
    );

    let evt_b = event_with(
        "eb1",
        common_hash, owner,
        200,
        EventKind::CreateChannel {
            name: "dev-duplicate".into(),
            channel_id: "ch-dev".into(),
            kind: "text".to_string(),
        },
    );

    // Merge them: both applied leniently, but second is a no-op since
    // channel_id already exists.
    let (merged_state, events) = merge(&[evt_a], &[evt_b], &common);

    assert_eq!(events.len(), 2);
    assert!(merged_state.channels.contains_key("ch-dev"));
    // The first event (timestamp 100) determines the channel name.
    assert_eq!(merged_state.channels["ch-dev"].name, "dev");
}

#[test]
fn edit_and_delete_message_lifecycle() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();

    // Create channel.
    let create_ch = make_event(
        &state,
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Peer sends message.
    let msg = event(
        &state,
        "msg1", alice,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "original text".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages[0].body, "original text");

    // Sender edits it.
    let edit = make_event(
        &state,
        alice,
        EventKind::EditMessage {
            message_id: "msg1".into(),
            new_body: "edited text".into(),
        },
    );
    assert_eq!(apply(&mut state, &edit), ApplyResult::Applied);
    assert_eq!(state.messages[0].body, "edited text");
    assert!(state.messages[0].edited);

    // Another peer reacts.
    let react = make_event(
        &state,
        bob,
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":thumbsup:".into(),
        },
    );
    assert_eq!(apply(&mut state, &react), ApplyResult::Applied);
    assert!(state.messages[0].reactions.contains_key(":thumbsup:"));

    // Sender deletes it.
    let delete = make_event(
        &state,
        alice,
        EventKind::DeleteMessage {
            message_id: "msg1".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);

    // Message should be marked deleted with reactions cleared.
    assert!(state.messages[0].deleted);
    assert_eq!(state.messages[0].body, "[message deleted]");
    assert!(state.messages[0].reactions.is_empty());
}

#[test]
fn merge_with_concurrent_mutations() {
    // Two peers diverge from the same state.
    let (common, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let common_hash = common.hash();

    // Peer A creates channel "dev" and sends a message.
    let evt_a1 = event_with(
        "ea1",
        common_hash.clone(),
        owner,
        100,
        EventKind::CreateChannel {
            name: "dev".into(),
            channel_id: "ch-dev".into(),
            kind: "text".to_string(),
        },
    );
    let evt_a2 = event_with(
        "ea2",
        common_hash.clone(),
        owner,
        101,
        EventKind::Message {
            channel_id: "ch-dev".into(),
            body: "First dev message".into(),
            reply_to: None,
        },
    );

    // Peer B creates channel "staging" and grants a permission.
    let evt_b1 = event_with(
        "eb1",
        common_hash.clone(),
        owner,
        150,
        EventKind::CreateChannel {
            name: "staging".into(),
            channel_id: "ch-staging".into(),
            kind: "text".to_string(),
        },
    );
    let evt_b2 = event_with(
        "eb2",
        common_hash, owner,
        151,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    );

    // Merge the two histories.
    let (merged_state, events) = merge(&[evt_a1, evt_a2], &[evt_b1, evt_b2], &common);

    // All 4 events should be in the merged history.
    assert_eq!(events.len(), 4);
    // Both channels should exist.
    assert!(merged_state.channels.contains_key("ch-dev"));
    assert!(merged_state.channels.contains_key("ch-staging"));
    // Message should be there.
    assert_eq!(merged_state.messages.len(), 1);
    assert_eq!(merged_state.messages[0].body, "First dev message");
    // Permission should be granted.
    assert!(merged_state.has_permission(&alice, &Permission::ManageChannels));
}

#[test]
fn merge_with_conflicting_deletes() {
    // Start with two channels in the common state.
    let (mut common, owner) = test_state();
    let create_a = make_event(
        &common,
        owner,
        EventKind::CreateChannel {
            name: "alpha".into(),
            channel_id: "ch-alpha".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut common, &create_a), ApplyResult::Applied);
    let create_b = make_event(
        &common,
        owner,
        EventKind::CreateChannel {
            name: "beta".into(),
            channel_id: "ch-beta".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut common, &create_b), ApplyResult::Applied);
    assert!(common.channels.contains_key("ch-alpha"));
    assert!(common.channels.contains_key("ch-beta"));

    let common_hash = common.hash();

    // Peer A deletes "alpha".
    let del_a = event_with(
        "da1",
        common_hash.clone(),
        owner,
        100,
        EventKind::DeleteChannel {
            channel_id: "ch-alpha".into(),
        },
    );

    // Peer B deletes "beta".
    let del_b = event_with(
        "db1",
        common_hash, owner,
        200,
        EventKind::DeleteChannel {
            channel_id: "ch-beta".into(),
        },
    );

    // After merge, both channels should be deleted.
    let (merged_state, events) = merge(&[del_a], &[del_b], &common);
    assert_eq!(events.len(), 2);
    assert!(!merged_state.channels.contains_key("ch-alpha"));
    assert!(!merged_state.channels.contains_key("ch-beta"));
}

#[test]
fn replay_100_events_produces_correct_state() {
    // Build all events once, apply them, then replay the same events
    // on a fresh state to verify identical hash.
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();
    let carol = Identity::generate().endpoint_id();
    let mut all_events = Vec::new();
    let authors = [owner, alice, bob, carol];

    // Create 5 channels.
    for i in 0..5 {
        let evt = make_event(
            &state,
            owner,
            EventKind::CreateChannel {
                name: format!("channel-{i}"),
                channel_id: format!("ch-{i}"),
                kind: "text".to_string(),
            },
        );
        assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
        all_events.push(evt);
    }

    // Grant permissions to 3 peers.
    for peer in [alice, bob, carol] {
        let evt = make_event(
            &state,
            owner,
            EventKind::GrantPermission {
                peer_id: peer,
                permission: Permission::SendMessages,
            },
        );
        assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
        all_events.push(evt);
    }

    // Create 2 roles.
    for i in 0..2 {
        let evt = make_event(
            &state,
            owner,
            EventKind::CreateRole {
                name: format!("Role-{i}"),
                role_id: format!("role-{i}"),
            },
        );
        assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
        all_events.push(evt);
    }

    // Send 90 messages across channels (100 - 5 channels - 3 perms - 2 roles).
    for i in 0..90 {
        let channel_id = format!("ch-{}", i % 5);
        let author = authors[i % authors.len()];
        let evt = make_event(
            &state,
            author,
            EventKind::Message {
                channel_id,
                body: format!("Message #{i}"),
                reply_to: None,
            },
        );
        assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
        all_events.push(evt);
    }

    let final_hash = state.hash();
    assert_eq!(all_events.len(), 100);

    // Verify state contents.
    assert_eq!(state.channels.len(), 5);
    assert_eq!(state.messages.len(), 90);
    assert_eq!(state.roles.len(), 2);
    // 1 owner + 3 granted peers.
    assert_eq!(state.members.len(), 4);

    // Replay the exact same events from scratch on a fresh state with the SAME owner.
    let mut store = InMemoryStore::new();
    let mut replay_state = ServerState::new("server-1", "Test Server", owner);

    for evt in &all_events {
        store.append(evt.clone());
        apply_lenient(&mut replay_state, evt);
    }

    // Replayed state should have the same hash.
    assert_eq!(replay_state.hash(), final_hash);
    assert_eq!(store.all_events().len(), 100);
}

#[test]
fn stress_1000_messages_same_channel() {
    let (mut state, owner) = test_state();
    // Create a channel.
    let create_ch = make_event(
        &state,
        owner,
        EventKind::CreateChannel {
            name: "stress-test".into(),
            channel_id: "ch-stress".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Rapid-fire 1000 messages from 10 different authors.
    let authors: Vec<EndpointId> = (0..10).map(|_| Identity::generate().endpoint_id()).collect();
    for i in 0..1000 {
        let author = authors[i % 10];
        let msg = make_event(
            &state,
            author,
            EventKind::Message {
                channel_id: "ch-stress".into(),
                body: format!("msg-{i}"),
                reply_to: None,
            },
        );
        assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    }

    assert_eq!(state.messages.len(), 1000);
    // Verify each author has exactly 100 messages.
    for author in &authors {
        let count = state
            .messages
            .iter()
            .filter(|m| m.author == *author)
            .count();
        assert_eq!(count, 100, "author {} should have 100 messages", author);
    }
}

#[test]
fn untrusted_peer_cant_escalate() {
    let (mut state, owner) = test_state();
    let stranger = Identity::generate().endpoint_id();

    // Create a channel (as owner) so there's something to interact with.
    let create_ch = make_event(
        &state,
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Grant stranger only SendMessages.
    let grant = make_event(
        &state,
        owner,
        EventKind::GrantPermission {
            peer_id: stranger,
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Stranger tries to create a channel (should fail).
    let create = make_event(
        &state,
        stranger,
        EventKind::CreateChannel {
            name: "hacked".into(),
            channel_id: "ch-hacked".into(),
            kind: "text".to_string(),
        },
    );
    assert!(matches!(
        apply(&mut state, &create),
        ApplyResult::Rejected(_)
    ));
    assert!(!state.channels.contains_key("ch-hacked"));

    // Stranger tries to grant themselves Admin (should fail).
    let self_grant = make_event(
        &state,
        stranger,
        EventKind::GrantPermission {
            peer_id: stranger,
            permission: Permission::Administrator,
        },
    );
    assert!(matches!(
        apply(&mut state, &self_grant),
        ApplyResult::Rejected(_)
    ));
    assert!(!state.has_permission(&stranger, &Permission::Administrator));

    // Stranger tries to kick another peer (should fail).
    let kick = make_event(
        &state,
        stranger,
        EventKind::KickMember {
            peer_id: owner,
        },
    );
    assert!(matches!(apply(&mut state, &kick), ApplyResult::Rejected(_)));
    assert!(state.members.contains_key(&owner));

    // Stranger can still send messages (messages are open).
    let msg = make_event(
        &state,
        stranger,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "I can only send messages".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);
}

#[test]
fn profile_history_through_events() {
    let (mut state, _owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    // Peer sets profile "Alice".
    let set1 = make_event(
        &state,
        alice,
        EventKind::SetProfile {
            display_name: "Alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &set1), ApplyResult::Applied);
    assert_eq!(state.profiles[&alice].display_name, "Alice");

    // Then changes to "Bob".
    let set2 = make_event(
        &state,
        alice,
        EventKind::SetProfile {
            display_name: "Bob".into(),
        },
    );
    assert_eq!(apply(&mut state, &set2), ApplyResult::Applied);
    assert_eq!(state.profiles[&alice].display_name, "Bob");

    // Then to "Charlie".
    let set3 = make_event(
        &state,
        alice,
        EventKind::SetProfile {
            display_name: "Charlie".into(),
        },
    );
    assert_eq!(apply(&mut state, &set3), ApplyResult::Applied);

    // Final state should show "Charlie".
    assert_eq!(state.profiles[&alice].display_name, "Charlie");
    // All three SetProfile events should have been applied (seen IDs).
    assert!(state.seen_event_ids.contains(&set1.id));
    assert!(state.seen_event_ids.contains(&set2.id));
    assert!(state.seen_event_ids.contains(&set3.id));
}

#[test]
fn state_hash_changes_on_every_mutation() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();
    let mut hashes = vec![state.hash()];

    // Apply 10 different events and collect the hash after each.
    let event_kinds = vec![
        EventKind::CreateChannel {
            name: "ch-1".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
        EventKind::CreateChannel {
            name: "ch-2".into(),
            channel_id: "ch2".into(),
            kind: "text".to_string(),
        },
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
        EventKind::GrantPermission {
            peer_id: bob,
            permission: Permission::SendMessages,
        },
        EventKind::CreateRole {
            name: "Mod".into(),
            role_id: "r1".into(),
        },
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
        EventKind::Message {
            channel_id: "ch2".into(),
            body: "world".into(),
            reply_to: None,
        },
        EventKind::SetProfile {
            display_name: "Owner".into(),
        },
        EventKind::RenameChannel {
            channel_id: "ch1".into(),
            new_name: "renamed".into(),
        },
        EventKind::DeleteChannel {
            channel_id: "ch2".into(),
        },
    ];

    for kind in event_kinds {
        let evt = make_event(&state, owner, kind);
        assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
        hashes.push(state.hash());
    }

    // No two hashes should be the same.
    assert_eq!(hashes.len(), 11);
    for i in 0..hashes.len() {
        for j in (i + 1)..hashes.len() {
            assert_ne!(
                hashes[i], hashes[j],
                "hashes at index {i} and {j} should differ"
            );
        }
    }
}

#[test]
fn idempotency_across_all_event_kinds() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();

    // Setup: create a channel and a role so downstream events have targets.
    let setup_ch = make_event(
        &state,
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &setup_ch), ApplyResult::Applied);
    let setup_role = make_event(
        &state,
        owner,
        EventKind::CreateRole {
            name: "Mod".into(),
            role_id: "r1".into(),
        },
    );
    assert_eq!(apply(&mut state, &setup_role), ApplyResult::Applied);

    // Grant alice permissions so she is a member for AssignRole.
    let grant_alice = make_event(
        &state,
        owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageRoles,
        },
    );
    assert_eq!(apply(&mut state, &grant_alice), ApplyResult::Applied);

    // Send a message so we have a message to edit/delete/react to.
    let msg_evt = event(
        &state,
        "msg-idem", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "test".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg_evt), ApplyResult::Applied);

    // Test each EventKind variant: apply, then apply again via apply_lenient.
    let variants: Vec<EventKind> = vec![
        EventKind::CreateChannel {
            name: "new-ch".into(),
            channel_id: "ch-new".into(),
            kind: "text".to_string(),
        },
        EventKind::DeleteChannel {
            channel_id: "ch-new".into(),
        },
        EventKind::RenameChannel {
            channel_id: "ch1".into(),
            new_name: "renamed".into(),
        },
        EventKind::CreateRole {
            name: "Admin".into(),
            role_id: "r2".into(),
        },
        EventKind::DeleteRole {
            role_id: "r2".into(),
        },
        EventKind::SetPermission {
            role_id: "r1".into(),
            permission: "TestPerm".into(),
            granted: true,
        },
        EventKind::AssignRole {
            peer_id: alice,
            role_id: "r1".into(),
        },
        EventKind::GrantPermission {
            peer_id: bob,
            permission: Permission::SendMessages,
        },
        EventKind::RevokePermission {
            peer_id: bob,
            permission: Permission::SendMessages,
        },
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "duplicate test".into(),
            reply_to: None,
        },
        EventKind::EditMessage {
            message_id: "msg-idem".into(),
            new_body: "edited".into(),
        },
        EventKind::DeleteMessage {
            message_id: "msg-idem".into(),
        },
        EventKind::Reaction {
            message_id: "msg-idem".into(),
            emoji: ":+1:".into(),
        },
        EventKind::SetProfile {
            display_name: "Test".into(),
        },
        EventKind::RotateChannelKey {
            channel_id: "ch1".into(),
            encrypted_keys: vec![(owner, vec![1, 2, 3])],
        },
    ];

    for kind in variants {
        let evt = make_event(&state, owner, kind);
        let first_result = apply(&mut state, &evt);
        // First application should succeed (Applied or Rejected, but not AlreadySeen).
        assert_ne!(
            first_result,
            ApplyResult::AlreadySeen,
            "first apply should not be AlreadySeen"
        );

        let hash_after_first = state.hash();

        // Second application should be AlreadySeen.
        let second_result = apply_lenient(&mut state, &evt);
        assert_eq!(second_result, ApplyResult::AlreadySeen);

        // Hash should be unchanged after duplicate.
        assert_eq!(state.hash(), hash_after_first);
    }
}

// ── Merge stress tests ───────────────────────────────────────────────────

#[test]
fn merge_three_way_divergence() {
    // Three peers diverge from the same state.
    let (common, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let common_hash = common.hash();

    // Peer A creates channel "alpha" and sends a message.
    let events_a = vec![
        event_with(
            "a1",
            common_hash.clone(),
            owner,
            100,
            EventKind::CreateChannel {
                name: "alpha".into(),
                channel_id: "ch-alpha".into(),
                kind: "text".to_string(),
            },
        ),
        event_with(
            "a2",
            common_hash.clone(),
            owner,
            101,
            EventKind::Message {
                channel_id: "ch-alpha".into(),
                body: "msg from A".into(),
                reply_to: None,
            },
        ),
    ];

    // Peer B creates channel "beta" and sends a message.
    let events_b = vec![
        event_with(
            "b1",
            common_hash.clone(),
            owner,
            200,
            EventKind::CreateChannel {
                name: "beta".into(),
                channel_id: "ch-beta".into(),
                kind: "text".to_string(),
            },
        ),
        event_with(
            "b2",
            common_hash.clone(),
            owner,
            201,
            EventKind::Message {
                channel_id: "ch-beta".into(),
                body: "msg from B".into(),
                reply_to: None,
            },
        ),
    ];

    // Peer C creates channel "gamma" and grants a permission.
    let events_c = vec![
        event_with(
            "c1",
            common_hash.clone(),
            owner,
            300,
            EventKind::CreateChannel {
                name: "gamma".into(),
                channel_id: "ch-gamma".into(),
                kind: "text".to_string(),
            },
        ),
        event_with(
            "c2",
            common_hash, owner,
            301,
            EventKind::GrantPermission {
                peer_id: alice,
                permission: Permission::ManageChannels,
            },
        ),
    ];

    // Merge A+B first.
    let (state_ab, events_ab) = merge(&events_a, &events_b, &common);
    assert_eq!(events_ab.len(), 4);
    assert!(state_ab.channels.contains_key("ch-alpha"));
    assert!(state_ab.channels.contains_key("ch-beta"));

    // Then merge AB+C.
    let (final_state, final_events) = merge(&events_ab, &events_c, &common);

    // All channels and messages from all three should be present.
    assert_eq!(final_events.len(), 6);
    assert!(final_state.channels.contains_key("ch-alpha"));
    assert!(final_state.channels.contains_key("ch-beta"));
    assert!(final_state.channels.contains_key("ch-gamma"));
    assert_eq!(final_state.messages.len(), 2);
    assert!(final_state.has_permission(&alice, &Permission::ManageChannels));
}

#[test]
fn merge_preserves_permission_chain() {
    // Start with a common state where owner has set things up.
    let (common, owner) = test_state();
    let bob = Identity::generate().endpoint_id();
    let common_hash = common.hash();

    // Peer A (owner): grants Admin to peer B (diverged from common).
    let events_a = vec![event_with(
        "a1",
        common_hash.clone(),
        owner,
        100,
        EventKind::GrantPermission {
            peer_id: bob,
            permission: Permission::Administrator,
        },
    )];

    // Peer B (in their diverged history): creates a role.
    // Note: B doesn't have Admin yet in their own view, but after merge
    // the permission grant comes first (timestamp 100 < 200).
    let events_b = vec![event_with(
        "b1",
        common_hash,
        bob,
        200,
        EventKind::CreateRole {
            name: "Moderator".into(),
            role_id: "role-mod".into(),
        },
    )];

    // After merge, the grant event (ts=100) comes before the create role (ts=200).
    let (merged_state, events) = merge(&events_a, &events_b, &common);
    assert_eq!(events.len(), 2);

    // Both the permission grant and the role should exist.
    assert!(merged_state.has_permission(&bob, &Permission::Administrator));
    assert!(merged_state.roles.contains_key("role-mod"));
}

// ── StateVerification ────────────────────────────────────────────────────

#[test]
fn state_verification_does_not_mutate_state() {
    let (mut state, owner) = test_state();
    let hash_before = state.hash();

    let evt = event(
        &state,
        "e1", owner,
        EventKind::StateVerification {
            state_hash: hash_before.clone(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);

    // Hash should be the same (seen_event_ids is excluded from hash).
    assert_eq!(state.hash(), hash_before);
}

#[test]
fn identical_states_produce_matching_hashes() {
    let owner = Identity::generate().endpoint_id();
    let mut state_a = ServerState::new("server-1", "Test Server", owner);
    let mut state_b = ServerState::new("server-1", "Test Server", owner);

    // Apply the same events to both.
    let create = event(
        &state_a,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state_a, &create), ApplyResult::Applied);
    assert_eq!(apply_lenient(&mut state_b, &create), ApplyResult::Applied);

    assert_eq!(state_a.hash(), state_b.hash());
}

#[test]
fn state_verification_accepted_from_any_peer() {
    let (mut state, _owner) = test_state();
    let stranger = Identity::generate().endpoint_id();

    // A stranger can send a StateVerification event.
    let evt = event(
        &state,
        "e1", stranger,
        EventKind::StateVerification {
            state_hash: state.hash(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
}

// ── Server renaming ──────────────────────────────────────────────────────

#[test]
fn owner_can_rename_server() {
    let (mut state, owner) = test_state();
    assert_eq!(state.server_name, "Test Server");

    let evt = event(
        &state,
        "e1", owner,
        EventKind::RenameServer {
            new_name: "Renamed Server".into(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    assert_eq!(state.server_name, "Renamed Server");
}

#[test]
fn non_owner_cannot_rename_server() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    // Grant alice ManageChannels to ensure she has some perms but is not owner.
    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    let rename = event(
        &state,
        "e1", alice,
        EventKind::RenameServer {
            new_name: "Hacked".into(),
        },
    );
    assert!(matches!(
        apply(&mut state, &rename),
        ApplyResult::Rejected(_)
    ));
    assert_eq!(state.server_name, "Test Server");
}

#[test]
fn rename_server_changes_hash() {
    let (mut state, owner) = test_state();
    let hash_before = state.hash();

    let evt = event(
        &state,
        "e1", owner,
        EventKind::RenameServer {
            new_name: "New Name".into(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    assert_ne!(state.hash(), hash_before);
}

// ── Server description ──────────────────────────────────────────────────

#[test]
fn owner_can_set_server_description() {
    let (mut state, owner) = test_state();
    assert_eq!(state.description, "");

    let evt = event(
        &state,
        "e1", owner,
        EventKind::SetServerDescription {
            description: "A cool server".into(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    assert_eq!(state.description, "A cool server");
}

#[test]
fn non_owner_cannot_set_server_description() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    let grant = event(
        &state,
        "e0", owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::Administrator,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    let desc = event(
        &state,
        "e1", alice,
        EventKind::SetServerDescription {
            description: "Unauthorized".into(),
        },
    );
    assert!(matches!(apply(&mut state, &desc), ApplyResult::Rejected(_)));
    assert_eq!(state.description, "");
}

#[test]
fn description_is_stored_and_accessible() {
    let (mut state, owner) = test_state();

    let evt = event(
        &state,
        "e1", owner,
        EventKind::SetServerDescription {
            description: "Welcome to the server".into(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    assert_eq!(state.description, "Welcome to the server");

    // Updating description replaces it.
    let evt2 = event(
        &state,
        "e2", owner,
        EventKind::SetServerDescription {
            description: "Updated description".into(),
        },
    );
    assert_eq!(apply(&mut state, &evt2), ApplyResult::Applied);
    assert_eq!(state.description, "Updated description");
}

#[test]
fn description_changes_hash() {
    let (mut state, owner) = test_state();
    let hash_before = state.hash();

    let evt = event(
        &state,
        "e1", owner,
        EventKind::SetServerDescription {
            description: "Something".into(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    assert_ne!(state.hash(), hash_before);
}

#[test]
fn rename_server_round_trip_serialization() {
    let kind = EventKind::RenameServer {
        new_name: "New Name".into(),
    };
    let bytes = bincode::serialize(&kind).unwrap();
    let decoded: EventKind = bincode::deserialize(&bytes).unwrap();
    assert!(matches!(decoded, EventKind::RenameServer { ref new_name } if new_name == "New Name"));
}

#[test]
fn set_server_description_round_trip_serialization() {
    let kind = EventKind::SetServerDescription {
        description: "My desc".into(),
    };
    let bytes = bincode::serialize(&kind).unwrap();
    let decoded: EventKind = bincode::deserialize(&bytes).unwrap();
    assert!(matches!(
        decoded,
        EventKind::SetServerDescription { ref description } if description == "My desc"
    ));
}

#[test]
fn state_verification_round_trip_serialization() {
    let kind = EventKind::StateVerification {
        state_hash: StateHash::from_bytes(b"test"),
    };
    let bytes = bincode::serialize(&kind).unwrap();
    let decoded: EventKind = bincode::deserialize(&bytes).unwrap();
    assert!(matches!(
        decoded,
        EventKind::StateVerification { ref state_hash } if *state_hash == StateHash::from_bytes(b"test")
    ));
}

// ── Pin / Unpin ────────────────────────────────────────────────────────

#[test]
fn pin_message_adds_to_channel() {
    let (mut state, owner) = test_state();

    // Create a channel.
    let e1 = event(
        &state,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &e1), ApplyResult::Applied);

    // Send a message.
    let e2 = event(
        &state,
        "e2", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &e2), ApplyResult::Applied);

    // Pin the message.
    let e3 = event(
        &state,
        "e3", owner,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: "e2".into(),
        },
    );
    assert_eq!(apply(&mut state, &e3), ApplyResult::Applied);

    let ch = state.channels.get("ch1").unwrap();
    assert!(ch.pinned_messages.contains("e2"));
    assert_eq!(ch.pinned_messages.len(), 1);
}

#[test]
fn unpin_message_removes_from_channel() {
    let (mut state, owner) = test_state();

    let e1 = event(
        &state,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &e1), ApplyResult::Applied);

    let e2 = event(
        &state,
        "e2", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &e2), ApplyResult::Applied);

    // Pin.
    let e3 = event(
        &state,
        "e3", owner,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: "e2".into(),
        },
    );
    assert_eq!(apply(&mut state, &e3), ApplyResult::Applied);
    assert!(state
        .channels
        .get("ch1")
        .unwrap()
        .pinned_messages
        .contains("e2"));

    // Unpin.
    let e4 = event(
        &state,
        "e4", owner,
        EventKind::UnpinMessage {
            channel_id: "ch1".into(),
            message_id: "e2".into(),
        },
    );
    assert_eq!(apply(&mut state, &e4), ApplyResult::Applied);
    assert!(!state
        .channels
        .get("ch1")
        .unwrap()
        .pinned_messages
        .contains("e2"));
    assert!(state
        .channels
        .get("ch1")
        .unwrap()
        .pinned_messages
        .is_empty());
}

#[test]
fn pin_duplicate_is_idempotent() {
    let (mut state, owner) = test_state();

    let e1 = event(
        &state,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &e1), ApplyResult::Applied);

    let e2 = event(
        &state,
        "e2", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &e2), ApplyResult::Applied);

    // Pin the message.
    let e3 = event(
        &state,
        "e3", owner,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: "e2".into(),
        },
    );
    assert_eq!(apply(&mut state, &e3), ApplyResult::Applied);

    // Pin the same message again (different event ID, same message_id).
    let e4 = event(
        &state,
        "e4", owner,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: "e2".into(),
        },
    );
    assert_eq!(apply(&mut state, &e4), ApplyResult::Applied);

    // Should still only have one entry (HashSet deduplication).
    let ch = state.channels.get("ch1").unwrap();
    assert_eq!(ch.pinned_messages.len(), 1);
    assert!(ch.pinned_messages.contains("e2"));
}

#[test]
fn pin_message_on_nonexistent_channel_is_noop() {
    let (mut state, owner) = test_state();

    // Pin to a channel that doesn't exist — should not panic, still Applied.
    let e1 = event(
        &state,
        "e1", owner,
        EventKind::PinMessage {
            channel_id: "no-such-channel".into(),
            message_id: "msg1".into(),
        },
    );
    let result = apply(&mut state, &e1);
    assert_eq!(result, ApplyResult::Applied);
    // No channel was created or modified.
    assert!(!state.channels.contains_key("no-such-channel"));
}

#[test]
fn unpin_nonexistent_message_is_noop() {
    let (mut state, owner) = test_state();

    // Create a channel.
    let e1 = event(
        &state,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &e1), ApplyResult::Applied);

    // Unpin a message that was never pinned — should not panic.
    let e2 = event(
        &state,
        "e2", owner,
        EventKind::UnpinMessage {
            channel_id: "ch1".into(),
            message_id: "never-pinned".into(),
        },
    );
    let result = apply(&mut state, &e2);
    assert_eq!(result, ApplyResult::Applied);
    assert!(state
        .channels
        .get("ch1")
        .unwrap()
        .pinned_messages
        .is_empty());
}

#[test]
fn pin_survives_state_replay() {
    let (mut state, owner) = test_state();

    // Create a channel.
    let e1 = event(
        &state,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &e1), ApplyResult::Applied);

    // Send a message.
    let e2 = event(
        &state,
        "e2", owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "important".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &e2), ApplyResult::Applied);

    // Pin it.
    let e3 = event(
        &state,
        "e3", owner,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: "e2".into(),
        },
    );
    assert_eq!(apply(&mut state, &e3), ApplyResult::Applied);
    assert!(state
        .channels
        .get("ch1")
        .unwrap()
        .pinned_messages
        .contains("e2"));

    // Replay all events on a fresh state with the SAME owner.
    let mut replayed = ServerState::new("server-1", "Test Server", owner);
    apply_lenient(&mut replayed, &e1);
    apply_lenient(&mut replayed, &e2);
    apply_lenient(&mut replayed, &e3);

    let ch = replayed.channels.get("ch1").unwrap();
    assert!(ch.pinned_messages.contains("e2"));
    assert_eq!(ch.pinned_messages.len(), 1);
}

#[test]
fn multiple_pins_per_channel() {
    let (mut state, owner) = test_state();

    let e1 = event(
        &state,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &e1), ApplyResult::Applied);

    // Send 3 messages and pin all of them.
    for (i, msg_id) in ["m1", "m2", "m3"].iter().enumerate() {
        let eid = format!("msg{}", i);
        let e = event(
            &state,
            &eid, owner,
            EventKind::Message {
                channel_id: "ch1".into(),
                body: format!("message {}", i),
                reply_to: None,
            },
        );
        assert_eq!(apply(&mut state, &e), ApplyResult::Applied);

        let pin_eid = format!("pin{}", i);
        let pin = event(
            &state,
            &pin_eid, owner,
            EventKind::PinMessage {
                channel_id: "ch1".into(),
                message_id: msg_id.to_string(),
            },
        );
        assert_eq!(apply(&mut state, &pin), ApplyResult::Applied);
    }

    let ch = state.channels.get("ch1").unwrap();
    assert_eq!(ch.pinned_messages.len(), 3);
    assert!(ch.pinned_messages.contains("m1"));
    assert!(ch.pinned_messages.contains("m2"));
    assert!(ch.pinned_messages.contains("m3"));
}

#[test]
fn pin_across_channels() {
    let (mut state, owner) = test_state();

    // Create two channels.
    let e1 = event(
        &state,
        "e1", owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &e1), ApplyResult::Applied);

    let e2 = event(
        &state,
        "e2", owner,
        EventKind::CreateChannel {
            name: "random".into(),
            channel_id: "ch2".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &e2), ApplyResult::Applied);

    // Pin a message in ch1.
    let e3 = event(
        &state,
        "e3", owner,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: "msg-a".into(),
        },
    );
    assert_eq!(apply(&mut state, &e3), ApplyResult::Applied);

    // Pin a different message in ch2.
    let e4 = event(
        &state,
        "e4", owner,
        EventKind::PinMessage {
            channel_id: "ch2".into(),
            message_id: "msg-b".into(),
        },
    );
    assert_eq!(apply(&mut state, &e4), ApplyResult::Applied);

    // Verify pins are isolated per channel.
    let ch1 = state.channels.get("ch1").unwrap();
    assert_eq!(ch1.pinned_messages.len(), 1);
    assert!(ch1.pinned_messages.contains("msg-a"));
    assert!(!ch1.pinned_messages.contains("msg-b"));

    let ch2 = state.channels.get("ch2").unwrap();
    assert_eq!(ch2.pinned_messages.len(), 1);
    assert!(ch2.pinned_messages.contains("msg-b"));
    assert!(!ch2.pinned_messages.contains("msg-a"));
}
