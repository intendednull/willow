//! Add-a-friend / compare-fingerprints dialog.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/trust-verification.md`
//! §Compare-fingerprints flow.
//!
//! On desktop this is a centred modal; on mobile a full-height bottom
//! sheet (the media query in style.css flips the card order so the
//! peer card sits on top above the thumb). Three screens:
//!
//! - `Compare` — two cards side-by-side with `they match` / `they
//!   don't match` CTAs.
//! - `ConfirmMatch` — `verified.` copy, `done` default focus.
//! - `ConfirmMismatch` — `marked not verified.` copy, `compare again`
//!   / `close`.
//!
//! The dialog never touches the network: all state lives in the
//! `WebTrustStore`. Per the plan's ambiguity decisions, a self-compare
//! (`open == own peer id`) renders only the `you` card with a single
//! `close` CTA.

use std::sync::Arc;

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use willow_client::trust::{PeerTrust, TrustStoreHandle, UnverifiedReason};
use willow_crypto::sas_words;

use super::sas::{
    sas_copy, FingerprintGrid, FingerprintLabel, FingerprintLabelWhich, FingerprintSize,
    FingerprintVariant,
};
use crate::app::WebClientHandle;
use crate::icons;
use crate::state::{AppState, AppWriteSignals};

/// Which panel is showing in the dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Screen {
    #[default]
    Compare,
    ConfirmMatch,
    ConfirmMismatch,
}

/// Compute the two SAS fingerprints for the dialog.
///
/// The session key is derived from `blake3(local_pub || remote_pub ||
/// DS_TAG)` per the plan's ambiguity decisions — we swap in the real
/// per-DM key when the backend exposes one.
fn derive_session_seed(
    local: &willow_identity::EndpointId,
    remote: &willow_identity::EndpointId,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(willow_crypto::SAS_DS_TAG);
    hasher.update(local.as_bytes());
    hasher.update(remote.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Announce a one-shot message through `#trust-live-region`.
///
/// No-op on native. Sets text_content to "" first, then the target
/// string, so SRs re-read duplicate messages.
pub(crate) fn announce(msg: &str) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Some(region) = doc.get_element_by_id("trust-live-region") else {
        return;
    };
    region.set_text_content(Some(""));
    let msg = msg.to_string();
    let region_clone = region.clone();
    // One-shot 20ms timer fires per `announce()` call (per trust
    // action). `once_into_js` hands the closure to JS so GC reclaims
    // it after the callback runs — `.forget()` would leak per
    // announcement (issue #193).
    let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
        region_clone.set_text_content(Some(&msg));
    });
    if let Some(win) = web_sys::window() {
        win.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 20)
            .ok();
    }
}

/// Root-mounted compare-fingerprints dialog. Renders when the
/// `trust.compare_target` signal holds `Some(peer_id)`.
#[component]
pub fn AddFriendDialog() -> impl IntoView {
    let app_state = use_context::<AppState>().expect("AppState in context");
    let write = use_context::<AppWriteSignals>().expect("AppWriteSignals in context");
    let client = use_context::<WebClientHandle>().expect("WebClientHandle in context");
    let trust_store = use_context::<TrustStoreHandle>().expect("TrustStoreHandle in context");

    let compare_target = app_state.trust.compare_target;
    let set_compare_target = write.trust.set_compare_target;
    let set_trust_map = write.trust.set_trust_map;
    let (screen, set_screen) = signal(Screen::Compare);

    // Reset to the Compare screen whenever a fresh target is set.
    Effect::new(move |_| {
        let _ = compare_target.get();
        set_screen.set(Screen::Compare);
    });

    let identity = client.identity();
    let local_pid = identity.endpoint_id();

    // Derive the preview words once per target.
    let words_you = Memo::new(move |_| {
        let Some(peer_id) = compare_target.get() else {
            return std::array::from_fn::<_, 6, _>(|_| "…".to_string());
        };
        let Ok(remote_eid) = peer_id.parse::<willow_identity::EndpointId>() else {
            return std::array::from_fn::<_, 6, _>(|_| "…".to_string());
        };
        let seed = derive_session_seed(&local_pid, &remote_eid);
        sas_words(&seed, &local_pid, &remote_eid)
    });
    let words_them = words_you;

    let refresh_trust_map = {
        let store = Arc::clone(&trust_store);
        move || {
            let snap: std::collections::HashMap<String, PeerTrust> =
                store.snapshot().into_iter().collect();
            set_trust_map.set(snap);
        }
    };

    let on_match = {
        let store = Arc::clone(&trust_store);
        let refresh = refresh_trust_map.clone();
        move || {
            let Some(peer_id) = compare_target.get_untracked() else {
                return;
            };
            let Ok(remote_eid) = peer_id.parse::<willow_identity::EndpointId>() else {
                return;
            };
            let pinned = *remote_eid.as_bytes();
            let at_ms = js_sys::Date::now() as i64;
            store.set(
                &peer_id,
                PeerTrust::Verified {
                    at_ms,
                    pinned_key: pinned,
                },
            );
            refresh();
            announce("verified peer. compare fingerprints dialog closed.");
            set_screen.set(Screen::ConfirmMatch);
        }
    };

    let on_mismatch = {
        let store = Arc::clone(&trust_store);
        let refresh = refresh_trust_map.clone();
        move || {
            let Some(peer_id) = compare_target.get_untracked() else {
                return;
            };
            store.set(
                &peer_id,
                PeerTrust::Unverified {
                    reason: UnverifiedReason::SasMismatch,
                },
            );
            refresh();
            announce("marked not verified. compare fingerprints dialog still open; choose compare again or close.");
            set_screen.set(Screen::ConfirmMismatch);
        }
    };

    let close = move || set_compare_target.set(None);

    let on_esc = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Escape" {
            ev.prevent_default();
            close();
        }
    };

    view! {
        {move || {
            let Some(peer_id) = compare_target.get() else {
                return ().into_any();
            };

            let own_pid = local_pid.to_string();
            let is_self = peer_id == own_pid;

            let peer_name = app_state
                .network
                .peers
                .get()
                .into_iter()
                .find_map(|(id, n, _)| if id == peer_id { Some(n) } else { None })
                .unwrap_or_else(|| {
                    if peer_id.len() > 8 {
                        format!("{}…", &peer_id[..8])
                    } else {
                        peer_id.clone()
                    }
                });

            let you_display = app_state.server.display_name.get();
            let you_avatar_letter = you_display
                .chars()
                .next()
                .map(|c| c.to_ascii_uppercase().to_string())
                .unwrap_or_else(|| "y".to_string());
            let them_avatar_letter = peer_name
                .chars()
                .next()
                .map(|c| c.to_ascii_uppercase().to_string())
                .unwrap_or_else(|| "?".to_string());

            let current_screen = screen.get();

            let on_match_cb = on_match.clone();
            let on_mismatch_cb = on_mismatch.clone();

            let peer_card = match current_screen {
                Screen::Compare => {
                    let on_match_cb = on_match_cb.clone();
                    let on_mismatch_cb = on_mismatch_cb.clone();
                    let peer_name = peer_name.clone();
                    view! {
                        <div class="add-friend__panel add-friend__card-peer">
                            <div class="add-friend__panel-head">
                                <div class="add-friend__avatar">{them_avatar_letter.clone()}</div>
                                <div class="add-friend__identity">
                                    <span class="add-friend__identity-label">{peer_name}</span>
                                    <span class="add-friend__identity-meta">{sas_copy::PEER_META}</span>
                                </div>
                            </div>
                            <FingerprintLabel which=FingerprintLabelWhich::Peer size=FingerprintSize::Md/>
                            <FingerprintGrid
                                words=Signal::derive(move || words_them.get())
                                size=FingerprintSize::Md
                                variant=FingerprintVariant::Peer
                                aria_label="their six-word fingerprint"
                            />
                            <div class="add-friend__ctas">
                                <button
                                    class="add-friend__cta-secondary"
                                    on:click=move |_| on_mismatch_cb()
                                >
                                    {sas_copy::NO_MATCH_CTA}
                                </button>
                                <button
                                    class="add-friend__cta-primary"
                                    autofocus
                                    on:click=move |_| on_match_cb()
                                >
                                    {icons::icon_check()}
                                    <span>{sas_copy::MATCH_CTA}</span>
                                </button>
                            </div>
                        </div>
                    }
                    .into_any()
                }
                Screen::ConfirmMatch => {
                    view! {
                        <div class="add-friend__panel add-friend__card-peer">
                            <div class="add-friend__confirm">
                                <h3 class="add-friend__confirm-title">{sas_copy::CONFIRM_MATCH_TITLE}</h3>
                                <p class="add-friend__confirm-body">{sas_copy::CONFIRM_MATCH_BODY}</p>
                                <FingerprintGrid
                                    words=Signal::derive(move || words_them.get())
                                    size=FingerprintSize::Md
                                    variant=FingerprintVariant::Matched
                                    aria_label="their six-word fingerprint"
                                />
                                <div class="add-friend__ctas">
                                    <button
                                        class="add-friend__cta-primary"
                                        autofocus
                                        on:click=move |_| close()
                                    >
                                        "done"
                                    </button>
                                </div>
                            </div>
                        </div>
                    }
                    .into_any()
                }
                Screen::ConfirmMismatch => {
                    view! {
                        <div class="add-friend__panel add-friend__card-peer">
                            <div class="add-friend__confirm">
                                <h3 class="add-friend__confirm-title">{sas_copy::CONFIRM_MISMATCH_TITLE}</h3>
                                <p class="add-friend__confirm-body">{sas_copy::CONFIRM_MISMATCH_BODY}</p>
                                <FingerprintGrid
                                    words=Signal::derive(move || words_them.get())
                                    size=FingerprintSize::Md
                                    variant=FingerprintVariant::Mismatch
                                    aria_label="their six-word fingerprint"
                                />
                                <div class="add-friend__ctas">
                                    <button
                                        class="add-friend__cta-secondary"
                                        on:click=move |_| close()
                                    >
                                        "close"
                                    </button>
                                    <button
                                        class="add-friend__cta-primary"
                                        autofocus
                                        on:click=move |_| set_screen.set(Screen::Compare)
                                    >
                                        "compare again"
                                    </button>
                                </div>
                            </div>
                        </div>
                    }
                    .into_any()
                }
            };

            let you_display_clone = you_display.clone();
            let self_view = view! {
                <div class="add-friend__grid">
                    <div class="add-friend__panel">
                        <div class="add-friend__panel-head">
                            <div class="add-friend__avatar">{you_avatar_letter.clone()}</div>
                            <div class="add-friend__identity">
                                <span class="add-friend__identity-label">{format!("you · {you_display_clone}")}</span>
                                <span class="add-friend__identity-meta">{sas_copy::YOU_META}</span>
                            </div>
                        </div>
                        <FingerprintLabel which=FingerprintLabelWhich::You size=FingerprintSize::Md/>
                        <FingerprintGrid
                            words=Signal::derive(move || words_you.get())
                            size=FingerprintSize::Md
                            variant=FingerprintVariant::You
                            aria_label="your six-word fingerprint"
                        />
                        <div class="add-friend__ctas">
                            <button
                                class="add-friend__cta-secondary"
                                autofocus
                                on:click=move |_| close()
                            >
                                "close"
                            </button>
                        </div>
                    </div>
                </div>
            }
            .into_any();

            let paired_view = view! {
                <div class="add-friend__grid">
                    <div class="add-friend__panel">
                        <div class="add-friend__panel-head">
                            <div class="add-friend__avatar">{you_avatar_letter}</div>
                            <div class="add-friend__identity">
                                <span class="add-friend__identity-label">{format!("you · {you_display}")}</span>
                                <span class="add-friend__identity-meta">{sas_copy::YOU_META}</span>
                            </div>
                        </div>
                        <FingerprintLabel which=FingerprintLabelWhich::You size=FingerprintSize::Md/>
                        <FingerprintGrid
                            words=Signal::derive(move || words_you.get())
                            size=FingerprintSize::Md
                            variant=FingerprintVariant::You
                            aria_label="your six-word fingerprint"
                        />
                    </div>
                    {peer_card}
                </div>
            }
            .into_any();

            let body = if is_self { self_view } else { paired_view };

            view! {
                <div class="add-friend__backdrop"></div>
                <div class="add-friend__dialog" on:keydown=on_esc>
                    <div
                        class="add-friend__card"
                        role="dialog"
                        aria-modal="true"
                        aria-labelledby="add-friend-title"
                        tabindex="-1"
                    >
                        <h2 class="add-friend__title" id="add-friend-title">{sas_copy::TITLE}</h2>
                        <p class="add-friend__intro">{sas_copy::INTRO}</p>
                        {body}
                        <div class="add-friend__reassurance">
                            {icons::icon_shield()}
                            <span>{sas_copy::REASSURANCE}</span>
                        </div>
                    </div>
                </div>
            }
            .into_any()
        }}
    }
}
