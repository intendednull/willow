//! Phase 2d — read-only banner inside archived ephemeral channels.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md` §Archive surface
//! (read-only review).

use leptos::prelude::*;

/// Read-only banner shown above the message list when an archived
/// ephemeral channel is open. Tapping the in-banner `post` button
/// fires the `on_expand` callback so the host pane can mount its
/// composer.
#[component]
pub fn ReadOnlyBanner(#[prop(into)] on_expand: Callback<()>) -> impl IntoView {
    view! {
        <div class="read-only-banner" role="status">
            <span class="read-only-banner-text">
                "archived — read-only · post or tap revive to bring it back"
            </span>
            <button
                class="read-only-banner-expand"
                type="button"
                on:click=move |_| on_expand.run(())
            >
                "post"
            </button>
        </div>
    }
}
