//! DTOs for the `WillowTestHooks` pull API.
//!
//! These mirror the TypeScript `Snapshot` / `AuthorHead` types defined
//! in `e2e/test-hooks.ts`. Field names are camelCase to match TS
//! convention; the kind discriminator on `ClientEvent` (a separate
//! module) stays PascalCase.

use serde::Serialize;

/// One author's DAG head, as exposed to JS.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorHeadDto {
    pub seq: u64,
    pub hash: String,
}

/// One channel's summary, as exposed to JS.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDto {
    pub name: String,
    pub member_count: u32,
}

/// Aggregated state snapshot for `expect.poll` matchers.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    pub event_count: u32,
    /// Per-author DAG heads, keyed by `EndpointId` hex string.
    pub heads: std::collections::BTreeMap<String, AuthorHeadDto>,
    pub last_event: Option<String>,
    pub channels: Vec<ChannelDto>,
}

// ── Builder functions (Phase 2.6+) ──────────────────────────────────────

use willow_client::ClientHandle;
use willow_network::Network;

/// Build a full `SnapshotDto` from a `ClientHandle`.
///
/// All underlying reads are async (actor round-trips). Called by the
/// `snapshot()` method on `WillowTestHooks` (added in Phase 2.6).
pub(crate) async fn build<N: Network>(handle: &ClientHandle<N>) -> SnapshotDto {
    let event_count = handle.dag_event_count().await as u32;
    let heads = build_heads(handle).await;
    let last_event = handle.dag_last_event_hash().await.map(|h| h.to_string());
    let channels = handle
        .channels()
        .await
        .into_iter()
        .map(|name| ChannelDto {
            name,
            member_count: 0, // Phase 2.6: wire real member count
        })
        .collect();
    SnapshotDto {
        event_count,
        heads,
        last_event,
        channels,
    }
}

/// Build the per-author heads map from a `ClientHandle`.
///
/// Keys are `EndpointId` hex strings; values are `AuthorHeadDto { seq, hash }`.
pub(crate) async fn build_heads<N: Network>(
    handle: &ClientHandle<N>,
) -> std::collections::BTreeMap<String, AuthorHeadDto> {
    let summary = handle.dag_heads_summary().await;
    summary
        .heads
        .into_iter()
        .map(|(endpoint, head)| {
            (
                endpoint.to_string(),
                AuthorHeadDto {
                    seq: head.seq,
                    hash: head.hash.to_string(),
                },
            )
        })
        .collect()
}
