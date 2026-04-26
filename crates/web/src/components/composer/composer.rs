//! `<Composer>` parent component — Phase 3a stub.
//!
//! T4 lands the skeleton; T5 replaces this with the real autogrow
//! textarea + send-button shell that supersedes `<ChatInput>`.

use leptos::prelude::*;

/// Composer placeholder. Filled in T5 with the autogrow textarea +
/// send-button shell.
#[component]
pub fn Composer() -> impl IntoView {
    view! { <div class="composer composer--stub" /> }
}
