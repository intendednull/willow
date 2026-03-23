mod app;
mod components;

fn main() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();

    // Remove the loading indicator.
    if let Some(window) = web_sys::window() {
        if let Some(doc) = window.document() {
            if let Some(el) = doc.get_element_by_id("loading") {
                el.remove();
            }
        }
    }

    leptos::mount::mount_to_body(app::App);
}
