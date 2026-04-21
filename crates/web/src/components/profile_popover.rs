//! Desktop profile-card popover wrapper.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
//! §Desktop popover. Anchors the shared [`ProfileCardContent`] relative
//! to the clicked avatar, flips to the left if it would overflow the
//! right edge, clamps horizontally if neither side fits.
//!
//! Mounted once at the app root. Subscribes to
//! [`use_profile_controller`](crate::profile::use_profile_controller).

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::{ProfileCardContent, ProfileVariant};
use crate::profile::{close_profile, use_profile_controller};

const WIDTH_PX: f64 = 320.0;
const GAP_PX: f64 = 8.0;

/// Root-mounted desktop popover. `display: none` on mobile shells via
/// a media-query in `style.css`.
#[component]
pub fn ProfilePopover() -> impl IntoView {
    let (open, _set_open) = use_profile_controller();

    let position = Signal::derive(move || {
        let state = open.get()?;
        let anchor = state.anchor.as_ref()?;
        let rect = anchor.get_bounding_client_rect();
        let win = web_sys::window()?;
        let vw = win.inner_width().ok()?.as_f64()?;
        // Default position: 8 px right of anchor. Flip to the left if
        // the right edge would overflow the viewport by 12 px.
        let mut left = rect.right() + GAP_PX;
        if left + WIDTH_PX > vw - 12.0 {
            left = rect.left() - WIDTH_PX - GAP_PX;
        }
        left = left.max(12.0).min(vw - WIDTH_PX - 12.0);
        let top = rect.top().max(12.0);
        Some((left, top))
    });

    let on_close = Callback::new(move |_| close_profile());

    // Focus the first interactive element in the popover on open, and
    // remember the previous focus so close restores it.
    Effect::new(move |prev: Option<Option<web_sys::HtmlElement>>| {
        let previous_focus: Option<web_sys::HtmlElement> = prev.flatten();
        if open.get().is_some() {
            // Capture the currently-focused element so we can restore
            // it later (spec §Accessibility: focus returns to the
            // anchor when the card closes).
            let active = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.active_element())
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok());
            // Move focus to the first focusable element inside the card
            // on the next tick so the DOM is in place.
            leptos::task::spawn_local(async move {
                gloo_timers::future::TimeoutFuture::new(0).await;
                let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                    return;
                };
                if let Some(first) = doc.query_selector(".profile-popover button").ok().flatten()
                {
                    if let Ok(el) = first.dyn_into::<web_sys::HtmlElement>() {
                        el.focus().ok();
                    }
                }
            });
            active
        } else {
            // Restore focus to the previous target on close.
            if let Some(el) = previous_focus {
                el.focus().ok();
            }
            None
        }
    });

    view! {
        <Show when=move || open.get().is_some() fallback=|| ()>
            {move || {
                let state = open.get().unwrap();
                let pos = position.get().unwrap_or((12.0, 12.0));
                let state_for_view = state.clone();
                let view_signal = Signal::derive(move || state_for_view.view.clone());
                let variant = if state.view.is_self {
                    ProfileVariant::Self_
                } else {
                    ProfileVariant::Peer
                };
                view! {
                    <div
                        class="profile-popover"
                        style=format!("left: {}px; top: {}px;", pos.0, pos.1)
                        role="presentation"
                    >
                        <ProfileCardContent
                            view=view_signal
                            variant=variant
                            on_close=on_close
                        />
                    </div>
                }
            }}
        </Show>
    }
}
