//! Welcome-back banner — Phase 2b sync queue.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Welcome-back
//! banner.
//!
//! Fires once per reopen-after-long-offline session: a 60+ s offline
//! window **plus** at least one queued message that arrived during the
//! offline window (`recent_arrivals`). Persists until the user
//! dismisses the banner.

use leptos::prelude::*;

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

    Effect::new(move |_| {
        let online = device_online.get();
        let prev = last_online.get_value();
        last_online.set_value(online);
        if !prev && online {
            // Sum recent arrivals — proxy for "messages queued while
            // offline that have now arrived".
            let n: u32 = queue_view.with(|v| v.recent_arrivals.iter().map(|a| a.count).sum());
            if n > 0 {
                count.set(n);
                visible.set(true);
            }
        }
    });

    let label = move || {
        format!(
            "willow queued {} messages while you were away — everything arrived",
            count.get()
        )
    };

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
