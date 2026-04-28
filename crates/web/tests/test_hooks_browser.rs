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
//! The dispatcher tests construct a `Broker<ClientEvent>` directly and
//! subscribe an `EventReceiver` to it. This avoids `ClientHandle` and
//! `MemNetwork` entirely, keeping all tests WASM-compatible.
//!
//! ## What's covered
//! Empty-DAG invariants of the JS-exposed pull API:
//! `event_count()` resolves to `0`, `last_event()` resolves to `null`.
//! Push dispatcher: emit, drop/stop, buffer-drain, overflow signalling.

#![cfg(feature = "test-hooks")]

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use willow_actor::{Broker, Publish, StateActor, System};
use willow_client::event_receiver::EventReceiver;
use willow_client::events::ClientEvent;
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

#[wasm_bindgen_test]
async fn snapshot_returns_empty_dto_on_empty_fixture() {
    let hooks = empty_hooks();
    let p = hooks.snapshot();
    let value = JsFuture::from(p).await.unwrap();
    let snap: willow_web::test_hooks::SnapshotDto =
        serde_wasm_bindgen::from_value(value).expect("deserialize snapshot");

    assert_eq!(snap.event_count, 0, "empty DAG => event_count == 0");
    assert!(snap.heads.is_empty(), "empty DAG => heads map empty");
    assert!(snap.last_event.is_none(), "empty DAG => last_event None");
    assert!(
        snap.channels.is_empty(),
        "empty ServerState => channels empty"
    );
}

// ───── Push-dispatcher tests (Phase 4) ───────────────────────────────────────

/// Build a broker + dispatcher without `ClientHandle` or `MemNetwork`.
///
/// Returns the broker address (for sending test events) and the
/// `DispatcherHandle` (dropping it stops the loop).  The `System` is
/// intentionally forgotten to keep the spawned actors alive for the
/// duration of the test.
async fn fresh_dispatcher_setup() -> (
    willow_actor::Addr<willow_actor::Broker<ClientEvent>>,
    willow_web::test_hooks::DispatcherHandle,
) {
    let sys = System::new();
    let broker_addr = sys.spawn(Broker::<ClientEvent>::default());
    let rx = EventReceiver::subscribe(&broker_addr, &sys.handle()).await;
    let dispatcher = willow_web::test_hooks::install_push_dispatcher(rx);
    // Keep the system (and its actors) alive for the test.
    std::mem::forget(sys);
    (broker_addr, dispatcher)
}

#[wasm_bindgen_test]
async fn dispatcher_emits_sync_completed_to_window_callback() {
    let captured: Rc<RefCell<Vec<JsValue>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_clone = captured.clone();

    let cb = Closure::wrap(Box::new(move |ev: JsValue| {
        captured_clone.borrow_mut().push(ev);
    }) as Box<dyn FnMut(JsValue)>);

    let window = web_sys::window().unwrap();
    js_sys::Reflect::set(
        &window,
        &"__willowEvent".into(),
        cb.as_ref().unchecked_ref(),
    )
    .unwrap();
    cb.forget();

    let (broker_addr, _dispatcher) = fresh_dispatcher_setup().await;

    // Send a SyncCompleted event through the broker.
    broker_addr
        .do_send(Publish(ClientEvent::SyncCompleted { ops_applied: 5 }))
        .unwrap();

    // Yield to let the dispatcher loop run.
    gloo_timers::future::TimeoutFuture::new(50).await;

    let events = captured.borrow();
    assert!(
        events.iter().any(|ev| {
            let s = js_sys::JSON::stringify(ev)
                .ok()
                .and_then(|js| js.as_string())
                .unwrap_or_default();
            s.contains(r#""kind":"SyncCompleted""#)
        }),
        "expected at least one SyncCompleted event; got {:?}",
        events
            .iter()
            .map(|ev| js_sys::JSON::stringify(ev)
                .ok()
                .and_then(|js| js.as_string())
                .unwrap_or_default())
            .collect::<Vec<_>>()
    );

    // Cleanup.
    js_sys::Reflect::delete_property(&window, &"__willowEvent".into()).unwrap();
}

#[wasm_bindgen_test]
async fn dropping_dispatcher_handle_stops_emissions() {
    let captured: Rc<RefCell<Vec<JsValue>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_clone = captured.clone();

    let cb = Closure::wrap(Box::new(move |ev: JsValue| {
        captured_clone.borrow_mut().push(ev);
    }) as Box<dyn FnMut(JsValue)>);

    let window = web_sys::window().unwrap();
    js_sys::Reflect::set(
        &window,
        &"__willowEvent".into(),
        cb.as_ref().unchecked_ref(),
    )
    .unwrap();
    cb.forget();

    let broker_addr = {
        let (broker_addr, _dispatcher) = fresh_dispatcher_setup().await;

        broker_addr
            .do_send(Publish(ClientEvent::SyncCompleted { ops_applied: 1 }))
            .unwrap();
        gloo_timers::future::TimeoutFuture::new(50).await;

        broker_addr
        // _dispatcher dropped here, abort flag set to true.
    };

    let count_after_drop = captured.borrow().len();

    // Send a second event after the handle is dropped.
    broker_addr
        .do_send(Publish(ClientEvent::SyncCompleted { ops_applied: 2 }))
        .unwrap();
    gloo_timers::future::TimeoutFuture::new(50).await;

    let count_after_post_drop_event = captured.borrow().len();

    assert!(
        count_after_post_drop_event <= count_after_drop + 1,
        "dispatcher should not deliver events after handle drop \
         (got {count_after_post_drop_event} after drop, was {count_after_drop} at drop)"
    );

    js_sys::Reflect::delete_property(&window, &"__willowEvent".into()).unwrap();
}

#[wasm_bindgen_test]
async fn buffer_drains_on_first_dispatch_after_binding_appears() {
    let window = web_sys::window().unwrap();

    // Remove any stale callback so dispatch_or_buffer goes to the buffer path.
    js_sys::Reflect::delete_property(&window, &"__willowEvent".into()).unwrap();

    // Pre-seed the buffer as if a dispatcher had run before the
    // binding existed.
    let pre_buffer = js_sys::Array::new();
    pre_buffer.push(&JsValue::from_str("PREEXISTING"));
    js_sys::Reflect::set(&window, &"__willowEventBuffer".into(), &pre_buffer).unwrap();

    let captured: Rc<RefCell<Vec<JsValue>>> = Rc::new(RefCell::new(Vec::new()));
    let captured_clone = captured.clone();

    let cb = Closure::wrap(Box::new(move |ev: JsValue| {
        captured_clone.borrow_mut().push(ev);
    }) as Box<dyn FnMut(JsValue)>);
    js_sys::Reflect::set(
        &window,
        &"__willowEvent".into(),
        cb.as_ref().unchecked_ref(),
    )
    .unwrap();
    cb.forget();

    let (broker_addr, _dispatcher) = fresh_dispatcher_setup().await;

    // Send a real event — this triggers the per-dispatch drain of the pre-seeded buffer.
    broker_addr
        .do_send(Publish(ClientEvent::SyncCompleted { ops_applied: 7 }))
        .unwrap();
    gloo_timers::future::TimeoutFuture::new(50).await;

    let events = captured.borrow();
    let strs: Vec<String> = events
        .iter()
        .map(|ev| ev.as_string().unwrap_or_default())
        .collect();
    assert!(
        strs.contains(&"PREEXISTING".to_string()),
        "buffered pre-existing event should be drained; got {strs:?}"
    );

    js_sys::Reflect::delete_property(&window, &"__willowEvent".into()).unwrap();
    js_sys::Reflect::delete_property(&window, &"__willowEventBuffer".into()).unwrap();
}

#[wasm_bindgen_test]
async fn buffer_overflow_calls_willow_overflow_callback() {
    let window = web_sys::window().unwrap();

    // Set up the overflow hook first.
    let overflow_count: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
    let overflow_clone = overflow_count.clone();
    let overflow_cb = Closure::wrap(Box::new(move |dropped: f64| {
        *overflow_clone.borrow_mut() += dropped as u32;
    }) as Box<dyn FnMut(f64)>);
    js_sys::Reflect::set(
        &window,
        &"__willowOverflow".into(),
        overflow_cb.as_ref().unchecked_ref(),
    )
    .unwrap();
    overflow_cb.forget();

    // Pre-fill the buffer to capacity (65_536 entries) so the next push overflows.
    let pre_buffer = js_sys::Array::new();
    for i in 0..65_536u32 {
        pre_buffer.push(&JsValue::from_f64(i as f64));
    }
    js_sys::Reflect::set(&window, &"__willowEventBuffer".into(), &pre_buffer).unwrap();

    // Do NOT bind __willowEvent — we want push_into_buffer to be the path under test.
    js_sys::Reflect::delete_property(&window, &"__willowEvent".into()).unwrap();

    let (broker_addr, _dispatcher) = fresh_dispatcher_setup().await;

    // Triggering a new event causes the dispatcher to push into a full buffer.
    broker_addr
        .do_send(Publish(ClientEvent::SyncCompleted { ops_applied: 99 }))
        .unwrap();
    gloo_timers::future::TimeoutFuture::new(50).await;

    assert!(
        *overflow_count.borrow() >= 1,
        "expected at least one overflow signal, got {}",
        *overflow_count.borrow()
    );

    js_sys::Reflect::delete_property(&window, &"__willowOverflow".into()).unwrap();
    js_sys::Reflect::delete_property(&window, &"__willowEventBuffer".into()).unwrap();
}
