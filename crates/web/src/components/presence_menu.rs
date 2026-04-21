//! # PresenceMenu
//!
//! Self-presence override menu. Four entries in order: auto / away /
//! gone / invisible. Spec §Self-presence + manual override.

use leptos::prelude::*;
use willow_client::presence::{PresenceOverride, PresenceState};

use crate::app::WebClientHandle;
use crate::components::{StatusDot, StatusDotBorder, StatusDotSize};
use crate::state::AppState;

/// Menu of the four override entries. Fires `on_close` after a choice
/// is applied so the caller can collapse the trigger.
#[component]
pub fn PresenceMenu(
    /// Open flag. The parent owns the signal; the menu reads it as a
    /// mount guard.
    #[prop(into)]
    open: ReadSignal<bool>,
    /// Close callback — fired on entry click / escape / outside click.
    on_close: Callback<()>,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();
    let app_state = use_context::<AppState>().unwrap();
    let current_override = app_state.presence.self_override;

    let entries: Vec<(PresenceOverride, &'static str, PresenceState)> = vec![
        (PresenceOverride::Auto, "auto", PresenceState::Here),
        (PresenceOverride::Away, "away", PresenceState::Away),
        (PresenceOverride::Gone, "gone", PresenceState::Gone),
        (
            PresenceOverride::Invisible,
            "invisible",
            PresenceState::Invisible,
        ),
    ];

    view! {
        {move || {
            if !open.get() {
                return None;
            }
            let rows: Vec<_> = entries.iter().copied().map(|(ov, label, preview)| {
                let handle_click = handle.clone();
                let on_close_click = on_close;
                let is_current = Signal::derive(move || current_override.get() == ov);
                view! {
                    <button
                        class="presence-menu__item"
                        role="menuitemradio"
                        aria-checked=move || if is_current.get() { "true" } else { "false" }
                        on:click=move |ev: web_sys::MouseEvent| {
                            ev.stop_propagation();
                            let h = handle_click.clone();
                            wasm_bindgen_futures::spawn_local(async move {
                                h.set_self_presence(ov).await;
                            });
                            on_close_click.run(());
                        }
                    >
                        <StatusDot
                            state=Signal::derive(move || preview)
                            size=StatusDotSize::MeStrip
                            border=StatusDotBorder::Bg1
                            ambient=false
                        />
                        <span>{label}</span>
                    </button>
                }
            }).collect();
            Some(view! {
                <div class="presence-menu" role="menu" aria-label="change your status">
                    {rows}
                </div>
            })
        }}
    }
}
