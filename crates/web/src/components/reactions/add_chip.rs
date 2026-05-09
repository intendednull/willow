//! `<AddReactionChip>` — desktop-only hover affordance at the tail of
//! the reactions strip.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/reactions-pins.md`
//! §Add-reaction chip — "appears at the tail of the reaction strip
//! on row hover: dashed `--line` border, transparent background,
//! `--ink-3`, containing a `plus` + `smile` icon. Click opens the
//! emoji picker. Hidden on mobile — the add action is in the
//! action sheet."
//!
//! The mobile hide is enforced via a CSS `@media (max-width: 720px)`
//! rule on `.add-reaction-chip` (added in `style.css`); the
//! component itself always renders so the row markup stays the same
//! across viewports.

use leptos::prelude::*;

use crate::icons;

/// Add-reaction chip rendered at the tail of `<ReactionStrip>` on
/// desktop hover. `on_open_picker` is fired on click — wire it to
/// the same picker open-state signal as the row's hover-toolbar
/// `smile` button.
#[component]
pub fn AddReactionChip(on_open_picker: Callback<()>) -> impl IntoView {
    view! {
        <button
            class="add-reaction-chip"
            type="button"
            aria-label="add reaction"
            on:click=move |ev| {
                ev.stop_propagation();
                on_open_picker.run(());
            }
        >
            <span aria-hidden="true">{icons::icon_plus()}</span>
            <span aria-hidden="true">{icons::icon_smile()}</span>
        </button>
    }
}
