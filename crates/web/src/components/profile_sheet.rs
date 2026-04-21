//! Mobile profile-card bottom-sheet wrapper.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
//! §Mobile bottom sheet. Renders a scrim + slide-up sheet holding the
//! shared [`ProfileCardContent`].
//!
//! Mounted once at the app root. Hidden on desktop via a CSS
//! media-query. Scrim tap + Escape + back gesture dismiss.

use leptos::prelude::*;

use super::{ProfileCardContent, ProfileVariant};
use crate::profile::{close_profile, use_profile_controller};

/// Root-mounted mobile bottom sheet. `display: none` on desktop shells
/// via a media-query in `style.css`.
#[component]
pub fn ProfileSheet() -> impl IntoView {
    let (open, _set_open) = use_profile_controller();
    let on_close = Callback::new(move |_| close_profile());

    view! {
        <Show when=move || open.get().is_some() fallback=|| ()>
            {move || {
                let state = open.get().unwrap();
                let state_for_view = state.clone();
                let view_signal =
                    Signal::derive(move || state_for_view.view.clone());
                let variant = if state.view.is_self {
                    ProfileVariant::Self_
                } else {
                    ProfileVariant::Peer
                };
                let aria =
                    format!("profile — {}", state.view.display_name);
                view! {
                    <>
                        <div
                            class="profile-sheet__scrim"
                            on:click=move |_| close_profile()
                            role="presentation"
                        ></div>
                        <div
                            class="profile-sheet"
                            role="dialog"
                            aria-label=aria
                        >
                            <div class="profile-sheet__handle" aria-hidden="true"></div>
                            <ProfileCardContent
                                view=view_signal
                                variant=variant
                                on_close=on_close
                            />
                        </div>
                    </>
                }
            }}
        </Show>
    }
}
