use willow_web::app;

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
    // The payload is `{ kind: "willow-push", cat, ref }`. When the app
    // is focused the SW forwards here instead of calling
    // `registration.showNotification` — per the privacy contract the
    // in-app toast is the visible surface when we already have DOM.
    wire_service_worker_bridge();

    leptos::mount::mount_to_body(app::App);
}

/// Install the `navigator.serviceWorker.onmessage` listener that
/// stashes the most recent push payload on `window.__willowLastPush`
/// and dispatches a plain `willow-push` Event on the window so the
/// Leptos app can poll the property when the event fires. Using a
/// custom-event-free path sidesteps a `web-sys` feature dependency.
fn wire_service_worker_bridge() {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    let Some(window) = web_sys::window() else {
        return;
    };

    let sw = window.navigator().service_worker();
    let window_for_dispatch = window.clone();
    let onmessage =
        Closure::<dyn Fn(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
            // Stash the payload on `window.__willowLastPush` so the
            // in-app Notifier handler can read it after the event
            // fires.
            let _ = js_sys::Reflect::set(
                &window_for_dispatch,
                &wasm_bindgen::JsValue::from_str("__willowLastPush"),
                &ev.data(),
            );
            if let Ok(evt) = web_sys::Event::new("willow-push") {
                let _ = window_for_dispatch.dispatch_event(&evt);
            }
        });
    let _ = sw.add_event_listener_with_callback("message", onmessage.as_ref().unchecked_ref());
    onmessage.forget();
}
