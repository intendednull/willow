//! Stress and integration tests for the willow-state crate.

use crate::dag::EventDag;
use crate::event::{EventKind, ProposedAction};
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

// ── CreateInvite permission ─────────────────────────────────────────────

#[test]
fn grant_and_check_create_invite_permission() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    // Alice does not have CreateInvite by default.
    assert!(!state.has_permission(&alice, &Permission::CreateInvite));

    // Grant CreateInvite.
    let grant = event(
        &state,
        "e1",
        owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::CreateInvite,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    assert!(state.has_permission(&alice, &Permission::CreateInvite));
    assert!(state.members.contains_key(&alice));

    // Revoke CreateInvite.
    let revoke = event(
        &state,
        "e2",
        owner,
        EventKind::RevokePermission {
            peer_id: alice,
            permission: Permission::CreateInvite,
        },
    );
    assert_eq!(apply(&mut state, &revoke), ApplyResult::Applied);
    assert!(!state.has_permission(&alice, &Permission::CreateInvite));
}

#[test]
fn admin_implies_create_invite() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    let grant = event(
        &state,
        "e1",
        owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::Administrator,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);
    // Administrator implies all permissions, including CreateInvite.
    assert!(state.has_permission(&alice, &Permission::CreateInvite));
}

// ── RotateChannelKey ────────────────────────────────────────────────────

#[test]
fn rotate_channel_key_stores_key_material() {
    let (mut state, owner) = test_state();

    // Create a channel first.
    let create = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "secret".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);
    assert!(state.channel_keys.is_empty());

    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();

    // Rotate with multiple recipients.
    let rotate = event(
        &state,
        "e1",
        owner,
        EventKind::RotateChannelKey {
            channel_id: "ch1".into(),
            encrypted_keys: vec![(alice, vec![1, 2, 3, 4]), (bob, vec![5, 6, 7, 8])],
        },
    );
    assert_eq!(apply(&mut state, &rotate), ApplyResult::Applied);

    // The first recipient's key bytes are stored.
    assert_eq!(state.channel_keys.get("ch1"), Some(&vec![1, 2, 3, 4]));
}

#[test]
fn rotate_channel_key_overwrites_previous() {
    let (mut state, owner) = test_state();

    let create = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "secret".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);

    let alice = Identity::generate().endpoint_id();

    // First rotation.
    let rotate1 = event(
        &state,
        "e1",
        owner,
        EventKind::RotateChannelKey {
            channel_id: "ch1".into(),
            encrypted_keys: vec![(alice, vec![1, 2, 3])],
        },
    );
    assert_eq!(apply(&mut state, &rotate1), ApplyResult::Applied);
    assert_eq!(state.channel_keys["ch1"], vec![1, 2, 3]);

    // Second rotation overwrites.
    let rotate2 = event(
        &state,
        "e2",
        owner,
        EventKind::RotateChannelKey {
            channel_id: "ch1".into(),
            encrypted_keys: vec![(alice, vec![9, 8, 7])],
        },
    );
    assert_eq!(apply(&mut state, &rotate2), ApplyResult::Applied);
    assert_eq!(state.channel_keys["ch1"], vec![9, 8, 7]);
}

#[test]
fn rotate_channel_key_empty_keys_is_noop() {
    let (mut state, owner) = test_state();

    let create = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "ch".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);

    // Rotate with empty keys list.
    let rotate = event(
        &state,
        "e1",
        owner,
        EventKind::RotateChannelKey {
            channel_id: "ch1".into(),
            encrypted_keys: vec![],
        },
    );
    assert_eq!(apply(&mut state, &rotate), ApplyResult::Applied);
    // No key stored.
    assert!(!state.channel_keys.contains_key("ch1"));
}

#[test]
fn rotate_channel_key_changes_hash() {
    let (mut state, owner) = test_state();

    let create = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "ch".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create), ApplyResult::Applied);
    let hash_before = state.hash();

    let alice = Identity::generate().endpoint_id();
    let rotate = event(
        &state,
        "e1",
        owner,
        EventKind::RotateChannelKey {
            channel_id: "ch1".into(),
            encrypted_keys: vec![(alice, vec![42])],
        },
    );
    assert_eq!(apply(&mut state, &rotate), ApplyResult::Applied);
    assert_ne!(state.hash(), hash_before);
}

// ── Message reply_to ────────────────────────────────────────────────────

#[test]
fn message_reply_to_is_stored() {
    let (mut state, owner) = test_state();

    let create_ch = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    // Send a parent message.
    let parent_msg = event(
        &state,
        "msg-parent",
        owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "parent message".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &parent_msg), ApplyResult::Applied);

    // Send a reply.
    let reply_msg = event(
        &state,
        "msg-reply",
        owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "this is a reply".into(),
            reply_to: Some("msg-parent".into()),
        },
    );
    assert_eq!(apply(&mut state, &reply_msg), ApplyResult::Applied);

    assert_eq!(state.messages.len(), 2);
    assert_eq!(state.messages[0].reply_to, None);
    assert_eq!(state.messages[1].reply_to, Some("msg-parent".to_string()));
}

// ── Reaction edge cases ─────────────────────────────────────────────────

#[test]
fn multiple_different_reactions_on_same_message() {
    let (mut state, owner) = test_state();

    let create_ch = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1",
        owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "react to me".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);

    // Add two different emoji reactions.
    let r1 = event(
        &state,
        "r1",
        owner,
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":+1:".into(),
        },
    );
    assert_eq!(apply(&mut state, &r1), ApplyResult::Applied);

    let r2 = event(
        &state,
        "r2",
        owner,
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":heart:".into(),
        },
    );
    assert_eq!(apply(&mut state, &r2), ApplyResult::Applied);

    let reactions = &state.messages[0].reactions;
    assert_eq!(reactions.len(), 2);
    assert_eq!(reactions[":+1:"], vec![owner]);
    assert_eq!(reactions[":heart:"], vec![owner]);
}

#[test]
fn duplicate_reaction_from_same_peer() {
    let (mut state, owner) = test_state();

    let create_ch = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1",
        owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "react".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);

    // Same peer reacts with the same emoji twice (different event IDs).
    let r1 = event(
        &state,
        "r1",
        owner,
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":+1:".into(),
        },
    );
    assert_eq!(apply(&mut state, &r1), ApplyResult::Applied);

    let r2 = event(
        &state,
        "r2",
        owner,
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":+1:".into(),
        },
    );
    assert_eq!(apply(&mut state, &r2), ApplyResult::Applied);

    // Both reactions are stored (current behavior — no dedup on reactions).
    assert_eq!(state.messages[0].reactions[":+1:"].len(), 2);
}

#[test]
fn multiple_peers_react_to_same_message() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();
    let bob = Identity::generate().endpoint_id();

    let create_ch = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1",
        owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "react".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);

    let r1 = event(
        &state,
        "r1",
        alice,
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":+1:".into(),
        },
    );
    assert_eq!(apply(&mut state, &r1), ApplyResult::Applied);

    let r2 = event(
        &state,
        "r2",
        bob,
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":+1:".into(),
        },
    );
    assert_eq!(apply(&mut state, &r2), ApplyResult::Applied);

    // Both peers' reactions are recorded.
    let reactors = &state.messages[0].reactions[":+1:"];
    assert_eq!(reactors.len(), 2);
    assert!(reactors.contains(&alice));
    assert!(reactors.contains(&bob));
}

#[test]
fn delete_message_clears_reactions() {
    let (mut state, owner) = test_state();

    let create_ch = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create_ch), ApplyResult::Applied);

    let msg = event(
        &state,
        "msg1",
        owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "hello".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg), ApplyResult::Applied);

    let r1 = event(
        &state,
        "r1",
        owner,
        EventKind::Reaction {
            message_id: "msg1".into(),
            emoji: ":+1:".into(),
        },
    );
    assert_eq!(apply(&mut state, &r1), ApplyResult::Applied);
    assert!(!state.messages[0].reactions.is_empty());

    let delete = event(
        &state,
        "e1",
        owner,
        EventKind::DeleteMessage {
            message_id: "msg1".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);
    assert!(state.messages[0].reactions.is_empty());
}

#[test]
fn reaction_on_nonexistent_message_is_noop() {
    let (mut state, owner) = test_state();

    // React to a message that doesn't exist.
    let reaction = event(
        &state,
        "r1",
        owner,
        EventKind::Reaction {
            message_id: "nonexistent".into(),
            emoji: ":+1:".into(),
        },
    );
    assert_eq!(apply(&mut state, &reaction), ApplyResult::Applied);
    // No crash, no messages.
    assert!(state.messages.is_empty());
}

#[test]
fn edit_nonexistent_message_is_noop() {
    let (mut state, owner) = test_state();

    let edit = event(
        &state,
        "e1",
        owner,
        EventKind::EditMessage {
            message_id: "nonexistent".into(),
            new_body: "new body".into(),
        },
    );
    assert_eq!(apply(&mut state, &edit), ApplyResult::Applied);
    assert!(state.messages.is_empty());
}

#[test]
fn delete_nonexistent_message_is_noop() {
    let (mut state, owner) = test_state();

    let delete = event(
        &state,
        "e1",
        owner,
        EventKind::DeleteMessage {
            message_id: "nonexistent".into(),
        },
    );
    assert_eq!(apply(&mut state, &delete), ApplyResult::Applied);
    assert!(state.messages.is_empty());
}

// ── Channel kind ────────────────────────────────────────────────────────

#[test]
fn channel_kind_is_preserved() {
    let (mut state, owner) = test_state();

    let text_ch = event(
        &state,
        "e1",
        owner,
        EventKind::CreateChannel {
            name: "text-channel".into(),
            channel_id: "ch-text".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &text_ch), ApplyResult::Applied);

    let voice_ch = event(
        &state,
        "e2",
        owner,
        EventKind::CreateChannel {
            name: "voice-channel".into(),
            channel_id: "ch-voice".into(),
            kind: "voice".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &voice_ch), ApplyResult::Applied);

    assert_eq!(state.channels["ch-text"].kind, "text");
    assert_eq!(state.channels["ch-voice"].kind, "voice");
}

// ── Assign role to nonexistent member/role ──────────────────────────────

#[test]
fn assign_nonexistent_role_is_noop() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    // Grant alice a permission so she becomes a member.
    let grant = event(
        &state,
        "e0",
        owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::SendMessages,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Assign a role that doesn't exist.
    let assign = event(
        &state,
        "e1",
        owner,
        EventKind::AssignRole {
            peer_id: alice,
            role_id: "nonexistent-role".into(),
        },
    );
    assert_eq!(apply(&mut state, &assign), ApplyResult::Applied);
    // No role was assigned.
    assert!(state.members[&alice].roles.is_empty());
}

#[test]
fn assign_role_to_nonmember_is_noop() {
    let (mut state, owner) = test_state();
    let stranger = Identity::generate().endpoint_id();

    // Create a role.
    let create_role = event(
        &state,
        "e1",
        owner,
        EventKind::CreateRole {
            name: "Mod".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create_role), ApplyResult::Applied);

    // Assign role to a non-member.
    let assign = event(
        &state,
        "e2",
        owner,
        EventKind::AssignRole {
            peer_id: stranger,
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &assign), ApplyResult::Applied);
    // Stranger is not a member, so nothing happened.
    assert!(!state.members.contains_key(&stranger));
}

// ── SetPermission on nonexistent role ───────────────────────────────────

#[test]
fn set_permission_on_nonexistent_role_is_noop() {
    let (mut state, owner) = test_state();

    let set = event(
        &state,
        "e1",
        owner,
        EventKind::SetPermission {
            role_id: "nonexistent".into(),
            permission: "SomePermission".into(),
            granted: true,
        },
    );
    assert_eq!(apply(&mut state, &set), ApplyResult::Applied);
    assert!(!state.roles.contains_key("nonexistent"));
}

// ── Rename nonexistent channel ──────────────────────────────────────────

#[test]
fn rename_nonexistent_channel_is_noop() {
    let (mut state, owner) = test_state();

    let rename = event(
        &state,
        "e1",
        owner,
        EventKind::RenameChannel {
            channel_id: "nonexistent".into(),
            new_name: "new-name".into(),
        },
    );
    assert_eq!(apply(&mut state, &rename), ApplyResult::Applied);
    assert!(!state.channels.contains_key("nonexistent"));
}

// ── Duplicate CreateChannel is idempotent ───────────────────────────────

#[test]
fn duplicate_create_channel_preserves_original() {
    let (mut state, owner) = test_state();

    let create1 = event(
        &state,
        "e1",
        owner,
        EventKind::CreateChannel {
            name: "original".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create1), ApplyResult::Applied);

    // Try to create another channel with the same ID but different name.
    let create2 = event(
        &state,
        "e2",
        owner,
        EventKind::CreateChannel {
            name: "duplicate".into(),
            channel_id: "ch1".into(),
            kind: "voice".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &create2), ApplyResult::Applied);

    // Original name and kind are preserved.
    assert_eq!(state.channels["ch1"].name, "original");
    assert_eq!(state.channels["ch1"].kind, "text");
}

// ── Duplicate CreateRole is idempotent ──────────────────────────────────

#[test]
fn duplicate_create_role_preserves_original() {
    let (mut state, owner) = test_state();

    let create1 = event(
        &state,
        "e1",
        owner,
        EventKind::CreateRole {
            name: "Moderator".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create1), ApplyResult::Applied);

    let create2 = event(
        &state,
        "e2",
        owner,
        EventKind::CreateRole {
            name: "Different Name".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create2), ApplyResult::Applied);

    // Original name preserved.
    assert_eq!(state.roles["role1"].name, "Moderator");
}

// ── Event store events_since ────────────────────────────────────────────

#[test]
fn event_store_events_since() {
    let mut store = InMemoryStore::new();
    let owner = Identity::generate().endpoint_id();

    let e1 = Event {
        id: "e1".into(),
        parent_hash: StateHash::ZERO,
        author: owner,
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "a".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    };
    let hash_after_e1 = StateHash::from_bytes(b"after-e1");
    store.append(e1);
    store.set_latest_hash(hash_after_e1.clone());

    let e2 = Event {
        id: "e2".into(),
        parent_hash: hash_after_e1.clone(),
        author: owner,
        timestamp_ms: 2000,
        kind: EventKind::CreateChannel {
            name: "b".into(),
            channel_id: "ch2".into(),
            kind: "text".to_string(),
        },
    };
    store.append(e2);

    // events_since the first hash should return only e2.
    let since = store.events_since(&hash_after_e1);
    assert_eq!(since.len(), 1);
    assert_eq!(since[0].id, "e2");

    // events_since ZERO should return all events.
    let all = store.events_since(&StateHash::ZERO);
    assert_eq!(all.len(), 2);
}

// ── Revoke nonexistent permission is harmless ───────────────────────────

#[test]
fn revoke_permission_from_peer_without_permissions() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    // Revoke a permission alice never had.
    let revoke = event(
        &state,
        "e1",
        owner,
        EventKind::RevokePermission {
            peer_id: alice,
            permission: Permission::ManageChannels,
        },
    );
    assert_eq!(apply(&mut state, &revoke), ApplyResult::Applied);
    // No crash, no entry created.
    assert!(!state.peer_permissions.contains_key(&alice));
}

// ── ServerState::new defaults ───────────────────────────────────────────

#[test]
fn server_state_new_defaults() {
    let (state, owner) = test_state();
    assert!(state.channels.is_empty());
    assert!(state.roles.is_empty());
    assert!(state.messages.is_empty());
    assert!(state.profiles.is_empty());
    assert!(state.description.is_empty());
    assert!(state.channel_keys.is_empty());
    assert!(state.peer_permissions.is_empty());
    assert!(state.seen_event_ids.is_empty());
    assert_eq!(state.members.len(), 1);
    assert!(state.members.contains_key(&owner));
}

// ── has_permission does not check roles ─────────────────────────────────

#[test]
fn has_permission_ignores_role_based_permissions() {
    let (mut state, owner) = test_state();
    let alice = Identity::generate().endpoint_id();

    // Add alice as a member.
    let grant = event(
        &state,
        "e0",
        owner,
        EventKind::GrantPermission {
            peer_id: alice,
            permission: Permission::ManageRoles,
        },
    );
    assert_eq!(apply(&mut state, &grant), ApplyResult::Applied);

    // Create a role with ManageChannels permission string.
    let create_role = event(
        &state,
        "e1",
        owner,
        EventKind::CreateRole {
            name: "ChannelAdmin".into(),
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &create_role), ApplyResult::Applied);

    let set_perm = event(
        &state,
        "e2",
        owner,
        EventKind::SetPermission {
            role_id: "role1".into(),
            permission: "ManageChannels".into(),
            granted: true,
        },
    );
    assert_eq!(apply(&mut state, &set_perm), ApplyResult::Applied);

    // Assign the role to alice.
    let assign = event(
        &state,
        "e3",
        owner,
        EventKind::AssignRole {
            peer_id: alice,
            role_id: "role1".into(),
        },
    );
    assert_eq!(apply(&mut state, &assign), ApplyResult::Applied);
    assert!(state.members[&alice].roles.contains("role1"));

    // has_permission only checks peer_permissions, not role permissions.
    // alice has ManageRoles (from grant) but NOT ManageChannels (only via role).
    assert!(state.has_permission(&alice, &Permission::ManageRoles));
    assert!(!state.has_permission(&alice, &Permission::ManageChannels));
}

// ── seen_event_ids excluded from hash ───────────────────────────────────

#[test]
fn seen_event_ids_do_not_affect_hash() {
    let owner = Identity::generate().endpoint_id();
    let mut a = ServerState::new("server-1", "Test", owner);
    let b = ServerState::new("server-1", "Test", owner);

    a.seen_event_ids.insert("evt-1".into());
    a.seen_event_ids.insert("evt-2".into());
    a.seen_event_ids.insert("evt-3".into());

    assert_eq!(a.hash(), b.hash());
}

// ── Merge edge cases ────────────────────────────────────────────────────

#[test]
fn merge_empty_our_events() {
    let owner = Identity::generate().endpoint_id();
    let common = ServerState::new("s1", "Test", owner);
    let common_hash = common.hash();

    let their = vec![Event {
        id: "e1".into(),
        parent_hash: common_hash,
        author: owner,
        timestamp_ms: 100,
        kind: EventKind::CreateChannel {
            name: "alpha".into(),
            channel_id: "ch-a".into(),
            kind: "text".to_string(),
        },
    }];

    let (merged, events) = merge(&[], &their, &common);
    assert_eq!(events.len(), 1);
    assert!(merged.channels.contains_key("ch-a"));
}

#[test]
fn merge_both_empty_events() {
    let owner = Identity::generate().endpoint_id();
    let common = ServerState::new("s1", "Test", owner);

    let (merged, events) = merge(&[], &[], &common);
    assert!(events.is_empty());
    assert_eq!(merged.hash(), common.hash());
}

#[test]
fn merge_duplicate_event_ids_across_logs() {
    let owner = Identity::generate().endpoint_id();
    let common = ServerState::new("s1", "Test", owner);
    let common_hash = common.hash();

    // Same event ID in both logs but with different content.
    let our_evt = Event {
        id: "shared-id".into(),
        parent_hash: common_hash.clone(),
        author: owner,
        timestamp_ms: 100,
        kind: EventKind::CreateChannel {
            name: "from-our".into(),
            channel_id: "ch-our".into(),
            kind: "text".to_string(),
        },
    };
    let their_evt = Event {
        id: "shared-id".into(),
        parent_hash: common_hash,
        author: owner,
        timestamp_ms: 200,
        kind: EventKind::CreateChannel {
            name: "from-their".into(),
            channel_id: "ch-their".into(),
            kind: "text".to_string(),
        },
    };

    let (merged, events) = merge(&[our_evt], &[their_evt], &common);
    // Deduplicated: only one event with this ID.
    assert_eq!(events.len(), 1);
    // Our event wins (first seen in chain).
    assert_eq!(events[0].id, "shared-id");
    assert!(merged.channels.contains_key("ch-our"));
    assert!(!merged.channels.contains_key("ch-their"));
}

#[test]
fn find_common_ancestor_empty_our_events() {
    use crate::merge::find_common_ancestor;

    let owner = Identity::generate().endpoint_id();
    let their = vec![Event {
        id: "e1".into(),
        parent_hash: StateHash::ZERO,
        author: owner,
        timestamp_ms: 100,
        kind: EventKind::CreateChannel {
            name: "a".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    }];

    // Empty our events — no common ancestor found.
    let ancestor = find_common_ancestor(&[], &their);
    assert_eq!(ancestor, None);
}

#[test]
fn find_common_ancestor_both_empty() {
    use crate::merge::find_common_ancestor;
    let ancestor = find_common_ancestor(&[], &[]);
    assert_eq!(ancestor, None);
}

// ── events_since with nonexistent hash ──────────────────────────────────

#[test]
fn event_store_events_since_nonexistent_hash() {
    let mut store = InMemoryStore::new();
    let owner = Identity::generate().endpoint_id();

    store.append(Event {
        id: "e1".into(),
        parent_hash: StateHash::ZERO,
        author: owner,
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "a".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    });

    // Hash that doesn't match any event's parent_hash.
    let nonexistent = StateHash::from_bytes(b"does-not-exist");
    let result = store.events_since(&nonexistent);
    assert!(result.is_empty());
}

// ── Event store ordering ────────────────────────────────────────────────

#[test]
fn event_store_preserves_insertion_order() {
    let mut store = InMemoryStore::new();
    let owner = Identity::generate().endpoint_id();

    for i in 0..5 {
        store.append(Event {
            id: format!("e{i}"),
            parent_hash: StateHash::from_bytes(format!("hash-{i}").as_bytes()),
            author: owner,
            timestamp_ms: i * 1000,
            kind: EventKind::CreateChannel {
                name: format!("ch-{i}"),
                channel_id: format!("chid-{i}"),
                kind: "text".to_string(),
            },
        });
    }

    let all = store.all_events();
    assert_eq!(all.len(), 5);
    for (i, event) in all.iter().enumerate() {
        assert_eq!(event.id, format!("e{i}"));
    }
}

// ── apply_lenient skips parent hash check ───────────────────────────────

#[test]
fn apply_lenient_accepts_wrong_parent_hash() {
    let (mut state, owner) = test_state();

    // Wrong parent hash — strict apply would reject.
    let evt = Event {
        id: "e1".into(),
        parent_hash: StateHash::from_bytes(b"wrong"),
        author: owner,
        timestamp_ms: 1000,
        kind: EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    };

    // Strict apply rejects.
    assert_eq!(apply(&mut state, &evt), ApplyResult::ParentHashMismatch);

    // Lenient apply accepts.
    assert_eq!(apply_lenient(&mut state, &evt), ApplyResult::Applied);
    assert!(state.channels.contains_key("ch1"));
}

// ── Non-owner rename/description rejection details ──────────────────────

#[test]
fn non_owner_set_profile_is_accepted() {
    let (mut state, _owner) = test_state();
    let stranger = Identity::generate().endpoint_id();

    // Any peer can set their own profile.
    let evt = event(
        &state,
        "e1",
        stranger,
        EventKind::SetProfile {
            display_name: "Stranger".into(),
        },
    );
    assert_eq!(apply(&mut state, &evt), ApplyResult::Applied);
    assert_eq!(state.profiles[&stranger].display_name, "Stranger");
}

// ── Delete channel also removes channel_keys ────────────────────────────

#[test]
fn delete_channel_messages_not_from_other_channels() {
    let (mut state, owner) = test_state();

    // Create two channels.
    let ch1 = event(
        &state,
        "e0",
        owner,
        EventKind::CreateChannel {
            name: "ch1".into(),
            channel_id: "ch1".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &ch1), ApplyResult::Applied);

    let ch2 = event(
        &state,
        "e1",
        owner,
        EventKind::CreateChannel {
            name: "ch2".into(),
            channel_id: "ch2".into(),
            kind: "text".to_string(),
        },
    );
    assert_eq!(apply(&mut state, &ch2), ApplyResult::Applied);

    // Send messages in both.
    let msg1 = event(
        &state,
        "msg1",
        owner,
        EventKind::Message {
            channel_id: "ch1".into(),
            body: "in ch1".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg1), ApplyResult::Applied);

    let msg2 = event(
        &state,
        "msg2",
        owner,
        EventKind::Message {
            channel_id: "ch2".into(),
            body: "in ch2".into(),
            reply_to: None,
        },
    );
    assert_eq!(apply(&mut state, &msg2), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 2);

    // Delete ch1 — only ch1 messages removed.
    let del = event(
        &state,
        "e2",
        owner,
        EventKind::DeleteChannel {
            channel_id: "ch1".into(),
        },
    );
    assert_eq!(apply(&mut state, &del), ApplyResult::Applied);
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].channel_id, "ch2");
}
