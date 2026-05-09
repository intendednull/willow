//! `<UploadDialog>` — modal sheet driven by the
//! [`crate::upload_state::UploadQueue`] context.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/files-inline.md`
//! §Upload dialog.
//!
//! v1 layout per spec: modal sheet on `--bg-1` with `--line` border,
//! radius 12 px, `--shadow-2`; scrim behind the sheet absorbs
//! click-away dismissal. Picker row exposes a `choose files` button
//! (`--moss-1` border, `--ink-0` text) plus an `or drop files here`
//! hint, driving a hidden multi-`<input type="file">`. Per-file rows
//! show file glyph, filename, size, status (uploading / done / failed),
//! and a cancel IconBtn. Footer offers `cancel all` plus `attach to
//! message`; the confirm button stays disabled until at least one
//! upload resolves.
//!
//! Out of scope for v1: per-file progress bars (iroh blob store has
//! no incremental progress hook yet — drop-in once that lands), drag
//! highlight on the picker row (T10 page-level overlay), and paste
//! routing (T12). All three feed the same `UploadQueue` context.

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::app::WebClientHandle;
use crate::icons;
use crate::upload_state::{use_upload_queue, UploadStatus};

const MAX_ATTACHMENT_SIZE: u64 = 25 * 1024 * 1024;

/// Format `bytes` as a human-readable size (`1.2 KB`, `7.0 MB`).
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Modal sheet that drives the multi-file upload + send flow.
///
/// Reads `UploadQueue` + `WebClientHandle` from context. Visibility
/// is owned by `queue.open` — flip it from any other surface (the
/// composer attach button, a future drag overlay, a paste handler)
/// to mount the dialog with the queue's current contents.
///
/// `channel` is the active channel name; the confirm action posts
/// each completed entry as a `FileMessage` event in this channel.
#[component]
pub fn UploadDialog(channel: ReadSignal<String>) -> impl IntoView {
    let queue = use_upload_queue();
    let handle = use_context::<WebClientHandle>();

    // Confirm button enabled when at least one entry has resolved.
    let confirm_disabled = Memo::new(move |_| {
        queue.entries.with(|entries| {
            !entries
                .iter()
                .any(|e| matches!(e.status.get(), UploadStatus::Done(_)))
        })
    });

    view! {
        {move || {
            if !queue.open.get() {
                return None;
            }
            // Re-create per-render so each handler is freshly Fn.
            let handle_change = handle.clone();
            let handle_attach = handle.clone();
            let input_ref = NodeRef::<leptos::html::Input>::new();

            let on_browse = move |_ev: web_sys::MouseEvent| {
                if let Some(input) = input_ref.get() {
                    let el: &web_sys::HtmlInputElement = &input;
                    el.set_value("");
                    el.click();
                }
            };

            let on_change = move |_ev: web_sys::Event| {
                let Some(input) = input_ref.get() else {
                    return;
                };
                let el: &web_sys::HtmlInputElement = &input;
                let Some(files) = el.files() else {
                    return;
                };
                for i in 0..files.length() {
                    let Some(file) = files.get(i) else {
                        continue;
                    };
                    let size = file.size() as u64;
                    if size > MAX_ATTACHMENT_SIZE {
                        if let Some(window) = web_sys::window() {
                            let _ = window.alert_with_message(&format!(
                                "{} is too large (max 25 MB while the upload \
                                 dialog is in progress).",
                                file.name(),
                            ));
                        }
                        continue;
                    }
                    let filename = file.name();
                    let mime = file.type_();
                    let (id, status) = queue.push(filename.clone(), mime.clone(), size);
                    spawn_upload(handle_change.clone(), id.clone(), status, file);
                }
            };

            let on_cancel_all = move |_ev: web_sys::MouseEvent| {
                queue.cancel_all();
            };

            let on_attach = move |_ev: web_sys::MouseEvent| {
                let Some(handle) = handle_attach.clone() else {
                    tracing::warn!("UploadDialog attach: WebClientHandle missing");
                    return;
                };
                let channel_now = channel.get_untracked();
                let entries = queue.entries.get_untracked();
                for entry in entries {
                    let UploadStatus::Done(hash) = entry.status.get_untracked() else {
                        continue;
                    };
                    let handle = handle.clone();
                    let channel = channel_now.clone();
                    let filename = entry.filename.clone();
                    let mime = entry.mime.clone();
                    let size = entry.size;
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Err(e) = handle
                            .send_attachment_message(
                                &channel, &hash, &filename, &mime, size, None, None, "", None,
                            )
                            .await
                        {
                            tracing::warn!(
                                filename = %filename,
                                error = ?e,
                                "UploadDialog: send_attachment_message failed",
                            );
                        }
                    });
                }
                queue.cancel_all();
            };

            Some(view! {
                <div class="upload-dialog__scrim" on:click=on_cancel_all></div>
                <div class="upload-dialog" role="dialog" aria-label="upload attachments">
                    <div class="upload-dialog__picker">
                        <button
                            class="upload-dialog__browse"
                            type="button"
                            on:click=on_browse
                        >
                            "choose files"
                        </button>
                        <span class="upload-dialog__hint">"or drop files here"</span>
                    </div>
                    <input
                        node_ref=input_ref
                        type="file"
                        multiple
                        aria-label="choose files"
                        style="display:none"
                        on:change=on_change
                    />
                    <ul class="upload-dialog__list" role="list">
                        <For
                            each=move || queue.entries.get()
                            key=|entry| entry.id.clone()
                            let:entry
                        >
                            {
                                let row_id = entry.id.clone();
                                let row_filename = entry.filename.clone();
                                let cancel_label = format!("cancel upload of {row_filename}");
                                let row_status = entry.status;
                                let on_cancel = move |ev: web_sys::MouseEvent| {
                                    ev.stop_propagation();
                                    queue.remove(&row_id);
                                };
                                view! {
                                    <li class="upload-dialog__row">
                                        <span class="upload-dialog__icon" aria-hidden="true">
                                            {icons::icon_file()}
                                        </span>
                                        <span class="upload-dialog__filename">
                                            {entry.filename.clone()}
                                        </span>
                                        <span class="upload-dialog__size">
                                            {format_size(entry.size)}
                                        </span>
                                        <span class="upload-dialog__status">
                                            {move || match row_status.get() {
                                                UploadStatus::Uploading => "uploading…".to_string(),
                                                UploadStatus::Done(_) => "✓".to_string(),
                                                UploadStatus::Failed(e) => format!("✗ {e}"),
                                            }}
                                        </span>
                                        <button
                                            class="upload-dialog__cancel"
                                            type="button"
                                            aria-label=cancel_label
                                            on:click=on_cancel
                                        >
                                            {icons::icon_x()}
                                        </button>
                                    </li>
                                }
                            }
                        </For>
                    </ul>
                    <div class="upload-dialog__footer">
                        <button
                            class="upload-dialog__cancel-all"
                            type="button"
                            aria-label="cancel all uploads"
                            on:click=on_cancel_all
                        >
                            "cancel all"
                        </button>
                        <span class="upload-dialog__spacer"></span>
                        <button
                            class="upload-dialog__confirm"
                            type="button"
                            aria-label="attach to message"
                            prop:disabled=move || confirm_disabled.get()
                            on:click=on_attach
                        >
                            "attach to message"
                        </button>
                    </div>
                </div>
            })
        }}
    }
}

/// Read the picked file's bytes and call `upload_attachment`,
/// updating the entry's status to [`UploadStatus::Done`] on success
/// or [`UploadStatus::Failed`] on error. Mirrors the bytes-pump path
/// in `<FileShareButton>` but writes to the queue's row signal
/// instead of triggering a send directly.
fn spawn_upload(
    handle: Option<WebClientHandle>,
    _id: String,
    status: RwSignal<UploadStatus>,
    file: web_sys::File,
) {
    let Some(handle) = handle else {
        status.set(UploadStatus::Failed("network not connected".to_string()));
        return;
    };

    let Ok(reader) = web_sys::FileReader::new() else {
        status.set(UploadStatus::Failed("FileReader::new failed".to_string()));
        return;
    };
    let reader_clone = reader.clone();
    let cb = Closure::once(move || {
        let result = match reader_clone.result() {
            Ok(r) => r,
            Err(e) => {
                let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                status.set(UploadStatus::Failed(format!("read failed: {msg}")));
                return;
            }
        };
        let array_buf = match result.dyn_into::<js_sys::ArrayBuffer>() {
            Ok(b) => b,
            Err(_) => {
                status.set(UploadStatus::Failed(
                    "FileReader returned non-ArrayBuffer".to_string(),
                ));
                return;
            }
        };
        let uint8 = js_sys::Uint8Array::new(&array_buf);
        let data = uint8.to_vec();
        let handle_inner = handle.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match handle_inner.upload_attachment(data).await {
                Ok((hash, _size)) => {
                    status.set(UploadStatus::Done(hash));
                }
                Err(e) => {
                    status.set(UploadStatus::Failed(format!("{e}")));
                }
            }
        });
    });
    reader.set_onloadend(Some(cb.as_ref().unchecked_ref()));
    let _ = reader.read_as_array_buffer(&file);
    cb.forget();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_thresholds() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 7), "7.0 MB");
    }
}
