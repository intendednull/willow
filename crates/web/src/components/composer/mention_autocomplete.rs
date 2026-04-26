//! `<MentionAutocomplete>` — popover above the composer for `@`-mention
//! suggestions.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/composer.md` §Mention
//! autocomplete. Plan: `docs/plans/2026-04-26-ui-phase-3a-composer.md`
//! Tasks T13–T14.
//!
//! ## Composition
//!
//! - Renders nothing when `candidates.get()` is empty so the popover
//!   doesn't claim layout.
//! - Otherwise lays out one row per candidate: avatar (20 px stub),
//!   display name, handle (mono `--ink-3`), presence dot. Spec lines
//!   99–101.
//! - The `selected` index is owned by the parent so keyboard navigation
//!   can update it without forcing an extra round-trip through this
//!   component.
//! - Each row is clickable and on hover highlights via the
//!   `--selected` modifier when its index matches `selected.get()`.
//!
//! ## Anchoring
//!
//! Spec line 97: "popover above the composer, `-8px` offset, aligned to
//! the `@`." A faithful implementation would mirror the textarea into
//! a hidden measurement element to find the `@` glyph's pixel
//! position. v1 punts on that and anchors to the textarea's
//! left/top instead — cheaper, and the popover already sits directly
//! above the `@` for short queries because the textarea is single-line
//! by default. The TODO comment in the parent points back here so a
//! follow-up can swap to the mirror-based anchor.
//!
//! ## Selection contract
//!
//! `on_select(candidate)` fires both on click and (driven by the
//! parent's keydown handler) on `Enter` / `Tab`. The parent owns the
//! splice — this component just emits which candidate the user chose.

use leptos::prelude::*;
use willow_client::views::MentionCandidate;

/// Mention autocomplete popover.
///
/// All filtering happens in the parent via
/// [`willow_client::mentions::Suggestions::filter`]. This component is
/// purely presentational so unit tests can drive each rendering case
/// without standing up a real `ClientHandle`.
#[component]
pub fn MentionAutocomplete(
    /// Already-filtered + ranked candidates the popover should show.
    /// Empty list collapses the popover entirely.
    candidates: Signal<Vec<MentionCandidate>>,
    /// Currently-selected row index. The parent updates this on
    /// `ArrowUp` / `ArrowDown` so keyboard navigation feels
    /// instant. Hover and click also update it through the
    /// `on_hover_index` callback below.
    selected: RwSignal<usize>,
    /// Fires when the user picks a candidate (via click or — driven
    /// by the parent's keydown handler — `Enter` / `Tab`).
    on_select: Callback<MentionCandidate>,
    /// Anchor coordinates `(top_px, left_px)` relative to the
    /// document. The parent computes this from
    /// `textarea.getBoundingClientRect()` so we don't reach into the
    /// DOM here.
    anchor: Signal<(i32, i32)>,
) -> impl IntoView {
    move || {
        let list = candidates.get();
        if list.is_empty() {
            return None;
        }
        let (top, left) = anchor.get();
        let style = format!("top: {top}px; left: {left}px;");

        // Snapshot the current list so the row closures can index by
        // peer id. We avoid `selected.get()` inside the row builder
        // because that would re-fire the entire `for` loop on
        // selection change; instead each row reads `selected.get()`
        // inside its `class` closure, which is fine-grained.
        let rows = list.clone();
        let total = rows.len();

        Some(
            view! {
                <div
                    class="mention-popover"
                    role="listbox"
                    aria-label="mention suggestions"
                    style=style
                >
                    {rows
                        .into_iter()
                        .enumerate()
                        .map(|(idx, c)| {
                            let is_channel_row = c.handle == CHANNEL_HANDLE;
                            let presence_class = format!(
                                "mention-popover__presence-dot mention-popover__presence-dot--{}",
                                c.presence.id()
                            );
                            let aria_label = if is_channel_row {
                                "everyone in this channel · notifies all members".to_string()
                            } else {
                                format!("{} @{}", c.display_name, c.handle)
                            };
                            let display_name = c.display_name.clone();
                            let handle_text = format!("@{}", c.handle);
                            let candidate = c.clone();
                            let on_click_select = on_select;
                            let on_click = move |ev: web_sys::MouseEvent| {
                                ev.prevent_default();
                                ev.stop_propagation();
                                on_click_select.run(candidate.clone());
                            };
                            let on_hover = move |_ev: web_sys::MouseEvent| {
                                selected.set(idx);
                            };
                            let row_class = move || {
                                let mut cls = String::from("mention-popover__row");
                                if selected.get() == idx {
                                    cls.push_str(" mention-popover__row--selected");
                                }
                                if is_channel_row {
                                    cls.push_str(" mention-popover__row--channel");
                                }
                                cls
                            };
                            let aria_selected = move || (selected.get() == idx).to_string();
                            view! {
                                <button
                                    type="button"
                                    class=row_class
                                    role="option"
                                    aria-label=aria_label
                                    aria-selected=aria_selected
                                    data-row-index=idx.to_string()
                                    data-row-total=total.to_string()
                                    on:mousedown=on_click
                                    on:mouseenter=on_hover
                                >
                                    <span
                                        class="mention-popover__avatar"
                                        aria-hidden="true"
                                    ></span>
                                    <span class="mention-popover__display">
                                        {display_name}
                                    </span>
                                    <span class="mention-popover__handle">
                                        {handle_text}
                                    </span>
                                    <span
                                        class=presence_class
                                        aria-hidden="true"
                                    ></span>
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            }
            .into_any(),
        )
    }
}

/// Sentinel handle for the synthetic `@channel` candidate (lit up in
/// T14 once the `ManageChannels` gate is in place). Defined here so
/// row rendering can stay symmetric across both real-peer and
/// synthetic rows.
pub const CHANNEL_HANDLE: &str = "channel";
