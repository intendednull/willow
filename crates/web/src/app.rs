use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use willow_client::{ChatMessage, Client, ClientConfig, ClientEvent};

use crate::components::{
    ChannelHeader, ChatInput, FileShareButton, MemberList, MessageList, ServerList,
    ServerSettingsPanel, SettingsPanel, Sidebar, WelcomeScreen,
};

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
    let (messages, set_messages) = signal(Vec::<ChatMessage>::new());
    let (channels, set_channels) = signal(Vec::<String>::new());
    let (peers, set_peers) = signal(Vec::<(String, String, bool)>::new());
    let (current_channel, set_current_channel) = signal(String::from("general"));
    let (peer_count, set_peer_count) = signal(0usize);
    let (show_settings, set_show_settings) = signal(false);
    let (show_server_settings, set_show_server_settings) = signal(false);
    let (show_sidebar, set_show_sidebar) = signal(false);
    let (show_members, set_show_members) = signal(false);
    let (peer_id, set_peer_id) = signal(String::new());
    let (servers, set_servers) = signal(Vec::<(String, String)>::new());
    let (active_server_id, set_active_server_id) = signal(String::new());
    let (unread, set_unread) = signal(HashMap::<String, usize>::new());
    let (connection_status, set_connection_status) = signal("connecting".to_string());
    let (replying_to, set_replying_to) = signal(Option::<ChatMessage>::None);
    let (editing, set_editing) = signal(Option::<ChatMessage>::None);
    let (loading, set_loading) = signal(true);
    let (display_name, set_display_name) = signal(String::new());
    let (roles, set_roles) = signal(Vec::<(String, String, Vec<String>)>::new());
    let (typing_names, set_typing_names) = signal(Vec::<String>::new());

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
    let refresh_all_signals = move || {
        let c = refresh_client.borrow();
        set_servers.set(c.server_list());
        set_channels.set(c.channels());
        set_peer_id.set(c.peer_id());
        set_display_name.set(c.display_name());
        set_roles.set(extract_roles(&c));
        if let Some(id) = c.active_server_id() {
            set_active_server_id.set(id.to_string());
        }
        let ch = c.state().chat.current_channel.clone();
        set_current_channel.set(ch.clone());
        set_messages.set(c.messages(&ch).into_iter().cloned().collect());
        set_show_settings.set(false);
        set_show_server_settings.set(false);
    };

    // Populate initial state from the client.
    refresh_all_signals();

    // Poll loop -- drain network events and refresh signals.
    let client_poll = client.clone();
    set_interval(
        move || {
            let mut c = client_poll.borrow_mut();
            let events = c.poll();
            let mut needs_msg_refresh = false;
            let mut needs_peer_refresh = false;
            let mut needs_channel_refresh = false;

            for event in events {
                match event {
                    ClientEvent::MessageReceived { ref message, .. } => {
                        needs_msg_refresh = true;
                        if !message.is_local {
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
                    ClientEvent::ProfileUpdated {
                        ref peer_id,
                        ref display_name,
                    } => {
                        // Update author names on all existing messages from this peer.
                        for msg in &mut c.state_mut().chat.messages {
                            if msg.author == *peer_id
                                || msg.author == willow_client::util::truncate_peer_id(peer_id)
                            {
                                msg.author = display_name.clone();
                            }
                        }
                        // Refresh local display name in case it was us.
                        set_display_name.set(c.display_name());
                        needs_msg_refresh = true;
                        needs_peer_refresh = true;
                    }
                    _ => {}
                }
            }

            if needs_msg_refresh {
                let ch = current_channel.get_untracked();
                set_messages.set(c.messages(&ch).into_iter().cloned().collect());
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
        let mut c = client_switch.borrow_mut();
        c.switch_channel(&name);
        set_messages.set(c.messages(&name).into_iter().cloned().collect());
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
        set_messages.set(c.messages(&ch).into_iter().cloned().collect());
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
        set_messages.set(c.messages(&first_ch).into_iter().cloned().collect());
        set_show_settings.set(false);
    };

    let settings_client = client.clone();

    // Edit message handler -- called when the user submits an edited message.
    let client_edit = client.clone();
    let on_edit_send = move |(message_id, new_body): (String, String)| {
        let ch = current_channel.get_untracked();
        let mut c = client_edit.borrow_mut();
        let _ = c.edit_message(&ch, &message_id, &new_body);
        set_editing.set(None);
        set_messages.set(c.messages(&ch).into_iter().cloned().collect());
    };

    // Delete message handler.
    let client_delete = client.clone();
    let on_delete_msg = move |msg: ChatMessage| {
        let ch = current_channel.get_untracked();
        let mut c = client_delete.borrow_mut();
        let _ = c.delete_message(&ch, &msg.id);
        set_messages.set(c.messages(&ch).into_iter().cloned().collect());
    };

    // React handler.
    let client_react = client.clone();
    let on_react = move |(msg, emoji): (ChatMessage, String)| {
        let ch = current_channel.get_untracked();
        let mut c = client_react.borrow_mut();
        let _ = c.react(&ch, &msg.id, &emoji);
        set_messages.set(c.messages(&ch).into_iter().cloned().collect());
    };

    let sidebar_client = client.clone();
    let file_client = client.clone();
    let member_client = client.clone();
    let typing_client = client.clone();

    // Welcome screen callbacks that refresh all signals.
    let welcome_client = client.clone();
    let refresh_for_created = refresh_all_signals.clone();
    let on_welcome_created = move |_: ()| {
        refresh_for_created();
    };
    let refresh_for_joined = refresh_all_signals.clone();
    let on_welcome_joined = move |_: ()| {
        refresh_for_joined();
    };

    view! {
        {move || {
            let srv = servers.get();
            if srv.is_empty() {
                let wc = welcome_client.clone();
                let on_created = on_welcome_created.clone();
                let on_join = on_welcome_joined.clone();
                view! {
                    <WelcomeScreen
                        client=wc
                        on_server_created=on_created
                        on_joined=on_join
                    />
                }.into_any()
            } else {
                let sc = settings_client.clone();
                let fc = file_client.clone();
                let sbc = sidebar_client.clone();
                let mc = member_client.clone();
                let tc = typing_client.clone();
                let ch_click = on_channel_click.clone();
                let srv_click = on_server_click.clone();
                let send = on_send.clone();
                let edit_send = on_edit_send.clone();
                let del_msg = on_delete_msg.clone();
                let react = on_react.clone();
                view! {
                    <div class="app">
                        <ServerList
                            servers=servers
                            active_server_id=active_server_id
                            on_server_click=srv_click
                            on_settings_click=move |_| {
                                set_show_settings.update(|v| *v = !*v);
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
                        />
                        <div class="main-content">
                            {move || {
                                let sc2 = sc.clone();
                                let pid = peer_id;
                                if show_server_settings.get() {
                                    let sc3 = sc2.clone();
                                    view! { <ServerSettingsPanel client=sc3 peer_id=pid roles=Signal::from(roles) on_back=move |_| set_show_server_settings.set(false) /> }.into_any()
                                } else if show_settings.get() {
                                    view! { <SettingsPanel client=sc2 peer_id=pid on_server_settings=move |_| {
                                        set_show_settings.set(false);
                                        set_show_server_settings.set(true);
                                    } /> }.into_any()
                                } else {
                                    let fc2 = fc.clone();
                                    let send2 = send.clone();
                                    let edit_send2 = edit_send.clone();
                                    let del_msg2 = del_msg.clone();
                                    let react2 = react.clone();
                                    let tc2 = tc.clone();
                                    let on_typing_cb = Callback::new(move |_: ()| {
                                        tc2.borrow_mut().send_typing();
                                    });
                                    view! {
                                        <div class="chat-container">
                                            <ChannelHeader
                                                channel=current_channel
                                                peer_count=peer_count
                                                on_menu_click=move |_| set_show_sidebar.update(|v| *v = !*v)
                                                on_members_click=move |_| set_show_members.update(|v| *v = !*v)
                                            />
                                            <MessageList
                                                messages=messages
                                                loading=Signal::from(loading)
                                                local_display_name={let s: Signal<String> = Signal::from(display_name); s}
                                                on_message_click=Callback::new(move |msg: ChatMessage| {
                                                    set_replying_to.set(Some(msg));
                                                })
                                                on_edit=Callback::new(move |msg: ChatMessage| {
                                                    set_editing.set(Some(msg));
                                                })
                                                on_delete=Callback::new(del_msg2)
                                                on_react=Callback::new(react2)
                                            />
                                            <div class="typing-indicator">
                                                {move || {
                                                    let names = typing_names.get();
                                                    if names.is_empty() {
                                                        String::new()
                                                    } else if names.len() == 1 {
                                                        format!("{} is typing...", names[0])
                                                    } else if names.len() == 2 {
                                                        format!("{} and {} are typing...", names[0], names[1])
                                                    } else {
                                                        format!("{} and {} others are typing...", names[0], names.len() - 1)
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
