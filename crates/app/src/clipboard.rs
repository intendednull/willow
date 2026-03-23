//! Cross-platform clipboard access.

/// Copy text to the system clipboard.
#[cfg(not(target_arch = "wasm32"))]
pub fn copy_to_clipboard(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text);
    }
}

/// Read text from the system clipboard.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut c| c.get_text().ok())
        .filter(|s| !s.is_empty())
}

/// No-op on native (read_clipboard is synchronous).
#[cfg(not(target_arch = "wasm32"))]
pub fn request_paste() {}

/// Copy text to the clipboard (WASM -- uses navigator.clipboard API).
#[cfg(target_arch = "wasm32")]
pub fn copy_to_clipboard(text: &str) {
    if let Some(window) = web_sys::window() {
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(text);
    }
}

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

#[cfg(target_arch = "wasm32")]
thread_local! {
    static PASTE_BUFFER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Request an async clipboard read (WASM). The result will be available
/// on the next call to `read_clipboard()`.
#[cfg(target_arch = "wasm32")]
pub fn request_paste() {
    wasm_bindgen_futures::spawn_local(async {
        let Some(window) = web_sys::window() else {
            return;
        };
        let clipboard = window.navigator().clipboard();
        let promise = clipboard.read_text();
        match wasm_bindgen_futures::JsFuture::from(promise).await {
            Ok(val) => {
                if let Some(text) = val.as_string() {
                    if !text.is_empty() {
                        PASTE_BUFFER.with(|buf| {
                            *buf.borrow_mut() = Some(text);
                        });
                    }
                }
            }
            Err(_) => {}
        }
    });
}

/// Read text from the clipboard (WASM).
/// Returns the result of the last `request_paste()` call, if available.
#[cfg(target_arch = "wasm32")]
pub fn read_clipboard() -> Option<String> {
    PASTE_BUFFER.with(|buf| buf.borrow_mut().take())
}
