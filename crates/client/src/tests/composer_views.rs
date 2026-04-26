//! Tests for the composer-facing client surfaces — phase 3a tasks T1, T2, T3.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/composer.md`.
//! Plan: `docs/plans/2026-04-26-ui-phase-3a-composer.md`.
//!
//! These tests are pure client-tier: no DOM, no networking. They poke
//! the source `event_state_addr` directly to seed the grove with extra
//! members and channels, then call the new view selectors and pure
//! filter helpers.

use willow_identity::Identity;
use willow_state::{Member, Profile};

use crate::test_client;

/// Insert a member + matching profile into the test client's state.
async fn add_member_with_display(
    client: &crate::ClientHandle<willow_network::mem::MemNetwork>,
    peer_id: willow_identity::EndpointId,
    display_name: &str,
) {
    let display = display_name.to_string();
    willow_actor::state::mutate(&client.event_state_addr, move |es| {
        es.members.insert(
            peer_id,
            Member {
                peer_id,
                roles: Default::default(),
                display_name: None,
            },
        );
        let mut p = Profile::new(peer_id);
        p.display_name = display;
        es.profiles.insert(peer_id, p);
    })
    .await;
}

// ───── T1: mention_candidates ───────────────────────────────────────────

#[tokio::test]
async fn mention_candidates_includes_channel_peers() {
    // Two extra peers in the grove + local peer (genesis owner). The
    // candidate list must contain exactly the two extras.
    let (client, _broker) = test_client();
    let mira = Identity::generate().endpoint_id();
    let rin = Identity::generate().endpoint_id();
    add_member_with_display(&client, mira, "Mira").await;
    add_member_with_display(&client, rin, "Rin").await;

    let local = client.identity().endpoint_id();
    let cands = client.views().mention_candidates("general", local).await;
    let ids: std::collections::BTreeSet<_> = cands.iter().map(|c| c.peer_id).collect();
    let expected: std::collections::BTreeSet<_> = [mira, rin].into_iter().collect();
    assert_eq!(
        ids, expected,
        "mention_candidates must return exactly the two non-local channel peers"
    );
    assert_eq!(cands.len(), 2);
    // Display name fallback chain should pick the profile name.
    let mira_row = cands.iter().find(|c| c.peer_id == mira).unwrap();
    assert_eq!(mira_row.display_name, "Mira");
    // Handle is the lowercase 4-char hex prefix of the peer id today.
    assert_eq!(mira_row.handle.len(), 4);
    assert_eq!(mira_row.handle, mira_row.handle.to_lowercase());
}

#[tokio::test]
async fn mention_candidates_excludes_self() {
    // Local peer (genesis owner) is in events.members; the helper must
    // never return it, even when no other peers are in the grove.
    let (client, _broker) = test_client();
    let local = client.identity().endpoint_id();
    let cands = client.views().mention_candidates("general", local).await;
    assert!(
        cands.iter().all(|c| c.peer_id != local),
        "mention_candidates must filter out the local peer; got {:?}",
        cands.iter().map(|c| c.peer_id).collect::<Vec<_>>()
    );
    // Genesis-owner-only world → empty candidate list.
    assert!(
        cands.is_empty(),
        "no other members → empty candidate list; got {} entries",
        cands.len()
    );
}

#[tokio::test]
async fn mention_candidates_empty_when_channel_missing() {
    // Bonus pin: the helper guards against an unknown channel name.
    let (client, _broker) = test_client();
    let mira = Identity::generate().endpoint_id();
    add_member_with_display(&client, mira, "Mira").await;
    let local = client.identity().endpoint_id();
    let cands = client
        .views()
        .mention_candidates("nope-not-a-channel", local)
        .await;
    assert!(cands.is_empty());
}
