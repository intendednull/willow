use leptos::prelude::*;

use crate::app::ClientHandle;
use crate::components::RoleManager;

/// A single role entry: (role_id, role_name, list of granted permission strings).
type RoleEntry = (String, String, Vec<String>);

/// Server settings panel — invite generation, role management, and server info.
#[component]
pub fn ServerSettingsPanel(
    client: ClientHandle,
    peer_id: ReadSignal<String>,
    #[prop(into)] roles: Signal<Vec<RoleEntry>>,
    on_back: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    let (invite_peer, set_invite_peer) = signal(String::new());
    let (invite_code, set_invite_code) = signal(String::new());
    let (status_msg, set_status_msg) = signal(String::new());

    let server_name = {
        let c = client.borrow();
        c.active_server_name()
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

    // Copy peer ID handler.
    let on_copy_peer_id = move |_| {
        let id = peer_id.get_untracked();
        copy_to_clipboard(&id);
        set_status_msg.set("Peer ID copied.".to_string());
    };

    view! {
        <div class="settings-panel server-settings">
            <div class="server-settings-header">
                <button class="btn btn-sm" on:click=move |_| on_back(())>
                    "\u{2190} Back"
                </button>
                <h2>{server_name}</h2>
            </div>

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

            // Invite section.
            <div class="settings-section invite-section">
                <h3>"Invite a Peer"</h3>
                <p class="settings-hint">"The recipient needs to share their Peer ID with you first."</p>
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

            // Your Peer ID (for sharing).
            <div class="settings-section">
                <h3>"Your Peer ID"</h3>
                <p class="settings-hint">"Share this with others so they can invite you to their servers."</p>
                <div class="peer-id-display">
                    <code class="peer-id-text">{move || peer_id.get()}</code>
                    <button class="btn btn-sm" on:click=on_copy_peer_id>"Copy"</button>
                </div>
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
    let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
    let _ = js_sys::eval(&format!("navigator.clipboard.writeText('{escaped}')"));
}
