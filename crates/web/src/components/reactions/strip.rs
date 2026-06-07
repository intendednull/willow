//! `<ReactionStrip>` — flex row of emoji pills + an optional
//! add-reaction chip on the tail.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/reactions-pins.md`
//! §Reaction strip:
//!
//! - Pills carry the spec geometry (`--bg-2` on `--line`, radius
//!   `999px`, padding `2px 8px`).
//! - When the local user has reacted, that pill picks up the
//!   `--moss-1` border + tinted bg via the `.reaction-pill--reacted`
//!   modifier class.
//! - Hover (desktop) shows a tooltip via `title` — composed by
//!   `super::tooltip::reactor_tooltip(reactors)`.
//! - On click, the pill toggles the local user's reaction by calling
//!   `on_react(emoji)`. The caller closes over the message id.
//! - On the tail of the strip, an `<AddReactionChip>` opens the
//!   emoji picker via `on_open_picker(())` (desktop hover only —
//!   `@media (max-width: 720px)` hides it via CSS).

use std::collections::HashMap;

use leptos::prelude::*;

use super::add_chip::AddReactionChip;
use super::tooltip::reactor_tooltip;

/// Reaction-strip component.
///
/// `reactions` is the projection-resolved map from
/// `DisplayMessage::reactions` — emoji → reactor display names.
/// `local_display_name` lets the strip flag the local user's pill
/// when present in the reactor list.
#[component]
pub fn ReactionStrip(
    reactions: HashMap<String, Vec<String>>,
    /// Local user's display name, used to detect "I reacted" so the
    /// pill picks up `.reaction-pill--reacted`. Empty string ⇔
    /// "unknown viewer" — pills render in the neutral state.
    #[prop(default = String::new(), into)]
    local_display_name: String,
    /// Toggle the local user's reaction with the given emoji. Caller
    /// closes over the message reference.
    on_react: Callback<String>,
    /// Open the emoji picker (the `<AddReactionChip>` callback).
    /// Caller threads this to the same picker open-state used by the
    /// row's hover toolbar.
    on_open_picker: Callback<()>,
) -> impl IntoView {
    // Sort by emoji string so the order is stable across renders.
    let mut entries: Vec<(String, Vec<String>)> = reactions.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    view! {
        <div class="reactions-strip">
            {entries.into_iter().map(|(emoji, reactors)| {
                let reacted_by_local = !local_display_name.is_empty()
                    && reactors.iter().any(|r| r == &local_display_name);
                let count = reactors.len();
                let title = reactor_tooltip(&reactors);
                let emoji_for_click = emoji.clone();
                let emoji_for_aria = emoji.clone();
                let aria = format!("{emoji_for_aria} reacted by {count} — toggle your reaction");
                let class = if reacted_by_local {
                    "reaction-pill reaction-pill--reacted"
                } else {
                    "reaction-pill"
                };
                view! {
                    <button
                        class=class
                        type="button"
                        title=title
                        aria-label=aria
                        on:click=move |ev| {
                            ev.stop_propagation();
                            on_react.run(emoji_for_click.clone());
                        }
                    >
                        <span class="reaction-pill__emoji" aria-hidden="true">{emoji}</span>
                        <span class="reaction-pill__count">{count.to_string()}</span>
                    </button>
                }
            }).collect::<Vec<_>>()}
            <AddReactionChip on_open_picker=on_open_picker />
        </div>
    }
}
