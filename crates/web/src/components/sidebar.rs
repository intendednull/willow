use std::collections::HashMap;

use leptos::prelude::*;

use crate::app::ClientHandle;

/// Left sidebar showing the server name, channel list, and user info.
#[component]
pub fn Sidebar(
    channels: ReadSignal<Vec<String>>,
    current_channel: ReadSignal<String>,
    open: ReadSignal<bool>,
    unread: ReadSignal<HashMap<String, usize>>,
    connection_status: ReadSignal<String>,
    peer_count: ReadSignal<usize>,
    client: ClientHandle,
    on_channel_click: impl Fn(String) + Send + Clone + 'static,
    on_settings_click: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    let (creating, set_creating) = signal(false);
    let (new_name, set_new_name) = signal(String::new());

    let client_create = client.clone();
    let on_create_submit = move || {
        let name = new_name.get_untracked();
        let name = name.trim().to_string();
        if !name.is_empty() {
            let mut c = client_create.borrow_mut();
            let _ = c.create_channel(&name);
        }
        set_new_name.set(String::new());
        set_creating.set(false);
    };

    let on_create_keydown = {
        let submit = on_create_submit.clone();
        move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Enter" {
                ev.prevent_default();
                submit();
            } else if ev.key() == "Escape" {
                set_creating.set(false);
                set_new_name.set(String::new());
            }
        }
    };

    let client_user = client.clone();

    view! {
        <div class=move || if open.get() { "sidebar open" } else { "sidebar" }>
            <div class="sidebar-header">"Willow"</div>
            <div class="channel-list">
                <div class="channel-list-header">
                    <span class="channel-list-title">"CHANNELS"</span>
                    <button
                        class="channel-add-btn"
                        title="Create channel"
                        on:click=move |_| set_creating.set(true)
                    >
                        "+"
                    </button>
                </div>
                {move || {
                    if creating.get() {
                        Some(view! {
                            <div class="channel-create-input">
                                <input
                                    type="text"
                                    placeholder="channel name"
                                    prop:value=move || new_name.get()
                                    on:input=move |ev| set_new_name.set(event_target_value(&ev))
                                    on:keydown=on_create_keydown.clone()
                                    on:blur=move |_| {
                                        // Delay to allow click events to fire.
                                        set_creating.set(false);
                                        set_new_name.set(String::new());
                                    }
                                />
                            </div>
                        })
                    } else {
                        None
                    }
                }}
                <For
                    each=move || channels.get()
                    key=|ch| ch.clone()
                    let:channel
                >
                    {
                        let ch_active = channel.clone();
                        let ch_click = channel.clone();
                        let ch_delete = channel.clone();
                        let ch_unread = channel.clone();
                        let on_click = on_channel_click.clone();
                        let client_del = client.clone();
                        let active = move || current_channel.get() == ch_active;
                        view! {
                            <div
                                class=move || if active() { "channel-item active" } else { "channel-item" }
                                on:click=move |_| on_click(ch_click.clone())
                            >
                                <span>"# " {channel.clone()}</span>
                                <span class="channel-item-right">
                                    {
                                        let ch_u = ch_unread.clone();
                                        move || {
                                            let counts = unread.get();
                                            counts.get(&ch_u).copied().filter(|c| *c > 0).map(|c| {
                                                view! {
                                                    <span class="unread-badge">{c.to_string()}</span>
                                                }
                                            })
                                        }
                                    }
                                    {
                                        let ch_d = ch_delete.clone();
                                        let cl = client_del.clone();
                                        view! {
                                            <button
                                                class="delete-btn"
                                                title="Delete channel"
                                                on:click=move |ev| {
                                                    ev.stop_propagation();
                                                    let mut c = cl.borrow_mut();
                                                    let _ = c.delete_channel(&ch_d);
                                                }
                                            >
                                                "x"
                                            </button>
                                        }
                                    }
                                </span>
                            </div>
                        }
                    }
                </For>
            </div>
            <div class="connection-status">
                <span class=move || {
                    let status = connection_status.get();
                    match status.as_str() {
                        "connected" => "status-dot connected",
                        "connecting" => "status-dot connecting",
                        _ => "status-dot disconnected",
                    }
                }></span>
                <span class="connection-text">{move || {
                    let status = connection_status.get();
                    let n = peer_count.get();
                    match status.as_str() {
                        "connected" => {
                            if n == 1 {
                                "Connected (1 peer)".to_string()
                            } else {
                                format!("Connected ({n} peers)")
                            }
                        }
                        "connecting" => "Connecting...".to_string(),
                        _ => "Disconnected".to_string(),
                    }
                }}</span>
            </div>
            <div class="user-area">
                <div class="status-dot"></div>
                <span style="font-size: 12px; color: var(--text-muted);">
                    {
                        let client_name = client_user.clone();
                        move || {
                            let c = client_name.borrow();
                            let name = c.display_name();
                            if name.len() > 20 { format!("{}...", &name[..20]) } else { name }
                        }
                    }
                </span>
                <button
                    class="btn btn-sm theme-toggle"
                    title="Toggle theme"
                    on:click=move |_| crate::app::toggle_theme()
                >
                    {move || {
                        let is_dark = js_sys::eval(
                            "document.documentElement.getAttribute('data-theme') !== 'light'"
                        ).ok().and_then(|v| v.as_bool()).unwrap_or(true);
                        if is_dark { "\u{2600}\u{fe0f}" } else { "\u{1f319}" }
                    }}
                </button>
                <button
                    class="btn btn-sm"
                    style="margin-left: auto; background: transparent; color: var(--text-muted);"
                    on:click=move |_| on_settings_click(())
                >
                    "Settings"
                </button>
            </div>
        </div>
    }
}
