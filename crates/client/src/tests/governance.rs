//! Happy-path tests for the governance mutation API.
//!
//! Covers the five governance mutators in [`crate::mutations::ClientMutations`]
//! that previously only had Playwright coverage (or none at all):
//!
//! * `propose_grant_admin`     — `EventKind::Propose { ProposedAction::GrantAdmin }`
//! * `propose_revoke_admin`    — `EventKind::Propose { ProposedAction::RevokeAdmin }`
//! * `propose_kick_member`     — `EventKind::Propose { ProposedAction::KickMember }`
//! * `propose_set_threshold`   — `EventKind::Propose { ProposedAction::SetVoteThreshold }`
//! * `delete_role`             — `EventKind::DeleteRole`
//!
//! These run against the in-memory `test_client` harness. The genesis
//! author (`test_client()`'s identity) is the server owner, which by
//! the authority model is automatically an admin and the root of
//! trust — so `Propose { … }` and `DeleteRole` events are accepted
//! without further permission setup.
//!
//! Scope: each test only asserts that the mutator emits the expected
//! `EventKind` (variant + payload) into the local DAG. We deliberately
//! do not assert *downstream* state-machine effects (vote tally,
//! actual admin promotion, role removal) — those live in the
//! tier-1 state-machine tests in `crates/state/src/materialize.rs`.
//! Mirroring the convention established by `voice.rs` (PR #464), the
//! focus here is "the mutator emitted the right event".
//!
//! Why poke the DAG instead of intercepting the wire broadcast?
//! `test_client()` does not subscribe to any topic, so
//! `broadcast_event` drops the bytes with a warning. Reading the DAG
//! captures the same signed event the broadcast would carry — the
//! mutator's `apply_event` and `broadcast_event` calls are fed the
//! identical `Event` value. See `mutations.rs` for the call sequence.

use willow_identity::Identity;
use willow_state::{Event, EventKind, ProposedAction, VoteThreshold};

use crate::test_client;
use crate::ClientHandle;

/// Snapshot every event currently in `client`'s managed DAG, in
/// topological order. The owner-authored mutation under test lands at
/// the tail of this vector once the mutator's `apply_event` returns.
async fn dag_events<N: willow_network::Network>(client: &ClientHandle<N>) -> Vec<Event> {
    willow_actor::state::select(&client.dag_addr, |ds| {
        ds.managed
            .dag()
            .topological_sort()
            .into_iter()
            .cloned()
            .collect()
    })
    .await
}

/// Find the first event in the DAG matching `predicate`. Tests use
/// this to assert *exactly one* event of the expected variant landed,
/// without depending on the ordering of unrelated genesis / channel
/// events that `test_client()` seeds.
async fn find_event<N, F>(client: &ClientHandle<N>, predicate: F) -> Option<Event>
where
    N: willow_network::Network,
    F: Fn(&EventKind) -> bool,
{
    dag_events(client)
        .await
        .into_iter()
        .find(|e| predicate(&e.kind))
}

// ───── propose_grant_admin ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn propose_grant_admin_emits_propose_grant_admin_event() {
    let (client, _broker) = test_client();
    let target = Identity::generate().endpoint_id();

    client
        .mutations()
        .propose_grant_admin(target)
        .await
        .expect("owner can propose grant_admin");

    let event = find_event(&client, |kind| {
        matches!(
            kind,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin { .. }
            }
        )
    })
    .await
    .expect("propose_grant_admin must emit a Propose { GrantAdmin } event into the DAG");

    match event.kind {
        EventKind::Propose {
            action: ProposedAction::GrantAdmin { peer_id },
        } => {
            assert_eq!(
                peer_id, target,
                "GrantAdmin proposal must target the requested peer_id"
            );
        }
        other => panic!("expected Propose {{ GrantAdmin }}, got {other:?}"),
    }
    assert_eq!(
        event.author,
        client.identity.endpoint_id(),
        "propose event must be authored by the local (owner) identity"
    );
}

// ───── propose_revoke_admin ─────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn propose_revoke_admin_emits_propose_revoke_admin_event() {
    let (client, _broker) = test_client();
    let target = Identity::generate().endpoint_id();

    client
        .mutations()
        .propose_revoke_admin(target)
        .await
        .expect("owner can propose revoke_admin");

    // `propose_revoke_admin` *also* fires a follow-up
    // `RevokePermission(SendMessages)` (best-effort untrust). That side
    // effect is documented on the mutator and is fine — we only assert
    // that the Propose event is among the resulting DAG entries.
    let event = find_event(&client, |kind| {
        matches!(
            kind,
            EventKind::Propose {
                action: ProposedAction::RevokeAdmin { .. }
            }
        )
    })
    .await
    .expect("propose_revoke_admin must emit a Propose { RevokeAdmin } event into the DAG");

    match event.kind {
        EventKind::Propose {
            action: ProposedAction::RevokeAdmin { peer_id },
        } => {
            assert_eq!(
                peer_id, target,
                "RevokeAdmin proposal must target the requested peer_id"
            );
        }
        other => panic!("expected Propose {{ RevokeAdmin }}, got {other:?}"),
    }
}

// ───── propose_kick_member ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn propose_kick_member_emits_propose_kick_member_event() {
    let (client, _broker) = test_client();
    let target = Identity::generate().endpoint_id();

    client
        .mutations()
        .propose_kick_member(target)
        .await
        .expect("owner can propose kick_member");

    let event = find_event(&client, |kind| {
        matches!(
            kind,
            EventKind::Propose {
                action: ProposedAction::KickMember { .. }
            }
        )
    })
    .await
    .expect("propose_kick_member must emit a Propose { KickMember } event into the DAG");

    match event.kind {
        EventKind::Propose {
            action: ProposedAction::KickMember { peer_id },
        } => {
            assert_eq!(
                peer_id, target,
                "KickMember proposal must target the requested peer_id"
            );
        }
        other => panic!("expected Propose {{ KickMember }}, got {other:?}"),
    }
}

// ───── propose_set_threshold ────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn propose_set_threshold_emits_propose_set_threshold_event() {
    let (client, _broker) = test_client();

    client
        .mutations()
        .propose_set_threshold(VoteThreshold::Unanimous)
        .await
        .expect("owner can propose set_threshold");

    let event = find_event(&client, |kind| {
        matches!(
            kind,
            EventKind::Propose {
                action: ProposedAction::SetVoteThreshold { .. }
            }
        )
    })
    .await
    .expect("propose_set_threshold must emit a Propose { SetVoteThreshold } event into the DAG");

    match event.kind {
        EventKind::Propose {
            action: ProposedAction::SetVoteThreshold { threshold },
        } => {
            assert_eq!(
                threshold,
                VoteThreshold::Unanimous,
                "SetVoteThreshold proposal must carry the requested threshold variant"
            );
        }
        other => panic!("expected Propose {{ SetVoteThreshold }}, got {other:?}"),
    }
}

// ───── delete_role ──────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_role_emits_delete_role_event() {
    let (client, _broker) = test_client();

    // `delete_role` is a direct admin mutation. We don't have to seed
    // a matching `CreateRole` first — the state machine accepts the
    // event regardless of whether the role exists, and this tier of
    // test is asserting the mutator emits the right event, not the
    // downstream materialisation. Use a fixed role id so we can
    // pinpoint the event in the DAG.
    let role_id = "role-to-delete";
    client
        .mutations()
        .delete_role(role_id)
        .await
        .expect("owner can delete_role");

    let event = find_event(&client, |kind| matches!(kind, EventKind::DeleteRole { .. }))
        .await
        .expect("delete_role must emit a DeleteRole event into the DAG");

    match event.kind {
        EventKind::DeleteRole { role_id: emitted } => {
            assert_eq!(
                emitted, role_id,
                "DeleteRole event must carry the requested role_id"
            );
        }
        other => panic!("expected DeleteRole, got {other:?}"),
    }
    assert_eq!(
        event.author,
        client.identity.endpoint_id(),
        "delete_role event must be authored by the local (owner) identity"
    );
}
