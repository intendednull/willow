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
