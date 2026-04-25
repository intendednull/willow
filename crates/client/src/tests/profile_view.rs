//! Tests for `ProfileView` derivation + `since_hint` + the
//! `update_profile_fields` mutation.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`.

use crate::{since_hint, test_client, ProfileDelta, ProfileView};
use willow_state::{CrestPattern, PinnedFragment, PinnedKind};

/// Apply a `ProfileDelta` through the real mutation path, then return a
/// freshly-built `ProfileView` for the local peer.
async fn apply_and_view(delta: ProfileDelta) -> ProfileView {
    let (client, _broker) = test_client();
    client
        .mutations()
        .update_profile_fields(delta)
        .await
        .expect("update_profile_fields must succeed");
    let local = client.identity().endpoint_id();
    client.views().profile_view_of(&local, &local).await
}

#[tokio::test]
async fn profile_view_reads_updated_fields() {
    let v = apply_and_view(ProfileDelta {
        display_name: Some("mira".into()),
        pronouns: Some(Some("she/her".into())),
        bio: Some(Some("gardener".into())),
        tagline: Some(Some("tending the moss".into())),
        crest_pattern: Some(Some(CrestPattern::Fronds)),
        crest_color: Some(Some("#6b8e4e".into())),
        pinned: Some(Some(PinnedFragment {
            kind: PinnedKind::Quote,
            body: "quiet is a kind of music".into(),
        })),
        elsewhere: Some(vec!["coast · west".into()]),
        since: Some(Some("spring · yr 2".into())),
    })
    .await;
    assert_eq!(v.display_name, "mira");
    assert_eq!(v.pronouns.as_deref(), Some("she/her"));
    assert_eq!(v.bio.as_deref(), Some("gardener"));
    assert_eq!(v.tagline.as_deref(), Some("tending the moss"));
    assert_eq!(v.crest_pattern, Some(CrestPattern::Fronds));
    assert_eq!(v.crest_color.as_deref(), Some("#6b8e4e"));
    assert_eq!(v.elsewhere, vec!["coast · west".to_string()]);
    assert_eq!(v.since.as_deref(), Some("spring · yr 2"));
    assert!(v.is_self);
    // Fingerprint is 6 words joined by ` · ` — short form is the first 3.
    assert_eq!(v.fingerprint_full.split(" · ").count(), 6);
    assert_eq!(v.fingerprint_short.split(" · ").count(), 3);
}

#[tokio::test]
async fn profile_view_defaults_crest_to_none_for_missing_fields() {
    // No UpdateProfile ever applied — every optional field starts None.
    let (client, _broker) = test_client();
    let local = client.identity().endpoint_id();
    let v = client.views().profile_view_of(&local, &local).await;
    assert!(v.crest_pattern.is_none());
    assert!(v.crest_color.is_none());
    assert!(v.bio.is_none());
    assert!(v.pinned.is_none());
    assert!(v.elsewhere.is_empty());
    // UI falls back to Leaf / --moss-2 at render time; the derivation
    // itself preserves the "unset" signal.
}

#[tokio::test]
async fn profile_view_is_self_matches_local_peer() {
    let (client, _broker) = test_client();
    let local = client.identity().endpoint_id();
    let v = client.views().profile_view_of(&local, &local).await;
    assert!(v.is_self);
    // Querying for a different peer should clear is_self.
    let other = willow_identity::Identity::generate().endpoint_id();
    let v2 = client.views().profile_view_of(&other, &local).await;
    assert!(!v2.is_self);
}

#[tokio::test]
async fn profile_view_handle_derives_from_peer_id() {
    let (client, _broker) = test_client();
    let local = client.identity().endpoint_id();
    let v = client.views().profile_view_of(&local, &local).await;
    // The handle is derived from the peer id string; it must be non-empty
    // and shorter than the full 64-hex peer id.
    assert!(!v.handle.is_empty());
    assert!(v.handle.len() < v.peer_id.len());
}

#[test]
fn since_hint_format_contains_season_and_year() {
    let earliest = 1_714_000_000_000u64; // somewhere in 2024
    let now = earliest + 2 * 365 * 86_400_000;
    let s = since_hint(earliest, now);
    assert!(
        s.starts_with("spring")
            || s.starts_with("summer")
            || s.starts_with("fall")
            || s.starts_with("winter"),
        "season missing from '{s}'"
    );
    assert!(s.contains("yr 2"), "year offset missing from '{s}'");
}

#[test]
fn since_hint_defaults_to_yr_1_when_earliest_equals_now() {
    // A just-joined peer still renders at least "yr 1" — spec §Soft time.
    let now = 1_714_000_000_000u64;
    let s = since_hint(now, now);
    assert!(s.ends_with("yr 1"), "expected yr 1, got '{s}'");
}

#[test]
fn profile_delta_default_is_noop_shape() {
    let d = ProfileDelta::default();
    assert!(d.display_name.is_none());
    assert!(d.pronouns.is_none());
    assert!(d.bio.is_none());
    assert!(d.elsewhere.is_none());
}

#[tokio::test]
async fn update_profile_fields_broadcasts_event() {
    // Subscribing directly to the broker and then firing a mutation
    // should yield a ProfileUpdated / DAG-level event — we rely on the
    // full apply path working: this test just checks the mutation call
    // does not error and produces the expected state change.
    let (client, _broker) = test_client();
    let local = client.identity().endpoint_id();
    client
        .mutations()
        .update_profile_fields(ProfileDelta {
            display_name: Some("mira".into()),
            ..ProfileDelta::default()
        })
        .await
        .expect("update_profile_fields must succeed");
    let v = client.views().profile_view_of(&local, &local).await;
    assert_eq!(v.display_name, "mira");
}

#[tokio::test]
async fn shared_groves_empty_when_server_entries_lack_membership() {
    // Today the client tracks member lists separately from
    // `ServerEntry`; the helper therefore returns an empty Vec until
    // the multi-grove plumbing lands. This test pins the contract so
    // callers know to handle the empty case.
    let (client, _broker) = test_client();
    let local = client.identity().endpoint_id();
    let other = willow_identity::Identity::generate().endpoint_id();
    let registry = client.views().server_registry.get().await;
    let shared = registry.shared_groves(&local, &other);
    assert!(shared.is_empty());
}
