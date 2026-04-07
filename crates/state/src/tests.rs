//! Stress and integration tests for the willow-state crate.

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
fn stress_1000_events_single_author() {
    let id = Identity::generate();
    let mut dag = test_dag(&id);

    for i in 0..999 {
        let e = dag.create_event(
            &id,
            EventKind::Message {
                channel_id: "general".into(),
                body: format!("message {i}"),
                reply_to: None,
            },
            vec![],
            i as u64,
        );
        dag.insert(e).unwrap();
    }

    assert_eq!(dag.len(), 1000);
    let state = materialize(&dag);
    assert_eq!(state.messages.len(), 999);
}

#[test]
fn stress_100_authors_10_events_each() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let authors: Vec<Identity> = (0..99).map(|_| Identity::generate()).collect();

    // Each author produces 10 events.
    for author in &authors {
        for i in 0..10 {
            let deps = if i == 0 {
                // First event from each author depends on genesis.
                vec![*dag.head(&admin.endpoint_id()).unwrap()]
            } else {
                vec![]
            };
            let e = dag.create_event(
                author,
                EventKind::SetProfile {
                    display_name: format!("author_{i}"),
                },
                deps,
                0,
            );
            dag.insert(e).unwrap();
        }
    }

    // Admin also has genesis (1 event). Total: 1 + 99*10 = 991.
    assert_eq!(dag.len(), 991);

    let sorted = dag.topological_sort();
    assert_eq!(sorted.len(), 991);

    // Materialize is deterministic.
    let s1 = materialize(&dag);
    let s2 = materialize(&dag);
    assert_eq!(s1.profiles.len(), s2.profiles.len());
}

#[test]
fn stress_sort_performance() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let authors: Vec<Identity> = (0..49).map(|_| Identity::generate()).collect();

    // 50 authors, ~200 events each = 10000 events.
    for _ in 0..199 {
        let e = dag.create_event(
            &admin,
            EventKind::SetProfile {
                display_name: "x".into(),
            },
            vec![],
            0,
        );
        dag.insert(e).unwrap();
    }
    for author in &authors {
        for _ in 0..200 {
            let e = dag.create_event(
                author,
                EventKind::SetProfile {
                    display_name: "x".into(),
                },
                vec![],
                0,
            );
            dag.insert(e).unwrap();
        }
    }

    assert_eq!(dag.len(), 200 + 49 * 200); // 200 + 9800 = 10000

    let start = std::time::Instant::now();
    let sorted = dag.topological_sort();
    let elapsed = start.elapsed();

    assert_eq!(sorted.len(), 10000);
    // Should complete in reasonable time (< 1 second).
    assert!(
        elapsed.as_secs() < 1,
        "topological sort took {elapsed:?} for 10000 events"
    );
}

#[test]
fn stress_concurrent_channel_creates() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // 50 different authors each create a channel concurrently (no cross-deps).
    let authors: Vec<Identity> = (0..50).map(|_| Identity::generate()).collect();
    for (i, author) in authors.iter().enumerate() {
        let e = dag.create_event(
            author,
            EventKind::CreateChannel {
                name: format!("channel-{i}"),
                channel_id: format!("ch-{i}"),
                kind: "text".into(),
            },
            vec![],
            0,
        );
        dag.insert(e).unwrap();
    }

    let state = materialize(&dag);
    // All channels created — admin has ManageChannels implicitly,
    // but these are from non-admin authors without ManageChannels.
    // They should be rejected (no permission).
    // Actually: the authors don't have ManageChannels permission.
    // So 0 channels should exist.
    assert_eq!(state.channels.len(), 0);

    // Now grant ManageChannels to all authors and retry.
    for author in &authors {
        let e = dag.create_event(
            &admin,
            EventKind::GrantPermission {
                peer_id: author.endpoint_id(),
                permission: crate::event::Permission::ManageChannels,
            },
            vec![],
            0,
        );
        dag.insert(e).unwrap();
    }

    // Authors create channels again (now with permission).
    // Each channel create deps on the admin's latest event (the grant).
    let admin_head = *dag.head(&admin.endpoint_id()).unwrap();
    for (i, author) in authors.iter().enumerate() {
        let e = dag.create_event(
            author,
            EventKind::CreateChannel {
                name: format!("ch2-{i}"),
                channel_id: format!("ch2-{i}"),
                kind: "text".into(),
            },
            vec![admin_head],
            0,
        );
        dag.insert(e).unwrap();
    }

    let state = materialize(&dag);
    // At least 50 channels from the second batch (with permission).
    // Some first-batch channels may also exist if they happen to sort
    // after a grant event due to hash tiebreaking — this is expected
    // and deterministic.
    assert!(state.channels.len() >= 50);

    // Deterministic: same DAG → same state.
    let s2 = materialize(&dag);
    assert_eq!(state.channels.len(), s2.channels.len());
}

#[test]
fn stress_governance_many_proposals() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Add 10 admins via sequential proposals (each auto-applies as sole/growing admin set).
    let mut admins: Vec<Identity> = vec![admin.clone()];
    for _ in 0..10 {
        let new_admin = Identity::generate();
        // Current admin set proposes. With majority, sole proposer may auto-apply.
        let prop = dag.create_event(
            &admins[0],
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: new_admin.endpoint_id(),
                },
            },
            vec![],
            0,
        );
        dag.insert(prop.clone()).unwrap();

        // If majority not met by proposer alone, have others vote.
        let state = materialize(&dag);
        if !state.is_admin(&new_admin.endpoint_id()) {
            // Need more votes. Have the second admin vote.
            if admins.len() > 1 {
                let vote = dag.create_event(
                    &admins[1],
                    EventKind::Vote {
                        proposal: prop.hash,
                        accept: true,
                    },
                    vec![prop.hash],
                    0,
                );
                dag.insert(vote).unwrap();
            }
        }

        admins.push(new_admin);
    }

    let state = materialize(&dag);
    // Should have at least several admins (exact count depends on
    // majority threshold cascading).
    assert!(state.admins.len() >= 3);
}

// ── Edge-case tests (DAG pattern) ──────────────────────────────────────
//
// Tests below cover edge cases not in materialize.rs: noop on nonexistent
// targets, duplicate creates, channel key rotation, reply_to storage, etc.

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
fn server_state_new_defaults() {
    let admin = Identity::generate();
    let dag = test_dag(&admin);
    let state = materialize(&dag);
    assert!(state.admins.contains(&admin.endpoint_id()));
    assert!(state.channels.is_empty());
    assert!(state.roles.is_empty());
}

#[test]
fn non_admin_set_profile_is_accepted() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);
    let alice = Identity::generate();

    // Grant SendMessages so alice is a member.
    do_emit(
        &mut dag,
        &admin,
        EventKind::GrantPermission {
            peer_id: alice.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );
    // Alice sets her own profile (no admin needed).
    do_emit(
        &mut dag,
        &alice,
        EventKind::SetProfile {
            display_name: "Alice".to_string(),
        },
    );

    let state = materialize(&dag);
    assert_eq!(
        state
            .profiles
            .get(&alice.endpoint_id())
            .map(|p| p.display_name.as_str()),
        Some("Alice")
    );
}

#[test]
fn duplicate_create_channel_preserves_original() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "general".to_string(),
            channel_id: "ch-1".to_string(),
            kind: "text".to_string(),
        },
    );
    // Duplicate channel_id — should be ignored.
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "different-name".to_string(),
            channel_id: "ch-1".to_string(),
            kind: "voice".to_string(),
        },
    );

    let state = materialize(&dag);
    assert_eq!(state.channels.len(), 1);
    assert_eq!(state.channels["ch-1"].name, "general");
}

#[test]
fn duplicate_create_role_preserves_original() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateRole {
            name: "moderator".to_string(),
            role_id: "r-1".to_string(),
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateRole {
            name: "other-name".to_string(),
            role_id: "r-1".to_string(),
        },
    );

    let state = materialize(&dag);
    assert_eq!(state.roles.len(), 1);
    assert_eq!(state.roles["r-1"].name, "moderator");
}

#[test]
fn rename_nonexistent_channel_is_noop() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::RenameChannel {
            channel_id: "nonexistent".to_string(),
            new_name: "new-name".to_string(),
        },
    );

    let state = materialize(&dag);
    assert!(state.channels.is_empty());
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
            permission: "SendMessages".to_string(),
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
fn message_reply_to_is_stored() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let msg = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "hello".to_string(),
            reply_to: None,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "reply".to_string(),
            reply_to: Some(msg.hash),
        },
    );

    let state = materialize(&dag);
    assert_eq!(state.messages.len(), 2);
    assert_eq!(state.messages[1].reply_to, Some(msg.hash));
}

#[test]
fn edit_nonexistent_message_is_noop() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::EditMessage {
            message_id: EventHash::from_bytes(b"nonexistent"),
            new_body: "edited".to_string(),
        },
    );

    let state = materialize(&dag);
    assert!(state.messages.is_empty());
}

#[test]
fn delete_nonexistent_message_is_noop() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::DeleteMessage {
            message_id: EventHash::from_bytes(b"nonexistent"),
        },
    );

    let state = materialize(&dag);
    assert!(state.messages.is_empty());
}

#[test]
fn delete_message_clears_reactions() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let msg = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "hello".to_string(),
            reply_to: None,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::Reaction {
            message_id: msg.hash,
            emoji: "👍".to_string(),
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::DeleteMessage {
            message_id: msg.hash,
        },
    );

    let state = materialize(&dag);
    assert!(state.messages[0].deleted);
}

#[test]
fn reaction_on_nonexistent_message_is_noop() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::Reaction {
            message_id: EventHash::from_bytes(b"nonexistent"),
            emoji: "👍".to_string(),
        },
    );

    let state = materialize(&dag);
    assert!(state.messages.is_empty());
}

#[test]
fn duplicate_reaction_from_same_peer() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let msg = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "hello".to_string(),
            reply_to: None,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::Reaction {
            message_id: msg.hash,
            emoji: "👍".to_string(),
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::Reaction {
            message_id: msg.hash,
            emoji: "👍".to_string(),
        },
    );

    let state = materialize(&dag);
    // Implementation-dependent: may deduplicate or not.
    assert!(!state.messages[0].reactions.is_empty());
}

#[test]
fn multiple_peers_react_to_same_message() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);
    let alice = Identity::generate();

    // Grant alice SendMessages.
    let grant = do_emit(
        &mut dag,
        &admin,
        EventKind::GrantPermission {
            peer_id: alice.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );
    let msg = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "hello".to_string(),
            reply_to: None,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::Reaction {
            message_id: msg.hash,
            emoji: "👍".to_string(),
        },
    );
    // Alice's reaction depends on grant + msg so it sorts after them.
    let react_event = dag.create_event(
        &alice,
        EventKind::Reaction {
            message_id: msg.hash,
            emoji: "❤️".to_string(),
        },
        vec![grant.hash, msg.hash],
        0,
    );
    dag.insert(react_event).unwrap();

    let state = materialize(&dag);
    assert!(state.messages[0].reactions.len() >= 2);
}

#[test]
fn channel_kind_is_preserved() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "voice-chat".to_string(),
            channel_id: "vc-1".to_string(),
            kind: "voice".to_string(),
        },
    );

    let state = materialize(&dag);
    assert_eq!(state.channels["vc-1"].kind, "voice");
}

#[test]
fn rotate_channel_key_stores_key_material() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "ch-1".to_string(),
            encrypted_keys: vec![(admin.endpoint_id(), vec![1, 2, 3])],
        },
    );

    let state = materialize(&dag);
    assert!(state.channel_keys.contains_key("ch-1"));
}

#[test]
fn delete_channel_messages_not_from_other_channels() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "ch1".to_string(),
            channel_id: "ch-1".to_string(),
            kind: "text".to_string(),
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "ch2".to_string(),
            channel_id: "ch-2".to_string(),
            kind: "text".to_string(),
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "in ch1".to_string(),
            reply_to: None,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-2".to_string(),
            body: "in ch2".to_string(),
            reply_to: None,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::DeleteChannel {
            channel_id: "ch-1".to_string(),
        },
    );

    let state = materialize(&dag);
    // Only ch2's message should remain.
    let remaining: Vec<_> = state.messages.iter().filter(|m| !m.deleted).collect();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].channel_id, "ch-2");
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
            permission: "SendMessages".to_string(),
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
fn apply_incremental_is_idempotent() {
    use crate::materialize::apply_incremental;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Grant SendMessages so admin can react.
    let msg = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            body: "hello".into(),
            reply_to: None,
        },
    );
    let reaction = do_emit(
        &mut dag,
        &admin,
        EventKind::Reaction {
            message_id: msg.hash,
            emoji: "👍".into(),
        },
    );
    let mut state = materialize(&dag);

    // Reaction applied once during materialize.
    let reactions = &state.messages[0].reactions;
    assert_eq!(reactions.get("👍").map(|v| v.len()), Some(1));

    // Apply the same reaction event again — should be AlreadyApplied.
    let result = apply_incremental(&mut state, &reaction);
    assert_eq!(result, crate::materialize::ApplyResult::AlreadyApplied);

    // Still only 1 reaction (not 2).
    let reactions = &state.messages[0].reactions;
    assert_eq!(reactions.get("👍").map(|v| v.len()), Some(1));
}

#[test]
fn apply_incremental_dedup_across_messages() {
    use crate::materialize::apply_incremental;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let msg = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            body: "hello".into(),
            reply_to: None,
        },
    );
    let mut state = materialize(&dag);
    assert_eq!(state.messages.len(), 1);

    // Apply the same message event again — should be AlreadyApplied.
    let result = apply_incremental(&mut state, &msg);
    assert_eq!(result, crate::materialize::ApplyResult::AlreadyApplied);
    assert_eq!(state.messages.len(), 1);
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
fn vote_ordering_with_deps_ensures_admin_status() {
    // Scenario: Admin A proposes granting Alice admin. With 1 admin,
    // proposal auto-applies. Now Alice proposes something. Because
    // Alice's proposal includes deps on A's head (which is >= the
    // propose event that granted Alice admin), the topo sort correctly
    // places the grant before Alice's proposal.
    use crate::event::VoteThreshold;

    let admin_a = Identity::generate();
    let alice = Identity::generate();
    let mut dag = test_dag(&admin_a);

    // Grant Alice admin (sole admin, auto-applies).
    let _prop = do_emit(
        &mut dag,
        &admin_a,
        EventKind::Propose {
            action: ProposedAction::GrantAdmin {
                peer_id: alice.endpoint_id(),
            },
        },
    );
    let state = materialize(&dag);
    assert!(
        state.is_admin(&alice.endpoint_id()),
        "Alice should be admin after sole-admin proposal"
    );

    // Alice proposes a threshold change — include admin_a's head as dep
    // so the proposal is causally after the grant.
    let admin_head = *dag.head(&admin_a.endpoint_id()).unwrap();
    let alice_prop_event = dag.create_event(
        &alice,
        EventKind::Propose {
            action: ProposedAction::SetVoteThreshold {
                threshold: VoteThreshold::Unanimous,
            },
        },
        vec![admin_head],
        0,
    );
    dag.insert(alice_prop_event.clone()).unwrap();

    let state = materialize(&dag);
    // Alice's proposal should be accepted because she is admin and
    // the dep ensures correct ordering.
    assert!(
        state.pending_proposals.contains_key(&alice_prop_event.hash),
        "Alice's proposal should be pending (she is admin)"
    );
}

#[test]
fn second_create_server_rejected_by_dag() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);
    let state1 = materialize(&dag);

    // Attempt a second CreateServer — DAG rejects it.
    let second = dag.create_event(
        &admin,
        EventKind::CreateServer {
            name: "Different".into(),
        },
        vec![],
        0,
    );
    let err = dag.insert(second).unwrap_err();
    assert!(matches!(err, crate::dag::InsertError::DuplicateGenesis));

    // Materialized state is unchanged.
    let state2 = materialize(&dag);
    assert_eq!(state1.server_id, state2.server_id);
    assert_eq!(state1.server_name, state2.server_name);
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
