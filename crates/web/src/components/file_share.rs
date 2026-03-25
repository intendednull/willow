use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::app::WebClientHandle;

/// Maximum inline file size (256 KB).
const MAX_FILE_SIZE: u64 = 256 * 1024;

/// Attachment button that opens a native file picker and shares small files
/// inline via base64-encoded messages.
///
/// Files larger than 256 KB are rejected with a browser alert.
#[component]
pub fn FileShareButton(channel: ReadSignal<String>) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();

    // Create a hidden file input and trigger it on button click.
    let input_ref = NodeRef::<leptos::html::Input>::new();

    let on_click = move |_| {
        if let Some(input) = input_ref.get() {
            let el: &web_sys::HtmlInputElement = &input;
            // Reset so the same file can be picked again.
            el.set_value("");
            el.click();
        }
    };

    let handle_change = handle.clone();
    let on_change = move |_ev: web_sys::Event| {
        let Some(input) = input_ref.get() else {
            return;
        };
        let el: &web_sys::HtmlInputElement = &input;

        let Some(files) = el.files() else {
            return;
        };
        let Some(file) = files.get(0) else {
            return;
        };

        let size = file.size() as u64;
        if size > MAX_FILE_SIZE {
            let window = web_sys::window().unwrap();
            let _ = window.alert_with_message("File is too large. Maximum size is 256 KB.");
            return;
        }

        let filename = file.name();
        let ch = channel.get_untracked();
        let handle_inner = handle_change.clone();

        let reader = web_sys::FileReader::new().unwrap();
        let reader_clone = reader.clone();

        let cb = Closure::once(move || {
            let result = reader_clone.result().unwrap();
            let array_buf = result.dyn_into::<js_sys::ArrayBuffer>().unwrap();
            let uint8 = js_sys::Uint8Array::new(&array_buf);
            let data = uint8.to_vec();

            if let Err(e) = handle_inner.share_file_inline(&ch, &filename, &data) {
                let window = web_sys::window().unwrap();
                let _ = window.alert_with_message(&format!("Failed to share file: {e}"));
            }
        });

        reader.set_onloadend(Some(cb.as_ref().unchecked_ref()));
        let _ = reader.read_as_array_buffer(&file);
        // Intentional leak: the FileReader callback must outlive this scope.
        // Since file picks are infrequent, the leak is acceptable.
        cb.forget();
    };

    view! {
        <button class="file-share-btn" title="Attach file" on:click=on_click>
            "\u{1F4CE}"
        </button>
        <input
            node_ref=input_ref
            type="file"
            style="display:none"
            on:change=on_change
        />
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

        let blob = web_sys::Blob::new_with_u8_array_sequence(&parts).unwrap();
        let url = web_sys::Url::create_object_url_with_blob(&blob).unwrap();

        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let a = document.create_element("a").unwrap();
        a.set_attribute("href", &url).unwrap();
        a.set_attribute("download", &fname_download).unwrap();
        a.set_attribute("style", "display:none").unwrap();
        document.body().unwrap().append_child(&a).unwrap();

        let html_a: web_sys::HtmlElement = a.clone().dyn_into().unwrap();
        html_a.click();

        document.body().unwrap().remove_child(&a).unwrap();
        web_sys::Url::revoke_object_url(&url).unwrap();
    };

    view! {
        <div class="file-card">
            <span class="file-icon">"\u{1F4C4}"</span>
            <div class="file-info">
                <span class="file-name">{fname_display}</span>
                <span class="file-size">{size_str}</span>
            </div>
            <button class="download-btn btn btn-sm btn-primary" on:click=on_download>
                "\u{2B07}"
            </button>
        </div>
    }
}
