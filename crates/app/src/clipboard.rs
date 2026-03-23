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

/// Read text from the clipboard (WASM).
///
/// Reads from `window.__willow_paste`, set by a JS paste event listener
/// in index.html. The JS side intercepts Ctrl+V before Bevy can prevent
/// it, allowing the browser's native paste event to fire. No permission
/// popup required.
#[cfg(target_arch = "wasm32")]
pub fn read_clipboard() -> Option<String> {
    use web_sys::js_sys;
    use web_sys::wasm_bindgen::JsValue;
    let window = web_sys::window()?;
    let key = JsValue::from_str("__willow_paste");
    let val = js_sys::Reflect::get(&window, &key).ok()?;
    if val.is_undefined() || val.is_null() {
        return None;
    }
    let _ = js_sys::Reflect::set(&window, &key, &JsValue::NULL);
    val.as_string().filter(|s: &String| !s.is_empty())
}
