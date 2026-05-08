//! Phase 2d — ephemeral channel client API tests.
//!
//! Exercises [`ClientHandle::create_ephemeral_channel`] and
//! [`ClientHandle::revive_channel`] against the in-memory
//! `test_client` harness. No networking — the tests build mutations
//! through the actor pipeline and inspect the materialized
//! `ServerState` via [`ClientHandle::state_snapshot`].

use crate::test_client;
use willow_state::{EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS, IDLE_THRESHOLD_MIN_MS};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_ephemeral_channel_records_config() {
    let (client, _rx) = test_client();
    client
        .create_ephemeral_channel(
            "side-room",
            EphemeralKind::Channel,
            DEFAULT_CHANNEL_THRESHOLD_MS,
        )
        .await
        .unwrap();
    // Give actors a moment to apply.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let state = client.state_snapshot().await;
    let ch = state
        .channels
        .values()
        .find(|c| c.name == "side-room")
        .expect("side-room channel must exist");
    assert!(ch.ephemeral.is_some(), "ephemeral config must be recorded");
    assert_eq!(
        ch.ephemeral.as_ref().unwrap().idle_threshold_ms,
        DEFAULT_CHANNEL_THRESHOLD_MS
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn archives_view_lists_archived_ephemerals_only() {
    let (client, _rx) = test_client();
    // Channel that will archive immediately because we'll fast-forward
    // the frontier past its last-activity + threshold.
    client
        .create_ephemeral_channel("expired", EphemeralKind::Channel, IDLE_THRESHOLD_MIN_MS)
        .await
        .unwrap();
    // Permanent channel — must not appear in archives.
    client.create_channel("general").await.unwrap();
    // Active ephemeral with a far-future threshold — must not archive.
    client
        .create_ephemeral_channel(
            "active",
            EphemeralKind::Channel,
            DEFAULT_CHANNEL_THRESHOLD_MS,
        )
        .await
        .unwrap();
    // Touch each ephemeral channel so they all have a known
    // last_activity_hlc anchored near "now". A channel without any
    // activity defaults to 0 and would be archived past any frontier
    // > its threshold — but that's correct behaviour, not what this
    // test exercises (we want to verify that *only* the expired
    // channel surfaces given a frontier in the gap between the two
    // thresholds).
    client.send_message("expired", "seed").await.unwrap();
    client.send_message("active", "seed").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let snap = client.state_snapshot().await;
    let expired = snap
        .channels
        .values()
        .find(|c| c.name == "expired")
        .expect("expired channel must exist");
    let last = expired
        .last_activity_hlc
        .expect("expired must have last_activity_hlc set after send");

    // Frontier = last + threshold + 1 ms — this drives `expired` past
    // its idle threshold. `active` has a much larger threshold so it
    // stays active. `general` has no ephemeral config so it never shows.
    let frontier = last.saturating_add(IDLE_THRESHOLD_MIN_MS).saturating_add(1);
    let view = client.archives_view_at(frontier).await;
    let names: Vec<&str> = view.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["expired"],
        "only `expired` should appear in archives at frontier {frontier}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revive_channel_advances_last_activity_hlc() {
    let (client, _rx) = test_client();
    client
        .create_ephemeral_channel(
            "side-room",
            EphemeralKind::Channel,
            DEFAULT_CHANNEL_THRESHOLD_MS,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Capture HLC, revive, confirm advance.
    let before = {
        let state = client.state_snapshot().await;
        state
            .channels
            .values()
            .find(|c| c.name == "side-room")
            .unwrap()
            .last_activity_hlc
    };
    client.revive_channel("side-room").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let after = {
        let state = client.state_snapshot().await;
        state
            .channels
            .values()
            .find(|c| c.name == "side-room")
            .unwrap()
            .last_activity_hlc
    };
    // before may be None (creation alone doesn't advance the HLC);
    // after must be Some(_) because revive sets it.
    assert!(
        after.is_some(),
        "revive must set last_activity_hlc, got {after:?}"
    );
    if let (Some(b), Some(a)) = (before, after) {
        assert!(a >= b, "revive must not move HLC backwards: {b} -> {a}");
    }
}
