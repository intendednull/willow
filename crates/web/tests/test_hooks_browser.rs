//! In-browser tests for `WillowTestHooks`.
//!
//! Run with:
//!   wasm-pack test crates/web --headless --chrome --features test-hooks --test test_hooks_browser
//!
//! ## Why no `MemNetwork`?
//! `MemNetwork` uses `tokio::sync::broadcast` which is native-only.
//! These tests therefore construct just the actor addresses they need
//! (`StateActor<DagState>` + `StateActor<ServerState>`) and hand them
//! to `WillowTestHooks::from_actors` — no `ClientHandle` required.
//!
//! ## What's covered
//! Empty-DAG invariants of the JS-exposed pull API:
//! `event_count()` resolves to `0`, `last_event()` resolves to `null`.
//! Non-empty fixtures (after `append_local`) land in subsequent tasks.

#![cfg(feature = "test-hooks")]

use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use willow_actor::{StateActor, System};
use willow_client::state_actors::DagState;
use willow_identity::Identity;
use willow_state::ServerState;
use willow_web::test_hooks::WillowTestHooks;

wasm_bindgen_test_configure!(run_in_browser);

/// Construct a `WillowTestHooks` instance backed by empty actor state.
///
/// The `DagState` uses `ManagedDag::empty(...)` (no genesis seeded),
/// and the `ServerState` is a freshly-constructed shell with a
/// throwaway endpoint id. Neither carries any events.
fn empty_hooks() -> WillowTestHooks {
    let sys = System::new();
    let dag_addr = sys.spawn(StateActor::new(DagState::default()));
    let throwaway = Identity::generate().endpoint_id();
    let state_addr = sys.spawn(StateActor::new(ServerState::new("test", "Test", throwaway)));
    // Forget the System to keep the spawned actors alive for the test.
    std::mem::forget(sys);
    WillowTestHooks::from_actors(dag_addr, state_addr)
}

#[wasm_bindgen_test]
async fn empty_hooks_event_count_is_zero() {
    let hooks = empty_hooks();

    let count_js: JsValue = JsFuture::from(hooks.event_count())
        .await
        .expect("event_count");
    let count = count_js.as_f64().expect("event_count is a number") as u32;

    assert_eq!(count, 0, "empty DAG should have event_count = 0");
}

#[wasm_bindgen_test]
async fn empty_hooks_last_event_is_null() {
    let hooks = empty_hooks();

    let last_js: JsValue = JsFuture::from(hooks.last_event())
        .await
        .expect("last_event");

    assert!(
        last_js.is_null(),
        "last_event on empty DAG must be null, got {last_js:?}"
    );
}

#[wasm_bindgen_test]
async fn heads_returns_empty_map_on_empty_dag() {
    let hooks = empty_hooks();
    let p = hooks.heads();
    let value = JsFuture::from(p).await.unwrap();
    let map: std::collections::BTreeMap<String, willow_web::test_hooks::AuthorHeadDto> =
        serde_wasm_bindgen::from_value(value).expect("deserialize heads");
    assert!(
        map.is_empty(),
        "empty DAG must produce empty heads map; got {:?}",
        map.keys().collect::<Vec<_>>()
    );
}
