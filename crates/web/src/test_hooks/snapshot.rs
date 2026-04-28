//! DTOs for the `WillowTestHooks` pull API.
//!
//! These mirror the TypeScript `Snapshot` / `AuthorHead` types defined
//! in `e2e/test-hooks.ts`. Field names are camelCase to match TS
//! convention; the kind discriminator on `ClientEvent` (a separate
//! module) stays PascalCase.

use serde::{Deserialize, Serialize};
use willow_state::ChannelKind;

/// One author's DAG head, as exposed to JS.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorHeadDto {
    pub seq: u64,
    pub hash: String,
}

/// One channel's summary, as exposed to JS.
///
/// `kind` is forwarded directly through `ChannelKind`'s own `Serialize`
/// impl so the wire form (`"Text"` / `"Voice"`) is contracted by the
/// state crate, not by `Debug` formatting.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDto {
    pub name: String,
    pub kind: ChannelKind,
}

/// Aggregated state snapshot for `expect.poll` matchers.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDto {
    pub event_count: u32,
    /// Per-author DAG heads, keyed by `EndpointId` hex string.
    pub heads: std::collections::BTreeMap<String, AuthorHeadDto>,
    pub last_event: Option<String>,
    pub channels: Vec<ChannelDto>,
}

// ── Builder functions (Phase 2.5 / 2.7 wiring) ──────────────────────────
//
// These take raw actor addresses (`Addr<StateActor<DagState>>` etc.) so
// they don't depend on the generic `ClientHandle<N>` — the wasm_bindgen
// boundary in `WillowTestHooks` is monomorphic. Reads go through
// `willow_actor::state::select` (the standard async ask path).
//
// The `build` and `build_heads` functions are not yet wired into the
// JS-exposed `heads()` / `snapshot()` methods — those land in Tasks 2.5
// / 2.7 of the PR-1 plan. They compile here as `pub(crate)` stubs so
// the surrounding scaffolding stays consistent.

use std::collections::BTreeMap;
use willow_actor::{Addr, StateActor};
use willow_client::state_actors::DagState;
use willow_state::ServerState;

/// Build the per-author heads map from the in-memory `DagState`.
///
/// Synchronous helper invoked from inside a `state::select` closure on
/// the DAG actor. Keys are `EndpointId` hex strings; values are
/// `AuthorHeadDto { seq, hash }`.
pub(crate) fn build_heads(ds: &DagState) -> BTreeMap<String, AuthorHeadDto> {
    ds.managed
        .dag()
        .heads_summary()
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

/// Build a full `SnapshotDto` by reading both the DAG actor and the
/// materialised `ServerState` actor.
///
/// Two actor-asks (cheap, sub-ms each on local mailbox dispatch). The
/// snapshot is consistent within each ask but not across the pair —
/// the gap is acceptable for `expect.poll`-style tests, which retry
/// until the predicate stabilises.
pub(crate) async fn build(
    dag_addr: &Addr<StateActor<DagState>>,
    state_addr: &Addr<StateActor<ServerState>>,
) -> SnapshotDto {
    let (event_count, heads, last_event) = willow_actor::state::select(dag_addr, |ds| {
        (
            ds.managed.dag().len() as u32,
            build_heads(ds),
            ds.managed
                .dag()
                .topological_sort()
                .last()
                .map(|e| e.hash.to_string()),
        )
    })
    .await;
    let channels = willow_actor::state::select(state_addr, |ss| {
        ss.channels
            .values()
            .map(|ch| ChannelDto {
                name: ch.name.clone(),
                kind: ch.kind.clone(),
            })
            .collect::<Vec<_>>()
    })
    .await;
    SnapshotDto {
        event_count,
        heads,
        last_event,
        channels,
    }
}
