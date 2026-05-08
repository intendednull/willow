//! //! Permission grant / revoke / authority enforcement tests.

#![allow(unused_imports, dead_code)]

use crate::dag::EventDag;
use crate::event::{Event, EventKind, Permission, ProposedAction};
use crate::hash::EventHash;
use crate::materialize::materialize;
use willow_identity::Identity;

fn genesis_kind() -> EventKind {
    EventKind::CreateServer {
        name: "Stress Test".into(),
    }
}

fn test_dag(id: &Identity) -> EventDag {
    let mut dag = EventDag::new();
    let genesis = dag.create_event(id, genesis_kind(), vec![], 0);
    dag.insert(genesis).unwrap();
    dag
}

/// Create an event and insert it into the DAG. Returns the inserted event.
fn do_emit(dag: &mut EventDag, id: &Identity, kind: EventKind) -> Event {
    let e = dag.create_event(id, kind, vec![], 0);
    dag.insert(e.clone()).unwrap();
    e
}

#[test]
fn grant_and_check_create_invite_permission() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);
    let alice = Identity::generate();

    // Grant CreateInvite to alice.
    do_emit(
        &mut dag,
        &admin,
        EventKind::GrantPermission {
            peer_id: alice.endpoint_id(),
            permission: Permission::CreateInvite,
        },
    );

    let state = materialize(&dag);
    assert!(state.has_permission(&alice.endpoint_id(), &Permission::CreateInvite));
    assert!(state.members.contains_key(&alice.endpoint_id()));
}

#[test]
fn admin_implies_create_invite() {
    let admin = Identity::generate();
    let dag = test_dag(&admin);
    let state = materialize(&dag);
    // Admin has all permissions implicitly.
    assert!(state.has_permission(&admin.endpoint_id(), &Permission::CreateInvite));
}

#[test]
fn revoke_permission_from_peer_without_permissions() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);
    let alice = Identity::generate();

    // Revoke a permission alice never had — should be a no-op.
    do_emit(
        &mut dag,
        &admin,
        EventKind::RevokePermission {
            peer_id: alice.endpoint_id(),
            permission: Permission::CreateInvite,
        },
    );

    let state = materialize(&dag);
    assert!(!state.has_permission(&alice.endpoint_id(), &Permission::CreateInvite));
}

#[test]
fn set_permission_on_nonexistent_role_is_noop() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::SetPermission {
            role_id: "nonexistent".to_string(),
            permission: Permission::SendMessages,
            granted: true,
        },
    );

    let state = materialize(&dag);
    assert!(state.roles.is_empty());
}

#[test]
fn assign_role_to_nonmember_is_noop() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);
    let alice = Identity::generate();

    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateRole {
            name: "mod".to_string(),
            role_id: "r-1".to_string(),
        },
    );
    // Assign to alice who is not a member.
    do_emit(
        &mut dag,
        &admin,
        EventKind::AssignRole {
            peer_id: alice.endpoint_id(),
            role_id: "r-1".to_string(),
        },
    );

    let state = materialize(&dag);
    // Alice should not appear as member.
    assert!(!state.members.contains_key(&alice.endpoint_id()));
}

#[test]
fn has_permission_ignores_role_based_permissions() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);
    let alice = Identity::generate();

    // Create role with SendMessages.
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateRole {
            name: "chatter".to_string(),
            role_id: "r-1".to_string(),
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::SetPermission {
            role_id: "r-1".to_string(),
            permission: Permission::SendMessages,
            granted: true,
        },
    );
    // Grant alice SendMessages directly so she becomes a member.
    do_emit(
        &mut dag,
        &admin,
        EventKind::GrantPermission {
            peer_id: alice.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );
    // Assign role to alice.
    do_emit(
        &mut dag,
        &admin,
        EventKind::AssignRole {
            peer_id: alice.endpoint_id(),
            role_id: "r-1".to_string(),
        },
    );

    let state = materialize(&dag);
    // has_permission checks peer_permissions, not roles.
    assert!(state.has_permission(&alice.endpoint_id(), &Permission::SendMessages));
}

#[test]
fn assign_nonexistent_role_is_noop() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::AssignRole {
            peer_id: admin.endpoint_id(),
            role_id: "nonexistent".to_string(),
        },
    );

    let state = materialize(&dag);
    // Admin is always a member but should have no roles assigned.
    let member = state.members.get(&admin.endpoint_id());
    assert!(member.map(|m| m.roles.is_empty()).unwrap_or(true));
}

#[test]
fn all_permission_variants_grant_and_revoke() {
    let admin = Identity::generate();
    let peer = Identity::generate();
    let mut dag = test_dag(&admin);

    // Grant each of the 5 permission variants to peer.
    for perm in [
        Permission::SyncProvider,
        Permission::ManageChannels,
        Permission::ManageRoles,
        Permission::SendMessages,
        Permission::CreateInvite,
    ] {
        do_emit(
            &mut dag,
            &admin,
            EventKind::GrantPermission {
                peer_id: peer.endpoint_id(),
                permission: perm,
            },
        );
    }
    let state = materialize(&dag);
    assert!(state.has_permission(&peer.endpoint_id(), &Permission::SyncProvider));
    assert!(state.has_permission(&peer.endpoint_id(), &Permission::ManageChannels));
    assert!(state.has_permission(&peer.endpoint_id(), &Permission::ManageRoles));
    assert!(state.has_permission(&peer.endpoint_id(), &Permission::SendMessages));
    assert!(state.has_permission(&peer.endpoint_id(), &Permission::CreateInvite));

    // Revoke one, verify it's removed while others remain.
    do_emit(
        &mut dag,
        &admin,
        EventKind::RevokePermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::ManageChannels,
        },
    );
    let state = materialize(&dag);
    assert!(!state.has_permission(&peer.endpoint_id(), &Permission::ManageChannels));
    assert!(state.has_permission(&peer.endpoint_id(), &Permission::SyncProvider));
    assert!(state.has_permission(&peer.endpoint_id(), &Permission::SendMessages));
}

#[test]
fn kick_only_via_governance() {
    // Verify that kicking requires ProposedAction::KickMember vote path.
    // Granting all 5 permissions does NOT let a non-admin propose a kick.
    let admin = Identity::generate();
    let peer = Identity::generate();
    let mut dag = test_dag(&admin);

    // Grant all 5 permissions to peer.
    for perm in [
        Permission::SyncProvider,
        Permission::ManageChannels,
        Permission::ManageRoles,
        Permission::SendMessages,
        Permission::CreateInvite,
    ] {
        do_emit(
            &mut dag,
            &admin,
            EventKind::GrantPermission {
                peer_id: peer.endpoint_id(),
                permission: perm,
            },
        );
    }

    // Peer tries to propose a kick — should be rejected (not admin).
    let admin_head = *dag.head(&admin.endpoint_id()).unwrap();
    let e = dag.create_event(
        &peer,
        EventKind::Propose {
            action: ProposedAction::KickMember {
                peer_id: admin.endpoint_id(),
            },
        },
        vec![admin_head],
        0,
    );
    dag.insert(e).unwrap();
    let state = materialize(&dag);
    // Proposal rejected because peer is not admin.
    assert!(state.pending_proposals.is_empty());
}

#[test]
fn last_admin_cannot_self_kick() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Sole admin proposes self-kick. With majority threshold (1/1),
    // the proposer's implicit yes vote auto-applies immediately.
    do_emit(
        &mut dag,
        &admin,
        EventKind::Propose {
            action: ProposedAction::KickMember {
                peer_id: admin.endpoint_id(),
            },
        },
    );

    let state = materialize(&dag);
    // Admin must still be present — 0-admin state is unreachable.
    assert!(state.admins.contains(&admin.endpoint_id()));
    assert_eq!(state.admins.len(), 1);
    // Member should also still be present (kick was blocked).
    assert!(state.members.contains_key(&admin.endpoint_id()));
}

#[test]
fn last_admin_cannot_self_revoke() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::Propose {
            action: ProposedAction::RevokeAdmin {
                peer_id: admin.endpoint_id(),
            },
        },
    );

    let state = materialize(&dag);
    assert!(state.admins.contains(&admin.endpoint_id()));
    assert_eq!(state.admins.len(), 1);
}

#[test]
fn second_to_last_admin_can_be_kicked() {
    let admin_a = Identity::generate();
    let mut dag = test_dag(&admin_a);

    // While sole admin, set threshold to Count(1) so future proposals
    // auto-apply with a single vote.
    do_emit(
        &mut dag,
        &admin_a,
        EventKind::Propose {
            action: ProposedAction::SetVoteThreshold {
                threshold: crate::event::VoteThreshold::Count(1),
            },
        },
    );

    // Grant admin to B (auto-applies with Count(1)).
    let admin_b = Identity::generate();
    do_emit(
        &mut dag,
        &admin_a,
        EventKind::Propose {
            action: ProposedAction::GrantAdmin {
                peer_id: admin_b.endpoint_id(),
            },
        },
    );

    let state = materialize(&dag);
    assert_eq!(state.admins.len(), 2);

    // Now A proposes to kick B. With Count(1), A's implicit yes auto-applies.
    do_emit(
        &mut dag,
        &admin_a,
        EventKind::Propose {
            action: ProposedAction::KickMember {
                peer_id: admin_b.endpoint_id(),
            },
        },
    );

    let state = materialize(&dag);
    // B should be kicked, A remains as sole admin.
    assert_eq!(state.admins.len(), 1);
    assert!(state.admins.contains(&admin_a.endpoint_id()));
    assert!(!state.members.contains_key(&admin_b.endpoint_id()));
}

/// Regression guard for issue #109: an outsider (not a member, not an
/// admin, never granted ManageChannels) must not be able to inject
/// channel key material via `RotateChannelKey`. The materializer must
/// reject the event before applying the mutation.
#[test]
fn rotate_channel_key_by_outsider_is_rejected() {
    let admin = Identity::generate();
    let mallory = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-general".to_string(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
    );

    // Mallory is a brand-new identity with no relationship to the server.
    // She tries to inject her own encrypted key for the channel.
    do_emit(
        &mut dag,
        &mallory,
        EventKind::RotateChannelKey {
            channel_id: "ch-general".to_string(),
            encrypted_keys: vec![(mallory.endpoint_id(), vec![0xde, 0xad, 0xbe, 0xef])],
        },
    );

    let state = materialize(&dag);
    // Mallory's injected key must NOT appear in state.
    let mallory_key_present = state
        .channel_keys
        .get("ch-general")
        .map(|keys| keys.contains_key(&mallory.endpoint_id()))
        .unwrap_or(false);
    assert!(
        !mallory_key_present,
        "outsider must not be able to rotate channel keys"
    );
}

/// Regression guard for issue #109: a regular member without
/// ManageChannels cannot rotate channel keys either — the permission
/// check is the primary gate.
#[test]
fn rotate_channel_key_by_member_without_manage_channels_is_rejected() {
    use crate::managed::ManagedDag;
    use crate::materialize::ApplyResult;

    let alice = Identity::generate();
    let bob = Identity::generate();

    let mut managed = ManagedDag::new(&alice, "Test Server", 5000).unwrap();

    // Alice creates a channel and grants Bob SendMessages (which also
    // adds him to `members`). Bob is a legitimate member but lacks
    // ManageChannels.
    let create = managed.dag().create_event(
        &alice,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
        vec![],
        10,
    );
    managed.insert_and_apply(create).unwrap();

    let grant_send = managed.dag().create_event(
        &alice,
        EventKind::GrantPermission {
            peer_id: bob.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        20,
    );
    managed.insert_and_apply(grant_send).unwrap();

    // Bob tries to rotate the channel key. He has SendMessages but not
    // ManageChannels, so the permission check should reject.
    let rotate = managed.dag().create_event(
        &bob,
        EventKind::RotateChannelKey {
            channel_id: "ch1".into(),
            encrypted_keys: vec![(bob.endpoint_id(), vec![1, 2, 3])],
        },
        vec![],
        30,
    );
    let outcome = managed.insert_and_apply(rotate).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Rejected(_))),
        "Bob's rotate should be rejected: {:?}",
        outcome.apply_result
    );
    // Bob's key material must not have been inserted.
    let bob_key_present = managed
        .state()
        .channel_keys
        .get("ch1")
        .map(|keys| keys.contains_key(&bob.endpoint_id()))
        .unwrap_or(false);
    assert!(!bob_key_present);
}

/// Regression guard for issue #109: an admin (implicit all-permissions)
/// still can rotate channel keys after the fix. Sanity check that the
/// permission + membership additions did not break the legitimate path.
#[test]
fn rotate_channel_key_by_admin_still_works() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-general".to_string(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
    );

    do_emit(
        &mut dag,
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "ch-general".to_string(),
            encrypted_keys: vec![(admin.endpoint_id(), vec![9, 9, 9])],
        },
    );

    let state = materialize(&dag);
    assert_eq!(
        state.channel_keys["ch-general"][&admin.endpoint_id()],
        vec![9, 9, 9]
    );
}

#[test]
fn check_permission_allows_admin_propose() {
    let owner = Identity::generate();
    let dag = test_dag(&owner);
    let state = materialize(&dag);

    let kind = EventKind::Propose {
        action: ProposedAction::KickMember {
            peer_id: owner.endpoint_id(),
        },
    };
    assert!(crate::materialize::check_permission(&state, &owner.endpoint_id(), &kind).is_ok());
}

#[test]
fn check_permission_rejects_non_admin_propose() {
    let owner = Identity::generate();
    let peer = Identity::generate();
    let dag = test_dag(&owner);
    let state = materialize(&dag);

    let kind = EventKind::Propose {
        action: ProposedAction::KickMember {
            peer_id: owner.endpoint_id(),
        },
    };
    assert!(crate::materialize::check_permission(&state, &peer.endpoint_id(), &kind).is_err());
}

#[test]
fn check_permission_allows_granted_send_messages() {
    let owner = Identity::generate();
    let peer = Identity::generate();
    let mut dag = test_dag(&owner);

    // Grant SendMessages to peer.
    do_emit(
        &mut dag,
        &owner,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );
    let state = materialize(&dag);

    let kind = EventKind::Message {
        channel_id: "ch1".into(),
        body: "hello".into(),
        reply_to: None,
    };
    assert!(crate::materialize::check_permission(&state, &peer.endpoint_id(), &kind).is_ok());
}

#[test]
fn check_permission_rejects_without_send_messages() {
    let owner = Identity::generate();
    let peer = Identity::generate();
    let dag = test_dag(&owner);
    let state = materialize(&dag);

    let kind = EventKind::Message {
        channel_id: "ch1".into(),
        body: "hello".into(),
        reply_to: None,
    };
    assert!(crate::materialize::check_permission(&state, &peer.endpoint_id(), &kind).is_err());
}

// ── FileMessage permission gate (phase 3b) ──────────────────────────────

#[test]
fn check_permission_allows_file_message_with_send_messages() {
    let owner = Identity::generate();
    let peer = Identity::generate();
    let mut dag = test_dag(&owner);

    do_emit(
        &mut dag,
        &owner,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );
    let state = materialize(&dag);

    let kind = EventKind::FileMessage {
        channel_id: "ch1".into(),
        hash: "deadbeef".into(),
        filename: "photo.jpg".into(),
        mime_type: "image/jpeg".into(),
        size_bytes: 1024,
        width: Some(100),
        height: Some(100),
        body: String::new(),
        reply_to: None,
    };
    assert!(
        crate::materialize::check_permission(&state, &peer.endpoint_id(), &kind).is_ok(),
        "FileMessage must be allowed when peer has SendMessages — same gate as text Message"
    );
}

#[test]
fn check_permission_rejects_file_message_without_send_messages() {
    let owner = Identity::generate();
    let peer = Identity::generate();
    let dag = test_dag(&owner);
    let state = materialize(&dag);

    let kind = EventKind::FileMessage {
        channel_id: "ch1".into(),
        hash: "deadbeef".into(),
        filename: "photo.jpg".into(),
        mime_type: "image/jpeg".into(),
        size_bytes: 1024,
        width: None,
        height: None,
        body: String::new(),
        reply_to: None,
    };
    assert!(
        crate::materialize::check_permission(&state, &peer.endpoint_id(), &kind).is_err(),
        "FileMessage from a non-member peer must be rejected — \
         no implicit upload right just because the data is in the blob store"
    );
}

#[test]
fn check_permission_admin_implicitly_has_all() {
    let owner = Identity::generate();
    let dag = test_dag(&owner);
    let state = materialize(&dag);

    // Owner (admin) should pass all permission-gated checks without
    // explicit grants.
    for kind in [
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hi".into(),
            reply_to: None,
        },
        EventKind::CreateChannel {
            name: "dev".into(),
            channel_id: "ch2".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
        EventKind::CreateRole {
            name: "mod".into(),
            role_id: "r1".into(),
        },
    ] {
        assert!(
            crate::materialize::check_permission(&state, &owner.endpoint_id(), &kind).is_ok(),
            "admin should pass check for {:?}",
            kind
        );
    }
}

#[test]
fn check_permission_unrestricted_events_always_pass() {
    let owner = Identity::generate();
    let peer = Identity::generate();
    let dag = test_dag(&owner);
    let state = materialize(&dag);

    // Unrestricted events pass even for non-admin peers with no grants.
    for kind in [
        EventKind::SetProfile {
            display_name: "alice".into(),
        },
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: EventHash::ZERO,
        },
        EventKind::UnpinMessage {
            channel_id: "ch1".into(),
            message_id: EventHash::ZERO,
        },
    ] {
        assert!(
            crate::materialize::check_permission(&state, &peer.endpoint_id(), &kind).is_ok(),
            "unrestricted event should pass for any peer: {:?}",
            kind
        );
    }
}

#[test]
fn check_permission_rejects_non_admin_rename_server() {
    let owner = Identity::generate();
    let peer = Identity::generate();
    let dag = test_dag(&owner);
    let state = materialize(&dag);

    let kind = EventKind::RenameServer {
        new_name: "hacked".into(),
    };
    assert!(crate::materialize::check_permission(&state, &peer.endpoint_id(), &kind).is_err());
}

#[test]
fn check_permission_rejects_non_admin_set_server_description() {
    let owner = Identity::generate();
    let peer = Identity::generate();
    let dag = test_dag(&owner);
    let state = materialize(&dag);

    let kind = EventKind::SetServerDescription {
        description: "hacked".into(),
    };
    assert!(crate::materialize::check_permission(&state, &peer.endpoint_id(), &kind).is_err());
}

#[test]
fn create_and_insert_rejects_without_permission() {
    use crate::dag::InsertError;
    use crate::managed::ManagedDag;

    let owner = Identity::generate();
    let peer = Identity::generate();
    let mut managed = ManagedDag::new(&owner, "Test", 5000).unwrap();

    // Peer has no grants — should be rejected.
    let result = managed.create_and_insert(
        &peer,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
        1000,
    );
    assert!(
        matches!(result, Err(InsertError::PermissionDenied(_))),
        "expected PermissionDenied, got: {:?}",
        result
    );
}

#[test]
fn create_and_insert_does_not_advance_seq_on_rejection() {
    use crate::managed::ManagedDag;

    let owner = Identity::generate();
    let peer = Identity::generate();
    let mut managed = ManagedDag::new(&owner, "Test", 5000).unwrap();

    let seq_before = managed.dag().latest_seq(&peer.endpoint_id());

    // Rejected — should not advance sequence.
    let _ = managed.create_and_insert(
        &peer,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
        1000,
    );

    let seq_after = managed.dag().latest_seq(&peer.endpoint_id());
    assert_eq!(
        seq_before, seq_after,
        "sequence should not advance on rejection"
    );
}

#[test]
fn create_and_insert_succeeds_with_permission() {
    use crate::managed::ManagedDag;

    let owner = Identity::generate();
    let peer = Identity::generate();
    let mut managed = ManagedDag::new(&owner, "Test", 5000).unwrap();

    // Grant SendMessages to peer.
    managed
        .create_and_insert(
            &owner,
            EventKind::GrantPermission {
                peer_id: peer.endpoint_id(),
                permission: Permission::SendMessages,
            },
            1000,
        )
        .expect("admin grant should succeed");

    // Now peer can send a message.
    let result = managed.create_and_insert(
        &peer,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
        2000,
    );
    assert!(
        result.is_ok(),
        "should succeed with permission: {:?}",
        result.err()
    );
}

/// Security property: a member with `SendMessages` must NOT be able to edit
/// a message authored by a different member. Only the original author may edit
/// their own message.
///
/// This guards against a peer using a valid `EditMessage` event to silently
/// overwrite another user's words.
#[test]
fn member_cannot_edit_other_members_message() {
    let owner = Identity::generate();
    let peer_a = Identity::generate();
    let peer_b = Identity::generate();
    let mut dag = test_dag(&owner);

    // Owner creates a channel.
    do_emit(
        &mut dag,
        &owner,
        EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-1".to_string(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
    );

    // Grant SendMessages to both peers.
    do_emit(
        &mut dag,
        &owner,
        EventKind::GrantPermission {
            peer_id: peer_a.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );
    let grant_b = do_emit(
        &mut dag,
        &owner,
        EventKind::GrantPermission {
            peer_id: peer_b.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );

    // Peer B sends a message (causally after the grant so it sorts correctly).
    let b_msg = dag.create_event(
        &peer_b,
        EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "original message from B".to_string(),
            reply_to: None,
        },
        vec![grant_b.hash],
        0,
    );
    dag.insert(b_msg.clone()).unwrap();

    // Peer A tries to edit peer B's message. A has SendMessages permission
    // but is NOT the message author — this should be rejected.
    let edit_attempt = dag.create_event(
        &peer_a,
        EventKind::EditMessage {
            message_id: b_msg.hash,
            new_body: "tampered by A".to_string(),
        },
        vec![b_msg.hash],
        0,
    );
    dag.insert(edit_attempt).unwrap();

    let state = materialize(&dag);
    // The original body must be intact — edit should have been rejected.
    let msg = state
        .messages
        .iter()
        .find(|m| m.id == b_msg.hash)
        .expect("B's message should exist in state");
    assert_eq!(
        msg.body, "original message from B",
        "peer A must not be able to edit peer B's message"
    );
    assert!(
        !msg.edited,
        "edited flag must not be set after unauthorized edit"
    );
}

///// Security property: a member with `SendMessages` must NOT be able to delete
/// a message authored by a different member. Only the original author may
/// delete their own message.
#[test]
fn member_cannot_delete_other_members_message() {
    let owner = Identity::generate();
    let peer_a = Identity::generate();
    let peer_b = Identity::generate();
    let mut dag = test_dag(&owner);

    // Owner creates a channel.
    do_emit(
        &mut dag,
        &owner,
        EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-1".to_string(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
    );

    // Grant SendMessages to both peers.
    do_emit(
        &mut dag,
        &owner,
        EventKind::GrantPermission {
            peer_id: peer_a.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );
    let grant_b = do_emit(
        &mut dag,
        &owner,
        EventKind::GrantPermission {
            peer_id: peer_b.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );

    // Peer B sends a message (causally after the grant so it sorts correctly).
    let b_msg = dag.create_event(
        &peer_b,
        EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "original message from B".to_string(),
            reply_to: None,
        },
        vec![grant_b.hash],
        0,
    );
    dag.insert(b_msg.clone()).unwrap();

    // Peer A tries to delete peer B's message. A has SendMessages permission
    // but is NOT the message author — this should be rejected.
    let delete_attempt = dag.create_event(
        &peer_a,
        EventKind::DeleteMessage {
            message_id: b_msg.hash,
        },
        vec![b_msg.hash],
        0,
    );
    dag.insert(delete_attempt).unwrap();

    let state = materialize(&dag);
    // B's message must still exist and not be marked deleted.
    let msg = state
        .messages
        .iter()
        .find(|m| m.id == b_msg.hash)
        .expect("B's message should exist in state");
    assert!(
        !msg.deleted,
        "peer A must not be able to delete peer B's message"
    );
    assert_eq!(
        msg.body, "original message from B",
        "body must be unchanged after unauthorized delete"
    );
}

#[test]
fn mute_not_admin_gated() {
    // Per-identity mute is never admin-gated — any member (or even a
    // peer with no permissions at all) can mute their own view.
    let admin = Identity::generate();
    let stranger = Identity::generate();
    let mut dag = test_dag(&admin);
    do_emit(
        &mut dag,
        &stranger,
        EventKind::MuteChannel {
            channel_id: "ch-1".into(),
            muted: true,
        },
    );
    do_emit(&mut dag, &stranger, EventKind::MuteGrove { muted: true });
    let state = materialize(&dag);
    let ms = state
        .mute_state
        .get(&stranger.endpoint_id())
        .expect("stranger's mute entry must exist — no admin check");
    assert!(ms.channels.contains("ch-1"));
    assert!(ms.grove_muted);
}

/// Emit `SetPermission` with a typed `Permission` enum value, serialize
/// via the wire format (`willow-transport` = bincode), deserialize, apply
/// to a fresh DAG, and assert the role's permission set contains the
/// typed permission.
#[test]
fn set_permission_with_typed_permission_round_trips() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateRole {
            name: "mod".into(),
            role_id: "r-1".into(),
        },
    );
    let set_event = do_emit(
        &mut dag,
        &admin,
        EventKind::SetPermission {
            role_id: "r-1".into(),
            permission: Permission::ManageChannels,
            granted: true,
        },
    );

    // Wire-round-trip the event through bincode (the format used by
    // `willow-transport` and the storage layer) and re-apply.
    let bytes = bincode::serialize(&set_event).unwrap();
    let decoded: Event = bincode::deserialize(&bytes).unwrap();
    match &decoded.kind {
        EventKind::SetPermission { permission, .. } => {
            assert_eq!(*permission, Permission::ManageChannels);
        }
        other => panic!("expected SetPermission, got {other:?}"),
    }

    let state = materialize(&dag);
    let role = state.roles.get("r-1").expect("role created");
    assert!(role.permissions.contains(&Permission::ManageChannels));
}

/// Synthesize a JSON document carrying the legacy `permission: "<name>"`
/// string form (the shape MCP / agent boundary accepts) and assert the
/// custom deserializer maps it to the typed `Permission::ManageChannels`.
#[test]
fn set_permission_legacy_string_form_still_loads() {
    let json = serde_json::json!({
        "SetPermission": {
            "role_id": "r-1",
            "permission": "ManageChannels",
            "granted": true,
        }
    });
    let kind: EventKind = serde_json::from_value(json).expect("legacy string form must load");
    match kind {
        EventKind::SetPermission { permission, .. } => {
            assert_eq!(permission, Permission::ManageChannels);
        }
        other => panic!("expected SetPermission, got {other:?}"),
    }
}

/// Unknown legacy permission strings deserialize successfully (so the
/// event still enters the DAG and the chain is not broken) but apply as
/// a no-op — the unknown name is dropped.
#[test]
fn set_permission_legacy_unknown_string_drops_silently() {
    let json = serde_json::json!({
        "SetPermission": {
            "role_id": "r-1",
            "permission": "FrobnicateWidgets",
            "granted": true,
        }
    });
    let kind: EventKind =
        serde_json::from_value(json).expect("unknown legacy string must deserialize, not fail");
    match kind {
        EventKind::SetPermission { permission, .. } => {
            // Unknown name is mapped to the sentinel that apply_event drops.
            assert_eq!(permission, Permission::__UnknownLegacy);
        }
        other => panic!("expected SetPermission, got {other:?}"),
    }

    // Apply path: synthesize the post-deserialize event in memory (the
    // sentinel never crosses the wire — it only exists after a custom
    // deserialize from an unrecognised string form). Bypass `do_emit`
    // (which signs + bincodes the kind) and feed the event directly to
    // `apply_incremental`, mirroring what would happen if a JSON
    // snapshot containing the unknown name were replayed into state.
    use crate::materialize::apply_incremental;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateRole {
            name: "mod".into(),
            role_id: "r-1".into(),
        },
    );
    let mut state = materialize(&dag);

    // Fabricate an event whose kind carries the sentinel; we reuse a
    // valid hash from the genesis chain since `apply_event` does not
    // re-verify signatures and we only care about the apply branch.
    let signable = EventKind::SetPermission {
        role_id: "r-1".into(),
        permission: Permission::__UnknownLegacy,
        granted: true,
    };
    let synthetic = Event {
        hash: EventHash::from_bytes(b"synthetic-unknown-legacy"),
        author: admin.endpoint_id(),
        seq: 99,
        prev: EventHash::ZERO,
        deps: vec![],
        kind: signable,
        sig: willow_identity::Signature::from_bytes(&[0u8; 64]),
        timestamp_hint_ms: 0,
    };
    let _ = apply_incremental(&mut state, &synthetic);

    let role = state.roles.get("r-1").expect("role created");
    assert!(
        role.permissions.is_empty(),
        "unknown legacy permission must apply as a no-op"
    );
}

// ───── Membership gate for "unrestricted" event kinds ─────────────────────
//
// Issue #177 / SEC: `SetProfile`, `UpdateProfile`, `PinMessage`,
// `UnpinMessage` are documented as "any member can execute" but the
// handlers in `apply_mutation` did not actually enforce the membership
// predicate. Late-arriving events from a kicked or never-joined signer
// were silently applied, allowing post-kick state mutation.
//
// These tests pin down the membership gate at the apply boundary using
// the same pattern as `RotateChannelKey` (defense-in-depth even when
// `required_permission()` returns `None`).

/// Helper: set up a server where `peer` was a member then was kicked.
///
/// Returns a [`ManagedDag`] whose state has `peer` removed from
/// `members`, plus the admin and peer identities.
fn setup_kicked_peer() -> (
    crate::managed::ManagedDag,
    Identity,
    Identity,
    EventHash, /* kick_proposal_hash */
) {
    let admin = Identity::generate();
    let peer = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    // Promote peer to a member by granting SendMessages.
    let grant = managed.dag().create_event(
        &admin,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        10,
    );
    managed.insert_and_apply(grant).unwrap();
    assert!(managed.state().members.contains_key(&peer.endpoint_id()));

    // Admin is sole admin, so a Majority threshold (default) auto-applies
    // their own self-vote of 1/1 > 0.5.
    let kick = managed.dag().create_event(
        &admin,
        EventKind::Propose {
            action: ProposedAction::KickMember {
                peer_id: peer.endpoint_id(),
            },
        },
        vec![],
        20,
    );
    let kick_hash = kick.hash;
    managed.insert_and_apply(kick).unwrap();
    assert!(
        !managed.state().members.contains_key(&peer.endpoint_id()),
        "peer must be kicked",
    );

    (managed, admin, peer, kick_hash)
}

// SetProfile -----------------------------------------------------------------

#[test]
fn set_profile_by_member_is_applied() {
    use crate::materialize::ApplyResult;

    let admin = Identity::generate();
    let peer = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let grant = managed.dag().create_event(
        &admin,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        10,
    );
    managed.insert_and_apply(grant).unwrap();

    let set = managed.dag().create_event(
        &peer,
        EventKind::SetProfile {
            display_name: "Peer".into(),
        },
        vec![],
        20,
    );
    let outcome = managed.insert_and_apply(set).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Applied)),
        "member SetProfile must apply: {:?}",
        outcome.apply_result,
    );
    assert_eq!(
        managed
            .state()
            .profiles
            .get(&peer.endpoint_id())
            .map(|p| p.display_name.as_str()),
        Some("Peer"),
    );
}

#[test]
fn set_profile_by_kicked_member_is_rejected() {
    use crate::materialize::ApplyResult;

    let (mut managed, _admin, peer, _kick) = setup_kicked_peer();

    let set = managed.dag().create_event(
        &peer,
        EventKind::SetProfile {
            display_name: "Sneaky".into(),
        },
        vec![],
        30,
    );
    let outcome = managed.insert_and_apply(set).unwrap();
    match &outcome.apply_result {
        Some(ApplyResult::Rejected(reason)) => {
            assert!(
                reason.contains("is not a member"),
                "rejection reason must mention non-membership: {reason}",
            );
        }
        other => panic!("kicked-member SetProfile must be rejected: {other:?}"),
    }
    // State must not reflect the kicked peer's profile change.
    let dn = managed
        .state()
        .profiles
        .get(&peer.endpoint_id())
        .map(|p| p.display_name.clone())
        .unwrap_or_default();
    assert_ne!(dn, "Sneaky", "kicked peer must not mutate profile state");
}

#[test]
fn set_profile_by_non_member_is_rejected() {
    use crate::materialize::ApplyResult;

    let admin = Identity::generate();
    let stranger = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let set = managed.dag().create_event(
        &stranger,
        EventKind::SetProfile {
            display_name: "Stranger".into(),
        },
        vec![],
        10,
    );
    let outcome = managed.insert_and_apply(set).unwrap();
    match &outcome.apply_result {
        Some(ApplyResult::Rejected(reason)) => {
            assert!(
                reason.contains("is not a member"),
                "rejection reason must mention non-membership: {reason}",
            );
        }
        other => panic!("non-member SetProfile must be rejected: {other:?}"),
    }
    assert!(
        !managed
            .state()
            .profiles
            .contains_key(&stranger.endpoint_id()),
        "non-member must not create profile entry",
    );
}

// UpdateProfile --------------------------------------------------------------

#[test]
fn update_profile_by_member_is_applied() {
    use crate::materialize::ApplyResult;
    use crate::types::ProfileDelta;

    let admin = Identity::generate();
    let peer = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let grant = managed.dag().create_event(
        &admin,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        10,
    );
    managed.insert_and_apply(grant).unwrap();

    let upd = managed.dag().create_event(
        &peer,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: Some(Some("they/them".into())),
            bio: None,
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: None,
            since: None,
        })),
        vec![],
        20,
    );
    let outcome = managed.insert_and_apply(upd).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Applied)),
        "member UpdateProfile must apply: {:?}",
        outcome.apply_result,
    );
    assert_eq!(
        managed
            .state()
            .profiles
            .get(&peer.endpoint_id())
            .and_then(|p| p.pronouns.as_deref()),
        Some("they/them"),
    );
}

#[test]
fn update_profile_by_kicked_member_is_rejected() {
    use crate::materialize::ApplyResult;
    use crate::types::ProfileDelta;

    let (mut managed, _admin, peer, _kick) = setup_kicked_peer();

    let upd = managed.dag().create_event(
        &peer,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: Some(Some("rogue".into())),
            bio: None,
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: None,
            since: None,
        })),
        vec![],
        30,
    );
    let outcome = managed.insert_and_apply(upd).unwrap();
    match &outcome.apply_result {
        Some(ApplyResult::Rejected(reason)) => {
            assert!(
                reason.contains("is not a member"),
                "rejection reason must mention non-membership: {reason}",
            );
        }
        other => panic!("kicked-member UpdateProfile must be rejected: {other:?}"),
    }
    assert_ne!(
        managed
            .state()
            .profiles
            .get(&peer.endpoint_id())
            .and_then(|p| p.pronouns.as_deref()),
        Some("rogue"),
        "kicked peer must not mutate profile state",
    );
}

#[test]
fn update_profile_by_non_member_is_rejected() {
    use crate::materialize::ApplyResult;
    use crate::types::ProfileDelta;

    let admin = Identity::generate();
    let stranger = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let upd = managed.dag().create_event(
        &stranger,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: Some(Some("nope".into())),
            bio: None,
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: None,
            since: None,
        })),
        vec![],
        10,
    );
    let outcome = managed.insert_and_apply(upd).unwrap();
    match &outcome.apply_result {
        Some(ApplyResult::Rejected(reason)) => {
            assert!(
                reason.contains("is not a member"),
                "rejection reason must mention non-membership: {reason}",
            );
        }
        other => panic!("non-member UpdateProfile must be rejected: {other:?}"),
    }
    assert!(
        !managed
            .state()
            .profiles
            .contains_key(&stranger.endpoint_id()),
        "non-member must not create profile entry",
    );
}

// PinMessage / UnpinMessage --------------------------------------------------

/// Helper: build a server with one channel and one message authored by
/// `peer` (a member). Returns the message hash, useful for Pin/Unpin
/// scenarios.
fn setup_channel_with_peer_message(
    managed: &mut crate::managed::ManagedDag,
    admin: &Identity,
    peer: &Identity,
    channel_id: &str,
) -> EventHash {
    let create = managed.dag().create_event(
        admin,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: channel_id.into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
        vec![],
        5,
    );
    managed.insert_and_apply(create).unwrap();

    let msg = managed.dag().create_event(
        peer,
        EventKind::Message {
            channel_id: channel_id.into(),
            body: "hi".into(),
            reply_to: None,
        },
        vec![],
        15,
    );
    let h = msg.hash;
    managed.insert_and_apply(msg).unwrap();
    h
}

#[test]
fn pin_message_by_member_is_applied() {
    use crate::materialize::ApplyResult;

    let admin = Identity::generate();
    let peer = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let grant = managed.dag().create_event(
        &admin,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        1,
    );
    managed.insert_and_apply(grant).unwrap();

    let msg_hash = setup_channel_with_peer_message(&mut managed, &admin, &peer, "ch1");

    let pin = managed.dag().create_event(
        &peer,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: msg_hash,
        },
        vec![],
        20,
    );
    let outcome = managed.insert_and_apply(pin).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Applied)),
        "member PinMessage must apply: {:?}",
        outcome.apply_result,
    );
    assert!(managed
        .state()
        .channels
        .get("ch1")
        .unwrap()
        .pinned_messages
        .contains(&msg_hash));
}

#[test]
fn pin_message_by_kicked_member_is_rejected() {
    use crate::materialize::ApplyResult;

    // Set up: peer is member, posts a message, then is kicked. Late
    // PinMessage from peer must be rejected.
    let admin = Identity::generate();
    let peer = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let grant = managed.dag().create_event(
        &admin,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        1,
    );
    managed.insert_and_apply(grant).unwrap();

    let msg_hash = setup_channel_with_peer_message(&mut managed, &admin, &peer, "ch1");

    let kick = managed.dag().create_event(
        &admin,
        EventKind::Propose {
            action: ProposedAction::KickMember {
                peer_id: peer.endpoint_id(),
            },
        },
        vec![],
        25,
    );
    managed.insert_and_apply(kick).unwrap();
    assert!(!managed.state().members.contains_key(&peer.endpoint_id()));

    let pin = managed.dag().create_event(
        &peer,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: msg_hash,
        },
        vec![],
        30,
    );
    let outcome = managed.insert_and_apply(pin).unwrap();
    match &outcome.apply_result {
        Some(ApplyResult::Rejected(reason)) => {
            assert!(
                reason.contains("is not a member"),
                "rejection reason must mention non-membership: {reason}",
            );
        }
        other => panic!("kicked-member PinMessage must be rejected: {other:?}"),
    }
    assert!(
        !managed
            .state()
            .channels
            .get("ch1")
            .unwrap()
            .pinned_messages
            .contains(&msg_hash),
        "kicked peer must not mutate pinned_messages",
    );
}

#[test]
fn pin_message_by_non_member_is_rejected() {
    use crate::materialize::ApplyResult;

    let admin = Identity::generate();
    let stranger = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let create = managed.dag().create_event(
        &admin,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
        vec![],
        5,
    );
    managed.insert_and_apply(create).unwrap();

    // Use admin's own message as the target — stranger never joined,
    // so they have no events of their own. Authority gate should fire
    // before message-existence is even checked.
    let msg = managed.dag().create_event(
        &admin,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
        vec![],
        10,
    );
    let msg_hash = msg.hash;
    managed.insert_and_apply(msg).unwrap();

    let pin = managed.dag().create_event(
        &stranger,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: msg_hash,
        },
        vec![],
        20,
    );
    let outcome = managed.insert_and_apply(pin).unwrap();
    match &outcome.apply_result {
        Some(ApplyResult::Rejected(reason)) => {
            assert!(
                reason.contains("is not a member"),
                "rejection reason must mention non-membership: {reason}",
            );
        }
        other => panic!("non-member PinMessage must be rejected: {other:?}"),
    }
    assert!(
        !managed
            .state()
            .channels
            .get("ch1")
            .unwrap()
            .pinned_messages
            .contains(&msg_hash),
        "non-member must not mutate pinned_messages",
    );
}

#[test]
fn unpin_message_by_member_is_applied() {
    use crate::materialize::ApplyResult;

    let admin = Identity::generate();
    let peer = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let grant = managed.dag().create_event(
        &admin,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        1,
    );
    managed.insert_and_apply(grant).unwrap();

    let msg_hash = setup_channel_with_peer_message(&mut managed, &admin, &peer, "ch1");

    // Admin pins, then peer unpins.
    let pin = managed.dag().create_event(
        &admin,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: msg_hash,
        },
        vec![],
        20,
    );
    managed.insert_and_apply(pin).unwrap();

    let unpin = managed.dag().create_event(
        &peer,
        EventKind::UnpinMessage {
            channel_id: "ch1".into(),
            message_id: msg_hash,
        },
        vec![],
        30,
    );
    let outcome = managed.insert_and_apply(unpin).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Applied)),
        "member UnpinMessage must apply: {:?}",
        outcome.apply_result,
    );
    assert!(!managed
        .state()
        .channels
        .get("ch1")
        .unwrap()
        .pinned_messages
        .contains(&msg_hash));
}

#[test]
fn unpin_message_by_kicked_member_is_rejected() {
    use crate::materialize::ApplyResult;

    let admin = Identity::generate();
    let peer = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let grant = managed.dag().create_event(
        &admin,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        1,
    );
    managed.insert_and_apply(grant).unwrap();

    let msg_hash = setup_channel_with_peer_message(&mut managed, &admin, &peer, "ch1");

    // Admin pins.
    let pin = managed.dag().create_event(
        &admin,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: msg_hash,
        },
        vec![],
        20,
    );
    managed.insert_and_apply(pin).unwrap();
    assert!(managed
        .state()
        .channels
        .get("ch1")
        .unwrap()
        .pinned_messages
        .contains(&msg_hash));

    // Kick peer.
    let kick = managed.dag().create_event(
        &admin,
        EventKind::Propose {
            action: ProposedAction::KickMember {
                peer_id: peer.endpoint_id(),
            },
        },
        vec![],
        25,
    );
    managed.insert_and_apply(kick).unwrap();
    assert!(!managed.state().members.contains_key(&peer.endpoint_id()));

    // Late UnpinMessage from kicked peer must be rejected.
    let unpin = managed.dag().create_event(
        &peer,
        EventKind::UnpinMessage {
            channel_id: "ch1".into(),
            message_id: msg_hash,
        },
        vec![],
        30,
    );
    let outcome = managed.insert_and_apply(unpin).unwrap();
    match &outcome.apply_result {
        Some(ApplyResult::Rejected(reason)) => {
            assert!(
                reason.contains("is not a member"),
                "rejection reason must mention non-membership: {reason}",
            );
        }
        other => panic!("kicked-member UnpinMessage must be rejected: {other:?}"),
    }
    assert!(
        managed
            .state()
            .channels
            .get("ch1")
            .unwrap()
            .pinned_messages
            .contains(&msg_hash),
        "kicked peer must not unpin",
    );
}

#[test]
fn unpin_message_by_non_member_is_rejected() {
    use crate::materialize::ApplyResult;

    let admin = Identity::generate();
    let stranger = Identity::generate();
    let mut managed = crate::managed::ManagedDag::new(&admin, "S", 5000).unwrap();

    let create = managed.dag().create_event(
        &admin,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
        vec![],
        5,
    );
    managed.insert_and_apply(create).unwrap();

    let msg = managed.dag().create_event(
        &admin,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
        vec![],
        10,
    );
    let msg_hash = msg.hash;
    managed.insert_and_apply(msg).unwrap();

    // Admin pins.
    let pin = managed.dag().create_event(
        &admin,
        EventKind::PinMessage {
            channel_id: "ch1".into(),
            message_id: msg_hash,
        },
        vec![],
        15,
    );
    managed.insert_and_apply(pin).unwrap();

    let unpin = managed.dag().create_event(
        &stranger,
        EventKind::UnpinMessage {
            channel_id: "ch1".into(),
            message_id: msg_hash,
        },
        vec![],
        20,
    );
    let outcome = managed.insert_and_apply(unpin).unwrap();
    match &outcome.apply_result {
        Some(ApplyResult::Rejected(reason)) => {
            assert!(
                reason.contains("is not a member"),
                "rejection reason must mention non-membership: {reason}",
            );
        }
        other => panic!("non-member UnpinMessage must be rejected: {other:?}"),
    }
    assert!(
        managed
            .state()
            .channels
            .get("ch1")
            .unwrap()
            .pinned_messages
            .contains(&msg_hash),
        "non-member must not unpin",
    );
}
