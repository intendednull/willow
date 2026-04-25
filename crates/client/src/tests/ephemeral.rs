//! Phase 2d — ephemeral channel client API tests.
//!
//! Exercises [`ClientHandle::create_ephemeral_channel`] and
//! [`ClientHandle::revive_channel`] against the in-memory
//! `test_client` harness. No networking — the tests build mutations
//! through the actor pipeline and inspect the materialized
//! `ServerState` via [`ClientHandle::state_snapshot`].

use crate::test_client;
use willow_state::{EphemeralKind, DEFAULT_CHANNEL_THRESHOLD_MS};

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
