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

    // Register the service worker for PWA support. Logs failures (HTTPS
    // misconfiguration, MIME mismatch, parse errors, scope violations) so
    // they surface instead of being silently swallowed (issue #606).
    if let Some(window) = web_sys::window() {
        let sw_container = window.navigator().service_worker();
        let promise = sw_container.register("/sw.js");
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = wasm_bindgen_futures::JsFuture::from(promise).await {
                let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                tracing::warn!("service worker registration failed: {msg}");
            }
        });
    }

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
