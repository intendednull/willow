//! `<AttachmentImage>` — inline image surface per
//! `docs/specs/2026-04-19-ui-design/files-inline.md` §Image.
//!
//! T5 of the phase-3b plan lands the full inline render (lazy `<img>`,
//! anchor wrapper for full-size open-in-new-tab, caption row,
//! viewport-aware `max-width`, fallback to file card on load error).
//! This file is the placeholder shipped alongside the [`super::pick`]
//! decision table so the module's public surface compiles.

use leptos::prelude::*;

/// Placeholder image-attachment component.
///
/// **Stub.** The full component lands in T5. Kept minimal here so the
/// module exports compile.
#[component]
pub fn AttachmentImage(filename: String) -> impl IntoView {
    view! {
        <div class="attachment attachment--image" data-stub="t5-pending">
            <span class="attachment__filename">{filename}</span>
        </div>
    }
}
