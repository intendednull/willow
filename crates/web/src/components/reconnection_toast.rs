//! Reconnection toast — Phase 2b sync queue.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Reconnection
//! toast.
//!
//! Listens to `app.queue.device_online` transitions. Fires only when
//! the preceding offline window was ≥ 60 s — read from
//! `QueueView::last_offline_ticks` which the client stamps on every
//! offline → online flip. This suppresses the toast on initial connect
//! and on brief reconnect blips. Auto-hides after 4 s; dismissible via
//! the `x` button.
//!
//! When the welcome-back banner is also visible the toast yields to it
//! (spec §Open questions §5 — `sync_queue_copy::BANNER_TAKES_PRECEDENCE`).

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::sync_queue_copy;
use crate::icons;
use crate::state::AppState;

/// How long to keep the toast visible, in milliseconds.
const AUTO_HIDE_MS: i32 = 4_000;

/// Cancel a JS timeout by handle, if one is set.
fn clear_timeout(handle: Option<i32>) {
    if let (Some(h), Some(win)) = (handle, web_sys::window()) {
        win.clear_timeout_with_handle(h);
    }
}

/// Reconnection toast component. Mounted once near the root.
#[component]
pub fn ReconnectionToast() -> impl IntoView {
    let app =
        use_context::<AppState>().expect("<ReconnectionToast> mounted outside an AppState context");
    let device_online = app.queue.device_online;
    let queue_view = app.queue.view;

    let visible = RwSignal::new(false);
    let queued_count = RwSignal::new(0u32);

    // Track last-seen state so the effect only fires on transitions.
    let last_online = StoredValue::new(true);

    // Hold the active auto-hide timer handle (JS timeout ID). Before
    // scheduling a new timer we cancel the previous one, so stale
    // auto-hides from earlier flaps can never clobber `visible` (fixes
    // #349). `i32` is `Send`, so no SendWrapper needed.
    let timeout_handle: StoredValue<Option<i32>> = StoredValue::new(None);

    Effect::new(move |_| {
        let online = device_online.get();
        let prev = last_online.get_value();
        last_online.set_value(online);
        if !prev && online {
            // Transitioned offline → online. Read the 60 s gate from
            // the freshly stamped `last_offline_ticks` — if the offline
            // window was shorter, keep the toast suppressed (first-
            // connect + brief blip behaviour, spec §Reconnection toast).
            let (offline_ticks, depth) = queue_view.with(|v| (v.last_offline_ticks, v.depth));
            if offline_ticks.is_none_or(|t| t < sync_queue_copy::RECONNECT_GATE_TICKS) {
                return;
            }
            queued_count.set(depth);
            visible.set(true);
            // Cancel any still-pending auto-hide from an earlier flap
            // before scheduling a fresh one.
            clear_timeout(timeout_handle.get_value());
            let vis = visible;
            // One-shot auto-hide timer; this Effect re-runs on every
            // reconnect so `forget()` would leak per reconnection
            // (issue #193). `once_into_js` hands the closure to JS
            // for GC after fire / `clear_timeout_with_handle` above.
            let cb = wasm_bindgen::closure::Closure::once_into_js(move || vis.set(false));
            let handle = web_sys::window().and_then(|win| {
                win.set_timeout_with_callback_and_timeout_and_arguments_0(
                    cb.unchecked_ref(),
                    AUTO_HIDE_MS,
                )
                .ok()
            });
            timeout_handle.set_value(handle);
        }
    });

    let label = move || sync_queue_copy::toast_reconnected(queued_count.get());

    view! {
        <Show when=move || visible.get()>
            <div
                class="reconnection-toast"
                role="status"
                aria-live="polite"
            >
                <span class="reconnection-toast__icon">{icons::icon_check_small()}</span>
                <span class="reconnection-toast__label">{label}</span>
                <button
                    type="button"
                    class="reconnection-toast__dismiss"
                    aria-label="dismiss reconnection toast"
                    on:click=move |_| {
                        visible.set(false);
                        // Cancel any pending auto-hide so a future flap
                        // starts with a clean slate.
                        clear_timeout(timeout_handle.get_value());
                        timeout_handle.set_value(None);
                    }
                >
                    "×"
                </button>
            </div>
        </Show>
    }
}
