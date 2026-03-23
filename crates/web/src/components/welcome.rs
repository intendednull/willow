use leptos::prelude::*;

use crate::app::ClientHandle;

/// Welcome/onboarding screen shown when the user has no servers.
///
/// Presents two options: create a new server or join an existing one via
/// invite code. Calls the appropriate callbacks when a server is created
/// or joined so the parent can refresh all reactive signals.
#[component]
pub fn WelcomeScreen(
    client: ClientHandle,
    on_server_created: impl Fn(()) + Send + Clone + 'static,
    on_joined: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    // Create-server state.
    let (server_name, set_server_name) = signal(String::new());
    let (display_name, set_display_name) = signal(String::new());
    let (create_status, set_create_status) = signal(String::new());

    // Peer ID for sharing.
    let peer_id = client.borrow().peer_id();
    let (copy_label, set_copy_label) = signal("Copy");

    // Join-server state.
    let (join_code, set_join_code) = signal(String::new());
    let (join_status, set_join_status) = signal(String::new());

    // Create server handler.
    let client_create = client.clone();
    let on_create = move |_| {
        let name = server_name.get_untracked();
        if name.trim().is_empty() {
            set_create_status.set("Please enter a server name.".to_string());
            return;
        }
        let mut c = client_create.borrow_mut();
        match c.create_server(name.trim()) {
            Ok(_server_id) => {
                // Optionally set display name.
                let dn = display_name.get_untracked();
                if !dn.trim().is_empty() {
                    let _ = c.set_server_display_name(dn.trim());
                }
                set_create_status.set(String::new());
                drop(c);
                on_server_created(());
            }
            Err(e) => {
                set_create_status.set(format!("Error: {e}"));
            }
        }
    };

    // Join server handler.
    let client_join = client.clone();
    let on_join = move |_| {
        let code = join_code.get_untracked();
        if code.trim().is_empty() {
            set_join_status.set("Please paste an invite code.".to_string());
            return;
        }
        let mut c = client_join.borrow_mut();
        match c.accept_invite(code.trim()) {
            Ok(()) => {
                set_join_status.set(String::new());
                set_join_code.set(String::new());
                drop(c);
                on_joined(());
            }
            Err(e) => {
                set_join_status.set(format!("Invalid invite code: {e}"));
            }
        }
    };

    view! {
        <div class="welcome-screen">
            <div class="welcome-card">
                <h1>"Welcome to Willow"</h1>
                <p class="tagline">
                    "P2P encrypted chat \u{2014} no accounts, no servers, no middlemen."
                </p>
                <div class="welcome-options">
                    // Create a server.
                    <div class="welcome-option">
                        <h2>"Create a Server"</h2>
                        {move || {
                            let msg = create_status.get();
                            if msg.is_empty() {
                                None
                            } else {
                                Some(view! {
                                    <div class="welcome-status">{msg}</div>
                                })
                            }
                        }}
                        <label>"Server Name"</label>
                        <input
                            type="text"
                            placeholder="My Server"
                            prop:value=move || server_name.get()
                            on:input=move |ev| set_server_name.set(event_target_value(&ev))
                        />
                        <label>"Display Name (optional)"</label>
                        <input
                            type="text"
                            placeholder="Your name..."
                            prop:value=move || display_name.get()
                            on:input=move |ev| set_display_name.set(event_target_value(&ev))
                        />
                        <button class="btn btn-primary welcome-btn" on:click=on_create>
                            "Create Server"
                        </button>
                    </div>
                    // Join a server.
                    <div class="welcome-option">
                        <h2>"Join a Server"</h2>
                        <label>"Your Peer ID"</label>
                        <p class="welcome-hint">"Share this with a server owner so they can generate an invite for you."</p>
                        <div class="welcome-peer-id">
                            <code class="peer-id-text">{peer_id.clone()}</code>
                            <button
                                class="btn btn-sm"
                                on:click={
                                    let pid = peer_id.clone();
                                    move |_| {
                                        let _ = js_sys::eval(&format!(
                                            "navigator.clipboard.writeText('{}')",
                                            pid
                                        ));
                                        set_copy_label.set("Copied!");
                                        set_timeout(move || set_copy_label.set("Copy"), std::time::Duration::from_secs(2));
                                    }
                                }
                            >
                                {move || copy_label.get()}
                            </button>
                        </div>
                        {move || {
                            let msg = join_status.get();
                            if msg.is_empty() {
                                None
                            } else {
                                Some(view! {
                                    <div class="welcome-status welcome-status-error">{msg}</div>
                                })
                            }
                        }}
                        <label>"Invite Code"</label>
                        <textarea
                            class="welcome-invite-input"
                            placeholder="Paste invite code here..."
                            prop:value=move || join_code.get()
                            on:input=move |ev| set_join_code.set(event_target_value(&ev))
                        ></textarea>
                        <button class="btn btn-primary welcome-btn" on:click=on_join>
                            "Join Server"
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}
