use leptos::prelude::*;

use crate::app::ClientHandle;
use crate::util::copy_to_clipboard;

/// Profile settings panel — display name, relay address, peer ID.
#[component]
pub fn SettingsPanel(
    client: ClientHandle,
    peer_id: ReadSignal<String>,
    on_server_settings: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    let (display_name, set_display_name) = signal(String::new());
    let (status_msg, set_status_msg) = signal(String::new());
    let (server_name, set_server_name) = signal(String::new());

    // Initialize from client.
    {
        let c = client.borrow();
        set_display_name.set(c.server_display_name());
        set_server_name.set(c.active_server_name());
    }

    // Save handler.
    let client_save = client.clone();
    let on_save = move |_| {
        let name = display_name.get_untracked();
        let mut c = client_save.borrow_mut();
        if !name.trim().is_empty() {
            let _ = c.set_server_display_name(name.trim());
        }
        set_status_msg.set("Saved.".to_string());
    };

    // Copy peer ID.
    let on_copy_peer_id = move |_| {
        let id = peer_id.get_untracked();
        copy_to_clipboard(&id);
        set_status_msg.set("Peer ID copied.".to_string());
    };

    view! {
        <div class="settings-panel">
            <h2>"Settings"</h2>

            // Status message.
            {move || {
                let msg = status_msg.get();
                if msg.is_empty() {
                    None
                } else {
                    Some(view! {
                        <div class="settings-status">{msg}</div>
                    })
                }
            }}

            // Peer ID display.
            <div class="settings-section">
                <label>"Your Peer ID"</label>
                <div class="peer-id-display">
                    <code class="peer-id-text">{move || peer_id.get()}</code>
                    <button class="btn btn-sm" on:click=on_copy_peer_id>"Copy"</button>
                </div>
            </div>

            // Per-server profile.
            <div class="settings-section">
                <div class="settings-server-label">
                    <span class="settings-server-prefix">"Profile for: "</span>
                    <span class="settings-server-name">{move || server_name.get()}</span>
                </div>
                <label>"Display Name (this server)"</label>
                <input
                    type="text"
                    placeholder="Enter display name..."
                    prop:value=move || display_name.get()
                    on:input=move |ev| set_display_name.set(event_target_value(&ev))
                />
            </div>

            <button class="btn btn-primary" on:click=on_save>"Save"</button>

            // Link to server settings.
            <div class="settings-section" style="margin-top: 24px;">
                <button class="btn btn-secondary server-settings-link" on:click=move |_| on_server_settings(())>
                    "Server Settings \u{2192}"
                </button>
            </div>
        </div>
    }
}
