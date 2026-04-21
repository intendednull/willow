//! Reconnection toast — Phase 2b sync queue.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Reconnection
//! toast.
//!
//! Listens to `app.queue.device_online` transitions. Fires only when
//! the offline window was ≥ 60 s (tracked via
//! `QueueMeta::offline_since_tick` → exposed through the queue view as
//! part of `device_online`'s transition). Auto-hides after 4 s;
//! dismissible via the `x` button.
//!
//! When the welcome-back banner is also visible the toast yields to it
//! (spec §Open questions §5).

use leptos::prelude::*;

use crate::icons;
use crate::state::AppState;

/// How long to keep the toast visible, in milliseconds.
const AUTO_HIDE_MS: i32 = 4_000;

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

    Effect::new(move |_| {
        let online = device_online.get();
        let prev = last_online.get_value();
        last_online.set_value(online);
        if !prev && online {
            // Transitioned offline → online. Show the toast with the
            // current queue depth as the snapshot count.
            queued_count.set(queue_view.with(|v| v.depth));
            visible.set(true);
            let vis = visible;
            let handle = gloo_timers::callback::Timeout::new(AUTO_HIDE_MS as u32, move || {
                vis.set(false);
            });
            handle.forget();
        }
    });

    let label = move || {
        let n = queued_count.get();
        if n > 0 {
            format!("reconnected · delivering {n} messages")
        } else {
            "reconnected".to_string()
        }
    };

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
                    on:click=move |_| visible.set(false)
                >
                    "×"
                </button>
            </div>
        </Show>
    }
}
