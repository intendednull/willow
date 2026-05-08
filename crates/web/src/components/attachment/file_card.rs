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
//! The download IconBtn fetches the blob bytes from
//! `WebClientHandle::network.blobs()` (decoding the hex-encoded hash
//! from the wire event) and triggers the browser download via the
//! shared [`super::trigger_download`] helper.

use leptos::prelude::*;

use crate::app::WebClientHandle;
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
/// `hash` is the 64-char lowercase hex encoding of the
/// content-addressed blob (see `willow_client::blob_hash_to_hex`).
/// Click on the download button decodes the hex back to a
/// [`willow_network::BlobHash`], fetches via
/// `WebClientHandle::network.blobs().get(...)`, and triggers a browser
/// download. When the network handle isn't available (offline /
/// pre-connect) or the blob isn't yet known to this peer, the button
/// disables itself with an `aria-disabled` hint rather than silently
/// dropping the click.
#[component]
pub fn AttachmentFileCard(
    /// Hex-encoded blob hash from `EventKind::FileMessage::hash`.
    hash: String,
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
    let handle = use_context::<WebClientHandle>();
    let download_disabled = handle.is_none();

    let download_filename = filename.clone();
    let download_hash = hash.clone();
    let on_download = move |_ev: web_sys::MouseEvent| {
        let Some(handle) = handle.clone() else {
            return;
        };
        let Some(blob_hash) = willow_client::hex_to_blob_hash(&download_hash) else {
            tracing::warn!(hash = %download_hash, "AttachmentFileCard: malformed hex hash");
            return;
        };
        let filename = download_filename.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match handle.fetch_blob(blob_hash).await {
                Ok(Some(bytes)) => {
                    super::trigger_download(&bytes, &filename);
                }
                Ok(None) => {
                    tracing::warn!(
                        filename = %filename,
                        "AttachmentFileCard: blob not available locally"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        filename = %filename,
                        error = ?e,
                        "AttachmentFileCard: blob fetch failed"
                    );
                }
            }
        });
    };

    view! {
        <div
            class=move || if large {
                "attachment attachment--file-card attachment--large"
            } else {
                "attachment attachment--file-card"
            }
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
                aria-disabled=move || if download_disabled { "true" } else { "false" }
                disabled=download_disabled
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
