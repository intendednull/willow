use leptos::prelude::*;

use crate::app::ClientHandle;
use crate::components::RoleManager;

/// A single role entry: (role_id, role_name, list of granted permission strings).
type RoleEntry = (String, String, Vec<String>);

/// Settings panel for display name, relay, invites, join, and role management.
#[component]
pub fn SettingsPanel(
    client: ClientHandle,
    peer_id: ReadSignal<String>,
    #[prop(into)] roles: Signal<Vec<RoleEntry>>,
    on_joined: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    let (display_name, set_display_name) = signal(String::new());
    let (relay_addr, set_relay_addr) = signal(crate::app::DEFAULT_RELAY.to_string());
    let (invite_peer, set_invite_peer) = signal(String::new());
    let (invite_code, set_invite_code) = signal(String::new());
    let (join_code, set_join_code) = signal(String::new());
    let (status_msg, set_status_msg) = signal(String::new());

    // Active server name for display.
    let (server_name, set_server_name) = signal(String::new());

    // Initialize display name and server name from client.
    {
        let c = client.borrow();
        set_display_name.set(c.server_display_name());
        set_server_name.set(c.active_server_name());
    }

    // Save handler -- uses per-server display name.
    let client_save = client.clone();
    let on_save = move |_| {
        let name = display_name.get_untracked();
        let mut c = client_save.borrow_mut();
        if !name.trim().is_empty() {
            let _ = c.set_server_display_name(name.trim());
        }
        set_status_msg.set("Saved.".to_string());
    };

    // Copy peer ID to clipboard.
    let on_copy_peer_id = move |_| {
        let id = peer_id.get_untracked();
        copy_to_clipboard(&id);
        set_status_msg.set("Peer ID copied.".to_string());
    };

    // Generate invite handler.
    let client_invite = client.clone();
    let on_generate_invite = move |_| {
        let recipient = invite_peer.get_untracked();
        if recipient.trim().is_empty() {
            set_status_msg.set("Enter a recipient Peer ID.".to_string());
            return;
        }
        let c = client_invite.borrow();
        match c.generate_invite(recipient.trim()) {
            Ok(code) => {
                set_invite_code.set(code);
                set_status_msg.set("Invite generated.".to_string());
            }
            Err(e) => {
                set_status_msg.set(format!("Error: {e}"));
            }
        }
    };

    // Copy invite code handler.
    let on_copy_invite = move |_| {
        let code = invite_code.get_untracked();
        if !code.is_empty() {
            copy_to_clipboard(&code);
            set_status_msg.set("Invite code copied.".to_string());
        }
    };

    // Join handler.
    let client_join = client.clone();
    let on_join = move |_| {
        let code = join_code.get_untracked();
        if code.trim().is_empty() {
            set_status_msg.set("Enter an invite code.".to_string());
            return;
        }
        let mut c = client_join.borrow_mut();
        match c.accept_invite(code.trim()) {
            Ok(()) => {
                set_status_msg.set("Joined successfully!".to_string());
                set_join_code.set(String::new());
                drop(c); // release borrow before callback
                on_joined(());
            }
            Err(e) => {
                set_status_msg.set(format!("Join failed: {e}"));
            }
        }
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

            // Relay address.
            <div class="settings-section">
                <label>"Relay Address"</label>
                <input
                    type="text"
                    placeholder="/ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooW..."
                    prop:value=move || relay_addr.get()
                    on:input=move |ev| set_relay_addr.set(event_target_value(&ev))
                />
            </div>

            <button class="btn btn-primary" on:click=on_save>"Save & Reconnect"</button>

            // Invite section.
            <div class="settings-section invite-section">
                <h3>"Invite a Peer"</h3>
                <label>"Recipient Peer ID"</label>
                <input
                    type="text"
                    placeholder="12D3KooW..."
                    prop:value=move || invite_peer.get()
                    on:input=move |ev| set_invite_peer.set(event_target_value(&ev))
                />
                <button class="btn btn-primary" on:click=on_generate_invite>"Generate Invite"</button>
                {move || {
                    let code = invite_code.get();
                    if code.is_empty() {
                        None
                    } else {
                        Some(view! {
                            <div class="invite-code-display">
                                <textarea readonly prop:value=code.clone()></textarea>
                                <button class="btn btn-sm" on:click=on_copy_invite>"Copy"</button>
                            </div>
                        })
                    }
                }}
            </div>

            // Join section.
            <div class="settings-section join-section">
                <h3>"Join a Server"</h3>
                <label>"Invite Code"</label>
                <div class="invite-code-display">
                    <textarea
                        placeholder="Paste invite code..."
                        prop:value=move || join_code.get()
                        on:input=move |ev| set_join_code.set(event_target_value(&ev))
                    ></textarea>
                </div>
                <button class="btn btn-primary" on:click=on_join>"Join"</button>
            </div>

            // Role management section.
            <div class="settings-section role-section">
                <RoleManager
                    client=client.clone()
                    peer_id=peer_id
                    roles=roles
                />
            </div>
        </div>
    }
}

/// Copy text to the clipboard via the web API.
fn copy_to_clipboard(text: &str) {
    // Use eval to call clipboard API without needing extra web-sys features.
    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
    let _ = js_sys::eval(&format!("navigator.clipboard.writeText('{escaped}')"));
}
