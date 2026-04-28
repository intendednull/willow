//! Push dispatcher for `WillowTestHooks`.
//!
//! Subscribes to a [`willow_client::EventReceiver`] and forwards each
//! wire-visible [`ClientEvent`] to `window.__willowEvent` (a Playwright
//! `exposeBinding`). On overflow calls `window.__willowOverflow(droppedCount)`
//! so the test fixture can fail the test immediately.
//!
//! When the binding is absent, events are buffered in
//! `window.__willowEventBuffer` (capacity 65,536). The dispatcher performs
//! a three-edge drain:
//!
//! 1. **Init drain** — on `install_push_dispatcher`, drain any buffer left
//!    by a prior dispatcher (hot reload, auth re-init).
//! 2. **Per-dispatch drain** — before forwarding each new event, drain the
//!    buffer so events arrive in order once the binding appears.
//! 3. **Read-side drain** — handled by the Playwright fixture (JS-only).

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use willow_client::EventReceiver;

use super::wire::to_wire;

const BUFFER_CAPACITY: usize = 65_536;

/// Returned from [`install_push_dispatcher`]. Dropping aborts the dispatch loop.
pub struct DispatcherHandle {
    abort: Rc<RefCell<bool>>,
}

impl Drop for DispatcherHandle {
    fn drop(&mut self) {
        *self.abort.borrow_mut() = true;
    }
}

/// Install the push dispatcher.
///
/// Spawns a `wasm_bindgen_futures` task that loops on the broker `recv()`,
/// converts each [`ClientEvent`] to its wire shape, and forwards to
/// `window.__willowEvent`.
///
/// Returns a [`DispatcherHandle`] — dropping it stops the dispatch loop.
pub fn install_push_dispatcher(mut rx: EventReceiver) -> DispatcherHandle {
    let abort = Rc::new(RefCell::new(false));
    let abort_clone = abort.clone();

    spawn_local(async move {
        // Edge 1: drain on dispatcher init — covers prior-dispatcher buffer leftovers.
        drain_buffer_into_callback();

        while !*abort_clone.borrow() {
            let Some(event) = rx.recv().await else { break };
            let Some(wire) = to_wire(&event) else {
                continue;
            };

            let js = match serde_wasm_bindgen::to_value(&wire) {
                Ok(v) => v,
                Err(e) => {
                    web_sys::console::error_1(
                        &format!("test-hooks: serialize failed: {e:?}").into(),
                    );
                    continue;
                }
            };

            // Edge 2: drain on every dispatch — covers binding-becomes-available case.
            drain_buffer_into_callback();
            dispatch_or_buffer(js);
        }
    });

    DispatcherHandle { abort }
}

fn dispatch_or_buffer(js: JsValue) {
    let Some(window) = web_sys::window() else {
        return;
    };

    if let Ok(callback) = js_sys::Reflect::get(&window, &"__willowEvent".into()) {
        if let Some(func) = callback.dyn_ref::<js_sys::Function>() {
            if let Err(e) = func.call1(&JsValue::NULL, &js) {
                web_sys::console::warn_1(&format!("test-hooks: __willowEvent threw: {e:?}").into());
            }
            return;
        }
    }

    push_into_buffer(&window, js);
}

fn drain_buffer_into_callback() {
    let Some(window) = web_sys::window() else {
        return;
    };

    let Ok(callback) = js_sys::Reflect::get(&window, &"__willowEvent".into()) else {
        return;
    };
    let Some(func) = callback.dyn_ref::<js_sys::Function>() else {
        return;
    };

    let Ok(buffer) = js_sys::Reflect::get(&window, &"__willowEventBuffer".into()) else {
        return;
    };
    let Some(arr) = buffer.dyn_ref::<js_sys::Array>() else {
        return;
    };

    while arr.length() > 0 {
        let item = arr.shift();
        if let Err(e) = func.call1(&JsValue::NULL, &item) {
            web_sys::console::warn_1(
                &format!("test-hooks: __willowEvent (drain) threw: {e:?}").into(),
            );
        }
    }
}

fn push_into_buffer(window: &web_sys::Window, js: JsValue) {
    let buffer = match js_sys::Reflect::get(window, &"__willowEventBuffer".into()) {
        Ok(b) if b.is_object() && b.dyn_ref::<js_sys::Array>().is_some() => b,
        _ => {
            let arr = js_sys::Array::new();
            let _ = js_sys::Reflect::set(window, &"__willowEventBuffer".into(), &arr);
            arr.into()
        }
    };

    let arr: js_sys::Array = buffer.unchecked_into();

    if arr.length() as usize >= BUFFER_CAPACITY {
        // Overflow: drop oldest, signal the test fixture.
        arr.shift();
        signal_overflow(window, 1);
    }

    arr.push(&js);
}

fn signal_overflow(window: &web_sys::Window, dropped: u32) {
    if let Ok(cb) = js_sys::Reflect::get(window, &"__willowOverflow".into()) {
        if let Some(func) = cb.dyn_ref::<js_sys::Function>() {
            let _ = func.call1(&JsValue::NULL, &JsValue::from_f64(dropped as f64));
        }
    }
    web_sys::console::error_1(
        &format!("test-hooks: __willow buffer overflow ({dropped} dropped)").into(),
    );
}
