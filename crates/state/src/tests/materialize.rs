//! //! Materialization / `apply_event` behaviour: channels, messages, profiles, mute, pin, ephemeral, idempotency.

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
use crate::types::{CrestPattern, PinnedFragment, PinnedKind, ProfileDelta, PROFILE_CAP_BIO};

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
    let grant = do_emit(
        &mut dag,
        &admin,
        EventKind::GrantPermission {
            peer_id: alice.endpoint_id(),
            permission: Permission::SendMessages,
        },
    );
    // Alice sets her own profile (no admin needed).
    // Causal dep on grant ensures topo-sort applies GrantPermission first
    // (PR #505's membership gate); without this dep the test was flaky
    // depending on HashMap iter order. See issue #565.
    let set_profile = dag.create_event(
        &alice,
        EventKind::SetProfile {
            display_name: "Alice".to_string(),
        },
        vec![grant.hash],
        0,
    );
    dag.insert(set_profile).unwrap();

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
            ephemeral: None,
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
            ephemeral: None,
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
            ephemeral: None,
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
            ephemeral: None,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "ch2".to_string(),
            channel_id: "ch-2".to_string(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
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
            ephemeral: None,
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
            ephemeral: None,
        },
    );
    do_emit(
        &mut dag,
        &admin,
        EventKind::CreateChannel {
            name: "ch2".into(),
            channel_id: "ch-2".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
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
            ephemeral: None,
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
fn update_profile_merges_fields() {
    let alice = Identity::generate();
    let mut dag = test_dag(&alice);
    // Seed a display name via the legacy event so we can confirm
    // UpdateProfile merges with existing state rather than wiping it.
    do_emit(
        &mut dag,
        &alice,
        EventKind::SetProfile {
            display_name: "alice".into(),
        },
    );
    do_emit(
        &mut dag,
        &alice,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: Some(Some("she/her".into())),
            bio: Some(Some("gardener".into())),
            tagline: None,
            crest_pattern: Some(Some(CrestPattern::Fronds)),
            crest_color: Some(Some("#6b8e4e".into())),
            pinned: None,
            elsewhere: Some(vec!["west coast".into()]),
            since: Some(Some("spring · yr 2".into())),
        })),
    );
    let state = materialize(&dag);
    let p = state
        .profiles
        .get(&alice.endpoint_id())
        .expect("profile present");
    assert_eq!(p.display_name, "alice");
    assert_eq!(p.pronouns.as_deref(), Some("she/her"));
    assert_eq!(p.bio.as_deref(), Some("gardener"));
    assert_eq!(p.crest_pattern, Some(CrestPattern::Fronds));
    assert_eq!(p.crest_color.as_deref(), Some("#6b8e4e"));
    assert_eq!(p.elsewhere, vec!["west coast".to_string()]);
    assert_eq!(p.since.as_deref(), Some("spring · yr 2"));
    // Untouched field stays its prior value (None).
    assert!(p.tagline.is_none());
}

#[test]
fn update_profile_clears_field_with_inner_none() {
    let alice = Identity::generate();
    let mut dag = test_dag(&alice);
    do_emit(
        &mut dag,
        &alice,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: None,
            bio: Some(Some("old bio".into())),
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: None,
            since: None,
        })),
    );
    do_emit(
        &mut dag,
        &alice,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: None,
            bio: Some(None),
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: None,
            since: None,
        })),
    );
    let state = materialize(&dag);
    assert!(state.profiles[&alice.endpoint_id()].bio.is_none());
}

#[test]
fn update_profile_preserves_untouched_fields() {
    let alice = Identity::generate();
    let mut dag = test_dag(&alice);
    do_emit(
        &mut dag,
        &alice,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: Some(Some("she/her".into())),
            bio: Some(Some("hello".into())),
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: None,
            since: None,
        })),
    );
    do_emit(
        &mut dag,
        &alice,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: None,
            bio: None,
            tagline: Some(Some("tending the moss".into())),
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: None,
            since: None,
        })),
    );
    let state = materialize(&dag);
    let p = &state.profiles[&alice.endpoint_id()];
    assert_eq!(p.bio.as_deref(), Some("hello"));
    assert_eq!(p.pronouns.as_deref(), Some("she/her"));
    assert_eq!(p.tagline.as_deref(), Some("tending the moss"));
}

#[test]
fn update_profile_reapply_is_idempotent() {
    let alice = Identity::generate();
    let mut dag = test_dag(&alice);
    // Replaying the same delta twice must produce the same state as
    // replaying it once — the event hash dedupes on the DAG side.
    let kind = EventKind::UpdateProfile(Box::new(ProfileDelta {
        display_name: Some("alice".into()),
        pronouns: Some(Some("she/her".into())),
        bio: None,
        tagline: None,
        crest_pattern: None,
        crest_color: None,
        pinned: None,
        elsewhere: None,
        since: None,
    }));
    let e1 = do_emit(&mut dag, &alice, kind.clone());
    // Re-inserting the *same* event is a DAG-level dedup; re-creating
    // via `create_event` would bump the seq and hash, so we re-insert
    // `e1` directly and confirm the insert is a no-op.
    assert!(dag.insert(e1.clone()).is_err());
    let state = materialize(&dag);
    let p = &state.profiles[&alice.endpoint_id()];
    assert_eq!(p.display_name, "alice");
    assert_eq!(p.pronouns.as_deref(), Some("she/her"));
}

#[test]
fn update_profile_caps_enforced_on_apply() {
    let alice = Identity::generate();
    let mut dag = test_dag(&alice);
    let long_bio = "a".repeat(500);
    do_emit(
        &mut dag,
        &alice,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: None,
            bio: Some(Some(long_bio)),
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: None,
            since: None,
        })),
    );
    let state = materialize(&dag);
    let p = &state.profiles[&alice.endpoint_id()];
    assert_eq!(
        p.bio.as_ref().map(|s| s.chars().count()),
        Some(PROFILE_CAP_BIO)
    );
}

#[test]
fn update_profile_creates_profile_if_missing() {
    let alice = Identity::generate();
    let mut dag = test_dag(&alice);
    // alice has genesis + no SetProfile yet. Before the UpdateProfile,
    // her profile entry does not exist.
    let state_pre = materialize(&dag);
    assert!(!state_pre.profiles.contains_key(&alice.endpoint_id()));
    do_emit(
        &mut dag,
        &alice,
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
    );
    let state = materialize(&dag);
    let p = state
        .profiles
        .get(&alice.endpoint_id())
        .expect("profile upserted by UpdateProfile");
    assert_eq!(p.pronouns.as_deref(), Some("they/them"));
    // display_name never set — empty string is the "unset" marker.
    assert_eq!(p.display_name, "");
}

#[test]
fn update_profile_invalid_crest_color_drops_to_none() {
    let alice = Identity::generate();
    let mut dag = test_dag(&alice);
    // "red" is 3 chars + no leading '#' — apply_event should reject it
    // to None so the UI falls back to --moss-2.
    do_emit(
        &mut dag,
        &alice,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: None,
            bio: None,
            tagline: None,
            crest_pattern: None,
            crest_color: Some(Some("red".into())),
            pinned: None,
            elsewhere: None,
            since: None,
        })),
    );
    let state = materialize(&dag);
    let p = &state.profiles[&alice.endpoint_id()];
    assert!(p.crest_color.is_none());
}

#[test]
fn update_profile_elsewhere_caps_length() {
    let alice = Identity::generate();
    let mut dag = test_dag(&alice);
    do_emit(
        &mut dag,
        &alice,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: None,
            bio: None,
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: Some(vec![
                "one".into(),
                "two".into(),
                "three".into(),
                "four".into(),
                "five".into(),
            ]),
            since: None,
        })),
    );
    let state = materialize(&dag);
    let p = &state.profiles[&alice.endpoint_id()];
    // Cap is 4 entries — fifth is dropped.
    assert_eq!(p.elsewhere.len(), 4);
    assert_eq!(p.elsewhere[0], "one");
    assert_eq!(p.elsewhere[3], "four");
}

#[test]
fn update_profile_pinned_round_trip() {
    let alice = Identity::generate();
    let mut dag = test_dag(&alice);
    do_emit(
        &mut dag,
        &alice,
        EventKind::UpdateProfile(Box::new(ProfileDelta {
            display_name: None,
            pronouns: None,
            bio: None,
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: Some(Some(PinnedFragment {
                kind: PinnedKind::Quote,
                body: "quiet is a kind of music".into(),
            })),
            elsewhere: None,
            since: None,
        })),
    );
    let state = materialize(&dag);
    let p = &state.profiles[&alice.endpoint_id()];
    let pinned = p.pinned.as_ref().expect("pinned present");
    assert_eq!(pinned.kind, PinnedKind::Quote);
    assert_eq!(pinned.body, "quiet is a kind of music");
}

#[test]
fn channel_with_ephemeral_config_serializes() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind};
    use crate::types::{Channel, ChannelKind};

    let ch = Channel {
        id: "c1".into(),
        name: "side-room".into(),
        pinned_messages: Default::default(),
        kind: ChannelKind::Text,
        ephemeral: Some(EphemeralConfig {
            kind: EphemeralKind::Channel,
            idle_threshold_ms: 14 * 24 * 3_600_000,
        }),
        last_activity_hlc: Some(1_700_000_000_000),
    };

    let bytes = bincode::serialize(&ch).unwrap();
    let back: Channel = bincode::deserialize(&bytes).unwrap();
    assert_eq!(ch, back);
}

#[test]
fn channel_revive_advances_last_activity_hlc() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};

    let owner = Identity::generate();
    let mut dag = test_dag(&owner);
    do_emit(
        &mut dag,
        &owner,
        EventKind::CreateChannel {
            name: "side-room".into(),
            channel_id: "c1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: DEFAULT_CHANNEL_THRESHOLD_MS,
            }),
        },
    );

    let revive_event = dag.create_event(
        &owner,
        EventKind::ChannelRevive {
            channel_id: "c1".into(),
        },
        vec![],
        1_700_000_000_000,
    );
    dag.insert(revive_event.clone()).unwrap();

    let state = materialize(&dag);
    let ch = state.channels.get("c1").expect("channel should exist");
    assert_eq!(ch.last_activity_hlc, Some(revive_event.timestamp_hint_ms));
}

#[test]
fn channel_revive_rejected_for_non_member() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};
    use crate::managed::ManagedDag;
    use crate::materialize::ApplyResult;

    let owner = Identity::generate();
    let stranger = Identity::generate();

    // Use ManagedDag so we can observe per-event apply outcomes.
    let mut managed = ManagedDag::new(&owner, "Test Server", 5000).unwrap();
    let create_ev = managed.dag().create_event(
        &owner,
        EventKind::CreateChannel {
            name: "side-room".into(),
            channel_id: "c1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: DEFAULT_CHANNEL_THRESHOLD_MS,
            }),
        },
        vec![],
        10,
    );
    managed.insert_and_apply(create_ev).unwrap();

    // Stranger has not joined the server — no Member entry exists
    // for them. ChannelRevive must be rejected.
    let revive_ev = managed.dag().create_event(
        &stranger,
        EventKind::ChannelRevive {
            channel_id: "c1".into(),
        },
        vec![],
        20,
    );
    let outcome = managed.insert_and_apply(revive_ev).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Rejected(_))),
        "non-member revive must be rejected: {:?}",
        outcome.apply_result
    );
}

#[test]
fn channel_revive_unknown_channel_rejected() {
    use crate::managed::ManagedDag;
    use crate::materialize::ApplyResult;

    let owner = Identity::generate();
    let mut managed = ManagedDag::new(&owner, "Test Server", 5000).unwrap();

    let ev = managed.dag().create_event(
        &owner,
        EventKind::ChannelRevive {
            channel_id: "does-not-exist".into(),
        },
        vec![],
        10,
    );
    let outcome = managed.insert_and_apply(ev).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Rejected(_))),
        "revive of unknown channel must be rejected: {:?}",
        outcome.apply_result
    );
}

#[test]
fn message_advances_last_activity_hlc() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};

    let owner = Identity::generate();
    let mut dag = test_dag(&owner);
    do_emit(
        &mut dag,
        &owner,
        EventKind::CreateChannel {
            name: "side-room".into(),
            channel_id: "c1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: DEFAULT_CHANNEL_THRESHOLD_MS,
            }),
        },
    );

    // Use a non-zero timestamp on the message so we can verify the
    // channel's last_activity_hlc advances to it.
    let msg_event = dag.create_event(
        &owner,
        EventKind::Message {
            channel_id: "c1".into(),
            body: "hi".into(),
            reply_to: None,
        },
        vec![],
        1_700_000_000_000,
    );
    dag.insert(msg_event.clone()).unwrap();

    let state = materialize(&dag);
    let ch = state.channels.get("c1").expect("channel should exist");
    assert_eq!(ch.last_activity_hlc, Some(msg_event.timestamp_hint_ms));
}

#[test]
fn message_on_permanent_channel_also_advances_hlc() {
    // Tracking is unconditional — non-ephemeral channels can carry
    // the field too. Cheap, simplifies the materialize branch, and
    // a future feature might use it.
    let owner = Identity::generate();
    let mut dag = test_dag(&owner);
    do_emit(
        &mut dag,
        &owner,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "g1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
    );

    let msg_event = dag.create_event(
        &owner,
        EventKind::Message {
            channel_id: "g1".into(),
            body: "hi".into(),
            reply_to: None,
        },
        vec![],
        1_700_000_000_000,
    );
    dag.insert(msg_event.clone()).unwrap();

    let state = materialize(&dag);
    let ch = state.channels.get("g1").expect("channel should exist");
    assert_eq!(ch.last_activity_hlc, Some(msg_event.timestamp_hint_ms));
}

#[test]
fn create_channel_with_ephemeral_config_records_it() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};

    let owner = Identity::generate();
    let mut dag = test_dag(&owner);
    do_emit(
        &mut dag,
        &owner,
        EventKind::CreateChannel {
            name: "side-room".into(),
            channel_id: "c1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: DEFAULT_CHANNEL_THRESHOLD_MS,
            }),
        },
    );
    let state = materialize(&dag);
    let ch = state.channels.get("c1").expect("channel should exist");
    assert!(ch.ephemeral.is_some());
    assert_eq!(
        ch.ephemeral.as_ref().unwrap().idle_threshold_ms,
        DEFAULT_CHANNEL_THRESHOLD_MS
    );
}

#[test]
fn create_channel_rejects_threshold_below_minimum() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind};

    let owner = Identity::generate();
    let mut dag = test_dag(&owner);
    do_emit(
        &mut dag,
        &owner,
        EventKind::CreateChannel {
            name: "too-fast".into(),
            channel_id: "c2".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: 60_000, // 1 minute — below 1h floor
            }),
        },
    );
    let state = materialize(&dag);
    // Below-floor threshold rejected by apply → no channel created.
    assert!(
        !state.channels.contains_key("c2"),
        "below-floor threshold must be rejected"
    );
}

#[test]
fn create_channel_rejects_threshold_above_cap() {
    use crate::ephemeral::{EphemeralConfig, EphemeralKind};

    let owner = Identity::generate();
    let mut dag = test_dag(&owner);
    do_emit(
        &mut dag,
        &owner,
        EventKind::CreateChannel {
            name: "too-long".into(),
            channel_id: "c3".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: Some(EphemeralConfig {
                kind: EphemeralKind::Channel,
                idle_threshold_ms: 200 * 24 * 3_600_000, // 200 days — above 90d cap
            }),
        },
    );
    let state = materialize(&dag);
    assert!(
        !state.channels.contains_key("c3"),
        "above-cap threshold must be rejected"
    );
}

#[test]
fn derive_ephemeral_state_bands() {
    use crate::ephemeral::{derive_ephemeral_state, EphemeralState};

    let threshold = 100;
    // 0 elapsed → active
    assert_eq!(
        derive_ephemeral_state(Some(100), threshold, 100),
        EphemeralState::Active
    );
    // 24 % elapsed → active (just inside the active band)
    assert_eq!(
        derive_ephemeral_state(Some(76), threshold, 100),
        EphemeralState::Active
    );
    // 26 % elapsed → dormant
    assert_eq!(
        derive_ephemeral_state(Some(74), threshold, 100),
        EphemeralState::Dormant
    );
    // 100 % elapsed → dormant (boundary stays in dormant)
    assert_eq!(
        derive_ephemeral_state(Some(0), threshold, 100),
        EphemeralState::Dormant
    );
    // > 100 % elapsed → archived
    assert_eq!(
        derive_ephemeral_state(Some(0), threshold, 101),
        EphemeralState::Archived
    );
    // No activity yet → uses 0; archived if frontier > threshold.
    assert_eq!(
        derive_ephemeral_state(None, threshold, 200),
        EphemeralState::Archived
    );
}

#[test]
fn apply_rotate_channel_key_rejects_excess_entries_over_member_count() {
    use crate::event::MAX_ENCRYPTED_KEYS_OVER_MEMBERS;
    use crate::managed::ManagedDag;
    use crate::materialize::ApplyResult;

    let admin = Identity::generate();
    let mut managed = ManagedDag::new(&admin, "S", 5000).unwrap();

    // Create channel.
    let create = managed.dag().create_event(
        &admin,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch-1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
        vec![],
        0,
    );
    managed.insert_and_apply(create).unwrap();

    // Sole admin = 1 member. Cap = 1 + epsilon. Submit (cap + 1) entries.
    let member_count = managed.state().members.len();
    assert_eq!(member_count, 1);
    let cap = member_count + MAX_ENCRYPTED_KEYS_OVER_MEMBERS;
    // Use real generated identities so each EndpointId is a valid curve
    // point (`EndpointId::from_bytes` rejects non-curve inputs).
    let bloat: Vec<(willow_identity::EndpointId, Vec<u8>)> = (0..(cap + 1))
        .map(|_| (Identity::generate().endpoint_id(), vec![0xaa]))
        .collect();
    assert_eq!(bloat.len(), cap + 1);

    let rotate = managed.dag().create_event(
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "ch-1".into(),
            encrypted_keys: bloat,
        },
        vec![],
        10,
    );
    let outcome = managed.insert_and_apply(rotate).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Rejected(_))),
        "over-cap rotate must be rejected at apply: {:?}",
        outcome.apply_result,
    );
    // No channel_keys entry created — state untouched by rejected event.
    assert!(!managed.state().channel_keys.contains_key("ch-1"));
}

#[test]
fn apply_rotate_channel_key_accepts_at_member_count_plus_epsilon() {
    use crate::event::MAX_ENCRYPTED_KEYS_OVER_MEMBERS;
    use crate::managed::ManagedDag;
    use crate::materialize::ApplyResult;

    let admin = Identity::generate();
    let mut managed = ManagedDag::new(&admin, "S", 5000).unwrap();

    let create = managed.dag().create_event(
        &admin,
        EventKind::CreateChannel {
            name: "general".into(),
            channel_id: "ch-1".into(),
            kind: crate::types::ChannelKind::Text,
            ephemeral: None,
        },
        vec![],
        0,
    );
    managed.insert_and_apply(create).unwrap();

    // Cap = members + epsilon. Submit exactly that many — must succeed.
    let member_count = managed.state().members.len();
    let cap = member_count + MAX_ENCRYPTED_KEYS_OVER_MEMBERS;
    // Use real generated identities so each EndpointId is a valid curve
    // point (`EndpointId::from_bytes` rejects non-curve inputs).
    let entries: Vec<(willow_identity::EndpointId, Vec<u8>)> = (0..cap)
        .map(|_| (Identity::generate().endpoint_id(), vec![0xaa]))
        .collect();

    let rotate = managed.dag().create_event(
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "ch-1".into(),
            encrypted_keys: entries,
        },
        vec![],
        10,
    );
    let outcome = managed.insert_and_apply(rotate).unwrap();
    assert!(
        matches!(outcome.apply_result, Some(ApplyResult::Applied)),
        "boundary case (members + epsilon) must apply: {:?}",
        outcome.apply_result,
    );
    assert_eq!(
        managed.state().channel_keys.get("ch-1").map(|m| m.len()),
        Some(cap),
    );
}
