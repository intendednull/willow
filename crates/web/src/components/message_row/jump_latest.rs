//! # Jump-to-latest pill
//!
//! Floating pill that appears at the bottom-right of the message list
//! when the user has scrolled up past the 120 px auto-scroll gate.
//! Spec: [`docs/specs/2026-04-19-ui-design/message-row.md`] §Scroll
//! anchoring (jump-to-latest pill).
//!
//! The owning `MessageList` controls two things externally:
//!
//! 1. **Mounting.** The pill only mounts when the user is more than
//!    120 px from the bottom. That gate lives in `chat.rs`.
//! 2. **Click behaviour.** The click handler is passed in as
//!    `on_click` — it smooth-scrolls the list to the bottom and
//!    resets the unread `new_count` signal.
//!
//! The pill itself is a pure view: it reads the `new_count` signal
//! and renders `" · {N} new"` when the count is greater than zero.

use leptos::prelude::*;

use crate::icons;

/// Renders the `jump to latest` pill.
///
/// `new_count` is the number of unread messages that have arrived
/// since the user last sat within 120 px of the bottom. When it's
/// zero the pill still mounts — the mounting gate in `MessageList`
/// decides when to show/hide — but the `" · {N} new"` suffix is
/// omitted.
///
/// `on_click` is invoked when the user clicks the pill; the owning
/// `MessageList` wires it to the smooth-scroll-to-bottom that also
/// resets `new_count` to zero.
#[component]
pub fn JumpToLatestPill(
    /// Number of unread messages accumulated while the user was away
    /// from the bottom. Rendered as ` · {N} new` when greater than zero.
    #[prop(into)]
    new_count: Signal<u32>,
    /// Invoked when the user clicks the pill.
    #[prop(into)]
    on_click: Callback<()>,
) -> impl IntoView {
    view! {
        <button
            class="jump-to-latest"
            type="button"
            aria-label="jump to latest messages"
            on:click=move |_| on_click.run(())
        >
            {icons::icon_chevron_down()}
            <span class="jump-to-latest__label">"jump to latest"</span>
            {move || {
                let n = new_count.get();
                if n > 0 {
                    Some(view! {
                        <span class="jump-to-latest__count">
                            " · "{n}" new"
                        </span>
                    })
                } else {
                    // Spec: the `" · {N} new"` suffix appears only
                    // when the count is positive. Omit the span
                    // entirely so the pill reads as plain
                    // "jump to latest".
                    None
                }
            }}
        </button>
    }
}
