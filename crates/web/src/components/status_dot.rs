//! # StatusDot atom
//!
//! Small presence glyph placed at the bottom-right of an avatar. Every
//! surface that shows a peer routes through this component — see
//! `docs/specs/2026-04-19-ui-design/presence.md` §StatusDot.
//!
//! The dot carries a colour, a shape, an optional icon glyph, and an
//! aria-label derived from the state catalog. Colour is never the only
//! cue: `in a call` uses a *ring* shape, `whispering` pairs with an
//! `ear` icon, `queued · N` pairs with an `hourglass` icon, `gone` /
//! `away` differ by label text.

use leptos::prelude::*;
use willow_client::presence::PresenceState;

use crate::icons;

/// Rendering size preset. Each preset maps to a `data-size` attribute
/// consumed by the foundation CSS so one component handles every
/// surface's dot without bespoke styles.
///
/// Pixel sizes per spec §Sizing:
///   - `Profile` → 13 px (profile-card banner)
///   - `Row`     → 9 px  (letters-dms row)
///   - `Rail`    → 10 px (member rail)
///   - `MeStrip` → 8 px  (me-strip footer)
///   - `Author`  → 9 px  (message-row author avatar)
///   - `CallTile`→ 14 px (call participant tile)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusDotSize {
    Profile,
    Row,
    Rail,
    MeStrip,
    Author,
    CallTile,
}

impl StatusDotSize {
    fn css_key(&self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Row => "row",
            Self::Rail => "rail",
            Self::MeStrip => "me-strip",
            Self::Author => "author",
            Self::CallTile => "call-tile",
        }
    }
}

/// Border token used as the 2 px knock-out ring so the dot reads cleanly
/// against the underlying surface. Panels (sidebar, rail) sit on `--bg-1`,
/// main-pane surfaces (message list, call grid) sit on `--bg-0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusDotBorder {
    Bg0,
    Bg1,
}

impl StatusDotBorder {
    fn css_key(&self) -> &'static str {
        match self {
            Self::Bg0 => "bg0",
            Self::Bg1 => "bg1",
        }
    }
}

/// Lower-case label used by aria-label rendering. Spec §Aria labels.
fn state_label(state: PresenceState) -> String {
    state.label()
}

/// Render the status dot for a given [`PresenceState`].
///
/// Props:
///   - `state` — derived presence (from [`willow_client::presence`]).
///   - `size` — spatial preset; selects the pixel size.
///   - `border` — surface token for the knock-out ring.
///   - `ambient` — if true, `here` / `whispering` pulse softly. False on
///     stopping surfaces (profile card banner).
#[component]
pub fn StatusDot(
    #[prop(into)] state: Signal<PresenceState>,
    #[prop(default = StatusDotSize::Rail)] size: StatusDotSize,
    #[prop(default = StatusDotBorder::Bg1)] border: StatusDotBorder,
    #[prop(default = false)] ambient: bool,
) -> impl IntoView {
    let size_key = size.css_key();
    let border_key = border.css_key();

    view! {
        {move || {
            let s = state.get();
            if matches!(s, PresenceState::Invisible) {
                // Spec §State catalog: invisible renders nothing.
                return None;
            }
            let state_id = s.id();
            let aria = format!("status: {}", state_label(s));
            let mut cls = format!("status-dot status-dot--{size_key} status-dot--{state_id}");
            if ambient && matches!(s, PresenceState::Here | PresenceState::Whispering) {
                cls.push_str(" presence-pulse");
            }
            let inner = match s {
                PresenceState::Whispering => Some(view! {
                    <span class="status-dot__glyph" aria-hidden="true">
                        {icons::icon_ear()}
                    </span>
                }.into_any()),
                PresenceState::Queued(_) => Some(view! {
                    <span class="status-dot__glyph" aria-hidden="true">
                        {icons::icon_hourglass_sm()}
                    </span>
                }.into_any()),
                _ => None,
            };
            Some(view! {
                <span
                    class=cls
                    data-state=state_id
                    data-border=border_key
                    role="img"
                    aria-label=aria
                >
                    {inner}
                </span>
            })
        }}
    }
}
