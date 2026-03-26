use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;

use crate::app::WebClientHandle;
use crate::icons;

/// Panel for creating a new server or joining an existing one via invite code.
/// Shown when the user clicks the "+" button in the server rail.
#[component]
pub fn AddServerPanel(on_done: impl Fn(()) + Send + Clone + 'static) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();

    // Create server state.
    let (server_name, set_server_name) = signal(String::new());
    let (create_display_name, set_create_display_name) = signal(String::new());
    let (status_msg, set_status_msg) = signal(String::new());

    // Join server state.
    let (join_code, set_join_code) = signal(String::new());
    let (join_step, set_join_step) = signal(false);
    let (join_profile_name, set_join_profile_name) = signal(String::new());
    let (validated_code, set_validated_code) = signal(String::new());

    set_join_profile_name.set(handle.display_name());

    // Create handler.
    let handle_create = handle.clone();
    let on_done_create = on_done.clone();
    let on_create = move |_| {
        let name = server_name.get_untracked();
        if name.trim().is_empty() {
            set_status_msg.set("Please enter a server name.".to_string());
            return;
        }
        match handle_create.create_server(name.trim()) {
            Ok(_) => {
                let dn = create_display_name.get_untracked();
                if !dn.trim().is_empty() {
                    let _ = handle_create.set_server_display_name(dn.trim());
                }
                on_done_create(());
            }
            Err(e) => set_status_msg.set(format!("Error: {e}")),
        }
    };

    // Join step 1.
    let on_join_next = move |_: web_sys::MouseEvent| {
        let code = join_code.get_untracked();
        if code.trim().is_empty() {
            set_status_msg.set("Please paste an invite code.".to_string());
            return;
        }
        set_validated_code.set(code.trim().to_string());
        set_status_msg.set(String::new());
        set_join_step.set(true);
    };

    // Join step 2.
    let handle_join = SendWrapper::new(Rc::new(handle.clone()));
    let on_done_rc: SendWrapper<Rc<dyn Fn(())>> =
        SendWrapper::new(Rc::new(on_done) as Rc<dyn Fn(())>);

    view! {
        {move || {
            let msg = status_msg.get();
            if msg.is_empty() {
                None
            } else {
                Some(view! { <div class="settings-status">{msg}</div> })
            }
        }}

        <div class="welcome-options" style="margin-top: 16px;">
            <div class="welcome-option">
                <h3>"Create a Server"</h3>
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
                    prop:value=move || create_display_name.get()
                    on:input=move |ev| set_create_display_name.set(event_target_value(&ev))
                />
                <button class="btn btn-primary welcome-btn" on:click=on_create>
                    "Create Server"
                </button>
            </div>
            <div class="welcome-option">
                <h3>"Join a Server"</h3>
                {move || {
                    if join_step.get() {
                        let hj = handle_join.clone();
                        let done_cb = on_done_rc.clone();
                        let confirm = move |_: web_sys::MouseEvent| {
                            let code = validated_code.get_untracked();
                            match hj.accept_invite(&code) {
                                Ok(()) => {
                                    let name = join_profile_name.get_untracked();
                                    if !name.trim().is_empty() {
                                        let _ = hj.set_server_display_name(name.trim());
                                    }
                                    set_join_code.set(String::new());
                                    set_join_step.set(false);
                                    (done_cb)(());
                                }
                                Err(e) => {
                                    set_status_msg.set(format!("Invalid invite code: {e}"));
                                    set_join_step.set(false);
                                }
                            }
                        };
                        view! {
                            <div>
                                <label>"Display Name for this server"</label>
                                <p class="welcome-hint">"Pre-filled with your current name."</p>
                                <input
                                    type="text"
                                    placeholder="Your name..."
                                    prop:value=move || join_profile_name.get()
                                    on:input=move |ev| set_join_profile_name.set(event_target_value(&ev))
                                />
                                <div class="join-profile-buttons">
                                    <button class="btn btn-sm" on:click=move |_| set_join_step.set(false)>
                                        {icons::icon_arrow_left()} " Back"
                                    </button>
                                    <button class="btn btn-primary welcome-btn" on:click=confirm>
                                        "Join Server"
                                    </button>
                                </div>
                            </div>
                        }.into_any()
                    } else {
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
                                    "Next " {icons::icon_arrow_right()}
                                </button>
                            </div>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}
