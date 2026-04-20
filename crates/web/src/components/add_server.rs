use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;

use crate::app::WebClientHandle;
use crate::icons;
use crate::util::copy_to_clipboard;

#[derive(Clone, Copy, PartialEq, Eq)]
enum AddServerTab {
    Create,
    Join,
}

/// Panel for creating a new server or joining an existing one via invite code.
///
/// Tabbed Create / Join flows. The Join tab exposes the local peer id so the
/// user can share it with an existing grove owner to receive an invite. A
/// single `display_name` signal is supplied by the caller so the same value
/// applies to whichever path the user takes.
#[component]
pub fn AddServerPanel(
    on_done: impl Fn(()) + Send + Clone + 'static,
    display_name: ReadSignal<String>,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();
    let peer_id = handle.peer_id();
    let (copy_label, set_copy_label) = signal("copy");
    let (show_pid, set_show_pid) = signal(false);

    let (active_tab, set_active_tab) = signal(AddServerTab::Create);

    let (server_name, set_server_name) = signal(String::new());
    let (status_msg, set_status_msg) = signal(String::new());

    let (join_code, set_join_code) = signal(String::new());
    let (join_step, set_join_step) = signal(false);
    let (validated_code, set_validated_code) = signal(String::new());

    let handle_create = handle.clone();
    let on_done_create = on_done.clone();
    let on_create = move |_| {
        let name = server_name.get_untracked();
        if name.trim().is_empty() {
            set_status_msg.set("Please name your grove".to_string());
            return;
        }
        let h = handle_create.clone();
        let n = name.trim().to_string();
        let dn_raw = display_name.get_untracked();
        let dn_trimmed = dn_raw.trim();
        let dn = if dn_trimmed.is_empty() {
            "anonymous".to_string()
        } else {
            dn_trimmed.to_string()
        };
        let done_cb = on_done_create.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match h.create_server(&n).await {
                Ok(_) => {
                    h.set_server_display_name(&dn).await.ok();
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
                            <label>"Name your grove"</label>
                            <input
                                type="text"
                                placeholder="backyard"
                                prop:value=move || server_name.get()
                                on:input=move |ev| set_server_name.set(event_target_value(&ev))
                            />
                            <button class="btn btn-primary welcome-btn" on:click=on_create>
                                "continue"
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
                            let raw = display_name.get_untracked();
                            let trimmed = raw.trim();
                            let name = if trimmed.is_empty() {
                                "anonymous".to_string()
                            } else {
                                trimmed.to_string()
                            };
                            wasm_bindgen_futures::spawn_local(async move {
                                match h.accept_invite(&code).await {
                                    Ok(()) => {
                                        h.set_server_display_name(&name).await.ok();
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
                                <p class="welcome-hint">
                                    "Ready to join — confirm and you're in."
                                </p>
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
                        let pid_full = peer_id.clone();
                        let pid_full_copy = pid_full.clone();
                        let pid_full_reveal = pid_full.clone();
                        view! {
                            <div class="welcome-option">
                                <ol class="welcome-join-steps">
                                    <li class="welcome-join-steps__share">
                                        <span class="welcome-join-steps__text">
                                            "Share your id with a grove steward"
                                        </span>
                                        <div class="welcome-join-steps__controls">
                                            <button
                                                type="button"
                                                class="welcome-join-icon-btn"
                                                aria-label="copy your peer id"
                                                title={move || copy_label.get().to_string()}
                                                on:click={
                                                    let pid = pid_full_copy.clone();
                                                    move |_| {
                                                        copy_to_clipboard(&pid);
                                                        set_copy_label.set("copied");
                                                        set_timeout(
                                                            move || set_copy_label.set("copy"),
                                                            std::time::Duration::from_secs(2),
                                                        );
                                                    }
                                                }
                                            >
                                                {icons::icon_copy()}
                                            </button>
                                            <button
                                                type="button"
                                                class=move || {
                                                    if show_pid.get() {
                                                        "welcome-join-icon-btn active"
                                                    } else {
                                                        "welcome-join-icon-btn"
                                                    }
                                                }
                                                aria-label="show full peer id"
                                                aria-pressed=move || show_pid.get().to_string()
                                                on:click=move |_| set_show_pid.update(|v| *v = !*v)
                                            >
                                                {icons::icon_eye()}
                                            </button>
                                        </div>
                                        {move || show_pid.get().then(|| view! {
                                            <code
                                                class="peer-id-text welcome-join-steps__full-id"
                                                data-full-id={pid_full_reveal.clone()}
                                            >
                                                {pid_full_reveal.clone()}
                                            </code>
                                        })}
                                    </li>
                                    <li>"They send back a grove invite"</li>
                                    <li>"Paste the invite code below"</li>
                                </ol>
                                <label>"Invite code"</label>
                                <textarea
                                    class="welcome-invite-input"
                                    placeholder="paste willow://… here"
                                    prop:value=move || join_code.get()
                                    on:input=move |ev| set_join_code.set(event_target_value(&ev))
                                ></textarea>
                                <button class="btn btn-primary welcome-btn" on:click=on_join_next>
                                    "continue"
                                </button>
                            </div>
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}
