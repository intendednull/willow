//! Offline status strip — Phase 2b sync queue.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/sync-queue.md` §Offline status
//! strip. Amber strip anchored below the window chrome. Renders only
//! when `queue.view.peer_count > 0`. Clicking opens the sync-queue
//! surface (`queue.open = true`).

use leptos::prelude::*;
use willow_client::RelayStatus;

use crate::components::sync_queue_copy;
use crate::icons;
use crate::state::AppState;

/// Offline status strip — amber summary + relay suffix.
///
/// Mounted once below the window chrome. Zero layout contribution when
/// `queue_peer_count == 0` because of the outer `<Show>` wrapper.
#[component]
pub fn OfflineStrip() -> impl IntoView {
    let app =
        use_context::<AppState>().expect("<OfflineStrip> mounted outside an AppState context");
    let queue_view = app.queue.view;
    let relay = app.queue.relay_status;
    let queue_open = app.queue.open;

    let show = move || queue_view.get().peer_count > 0;

    let text = move || {
        let v = queue_view.get();
        match v.peer_count {
            0 => String::new(),
            1 => {
                let peer_name = v
                    .per_peer
                    .keys()
                    .next()
                    .map(|pid| {
                        // Stringified peer id — display-name resolution is
                        // done in components that already have profile
                        // context. For the strip, we fall back to the
                        // truncated id + " peer" when we have no better.
                        let s = pid.to_string();
                        if s.len() > 8 {
                            format!("{}...", &s[..6])
                        } else {
                            s
                        }
                    })
                    .unwrap_or_else(|| "someone".to_string());
                sync_queue_copy::strip_singular(&peer_name, v.depth)
            }
            n => sync_queue_copy::strip_default(n, v.depth),
        }
    };

    let relay_suffix = move || match relay.get() {
        RelayStatus::Unreachable => sync_queue_copy::STRIP_RELAY_SUFFIX,
        _ => "",
    };

    view! {
        <Show when=show>
            <button
                class="offline-strip"
                aria-label="open sync queue"
                on:click=move |_| queue_open.set(true)
            >
                {move || (relay.get() == RelayStatus::Unreachable).then(icons::icon_signal)}
                {icons::icon_hourglass_sm()}
                <span class="offline-strip__summary" aria-live="polite">
                    {text}
                    {relay_suffix}
                </span>
                <span class="offline-strip__chevron">{icons::icon_chevron_right()}</span>
            </button>
        </Show>
    }
}
