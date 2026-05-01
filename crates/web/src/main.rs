use willow_web::{app, service_worker_bridge};

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

    // Wire navigator.serviceWorker.onmessage so pushes forwarded by the
    // service worker (focused-client path) reach the in-app Notifier.
    // The bridge validates the `kind` discriminator before accepting
    // (issue #244) and stashes payloads in a module-local cell rather
    // than on the global `window`. When the app is focused the SW
    // forwards here instead of calling `registration.showNotification`
    // — per the privacy contract the in-app toast is the visible
    // surface when we already have DOM.
    service_worker_bridge::install();

    leptos::mount::mount_to_body(app::App);
}
