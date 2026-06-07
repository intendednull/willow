//! `<EditBar>` — the slim hint that sits above the composer while the
//! user is editing one of their own messages.
//!
//! Implements `docs/specs/2026-04-19-ui-design/composer.md` §Edit mode:
//! a single-line hint reading `editing message · esc to cancel` in the
//! composer's hint size (12 px) and `--ink-3` colour, plus a small
//! `cancel` text button. The send button label flip to `save` is
//! owned by the parent `<Composer>` (it has the typed-builder context
//! to know which submit path to call).
//!
//! The cancel control's ARIA label is `cancel edit`, matching the
//! spec's accessibility table even though the spec body only mentions
//! `Escape` cancellation. We render the button anyway because:
//!
//! 1. The accessibility table is a hard contract.
//! 2. Pointer-only users need a non-keyboard escape hatch from edit
//!    mode.
//!
//! Renders nothing when `editing.get()` is `None`.

use leptos::prelude::*;
use willow_client::DisplayMessage;

/// Edit-mode hint bar above the composer.
#[component]
pub fn EditBar(
    /// The message currently being edited. `None` collapses the
    /// component to nothing.
    editing: ReadSignal<Option<DisplayMessage>>,
    /// Fires when the user dismisses edit mode via the cancel button.
    /// `Escape` cancellation is owned by the composer's keydown
    /// handler and goes through the same parent callback.
    on_cancel: Callback<()>,
) -> impl IntoView {
    move || {
        editing.get()?;
        let on_cancel_click = move |ev: web_sys::MouseEvent| {
            ev.stop_propagation();
            on_cancel.run(());
        };
        Some(view! {
            <div class="composer__edit-bar">
                <span class="composer__edit-bar-text">
                    "editing message · esc to cancel"
                </span>
                <button
                    type="button"
                    class="composer__edit-bar-cancel"
                    aria-label="cancel edit"
                    on:click=on_cancel_click
                >
                    "cancel"
                </button>
            </div>
        })
    }
}
