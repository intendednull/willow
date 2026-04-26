//! Welcome-back banner — Phase 2b sync queue.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Welcome-back
//! banner.
//!
//! Fires once per reopen-after-long-offline session: a 60+ s offline
//! window (read from `QueueView::last_offline_ticks`) **plus** at
//! least one queued message that arrived during the offline window
//! (`recent_arrivals`). Persists until the user dismisses the banner.

use leptos::prelude::*;

use crate::components::sync_queue_copy;
use crate::icons;
use crate::state::AppState;

/// Welcome-back banner component. Mounted once at the top of the home
/// view.
#[component]
pub fn WelcomeBackBanner() -> impl IntoView {
    let app =
        use_context::<AppState>().expect("<WelcomeBackBanner> mounted outside an AppState context");
    let device_online = app.queue.device_online;
    let queue_view = app.queue.view;

    let visible = RwSignal::new(false);
    let count = RwSignal::new(0u32);

    let last_online = StoredValue::new(true);

    // Track which `last_offline_ticks` value we last showed the banner
    // for. On a rapid flap the same tick value must not trigger the
    // banner a second time (fixes #351).
    let last_consumed_ticks: StoredValue<Option<u64>> = StoredValue::new(None);

    Effect::new(move |_| {
        let online = device_online.get();
        let prev = last_online.get_value();
        last_online.set_value(online);
        if !prev && online {
            // 60 s offline gate — only fire after a long window.
            let (offline_ticks, arrivals_sum) = queue_view.with(|v| {
                (
                    v.last_offline_ticks,
                    v.recent_arrivals.iter().map(|a| a.count).sum::<u32>(),
                )
            });
            if offline_ticks.is_none_or(|t| t < sync_queue_copy::RECONNECT_GATE_TICKS) {
                return;
            }
            // Guard: skip if this exact offline window has already
            // triggered the banner (same tick value seen twice on a
            // rapid flap).
            if offline_ticks == last_consumed_ticks.get_value() {
                return;
            }
            last_consumed_ticks.set_value(offline_ticks);
            if arrivals_sum > 0 {
                count.set(arrivals_sum);
                visible.set(true);
            }
        }
    });

    let label = move || sync_queue_copy::banner_welcome_back(count.get());

    view! {
        <Show when=move || visible.get()>
            <div class="welcome-back-banner" role="status" aria-live="polite">
                <span class="welcome-back-banner__glyph">{icons::icon_willow_mark()}</span>
                <span class="welcome-back-banner__label">{label}</span>
                <button
                    type="button"
                    class="welcome-back-banner__dismiss"
                    aria-label="dismiss welcome back banner"
                    on:click=move |_| visible.set(false)
                >
                    "×"
                </button>
            </div>
        </Show>
    }
}
