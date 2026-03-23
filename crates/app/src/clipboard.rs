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

/// Copy text to the clipboard (WASM — uses navigator.clipboard API).
#[cfg(target_arch = "wasm32")]
pub fn copy_to_clipboard(text: &str) {
    if let Some(window) = web_sys::window() {
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(text);
    }
}

/// Read text from the clipboard (WASM — not supported synchronously).
/// The async clipboard API requires a Promise; return None for now.
#[cfg(target_arch = "wasm32")]
pub fn read_clipboard() -> Option<String> {
    None
}
