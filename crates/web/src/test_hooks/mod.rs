//! Test instrumentation for the Willow web UI.
//!
//! This module is gated behind the `test-hooks` cargo feature and is
//! **never compiled into production builds**. It exposes
//! `WillowTestHooks` to JavaScript via `wasm_bindgen` so Playwright
//! e2e tests can synchronise on real signals (applied events, DAG
//! heads, snapshot fields) instead of arbitrary `waitForTimeout`s.
//!
//! See `docs/specs/2026-04-27-event-based-waits-design.md`.

#![cfg(feature = "test-hooks")]

mod snapshot;
pub use snapshot::{AuthorHeadDto, ChannelDto, SnapshotDto};

mod wire;
pub use wire::{to_wire, WireEvent};

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use willow_actor::{Addr, StateActor};
use willow_client::state_actors::DagState;
use willow_client::ClientHandle;
use willow_network::Network;
use willow_state::ServerState;

/// Read-only test instrumentation handle exposed to JS as `window.__willow`.
///
/// Stores the actor addresses extracted from the `ClientHandle` at
/// construction time so the `#[wasm_bindgen]` struct stays monomorphic.
/// All exposed methods return `js_sys::Promise` — the WASM async runtime
/// drives the actor-ask round-trip rather than blocking on it.
#[wasm_bindgen]
pub struct WillowTestHooks {
    dag_addr: Addr<StateActor<DagState>>,
    state_addr: Addr<StateActor<ServerState>>,
}

impl WillowTestHooks {
    /// Construct from a `ClientHandle` (production path: `app.rs` mount).
    ///
    /// Borrows the handle so callers don't need to `.clone()` just to
    /// construct the hooks; the underlying actor addresses are cloned
    /// internally.
    pub fn new<N: Network + 'static>(handle: &ClientHandle<N>) -> Self {
        Self {
            dag_addr: handle.dag_addr_clone(),
            state_addr: handle.event_state_addr_clone(),
        }
    }

    /// Construct directly from raw actor addresses (test path).
    ///
    /// Bypasses `ClientHandle` entirely so wasm32 browser tests don't
    /// need `MemNetwork` (which depends on `tokio::sync::broadcast`,
    /// native-only).
    pub fn from_actors(
        dag_addr: Addr<StateActor<DagState>>,
        state_addr: Addr<StateActor<ServerState>>,
    ) -> Self {
        Self {
            dag_addr,
            state_addr,
        }
    }
}

#[wasm_bindgen]
impl WillowTestHooks {
    /// Total events applied to the local DAG. Resolves to a `number`.
    ///
    /// Returned as a `Promise` so the underlying actor ask can complete
    /// asynchronously on the WASM cooperative scheduler.
    pub fn event_count(&self) -> js_sys::Promise {
        let addr = self.dag_addr.clone();
        future_to_promise(async move {
            let count =
                willow_actor::state::select(&addr, |ds| ds.managed.dag().len() as u32).await;
            Ok(JsValue::from_f64(count as f64))
        })
    }

    /// Hex-encoded hash of the most recently applied event, or `null`.
    ///
    /// "Most recently applied" is defined as the last element of the
    /// deterministic topological sort of the DAG. Resolves to a hex
    /// `string` or `null`.
    pub fn last_event(&self) -> js_sys::Promise {
        let addr = self.dag_addr.clone();
        future_to_promise(async move {
            let maybe_hash = willow_actor::state::select(&addr, |ds| {
                ds.managed
                    .dag()
                    .topological_sort()
                    .last()
                    .map(|e| e.hash.to_string())
            })
            .await;
            match maybe_hash {
                Some(hex) => Ok(JsValue::from_str(&hex)),
                None => Ok(JsValue::NULL),
            }
        })
    }

    /// Per-author DAG heads, keyed by `EndpointId` hex string. Resolves to
    /// `Record<string, AuthorHead>`.
    pub fn heads(&self) -> js_sys::Promise {
        let addr = self.dag_addr.clone();
        future_to_promise(async move {
            let map: std::collections::BTreeMap<String, snapshot::AuthorHeadDto> =
                willow_actor::state::select(&addr, snapshot::build_heads).await;
            serde_wasm_bindgen::to_value(&map).map_err(Into::into)
        })
    }

    /// Aggregated state snapshot. Resolves to an object matching the spec's
    /// `Snapshot` interface: `{ eventCount, heads, lastEvent, channels }`.
    pub fn snapshot(&self) -> js_sys::Promise {
        let dag_addr = self.dag_addr.clone();
        let state_addr = self.state_addr.clone();
        future_to_promise(async move {
            let snap = snapshot::build(&dag_addr, &state_addr).await;
            serde_wasm_bindgen::to_value(&snap).map_err(Into::into)
        })
    }
}
