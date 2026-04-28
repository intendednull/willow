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

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use willow_client::state_actors::DagState;
use willow_client::ClientHandle;
use willow_network::Network;

/// Type alias for the DAG actor address, kept here so the `#[wasm_bindgen]`
/// struct stays monomorphic.
type DagAddr = willow_actor::Addr<willow_actor::StateActor<DagState>>;

/// Read-only test instrumentation handle exposed to JS as `window.__willow`.
///
/// Generic-free by storing only the `DagAddr` extracted from the
/// `ClientHandle` at construction time. All `#[wasm_bindgen]`-exposed
/// methods return `js_sys::Promise` so the WASM async runtime (rather than
/// a blocking call) drives the actor-ask round-trip.
#[wasm_bindgen]
pub struct WillowTestHooks {
    dag_addr: DagAddr,
}

impl WillowTestHooks {
    /// Construct from any `ClientHandle<N>`. Captures the DAG actor address
    /// so the wasm_bindgen-exposed methods stay monomorphic.
    pub fn new<N: Network + 'static>(handle: ClientHandle<N>) -> Self {
        Self {
            dag_addr: handle.dag_addr_clone(),
        }
    }

    /// Construct directly from a DAG actor address.
    ///
    /// Useful in WASM browser tests where `MemNetwork` is unavailable
    /// (it depends on `tokio::sync::broadcast`, which is native-only).
    /// Tests that only exercise the DAG read-path can construct a
    /// `StateActor<DagState>` directly without a full `ClientHandle`.
    pub fn from_dag_addr(dag_addr: DagAddr) -> Self {
        Self { dag_addr }
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
}
