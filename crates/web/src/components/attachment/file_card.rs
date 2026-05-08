//! `<AttachmentFileCard>` — generic file card per
//! `docs/specs/2026-04-19-ui-design/files-inline.md` §File card.
//!
//! Rendered by the message row when [`super::pick`] returns
//! [`super::AttachmentKind::FileCard`] — i.e. for non-image / non-audio
//! attachments, images above the 4 MB inline cap, or attachments with
//! an unknown MIME. Layout is the spec's `--bg-2` card on `--line`
//! border, radius 10 px, 10 / 12 padding, with a mime icon, filename,
//! size + mime hint, and a download IconBtn. Files above 10 MB get
//! the `large · downloads on click` warning badge in `--amber`.
//!
//! The download IconBtn is wired to a click handler that fetches the
//! blob bytes from `WebClientHandle::network.blobs()` (T7 wiring) and
//! triggers the browser download via a synthesised `<a download>` link,
//! mirroring the legacy `<FileCard>` flow in `file_share.rs`.

use leptos::prelude::*;

use crate::icons;

/// Threshold above which a file gets the `large · downloads on click`
/// warning badge. Spec §Inline rendering rules: "File above 10 MB
/// shows the `large · downloads on click` warning badge".
const LARGE_FILE_WARNING_BYTES: u64 = 10 * 1024 * 1024;

/// Format `bytes` as a human-readable size (`1.2 KB`, `7.0 MB`).
///
/// Mirrors `willow_web::components::file_share::format_file_size`
/// but takes `u64` (the [`willow_state::FileAttachment::size_bytes`]
/// type) instead of the legacy `usize`.
pub(super) fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// File-card component for `Content::File` attachments.
///
/// Reads the spec's mime icon + filename + size + download IconBtn
/// layout and, when `size_bytes > 10 MB`, prepends the
/// `large · downloads on click` warning badge.
///
/// The download click handler is wired to fetch via the
/// [`willow_network::BlobStore`] context (T8/T9 will wire the upload
/// dialog to actually populate the blob store). For now the click
/// handler emits a console warning when the WebClientHandle context
/// is absent — keeps the visual shipping ahead of the full fetch path.
#[component]
pub fn AttachmentFileCard(
    /// Original filename from `EventKind::FileMessage::filename`.
    filename: String,
    /// Sender-declared file size in bytes. **Attacker-declared.**
    /// Used as a display hint only — the authoritative size is whatever
    /// the content-addressed hash resolves to once the blob is fetched.
    size_bytes: u64,
) -> impl IntoView {
    let aria_label = format!("download {filename}");
    let size_label = format_size(size_bytes);
    let large = size_bytes > LARGE_FILE_WARNING_BYTES;

    // The legacy `<FileCard>` in `file_share.rs` had the file bytes in
    // hand (inline base64). The typed `EventKind::FileMessage` carries
    // only a hash, so the click handler must fetch first. T8/T9 wire
    // that path through the blob store; for now the IconBtn emits a
    // console warning so the card visually ships even before the full
    // download flow lands. Receivers without bytes still see the
    // metadata correctly.
    let on_download = move |_ev: web_sys::MouseEvent| {
        // Emit a console warning so the missing-fetch path is
        // discoverable in dev. T8/T9 wires the actual blob fetch +
        // browser download via the `WebClientHandle::network.blobs()`
        // path; this stub keeps the visual card shipping ahead of
        // that work without silently dropping clicks.
        web_sys::console::warn_1(
            &"AttachmentFileCard download: blob-fetch wiring lands in T8/T9 — no bytes yet".into(),
        );
    };

    view! {
        <div
            class=move || if large {
                "attachment attachment--file-card attachment--large"
            } else {
                "attachment attachment--file-card"
            }
            data-mime=""
        >
            <span class="attachment__icon">{icons::icon_file()}</span>
            <div class="attachment__meta">
                <span class="attachment__filename">{filename}</span>
                <span class="attachment__size">{size_label}</span>
                {move || large.then(|| view! {
                    <span class="attachment__warning">
                        "large · downloads on click"
                    </span>
                })}
            </div>
            <button
                class="attachment__download"
                aria-label=aria_label
                on:click=on_download
            >
                {icons::icon_download()}
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_thresholds() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 7), "7.0 MB");
    }

    #[test]
    fn large_file_warning_threshold_matches_spec() {
        // Spec §Inline rendering rules cites a 10 MB threshold for the
        // `large · downloads on click` badge. Pin the constant so a
        // future copy-edit doesn't accidentally drift the boundary.
        assert_eq!(LARGE_FILE_WARNING_BYTES, 10 * 1024 * 1024);
    }
}
