//! # Profile card stub (phase 1e)
//!
//! Minimal composition of avatar + [`StatusDot`] + [`PeerStatusLabel`]
//! so surfaces that want to reveal a peer's full presence (tap-to-reveal
//! on mobile, hover card on desktop) can drop in a single atom.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md` (deferred —
//! this stub satisfies presence.md §Ownership map so the atom slot
//! exists before the full card chrome lands in a later phase).

use leptos::prelude::*;
use willow_client::presence::PresenceState;

use super::peer_color;
use super::peer_status_label::PeerStatusLabel;
use super::status_dot::{StatusDot, StatusDotBorder, StatusDotSize};
use crate::state::AppState;

/// Minimal profile card — avatar + name + (presence atom). No chrome.
///
/// Reads the peer's presence state from [`AppState`] so the card stays
/// in sync with every other surface that shows the same peer.
#[component]
pub fn ProfileCardStub(
    /// Peer id (string form) to render.
    #[prop(into)]
    peer_id: Signal<String>,
    /// Display name to render under the avatar.
    #[prop(into)]
    display_name: Signal<String>,
) -> impl IntoView {
    let app_state = use_context::<AppState>().unwrap();
    let pid_for_state = peer_id;
    let presence = Signal::derive(move || {
        app_state
            .presence
            .per_peer
            .get()
            .get(&pid_for_state.get())
            .copied()
            .unwrap_or(PresenceState::Here)
    });

    view! {
        <div class="profile-card-stub" role="group" aria-label="peer profile">
            <div class="profile-card-stub__avatar" style=move || {
                format!(
                    "background: {};",
                    peer_color(&peer_id.get()),
                )
            }>
                <span class="profile-card-stub__initial">
                    {move || display_name.get()
                        .chars().next().unwrap_or('?')
                        .to_uppercase().to_string()}
                </span>
                <StatusDot
                    state=presence
                    size=StatusDotSize::Profile
                    border=StatusDotBorder::Bg1
                    ambient=false
                />
            </div>
            <div class="profile-card-stub__name">{move || display_name.get()}</div>
            <PeerStatusLabel state=presence show_dot=false/>
        </div>
    }
}
