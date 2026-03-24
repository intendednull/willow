use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use willow_client::{Client, ClientConfig, ClientEvent, DisplayMessage, VoiceSignalPayload};

use crate::components::{
    AddServerPanel, ChannelHeader, ChatInput, FileShareButton, MemberList, MessageList,
    PinnedPanel, ServerList, ServerSettingsPanel, SettingsPanel, Sidebar, VoiceControls,
    WelcomeScreen,
};
use crate::voice::VoiceManager;

fn play_notification_sound() {
    let _ = js_sys::eval(
        r#"(function(){try{var c=new(window.AudioContext||window.webkitAudioContext)();var o=c.createOscillator();var g=c.createGain();o.connect(g);g.connect(c.destination);o.frequency.value=800;g.gain.value=0.1;o.start();o.stop(c.currentTime+0.15);}catch(e){}})()"#,
    );
}

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

/// Wrapper around `Rc<RefCell<Client>>` that is `Send` for single-threaded WASM.
pub type ClientHandle = SendWrapper<Rc<RefCell<Client>>>;

/// Wrapper around `Rc<RefCell<VoiceManager>>` that is `Send` for single-threaded WASM.
pub type VoiceManagerHandle = SendWrapper<Rc<RefCell<VoiceManager>>>;

/// Default relay address for the deployed Willow relay server.
pub const DEFAULT_RELAY: &str =
    "/ip4/172.234.217.219/tcp/9091/ws/p2p/12D3KooWMBmUF1rHYG5CneKi8JZfKdMAciJd4oCgknTJkbwCUurd";

fn new_client_handle() -> ClientHandle {
    let config = ClientConfig {
        relay_addr: Some(DEFAULT_RELAY.to_string()),
        ..ClientConfig::default()
    };
    SendWrapper::new(Rc::new(RefCell::new(Client::new(config))))
}

/// Root application component. Creates the `Client`, connects to the P2P
/// network, and runs a poll loop to bridge client state into reactive signals.
#[component]
pub fn App() -> impl IntoView {
    init_theme();

    // Create and connect the client.
    let client = new_client_handle();

    {
        let mut c = client.borrow_mut();
        c.connect();
    }

    // Reactive state signals.
    let (messages, set_messages) = signal(Vec::<DisplayMessage>::new());
    let (channels, set_channels) = signal(Vec::<String>::new());
    let (peers, set_peers) = signal(Vec::<(String, String, bool)>::new());
    let (current_channel, set_current_channel) = signal(String::from("general"));
    let (peer_count, set_peer_count) = signal(0usize);
    let (show_settings, set_show_settings) = signal(false);
    let (show_server_settings, set_show_server_settings) = signal(false);
    let (show_sidebar, set_show_sidebar) = signal(false);
    let (show_members, set_show_members) = signal(false);
    let (show_add_server, set_show_add_server) = signal(false);
    let (peer_id, set_peer_id) = signal(String::new());
    let (servers, set_servers) = signal(Vec::<(String, String)>::new());
    let (active_server_id, set_active_server_id) = signal(String::new());
    let (active_server_name, set_active_server_name) = signal(String::new());
    let (unread, set_unread) = signal(HashMap::<String, usize>::new());
    let (connection_status, set_connection_status) = signal("connecting".to_string());
    let (replying_to, set_replying_to) = signal(Option::<DisplayMessage>::None);
    let (editing, set_editing) = signal(Option::<DisplayMessage>::None);
    let (loading, set_loading) = signal(true);
    let (display_name, set_display_name) = signal(String::new());
    let (roles, set_roles) = signal(Vec::<(String, String, Vec<String>)>::new());
    let (typing_names, set_typing_names) = signal(Vec::<String>::new());
    let (show_pinned, set_show_pinned) = signal(false);
    let (pinned_messages, set_pinned_messages) = signal(Vec::<DisplayMessage>::new());
    let (pin_labels, set_pin_labels) = signal(HashMap::<String, String>::new());

    // Voice state signals.
    let (voice_channel, set_voice_channel) = signal(Option::<String>::None);
    let (voice_muted, set_voice_muted) = signal(false);
    let (voice_deafened, set_voice_deafened) = signal(false);
    let (_voice_participants_map, set_voice_participants_map) =
        signal(HashMap::<String, Vec<String>>::new());
    let (voice_channel_name, set_voice_channel_name) = signal(String::new());

    // Create the VoiceManager. The signal callback sends voice signaling
    // messages back through the client's network.
    let voice_signal_client = client.clone();
    let voice_channel_for_signal = voice_channel;
    let voice_manager: VoiceManagerHandle = SendWrapper::new(Rc::new(RefCell::new(
        VoiceManager::new(move |target_peer: &str, signal_type: &str, payload: &str| {
            let ch_id = voice_channel_for_signal.get_untracked().unwrap_or_default();
            let signal = match signal_type {
                "offer" => VoiceSignalPayload::Offer(payload.to_string()),
                "answer" => VoiceSignalPayload::Answer(payload.to_string()),
                "ice" => VoiceSignalPayload::IceCandidate(payload.to_string()),
                _ => return,
            };
            let c = voice_signal_client.borrow();
            c.send_voice_signal(&ch_id, target_peer, signal);
        }),
    )));

    // Auto-clear loading after LOADING_TIMEOUT_MS even if no peer connects.
    set_timeout(
        move || {
            set_loading.set(false);
        },
        std::time::Duration::from_millis(LOADING_TIMEOUT_MS as u64),
    );

    // Closure that refreshes all signals from the client state. Used after
    // server creation, joining, and on initial load.
    let refresh_client = client.clone();
    let refresh_all_signals: SendWrapper<std::rc::Rc<dyn Fn()>> =
        SendWrapper::new(std::rc::Rc::new(move || {
            let c = refresh_client.borrow();
            set_servers.set(c.server_list());
            set_channels.set(c.channels());
            set_peer_id.set(c.peer_id());
            set_display_name.set(c.display_name());
            set_roles.set(extract_roles(&c));
            if let Some(id) = c.active_server_id() {
                set_active_server_id.set(id.to_string());
            }
            set_active_server_name.set(c.active_server_name());
            let ch = c.state().chat.current_channel.clone();
            set_current_channel.set(ch.clone());
            set_messages.set(c.messages(&ch));
            set_show_settings.set(false);
            set_show_server_settings.set(false);
            set_show_add_server.set(false);
        }));

    // Populate initial state from the client.
    refresh_all_signals();

    // Poll loop -- drain network events and refresh signals.
    let client_poll = client.clone();
    let vm_poll = voice_manager.clone();
    set_interval(
        move || {
            let mut c = client_poll.borrow_mut();
            let events = c.poll();
            let mut needs_msg_refresh = false;
            let mut needs_peer_refresh = false;
            let mut needs_channel_refresh = false;

            for event in events {
                match event {
                    ClientEvent::MessageReceived { is_local, .. } => {
                        needs_msg_refresh = true;
                        if !is_local {
                            let hidden = js_sys::eval("document.hidden")
                                .ok()
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if hidden {
                                play_notification_sound();
                            }
                        }
                    }
                    ClientEvent::MessageEdited { .. }
                    | ClientEvent::MessageDeleted { .. }
                    | ClientEvent::ReactionAdded { .. }
                    | ClientEvent::SyncCompleted { .. } => {
                        needs_msg_refresh = true;
                    }
                    ClientEvent::PeerConnected(_) => {
                        needs_peer_refresh = true;
                        set_connection_status.set("connected".to_string());
                        set_loading.set(false);
                    }
                    ClientEvent::PeerDisconnected(_) => {
                        needs_peer_refresh = true;
                    }
                    ClientEvent::Listening(_) => {
                        // We are listening but may have no peers yet.
                        let status = connection_status.get_untracked();
                        if status == "connecting" {
                            // Stay "connecting" until a peer connects, but at
                            // least we know the node is up.
                            set_connection_status.set("connecting".to_string());
                        }
                    }
                    ClientEvent::ChannelCreated(_) | ClientEvent::ChannelDeleted(_) => {
                        needs_channel_refresh = true;
                    }
                    ClientEvent::ProfileUpdated { .. } => {
                        // Display names are resolved at render time now.
                        set_display_name.set(c.display_name());
                        needs_msg_refresh = true;
                        needs_peer_refresh = true;
                    }
                    ClientEvent::VoiceJoined {
                        channel_id,
                        peer_id,
                    } => {
                        set_voice_participants_map.update(|m| {
                            let participants = m.entry(channel_id.clone()).or_default();
                            if !participants.contains(&peer_id) {
                                participants.push(peer_id.clone());
                            }
                        });
                        // If we're in this channel, create offer to new peer.
                        if voice_channel.get_untracked() == Some(channel_id) {
                            let vm = vm_poll.clone();
                            let pid = peer_id;
                            wasm_bindgen_futures::spawn_local(handle_voice_create_offer(vm, pid));
                        }
                    }
                    ClientEvent::VoiceLeft {
                        channel_id,
                        peer_id,
                    } => {
                        set_voice_participants_map.update(|m| {
                            if let Some(v) = m.get_mut(&channel_id) {
                                v.retain(|p| p != &peer_id);
                            }
                        });
                        vm_poll.borrow_mut().close_connection(&peer_id);
                    }
                    ClientEvent::VoiceSignal {
                        from_peer, signal, ..
                    } => {
                        let vm = vm_poll.clone();
                        let from = from_peer;
                        match signal {
                            VoiceSignalPayload::Offer(sdp) => {
                                wasm_bindgen_futures::spawn_local(handle_voice_offer(
                                    vm, from, sdp,
                                ));
                            }
                            VoiceSignalPayload::Answer(sdp) => {
                                wasm_bindgen_futures::spawn_local(handle_voice_answer(
                                    vm, from, sdp,
                                ));
                            }
                            VoiceSignalPayload::IceCandidate(json) => {
                                let _ = vm.borrow().handle_ice_candidate(&from, &json);
                            }
                        }
                    }
                    _ => {}
                }
            }

            if needs_msg_refresh {
                let ch = current_channel.get_untracked();
                set_messages.set(c.messages(&ch));
                // Refresh pinned messages and labels.
                set_pinned_messages.set(c.pinned_messages(&ch));
                let mut labels = HashMap::new();
                for msg in c.messages(&ch) {
                    let label = if c.is_pinned(&ch, &msg.id) {
                        "Unpin"
                    } else {
                        "Pin"
                    };
                    labels.insert(msg.id.clone(), label.to_string());
                }
                set_pin_labels.set(labels);
                // Update unread counts from the active server.
                let mut unread_map = HashMap::new();
                if let Some(ctx) = c.state().active() {
                    for (topic, count) in &ctx.unread {
                        if let Some(name) = ctx.name_for_topic(topic) {
                            unread_map.insert(name.to_string(), *count);
                        }
                    }
                }
                set_unread.set(unread_map);
            }
            if needs_peer_refresh {
                let peer_list = c.server_members();
                let count = peer_list.iter().filter(|(_, _, online)| *online).count();
                set_peers.set(peer_list);
                set_peer_count.set(count);
                if count > 0 {
                    set_connection_status.set("connected".to_string());
                } else {
                    set_connection_status.set("connecting".to_string());
                }
            }
            if needs_channel_refresh {
                set_channels.set(c.channels());
                set_roles.set(extract_roles(&c));
            }
            if needs_msg_refresh || needs_peer_refresh {
                // Roles may change via sync events, so refresh on any state change.
                set_roles.set(extract_roles(&c));
            }

            // Always refresh typing state (it auto-expires).
            let ch = current_channel.get_untracked();
            let typers = c.typing_in(&ch);
            set_typing_names.set(typers);
        },
        std::time::Duration::from_millis(50),
    );

    // Channel switch handler.
    let client_switch = client.clone();
    let on_channel_click = move |name: String| {
        set_current_channel.set(name.clone());
        set_show_sidebar.set(false); // close sidebar on mobile
        set_show_pinned.set(false); // close pinned panel on channel switch
        let c = client_switch.borrow();
        // Use immutable borrow first for reads.
        set_messages.set(c.messages(&name));
        set_pinned_messages.set(c.pinned_messages(&name));
        let mut labels = HashMap::new();
        for msg in c.messages(&name) {
            let label = if c.is_pinned(&name, &msg.id) {
                "Unpin"
            } else {
                "Pin"
            };
            labels.insert(msg.id.clone(), label.to_string());
        }
        set_pin_labels.set(labels);
        drop(c);
        let mut c = client_switch.borrow_mut();
        c.switch_channel(&name);
        // Clear unread for this channel.
        set_unread.update(|m| {
            m.remove(&name);
        });
    };

    // Send message handler -- supports replies when replying_to is set.
    let client_send = client.clone();
    let on_send = move |body: String| {
        let ch = current_channel.get_untracked();
        let mut c = client_send.borrow_mut();
        if let Some(reply_msg) = replying_to.get_untracked() {
            let _ = c.send_reply(&ch, &reply_msg.id, &body);
            set_replying_to.set(None);
        } else {
            let _ = c.send_message(&ch, &body);
        }
        set_messages.set(c.messages(&ch));
    };

    // Server switch handler.
    let client_server = client.clone();
    let on_server_click = move |id: String| {
        let mut c = client_server.borrow_mut();
        c.switch_server(&id);
        set_active_server_id.set(id);
        set_servers.set(c.server_list());
        let chs = c.channels();
        set_channels.set(chs.clone());
        let first_ch = chs
            .first()
            .cloned()
            .unwrap_or_else(|| "general".to_string());
        set_current_channel.set(first_ch.clone());
        set_messages.set(c.messages(&first_ch));
        set_active_server_name.set(c.active_server_name());
        set_show_settings.set(false);
        set_show_add_server.set(false);
    };

    let settings_client = client.clone();

    // Edit message handler -- called when the user submits an edited message.
    let client_edit = client.clone();
    let on_edit_send = move |(message_id, new_body): (String, String)| {
        let ch = current_channel.get_untracked();
        let mut c = client_edit.borrow_mut();
        let _ = c.edit_message(&ch, &message_id, &new_body);
        set_editing.set(None);
        set_messages.set(c.messages(&ch));
    };

    // Delete message handler.
    let client_delete = client.clone();
    let on_delete_msg = move |msg: DisplayMessage| {
        let ch = current_channel.get_untracked();
        let mut c = client_delete.borrow_mut();
        let _ = c.delete_message(&ch, &msg.id);
        set_messages.set(c.messages(&ch));
    };

    // React handler.
    let client_react = client.clone();
    let on_react = move |(msg, emoji): (DisplayMessage, String)| {
        let ch = current_channel.get_untracked();
        let mut c = client_react.borrow_mut();
        let _ = c.react(&ch, &msg.id, &emoji);
        set_messages.set(c.messages(&ch));
    };

    let sidebar_client = client.clone();
    let file_client = client.clone();
    let member_client = client.clone();
    let typing_client = client.clone();
    let pin_client = client.clone();

    // Voice mute handler.
    let vm_mute = voice_manager.clone();
    let on_voice_mute = move |_: ()| {
        let new_muted = !voice_muted.get_untracked();
        set_voice_muted.set(new_muted);
        vm_mute.borrow().set_muted(new_muted);
    };

    // Voice deafen handler.
    let vm_deafen = voice_manager.clone();
    let on_voice_deafen = move |_: ()| {
        let new_deafened = !voice_deafened.get_untracked();
        set_voice_deafened.set(new_deafened);
        // When deafened, also mute the mic.
        if new_deafened {
            set_voice_muted.set(true);
            vm_deafen.borrow().set_muted(true);
        } else {
            set_voice_muted.set(false);
            vm_deafen.borrow().set_muted(false);
        }
    };

    // Voice disconnect handler.
    let vm_disconnect = voice_manager.clone();
    let client_voice_leave = client.clone();
    let on_voice_disconnect = move |_: ()| {
        client_voice_leave.borrow_mut().leave_voice();
        vm_disconnect.borrow_mut().close_all();
        set_voice_channel.set(None);
        set_voice_channel_name.set(String::new());
        set_voice_muted.set(false);
        set_voice_deafened.set(false);
    };

    // Welcome screen callback that refreshes all signals.
    let welcome_client = client.clone();
    let refresh_for_welcome = refresh_all_signals.clone();
    let on_welcome_done = move |_: ()| {
        refresh_for_welcome();
    };

    // Store the refresh function so reactive closures can access it without moving.
    let refresh_stored = StoredValue::new(refresh_all_signals);

    view! {
        {move || {
            let srv = servers.get();
            if srv.is_empty() {
                let wc = welcome_client.clone();
                let on_done = on_welcome_done.clone();
                view! {
                    <WelcomeScreen
                        client=wc
                        on_done=on_done
                    />
                }.into_any()
            } else {
                let sc = settings_client.clone();
                let fc = file_client.clone();
                let sbc = sidebar_client.clone();
                let mc = member_client.clone();
                let tc = typing_client.clone();
                let pc = pin_client.clone();
                let ch_click = on_channel_click.clone();
                let srv_click = on_server_click.clone();
                let send = on_send.clone();
                let edit_send = on_edit_send.clone();
                let del_msg = on_delete_msg.clone();
                let react = on_react.clone();
                let on_mute = on_voice_mute.clone();
                let on_deafen = on_voice_deafen.clone();
                let on_disconnect = on_voice_disconnect.clone();
                view! {
                    <div class="app">
                        <ServerList
                            servers=servers
                            active_server_id=active_server_id
                            on_server_click=srv_click
                            on_add_server_click=move |_| {
                                set_show_add_server.update(|v| *v = !*v);
                                set_show_settings.set(false);
                                set_show_server_settings.set(false);
                                set_show_sidebar.set(false);
                            }
                        />
                        // Overlay to close sidebar on mobile tap
                        <div
                            class=move || if show_sidebar.get() { "sidebar-overlay open" } else { "sidebar-overlay" }
                            on:click=move |_| set_show_sidebar.set(false)
                        />
                        <Sidebar
                            channels=channels
                            current_channel=current_channel
                            open=show_sidebar
                            unread=unread
                            connection_status=connection_status
                            peer_count=peer_count
                            server_name=active_server_name
                            client=sbc
                            on_channel_click=ch_click
                            on_settings_click=move |_| {
                                set_show_settings.update(|v| *v = !*v);
                                set_show_server_settings.set(false);
                                set_show_sidebar.set(false);
                            }
                            on_server_settings_click=move |_| {
                                set_show_server_settings.update(|v| *v = !*v);
                                set_show_settings.set(false);
                                set_show_sidebar.set(false);
                            }
                            on_voice_join={
                                let vc_client = client.clone();
                                let vm = voice_manager.clone();
                                move |channel_name: String| {
                                    // Join voice channel via client.
                                    let mut c = vc_client.borrow_mut();
                                    c.join_voice(&channel_name);
                                    set_voice_channel.set(Some(channel_name.clone()));
                                    set_voice_channel_name.set(channel_name);
                                    set_show_sidebar.set(false);

                                    // Acquire microphone asynchronously.
                                    let vm2 = vm.clone();
                                    wasm_bindgen_futures::spawn_local(async move {
                                        // Use js_sys to call getUserMedia directly
                                        // since VoiceManager::acquire_microphone holds RefCell across await.
                                        let stream = crate::voice::acquire_microphone_async().await;
                                        match stream {
                                            Ok(s) => vm2.borrow_mut().set_local_stream(s),
                                            Err(e) => {
                                                let _ = js_sys::eval(&format!(
                                                    "console.error('Mic error: {}')",
                                                    e
                                                ));
                                            }
                                        }
                                    });
                                }
                            }
                        />
                        {move || {
                            let on_mute = on_mute.clone();
                            let on_deafen = on_deafen.clone();
                            let on_disconnect = on_disconnect.clone();
                            if voice_channel.get().is_some() {
                                Some(view! {
                                    <VoiceControls
                                        channel_name=voice_channel_name
                                        muted=voice_muted
                                        deafened=voice_deafened
                                        on_mute=on_mute
                                        on_deafen=on_deafen
                                        on_disconnect=on_disconnect
                                    />
                                })
                            } else {
                                None
                            }
                        }}
                        <div class="main-content">
                            {move || {
                                let sc2 = sc.clone();
                                let pid = peer_id;
                                if show_add_server.get() {
                                    let add_client = sc2.clone();
                                    view! {
                                        <div class="settings-panel">
                                            <div class="server-settings-header">
                                                <button class="btn btn-sm" on:click=move |_| set_show_add_server.set(false)>
                                                    "\u{2190} Back"
                                                </button>
                                                <h2>"Add a Server"</h2>
                                            </div>
                                            <AddServerPanel
                                                client=add_client
                                                on_done=move |_| {
                                                    refresh_stored.with_value(|f| f());
                                                    set_show_add_server.set(false);
                                                }
                                            />
                                        </div>
                                    }.into_any()
                                } else if show_server_settings.get() {
                                    let sc3 = sc2.clone();
                                    view! { <ServerSettingsPanel client=sc3 peer_id=pid roles=Signal::from(roles) on_back=move |_| set_show_server_settings.set(false) /> }.into_any()
                                } else if show_settings.get() {
                                    view! { <SettingsPanel client=sc2 peer_id=pid on_server_settings=move |_| {
                                        set_show_settings.set(false);
                                        set_show_server_settings.set(true);
                                    } /> }.into_any()
                                } else {
                                    let fc2 = fc.clone();
                                    let pc2 = pc.clone();
                                    let send2 = send.clone();
                                    let edit_send2 = edit_send.clone();
                                    let del_msg2 = del_msg.clone();
                                    let react2 = react.clone();
                                    let tc2 = tc.clone();
                                    let on_typing_cb = Callback::new(move |_: ()| {
                                        tc2.borrow_mut().send_typing();
                                    });
                                    let on_pin_cb = Callback::new(move |msg: DisplayMessage| {
                                        let ch = current_channel.get_untracked();
                                        let mut c = pc2.borrow_mut();
                                        if c.is_pinned(&ch, &msg.id) {
                                            let _ = c.unpin_message(&ch, &msg.id);
                                        } else {
                                            let _ = c.pin_message(&ch, &msg.id);
                                        }
                                        set_pinned_messages.set(c.pinned_messages(&ch));
                                        let mut labels = HashMap::new();
                                        for m in c.messages(&ch) {
                                            let label = if c.is_pinned(&ch, &m.id) { "Unpin" } else { "Pin" };
                                            labels.insert(m.id.clone(), label.to_string());
                                        }
                                        set_pin_labels.set(labels);
                                    });
                                    view! {
                                        <div class="chat-container">
                                            <ChannelHeader
                                                channel=current_channel
                                                peer_count=peer_count
                                                on_menu_click=move |_| set_show_sidebar.update(|v| *v = !*v)
                                                on_members_click=move |_| set_show_members.update(|v| *v = !*v)
                                                on_pinned_click=Callback::new(move |_| set_show_pinned.update(|v| *v = !*v))
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
                                                                set_show_pinned.set(false);
                                                            }
                                                            on_close=move |_| set_show_pinned.set(false)
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
                                                    set_replying_to.set(Some(msg));
                                                    // Auto-focus the input field so keyboard opens on mobile.
                                                    let _ = js_sys::eval("setTimeout(function(){var i=document.querySelector('.input-area input,.input-area textarea');if(i)i.focus();},50)");
                                                })
                                                on_edit=Callback::new(move |msg: DisplayMessage| {
                                                    set_editing.set(Some(msg));
                                                    let _ = js_sys::eval("setTimeout(function(){var i=document.querySelector('.input-area input,.input-area textarea');if(i)i.focus();},50)");
                                                })
                                                on_delete=Callback::new(del_msg2)
                                                on_react=Callback::new(react2)
                                                on_pin=on_pin_cb
                                                pin_labels=Signal::from(pin_labels)
                                            />
                                            <div class="typing-indicator">
                                                {move || {
                                                    let names = typing_names.get();
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
                                                    client=fc2
                                                    channel=current_channel
                                                />
                                                <ChatInput
                                                    on_send=send2
                                                    replying_to=replying_to
                                                    on_cancel_reply=Callback::new(move |_| {
                                                        set_replying_to.set(None);
                                                    })
                                                    editing=editing
                                                    on_edit_send=Callback::new(edit_send2)
                                                    on_cancel_edit=Callback::new(move |_| {
                                                        set_editing.set(None);
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
                            on:click=move |_| set_show_members.set(false)
                        />
                        <div class=move || if show_members.get() { "member-list-wrapper open" } else { "member-list-wrapper" }>
                            <MemberList
                                peers=peers
                                client=mc
                                peer_id=peer_id
                            />
                        </div>
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
async fn handle_voice_create_offer(vm: VoiceManagerHandle, peer_id: String) {
    let mut mgr = vm.borrow_mut();
    let _ = mgr.create_offer(&peer_id).await;
}

/// Helper to handle an incoming WebRTC offer.
///
/// The `RefCell` borrow is held across await but this is safe on
/// single-threaded WASM where there is no preemption.
#[allow(clippy::await_holding_refcell_ref)]
async fn handle_voice_offer(vm: VoiceManagerHandle, from: String, sdp: String) {
    let mut mgr = vm.borrow_mut();
    let _ = mgr.handle_offer(&from, &sdp).await;
}

/// Helper to handle an incoming WebRTC answer.
///
/// The `RefCell` borrow is held across await but this is safe on
/// single-threaded WASM where there is no preemption.
#[allow(clippy::await_holding_refcell_ref)]
async fn handle_voice_answer(vm: VoiceManagerHandle, from: String, sdp: String) {
    let mgr = vm.borrow();
    let _ = mgr.handle_answer(&from, &sdp).await;
}

/// Extract roles from the client's event-sourced state as a list of
/// `(role_id, role_name, permission_strings)` tuples for reactive signals.
fn extract_roles(client: &willow_client::Client) -> Vec<(String, String, Vec<String>)> {
    let es = &client.state().event_state;
    let mut entries: Vec<(String, String, Vec<String>)> = es
        .roles
        .values()
        .map(|role| {
            let perms: Vec<String> = role.permissions.iter().cloned().collect();
            (role.id.clone(), role.name.clone(), perms)
        })
        .collect();
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    entries
}
