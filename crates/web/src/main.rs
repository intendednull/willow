mod app;
mod components;
mod event_processing;
mod handlers;
#[allow(dead_code)]
pub(crate) mod icons;
mod state;
pub mod state_bridge;
pub(crate) mod util;
pub mod voice;

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

    // Register the service worker for PWA support.
    let _ = js_sys::eval(
        "if ('serviceWorker' in navigator) { navigator.serviceWorker.register('/sw.js').catch(function() {}); }",
    );

    leptos::mount::mount_to_body(app::App);
}
