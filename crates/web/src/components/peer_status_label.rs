//! # PeerStatusLabel atom
//!
//! Composed text label that accompanies a [`StatusDot`] when a surface
//! wants inline copy — profile card, message author hover, me-strip,
//! letters-dms row.
//!
//! Composition: `[icon] [dot] <text>`. Icons (ear / hourglass) appear
//! for `whispering` / `queued`. `invisible` renders nothing.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/presence.md` §PeerStatusLabel.

use leptos::prelude::*;
use willow_client::presence::PresenceState;

use super::status_dot::{StatusDot, StatusDotBorder, StatusDotSize};
use crate::icons;

/// Render a peer status label.
///
/// Props:
///   - `state` — derived presence.
///   - `show_dot` — include a small inline dot (off when the row already
///     shows an avatar dot nearby).
#[component]
pub fn PeerStatusLabel(
    #[prop(into)] state: Signal<PresenceState>,
    #[prop(default = true)] show_dot: bool,
) -> impl IntoView {
    view! {
        {move || {
            let s = state.get();
            if matches!(s, PresenceState::Invisible) {
                return None;
            }
            let state_id = s.id();
            let cls = format!("peer-status-label peer-status-label--{state_id}");
            let icon_view = match s {
                PresenceState::Whispering => Some(view! {
                    <span class="peer-status-label__icon" aria-hidden="true">
                        {icons::icon_ear()}
                    </span>
                }.into_any()),
                PresenceState::Queued(_) => Some(view! {
                    <span class="peer-status-label__icon" aria-hidden="true">
                        {icons::icon_hourglass_sm()}
                    </span>
                }.into_any()),
                _ => None,
            };
            let dot_view = if show_dot {
                Some(view! {
                    <span class="peer-status-label__dot">
                        <StatusDot
                            state=Signal::derive(move || s)
                            size=StatusDotSize::MeStrip
                            border=StatusDotBorder::Bg1
                            ambient=false
                        />
                    </span>
                }.into_any())
            } else {
                None
            };

            // For queued, render the count in its own span so it picks
            // up the mono 12px --amber typography.
            let text_view = if let PresenceState::Queued(n) = s {
                let label_text = "queued".to_string();
                let n_str = if n > 99 { "99+".to_string() } else { n.to_string() };
                view! {
                    <span class="peer-status-label__text">{label_text}</span>
                    <span class="peer-status-label__count">{n_str}</span>
                }.into_any()
            } else {
                view! {
                    <span class="peer-status-label__text">{s.label()}</span>
                }.into_any()
            };

            Some(view! {
                <span class=cls data-state=state_id>
                    {icon_view}
                    {dot_view}
                    {text_view}
                </span>
            })
        }}
    }
}
