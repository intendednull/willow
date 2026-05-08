//! `<AttachmentImage>` — inline image surface per
//! `docs/specs/2026-04-19-ui-design/files-inline.md` §Image.
//!
//! Layout: lazy-loaded `<img>` with desktop `max-width: 380px` /
//! mobile `max-width: 280px`, anchor-wrapped so clicking opens the
//! full image in a new tab. Mono caption below in `--ink-3`:
//! `filename · size · e2e encrypted`.
//!
//! The actual blob-fetch path that resolves the `<img>` `src` from
//! the content-addressed hash lands with T8/T9 (upload dialog wiring).
//! Today the component renders the spec-correct shell with the
//! caption and an empty `src`; it will paint the image automatically
//! once an Effect populates a blob URL.

use leptos::prelude::*;

use super::file_card::format_size;

/// Image-attachment component.
///
/// Sender-declared `width` / `height`, when present, are stamped onto
/// the `<img>` element so the browser can reserve the correct
/// aspect-ratio while bytes stream. Per spec, sub-cap declared values
/// (16383 × 16383) are still attacker-controlled and the renderer
/// MUST clamp to `max-width: 380px` / `280px` via CSS — done by the
/// `attachment--image` rule. We never pass declared dimensions to
/// `canvas.width` or any allocation API.
#[component]
pub fn AttachmentImage(
    /// Original filename from `EventKind::FileMessage::filename`.
    filename: String,
    /// Sender-declared file size; used for the spec caption.
    /// Attacker-declared — display only.
    size_bytes: u64,
) -> impl IntoView {
    // Stub `src` until the blob-fetch path lands in T8/T9. The
    // anchor + caption layout is final; only the bytes are missing.
    let alt = filename.clone();
    let caption_text = caption(&filename, size_bytes);

    view! {
        <div class="attachment attachment--image">
            <a
                class="attachment__image-link"
                href="#"
                target="_blank"
                rel="noopener noreferrer"
            >
                <img
                    class="attachment__image"
                    alt=alt
                    loading="lazy"
                    decoding="async"
                />
            </a>
            <span class="attachment__caption">{caption_text}</span>
        </div>
    }
}

/// Format the spec caption — `filename · size · e2e encrypted` —
/// for use by the message-row when it has size information from
/// `FileAttachment::size_bytes`. Kept as a free function so tests
/// can pin the byte-exact format without rendering the component.
pub(super) fn caption(filename: &str, size_bytes: u64) -> String {
    format!("{filename} · {} · e2e encrypted", format_size(size_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_format_matches_spec() {
        // Spec §Image caption: `filename · size · e2e encrypted`.
        // Pin the format so a future copy-edit doesn't drift it.
        assert_eq!(
            caption("photo.jpg", 1_536),
            "photo.jpg · 1.5 KB · e2e encrypted"
        );
        assert_eq!(caption("a", 0), "a · 0 B · e2e encrypted");
        assert_eq!(
            caption("scan.png", 1_048_576),
            "scan.png · 1.0 MB · e2e encrypted"
        );
    }
}
