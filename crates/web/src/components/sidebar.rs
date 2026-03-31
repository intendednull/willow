use std::collections::HashMap;

use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::{ConfirmDialog, VoiceControls};
use crate::icons;

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
    on_channel_click: impl Fn(String) + Send + Clone + 'static,
    on_settings_click: impl Fn(()) + Send + Clone + 'static,
    on_server_settings_click: impl Fn(()) + Send + Clone + 'static,
    on_voice_join: impl Fn(String) + Send + Clone + 'static,
    on_channel_created: impl Fn(()) + Send + Clone + 'static,
    /// Voice channel the user is currently in, if any.
    #[prop(optional)]
    voice_channel: Option<ReadSignal<Option<String>>>,
    /// Name of the current voice channel.
    #[prop(optional)]
    voice_channel_name: Option<ReadSignal<String>>,
    /// Whether the local mic is muted.
    #[prop(optional)]
    voice_muted: Option<ReadSignal<bool>>,
    /// Whether the local audio output is deafened.
    #[prop(optional)]
    voice_deafened: Option<ReadSignal<bool>>,
    /// Called when the mute button is clicked.
    #[prop(optional)]
    on_voice_mute: Option<Callback<()>>,
    /// Called when the deafen button is clicked.
    #[prop(optional)]
    on_voice_deafen: Option<Callback<()>>,
    /// Called when the disconnect button is clicked.
    #[prop(optional)]
    on_voice_disconnect: Option<Callback<()>>,
) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();
    let app_state = use_context::<crate::state::AppState>().unwrap();

    let (creating, set_creating) = signal(false);
    let (new_name, set_new_name) = signal(String::new());
    let (create_voice, set_create_voice) = signal(false);

    // Channel delete confirmation state.
    let (show_del_confirm, set_show_del_confirm) = signal(false);
    let (pending_del_channel, set_pending_del_channel) = signal(Option::<String>::None);
    let handle_del_confirm = handle.clone();

    let handle_create = handle.clone();
    let on_create_submit = move || {
        let name = new_name.get_untracked();
        let name = name.trim().to_string();
        let is_voice = create_voice.get_untracked();
        if !name.is_empty() {
            if is_voice {
                let _ = handle_create.create_voice_channel(&name);
            } else {
                let _ = handle_create.create_channel(&name);
            }
            on_channel_created(());
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

    // Peer ID copy state.
    let (show_copied, set_show_copied) = signal(false);
    let handle_copy = handle.clone();

    view! {
        <div class=move || if open.get() { "sidebar open" } else { "sidebar" }>
            <div class="sidebar-header">
                <span>{move || server_name.get()}</span>
                <button
                    class="btn btn-sm server-gear-btn"
                    title="Server Settings"
                    on:click=move |_| on_server_settings_click(())
                >
                    {icons::icon_settings()}
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
                        {icons::icon_plus()}
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
                                    >{icons::icon_volume_2()} " Voice"</button>
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
                        let handle_kind = handle.clone();
                        let active = move || current_channel.get() == ch_active;

                        // Reactively check if this is a voice channel.
                        let is_voice = {
                            let name = ch_kind.clone();
                            let _hk = handle_kind.clone();
                            move || {
                                app_state.server.channel_kinds.get().iter().any(|(n, k)| n == &name && k == "voice")
                            }
                        };

                        let is_voice_for_class = is_voice.clone();
                        let is_voice_for_prefix = is_voice.clone();

                        view! {
                            <div
                                class=move || {
                                    let base = if is_voice_for_class() { "channel-item voice-channel" } else { "channel-item" };
                                    if active() { format!("{base} active") } else { base.to_string() }
                                }
                                on:click={
                                    let ch_text = ch_click.clone();
                                    let ch_vc = ch_voice_join.clone();
                                    move |_| {
                                        if is_voice() {
                                            on_voice(ch_vc.clone());
                                        } else {
                                            on_click(ch_text.clone());
                                        }
                                    }
                                }
                            >
                                <span>{move || if is_voice_for_prefix() { icons::icon_volume_2().into_any() } else { icons::icon_hash().into_any() }} " " {channel.clone()}</span>
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
                                        view! {
                                            <button
                                                class="delete-btn"
                                                title="Delete channel"
                                                on:click=move |ev| {
                                                    ev.stop_propagation();
                                                    set_pending_del_channel.set(Some(ch_d.clone()));
                                                    set_show_del_confirm.set(true);
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
                        "connecting" | "reconnecting" => "status-dot connecting",
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
                        "reconnecting" => "Reconnecting...".to_string(),
                        _ => "Disconnected".to_string(),
                    }
                }}</span>
            </div>
            {move || {
                let vc = voice_channel.and_then(|s| s.get());
                if vc.is_some() {
                    let vcn = voice_channel_name.expect("voice_channel_name required");
                    let vm = voice_muted.expect("voice_muted required");
                    let vd = voice_deafened.expect("voice_deafened required");
                    let on_m = on_voice_mute.expect("on_voice_mute required");
                    let on_d = on_voice_deafen.expect("on_voice_deafen required");
                    let on_dc = on_voice_disconnect.expect("on_voice_disconnect required");
                    Some(view! {
                        <VoiceControls
                            channel_name=vcn
                            muted=vm
                            deafened=vd
                            on_mute=move |v| on_m.run(v)
                            on_deafen=move |v| on_d.run(v)
                            on_disconnect=move |v| on_dc.run(v)
                        />
                    })
                } else {
                    None
                }
            }}
            <div class="user-area">
                <div class="status-dot"></div>
                <span style="font-size: 12px; color: var(--text-muted);">
                    {
                        move || {
                            let name = app_state.server.display_name.get();
                            if name.len() > 20 { format!("{}...", &name[..20]) } else { name }
                        }
                    }
                </span>
                <button class="copy-pid-btn" title="Copy Peer ID" on:click=move |_| {
                    let id = handle_copy.peer_id();
                    crate::util::copy_to_clipboard(&id);
                    set_show_copied.set(true);
                    set_timeout(move || set_show_copied.set(false), std::time::Duration::from_millis(1500));
                }>
                    {icons::icon_copy()}
                </button>
                {move || show_copied.get().then(|| view! {
                    <span class="copied-tooltip">"Copied!"</span>
                })}
                <button
                    class="btn btn-sm theme-toggle"
                    title="Toggle theme"
                    on:click=move |_| crate::app::toggle_theme()
                >
                    {move || {
                        let is_dark = js_sys::eval(
                            "document.documentElement.getAttribute('data-theme') !== 'light'"
                        ).ok().and_then(|v| v.as_bool()).unwrap_or(true);
                        if is_dark { icons::icon_sun().into_any() } else { icons::icon_moon().into_any() }
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
            <ConfirmDialog
                visible=show_del_confirm
                title="Delete Channel"
                message=Signal::derive(move || {
                    pending_del_channel.get()
                        .map(|n| format!("Delete #{}?", n))
                        .unwrap_or_default()
                })
                confirm_text="Delete"
                danger=true
                on_confirm=Callback::new(move |_| {
                    if let Some(name) = pending_del_channel.get_untracked() {
                        let _ = handle_del_confirm.delete_channel(&name);
                    }
                    set_pending_del_channel.set(None);
                    set_show_del_confirm.set(false);
                })
                on_cancel=Callback::new(move |_| {
                    set_pending_del_channel.set(None);
                    set_show_del_confirm.set(false);
                })
            />
        </div>
    }
}
