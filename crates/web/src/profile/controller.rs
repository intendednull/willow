//! Global controller signal for the profile card.
//!
//! Subscribes to `PROFILE_OPEN_EVENT` / `PROFILE_CLOSE_EVENT` at the
//! window level and exposes a read/write handle on
//! `AppState::profile.open`. Owns the Escape keydown listener and
//! debounces repeat opens for the same user id.

use std::sync::Arc;

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{CustomEvent, HtmlElement};
use willow_client::ProfileView;

use super::bus::{PROFILE_CLOSE_EVENT, PROFILE_OPEN_EVENT};

/// Shape surfaced to the two wrappers (popover + sheet).
#[derive(Clone)]
pub struct ProfileState {
    /// Merged profile payload built by
    /// [`willow_client::views::ClientViewHandle::profile_view_of`].
    pub view: Arc<ProfileView>,
    /// Anchor element (desktop-only). `None` on mobile.
    pub anchor: Option<send_wrapper::SendWrapper<HtmlElement>>,
}

impl PartialEq for ProfileState {
    fn eq(&self, other: &Self) -> bool {
        // Two states match iff they reference the same user. Comparing
        // by peer id keeps the signal de-dup cheap — the UI doesn't
        // need to re-render when the anchor rect shifts by a pixel.
        self.view.peer_id == other.view.peer_id
    }
}

/// Hook returning the read + write handles on the controller signal.
///
/// Calls [`install_listeners_once`] the first time it runs inside the
/// current document so window listeners are never stacked.
pub fn use_profile_controller() -> (
    ReadSignal<Option<ProfileState>>,
    WriteSignal<Option<ProfileState>>,
) {
    let app_state = use_context::<crate::state::AppState>().expect("AppState in context");
    let write = use_context::<crate::state::AppWriteSignals>().expect("AppWriteSignals in context");
    let read_sig = app_state.profile.open;
    let write_sig = write.profile.set_open;
    install_listeners_once(read_sig, write_sig);
    (read_sig, write_sig)
}

/// Attach window listeners for the three events the controller owns.
///
/// Idempotent — the helper tags `<body data-profile-bus="mounted">`
/// so repeat calls are no-ops. In practice the root `<App>` calls
/// [`use_profile_controller`] once, which calls this helper once.
fn install_listeners_once(
    open: ReadSignal<Option<ProfileState>>,
    set_open: WriteSignal<Option<ProfileState>>,
) {
    let Some(win) = web_sys::window() else { return };
    let body = match win.document().and_then(|d| d.body()) {
        Some(b) => b,
        None => return,
    };
    if body.get_attribute("data-profile-bus").as_deref() == Some("mounted") {
        return;
    }
    body.set_attribute("data-profile-bus", "mounted").ok();

    // OPEN — resolve the user id into a ProfileView via the client
    // handle stored in context, then push onto the controller signal.
    let Some(handle) = use_context::<crate::app::WebClientHandle>() else {
        return;
    };
    let handle_for_open = handle.clone();
    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
        let Ok(ce) = ev.dyn_into::<CustomEvent>() else {
            return;
        };
        let detail = ce.detail();
        let user_id = js_sys::Reflect::get(&detail, &"user_id".into())
            .ok()
            .and_then(|v| v.as_string());
        let Some(user_id) = user_id else { return };
        let anchor = js_sys::Reflect::get(&detail, &"anchor".into())
            .ok()
            .and_then(|v| v.dyn_into::<HtmlElement>().ok())
            .map(send_wrapper::SendWrapper::new);
        let Ok(peer_id) = user_id.parse::<willow_identity::EndpointId>() else {
            return;
        };
        let client = handle_for_open.clone();
        leptos::task::spawn_local(async move {
            let local = client.identity().endpoint_id();
            let view = client.views().profile_view_of(&peer_id, &local).await;
            set_open.set(Some(ProfileState {
                view: Arc::new(view),
                anchor,
            }));
        });
    });
    win.add_event_listener_with_callback(PROFILE_OPEN_EVENT, on_open.as_ref().unchecked_ref())
        .ok();
    on_open.forget();

    // CLOSE — clear the signal.
    let on_close = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        set_open.set(None);
    });
    win.add_event_listener_with_callback(PROFILE_CLOSE_EVENT, on_close.as_ref().unchecked_ref())
        .ok();
    on_close.forget();

    // ESCAPE — close on Escape, but only when a profile is actually open.
    let on_esc = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
        if let Ok(ke) = ev.dyn_into::<web_sys::KeyboardEvent>() {
            if ke.key() == "Escape" && open.with(Option::is_some) {
                set_open.set(None);
            }
        }
    });
    win.add_event_listener_with_callback("keydown", on_esc.as_ref().unchecked_ref())
        .ok();
    on_esc.forget();
}
