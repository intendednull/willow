//! `<AttachmentFileCard>` — generic file card per
//! `docs/specs/2026-04-19-ui-design/files-inline.md` §File card.
//!
//! T4 of the phase-3b plan lands the full visual treatment (mime icon,
//! filename, size, download IconBtn, large-file warning badge). This
//! file is the placeholder shipped alongside the [`super::pick`]
//! decision table so the module's public surface compiles without
//! waiting for the architectural EventKind work that wires
//! `Content::File` through to the message row.

use leptos::prelude::*;

/// Placeholder file-card component.
///
/// **Stub.** The full component lands in T4. Kept minimal here so the
/// module exports and downstream callers can compile against the
/// final API surface as it lands incrementally.
#[component]
pub fn AttachmentFileCard(filename: String, size_bytes: u64) -> impl IntoView {
    view! {
        <div class="attachment attachment--file-card" data-stub="t4-pending">
            <span class="attachment__filename">{filename}</span>
            <span class="attachment__size">{size_bytes.to_string()}</span>
        </div>
    }
}
