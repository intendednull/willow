use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::app::WebClientHandle;
use crate::icons;

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
            if let Some(window) = web_sys::window() {
                window.alert_with_message("File is too large. Maximum size is 256 KB.").ok();
            }
            return;
        }

        let filename = file.name();
        let ch = channel.get_untracked();
        let handle_inner = handle_change.clone();

        let Ok(reader) = web_sys::FileReader::new() else {
            tracing::error!("FileShareButton: FileReader::new failed");
            return;
        };
        let reader_clone = reader.clone();

        let cb = Closure::once(move || {
            let result = match reader_clone.result() {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("FileReader result error: {e:?}");
                    return;
                }
            };
            let array_buf = match result.dyn_into::<js_sys::ArrayBuffer>() {
                Ok(b) => b,
                Err(_) => {
                    tracing::error!("FileReader result was not an ArrayBuffer");
                    return;
                }
            };
            let uint8 = js_sys::Uint8Array::new(&array_buf);
            let data = uint8.to_vec();

            wasm_bindgen_futures::spawn_local(async move {
                if let Err(e) = handle_inner.share_file_inline(&ch, &filename, &data).await {
                    if let Some(window) = web_sys::window() {
                        window.alert_with_message(&format!("Failed to share file: {e}")).ok();
                    }
                }
            });
        });

        reader.set_onloadend(Some(cb.as_ref().unchecked_ref()));
        reader.read_as_array_buffer(&file).ok();
        // Intentional leak: the FileReader callback must outlive this scope.
        // Since file picks are infrequent, the leak is acceptable.
        cb.forget();
    };

    view! {
        <button class="file-share-btn" title="Attach file" on:click=on_click>
            {icons::icon_paperclip()}
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
