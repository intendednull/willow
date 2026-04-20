//! Downgrade / re-verify banner.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/trust-verification.md`
//! §Downgrade / re-verify prompts.
//!
//! Renders when a peer's trust is [`PeerTrust::DowngradedFromVerified`]
//! — surfaces the `keys changed — verify again` copy with a `compare
//! now` primary CTA and a `dismiss for now` secondary. Dismissing
//! stashes a 24-hour suppression key in `localStorage` (`willow.downgrade-dismiss.<peer_id>`);
//! the unverified badge on every surface **remains** for the full
//! duration of the unverified state.

use leptos::prelude::*;
use willow_client::trust::PeerTrust;

use super::sas::sas_copy;
use crate::icons;
use crate::state::{AppState, AppWriteSignals};

const DISMISS_KEY_PREFIX: &str = "willow.downgrade-dismiss.";
const DISMISS_DURATION_MS: i64 = 24 * 60 * 60 * 1000; // 24h

fn now_ms() -> i64 {
    js_sys::Date::now() as i64
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

#[cfg(not(target_arch = "wasm32"))]
fn storage() -> Option<()> {
    None
}

fn is_dismissed(peer_id: &str) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(storage) = storage() else {
            return false;
        };
        let key = format!("{DISMISS_KEY_PREFIX}{peer_id}");
        let Some(raw) = storage.get_item(&key).ok().flatten() else {
            return false;
        };
        let until: i64 = raw.parse().unwrap_or(0);
        until > now_ms()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = peer_id;
        false
    }
}

fn dismiss_for_24h(peer_id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(storage) = storage() else {
            return;
        };
        let key = format!("{DISMISS_KEY_PREFIX}{peer_id}");
        let until = now_ms() + DISMISS_DURATION_MS;
        let _ = storage.set_item(&key, &until.to_string());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = peer_id;
    }
}

/// Renders the downgrade banner when the peer's trust is
/// [`PeerTrust::DowngradedFromVerified`] and the dismiss window has
/// expired.
#[component]
pub fn DowngradeBanner(
    /// Peer id to render for. Banner self-hides when the trust state
    /// is anything other than `DowngradedFromVerified`.
    #[prop(into)]
    peer_id: Signal<String>,
) -> impl IntoView {
    let app_state = use_context::<AppState>().expect("AppState in context");
    let write = use_context::<AppWriteSignals>().expect("AppWriteSignals in context");
    let trust_map = app_state.trust.trust_map;
    let set_compare_target = write.trust.set_compare_target;

    // Local dismiss token — toggles when the user presses `dismiss`.
    let (dismiss_tick, set_dismiss_tick) = signal(0u32);
    let _ = dismiss_tick; // value unused, only the dep is.

    let visible = Memo::new(move |_| {
        let pid = peer_id.get();
        let map = trust_map.get();
        let is_downgraded = matches!(
            map.get(&pid),
            Some(PeerTrust::DowngradedFromVerified { .. })
        );
        let _ = dismiss_tick.get();
        is_downgraded && !is_dismissed(&pid)
    });

    view! {
        {move || {
            if !visible.get() {
                return None;
            }
            let pid_click = peer_id.get();
            let pid_dismiss = pid_click.clone();
            Some(view! {
                <div class="downgrade-banner" role="region" aria-live="polite">
                    <span class="downgrade-banner__icon" aria-hidden="true">
                        {icons::icon_shield()}
                    </span>
                    <div class="downgrade-banner__copy">
                        <span class="downgrade-banner__title">{sas_copy::DOWNGRADE_TITLE}</span>
                        <span class="downgrade-banner__body">{sas_copy::DOWNGRADE_BODY}</span>
                    </div>
                    <div class="downgrade-banner__actions">
                        <button
                            class="add-friend__cta-secondary"
                            on:click=move |_| {
                                dismiss_for_24h(&pid_dismiss);
                                set_dismiss_tick.update(|n| *n = n.wrapping_add(1));
                            }
                        >
                            {sas_copy::DOWNGRADE_DISMISS}
                        </button>
                        <button
                            class="add-friend__cta-primary"
                            on:click=move |_| {
                                set_compare_target.set(Some(pid_click.clone()));
                            }
                        >
                            {icons::icon_shield()}
                            <span>{sas_copy::DOWNGRADE_CTA}</span>
                        </button>
                    </div>
                </div>
            })
        }}
    }
}
