//! `<ReplyBar>` — the styled reply preview that sits above the composer.
//!
//! Implements `docs/specs/2026-04-19-ui-design/composer.md` §Reply preview.
//! Layout, top-to-bottom of the bar (left-to-right of the row):
//!
//!   2 px `--moss-2` left rule │ `replying to` label │ parent author
//!   (italic, `--ink-1`) │ truncated body preview (`--ink-2`, ellipsis)
//!   │ flex spacer │ `cancel` text button (`--ink-2`, hover `--ink-0`).
//!
//! Bar background `--bg-2` on `--line`, top-only radius 10 px so the
//! bottom edge fuses visually with the composer pill below. 6 / 12
//! padding per spec.
//!
//! The preview body region is itself clickable: clicking it (not the
//! cancel button) fires `on_jump_to_parent` with the parent message id
//! so the chat view can scroll the message list to the parent and
//! flash the row via `willow-row-flash`. The cancel button stops
//! propagation so it never triggers a jump.
//!
//! `Escape` cancellation is owned by the `<Composer>` keydown handler
//! and continues to fire `on_cancel` regardless of where focus lives.
//!
//! ARIA: the cancel control has `aria-label="cancel reply"` per the
//! spec's accessibility table.

use leptos::prelude::*;
use willow_client::DisplayMessage;

/// Maximum visible characters of the parent body preview before we
/// ellipsize. The single-line layout already truncates via CSS for
/// long words, but pre-truncating in Rust keeps the rendered DOM
/// bounded and matches the "single line, ellipsis" spec language for
/// older browsers without `text-overflow: ellipsis`.
const PREVIEW_CHAR_BUDGET: usize = 120;

/// Reply preview bar, rendered above the composer when a parent
/// message is selected for reply.
///
/// Returns nothing when `replying_to.get()` is `None`. Component is
/// reactive — it re-renders whenever the signal flips.
#[component]
pub fn ReplyBar(
    /// The message currently being replied to. `None` collapses the
    /// component to nothing.
    replying_to: ReadSignal<Option<DisplayMessage>>,
    /// Fires when the user clicks `cancel` (or the spec's keyboard
    /// `Escape` handler in the parent).
    on_cancel: Callback<()>,
    /// Optional. Fires with the parent message id when the user
    /// clicks the preview body (not the cancel button). The chat
    /// view wires this to `scroll-to + flash`.
    #[prop(optional)]
    on_jump_to_parent: Option<Callback<String>>,
) -> impl IntoView {
    move || {
        let msg = replying_to.get()?;
        let preview = truncate_preview(&msg.body);
        let parent_id = msg.id.clone();
        let author = msg.author_display_name.clone();

        let jump_cb = on_jump_to_parent;
        let on_preview_click = move |ev: web_sys::MouseEvent| {
            if let Some(ref cb) = jump_cb {
                ev.stop_propagation();
                cb.run(parent_id.clone());
            }
        };

        let on_cancel_click = move |ev: web_sys::MouseEvent| {
            // Stop propagation so the click doesn't also bubble into the
            // preview-body click handler above (which would dispatch a
            // jump-to-parent right as the user is dismissing the reply).
            ev.stop_propagation();
            on_cancel.run(());
        };

        Some(view! {
            <div class="composer__reply-bar">
                <span class="composer__reply-bar-rule" aria-hidden="true"></span>
                <button
                    type="button"
                    class="composer__reply-bar-preview"
                    on:click=on_preview_click
                >
                    <span class="composer__reply-bar-label">"replying to"</span>
                    <span class="composer__reply-bar-author">{author}</span>
                    <span class="composer__reply-bar-body">{preview}</span>
                </button>
                <button
                    type="button"
                    class="composer__reply-bar-cancel"
                    aria-label="cancel reply"
                    on:click=on_cancel_click
                >
                    "cancel"
                </button>
            </div>
        })
    }
}

/// Truncate `body` to roughly [`PREVIEW_CHAR_BUDGET`] characters,
/// appending `…` when truncation occurs. Operates on `chars()` rather
/// than bytes so multi-byte glyphs aren't split mid-codepoint.
fn truncate_preview(body: &str) -> String {
    let mut iter = body.chars();
    let mut out = String::with_capacity(PREVIEW_CHAR_BUDGET + 1);
    for _ in 0..PREVIEW_CHAR_BUDGET {
        match iter.next() {
            Some(c) => out.push(c),
            None => return out,
        }
    }
    if iter.next().is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_preview_shorter_than_budget_passes_through() {
        let s = "hi there";
        assert_eq!(truncate_preview(s), s);
    }

    #[test]
    fn truncate_preview_longer_than_budget_appends_ellipsis() {
        let s = "x".repeat(PREVIEW_CHAR_BUDGET + 5);
        let out = truncate_preview(&s);
        assert_eq!(out.chars().count(), PREVIEW_CHAR_BUDGET + 1);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn truncate_preview_is_codepoint_safe() {
        // Multi-byte glyphs must not be split mid-codepoint.
        let s = "🌿".repeat(PREVIEW_CHAR_BUDGET + 2);
        let out = truncate_preview(&s);
        // We don't pin the exact rendered bytes — only that the result
        // is valid UTF-8 (Rust's `String` guarantees it) and ends with
        // the ellipsis.
        assert!(out.ends_with('…'));
    }
}
