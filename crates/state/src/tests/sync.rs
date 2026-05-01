//! //! Multi-peer sync semantics: joining peers, batched grant + send.

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
    let mut managed = ManagedDag::new(&alice, "Test Server", 5000).unwrap();
    let general_id = {
        let event = managed.dag().create_event(
            &alice,
            EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: "ch-general".to_string(),
                kind: crate::types::ChannelKind::Text,
                ephemeral: None,
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
    let mut alice_state = ManagedDag::new(&alice, "Test", 5000).unwrap();
    let create_channel = alice_state.dag().create_event(
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
