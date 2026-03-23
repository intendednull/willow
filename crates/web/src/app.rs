use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use willow_client::{ChatMessage, Client, ClientConfig, ClientEvent};

use crate::components::{
    ChannelHeader, ChatInput, MemberList, MessageList, ServerList, SettingsPanel, Sidebar,
};

/// Wrapper around `Rc<RefCell<Client>>` that is `Send` for single-threaded WASM.
pub type ClientHandle = SendWrapper<Rc<RefCell<Client>>>;

/// Default relay address for the deployed Willow relay server.
pub const DEFAULT_RELAY: &str = "/ip4/172.234.217.219/tcp/9091/ws/p2p/12D3KooWMBmUF1rHYG5CneKi8JZfKdMAciJd4oCgknTJkbwCUurd";

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
    // Create and connect the client.
    let client = new_client_handle();

    {
        let mut c = client.borrow_mut();
        c.connect();
    }

    // Reactive state signals.
    let (messages, set_messages) = signal(Vec::<ChatMessage>::new());
    let (channels, set_channels) = signal(Vec::<String>::new());
    let (peers, set_peers) = signal(Vec::<String>::new());
    let (current_channel, set_current_channel) = signal(String::from("general"));
    let (peer_count, set_peer_count) = signal(0usize);
    let (show_settings, set_show_settings) = signal(false);
    let (show_sidebar, set_show_sidebar) = signal(false);
    let (peer_id, set_peer_id) = signal(String::new());
    let (servers, set_servers) = signal(Vec::<(String, String)>::new());
    let (active_server_id, set_active_server_id) = signal(String::new());
    let (unread, set_unread) = signal(HashMap::<String, usize>::new());

    // Populate initial state from the client.
    {
        let c = client.borrow();
        set_channels.set(c.channels());
        set_peer_id.set(c.peer_id());
        set_servers.set(c.server_list());
        if let Some(id) = c.active_server_id() {
            set_active_server_id.set(id.to_string());
        }
    }

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
                    ClientEvent::MessageReceived { .. }
                    | ClientEvent::MessageEdited { .. }
                    | ClientEvent::MessageDeleted { .. }
                    | ClientEvent::ReactionAdded { .. }
                    | ClientEvent::SyncCompleted { .. } => {
                        needs_msg_refresh = true;
                    }
                    ClientEvent::PeerConnected(_) | ClientEvent::PeerDisconnected(_) => {
                        needs_peer_refresh = true;
                    }
                    ClientEvent::ChannelCreated(_) | ClientEvent::ChannelDeleted(_) => {
                        needs_channel_refresh = true;
                    }
                    ClientEvent::ProfileUpdated { .. } => {
                        needs_msg_refresh = true;
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
                set_peers.set(c.peers().to_vec());
                set_peer_count.set(c.peers().len());
            }
            if needs_channel_refresh {
                set_channels.set(c.channels());
            }
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

    // Send message handler.
    let client_send = client.clone();
    let on_send = move |body: String| {
        let ch = current_channel.get_untracked();
        let mut c = client_send.borrow_mut();
        let _ = c.send_message(&ch, &body);
        set_messages.set(c.messages(&ch).into_iter().cloned().collect());
    };

    // Server switch handler.
    let client_server = client.clone();
    let on_server_click = move |id: String| {
        let mut c = client_server.borrow_mut();
        c.switch_server(&id);
        set_active_server_id.set(id);
        set_servers.set(c.server_list());
        set_channels.set(c.channels());
        set_current_channel.set(String::from("general"));
        let ch = "general";
        set_messages.set(c.messages(ch).into_iter().cloned().collect());
        set_show_settings.set(false);
    };

    let settings_client = client.clone();
    let joined_client = client.clone();
    let on_joined = move |_: ()| {
        let c = joined_client.borrow();
        set_servers.set(c.server_list());
        if let Some(id) = c.active_server_id() {
            set_active_server_id.set(id.to_string());
        }
        set_channels.set(c.channels());
        let ch = c.state().chat.current_channel.clone();
        set_current_channel.set(ch.clone());
        set_messages.set(c.messages(&ch).into_iter().cloned().collect());
        set_show_settings.set(false);
    };

    view! {
        <div class="app">
            <ServerList
                servers=servers
                active_server_id=active_server_id
                on_server_click=on_server_click
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
                client=client.clone()
                on_channel_click=on_channel_click
                on_settings_click=move |_| {
                    set_show_settings.update(|v| *v = !*v);
                    set_show_sidebar.set(false);
                }
            />
            <div class="main-content">
                {move || {
                    let sc = settings_client.clone();
                    let pid = peer_id;
                    if show_settings.get() {
                        view! { <SettingsPanel client=sc peer_id=pid on_joined=on_joined.clone() /> }.into_any()
                    } else {
                        view! {
                            <div class="chat-container">
                                <ChannelHeader
                                    channel=current_channel
                                    peer_count=peer_count
                                    on_menu_click=move |_| set_show_sidebar.update(|v| *v = !*v)
                                />
                                <MessageList messages=messages />
                                <ChatInput on_send=on_send.clone() />
                            </div>
                        }.into_any()
                    }
                }}
            </div>
            <MemberList
                peers=peers
                client=client.clone()
                peer_id=peer_id
            />
        </div>
    }
}
