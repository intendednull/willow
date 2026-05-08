//! `<AttachmentVoiceNote>` — voice-note playback card per
//! `docs/specs/2026-04-19-ui-design/files-inline.md` §Voice note.
//!
//! T6 of the phase-3b plan lands the full surface (play / pause IconBtn,
//! `<audio>` element, mm:ss / mm:ss timer, single-instance playback
//! coordinated by `VoiceNotePlayer`). This file is the placeholder
//! shipped alongside the [`super::pick`] decision table.

use leptos::prelude::*;

/// Placeholder voice-note component.
///
/// **Stub.** The full component lands in T6.
#[component]
pub fn AttachmentVoiceNote(filename: String) -> impl IntoView {
    view! {
        <div class="attachment attachment--voice-note" data-stub="t6-pending">
            <span class="attachment__filename">{filename}</span>
        </div>
    }
}
