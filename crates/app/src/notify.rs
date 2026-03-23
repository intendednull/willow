//! Cross-platform desktop/browser notifications.
//!
//! Shows a notification when a message arrives and the app window is not
//! focused (or on a different channel).

/// Send a desktop notification.
#[cfg(not(target_arch = "wasm32"))]
pub fn send_notification(title: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .appname("Willow")
        .timeout(notify_rust::Timeout::Milliseconds(5000))
        .show();
}

/// Send a browser notification (WASM).
#[cfg(target_arch = "wasm32")]
pub fn send_notification(title: &str, body: &str) {
    use web_sys::Notification;

    // Check permission.
    if Notification::permission() == web_sys::NotificationPermission::Granted {
        let opts = web_sys::NotificationOptions::new();
        opts.set_body(body);
        let _ = Notification::new_with_options(title, &opts);
    } else if Notification::permission() == web_sys::NotificationPermission::Default {
        // Request permission (async, fire-and-forget).
        let _ = Notification::request_permission();
    }
}
