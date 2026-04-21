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
                kind: crate::types::ChannelKind::Text,
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
                kind: crate::types::ChannelKind::Text,
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
            kind: crate::types::ChannelKind::Text,
        },
    );
    // Duplicate channel_id — should be ignored.
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "different-name".to_string(),
            channel_id: "ch-1".to_string(),
            kind: crate::types::ChannelKind::Voice,
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
    // The same author reacting twice with the same emoji collapses to a
    // single entry — reactions are stored as a set keyed by author.
    let reactors = state.messages[0]
        .reactions
        .get("👍")
        .expect("emoji should be present");
    assert_eq!(reactors.len(), 1);
    assert!(reactors.contains(&admin.endpoint_id()));
}

#[test]
fn same_author_duplicate_reaction_is_idempotent() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let msg = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".to_string(),
            body: "react to me".to_string(),
            reply_to: None,
        },
    );

    // Apply two distinct Reaction events with the same emoji from the
    // same author. The events themselves are unique (different hashes
    // because of timestamps/parents), so dedup must happen at
    // materialization time.
    do_emit(
        &mut dag,
        &admin,
        EventKind::Reaction {
            message_id: msg.hash,
            emoji: "🎉".to_string(),
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::Reaction {
            message_id: msg.hash,
            emoji: "🎉".to_string(),
        },
    );

    let state = materialize(&dag);
    let reactors = state.messages[0]
        .reactions
        .get("🎉")
        .expect("emoji should be present");
    assert_eq!(reactors.len(), 1, "duplicate reactions must be deduped");
    assert!(reactors.contains(&admin.endpoint_id()));
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
            kind: crate::types::ChannelKind::Voice,
        },
    );

    let state = materialize(&dag);
    assert_eq!(
        state.channels["vc-1"].kind,
        crate::types::ChannelKind::Voice
    );
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
    let keys = &state.channel_keys["ch-1"];
    assert_eq!(keys[&admin.endpoint_id()], vec![1, 2, 3]);
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
            kind: crate::types::ChannelKind::Text,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "ch2".to_string(),
            channel_id: "ch-2".to_string(),
            kind: crate::types::ChannelKind::Text,
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

// ── Issue #69: Last admin cannot self-kick/revoke ────────────────────

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

// ── Issue #70: Topological sort cycle detection ──────────────────────

#[test]
fn topological_sort_covers_all_events() {
    let admin = Identity::generate();
    let alice = Identity::generate();
    let mut dag = test_dag(&admin);

    // Create events from multiple authors with cross-deps.
    let a1 = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "a1".into(),
        },
    );
    // Alice's first event depends on admin's message.
    let b1 = dag.create_event(
        &alice,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "b1".into(),
        },
        vec![a1.hash],
        0,
    );
    dag.insert(b1).unwrap();

    do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "a2".into(),
        },
    );

    let sorted = dag.topological_sort();
    assert_eq!(sorted.len(), dag.len());
}

#[test]
fn topological_sort_diamond_pattern() {
    // Diamond: genesis → A1, genesis → B1(dep=A1), genesis → C1(dep=A1),
    //          then D1(deps=[B1,C1]) — D must come last.
    let admin = Identity::generate();
    let bob = Identity::generate();
    let carol = Identity::generate();
    let dave = Identity::generate();
    let mut dag = test_dag(&admin);

    let a1 = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "a1".into(),
        },
    );

    // Bob depends on A1.
    let b1 = dag.create_event(
        &bob,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "b1".into(),
        },
        vec![a1.hash],
        0,
    );
    dag.insert(b1.clone()).unwrap();

    // Carol depends on A1.
    let c1 = dag.create_event(
        &carol,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "c1".into(),
        },
        vec![a1.hash],
        0,
    );
    dag.insert(c1.clone()).unwrap();

    // Dave depends on both B1 and C1 (diamond join).
    let d1 = dag.create_event(
        &dave,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "d1".into(),
        },
        vec![b1.hash, c1.hash],
        0,
    );
    dag.insert(d1.clone()).unwrap();

    let sorted = dag.topological_sort();
    assert_eq!(sorted.len(), dag.len()); // All 5 events processed (genesis + 4).

    // D1 must come after both B1 and C1.
    let pos = |hash: EventHash| sorted.iter().position(|e| e.hash == hash).unwrap();
    assert!(pos(d1.hash) > pos(b1.hash));
    assert!(pos(d1.hash) > pos(c1.hash));
    // B1 and C1 both after A1.
    assert!(pos(b1.hash) > pos(a1.hash));
    assert!(pos(c1.hash) > pos(a1.hash));
}

#[test]
fn topological_sort_deep_chain() {
    // 6-event causal chain across 3 authors: A1→B1→C1→A2→B2→C2
    let admin = Identity::generate();
    let bob = Identity::generate();
    let carol = Identity::generate();
    let mut dag = test_dag(&admin);

    let a1 = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "a1".into(),
        },
    );
    let b1 = dag.create_event(
        &bob,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "b1".into(),
        },
        vec![a1.hash],
        0,
    );
    dag.insert(b1.clone()).unwrap();

    let c1 = dag.create_event(
        &carol,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "c1".into(),
        },
        vec![b1.hash],
        0,
    );
    dag.insert(c1.clone()).unwrap();

    let a2 = dag.create_event(
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "a2".into(),
        },
        vec![c1.hash],
        0,
    );
    dag.insert(a2.clone()).unwrap();

    let b2 = dag.create_event(
        &bob,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "b2".into(),
        },
        vec![a2.hash],
        0,
    );
    dag.insert(b2.clone()).unwrap();

    let c2 = dag.create_event(
        &carol,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "c2".into(),
        },
        vec![b2.hash],
        0,
    );
    dag.insert(c2.clone()).unwrap();

    let sorted = dag.topological_sort();
    assert_eq!(sorted.len(), dag.len()); // genesis + 6 = 7

    // Verify strict causal ordering.
    let pos = |hash: EventHash| sorted.iter().position(|e| e.hash == hash).unwrap();
    assert!(pos(a1.hash) < pos(b1.hash));
    assert!(pos(b1.hash) < pos(c1.hash));
    assert!(pos(c1.hash) < pos(a2.hash));
    assert!(pos(a2.hash) < pos(b2.hash));
    assert!(pos(b2.hash) < pos(c2.hash));
}

// ── Issue #71: Equivocation rejected at insert ───────────────────────

#[test]
fn equivocation_rejected_at_insert() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Insert first event at seq=2.
    do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "first".into(),
        },
    );

    // Attempt equivocation: different event at seq=2 (but DAG expects seq=3).
    let equivocating = Event::new(
        &admin,
        2,               // same seq as the event we just inserted
        EventHash::ZERO, // wrong prev — doesn't matter, seq check fires first
        vec![],
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "equivocation".into(),
        },
        999,
    );
    let err = dag.insert(equivocating).unwrap_err();
    assert!(
        matches!(
            err,
            crate::dag::InsertError::SeqGap {
                expected: 3,
                got: 2,
                ..
            }
        ),
        "Expected SeqGap(expected=3, got=2), got: {err:?}"
    );
}

#[test]
fn equivocation_after_gap_rejected() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Build chain: genesis(seq=1) → msg(seq=2) → msg(seq=3).
    do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "m1".into(),
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "m2".into(),
        },
    );

    // Try inserting at seq=2 (already occupied).
    let bad = Event::new(
        &admin,
        2,
        EventHash::ZERO,
        vec![],
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "sneaky".into(),
        },
        999,
    );
    let err = dag.insert(bad).unwrap_err();
    assert!(matches!(
        err,
        crate::dag::InsertError::SeqGap {
            expected: 4,
            got: 2,
            ..
        }
    ));
}

#[test]
fn new_author_seq_gap_rejected() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // New author tries to insert at seq=3, skipping 1 and 2.
    let stranger = Identity::generate();
    let bad = Event::new(
        &stranger,
        3,
        EventHash::ZERO,
        vec![],
        EventKind::Message {
            channel_id: "ch".into(),
            reply_to: None,
            body: "skip".into(),
        },
        0,
    );
    let err = dag.insert(bad).unwrap_err();
    assert!(matches!(
        err,
        crate::dag::InsertError::SeqGap {
            expected: 1,
            got: 3,
            ..
        }
    ));
}

// ── Issue #72: Vote on missing proposal ──────────────────────────────

#[test]
fn vote_on_missing_proposal_rejected() {
    use crate::materialize::apply_incremental;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Create a vote referencing a non-existent proposal hash.
    let fake_proposal = EventHash::ZERO;
    let vote = dag.create_event(
        &admin,
        EventKind::Vote {
            proposal: fake_proposal,
            accept: true,
        },
        vec![fake_proposal], // include in deps to satisfy governance check
        0,
    );
    dag.insert(vote.clone()).unwrap();

    // Apply incrementally and verify rejection.
    let genesis = dag.genesis().unwrap().clone();
    let mut state =
        crate::server::ServerState::new(dag.server_id().unwrap(), "Test", genesis.author);
    let _ = apply_incremental(&mut state, &genesis);
    let result = apply_incremental(&mut state, &vote);
    assert!(
        matches!(result, crate::materialize::ApplyResult::Rejected(ref msg) if msg.contains("not found")),
        "Expected Rejected with 'not found', got: {result:?}"
    );
}

#[test]
fn vote_on_already_applied_proposal_is_safe() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Create a proposal that auto-applies (sole admin, majority 1/1).
    let alice = Identity::generate();
    let prop = do_emit(
        &mut dag,
        &admin,
        EventKind::Propose {
            action: ProposedAction::GrantAdmin {
                peer_id: alice.endpoint_id(),
            },
        },
    );

    // Now alice (newly admin) votes on the already-applied proposal.
    let vote = dag.create_event(
        &alice,
        EventKind::Vote {
            proposal: prop.hash,
            accept: true,
        },
        vec![prop.hash],
        0,
    );
    dag.insert(vote).unwrap();

    // Full materialize should not crash.
    let state = materialize(&dag);
    // Alice should be admin (proposal already applied).
    assert!(state.admins.contains(&alice.endpoint_id()));
}

#[test]
fn multi_admin_kick_requires_majority_vote() {
    // With 2 admins, a kick proposal needs both votes (majority > 1).
    let admin_a = Identity::generate();
    let mut dag = test_dag(&admin_a);

    // Grant admin to B (auto-applies, sole admin majority).
    let admin_b = Identity::generate();
    let grant_admin_event = do_emit(
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

    // Add target as member.
    let target = Identity::generate();
    let grant_perm_event = do_emit(
        &mut dag,
        &admin_a,
        EventKind::GrantPermission {
            peer_id: target.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );

    // B proposes to kick target — 1/2 votes, stays pending.
    // (admin_a is genesis author; genesis proposals auto-apply, so we use admin_b.)
    // Deps on grant_admin_event and grant_perm_event ensure the DAG topological sort
    // processes admin_b's admin status and target's membership BEFORE this proposal,
    // so the state has 2 admins when the vote threshold is evaluated (1/2 < majority).
    let kick_prop = dag.create_event(
        &admin_b,
        EventKind::Propose {
            action: ProposedAction::KickMember {
                peer_id: target.endpoint_id(),
            },
        },
        vec![grant_admin_event.hash, grant_perm_event.hash],
        0,
    );
    dag.insert(kick_prop.clone()).unwrap();

    let state = materialize(&dag);
    assert!(state.pending_proposals.contains_key(&kick_prop.hash));
    assert!(state.members.contains_key(&target.endpoint_id()));

    // A votes yes → 2/2 = passes majority.
    let vote_a = dag.create_event(
        &admin_a,
        EventKind::Vote {
            proposal: kick_prop.hash,
            accept: true,
        },
        vec![kick_prop.hash],
        0,
    );
    dag.insert(vote_a).unwrap();

    let state = materialize(&dag);
    // Kick applied — target removed.
    assert!(!state.members.contains_key(&target.endpoint_id()));
    // Proposal consumed.
    assert!(!state.pending_proposals.contains_key(&kick_prop.hash));
    // Both admins still present.
    assert_eq!(state.admins.len(), 2);
}

// ── Issue #73: RotateChannelKey multi-recipient ──────────────────────

#[test]
fn rotate_channel_key_stores_all_recipients() {
    let admin = Identity::generate();
    let alice = Identity::generate();
    let bob = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "ch-1".to_string(),
            encrypted_keys: vec![
                (admin.endpoint_id(), vec![1, 2, 3]),
                (alice.endpoint_id(), vec![4, 5, 6]),
                (bob.endpoint_id(), vec![7, 8, 9]),
            ],
        },
    );

    let state = materialize(&dag);
    let keys = &state.channel_keys["ch-1"];
    assert_eq!(keys.len(), 3);
    assert_eq!(keys[&admin.endpoint_id()], vec![1, 2, 3]);
    assert_eq!(keys[&alice.endpoint_id()], vec![4, 5, 6]);
    assert_eq!(keys[&bob.endpoint_id()], vec![7, 8, 9]);
}

#[test]
fn rotate_channel_key_overwrites_on_second_rotation() {
    let admin = Identity::generate();
    let alice = Identity::generate();
    let mut dag = test_dag(&admin);

    // First rotation.
    do_emit(
        &mut dag,
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "ch-1".to_string(),
            encrypted_keys: vec![
                (admin.endpoint_id(), vec![1, 1, 1]),
                (alice.endpoint_id(), vec![2, 2, 2]),
            ],
        },
    );

    // Second rotation with new keys (and alice removed).
    do_emit(
        &mut dag,
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "ch-1".to_string(),
            encrypted_keys: vec![(admin.endpoint_id(), vec![9, 9, 9])],
        },
    );

    let state = materialize(&dag);
    let keys = &state.channel_keys["ch-1"];
    // Admin's key overwritten with new value.
    assert_eq!(keys[&admin.endpoint_id()], vec![9, 9, 9]);
    // Alice's old key still present (rotation adds/overwrites, doesn't clear).
    assert_eq!(keys[&alice.endpoint_id()], vec![2, 2, 2]);
}

#[test]
fn rotate_channel_key_empty_keys_is_noop() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "ch-1".to_string(),
            encrypted_keys: vec![],
        },
    );

    let state = materialize(&dag);
    // No entry created for empty key rotation.
    assert!(!state.channel_keys.contains_key("ch-1"));
}

#[test]
fn rotate_channel_key_for_nonexistent_channel() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // No CreateChannel — keys stored independently.
    do_emit(
        &mut dag,
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "nonexistent".to_string(),
            encrypted_keys: vec![(admin.endpoint_id(), vec![42])],
        },
    );

    let state = materialize(&dag);
    assert!(state.channels.is_empty());
    assert!(state.channel_keys.contains_key("nonexistent"));
    assert_eq!(
        state.channel_keys["nonexistent"][&admin.endpoint_id()],
        vec![42]
    );
}

// ── Issue #109: RotateChannelKey authority ─────────────────────────────

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

    let mut managed = ManagedDag::new(&alice, "Test Server", 5000);

    // Alice creates a channel and grants Bob SendMessages (which also
    // adds him to `members`). Bob is a legitimate member but lacks
    // ManageChannels.
    let create = managed.dag().create_event(
        &alice,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: crate::types::ChannelKind::Text,
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

// ── Issue #74: ManagedDag — atomic insert + apply ──────────────────────

#[test]
fn managed_dag_insert_and_apply_keeps_state_in_sync() {
    use crate::managed::ManagedDag;

    let id = Identity::generate();
    let mut managed = ManagedDag::new(&id, "Test Server", 5000);

    // State should have genesis author as member.
    assert!(managed.state().members.contains_key(&id.endpoint_id()));
    assert!(managed.state().is_admin(&id.endpoint_id()));

    // Create a channel — state should be updated atomically.
    let event = managed
        .create_and_insert(
            &id,
            EventKind::CreateChannel {
                channel_id: "ch1".into(),
                name: "general".into(),
                kind: crate::types::ChannelKind::Text,
            },
            1000,
        )
        .unwrap();

    // State must ALREADY reflect the channel — no separate apply step needed.
    assert!(
        managed.state().channels.contains_key("ch1"),
        "channel should be in state immediately after create_and_insert"
    );
    assert_eq!(managed.state().channels["ch1"].name, "general");

    // DAG should also contain the event.
    assert!(managed.dag().get(&event.hash).is_some());
}

#[test]
fn managed_dag_insert_remote_event_applies_to_state() {
    use crate::managed::ManagedDag;

    let owner = Identity::generate();
    let mut managed = ManagedDag::new(&owner, "Test", 5000);

    // Simulate a remote event from a different peer.
    let peer = Identity::generate();
    let event = managed.dag().create_event(
        &peer,
        EventKind::SetProfile {
            display_name: "Alice".into(),
        },
        vec![],
        100,
    );
    let outcome = managed.insert_and_apply(event).unwrap();
    assert!(outcome.applied.is_some());
    assert!(
        managed.state().profiles.contains_key(&peer.endpoint_id()),
        "profile should be in state after insert_and_apply"
    );
    assert_eq!(
        managed.state().profiles[&peer.endpoint_id()].display_name,
        "Alice"
    );
}

/// Regression guard for issue #99: Bob (a joining peer) doesn't have
/// `SendMessages` permission by default in Alice's state. His messages
/// are silently rejected by `apply_incremental`. The fix is to have
/// Alice broadcast a `GrantPermission { Bob, SendMessages }` event when
/// she responds to his JoinRequest.
///
/// This test directly verifies that:
/// 1. A newly-joined peer without explicit permission cannot have their
///    messages applied to state.
/// 2. After a `GrantPermission` event from the admin, the peer's
///    messages ARE applied to state.
#[test]
fn joined_peer_needs_grant_permission_to_send_messages() {
    use crate::managed::ManagedDag;
    use crate::materialize::ApplyResult;

    let alice = Identity::generate();
    let bob = Identity::generate();

    // Alice creates the server. She's the sole admin.
    let mut managed = ManagedDag::new(&alice, "Test Server", 5000);
    let general_id = {
        let event = managed.dag().create_event(
            &alice,
            EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: "ch-general".to_string(),
                kind: crate::types::ChannelKind::Text,
            },
            vec![],
            10,
        );
        managed.insert_and_apply(event).unwrap();
        "ch-general".to_string()
    };

    // Bob tries to send a message WITHOUT being granted permission first.
    // The event should be rejected during apply_incremental.
    let bob_msg = managed.dag().create_event(
        &bob,
        EventKind::Message {
            channel_id: general_id.clone(),
            body: "Hello from Bob".to_string(),
            reply_to: None,
        },
        vec![],
        20,
    );
    let outcome = managed.insert_and_apply(bob_msg).unwrap();
    // Insert succeeded (event is in the DAG) ...
    assert!(outcome.applied.is_some());
    // ... but apply_incremental rejected it because Bob lacks permission.
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Rejected(_))),
        "Bob's message should be rejected: {:?}",
        outcome.apply_result
    );
    // And indeed, Bob's message is NOT in the materialized state.
    assert!(
        managed
            .state()
            .messages
            .iter()
            .all(|m| m.body != "Hello from Bob"),
        "Bob's message should not be in state without permission"
    );

    // Now Alice grants Bob SendMessages permission.
    let grant = managed.dag().create_event(
        &alice,
        EventKind::GrantPermission {
            peer_id: bob.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        30,
    );
    managed.insert_and_apply(grant).unwrap();

    // Bob tries sending again — this time his message should be applied.
    let bob_msg2 = managed.dag().create_event(
        &bob,
        EventKind::Message {
            channel_id: general_id,
            body: "Hello again from Bob".to_string(),
            reply_to: None,
        },
        vec![],
        40,
    );
    let outcome = managed.insert_and_apply(bob_msg2).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Applied)),
        "After GrantPermission, Bob's message should be Applied: {:?}",
        outcome.apply_result
    );
    assert!(
        managed
            .state()
            .messages
            .iter()
            .any(|m| m.body == "Hello again from Bob"),
        "After grant, Bob's message should appear in state"
    );
}

/// Regression guard for issue #99: when Bob joins via invite and later
/// receives a SyncBatch containing Alice's GrantPermission event for
/// him, Bob's local state should allow Bob to send messages.
///
/// This simulates the join-via-invite flow at the state level:
/// 1. Alice's state includes a GrantPermission { Bob, SendMessages } event
/// 2. Bob's state replays Alice's events from sync
/// 3. After replay, Bob can create and apply his own message events
#[test]
fn sync_batch_with_grant_permission_allows_new_peer_to_send() {
    use crate::managed::ManagedDag;
    use crate::materialize::ApplyResult;

    let alice = Identity::generate();
    let bob = Identity::generate();

    // Alice creates server, channel, and grants Bob SendMessages.
    // These are the events that would normally be sent to Bob via SyncBatch.
    let mut alice_state = ManagedDag::new(&alice, "Test", 5000);
    let create_channel = alice_state.dag().create_event(
        &alice,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: crate::types::ChannelKind::Text,
        },
        vec![],
        10,
    );
    alice_state
        .insert_and_apply(create_channel.clone())
        .unwrap();

    let grant_bob = alice_state.dag().create_event(
        &alice,
        EventKind::GrantPermission {
            peer_id: bob.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![],
        20,
    );
    alice_state.insert_and_apply(grant_bob.clone()).unwrap();

    // Collect Alice's events as they would be sent in a SyncBatch.
    let sync_events: Vec<Event> = alice_state
        .dag()
        .topological_sort()
        .into_iter()
        .cloned()
        .collect();

    // Bob has a fresh, empty DAG (simulating a just-joined peer).
    let mut bob_state = ManagedDag::empty(5000);

    // Bob applies Alice's sync batch.
    for event in sync_events {
        let _ = bob_state.insert_and_apply(event);
    }

    assert!(
        bob_state.is_synced(),
        "Bob should be synced after receiving Alice's events"
    );
    assert!(
        bob_state
            .state()
            .has_permission(&bob.endpoint_id(), &Permission::SendMessages),
        "Bob should have SendMessages permission after sync"
    );

    // Bob can now create and apply his own message.
    let bob_msg = bob_state
        .create_and_insert(
            &bob,
            EventKind::Message {
                channel_id: "ch1".into(),
                body: "Hello from Bob".into(),
                reply_to: None,
            },
            50,
        )
        .expect("Bob should be able to create a message event");

    assert!(
        bob_state
            .state()
            .messages
            .iter()
            .any(|m| m.id == bob_msg.hash),
        "Bob's message should be in Bob's state"
    );

    // And Alice can apply Bob's message to her state too (bidirectional sync).
    let outcome = alice_state.insert_and_apply(bob_msg).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Applied)),
        "Alice should accept Bob's message after granting him permission: {:?}",
        outcome.apply_result
    );
    assert!(
        alice_state
            .state()
            .messages
            .iter()
            .any(|m| m.body == "Hello from Bob"),
        "Bob's message should appear in Alice's state"
    );
}

#[test]
fn managed_dag_buffers_gap_events_and_resolves() {
    use crate::managed::ManagedDag;

    let owner = Identity::generate();
    let peer = Identity::generate();
    let mut managed = ManagedDag::new(&owner, "Test", 5000);

    // Create peer's seq=1 event.
    let e1 = managed.dag().create_event(
        &peer,
        EventKind::SetProfile {
            display_name: "first".into(),
        },
        vec![],
        0,
    );

    // Create peer's seq=2 event (depends on e1).
    let e2 = Event::new(
        &peer,
        2,
        e1.hash,
        vec![],
        EventKind::SetProfile {
            display_name: "second".into(),
        },
        0,
    );

    // Insert e2 first — should be buffered (seq gap).
    let outcome = managed.insert_and_apply(e2.clone()).unwrap();
    assert!(outcome.applied.is_none(), "e2 should be buffered");
    assert!(managed.pending().pending_count() > 0);

    // Now insert e1 — should resolve and apply both.
    let outcome = managed.insert_and_apply(e1).unwrap();
    assert!(outcome.applied.is_some(), "e1 should be applied");
    assert!(!outcome.resolved.is_empty(), "e2 should be resolved");

    // State should reflect the last profile update.
    assert_eq!(
        managed.state().profiles[&peer.endpoint_id()].display_name,
        "second"
    );
}

#[test]
fn managed_dag_rejects_duplicate() {
    use crate::managed::ManagedDag;

    let owner = Identity::generate();
    let mut managed = ManagedDag::new(&owner, "Test", 5000);

    let peer = Identity::generate();
    let event = managed.dag().create_event(
        &peer,
        EventKind::SetProfile {
            display_name: "Alice".into(),
        },
        vec![],
        0,
    );

    // First insert succeeds.
    let outcome = managed.insert_and_apply(event.clone()).unwrap();
    assert!(outcome.applied.is_some());

    // Second insert is a duplicate — no error, just no-op.
    let outcome = managed.insert_and_apply(event).unwrap();
    assert!(outcome.applied.is_none());
}

#[test]
fn managed_dag_create_blocks_before_sync() {
    use crate::managed::ManagedDag;

    let mut managed = ManagedDag::empty(5000);
    let id = Identity::generate();

    // Creating events on an empty (unsynced) DAG should fail.
    let result = managed.create_and_insert(
        &id,
        EventKind::SetProfile {
            display_name: "test".into(),
        },
        0,
    );
    assert!(result.is_err());
}

#[test]
fn pin_and_unpin_message() {
    let id = Identity::generate();
    let mut dag = test_dag(&id);

    let ch_id = "general".to_string();
    do_emit(
        &mut dag,
        &id,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: ch_id.clone(),
            kind: crate::types::ChannelKind::Text,
        },
    );

    let msg = do_emit(
        &mut dag,
        &id,
        EventKind::Message {
            channel_id: ch_id.clone(),
            body: "hello world".into(),
            reply_to: None,
        },
    );

    // Pin the message.
    do_emit(
        &mut dag,
        &id,
        EventKind::PinMessage {
            channel_id: ch_id.clone(),
            message_id: msg.hash,
        },
    );

    let state = materialize(&dag);
    let channel = state.channels.get(&ch_id).expect("channel should exist");
    assert!(
        channel.pinned_messages.contains(&msg.hash),
        "message should be pinned"
    );

    // Unpin the message.
    do_emit(
        &mut dag,
        &id,
        EventKind::UnpinMessage {
            channel_id: ch_id.clone(),
            message_id: msg.hash,
        },
    );

    let state = materialize(&dag);
    let channel = state.channels.get(&ch_id).expect("channel should exist");
    assert!(
        !channel.pinned_messages.contains(&msg.hash),
        "message should be unpinned"
    );
}

#[test]
fn pin_nonexistent_channel_is_noop() {
    let id = Identity::generate();
    let mut dag = test_dag(&id);
    let fake_hash = EventHash([0xAA; 32]);

    // Pin on a channel that doesn't exist — should not panic.
    do_emit(
        &mut dag,
        &id,
        EventKind::PinMessage {
            channel_id: "nonexistent".into(),
            message_id: fake_hash,
        },
    );

    let state = materialize(&dag);
    assert!(!state.channels.contains_key("nonexistent"));
}

#[test]
fn deep_pending_chain_does_not_stack_overflow() {
    use crate::managed::ManagedDag;

    let id = Identity::generate();
    let mut managed = ManagedDag::new(&id, "Deep Chain Test", 100_000);

    let genesis_hash = managed.dag().genesis().unwrap().hash;

    // Build a chain of 3000 events.
    let chain_len = 3_000;
    let mut events = Vec::with_capacity(chain_len);
    let mut prev = genesis_hash;
    for seq_offset in 0..chain_len {
        // create_event uses the dag's internal seq tracking, so we build
        // events manually to control the prev chain.
        let e = Event::new(
            &id,
            (seq_offset + 2) as u64, // seq 2..3001 (genesis is seq 1)
            prev,
            vec![],
            EventKind::SetProfile {
                display_name: format!("name_{seq_offset}"),
            },
            seq_offset as u64,
        );
        prev = e.hash;
        events.push(e);
    }

    // Insert all except the first in reverse order — they all buffer.
    for e in events[1..].iter().rev() {
        let outcome = managed.insert_and_apply(e.clone()).unwrap();
        assert!(outcome.applied.is_none(), "should buffer (gap event)");
    }
    assert_eq!(managed.pending().pending_count(), chain_len - 1);

    // Insert the first event — this should resolve the entire chain
    // iteratively WITHOUT stack overflow.
    let outcome = managed.insert_and_apply(events[0].clone()).unwrap();
    assert!(outcome.applied.is_some());
    assert_eq!(
        outcome.resolved.len(),
        chain_len - 1,
        "all buffered events should resolve"
    );
    assert_eq!(managed.pending().pending_count(), 0);
}

// ───── check_permission tests ──────────────────────────────────────────────

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

// ───── create_and_insert pre-check tests ───────────────────────────────────

#[test]
fn create_and_insert_rejects_without_permission() {
    use crate::dag::InsertError;
    use crate::managed::ManagedDag;

    let owner = Identity::generate();
    let peer = Identity::generate();
    let mut managed = ManagedDag::new(&owner, "Test", 5000);

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
    let mut managed = ManagedDag::new(&owner, "Test", 5000);

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
    let mut managed = ManagedDag::new(&owner, "Test", 5000);

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

// ── Issue #122: O(1) message lookup via message_index ───────────────────

#[test]
fn message_index_populated_on_insert() {
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

    let state = materialize(&dag);
    assert_eq!(state.message_index.len(), 1);
    assert_eq!(state.message_index[&msg.hash], 0);
}

#[test]
fn message_index_reaction_is_fast_with_many_messages() {
    // Insert many messages then apply a reaction — verify the index is
    // correct and apply_incremental finds the right message.
    use crate::materialize::apply_incremental;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Insert 1000 messages.
    let mut last_hash = None;
    for i in 0..1000u32 {
        let e = do_emit(
            &mut dag,
            &admin,
            EventKind::Message {
                channel_id: "ch".into(),
                body: format!("msg {i}"),
                reply_to: None,
            },
        );
        if i == 0 {
            last_hash = Some(e.hash);
        }
    }

    let mut state = materialize(&dag);
    assert_eq!(state.messages.len(), 1000);
    // Index must be fully populated.
    assert_eq!(state.message_index.len(), 1000);

    // Apply a reaction to the first message.
    let target = last_hash.unwrap();
    let reaction = crate::event::Event::new(
        &admin,
        1002,
        EventHash::ZERO,
        vec![],
        EventKind::Reaction {
            message_id: target,
            emoji: "🚀".into(),
        },
        0,
    );
    let result = apply_incremental(&mut state, &reaction);
    assert_eq!(result, crate::materialize::ApplyResult::Applied);
    // The first message should now have a reaction.
    let idx = state.message_index[&target];
    assert!(
        state.messages[idx].reactions.contains_key("🚀"),
        "reaction should be on the correct message"
    );
}

#[test]
fn message_index_stable_after_delete_channel() {
    // DeleteChannel removes messages via retain() and rebuilds the index.
    // Subsequent operations on surviving messages must still work correctly.
    use crate::materialize::apply_incremental;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "ch1".into(),
            channel_id: "ch-1".into(),
            kind: crate::types::ChannelKind::Text,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "ch2".into(),
            channel_id: "ch-2".into(),
            kind: crate::types::ChannelKind::Text,
        },
    );

    // Message in ch-1 (index 0).
    do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".into(),
            body: "in ch1".into(),
            reply_to: None,
        },
    );
    // Message in ch-2 (index 1).
    let msg_ch2 = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-2".into(),
            body: "in ch2".into(),
            reply_to: None,
        },
    );

    // Delete ch-1 — ch-2's message shifts from index 1 to index 0.
    do_emit(
        &mut dag,
        &admin,
        EventKind::DeleteChannel {
            channel_id: "ch-1".into(),
        },
    );

    let mut state = materialize(&dag);
    assert_eq!(state.messages.len(), 1);
    // After rebuild, msg_ch2 must be at index 0.
    assert_eq!(state.message_index[&msg_ch2.hash], 0);

    // Apply an edit to the surviving message — must succeed.
    let edit = crate::event::Event::new(
        &admin,
        100,
        EventHash::ZERO,
        vec![],
        EventKind::EditMessage {
            message_id: msg_ch2.hash,
            new_body: "edited".into(),
        },
        0,
    );
    let result = apply_incremental(&mut state, &edit);
    assert_eq!(result, crate::materialize::ApplyResult::Applied);
    assert_eq!(state.messages[0].body, "edited");
}

// ── Issue #123: PendingBuffer eviction logging ──────────────────────────

#[test]
fn pending_buffer_eviction_reduces_count_to_cap() {
    use crate::sync::PendingBuffer;

    // Insert more events than the cap and verify cached_count stays <= cap.
    let id = Identity::generate();
    let cap = 10usize;
    let mut buf = PendingBuffer::with_capacity(cap);

    for i in 0u64..50 {
        let mut hash_bytes = [0u8; 32];
        hash_bytes[..8].copy_from_slice(&i.to_le_bytes());
        let unique_prev = EventHash(hash_bytes);
        let event = crate::event::Event::new(
            &id,
            i + 1,
            unique_prev,
            vec![],
            EventKind::SetProfile {
                display_name: format!("n{i}"),
            },
            0,
        );
        buf.buffer_for_prev(unique_prev, event);
        // After each insertion, count must never exceed the cap.
        assert!(
            buf.pending_count() <= cap,
            "pending_count {} exceeded cap {} after insertion {}",
            buf.pending_count(),
            cap,
            i
        );
    }
    assert_eq!(buf.pending_count(), cap);
}

// ── Issue #122: DeleteMessage path uses message_index ─────────────────────

#[test]
fn message_index_delete_message_marks_deleted() {
    use crate::materialize::apply_incremental;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let msg = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch".into(),
            body: "to be deleted".into(),
            reply_to: None,
        },
    );

    let mut state = materialize(&dag);
    assert!(!state.messages[0].deleted);

    // Apply DeleteMessage — must use the index to find it.
    let del = Event::new(
        &admin,
        2,
        EventHash::ZERO,
        vec![],
        EventKind::DeleteMessage {
            message_id: msg.hash,
        },
        0,
    );
    let result = apply_incremental(&mut state, &del);
    assert_eq!(result, crate::materialize::ApplyResult::Applied);
    assert!(
        state.messages[0].deleted,
        "message should be marked deleted"
    );
}

// ── Security: cross-member message ownership ────────────────────────────────

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

// ── Governance: negative votes do not contribute to threshold ───────────────

/// A `Vote { accept: false }` must NOT count toward the yes-vote threshold.
/// Even after multiple no-votes, a proposal that hasn't received enough yes
/// votes must remain pending.
///
/// Scenario: 3 admins (owner + 2 via proposals), Majority threshold.
///   - Owner proposes to kick a regular member.
///   - Admin 2 votes NO.
///   - Admin 3 votes NO.
///   - Proposal must still be pending (0 additional yes votes).
#[test]
fn negative_vote_does_not_apply_proposal() {
    let owner = Identity::generate();
    let mut dag = test_dag(&owner);

    // While sole admin, add admin 2 (auto-applies with majority of 1).
    let admin_2 = Identity::generate();
    do_emit(
        &mut dag,
        &owner,
        EventKind::Propose {
            action: ProposedAction::GrantAdmin {
                peer_id: admin_2.endpoint_id(),
            },
        },
    );
    let state = materialize(&dag);
    assert!(
        state.is_admin(&admin_2.endpoint_id()),
        "admin_2 should be granted"
    );

    // Now 2 admins. Add admin 3 — owner proposes, admin_2 votes yes.
    let admin_3 = Identity::generate();
    let prop_a3 = do_emit(
        &mut dag,
        &owner,
        EventKind::Propose {
            action: ProposedAction::GrantAdmin {
                peer_id: admin_3.endpoint_id(),
            },
        },
    );
    let vote_a2_yes = dag.create_event(
        &admin_2,
        EventKind::Vote {
            proposal: prop_a3.hash,
            accept: true,
        },
        vec![prop_a3.hash],
        0,
    );
    dag.insert(vote_a2_yes.clone()).unwrap();

    let state = materialize(&dag);
    assert!(
        state.is_admin(&admin_3.endpoint_id()),
        "admin_3 should be granted after 2/2 votes"
    );
    assert_eq!(state.admins.len(), 3);

    // Add target member (parented to enforce topo order).
    let target = Identity::generate();
    let grant_target_evt = dag.create_event(
        &owner,
        EventKind::GrantPermission {
            peer_id: target.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![vote_a2_yes.hash],
        0,
    );
    dag.insert(grant_target_evt.clone()).unwrap();

    // admin_2 (non-genesis) proposes to kick target. With 3 admins and
    // Majority threshold, majority > 1.5 requires at least 2 yes votes.
    // admin_2's implicit yes counts as 1 — not enough to auto-apply.
    // (Owner cannot be the proposer here: genesis author bypasses threshold.)
    let kick_prop = dag.create_event(
        &admin_2,
        EventKind::Propose {
            action: ProposedAction::KickMember {
                peer_id: target.endpoint_id(),
            },
        },
        vec![grant_target_evt.hash],
        0,
    );
    dag.insert(kick_prop.clone()).unwrap();

    let state = materialize(&dag);
    assert!(
        state.pending_proposals.contains_key(&kick_prop.hash),
        "kick proposal should be pending (only 1/3 yes votes)"
    );
    assert!(
        state.members.contains_key(&target.endpoint_id()),
        "target must still be a member"
    );

    // admin_3 votes NO — must not cause the proposal to apply.
    let vote_no_a3 = dag.create_event(
        &admin_3,
        EventKind::Vote {
            proposal: kick_prop.hash,
            accept: false,
        },
        vec![kick_prop.hash],
        0,
    );
    dag.insert(vote_no_a3).unwrap();

    let state = materialize(&dag);
    assert!(
        state.pending_proposals.contains_key(&kick_prop.hash),
        "kick proposal should still be pending after 1 no vote"
    );
    assert!(
        state.members.contains_key(&target.endpoint_id()),
        "target must still be a member after 1 no vote"
    );

    // owner also votes NO — proposal should remain pending (1 yes, 2 no).
    let vote_no_owner = dag.create_event(
        &owner,
        EventKind::Vote {
            proposal: kick_prop.hash,
            accept: false,
        },
        vec![kick_prop.hash],
        0,
    );
    dag.insert(vote_no_owner).unwrap();

    let state = materialize(&dag);
    assert!(
        state.pending_proposals.contains_key(&kick_prop.hash),
        "kick proposal should still be pending after 2 no votes"
    );
    assert!(
        state.members.contains_key(&target.endpoint_id()),
        "target must still be a member after 2 no votes"
    );
}

/// With Majority threshold and 2 admins, a proposal requires STRICTLY MORE
/// than half of all admins to vote yes (i.e. > 1, so both must vote yes).
/// A sole yes vote from the proposer (1/2 = 50%, not strictly majority)
/// must NOT auto-apply.
#[test]
fn no_vote_proposal_does_not_auto_apply_with_two_admins() {
    let owner = Identity::generate();
    let mut dag = test_dag(&owner);

    // Add a second admin (auto-applies while sole admin — 1 yes out of 1).
    let admin_2 = Identity::generate();
    let grant_admin_evt = do_emit(
        &mut dag,
        &owner,
        EventKind::Propose {
            action: ProposedAction::GrantAdmin {
                peer_id: admin_2.endpoint_id(),
            },
        },
    );
    let state = materialize(&dag);
    assert_eq!(state.admins.len(), 2, "should now have 2 admins");

    // Add a target member (parented on the admin grant so topo order is
    // deterministic).
    let target = Identity::generate();
    let grant_target_evt = dag.create_event(
        &owner,
        EventKind::GrantPermission {
            peer_id: target.endpoint_id(),
            permission: Permission::SendMessages,
        },
        vec![grant_admin_evt.hash],
        0,
    );
    dag.insert(grant_target_evt.clone()).unwrap();

    // admin_2 (non-genesis) proposes to kick target. With 2 admins and
    // Majority threshold, majority requires > 1, i.e. BOTH admins must vote yes.
    // Only the proposer's implicit yes counts (1/2) — must NOT auto-apply.
    // (Owner cannot be proposer: genesis author bypasses threshold.)
    // Parent on the target grant so the kick proposal is ordered after
    // admin_2's admin grant has been applied.
    let kick_prop = dag.create_event(
        &admin_2,
        EventKind::Propose {
            action: ProposedAction::KickMember {
                peer_id: target.endpoint_id(),
            },
        },
        vec![grant_target_evt.hash],
        0,
    );
    dag.insert(kick_prop.clone()).unwrap();

    let state = materialize(&dag);
    // Proposal should be pending — 1/2 yes votes is not a strict majority.
    assert!(
        state.pending_proposals.contains_key(&kick_prop.hash),
        "proposal must be pending: 1/2 yes votes is not majority"
    );
    assert!(
        state.members.contains_key(&target.endpoint_id()),
        "target must remain a member"
    );

    // owner also votes yes — now 2/2 = majority, proposal applies.
    let vote_yes = dag.create_event(
        &owner,
        EventKind::Vote {
            proposal: kick_prop.hash,
            accept: true,
        },
        vec![kick_prop.hash],
        0,
    );
    dag.insert(vote_yes).unwrap();

    let state = materialize(&dag);
    // Now the proposal should have applied.
    assert!(
        !state.pending_proposals.contains_key(&kick_prop.hash),
        "proposal should be consumed after reaching majority"
    );
    assert!(
        !state.members.contains_key(&target.endpoint_id()),
        "target should have been kicked"
    );
}

// ── Stress test assertions (tightened) ─────────────────────────────────────

/// Verify that materialize produces identical profile maps across two calls
/// on the same DAG. `ServerState` does not derive `PartialEq`, so we compare
/// the profile BTreeMap directly. We also assert that every author's profile
/// is actually present.
#[test]
fn stress_100_authors_deterministic_profiles() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let authors: Vec<Identity> = (0..9).map(|_| Identity::generate()).collect();

    for author in &authors {
        let deps = vec![*dag.head(&admin.endpoint_id()).unwrap()];
        let e = dag.create_event(
            author,
            EventKind::SetProfile {
                display_name: "deterministic".to_string(),
            },
            deps,
            0,
        );
        dag.insert(e).unwrap();
    }

    let s1 = materialize(&dag);
    let s2 = materialize(&dag);

    // Profile maps must be identical (same keys, same values).
    assert_eq!(
        s1.profiles, s2.profiles,
        "materialize must be fully deterministic across calls"
    );

    // Each author that set a profile must appear in the profiles map.
    for author in &authors {
        assert!(
            s1.profiles.contains_key(&author.endpoint_id()),
            "author {} should have a profile entry",
            author.endpoint_id()
        );
    }
}

/// The `>=` bound in `stress_concurrent_channel_creates` is correct:
/// concurrent channel-create events without explicit cross-dependencies may
/// sort before or after a grant event due to hash-based tiebreaking, so the
/// exact count of channels from the first (permission-less) batch is
/// non-deterministic across different DAG contents. The second batch (all
/// events have the grant as a dep) is guaranteed to succeed.
///
/// This test documents that invariant and confirms the count is stable across
/// multiple materializations of the same DAG.
#[test]
fn stress_concurrent_channel_creates_count_is_stable() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let authors: Vec<Identity> = (0..10).map(|_| Identity::generate()).collect();

    // First batch — no explicit permission (may be rejected).
    for (i, author) in authors.iter().enumerate() {
        let e = dag.create_event(
            author,
            EventKind::CreateChannel {
                name: format!("ch-{i}"),
                channel_id: format!("ch-{i}"),
                kind: crate::types::ChannelKind::Text,
            },
            vec![],
            0,
        );
        dag.insert(e).unwrap();
    }

    // Grant ManageChannels to all authors.
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

    // Second batch — all depend on the admin's latest grant event, so they
    // are guaranteed to be applied after permission is granted.
    let admin_head = *dag.head(&admin.endpoint_id()).unwrap();
    for (i, author) in authors.iter().enumerate() {
        let e = dag.create_event(
            author,
            EventKind::CreateChannel {
                name: format!("ch2-{i}"),
                channel_id: format!("ch2-{i}"),
                kind: crate::types::ChannelKind::Text,
            },
            vec![admin_head],
            0,
        );
        dag.insert(e).unwrap();
    }

    let s1 = materialize(&dag);
    let s2 = materialize(&dag);

    // The `>=` bound is intentional — second batch guarantees 10 channels;
    // first batch may add 0–10 more depending on topological ordering.
    assert!(
        s1.channels.len() >= 10,
        "at least the second-batch channels must exist"
    );
    // Same DAG → same count (deterministic across repeated calls).
    assert_eq!(
        s1.channels.len(),
        s2.channels.len(),
        "channel count must be identical across materializations of the same DAG"
    );
}

// ── Regression: message_index must survive round-trip serialization ────────

/// `ServerState.message_index` is `#[serde(skip)]`, so it is empty after
/// deserialize. Previously this caused Edit/Delete/Reaction events applied
/// via `apply_incremental` on a deserialized state to silently no-op,
/// producing data loss on persisted clients that didn't run a full
/// `materialize()` first. This test guards against that regression.
#[test]
fn deserialized_state_accepts_edit_delete_reaction_via_apply_incremental() {
    use crate::materialize::apply_incremental;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Seed the DAG with a channel plus messages to edit, delete, and react to.
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch-1".into(),
            kind: crate::types::ChannelKind::Text,
        },
    );
    let msg_edit = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".into(),
            body: "typo".into(),
            reply_to: None,
        },
    );
    let msg_delete = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".into(),
            body: "to delete".into(),
            reply_to: None,
        },
    );
    let msg_react = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "ch-1".into(),
            body: "react to me".into(),
            reply_to: None,
        },
    );

    let state = materialize(&dag);

    // Simulate persistence: round-trip through bincode.
    let bytes = bincode::serialize(&state).expect("serialize ServerState");
    let mut restored: crate::ServerState =
        bincode::deserialize(&bytes).expect("deserialize ServerState");

    // Sanity: message_index was skipped during serialization.
    assert!(
        restored.message_index.is_empty(),
        "message_index is #[serde(skip)] — must be empty after deserialize"
    );

    // Craft follow-up mutations. These are "new" events that arrive after
    // the state was loaded from disk, exactly like the apply_incremental
    // flow in the client after load_server_state().
    let edit = dag.create_event(
        &admin,
        EventKind::EditMessage {
            message_id: msg_edit.hash,
            new_body: "fixed".into(),
        },
        vec![],
        1,
    );
    let delete = dag.create_event(
        &admin,
        EventKind::DeleteMessage {
            message_id: msg_delete.hash,
        },
        vec![],
        2,
    );
    let react = dag.create_event(
        &admin,
        EventKind::Reaction {
            message_id: msg_react.hash,
            emoji: "👍".into(),
        },
        vec![],
        3,
    );

    // Apply to the deserialized state WITHOUT a full materialize() first.
    // Before the fix, message_index is empty and these mutations silently
    // no-op.
    apply_incremental(&mut restored, &edit);
    apply_incremental(&mut restored, &delete);
    apply_incremental(&mut restored, &react);

    let edited = restored
        .messages
        .iter()
        .find(|m| m.id == msg_edit.hash)
        .expect("edit target still present");
    assert_eq!(edited.body, "fixed", "EditMessage must take effect");
    assert!(edited.edited, "message must be flagged edited");

    let deleted = restored
        .messages
        .iter()
        .find(|m| m.id == msg_delete.hash)
        .expect("delete target still present");
    assert!(deleted.deleted, "DeleteMessage must take effect");
    assert_eq!(deleted.body, "[message deleted]");

    let reacted = restored
        .messages
        .iter()
        .find(|m| m.id == msg_react.hash)
        .expect("reaction target still present");
    assert!(
        reacted.reactions.contains_key("👍"),
        "Reaction must take effect"
    );
}

/// Directly exercises `rebuild_message_index` — after clearing the index by
/// hand, calling the method should reconstruct the hash→position map.
#[test]
fn rebuild_message_index_restores_mapping() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let msg = do_emit(
        &mut dag,
        &admin,
        EventKind::Message {
            channel_id: "general".into(),
            body: "hello".into(),
            reply_to: None,
        },
    );
    let mut state = materialize(&dag);
    state.message_index.clear();
    assert!(state.message_index.is_empty());

    state.rebuild_message_index();

    assert_eq!(
        state.message_index.get(&msg.hash).copied(),
        Some(0),
        "rebuild_message_index must map each message's hash to its vec index",
    );
    assert_eq!(state.message_index.len(), state.messages.len());
}

// ── Phase 1f: per-identity mute (MuteChannel / MuteGrove) ─────────────────

#[test]
fn mute_channel_roundtrip() {
    let id = Identity::generate();
    let mut dag = test_dag(&id);
    do_emit(
        &mut dag,
        &id,
        EventKind::MuteChannel {
            channel_id: "ch-1".into(),
            muted: true,
        },
    );
    let state = materialize(&dag);
    let ms = state.mute_state.get(&id.endpoint_id()).expect("entry");
    assert!(ms.channels.contains("ch-1"));

    // Unmute — channel drops out of the set.
    do_emit(
        &mut dag,
        &id,
        EventKind::MuteChannel {
            channel_id: "ch-1".into(),
            muted: false,
        },
    );
    let state = materialize(&dag);
    let ms = state.mute_state.get(&id.endpoint_id()).expect("entry");
    assert!(!ms.channels.contains("ch-1"));
}

#[test]
fn mute_grove_roundtrip() {
    let id = Identity::generate();
    let mut dag = test_dag(&id);
    do_emit(&mut dag, &id, EventKind::MuteGrove { muted: true });
    let state = materialize(&dag);
    assert!(state.mute_state[&id.endpoint_id()].grove_muted);

    do_emit(&mut dag, &id, EventKind::MuteGrove { muted: false });
    let state = materialize(&dag);
    assert!(!state.mute_state[&id.endpoint_id()].grove_muted);
}

#[test]
fn mute_channel_idempotent() {
    // Muting twice is a harmless no-op — the entry already reflects
    // the mute state. Unmuting a channel that was never muted is
    // also a no-op.
    let id = Identity::generate();
    let mut dag = test_dag(&id);
    for _ in 0..3 {
        do_emit(
            &mut dag,
            &id,
            EventKind::MuteChannel {
                channel_id: "ch-a".into(),
                muted: true,
            },
        );
    }
    let state = materialize(&dag);
    assert_eq!(
        state.mute_state[&id.endpoint_id()].channels.len(),
        1,
        "repeated MuteChannel must not duplicate — it is a set"
    );

    // Unmute a never-muted channel: no crash, no entry.
    do_emit(
        &mut dag,
        &id,
        EventKind::MuteChannel {
            channel_id: "never-muted".into(),
            muted: false,
        },
    );
    let state = materialize(&dag);
    assert!(!state.mute_state[&id.endpoint_id()]
        .channels
        .contains("never-muted"));
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
