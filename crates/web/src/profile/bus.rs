//! Window-level CustomEvent bus for opening / closing the profile card.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md` §Event-bus API.
//!
//! Any avatar surface calls [`open_profile`] with the clicked user id
//! and the anchor element. The global controller (mounted once at app
//! root) subscribes to the window and decides which wrapper renders.
//!
//! Payload shape: `detail = { user_id: string, anchor?: HTMLElement }`.
//! The `anchor` is required on desktop (positioning); mobile ignores it.

use wasm_bindgen::prelude::*;
use web_sys::{CustomEvent, CustomEventInit, HtmlElement};

pub const PROFILE_OPEN_EVENT: &str = "willow:profile:open";
pub const PROFILE_CLOSE_EVENT: &str = "willow:profile:close";

/// Dispatch a request to open the profile card for `user_id`.
///
/// Safe to call from any component's click handler. No-op outside a
/// browser context (native tests, SSR).
pub fn open_profile(user_id: &str, anchor: Option<HtmlElement>) {
    let Some(win) = web_sys::window() else { return };
    let detail = js_sys::Object::new();
    js_sys::Reflect::set(&detail, &"user_id".into(), &JsValue::from_str(user_id)).ok();
    if let Some(a) = anchor {
        js_sys::Reflect::set(&detail, &"anchor".into(), a.as_ref()).ok();
    }
    let init = CustomEventInit::new();
    init.set_detail(&detail);
    let Ok(ev) = CustomEvent::new_with_event_init_dict(PROFILE_OPEN_EVENT, &init) else {
        return;
    };
    win.dispatch_event(&ev).ok();
}

/// Dispatch a request to close the profile card.
pub fn close_profile() {
    let Some(win) = web_sys::window() else { return };
    let Ok(ev) = CustomEvent::new(PROFILE_CLOSE_EVENT) else {
        return;
    };
    win.dispatch_event(&ev).ok();
}
