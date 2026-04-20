use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;

use crate::app::WebClientHandle;
use crate::icons;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AddServerTab {
    Create,
    Join,
}

/// Panel for creating a new server or joining an existing one via invite code.
///
/// Internally tabbed — only one flow is visible at a time. Used on the
/// welcome screen and on the sidebar "+" add-server surface.
#[component]
pub fn AddServerPanel(on_done: impl Fn(()) + Send + Clone + 'static) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();

    let (active_tab, set_active_tab) = signal(AddServerTab::Create);

    // Create server state.
    let (server_name, set_server_name) = signal(String::new());
    let (create_display_name, set_create_display_name) = signal(String::new());
    let (status_msg, set_status_msg) = signal(String::new());

    // Join server state.
    let (join_code, set_join_code) = signal(String::new());
    let (join_step, set_join_step) = signal(false);
    let (join_profile_name, set_join_profile_name) = signal(String::new());
    let (validated_code, set_validated_code) = signal(String::new());

    let app_state = use_context::<crate::state::AppState>().unwrap();
    set_join_profile_name.set(app_state.server.display_name.get_untracked());

    let handle_create = handle.clone();
    let on_done_create = on_done.clone();
    let on_create = move |_| {
        let name = server_name.get_untracked();
        if name.trim().is_empty() {
            set_status_msg.set("Please enter a server name.".to_string());
            return;
        }
        let h = handle_create.clone();
        let n = name.trim().to_string();
        let dn = create_display_name.get_untracked();
        let done_cb = on_done_create.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match h.create_server(&n).await {
                Ok(_) => {
                    if !dn.trim().is_empty() {
                        h.set_server_display_name(dn.trim()).await.ok();
                    }
                    done_cb(());
                }
                Err(e) => set_status_msg.set(format!("Error: {e}")),
            }
        });
    };

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

    let handle_join = SendWrapper::new(Rc::new(handle.clone()));
    let on_done_rc: SendWrapper<Rc<dyn Fn(())>> =
        SendWrapper::new(Rc::new(on_done) as Rc<dyn Fn(())>);

    view! {
        <div class="welcome-tabs" role="tablist">
            <button
                type="button"
                class=move || {
                    if active_tab.get() == AddServerTab::Create {
                        "welcome-tab-btn active"
                    } else {
                        "welcome-tab-btn"
                    }
                }
                role="tab"
                aria-selected=move || (active_tab.get() == AddServerTab::Create).to_string()
                on:click=move |_| {
                    set_status_msg.set(String::new());
                    set_active_tab.set(AddServerTab::Create);
                }
            >
                "Create"
            </button>
            <button
                type="button"
                class=move || {
                    if active_tab.get() == AddServerTab::Join {
                        "welcome-tab-btn active"
                    } else {
                        "welcome-tab-btn"
                    }
                }
                role="tab"
                aria-selected=move || (active_tab.get() == AddServerTab::Join).to_string()
                on:click=move |_| {
                    set_status_msg.set(String::new());
                    set_active_tab.set(AddServerTab::Join);
                }
            >
                "Join"
            </button>
        </div>

        {move || {
            let msg = status_msg.get();
            if msg.is_empty() {
                None
            } else {
                Some(view! { <div class="settings-status">{msg}</div> })
            }
        }}

        <div class="welcome-tab-panel">
            {move || match active_tab.get() {
                AddServerTab::Create => {
                    let on_create = on_create.clone();
                    view! {
                        <div class="welcome-option">
                            <p class="welcome-hint">
                                "A grove is your own server — you decide who joins."
                            </p>
                            <label>"Grove name"</label>
                            <input
                                type="text"
                                placeholder="backyard"
                                prop:value=move || server_name.get()
                                on:input=move |ev| set_server_name.set(event_target_value(&ev))
                            />
                            <label>"Display name · optional"</label>
                            <input
                                type="text"
                                placeholder="what peers should call you"
                                prop:value=move || create_display_name.get()
                                on:input=move |ev| set_create_display_name.set(event_target_value(&ev))
                            />
                            <button class="btn btn-primary welcome-btn" on:click=on_create>
                                "Plant grove"
                            </button>
                        </div>
                    }.into_any()
                }
                AddServerTab::Join => {
                    if join_step.get() {
                        let hj = handle_join.clone();
                        let done_cb = on_done_rc.clone();
                        let confirm = move |_: web_sys::MouseEvent| {
                            let code = validated_code.get_untracked();
                            let h = hj.clone();
                            let done = done_cb.clone();
                            let name = join_profile_name.get_untracked();
                            wasm_bindgen_futures::spawn_local(async move {
                                match h.accept_invite(&code).await {
                                    Ok(()) => {
                                        if !name.trim().is_empty() {
                                            h.set_server_display_name(name.trim()).await.ok();
                                        }
                                        set_join_code.set(String::new());
                                        set_join_step.set(false);
                                        (done)(());
                                    }
                                    Err(e) => {
                                        set_status_msg.set(format!("Invalid invite code: {e}"));
                                        set_join_step.set(false);
                                    }
                                }
                            });
                        };
                        view! {
                            <div class="welcome-option">
                                <label>"Display name for this grove"</label>
                                <p class="welcome-hint">"Pre-filled with your current name."</p>
                                <input
                                    type="text"
                                    placeholder="your name…"
                                    prop:value=move || join_profile_name.get()
                                    on:input=move |ev| set_join_profile_name.set(event_target_value(&ev))
                                />
                                <div class="join-profile-buttons">
                                    <button class="btn btn-sm" on:click=move |_| set_join_step.set(false)>
                                        {icons::icon_arrow_left()} " Back"
                                    </button>
                                    <button class="btn btn-primary welcome-btn" on:click=confirm>
                                        "Join grove"
                                    </button>
                                </div>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div class="welcome-option">
                                <p class="welcome-hint">
                                    "Paste the letter of introduction you were sent."
                                </p>
                                <label>"Invite code"</label>
                                <textarea
                                    class="welcome-invite-input"
                                    placeholder="paste willow://… here"
                                    prop:value=move || join_code.get()
                                    on:input=move |ev| set_join_code.set(event_target_value(&ev))
                                ></textarea>
                                <button class="btn btn-primary welcome-btn" on:click=on_join_next>
                                    "Open letter " {icons::icon_arrow_right()}
                                </button>
                            </div>
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}
