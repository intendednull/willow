//! `<FileShareButton>` — composer attach affordance.
//!
//! Phase 3b.5: clicking the paperclip flips the spec'd
//! `<UploadDialog>` open via the [`crate::upload_state::UploadQueue`]
//! context. The dialog owns the multi-file pick + per-file status +
//! batch send flow per `docs/specs/2026-04-19-ui-design/files-inline.md`
//! §Upload dialog. The button keeps the spec's `aria-label="attach
//! file"` so screen readers read it the same way they did before.
//!
//! The legacy `parse_inline_file` reader (below) stays alive so
//! historical `[file:NAME:base64]` messages from pre-3b peers still
//! render. Senders no longer emit that format.

use leptos::prelude::*;

use crate::icons;
use crate::upload_state::use_upload_queue;

/// Composer paperclip — flips `UploadQueue::open` so the
/// `<UploadDialog>` mounts. The `channel` prop is kept for backward
/// compat; the dialog reads its own channel signal from context.
#[component]
pub fn FileShareButton(channel: ReadSignal<String>) -> impl IntoView {
    let _ = channel; // dialog reads channel via its own context
    let queue = use_upload_queue();
    let on_click = move |_ev: web_sys::MouseEvent| {
        queue.open.set(true);
    };
    view! {
        <button
            class="file-share-btn"
            aria-label="attach file"
            title="Attach file"
            on:click=on_click
        >
            {icons::icon_paperclip()}
        </button>
    }
}

/// Parse an inline file message body. Returns `(filename, data_bytes)` if the
/// body matches the `[file:name:base64]` format.
pub fn parse_inline_file(body: &str) -> Option<(String, Vec<u8>)> {
    let inner = body.strip_prefix("[file:")?.strip_suffix(']')?;
    let colon = inner.find(':')?;
    let filename = &inner[..colon];
    let b64 = &inner[colon + 1..];
    let data = willow_client::base64::decode(b64)?;
    Some((filename.to_string(), data))
}

/// Format a byte count into a human-readable string (e.g. "1.2 KB").
pub fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// A card rendered in place of a file message. Shows an icon, filename,
/// file size, and a download button that triggers a browser download.
#[component]
pub fn FileCard(filename: String, data: Vec<u8>) -> impl IntoView {
    let size_str = format_file_size(data.len());
    let fname_display = filename.clone();
    let fname_download = filename.clone();

    let on_download = move |_| {
        let array = js_sys::Uint8Array::from(data.as_slice());
        let parts = js_sys::Array::new();
        parts.push(&array.buffer());

        let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(&parts) else {
            tracing::error!("FileCard: Blob::new failed");
            return;
        };
        let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
            tracing::error!("FileCard: create_object_url failed");
            return;
        };

        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Ok(a) = document.create_element("a") else {
            return;
        };
        a.set_attribute("href", &url).ok();
        a.set_attribute("download", &fname_download).ok();
        a.set_attribute("style", "display:none").ok();
        if let Some(body) = document.body() {
            body.append_child(&a).ok();
            use wasm_bindgen::JsCast;
            if let Ok(html_a) = a.clone().dyn_into::<web_sys::HtmlElement>() {
                html_a.click();
            }
            body.remove_child(&a).ok();
        }
        web_sys::Url::revoke_object_url(&url).ok();
    };

    view! {
        <div class="file-card">
            <span class="file-icon">{icons::icon_file()}</span>
            <div class="file-info">
                <span class="file-name">{fname_display}</span>
                <span class="file-size">{size_str}</span>
            </div>
            <button class="download-btn btn btn-sm btn-primary" on:click=on_download>
                {icons::icon_download()}
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inline_file_valid() {
        // encode "hello" in base64 and wrap in the inline file format
        let data = b"hello";
        let b64 = willow_client::base64::encode(data);
        let body = format!("[file:notes.txt:{b64}]");
        let result = parse_inline_file(&body);
        assert!(result.is_some(), "should parse valid inline file body");
        let (filename, bytes) = result.unwrap();
        assert_eq!(filename, "notes.txt");
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn parse_inline_file_plain_message_returns_none() {
        assert!(
            parse_inline_file("just a normal message").is_none(),
            "plain text should not parse as a file"
        );
    }

    #[test]
    fn parse_inline_file_malformed_returns_none() {
        // Missing closing bracket.
        assert!(parse_inline_file("[file:foo.txt:abc").is_none());
        // No colon separating filename from base64.
        assert!(parse_inline_file("[file:nodatahere]").is_none());
        // Empty body.
        assert!(parse_inline_file("").is_none());
    }

    #[test]
    fn format_file_size_bytes() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(512), "512 B");
        assert_eq!(format_file_size(1023), "1023 B");
    }

    #[test]
    fn format_file_size_kilobytes() {
        assert_eq!(format_file_size(1024), "1.0 KB");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(1024 * 1023), "1023.0 KB");
    }

    #[test]
    fn format_file_size_megabytes() {
        assert_eq!(format_file_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_file_size(1024 * 1024 * 2), "2.0 MB");
    }

    #[test]
    fn parse_inline_file_preserves_filename_with_extension() {
        let b64 = willow_client::base64::encode(b"data");
        let body = format!("[file:my-document.pdf:{b64}]");
        let (name, _) = parse_inline_file(&body).unwrap();
        assert_eq!(name, "my-document.pdf");
    }
}
