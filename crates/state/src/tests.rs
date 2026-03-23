//! Tests for the event-sourced state machine.
//!
//! Covers determinism, idempotency, channel/message lifecycle, permission
//! enforcement, divergence detection, merge, and the in-memory event store.

use crate::hash::StateHash;
use crate::merge::{find_common_ancestor, merge};
use crate::server::ServerState;
use crate::store::{EventStore, InMemoryStore};
use crate::types::Permission;
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
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::ManageChannels,
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

// ── Permission lifecycle ─────────────────────────────────────────────────

#[test]
fn grant_and_revoke_permission() {
    let mut state = test_state();

    // Grant a permission.
    let grant = event(
        &state,
        "e1",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.has_permission("alice", &Permission::ManageChannels));
    assert!(state.members.contains_key("alice"));

    // Revoke.
    let revoke = event(
        &state,
        "e2",
        "owner",
        EventKind::RevokePermission {
            peer_id: "alice".into(),
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &revoke), ApplyResult::Applied);
    assert!(!state.has_permission("alice", &Permission::ManageChannels));
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

// ── Permission enforcement ──────────────────────────────────────────────

#[test]
fn unpermitted_author_rejected() {
    let mut state = test_state();

    // An unpermitted peer tries to create a channel.
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
fn unpermitted_peer_can_send_messages() {
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

    // A peer with no permissions sends a message -- this should be accepted.
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

#[test]
fn permission_enforcement() {
    let mut state = test_state();

    // Grant alice only SendMessages.
    let grant = event(
        &state,
        "e0",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Alice should NOT be able to create channels (requires ManageChannels).
    let create = event(
        &state,
        "e1",
        "alice",
        EventKind::CreateChannel {
            name: "unauthorized".into(),
            channel_id: "ch1".into(),
        },
    );
    let result = apply(&mut state, &create);
    assert!(matches!(result, ApplyResult::Rejected(_)));
    assert!(!state.channels.contains_key("ch1"));
}

#[test]
fn admin_permission_grants_all() {
    let mut state = test_state();

    // Grant alice Administrator.
    let grant = event(
        &state,
        "e0",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::Administrator,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Admin can create channels.
    let create = event(
        &state,
        "e1",
        "alice",
        EventKind::CreateChannel {
            name: "admin-channel".into(),
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);
    assert!(state.channels.contains_key("ch1"));

    // Admin can create roles.
    let create_role = event(
        &state,
        "e2",
        "alice",
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
        "e3",
        "alice",
        EventKind::GrantPermission {
            peer_id: "bob".into(),
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant_bob), ApplyResult::Applied);
    assert!(state.members.contains_key("bob"));

    let kick = event(
        &state,
        "e4",
        "alice",
        EventKind::KickMember {
            peer_id: "bob".into(),
        },
    );
    assert_eq!(apply(&mut state, &kick), ApplyResult::Applied);
    assert!(!state.members.contains_key("bob"));
}

#[test]
fn owner_always_has_permission() {
    let state = test_state();

    // Owner has every permission without any explicit grants.
    assert!(state.has_permission("owner", &Permission::ManageChannels));
    assert!(state.has_permission("owner", &Permission::ManageRoles));
    assert!(state.has_permission("owner", &Permission::KickMembers));
    assert!(state.has_permission("owner", &Permission::SendMessages));
    assert!(state.has_permission("owner", &Permission::SyncProvider));
    assert!(state.has_permission("owner", &Permission::CreateInvite));
    assert!(state.has_permission("owner", &Permission::Administrator));
}

#[test]
fn fine_grained_permissions() {
    let mut state = test_state();

    // Grant alice only SendMessages.
    let grant = event(
        &state,
        "e0",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Create a channel (as owner) so alice can send messages.
    let create_ch = event(
        &state,
        "e1",
        "owner",
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Alice can send messages (messages don't require permissions).
    let msg = event(
        &state,
        "msg1",
        "alice",
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello from alice".into(),
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);

    // Alice cannot create channels.
    let create = event(
        &state,
        "e2",
        "alice",
        EventKind::CreateChannel {
            name: "unauthorized".into(),
            channel_id: "ch2".into(),
        },
    );
    assert!(matches!(
        apply(&mut state, &create),
        ApplyResult::Rejected(_)
    ));

    // Alice cannot kick members.
    let kick = event(
        &state,
        "e3",
        "alice",
        EventKind::KickMember {
            peer_id: "owner".into(),
        },
    );
    assert!(matches!(apply(&mut state, &kick), ApplyResult::Rejected(_)));

    // Alice cannot create roles.
    let create_role = event(
        &state,
        "e4",
        "alice",
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
        "e5",
        "alice",
        EventKind::GrantPermission {
            peer_id: "bob".into(),
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
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::ManageChannels,
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
    assert!(replayed.has_permission("alice", &Permission::ManageChannels));
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

    // Grant alice ManageRoles so she becomes a member.
    let grant = event(
        &state,
        "e0",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::ManageRoles,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

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

    let grant = event(
        &state,
        "e0",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::ManageRoles,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

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
fn kick_member_removes_and_revokes_permissions() {
    let mut state = test_state();

    let grant = event(
        &state,
        "e0",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.members.contains_key("alice"));
    assert!(state.has_permission("alice", &Permission::ManageChannels));

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
    assert!(!state.has_permission("alice", &Permission::ManageChannels));
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

// ── Sync provider ────────────────────────────────────────────────────────

#[test]
fn sync_provider_permission() {
    let mut state = test_state();

    // Alice is not a sync provider by default.
    assert!(!state.is_sync_provider("alice"));

    let grant = event(
        &state,
        "e0",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::SyncProvider,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.is_sync_provider("alice"));

    // Owner is always a sync provider.
    assert!(state.is_sync_provider("owner"));
}

// ── Backward compat: is_trusted ──────────────────────────────────────────

#[test]
fn is_trusted_compat() {
    let mut state = test_state();

    // Owner is always trusted.
    assert!(state.is_trusted("owner"));
    // Stranger with no permissions is not trusted.
    assert!(!state.is_trusted("stranger"));

    // Grant any permission makes a peer "trusted".
    let grant = event(
        &state,
        "e0",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.is_trusted("alice"));

    // Revoke all permissions makes peer untrusted again.
    let revoke = event(
        &state,
        "e1",
        "owner",
        EventKind::RevokePermission {
            peer_id: "alice".into(),
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &revoke), ApplyResult::Applied);
    assert!(!state.is_trusted("alice"));
}

// ── Multiple permissions per peer ────────────────────────────────────────

#[test]
fn multiple_permissions_per_peer() {
    let mut state = test_state();

    let g1 = event(
        &state,
        "e0",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &g1), ApplyResult::Applied);

    let g2 = event(
        &state,
        "e1",
        "owner",
        EventKind::GrantPermission {
            peer_id: "alice".into(),
            permission: Permission::KickMembers,
        },
    );
    assert_eq!(apply(&mut state, &g2), ApplyResult::Applied);

    assert!(state.has_permission("alice", &Permission::ManageChannels));
    assert!(state.has_permission("alice", &Permission::KickMembers));
    assert!(!state.has_permission("alice", &Permission::ManageRoles));

    // Revoke one, keep the other.
    let r1 = event(
        &state,
        "e2",
        "owner",
        EventKind::RevokePermission {
            peer_id: "alice".into(),
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &r1), ApplyResult::Applied);

    assert!(!state.has_permission("alice", &Permission::ManageChannels));
    assert!(state.has_permission("alice", &Permission::KickMembers));
    // Alice is still "trusted" because she has at least one permission.
    assert!(state.is_trusted("alice"));
}
