/// Copy text to the clipboard.
///
/// Tries `navigator.clipboard.writeText` first (modern API, requires HTTPS).
/// Falls back to creating a temporary textarea and using `execCommand('copy')`.
pub fn copy_to_clipboard(text: &str) {
    use wasm_bindgen::JsCast;

    let Some(window) = web_sys::window() else {
        return;
    };

    // Try modern clipboard API first.
    let clipboard = window.navigator().clipboard();
    let _ = clipboard.write_text(text);

    // Also do the textarea fallback in case clipboard API fails silently
    // (e.g. non-HTTPS, no user gesture, browser restrictions).
    let Some(document) = window.document() else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    let Ok(el) = document.create_element("textarea") else {
        return;
    };
    let ta: web_sys::HtmlTextAreaElement = el.unchecked_into();
    ta.set_value(text);
    let style = ta.style();
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("left", "-9999px");
    let _ = body.append_child(&ta);
    ta.select();
    if let Ok(html_doc) = document.dyn_into::<web_sys::HtmlDocument>() {
        let _ = html_doc.exec_command("copy");
    }
    let _ = body.remove_child(&ta);
}
