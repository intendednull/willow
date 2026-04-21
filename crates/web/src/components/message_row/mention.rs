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
    /// the source handle was longer than 32 characters. Rendered
    /// preceded by a literal `@`.
    label: String,
    /// Whether this mention refers to the local peer.
    is_self: bool,
) -> impl IntoView {
    let class = if is_self {
        "mention-pill mention-pill--self"
    } else {
        "mention-pill"
    };
    let aria = format!("mention {label}");
    // TODO(profile-card.md): open the profile popover on click.
    view! {
        <button class=class aria-label=aria type="button">
            "@"{label}
        </button>
    }
}
