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
    server_name: ReadSignal<String>,
    client: ClientHandle,
    on_channel_click: impl Fn(String) + Send + Clone + 'static,
    on_settings_click: impl Fn(()) + Send + Clone + 'static,
    on_server_settings_click: impl Fn(()) + Send + Clone + 'static,
    on_voice_join: impl Fn(String) + Send + Clone + 'static,
) -> impl IntoView {
    let (creating, set_creating) = signal(false);
    let (new_name, set_new_name) = signal(String::new());
    let (create_voice, set_create_voice) = signal(false);

    let client_create = client.clone();
    let on_create_submit = move || {
        let name = new_name.get_untracked();
        let name = name.trim().to_string();
        let is_voice = create_voice.get_untracked();
        if !name.is_empty() {
            let mut c = client_create.borrow_mut();
            if is_voice {
                let _ = c.create_voice_channel(&name);
            } else {
                let _ = c.create_channel(&name);
            }
        }
        set_new_name.set(String::new());
        set_creating.set(false);
        set_create_voice.set(false);
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
            <div class="sidebar-header">
                <span>{move || server_name.get()}</span>
                <button
                    class="btn btn-sm server-gear-btn"
                    title="Server Settings"
                    on:click=move |_| on_server_settings_click(())
                >
                    "\u{2699}\u{fe0f}"
                </button>
            </div>
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
                                <div class="channel-type-toggle">
                                    <button
                                        class=move || if !create_voice.get() { "type-btn active" } else { "type-btn" }
                                        on:mousedown=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            set_create_voice.set(false);
                                        }
                                    >"# Text"</button>
                                    <button
                                        class=move || if create_voice.get() { "type-btn active" } else { "type-btn" }
                                        on:mousedown=move |ev: web_sys::MouseEvent| {
                                            ev.prevent_default();
                                            set_create_voice.set(true);
                                        }
                                    >"\u{1F50A} Voice"</button>
                                </div>
                                <input
                                    type="text"
                                    placeholder=move || if create_voice.get() { "voice channel name" } else { "channel name" }
                                    prop:value=move || new_name.get()
                                    on:input=move |ev| set_new_name.set(event_target_value(&ev))
                                    on:keydown=on_create_keydown.clone()
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
                        let ch_kind = channel.clone();
                        let ch_voice_join = channel.clone();
                        let on_click = on_channel_click.clone();
                        let on_voice = on_voice_join.clone();
                        let client_del = client.clone();
                        let client_kind = client.clone();
                        let active = move || current_channel.get() == ch_active;

                        // Check if this is a voice channel.
                        let is_voice = {
                            let c = client_kind.borrow();
                            c.channel_kinds().iter().any(|(n, k)| n == &ch_kind && k == "voice")
                        };
                        let prefix = if is_voice { "\u{1F50A} " } else { "# " };

                        view! {
                            <div
                                class=move || {
                                    let base = if is_voice { "channel-item voice-channel" } else { "channel-item" };
                                    if active() { format!("{base} active") } else { base.to_string() }
                                }
                                on:click={
                                    let ch_text = ch_click.clone();
                                    let ch_vc = ch_voice_join.clone();
                                    move |_| {
                                        if is_voice {
                                            on_voice(ch_vc.clone());
                                        } else {
                                            on_click(ch_text.clone());
                                        }
                                    }
                                }
                            >
                                <span>{prefix} {channel.clone()}</span>
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
