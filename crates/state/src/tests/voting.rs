//! //! Governance proposals + voting flow tests (admin grants, kicks, vote ordering).

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
