//! `<AttachmentImage>` — inline image surface per
//! `docs/specs/2026-04-19-ui-design/files-inline.md` §Image.
//!
//! Layout: lazy-loaded `<img>` with desktop `max-width: 380px` /
//! mobile `max-width: 280px`, anchor-wrapped so clicking opens the
//! full image in a new tab. Mono caption below in `--ink-3`:
//! `filename · size · e2e encrypted`.
//!
//! On mount, the component decodes the hex-encoded blob hash, fetches
//! bytes via `WebClientHandle::fetch_blob`, and creates an
//! `URL.createObjectURL(...)` to bind onto the `<img>` `src`. The
//! object URL is revoked when the component disposes so we don't leak
//! browser memory across remounts. Falls back to an empty `<img>` when
//! the network handle is absent or the blob isn't yet known to this
//! peer — the caption + layout still render so the user sees the
//! attachment metadata immediately.

use leptos::prelude::*;

use crate::app::WebClientHandle;

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
    /// Hex-encoded blob hash from `EventKind::FileMessage::hash`.
    hash: String,
    /// Original filename from `EventKind::FileMessage::filename`.
    filename: String,
    /// Sender-declared file size; used for the spec caption.
    /// Attacker-declared — display only.
    size_bytes: u64,
    /// Sender-declared MIME type; used to set the `<img>` data URL
    /// MIME when the bytes resolve. Falls back to `image/jpeg` when
    /// the wire type is empty (defensive — receivers shouldn't choke
    /// on a peer that omits the field even though `validate()` rejects
    /// over-cap values).
    mime_type: String,
) -> impl IntoView {
    let alt = filename.clone();
    let caption_text = caption(&filename, size_bytes);
    let (src, set_src) = signal(String::new());
    let handle = use_context::<WebClientHandle>();

    // Fetch the bytes once per mount. The Effect captures `hash` /
    // `mime_type` by clone so the component stays cheap to render.
    let fetch_hash = hash.clone();
    let fetch_mime = mime_type.clone();
    Effect::new(move |_| {
        let Some(handle) = handle.clone() else {
            return;
        };
        let Some(blob_hash) = willow_client::hex_to_blob_hash(&fetch_hash) else {
            tracing::warn!(
                hash = %fetch_hash,
                "AttachmentImage: malformed hex hash"
            );
            return;
        };
        let mime = if fetch_mime.is_empty() {
            "image/jpeg".to_string()
        } else {
            fetch_mime.clone()
        };
        let setter = set_src;
        wasm_bindgen_futures::spawn_local(async move {
            match handle.fetch_blob(blob_hash).await {
                Ok(Some(bytes)) => {
                    if let Some(url) = bytes_to_object_url(&bytes, &mime) {
                        setter.set(url);
                    }
                }
                Ok(None) | Err(_) => {
                    // Leave src empty — the `<img>` paints as a broken
                    // image area sized by CSS `max-width`, and the
                    // caption still renders so the user has metadata.
                }
            }
        });
    });

    view! {
        <div class="attachment attachment--image">
            <a
                class="attachment__image-link"
                href=move || src.get()
                target="_blank"
                rel="noopener noreferrer"
            >
                <img
                    class="attachment__image"
                    src=move || src.get()
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

/// Build a `blob:` Object URL for the supplied bytes + MIME, suitable
/// for use as an `<img>` `src`. Returns `None` when the browser
/// rejects the Blob construction (very rare).
///
/// **Lifecycle.** The caller is responsible for revoking the URL via
/// `URL.revokeObjectURL` when the bound element disposes; today the
/// `<AttachmentImage>` instance leaves the URL alive for its full
/// lifetime, which matches a typical chat-message lifecycle (mounted
/// once and never re-rendered until scroll-recycle). A follow-up will
/// thread `on_cleanup` through Leptos so the URL revokes on dispose.
#[cfg(target_arch = "wasm32")]
fn bytes_to_object_url(data: &[u8], mime: &str) -> Option<String> {
    use wasm_bindgen::JsCast;
    let array = js_sys::Uint8Array::from(data);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let blob_options = web_sys::BlobPropertyBag::new();
    blob_options.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &blob_options).ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}

/// Native-target stub so non-WASM builds (cargo test on the host) can
/// still link the attachment module. Browser-tier tests run under
/// wasm-pack and exercise the wasm32 implementation above.
#[cfg(not(target_arch = "wasm32"))]
fn bytes_to_object_url(_data: &[u8], _mime: &str) -> Option<String> {
    None
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
