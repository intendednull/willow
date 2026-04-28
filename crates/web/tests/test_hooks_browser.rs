//! In-browser tests for `WillowTestHooks`.
//!
//! Run with:
//!   wasm-pack test crates/web --headless --chrome --features test-hooks --test test_hooks_browser
//!
//! These tests construct a `StateActor<DagState>` directly (no networking)
//! and assert the test-hooks pull API observes the expected shape after
//! known events have been applied.
//!
//! ## Why no `MemNetwork`?
//! `MemNetwork` uses `tokio::sync::broadcast` which is native-only.
//! These tests therefore construct just the DAG actor they need and hand
//! it to `WillowTestHooks::from_dag_addr` — no `ClientHandle` required.

#![cfg(feature = "test-hooks")]

use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use willow_web::test_hooks::WillowTestHooks;

wasm_bindgen_test_configure!(run_in_browser);

/// Construct a `WillowTestHooks` backed by a fresh DAG seeded with one
/// `CreateServer` genesis event (via `ManagedDag::new`).
fn make_hooks() -> WillowTestHooks {
    let identity = willow_identity::Identity::generate();
    let managed = willow_state::ManagedDag::new(&identity, "test-server", 5_000).expect("genesis");
    let dag_state = willow_client::state_actors::DagState {
        managed,
        stashed: std::collections::HashMap::new(),
    };
    let sys = willow_actor::System::new();
    let dag_addr = sys.spawn(willow_actor::StateActor::new(dag_state));
    // Forget the System to keep the spawned actors alive for the test.
    std::mem::forget(sys);
    WillowTestHooks::from_dag_addr(dag_addr)
}

#[wasm_bindgen_test]
async fn snapshot_event_count_and_last_event_after_create_server() {
    let hooks = make_hooks();

    // event_count() resolves to a JS number.
    let count_js: JsValue = JsFuture::from(hooks.event_count())
        .await
        .expect("event_count");
    let count = count_js.as_f64().expect("event_count is a number") as u32;

    assert_eq!(count, 1, "CreateServer should be event #1");

    // last_event() resolves to a non-null JS string when the DAG is non-empty.
    let last_js: JsValue = JsFuture::from(hooks.last_event())
        .await
        .expect("last_event");

    assert!(
        !last_js.is_null() && !last_js.is_undefined(),
        "last_event should be Some after CreateServer"
    );
}
