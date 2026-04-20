use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use willow_client::{ClientConfig, ClientEvent, ClientHandle, DisplayMessage, VoiceSignalPayload};

use crate::components::{
    AddServerPanel, CallPage, ChannelHeader, ChatInput, CommandPalette, FileShareButton, JoinPage,
    MemberList, MessageList, PinnedPanel, ServerList, SettingsPanel, Sidebar, WelcomeScreen,
};
use crate::event_processing::process_event_batch;
use crate::handlers;
use crate::icons;
use crate::state::{self, ChannelViewState, SettingsTab};
use crate::voice::VoiceManager;

// Notification sounds disabled for now.
// fn play_notification_sound() { ... }

fn init_theme() {
    js_sys::eval(
        r#"var t=localStorage.getItem('willow-theme')||'dark';document.documentElement.setAttribute('data-theme',t);"#,
    )
    .ok();
}

pub fn toggle_theme() {
    js_sys::eval(
        r#"var h=document.documentElement;var c=h.getAttribute('data-theme')||'dark';var n=c==='dark'?'light':'dark';h.setAttribute('data-theme',n);localStorage.setItem('willow-theme',n);"#,
    )
    .ok();
}

/// How many milliseconds to wait before clearing the loading state automatically.
const LOADING_TIMEOUT_MS: u32 = 5_000;

/// Wrapper around `willow_client::ClientHandle` that is `Send` for single-threaded WASM.
pub type WebClientHandle = SendWrapper<ClientHandle<willow_network::iroh::IrohNetwork>>;

/// Wrapper around `Rc<RefCell<VoiceManager>>` that is `Send` for single-threaded WASM.
pub type VoiceManagerHandle = SendWrapper<Rc<RefCell<VoiceManager>>>;

/// Default relay URL for the deployed Willow relay server.
pub const DEFAULT_RELAY_URL: &str = "https://willow.intendednull.com:9443";

/// Resolve the relay URL at runtime: checks `window.__WILLOW_RELAY_URL`,
/// then falls back to the compiled-in default.
fn resolve_relay_url() -> String {
    if let Ok(val) = js_sys::eval("window.__WILLOW_RELAY_URL") {
        if let Some(s) = val.as_string() {
            if !s.is_empty() {
                return s;
            }
        }
    }
    DEFAULT_RELAY_URL.to_string()
}

/// Derive the HTTP URL for the relay's `/bootstrap-id` endpoint from a
/// relay URL. Handles `ws://` → `http://` and `wss://` → `https://`
/// scheme rewrites so clients can fetch the endpoint ID from the same
/// port as the relay's main HTTP/WebSocket listener.
///
/// This function is pure so it can be unit-tested without a browser.
pub(crate) fn bootstrap_id_url(relay_url: &str) -> String {
    let trimmed = relay_url.trim_end_matches('/');
    // Swap the scheme to the HTTP equivalent so `fetch` works even
    // when the caller configured a ws(s) URL (the iroh-relay serves
    // HTTP and WebSocket on the same port).
    let base = if let Some(rest) = trimmed.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        trimmed.to_string()
    };
    format!("{base}/bootstrap-id")
}

/// Fetch the bootstrap node's endpoint ID from the relay. The relay
/// multiplexes `/bootstrap-id` onto its main HTTP/WebSocket port — no
/// companion port required — see issue #101. Returns `None` on any
/// failure (network error, parse error, etc.).
async fn fetch_bootstrap_id(relay_url: &str) -> Option<willow_identity::EndpointId> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let bootstrap_url = bootstrap_id_url(relay_url);

    let window = web_sys::window()?;
    let resp_value = JsFuture::from(window.fetch_with_str(&bootstrap_url))
        .await
        .ok()?;
    let resp: web_sys::Response = resp_value.dyn_into().ok()?;
    let text_promise = resp.text().ok()?;
    let text_value = JsFuture::from(text_promise).await.ok()?;
    let text = text_value.as_string()?;
    text.trim().parse::<willow_identity::EndpointId>().ok()
}

fn new_client() -> WebClientHandle {
    let relay_url = resolve_relay_url();
    let config = ClientConfig {
        relay_addr: Some(relay_url),
        ..ClientConfig::default()
    };
    let (handle, _event_loop) = ClientHandle::<willow_network::iroh::IrohNetwork>::new(config);
    SendWrapper::new(handle)
}

/// Root application component. Creates the `ClientHandle`, connects to the P2P
/// network, and spawns async event processing to bridge client state into
/// reactive signals.
#[component]
pub fn App() -> impl IntoView {
    init_theme();

    // Create the client (connection happens async below).
    let handle = new_client();

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
                if let Ok(target) = target_peer.parse::<willow_identity::EndpointId>() {
                    voice_signal_handle.send_voice_signal(&ch_id, target, signal);
                }
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
            window
                .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref())
                .ok();
        }
        closure.forget();
    }

    // Wire derived signals that auto-update from state actor changes.
    crate::state::wire_derived_signals(&handle, handle.actor_system(), &write);

    // Detect join link from URL fragment.
    {
        let join_token_value = web_sys::window()
            .and_then(|w| w.location().hash().ok())
            .and_then(|hash| hash.strip_prefix("#join=").map(|s| s.to_string()));

        if let Some(ref token_str) = join_token_value {
            if let Some(token) = willow_client::ops::JoinToken::decode(token_str) {
                write.ui.set_join_token.set(Some(state::ParsedJoinToken {
                    raw: token_str.clone(),
                    link_id: token.link_id,
                    server_name: token.server_name,
                    inviter_name: token.inviter_name,
                }));
                write.ui.set_join_status.set(String::new());
            }
        }
    }

    // Listen for hash changes so navigation to #join=... works after initial load.
    {
        use wasm_bindgen::JsCast;
        let write_for_hash = write;
        let closure = wasm_bindgen::closure::Closure::<dyn Fn(web_sys::Event)>::new(
            move |_ev: web_sys::Event| {
                let join_token_value = web_sys::window()
                    .and_then(|w| w.location().hash().ok())
                    .and_then(|hash| hash.strip_prefix("#join=").map(|s| s.to_string()));

                if let Some(ref token_str) = join_token_value {
                    if let Some(token) = willow_client::ops::JoinToken::decode(token_str) {
                        write_for_hash
                            .ui
                            .set_join_token
                            .set(Some(state::ParsedJoinToken {
                                raw: token_str.clone(),
                                link_id: token.link_id,
                                server_name: token.server_name,
                                inviter_name: token.inviter_name,
                            }));
                        write_for_hash.ui.set_join_status.set(String::new());
                    }
                }
            },
        );
        if let Some(window) = web_sys::window() {
            window
                .add_event_listener_with_callback("hashchange", closure.as_ref().unchecked_ref())
                .ok();
        }
        closure.forget();
    }

    // Spawn the event loop and signal updater.
    {
        let mut handle_for_connect = (*handle).clone();
        let handle_for_events = handle.clone();
        let write_for_events = write;
        let state_for_events = app_state;
        let vm_for_events = voice_manager.clone();

        // Spawn a single async task that creates the network, connects,
        // and then processes the resulting ClientEvent stream.
        wasm_bindgen_futures::spawn_local(async move {
            // Build the iroh network configuration from our identity.
            let relay_url = resolve_relay_url();
            let parsed_relay = relay_url.parse::<willow_network::iroh::RelayUrl>().ok();
            if parsed_relay.is_none() {
                tracing::warn!(url = %relay_url, "failed to parse relay URL");
            }

            // Fetch the bootstrap node's endpoint ID from the relay.
            let bootstrap_peers = match fetch_bootstrap_id(&relay_url).await {
                Some(id) => {
                    tracing::info!(%id, "fetched bootstrap peer from relay");
                    vec![id]
                }
                None => {
                    tracing::warn!("could not fetch bootstrap peer ID from relay");
                    vec![]
                }
            };

            // Set bootstrap peers on the client handle so topic subscriptions use them.
            handle_for_connect.bootstrap_peers = bootstrap_peers.clone();

            let iroh_config = willow_network::iroh::Config {
                secret_key: handle_for_connect.identity().secret_key().clone(),
                relay_url: parsed_relay,
                bootstrap_peers,
                mdns: false,
            };

            // Create the iroh network node.
            let network = match willow_network::iroh::IrohNetwork::new(iroh_config).await {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!(%e, "failed to create IrohNetwork");
                    return;
                }
            };

            // Connect to the P2P network. This subscribes to topics, spawns
            // listeners, and returns the event broker.
            let _broker = handle_for_connect.connect(network).await;

            // Subscribe to client events via the broker.
            let mut event_rx = handle_for_connect.subscribe_events().await;

            while let Some(event) = event_rx.recv().await {
                // Collect all pending events into a batch.
                let mut batch = vec![event];
                while let Some(ev) = event_rx.try_recv() {
                    batch.push(ev);
                }
                process_event_batch(
                    &batch,
                    &handle_for_events,
                    &state_for_events,
                    &write_for_events,
                    &vm_for_events,
                );

                // If we just connected and a join is in progress, send the request.
                let has_connect = batch.iter().any(|e| {
                    matches!(e, ClientEvent::PeerConnected(_) | ClientEvent::Listening(_))
                });
                if has_connect {
                    let status = state_for_events.ui.join_status.get_untracked();
                    if status == "connecting" {
                        if let Some(token) = state_for_events.ui.join_token.get_untracked() {
                            handle_for_events.send_join_request(&token.link_id);
                        }
                    }
                }
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
                let typers = handle_typing.typing_in(&ch).await;
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
        let handle_voice_leave = handle_voice_leave.clone();
        wasm_bindgen_futures::spawn_local(async move {
            handle_voice_leave.leave_voice().await;
        });
        vm_disconnect.borrow_mut().close_all();
        write.voice.reset();
        write.ui.set_show_call_page.set(false);
        write
            .ui
            .set_call_layout
            .set(crate::state::CallLayout::default());
    };

    // Welcome screen callback (no-op — domain actors auto-notify).
    let on_welcome_done = move |_: ()| {};

    // Refresh is now automatic via domain actor Notify subscriptions.
    let refresh_stored = StoredValue::new(SendWrapper::new(Rc::new(move || {}) as Rc<dyn Fn()>));

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

    let vm_for_view = voice_manager.clone();

    let join_token_signal = app_state.ui.join_token;

    view! {
        <div id="app-root" class="density-balanced" data-accent="moss">
            {move || {
                // Join link takes priority over everything.
                if join_token_signal.get().is_some() {
                    return view! { <JoinPage /> }.into_any();
                }

                let srv = servers.get();
                if srv.is_empty() {
                    let on_done = on_welcome_done;
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
                                        let vc_leave = vc_handle.clone();
                                        wasm_bindgen_futures::spawn_local(async move {
                                            vc_leave.leave_voice().await;
                                        });
                                        vm.borrow_mut().close_all();
                                        write.voice.reset();
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
                                        wasm_bindgen_futures::spawn_local(async move {
                                            vc.join_voice(&ch_name).await;

                                            // Seed participants from client state after joining.
                                            // This ensures that on reconnect we pick up peers
                                            // who are already in the channel (their VoiceJoined
                                            // event was received before we joined).
                                            let parts: Vec<String> = vc.voice_participants(&ch_name).await.iter().map(|p| p.to_string()).collect();
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
                                    });
                                    let on_error = wasm_bindgen::closure::Closure::once(move |_err: wasm_bindgen::JsValue| {
                                        tracing::error!("Microphone access denied");
                                        write.voice.reset();
                                        write.ui.set_show_call_page.set(false);
                                    });
                                    drop(promise.then2(&on_success, &on_error));
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
                            on_channel_created=move |_| {}
                        />
                        <div class="main-content">
                            {move || {
                                if show_add_server.get() {
                                    let (add_name, set_add_name) = signal(String::new());
                                    view! {
                                        <div class="settings-panel">
                                            <div class="server-settings-header">
                                                <button class="btn btn-sm" on:click=move |_| write.ui.set_show_add_server.set(false)>
                                                    {icons::icon_arrow_left()} " Back"
                                                </button>
                                                <h2>"Add a Server"</h2>
                                            </div>
                                            <div class="welcome-name-row">
                                                <label for="add-server-display-name">"Display name · optional"</label>
                                                <input
                                                    id="add-server-display-name"
                                                    type="text"
                                                    placeholder="what peers should call you"
                                                    prop:value=move || add_name.get()
                                                    on:input=move |ev| set_add_name.set(event_target_value(&ev))
                                                />
                                            </div>
                                            <AddServerPanel
                                                on_done=move |_| {
                                                    refresh_stored.with_value(|f| f());
                                                    write.ui.set_show_add_server.set(false);
                                                }
                                                display_name=add_name
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
                                            let h = h.clone();
                                            wasm_bindgen_futures::spawn_local(async move {
                                                h.send_typing().await;
                                            });
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
                                                                js_sys::eval(&format!(
                                                                    "document.getElementById('msg-{}')?.scrollIntoView({{behavior:'smooth',block:'center'}})",
                                                                    msg_id.replace('\'', "")
                                                                )).ok();
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
                                                    js_sys::eval("setTimeout(function(){var i=document.querySelector('.input-area input,.input-area textarea');if(i)i.focus();},50)").ok();
                                                })
                                                on_edit=Callback::new(move |msg: DisplayMessage| {
                                                    write.chat.set_editing.set(Some(msg));
                                                    js_sys::eval("setTimeout(function(){var i=document.querySelector('.input-area input,.input-area textarea');if(i)i.focus();},50)").ok();
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
        </div>
    }
}

/// Helper to create a WebRTC offer in a spawned future.
///
/// The `RefCell` borrow is held across await but this is safe on
/// single-threaded WASM where there is no preemption.
#[allow(clippy::await_holding_refcell_ref)]
pub async fn handle_voice_create_offer(vm: VoiceManagerHandle, peer_id: String) {
    let mut mgr = vm.borrow_mut();
    mgr.create_offer(&peer_id).await.ok();
}

/// Helper to handle an incoming WebRTC offer.
///
/// The `RefCell` borrow is held across await but this is safe on
/// single-threaded WASM where there is no preemption.
#[allow(clippy::await_holding_refcell_ref)]
pub async fn handle_voice_offer(vm: VoiceManagerHandle, from: String, sdp: String) {
    let mut mgr = vm.borrow_mut();
    mgr.handle_offer(&from, &sdp).await.ok();
}

/// Helper to handle an incoming WebRTC answer.
///
/// The `RefCell` borrow is held across await but this is safe on
/// single-threaded WASM where there is no preemption.
#[allow(clippy::await_holding_refcell_ref)]
pub async fn handle_voice_answer(vm: VoiceManagerHandle, from: String, sdp: String) {
    let mgr = vm.borrow();
    mgr.handle_answer(&from, &sdp).await.ok();
}

#[cfg(test)]
mod tests {
    use super::bootstrap_id_url;

    #[test]
    fn bootstrap_id_url_http_is_passthrough() {
        assert_eq!(
            bootstrap_id_url("http://localhost:3340"),
            "http://localhost:3340/bootstrap-id"
        );
    }

    #[test]
    fn bootstrap_id_url_https_is_passthrough() {
        assert_eq!(
            bootstrap_id_url("https://willow.example.com:9443"),
            "https://willow.example.com:9443/bootstrap-id"
        );
    }

    #[test]
    fn bootstrap_id_url_ws_becomes_http() {
        assert_eq!(
            bootstrap_id_url("ws://localhost:3340"),
            "http://localhost:3340/bootstrap-id"
        );
    }

    #[test]
    fn bootstrap_id_url_wss_becomes_https() {
        assert_eq!(
            bootstrap_id_url("wss://willow.example.com:9443"),
            "https://willow.example.com:9443/bootstrap-id"
        );
    }

    #[test]
    fn bootstrap_id_url_strips_trailing_slash() {
        assert_eq!(
            bootstrap_id_url("https://willow.example.com:9443/"),
            "https://willow.example.com:9443/bootstrap-id"
        );
    }

    #[test]
    fn bootstrap_id_url_uses_same_port_as_relay() {
        // Regression test for issue #101: the old behaviour rewrote
        // ":9443" to ":9444" / ":3340" to ":3341". The derived URL must
        // use the SAME port as the relay URL.
        let derived = bootstrap_id_url("https://relay.example.com:9443");
        assert!(
            derived.contains(":9443/"),
            "derived URL should use the same port: {derived}"
        );
        assert!(
            !derived.contains(":9444"),
            "derived URL must not use the deprecated +1 port: {derived}"
        );
    }
}
