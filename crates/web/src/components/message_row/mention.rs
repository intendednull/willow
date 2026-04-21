//! # Mention pill
//!
//! Renders one resolved `@mention` from the message-body pipeline.
//! Styling follows `docs/specs/2026-04-19-ui-design/message-row.md`
//! §Mentions: moss-coloured pill for peer mentions, amber for the
//! self-mention (`@you` or a handle that resolves to the local peer).
//!
//! The parser itself lives in `willow_client::mentions` so the
//! projection layer can eventually populate `DisplayMessage.mentions`
//! (Phase 2a Task 4). This module is pure view code.

use leptos::prelude::*;

/// A single `@mention` pill. Pass `is_self=true` for the amber variant
/// that marks the local peer.
///
/// The pill renders as a `<button>` so it is keyboard-focusable and
/// reads as an interactive element to screen readers. Clicking the
/// pill is a no-op for now — Phase 2a Task 4 will wire it to the
/// profile-card popover when that lands.
#[component]
pub fn MentionPill(
    /// The mention label, already truncated by `parse_mentions` if
    /// the source handle was longer than 32 characters, and rewritten
    /// to `"you"` for self-mentions. Rendered preceded by a literal `@`.
    label: String,
    /// The full, pre-truncation, pre-self-override handle. Emitted as
    /// the pill's `title` attribute so hovering a truncated or
    /// re-labelled mention still reveals the original handle (spec
    /// §Edge cases). Defaults to `label` when the caller doesn't have
    /// a separate full handle (e.g. standalone `MentionPill` usage).
    #[prop(optional, into)]
    full_label: Option<String>,
    /// Whether this mention refers to the local peer.
    is_self: bool,
) -> impl IntoView {
    let class = if is_self {
        "mention-pill mention-pill--self"
    } else {
        "mention-pill"
    };
    let title = full_label.unwrap_or_else(|| label.clone());
    let aria = format!("mention {title}");
    // TODO(profile-card.md): open the profile popover on click.
    view! {
        <button class=class aria-label=aria title=title type="button">
            "@"{label}
        </button>
    }
}
