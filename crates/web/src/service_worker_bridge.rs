//! Service-worker `postMessage` bridge.
//!
//! `sw.js` forwards push payloads to focused clients via
//! `client.postMessage({ kind, cat, ref })`. The boundary is untrusted
//! by default — anything with a handle to the `ServiceWorker` (or, in
//! future, a `MessageChannel` peer) can call `postMessage`. The bridge
//! enforces two defenses:
//!
//! 1. **Kind discriminator.** `ev.data.kind` must equal one of the
//!    constants below. Messages without a recognized kind are dropped
//!    silently (no logging — this is hot-path on every push).
//! 2. **No global window stash.** The previous implementation parked
//!    the payload on `window.__willowLastPush`, where any script in
//!    the page could read it. We now keep the payload in a
//!    module-local `RefCell` (single-threaded WASM, see
//!    `CLAUDE.md` State Management table — "WASM single-threaded
//!    interior mutability").
//!
//! After validation, the bridge stashes the payload and dispatches a
//! plain `willow-push` window event. The Leptos handler in `app.rs`
//! listens for that event and pulls the payload via
//! [`take_last_push`].
//!
//! The custom-event-free path (plain `Event`, not `CustomEvent`)
//! sidesteps a `web-sys` feature dependency — see the original
//! comment in `main.rs`.

use std::cell::RefCell;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

/// `kind` value the SW sets on a forwarded push payload.
pub const PUSH_KIND: &str = "willow-push";

/// `kind` value the SW sets on a notification-click forward.
pub const NOTIFICATION_CLICK_KIND: &str = "willow-notification-click";

/// Window event name dispatched after a push payload is accepted.
pub const PUSH_EVENT: &str = "willow-push";

/// Validated payload stashed for the Leptos reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushPayload {
    /// Always equals one of [`PUSH_KIND`] / [`NOTIFICATION_CLICK_KIND`].
    pub kind: String,
    /// Notification category (e.g. `"msg"`, `"mention"`).
    pub cat: String,
    /// Optional opaque reference (channel id, message id, …).
    pub reference: Option<String>,
}

thread_local! {
    /// Last validated payload received from the SW. Replaced — not
    /// queued — on each push, mirroring the prior single-slot
    /// behavior. WASM is single-threaded so a plain `RefCell` is
    /// sufficient (see `CLAUDE.md` State Management table).
    static LAST_PUSH: RefCell<Option<PushPayload>> = const { RefCell::new(None) };
}

/// Pull and clear the last validated push payload, if any.
///
/// Returns `None` when no payload has arrived since the previous
/// call. Intended for the Leptos `willow-push` listener.
pub fn take_last_push() -> Option<PushPayload> {
    LAST_PUSH.with(|cell| cell.borrow_mut().take())
}

/// Validate a raw `MessageEvent.data` value and return a
/// [`PushPayload`] iff it carries a recognized `kind`.
///
/// Pulled out of the closure for direct unit-test coverage — driving
/// the full SW path in a headless browser is not feasible.
pub fn validate_payload(data: &wasm_bindgen::JsValue) -> Option<PushPayload> {
    if !data.is_object() {
        return None;
    }
    let kind = js_sys::Reflect::get(data, &"kind".into())
        .ok()
        .and_then(|v| v.as_string())?;
    if kind != PUSH_KIND && kind != NOTIFICATION_CLICK_KIND {
        return None;
    }
    let cat = js_sys::Reflect::get(data, &"cat".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "msg".to_string());
    let reference = js_sys::Reflect::get(data, &"ref".into())
        .ok()
        .and_then(|v| v.as_string());
    Some(PushPayload {
        kind,
        cat,
        reference,
    })
}

/// Install the `navigator.serviceWorker.onmessage` listener.
///
/// Validates each incoming `MessageEvent` against [`validate_payload`],
/// stashes the result in the module-local cell, and dispatches a
/// `willow-push` window event so the in-app Notifier can react.
///
/// Idempotent only at the call-site level — calling twice attaches two
/// listeners. Production calls it exactly once from `main.rs`.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let sw = window.navigator().service_worker();
    let window_for_dispatch = window.clone();
    let onmessage =
        Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
            let Some(payload) = validate_payload(&ev.data()) else {
                return;
            };
            store_and_dispatch(&window_for_dispatch, payload);
        });
    let _ = sw.add_event_listener_with_callback("message", onmessage.as_ref().unchecked_ref());
    onmessage.forget();
}

/// Stash `payload` and fire the `willow-push` window event.
///
/// Split out so browser tests can drive the post-validation path
/// without faking a `ServiceWorker` (which is unavailable under
/// `wasm-pack test`).
pub fn store_and_dispatch(window: &web_sys::Window, payload: PushPayload) {
    LAST_PUSH.with(|cell| *cell.borrow_mut() = Some(payload));
    if let Ok(evt) = web_sys::Event::new(PUSH_EVENT) {
        let _ = window.dispatch_event(&evt);
    }
}
