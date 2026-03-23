/// Copy text to the clipboard via the web API.
pub fn copy_to_clipboard(text: &str) {
    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
    let _ = js_sys::eval(&format!("navigator.clipboard.writeText('{escaped}')"));
}
