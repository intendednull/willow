/// Scroll the message row with id `msg-{message_id}` into the centre
/// of the viewport and apply a 180 ms `willow-row-flash` highlight.
///
/// Used by the composer's reply preview bar (T7 in the Phase 3a plan)
/// to surface the parent message a reply targets. The flash class is
/// added immediately and removed after `FLASH_DURATION_MS` via
/// `setTimeout`; CSS owns the actual animation (see
/// `@keyframes willow-row-flash` in `style.css`).
///
/// No-ops silently when the element isn't in the DOM (e.g. the parent
/// message hasn't loaded yet, or the chat view is unmounted). Runs
/// only on `wasm32` — native test builds skip the body so this file
/// stays dual-target.
pub fn scroll_to_message_and_flash(message_id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;

        const FLASH_DURATION_MS: i32 = 180;

        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Some(el) = document.get_element_by_id(&format!("msg-{message_id}")) else {
            return;
        };

        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_behavior(web_sys::ScrollBehavior::Smooth);
        opts.set_block(web_sys::ScrollLogicalPosition::Center);
        el.scroll_into_view_with_scroll_into_view_options(&opts);

        // Toggle the flash class. We add `flash` (the simple,
        // composable name spec'd in the plan) and remove it after the
        // animation duration so a re-jump to the same parent flashes
        // again.
        let class_list = el.class_list();
        let _ = class_list.add_1("flash");

        let target = el.clone();
        let cleanup = wasm_bindgen::closure::Closure::once_into_js(move || {
            let _ = target.class_list().remove_1("flash");
        });
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            cleanup.as_ref().unchecked_ref(),
            FLASH_DURATION_MS,
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = message_id;
    }
}

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
