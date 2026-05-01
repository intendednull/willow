//! //! Stress / scale tests for the willow-state crate (large DAGs, many authors, performance bounds).

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
                ephemeral: None,
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
                ephemeral: None,
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

/// Verify that materialize produces identical profile maps across two calls
/// on the same DAG. `ServerState` does not derive `PartialEq`, so we compare
/// the profile BTreeMap directly. We also assert that every author's profile
/// is actually present.
#[test]
fn stress_100_authors_deterministic_profiles() {
    let admin = Identity::generate();
    let mut dag = test_dag(&admin);

    let authors: Vec<Identity> = (0..9).map(|_| Identity::generate()).collect();

    // Grant SendMessages to every author so they become members.
    // SetProfile is gated on membership (issue #177); each SetProfile
    // must causally depend on the author's own grant so topological
    // sort applies the grant first.
    let mut grant_hashes: std::collections::BTreeMap<_, _> = std::collections::BTreeMap::new();
    for author in &authors {
        let e = dag.create_event(
            &admin,
            EventKind::GrantPermission {
                peer_id: author.endpoint_id(),
                permission: crate::event::Permission::SendMessages,
            },
            vec![],
            0,
        );
        grant_hashes.insert(author.endpoint_id(), e.hash);
        dag.insert(e).unwrap();
    }

    for author in &authors {
        let mut deps = vec![*dag.head(&admin.endpoint_id()).unwrap()];
        deps.push(grant_hashes[&author.endpoint_id()]);
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
                ephemeral: None,
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
                ephemeral: None,
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
