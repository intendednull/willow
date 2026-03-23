use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;

use crate::app::ClientHandle;
use crate::util::copy_to_clipboard;

/// Welcome/onboarding screen shown when the user has no servers.
///
/// Presents two options: create a new server or join an existing one via
/// invite code. Joining shows a profile step where the user can customize
/// their display name for the new server.
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
    // Two-step join: false = paste code, true = profile step.
    let (join_step_profile, set_join_step_profile) = signal(false);
    let (join_profile_name, set_join_profile_name) = signal(String::new());
    // Stash the validated code so the confirm handler can use it.
    let (validated_code, set_validated_code) = signal(String::new());

    // Pre-fill join profile name with current global display name.
    {
        let c = client.borrow();
        set_join_profile_name.set(c.display_name());
    }

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

    // Step 1: validate invite code and show profile step.
    let on_join_next = move |_: web_sys::MouseEvent| {
        let code = join_code.get_untracked();
        if code.trim().is_empty() {
            set_join_status.set("Please paste an invite code.".to_string());
            return;
        }
        set_validated_code.set(code.trim().to_string());
        set_join_status.set(String::new());
        set_join_step_profile.set(true);
    };

    // Step 2 state: wrap non-Copy client in Rc so closures can clone it.
    let client_join = SendWrapper::new(Rc::new(client.clone()));
    let on_joined_rc: SendWrapper<Rc<dyn Fn(())>> =
        SendWrapper::new(Rc::new(on_joined) as Rc<dyn Fn(())>);

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
                                        copy_to_clipboard(&pid);
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
                        {move || {
                            if join_step_profile.get() {
                                // Step 2: profile name before joining.
                                let cj = client_join.clone();
                                let joined_cb = on_joined_rc.clone();
                                let confirm = move |_: web_sys::MouseEvent| {
                                    let code = validated_code.get_untracked();
                                    let mut c = cj.borrow_mut();
                                    match c.accept_invite(&code) {
                                        Ok(()) => {
                                            let name = join_profile_name.get_untracked();
                                            if !name.trim().is_empty() {
                                                let _ = c.set_server_display_name(name.trim());
                                            }
                                            set_join_status.set(String::new());
                                            set_join_code.set(String::new());
                                            set_join_step_profile.set(false);
                                            drop(c);
                                            (joined_cb)(());
                                        }
                                        Err(e) => {
                                            set_join_status.set(format!("Invalid invite code: {e}"));
                                            set_join_step_profile.set(false);
                                        }
                                    }
                                };
                                view! {
                                    <div class="join-profile-step">
                                        <label>"Display Name for this server"</label>
                                        <p class="welcome-hint">"Pre-filled with your current name. Change it if you want a different identity on this server."</p>
                                        <input
                                            type="text"
                                            placeholder="Your name..."
                                            prop:value=move || join_profile_name.get()
                                            on:input=move |ev| set_join_profile_name.set(event_target_value(&ev))
                                        />
                                        <div class="join-profile-buttons">
                                            <button class="btn btn-sm" on:click=move |_| set_join_step_profile.set(false)>
                                                "\u{2190} Back"
                                            </button>
                                            <button class="btn btn-primary welcome-btn" on:click=confirm>
                                                "Join Server"
                                            </button>
                                        </div>
                                    </div>
                                }.into_any()
                            } else {
                                // Step 1: paste invite code.
                                view! {
                                    <div>
                                        <label>"Invite Code"</label>
                                        <textarea
                                            class="welcome-invite-input"
                                            placeholder="Paste invite code here..."
                                            prop:value=move || join_code.get()
                                            on:input=move |ev| set_join_code.set(event_target_value(&ev))
                                        ></textarea>
                                        <button class="btn btn-primary welcome-btn" on:click=on_join_next>
                                            "Next \u{2192}"
                                        </button>
                                    </div>
                                }.into_any()
                            }
                        }}
                    </div>
                </div>
            </div>
        </div>
    }
}
