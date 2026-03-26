use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use willow_client::{ClientConfig, ClientEvent, ClientHandle, DisplayMessage, VoiceSignalPayload};

use crate::components::{
    AddServerPanel, CallPage, ChannelHeader, ChatInput, CommandPalette, FileShareButton,
    MemberList, MessageList, PinnedPanel, ServerList, SettingsPanel, Sidebar, WelcomeScreen,
};
use crate::event_processing::{extract_roles, process_event_batch, refresh_all_signals};
use crate::handlers;
use crate::icons;
use crate::state::{self, ChannelViewState, SettingsTab};
use crate::voice::VoiceManager;

// Notification sounds disabled for now.
// fn play_notification_sound() { ... }

fn init_theme() {
    let _ = js_sys::eval(
        r#"var t=localStorage.getItem('willow-theme')||'dark';document.documentElement.setAttribute('data-theme',t);"#,
    );
}

pub fn toggle_theme() {
    let _ = js_sys::eval(
        r#"var h=document.documentElement;var c=h.getAttribute('data-theme')||'dark';var n=c==='dark'?'light':'dark';h.setAttribute('data-theme',n);localStorage.setItem('willow-theme',n);"#,
    );
}

/// How many milliseconds to wait before clearing the loading state automatically.
const LOADING_TIMEOUT_MS: u32 = 5_000;

/// Wrapper around `willow_client::ClientHandle` that is `Send` for single-threaded WASM.
pub type WebClientHandle = SendWrapper<ClientHandle>;

/// Wrapper around `Rc<RefCell<VoiceManager>>` that is `Send` for single-threaded WASM.
pub type VoiceManagerHandle = SendWrapper<Rc<RefCell<VoiceManager>>>;

/// Default relay address for the deployed Willow relay server.
pub const DEFAULT_RELAY: &str =
    "/dns4/willow.intendednull.com/tcp/9443/wss/p2p/12D3KooWMBmUF1rHYG5CneKi8JZfKdMAciJd4oCgknTJkbwCUurd";

fn new_client() -> (WebClientHandle, willow_client::ClientEventLoop) {
    let config = ClientConfig {
        relay_addr: Some(DEFAULT_RELAY.to_string()),
        ..ClientConfig::default()
    };
    let (handle, event_loop) = ClientHandle::new(config);
    (SendWrapper::new(handle), event_loop)
}

/// Root application component. Creates the `ClientHandle`, connects to the P2P
/// network, and spawns async event processing to bridge client state into
/// reactive signals.
#[component]
pub fn App() -> impl IntoView {
    init_theme();

    // Create and connect the client.
    let (handle, event_loop) = new_client();
    handle.connect();

    // Create all signals.
    let (app_state, write) = state::create_signals();

    // Provide context so child components can access the handle and state.
    provide_context(handle.clone());
    provide_context(app_state);
    provide_context(write);

    // Create the VoiceManager.
    let local_peer_id = handle.peer_id();
    let voice_signal_handle = handle.clone();
    let voice_channel_for_signal = app_state.voice.voice_channel;
    let set_remote_streams = write.voice.set_remote_video_streams;
    let set_speaking = write.voice.set_speaking_peers;
    let voice_manager: VoiceManagerHandle =
        SendWrapper::new(Rc::new(RefCell::new(VoiceManager::new(
            local_peer_id,
            move |target_peer: &str, signal_type: &str, payload: &str| {
                let ch_id = voice_channel_for_signal.get_untracked().unwrap_or_default();
                let signal = match signal_type {
                    "offer" => VoiceSignalPayload::Offer(payload.to_string()),
                    "answer" => VoiceSignalPayload::Answer(payload.to_string()),
                    "ice" => VoiceSignalPayload::IceCandidate(payload.to_string()),
                    _ => return,
                };
                voice_signal_handle.send_voice_signal(&ch_id, target_peer, signal);
            },
            move |peer_id: &str, stream: Option<web_sys::MediaStream>| {
                let pid = peer_id.to_string();
                set_remote_streams.update(move |map| {
                    if let Some(s) = stream {
                        map.insert(pid, send_wrapper::SendWrapper::new(s));
                    } else {
                        map.remove(&pid);
                    }
                });
            },
            move |peers: std::collections::HashSet<String>| {
                set_speaking.set(peers);
            },
        ))));

    provide_context(voice_manager.clone());

    // Auto-clear loading after LOADING_TIMEOUT_MS even if no peer connects.
    {
        let w = write;
        set_timeout(
            move || {
                w.network.set_loading.set(false);
            },
            std::time::Duration::from_millis(LOADING_TIMEOUT_MS as u64),
        );
    }

    // Register Ctrl+K / Cmd+K for command palette.
    {
        use wasm_bindgen::JsCast;
        let write_for_palette = write;
        let closure = wasm_bindgen::closure::Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(
            move |ev: web_sys::KeyboardEvent| {
                if (ev.ctrl_key() || ev.meta_key()) && ev.key() == "k" {
                    ev.prevent_default();
                    write_for_palette.ui.set_show_palette.update(|v| *v = !*v);
                }
            },
        );
        if let Some(window) = web_sys::window() {
            let _ = window
                .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
        }
        closure.forget();
    }

    // Populate initial state from the client.
    refresh_all_signals(&handle, &write);

    // Spawn the event loop and signal updater.
    {
        let handle_for_events = handle.clone();
        let write_for_events = write;
        let state_for_events = app_state;
        let vm_for_events = voice_manager.clone();

        let (client_event_tx, mut client_event_rx) =
            futures::channel::mpsc::unbounded::<ClientEvent>();

        // Spawn the event loop — processes network events and sends ClientEvents.
        wasm_bindgen_futures::spawn_local(event_loop.run(client_event_tx));

        // Spawn the signal updater — receives ClientEvents and updates signals.
        wasm_bindgen_futures::spawn_local(async move {
            use futures::StreamExt;
            while let Some(event) = client_event_rx.next().await {
                // Collect all pending events into a batch.
                let mut batch = vec![event];
                while let Ok(ev) = client_event_rx.try_recv() {
                    batch.push(ev);
                }
                process_event_batch(
                    &batch,
                    &handle_for_events,
                    &state_for_events,
                    &write_for_events,
                    &vm_for_events,
                );
            }
        });
    }

    // Spawn typing expiry timer (refreshes typing indicators every 2s).
    {
        let handle_typing = handle.clone();
        let state_typing = app_state;
        let write_typing = write;

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(2_000).await;
                let ch = state_typing.chat.current_channel.get_untracked();
                let typers = handle_typing.typing_in(&ch);
                // Read the current channel_views, extract typing for this channel.
                let current_views = state_typing.chat.channel_views.get_untracked();
                let current_typing = current_views
                    .get(&ch)
                    .map(|v| v.typing.clone())
                    .unwrap_or_default();
                if typers != current_typing {
                    let mut views = current_views;
                    views
                        .entry(ch)
                        .or_insert_with(ChannelViewState::default)
                        .typing = typers;
                    write_typing.chat.set_channel_views.set(views);
                }
            }
        });
    }

    // Build handler closures.
    let on_send = handlers::make_send_handler(handle.clone(), app_state, write);
    let on_edit_send = handlers::make_edit_handler(handle.clone(), app_state, write);
    let on_delete_msg = handlers::make_delete_handler(handle.clone(), app_state, write);
    let on_react = handlers::make_react_handler(handle.clone(), app_state, write);
    let on_channel_click = handlers::make_channel_click_handler(handle.clone(), app_state, write);
    let on_server_click = handlers::make_server_click_handler(handle.clone(), write);
    let on_pin = handlers::make_pin_handler(handle.clone(), app_state, write);

    // Voice mute handler.
    let vm_mute = voice_manager.clone();
    let on_voice_mute = move |_: ()| {
        let new_muted = !app_state.voice.voice_muted.get_untracked();
        write.voice.set_voice_muted.set(new_muted);
        vm_mute.borrow().set_muted(new_muted);
    };

    // Voice deafen handler.
    let vm_deafen = voice_manager.clone();
    let on_voice_deafen = move |_: ()| {
        let new_deafened = !app_state.voice.voice_deafened.get_untracked();
        write.voice.set_voice_deafened.set(new_deafened);
        if new_deafened {
            write.voice.set_voice_muted.set(true);
            vm_deafen.borrow().set_muted(true);
        } else {
            write.voice.set_voice_muted.set(false);
            vm_deafen.borrow().set_muted(false);
        }
    };

    // Voice disconnect handler.
    let vm_disconnect = voice_manager.clone();
    let handle_voice_leave = handle.clone();
    let on_voice_disconnect = move |_: ()| {
        handle_voice_leave.leave_voice();
        vm_disconnect.borrow_mut().close_all();
        write.voice.set_voice_channel.set(None);
        write.voice.set_voice_channel_name.set(String::new());
        write.voice.set_voice_muted.set(false);
        write.voice.set_voice_deafened.set(false);
        write.voice.set_video_source.set(None);
        write.voice.set_local_video_stream.set(None);
        write.voice.set_remote_video_streams.update(|m| m.clear());
        write
            .voice
            .set_speaking_peers
            .set(std::collections::HashSet::new());
        write
            .voice
            .set_voice_participants_map
            .update(|m| m.clear());
        write.ui.set_show_call_page.set(false);
        write
            .ui
            .set_call_layout
            .set(crate::state::CallLayout::default());
    };

    // Welcome screen callback that refreshes all signals.
    let handle_welcome = handle.clone();
    let on_welcome_done = move |_: ()| {
        refresh_all_signals(&handle_welcome, &write);
    };

    // Store refresh function for reactive closures.
    let handle_for_refresh = handle.clone();
    let refresh_stored = StoredValue::new(SendWrapper::new(Rc::new(move || {
        refresh_all_signals(&handle_for_refresh, &write);
    }) as Rc<dyn Fn()>));

    // Aliases for view closures.
    let servers = app_state.server.servers;
    let show_sidebar = app_state.ui.show_sidebar;
    let show_add_server = app_state.ui.show_add_server;
    let show_settings = app_state.ui.show_settings;
    let show_pinned = app_state.ui.show_pinned;
    let show_members = app_state.ui.show_members;
    let current_channel = app_state.chat.current_channel;
    let messages = app_state.chat.messages;
    let pinned_messages = app_state.chat.pinned_messages;
    let pin_labels = app_state.chat.pin_labels;
    let loading = app_state.network.loading;
    let display_name = app_state.server.display_name;
    let peer_count = app_state.network.peer_count;
    let peer_id = app_state.network.peer_id;
    let _roles = app_state.server.roles;
    let replying_to = app_state.chat.replying_to;
    let editing = app_state.chat.editing;
    let channel_views = app_state.chat.channel_views;
    let show_palette = app_state.ui.show_palette;
    let show_call_page = app_state.ui.show_call_page;

    // Pre-clone handle for use inside the view closure.
    let handle_for_voice_join = handle.clone();
    let handle_for_typing = handle.clone();
    let handle_for_ch_created = handle.clone();
    let vm_for_view = voice_manager.clone();

    view! {
        {move || {
            let srv = servers.get();
            if srv.is_empty() {
                let on_done = on_welcome_done.clone();
                view! {
                    <WelcomeScreen
                        on_done=on_done
                    />
                }.into_any()
            } else {
                let ch_click = on_channel_click.clone();
                let srv_click = on_server_click.clone();
                let ch_click_for_palette = on_channel_click.clone();
                let srv_click_for_palette = on_server_click.clone();
                let send = on_send.clone();
                let edit_send = on_edit_send.clone();
                let del_msg = on_delete_msg.clone();
                let react = on_react.clone();
                let on_mute = on_voice_mute.clone();
                let on_deafen = on_voice_deafen.clone();
                let on_disconnect = on_voice_disconnect.clone();
                let handle_vj = handle_for_voice_join.clone();
                let handle_ty = handle_for_typing.clone();
                let handle_cc = handle_for_ch_created.clone();
                let vm_v = vm_for_view.clone();
                let pin = on_pin.clone();
                view! {
                    <div class="app">
                        <ServerList
                            servers=app_state.server.servers
                            active_server_id=app_state.server.active_server_id
                            on_server_click=srv_click
                            on_add_server_click=move |_| {
                                write.ui.set_show_add_server.update(|v| *v = !*v);
                                write.ui.set_show_settings.set(false);
                                write.ui.set_show_sidebar.set(false);
                            }
                            on_open_settings=Callback::new(move |_| {
                                write.ui.set_settings_tab.set(SettingsTab::Server);
                                write.ui.set_show_settings.set(true);
                                write.ui.set_show_add_server.set(false);
                                write.ui.set_show_sidebar.set(false);
                            })
                        />
                        // Overlay to close sidebar on mobile tap
                        <div
                            class=move || if show_sidebar.get() { "sidebar-overlay open" } else { "sidebar-overlay" }
                            on:click=move |_| write.ui.set_show_sidebar.set(false)
                        />
                        <Sidebar
                            channels=app_state.chat.channels
                            current_channel=current_channel
                            open=show_sidebar
                            unread=app_state.server.unread
                            connection_status=app_state.network.connection_status
                            peer_count=peer_count
                            server_name=app_state.server.active_server_name
                            on_channel_click=ch_click
                            on_settings_click=move |_| {
                                write.ui.set_settings_tab.set(SettingsTab::Profile);
                                write.ui.set_show_settings.set(true);
                                write.ui.set_show_add_server.set(false);
                                write.ui.set_show_sidebar.set(false);
                            }
                            on_server_settings_click=move |_| {
                                write.ui.set_settings_tab.set(SettingsTab::Server);
                                write.ui.set_show_settings.set(true);
                                write.ui.set_show_add_server.set(false);
                                write.ui.set_show_sidebar.set(false);
                            }
                            on_voice_join={
                                let vc_handle = handle_vj.clone();
                                let vm = vm_v.clone();
                                move |channel_name: String| {
                                    write.ui.set_show_sidebar.set(false);

                                    // If in a different voice channel, disconnect from the old one first.
                                    let current_vc = app_state.voice.voice_channel.get_untracked();
                                    if current_vc.is_some() && current_vc.as_deref() != Some(&channel_name) {
                                        vc_handle.leave_voice();
                                        vm.borrow_mut().close_all();
                                        write.voice.set_voice_channel.set(None);
                                        write.voice.set_voice_channel_name.set(String::new());
                                        write.voice.set_voice_muted.set(false);
                                        write.voice.set_voice_deafened.set(false);
                                        write.voice.set_video_source.set(None);
                                        write.voice.set_local_video_stream.set(None);
                                        write.voice.set_remote_video_streams.update(|m| m.clear());
                                        write.voice.set_speaking_peers.set(std::collections::HashSet::new());
                                        write.voice.set_voice_participants_map.update(|m| m.clear());
                                    }

                                    // If already in this voice channel, just navigate to the call page.
                                    if app_state.voice.voice_channel.get_untracked() == Some(channel_name.clone()) {
                                        write.ui.set_show_call_page.set(true);
                                        write.ui.set_show_settings.set(false);
                                        write.ui.set_show_add_server.set(false);
                                        return;
                                    }

                                    // Request mic permission SYNCHRONOUSLY in the click handler
                                    // to preserve the user gesture chain (required on mobile).
                                    let window = web_sys::window().unwrap();
                                    let navigator = window.navigator();
                                    let Ok(media_devices) = navigator.media_devices() else {
                                        tracing::error!("No media devices available");
                                        return;
                                    };
                                    let constraints = web_sys::MediaStreamConstraints::new();
                                    constraints.set_audio(&true.into());
                                    constraints.set_video(&false.into());
                                    let Ok(promise) = media_devices.get_user_media_with_constraints(&constraints) else {
                                        tracing::error!("getUserMedia failed");
                                        return;
                                    };

                                    // Show controls and call page immediately (optimistic).
                                    write.voice.set_voice_channel.set(Some(channel_name.clone()));
                                    write.voice.set_voice_channel_name.set(channel_name.clone());
                                    write.ui.set_show_call_page.set(true);
                                    write.ui.set_show_settings.set(false);
                                    write.ui.set_show_add_server.set(false);

                                    // Handle the promise result asynchronously.
                                    let vc = vc_handle.clone();
                                    let vm2 = vm.clone();
                                    let ch_name = channel_name.clone();
                                    let on_success = wasm_bindgen::closure::Closure::once(move |stream: wasm_bindgen::JsValue| {
                                        use wasm_bindgen::JsCast;
                                        let stream: web_sys::MediaStream = stream.unchecked_into();
                                        vm2.borrow_mut().set_local_stream(stream);
                                        vc.join_voice(&ch_name);

                                        // Seed participants from client state after joining.
                                        // This ensures that on reconnect we pick up peers
                                        // who are already in the channel (their VoiceJoined
                                        // event was received before we joined).
                                        let parts = vc.voice_participants(&ch_name);
                                        write.voice.set_voice_participants_map.update(|m| {
                                            let list = m.entry(ch_name.clone()).or_default();
                                            for p in parts {
                                                if !list.contains(&p) {
                                                    list.push(p);
                                                }
                                            }
                                            // Also add the local user.
                                            let my_id = vc.peer_id();
                                            if !list.contains(&my_id) {
                                                list.push(my_id);
                                            }
                                        });
                                    });
                                    let on_error = wasm_bindgen::closure::Closure::once(move |_err: wasm_bindgen::JsValue| {
                                        tracing::error!("Microphone access denied");
                                        write.voice.set_voice_channel.set(None);
                                        write.voice.set_voice_channel_name.set(String::new());
                                        write.ui.set_show_call_page.set(false);
                                    });
                                    let _ = promise.then2(&on_success, &on_error);
                                    on_success.forget();
                                    on_error.forget();
                                }
                            }
                            voice_channel=app_state.voice.voice_channel
                            voice_channel_name=app_state.voice.voice_channel_name
                            voice_muted=app_state.voice.voice_muted
                            voice_deafened=app_state.voice.voice_deafened
                            on_voice_mute=Callback::new(on_mute.clone())
                            on_voice_deafen=Callback::new(on_deafen.clone())
                            on_voice_disconnect=Callback::new(on_disconnect.clone())
                            on_channel_created={
                                let ch_handle = handle_cc.clone();
                                move |_| {
                                    write.chat.set_channels.set(ch_handle.channels());
                                    write.server.set_roles.set(extract_roles(&ch_handle));
                                }
                            }
                        />
                        <div class="main-content">
                            {move || {
                                if show_add_server.get() {
                                    view! {
                                        <div class="settings-panel">
                                            <div class="server-settings-header">
                                                <button class="btn btn-sm" on:click=move |_| write.ui.set_show_add_server.set(false)>
                                                    {icons::icon_arrow_left()} " Back"
                                                </button>
                                                <h2>"Add a Server"</h2>
                                            </div>
                                            <AddServerPanel
                                                on_done=move |_| {
                                                    refresh_stored.with_value(|f| f());
                                                    write.ui.set_show_add_server.set(false);
                                                }
                                            />
                                        </div>
                                    }.into_any()
                                } else if show_settings.get() {
                                    let tab = app_state.ui.settings_tab.get_untracked();
                                    view! { <SettingsPanel
                                        peer_id=peer_id
                                        roles=Signal::from(_roles)
                                        default_tab=tab
                                        on_close=move |_| write.ui.set_show_settings.set(false)
                                    /> }.into_any()
                                } else if show_call_page.get() {
                                    let on_mute_cp = on_mute.clone();
                                    let on_deafen_cp = on_deafen.clone();
                                    let on_disconnect_cp = on_disconnect.clone();
                                    view! {
                                        <CallPage
                                            on_disconnect=Callback::new(on_disconnect_cp)
                                            on_mute=Callback::new(on_mute_cp)
                                            on_deafen=Callback::new(on_deafen_cp)
                                        />
                                    }.into_any()
                                } else {
                                    let send2 = send.clone();
                                    let edit_send2 = edit_send.clone();
                                    let del_msg2 = del_msg.clone();
                                    let react2 = react.clone();
                                    let on_typing_cb = {
                                        let h = handle_ty.clone();
                                        Callback::new(move |_: ()| {
                                            h.send_typing();
                                        })
                                    };
                                    let on_pin_cb = {
                                        let pin_handler = pin.clone();
                                        Callback::new(move |msg: DisplayMessage| {
                                            pin_handler(msg);
                                        })
                                    };
                                    view! {
                                        <div class="chat-container">
                                            <ChannelHeader
                                                channel=current_channel
                                                peer_count=peer_count
                                                on_menu_click=move |_| write.ui.set_show_sidebar.update(|v| *v = !*v)
                                                on_members_click=move |_| write.ui.set_show_members.update(|v| *v = !*v)
                                                on_pinned_click=Callback::new(move |_| write.ui.set_show_pinned.update(|v| *v = !*v))
                                                on_search_click=Callback::new(move |_| write.ui.set_show_palette.set(true))
                                            />
                                            {move || {
                                                if show_pinned.get() {
                                                    Some(view! {
                                                        <PinnedPanel
                                                            messages=pinned_messages
                                                            on_jump=move |msg_id: String| {
                                                                let _ = js_sys::eval(&format!(
                                                                    "document.getElementById('msg-{}')?.scrollIntoView({{behavior:'smooth',block:'center'}})",
                                                                    msg_id.replace('\'', "")
                                                                ));
                                                                write.ui.set_show_pinned.set(false);
                                                            }
                                                            on_close=move |_| write.ui.set_show_pinned.set(false)
                                                        />
                                                    })
                                                } else {
                                                    None
                                                }
                                            }}
                                            <MessageList
                                                messages=messages
                                                loading=Signal::from(loading)
                                                local_display_name={let s: Signal<String> = Signal::from(display_name); s}
                                                on_message_click=Callback::new(move |msg: DisplayMessage| {
                                                    write.chat.set_replying_to.set(Some(msg));
                                                    let _ = js_sys::eval("setTimeout(function(){var i=document.querySelector('.input-area input,.input-area textarea');if(i)i.focus();},50)");
                                                })
                                                on_edit=Callback::new(move |msg: DisplayMessage| {
                                                    write.chat.set_editing.set(Some(msg));
                                                    let _ = js_sys::eval("setTimeout(function(){var i=document.querySelector('.input-area input,.input-area textarea');if(i)i.focus();},50)");
                                                })
                                                on_delete=Callback::new(del_msg2)
                                                on_react=Callback::new(react2)
                                                on_pin=on_pin_cb
                                                pin_labels=Signal::from(pin_labels)
                                            />
                                            <div class="typing-indicator">
                                                {move || {
                                                    let ch = current_channel.get();
                                                    let views = channel_views.get();
                                                    let names = views
                                                        .get(&ch)
                                                        .map(|v| v.typing.clone())
                                                        .unwrap_or_default();
                                                    match names.len() {
                                                        0 => String::new(),
                                                        1 => format!("{} is typing...", names[0]),
                                                        2 => format!("{} and {} are typing...", names[0], names[1]),
                                                        3 => format!("{}, {}, and {} are typing...", names[0], names[1], names[2]),
                                                        _ => "Multiple people are typing...".to_string(),
                                                    }
                                                }}
                                            </div>
                                            <div class="input-row">
                                                <FileShareButton
                                                    channel=current_channel
                                                />
                                                <ChatInput
                                                    on_send=send2
                                                    replying_to=replying_to
                                                    on_cancel_reply=Callback::new(move |_| {
                                                        write.chat.set_replying_to.set(None);
                                                    })
                                                    editing=editing
                                                    on_edit_send=Callback::new(edit_send2)
                                                    on_cancel_edit=Callback::new(move |_| {
                                                        write.chat.set_editing.set(None);
                                                    })
                                                    on_typing=on_typing_cb
                                                />
                                            </div>
                                        </div>
                                    }.into_any()
                                }
                            }}
                        </div>
                        <div
                            class=move || if show_members.get() { "members-overlay open" } else { "members-overlay" }
                            on:click=move |_| write.ui.set_show_members.set(false)
                        />
                        <div class=move || if show_members.get() { "member-list-wrapper open" } else { "member-list-wrapper" }>
                            <MemberList
                                peers=app_state.network.peers
                                peer_id=peer_id
                            />
                        </div>
                        {move || {
                            if show_palette.get() {
                                let ch_click_palette = ch_click_for_palette.clone();
                                let srv_click_palette = srv_click_for_palette.clone();
                                Some(view! {
                                    <CommandPalette
                                        on_close=Callback::new(move |_| write.ui.set_show_palette.set(false))
                                        on_switch_channel=Callback::new(move |name: String| {
                                            ch_click_palette(name);
                                            write.ui.set_show_palette.set(false);
                                        })
                                        on_switch_server=Callback::new(move |id: String| {
                                            srv_click_palette(id);
                                            write.ui.set_show_palette.set(false);
                                        })
                                        on_open_members=Callback::new(move |_| {
                                            write.ui.set_show_members.set(true);
                                            write.ui.set_show_palette.set(false);
                                        })
                                    />
                                })
                            } else {
                                None
                            }
                        }}
                    </div>
                }.into_any()
            }
        }}
    }
}

/// Helper to create a WebRTC offer in a spawned future.
///
/// The `RefCell` borrow is held across await but this is safe on
/// single-threaded WASM where there is no preemption.
#[allow(clippy::await_holding_refcell_ref)]
pub async fn handle_voice_create_offer(vm: VoiceManagerHandle, peer_id: String) {
    let mut mgr = vm.borrow_mut();
    let _ = mgr.create_offer(&peer_id).await;
}

/// Helper to handle an incoming WebRTC offer.
///
/// The `RefCell` borrow is held across await but this is safe on
/// single-threaded WASM where there is no preemption.
#[allow(clippy::await_holding_refcell_ref)]
pub async fn handle_voice_offer(vm: VoiceManagerHandle, from: String, sdp: String) {
    let mut mgr = vm.borrow_mut();
    let _ = mgr.handle_offer(&from, &sdp).await;
}

/// Helper to handle an incoming WebRTC answer.
///
/// The `RefCell` borrow is held across await but this is safe on
/// single-threaded WASM where there is no preemption.
#[allow(clippy::await_holding_refcell_ref)]
pub async fn handle_voice_answer(vm: VoiceManagerHandle, from: String, sdp: String) {
    let mgr = vm.borrow();
    let _ = mgr.handle_answer(&from, &sdp).await;
}
