//! //! DAG insertion, equivocation, topological sort, gap buffering, and `ManagedDag` tests.

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

#[test]
fn managed_dag_insert_and_apply_keeps_state_in_sync() {
    use crate::managed::ManagedDag;

    let id = Identity::generate();
    let mut managed = ManagedDag::new(&id, "Test Server", 5000).unwrap();

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
                ephemeral: None,
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
    let mut managed = ManagedDag::new(&owner, "Test", 5000).unwrap();

    // Simulate a remote event from a different peer.
    let peer = Identity::generate();
    // Grant peer membership first — `SetProfile` is gated on
    // membership (issue #177); the grant adds peer to `state.members`.
    let grant = managed.dag().create_event(
        &owner,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: crate::event::Permission::SendMessages,
        },
        vec![],
        50,
    );
    let grant_hash = grant.hash;
    managed.insert_and_apply(grant).unwrap();

    // Causally link the SetProfile to the grant so topological sort
    // applies the grant first.
    let event = managed.dag().create_event(
        &peer,
        EventKind::SetProfile {
            display_name: "Alice".into(),
        },
        vec![grant_hash],
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

#[test]
fn managed_dag_buffers_gap_events_and_resolves() {
    use crate::managed::ManagedDag;

    let owner = Identity::generate();
    let peer = Identity::generate();
    let mut managed = ManagedDag::new(&owner, "Test", 5000).unwrap();

    // Grant peer membership first — `SetProfile` is gated on
    // membership (issue #177); the grant adds peer to `state.members`.
    let grant = managed.dag().create_event(
        &owner,
        EventKind::GrantPermission {
            peer_id: peer.endpoint_id(),
            permission: crate::event::Permission::SendMessages,
        },
        vec![],
        0,
    );
    let grant_hash = grant.hash;
    managed.insert_and_apply(grant).unwrap();

    // Create peer's seq=1 event, depending on the grant so topological
    // sort guarantees the grant applies first.
    let e1 = managed.dag().create_event(
        &peer,
        EventKind::SetProfile {
            display_name: "first".into(),
        },
        vec![grant_hash],
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
    let mut managed = ManagedDag::new(&owner, "Test", 5000).unwrap();

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
fn deep_pending_chain_does_not_stack_overflow() {
    use crate::managed::ManagedDag;

    let id = Identity::generate();
    let mut managed = ManagedDag::new(&id, "Deep Chain Test", 100_000).unwrap();

    let genesis_hash = managed.dag().genesis().unwrap().hash;

    // Build a chain of 1500 events. Kept below the per-author sub-cap
    // (max_entries / DEFAULT_PENDING_PER_AUTHOR_DIVISOR == 2000) so the
    // SEC-V-08 cap doesn't drop chain links — this test is about
    // iterative resolution, not capacity policy.
    let chain_len = 1_500;
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

#[test]
fn pending_buffer_eviction_reduces_count_to_cap() {
    use crate::sync::PendingBuffer;

    // Insert more events than the cap and verify cached_count stays <= cap.
    // Override the SEC-V-08 per-author sub-cap so this test focuses on
    // global capacity-eviction behaviour.
    let id = Identity::generate();
    let cap = 10usize;
    let mut buf = PendingBuffer::with_capacity(cap).with_per_author_cap(1_000);

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

#[test]
fn dag_insert_rejects_deps_over_cap() {
    use crate::dag::InsertError;
    use crate::event::MAX_EVENT_DEPS;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // Build deps vector one entry over the cap.
    let bad_deps: Vec<EventHash> = (0..=MAX_EVENT_DEPS)
        .map(|i| EventHash::from_bytes(&i.to_le_bytes()))
        .collect();
    assert_eq!(bad_deps.len(), MAX_EVENT_DEPS + 1);

    let bloated = dag.create_event(
        &admin,
        EventKind::SetProfile {
            display_name: "x".into(),
        },
        bad_deps,
        0,
    );
    let err = dag.insert(bloated).unwrap_err();
    match err {
        InsertError::DepsTooLong { got, max } => {
            assert_eq!(got, MAX_EVENT_DEPS + 1);
            assert_eq!(max, MAX_EVENT_DEPS);
        }
        other => panic!("expected DepsTooLong, got {other:?}"),
    }
}

#[test]
fn dag_insert_accepts_deps_at_cap() {
    use crate::event::MAX_EVENT_DEPS;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let ok_deps: Vec<EventHash> = (0..MAX_EVENT_DEPS)
        .map(|i| EventHash::from_bytes(&i.to_le_bytes()))
        .collect();
    assert_eq!(ok_deps.len(), MAX_EVENT_DEPS);

    let event = dag.create_event(
        &admin,
        EventKind::SetProfile {
            display_name: "x".into(),
        },
        ok_deps,
        0,
    );
    dag.insert(event).expect("deps at cap must be accepted");
}

#[test]
fn dag_insert_rejects_oversized_encrypted_key() {
    use crate::dag::InsertError;
    use crate::event::MAX_ENCRYPTED_KEY_BYTES;

    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    // One entry whose blob is one byte over the cap.
    let too_big = vec![0xab; MAX_ENCRYPTED_KEY_BYTES + 1];
    let bloated = dag.create_event(
        &admin,
        EventKind::RotateChannelKey {
            channel_id: "ch-1".into(),
            encrypted_keys: vec![(admin.endpoint_id(), too_big)],
        },
        vec![],
        0,
    );
    let err = dag.insert(bloated).unwrap_err();
    match err {
        InsertError::EncryptedKeyTooLarge { got, max } => {
            assert_eq!(got, MAX_ENCRYPTED_KEY_BYTES + 1);
            assert_eq!(max, MAX_ENCRYPTED_KEY_BYTES);
        }
        other => panic!("expected EncryptedKeyTooLarge, got {other:?}"),
    }
}
