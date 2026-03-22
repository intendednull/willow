//! # UI Module
//!
//! Top-level Bevy UI layout for the Willow chat client.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use std::collections::HashMap;

use crate::network_bridge::{
    ConnectCommand, LocalIdentity, NetworkBridgeEvent, NetworkCommandSender,
};
use willow_channel::{ChannelKind, Server};
use willow_crypto::ChannelKey;
use willow_messaging::hlc::HLC;
use willow_messaging::{Content, Message};
use willow_transport::{pack_envelope, unpack_envelope, MessageType};

/// Plugin for all UI systems and resources.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChatState::default())
            .insert_resource(InputState::default())
            .insert_resource(ChannelKeyStore::default())
            .insert_resource(ServerState::default())
            .insert_resource(AppView::default())
            .insert_resource(SettingsInput::default())
            .add_systems(Startup, (init_server, setup_ui).chain())
            .add_systems(
                Update,
                (
                    handle_keyboard_input,
                    send_message,
                    handle_network_events,
                    handle_channel_click,
                    sync_message_list,
                ),
            )
            .add_systems(
                Update,
                (
                    subscribe_channels,
                    sync_input_text,
                    update_peer_count,
                    update_channel_header,
                    update_channel_highlights,
                ),
            )
            .add_systems(
                Update,
                (handle_settings_button, handle_save_settings, toggle_view),
            );
    }
}

// ───── Resources ─────────────────────────────────────────────────────────────

/// The local server instance. Each peer auto-creates a server on first launch.
#[derive(Resource, Default)]
pub(crate) struct ServerState {
    pub(crate) server: Option<Server>,
    /// Maps gossipsub topic → (channel_name, channel_id) for display + key lookup.
    pub(crate) topic_map: HashMap<String, (String, willow_channel::ChannelId)>,
}

impl ServerState {
    /// Get the gossipsub topic for a channel by name.
    pub(crate) fn topic_for_name(&self, name: &str) -> Option<String> {
        self.topic_map
            .iter()
            .find(|(_, (n, _))| n == name)
            .map(|(topic, _)| topic.clone())
    }

    /// Get the channel name for a gossipsub topic.
    pub(crate) fn name_for_topic(&self, topic: &str) -> Option<&str> {
        self.topic_map.get(topic).map(|(name, _)| name.as_str())
    }

    /// List all channel names in sidebar order.
    pub(crate) fn channel_names(&self) -> Vec<String> {
        let Some(server) = &self.server else {
            return Vec::new();
        };
        let mut names: Vec<_> = server.channels().iter().map(|ch| ch.name.clone()).collect();
        names.sort();
        names
    }
}

#[derive(Resource)]
pub struct ChatState {
    pub messages: Vec<ChatMessage>,
    /// The current channel *name* (human-readable, e.g. "general").
    pub current_channel: String,
    pub peers: Vec<String>,
    pub hlc: HLC,
    pub(crate) messages_dirty: bool,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            current_channel: "general".to_string(),
            peers: Vec::new(),
            hlc: HLC::new(),
            messages_dirty: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// The gossipsub topic this message belongs to.
    pub topic: String,
    pub author: String,
    pub body: String,
    pub is_local: bool,
}

#[derive(Resource, Default)]
pub(crate) struct InputState {
    pub(crate) text: String,
    pub(crate) send_requested: bool,
}

/// Per-channel symmetric encryption keys, keyed by gossipsub topic.
#[derive(Resource, Default)]
pub(crate) struct ChannelKeyStore {
    pub(crate) keys: HashMap<String, ChannelKey>,
}

/// Which view is currently active.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub(crate) enum AppView {
    #[default]
    Chat,
    Settings,
}

/// Editable settings state.
#[derive(Resource)]
pub(crate) struct SettingsInput {
    pub(crate) relay_addr: String,
}

impl Default for SettingsInput {
    fn default() -> Self {
        let saved = crate::storage::load_settings().unwrap_or_default();
        Self {
            relay_addr: saved.relay_addr.unwrap_or_default(),
        }
    }
}

// ───── Components ────────────────────────────────────────────────────────────

#[derive(Component)]
struct MessageList;

#[derive(Component)]
struct ChannelHeader;

#[derive(Component)]
struct PeerCount;

#[derive(Component)]
pub(crate) struct InputText;

/// Sidebar channel button. Stores the channel *name*.
#[derive(Component)]
struct ChannelButton(String);

/// Container for the channel button list so we can rebuild it dynamically.
#[derive(Component)]
struct ChannelList;

/// The main content area (chat + input OR settings panel).
#[derive(Component)]
struct MainContent;

/// Settings panel root.
#[derive(Component)]
struct SettingsPanel;

/// Chat panel root (channel header + messages + input).
#[derive(Component)]
struct ChatPanel;

/// Settings relay address text display.
#[derive(Component)]
pub(crate) struct SettingsRelayText;

/// Settings button in the sidebar.
#[derive(Component)]
struct SettingsButton;

/// Save button in settings.
#[derive(Component)]
struct SaveSettingsButton;

// ───── Helpers ──────────────────────────────────────────────────────────────

fn truncate_peer_id(s: &str) -> String {
    if s.len() > 12 {
        format!("{}...", &s[..12])
    } else {
        s.to_string()
    }
}

/// Build a gossipsub topic string from a server ID and channel name.
fn make_topic(server: &Server, channel_name: &str) -> String {
    format!("{}/{}", server.id, channel_name)
}

// ───── Systems ───────────────────────────────────────────────────────────────

/// Load or create server, fire ConnectCommand, and subscribe to channels.
fn init_server(
    identity: Res<LocalIdentity>,
    mut server_state: ResMut<ServerState>,
    mut key_store: ResMut<ChannelKeyStore>,
    mut connect_writer: MessageWriter<ConnectCommand>,
) {
    let (server, keys) = if let Some((server, keys)) = crate::storage::load_server() {
        info!("loaded server '{}' from disk", server.name);
        (server, keys)
    } else {
        info!("creating new server");
        let mut server = Server::new("My Server", identity.0.peer_id());
        let mut keys = HashMap::new();

        let default_channels = ["general", "random", "voice"];
        for name in default_channels {
            let ch_id = server
                .create_channel(name, ChannelKind::Text)
                .expect("default channel creation should not fail");

            let topic = make_topic(&server, name);
            if let Some(key) = server.channel_key(&ch_id) {
                keys.insert(topic, key.clone());
            }
        }

        (server, keys)
    };

    // Populate topic map.
    for ch in server.channels() {
        let topic = make_topic(&server, &ch.name);
        server_state
            .topic_map
            .insert(topic.clone(), (ch.name.clone(), ch.id.clone()));
    }

    key_store.keys = keys;
    crate::storage::save_server(&server, &key_store.keys);
    server_state.server = Some(server);

    // Connect to the network with saved relay settings.
    let settings = crate::storage::load_settings().unwrap_or_default();
    connect_writer.write(ConnectCommand {
        relay_addr: settings.relay_addr,
    });
}

/// Subscribe to all channels once the network is connected.
fn subscribe_channels(
    connected: Res<crate::network_bridge::NetworkConnected>,
    server_state: Res<ServerState>,
    net_cmd: Res<NetworkCommandSender>,
    mut subscribed: Local<bool>,
) {
    if *subscribed || !connected.0 {
        return;
    }
    *subscribed = true;

    for topic in server_state.topic_map.keys() {
        let _ = net_cmd
            .0
            .send(crate::network_bridge::NetworkBridgeCommand::Subscribe(
                topic.clone(),
            ));
    }
    info!("subscribed to {} channels", server_state.topic_map.len());
}

fn setup_ui(
    mut commands: Commands,
    identity: Res<LocalIdentity>,
    server_state: Res<ServerState>,
    settings_input: Res<SettingsInput>,
) {
    commands.spawn(Camera2d);

    let peer_display = truncate_peer_id(&identity.0.peer_id().to_string());
    let server_name = server_state
        .server
        .as_ref()
        .map(|s| s.name.as_str())
        .unwrap_or("Willow");
    let channel_names = server_state.channel_names();

    // Root container
    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            ..default()
        })
        .with_children(|root| {
            // ── Left sidebar ──
            root.spawn((
                Node {
                    width: Val::Px(220.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.15, 0.15, 0.18)),
            ))
            .with_children(|sidebar| {
                sidebar.spawn((
                    Text::new(server_name),
                    TextFont::from_font_size(24.0),
                    TextColor(Color::srgb(0.9, 0.9, 0.9)),
                ));

                sidebar.spawn((
                    Text::new(format!("You: {peer_display}")),
                    TextFont::from_font_size(11.0),
                    TextColor(Color::srgb(0.5, 0.5, 0.5)),
                ));

                sidebar.spawn(Node {
                    height: Val::Px(20.0),
                    ..default()
                });

                sidebar.spawn((
                    Text::new("CHANNELS"),
                    TextFont::from_font_size(11.0),
                    TextColor(Color::srgb(0.5, 0.5, 0.55)),
                ));

                // Channel list container
                sidebar
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            ..default()
                        },
                        ChannelList,
                    ))
                    .with_children(|list| {
                        for name in &channel_names {
                            spawn_channel_button(list, name);
                        }
                    });

                sidebar.spawn(Node {
                    flex_grow: 1.0,
                    ..default()
                });

                // Settings button
                sidebar
                    .spawn((
                        Button,
                        Node {
                            margin: UiRect::bottom(Val::Px(8.0)),
                            padding: UiRect::all(Val::Px(6.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.22, 0.22, 0.25)),
                        SettingsButton,
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new("Settings"),
                            TextFont::from_font_size(12.0),
                            TextColor(Color::srgb(0.7, 0.7, 0.7)),
                        ));
                    });

                sidebar.spawn((
                    Text::new("0 peers connected"),
                    TextFont::from_font_size(11.0),
                    TextColor(Color::srgb(0.4, 0.8, 0.4)),
                    PeerCount,
                ));
            });

            // ── Main content area ──
            root.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.2, 0.2, 0.22)),
                MainContent,
            ))
            .with_children(|main| {
                // ── Chat panel ──
                spawn_chat_panel(main, &channel_names);

                // ── Settings panel (hidden by default) ──
                spawn_settings_panel(main, &settings_input);
            });
        });
}

fn spawn_chat_panel(parent: &mut ChildSpawnerCommands, channel_names: &[String]) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            ChatPanel,
        ))
        .with_children(|chat| {
            // Channel header bar
            chat.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(48.0),
                    padding: UiRect::horizontal(Val::Px(16.0)),
                    align_items: AlignItems::Center,
                    border: UiRect::bottom(Val::Px(1.0)),
                    ..default()
                },
                BorderColor::all(Color::srgb(0.15, 0.15, 0.18)),
            ))
            .with_children(|header| {
                let first = channel_names
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("general");
                header.spawn((
                    Text::new(format!("# {first}")),
                    TextFont::from_font_size(18.0),
                    TextColor(Color::srgb(0.9, 0.9, 0.9)),
                    ChannelHeader,
                ));
            });

            // Message area
            chat.spawn((
                Node {
                    flex_grow: 1.0,
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::ColumnReverse,
                    padding: UiRect::all(Val::Px(16.0)),
                    overflow: Overflow::clip_y(),
                    ..default()
                },
                MessageList,
            ));

            // Input area
            chat.spawn((
                Node {
                    width: Val::Percent(100.0),
                    min_height: Val::Px(56.0),
                    padding: UiRect::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.17, 0.17, 0.19)),
            ))
            .with_children(|input_area| {
                input_area
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            min_height: Val::Px(32.0),
                            padding: UiRect::horizontal(Val::Px(12.0)),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.25, 0.25, 0.28)),
                    ))
                    .with_children(|input| {
                        input.spawn((
                            Text::new("Type a message..."),
                            TextFont::from_font_size(14.0),
                            TextColor(Color::srgb(0.45, 0.45, 0.48)),
                            InputText,
                        ));
                    });
            });
        });
}

fn spawn_settings_panel(parent: &mut ChildSpawnerCommands, settings: &SettingsInput) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(24.0)),
                display: Display::None, // hidden by default
                ..default()
            },
            SettingsPanel,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Settings"),
                TextFont::from_font_size(22.0),
                TextColor(Color::srgb(0.9, 0.9, 0.9)),
            ));

            panel.spawn(Node {
                height: Val::Px(20.0),
                ..default()
            });

            // Relay address section
            panel.spawn((
                Text::new("Relay Address"),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.6, 0.6, 0.65)),
            ));

            panel.spawn((
                Text::new("Connect to a relay server for peer discovery. Leave empty for LAN-only (mDNS)."),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgb(0.45, 0.45, 0.5)),
                Node {
                    margin: UiRect::vertical(Val::Px(4.0)),
                    ..default()
                },
            ));

            // Relay input field
            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        min_height: Val::Px(36.0),
                        padding: UiRect::horizontal(Val::Px(12.0)),
                        align_items: AlignItems::Center,
                        margin: UiRect::vertical(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.18, 0.18, 0.2)),
                ))
                .with_children(|field| {
                    let display = if settings.relay_addr.is_empty() {
                        "/ip4/.../tcp/9091/ws/p2p/12D3KooW..."
                    } else {
                        &settings.relay_addr
                    };
                    let color = if settings.relay_addr.is_empty() {
                        Color::srgb(0.4, 0.4, 0.45)
                    } else {
                        Color::srgb(0.85, 0.85, 0.85)
                    };
                    field.spawn((
                        Text::new(display),
                        TextFont::from_font_size(13.0),
                        TextColor(color),
                        SettingsRelayText,
                    ));
                });

            panel.spawn(Node {
                height: Val::Px(12.0),
                ..default()
            });

            // Save button
            panel
                .spawn((
                    Button,
                    Node {
                        padding: UiRect::new(
                            Val::Px(16.0),
                            Val::Px(16.0),
                            Val::Px(8.0),
                            Val::Px(8.0),
                        ),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.25, 0.5, 0.9)),
                    SaveSettingsButton,
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("Save & Reconnect"),
                        TextFont::from_font_size(14.0),
                        TextColor(Color::WHITE),
                    ));
                });

            panel.spawn(Node {
                height: Val::Px(16.0),
                ..default()
            });

            panel.spawn((
                Text::new("Example: /ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooW..."),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgb(0.4, 0.4, 0.45)),
            ));
        });
}

fn spawn_channel_button(parent: &mut ChildSpawnerCommands, name: &str) {
    parent
        .spawn((
            Button,
            Node {
                margin: UiRect::top(Val::Px(4.0)),
                padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(4.0), Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            ChannelButton(name.to_string()),
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new(format!("# {name}")),
                TextFont::from_font_size(15.0),
                TextColor(Color::srgb(0.7, 0.7, 0.7)),
            ));
        });
}

#[allow(clippy::type_complexity)]
pub(crate) fn handle_keyboard_input(
    mut key_events: MessageReader<KeyboardInput>,
    mut input: ResMut<InputState>,
    view: Res<AppView>,
    mut settings_input: ResMut<SettingsInput>,
    mut relay_text_query: Query<
        (&mut Text, &mut TextColor),
        (With<SettingsRelayText>, Without<InputText>),
    >,
) {
    for event in key_events.read() {
        if !event.state.is_pressed() {
            continue;
        }

        if *view == AppView::Settings {
            // Route keyboard input to relay address field.
            match event.key_code {
                KeyCode::Backspace => {
                    settings_input.relay_addr.pop();
                }
                KeyCode::Escape => {} // handled by toggle_view
                _ => {
                    if let Some(ref s) = event.text {
                        for c in s.chars() {
                            if !c.is_control() {
                                settings_input.relay_addr.push(c);
                            }
                        }
                    }
                }
            }

            // Update the relay text display.
            for (mut text, mut color) in &mut relay_text_query {
                if settings_input.relay_addr.is_empty() {
                    **text = "/ip4/.../tcp/9091/ws/p2p/12D3KooW...".to_string();
                    *color = TextColor(Color::srgb(0.4, 0.4, 0.45));
                } else {
                    **text = settings_input.relay_addr.clone();
                    *color = TextColor(Color::srgb(0.85, 0.85, 0.85));
                }
            }
        } else {
            // Chat mode: route to message input.
            match event.key_code {
                KeyCode::Enter => {
                    if !input.text.is_empty() {
                        input.send_requested = true;
                    }
                }
                KeyCode::Backspace => {
                    input.text.pop();
                }
                _ => {
                    if let Some(ref s) = event.text {
                        for c in s.chars() {
                            if !c.is_control() {
                                input.text.push(c);
                            }
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn send_message(
    mut input: ResMut<InputState>,
    mut state: ResMut<ChatState>,
    identity: Res<LocalIdentity>,
    net_cmd: Res<NetworkCommandSender>,
    key_store: Res<ChannelKeyStore>,
    server_state: Res<ServerState>,
) {
    if !input.send_requested {
        return;
    }
    input.send_requested = false;

    let body = input.text.drain(..).collect::<String>();
    if body.is_empty() {
        return;
    }

    let channel_name = state.current_channel.clone();
    let peer_id = identity.0.peer_id();

    // Resolve the gossipsub topic from the channel name.
    let topic = match server_state.topic_for_name(&channel_name) {
        Some(t) => t,
        None => channel_name.clone(), // fallback for tests
    };

    let channel_id = willow_messaging::ChannelId::new();
    let mut msg = Message::text(channel_id, peer_id.clone(), &body, &mut state.hlc);

    // Encrypt content if we have a key for this topic.
    if let Some(key) = key_store.keys.get(&topic) {
        if let Ok(sealed) = willow_crypto::seal_content(&msg.content, key, 0) {
            msg.content = Content::Encrypted(sealed);
        }
    }

    if let Ok(envelope_data) = pack_envelope(MessageType::Chat, &msg) {
        if let Ok(signed_data) = willow_identity::pack(&envelope_data, &identity.0) {
            let _ = net_cmd
                .0
                .send(crate::network_bridge::NetworkBridgeCommand::Publish {
                    topic: topic.clone(),
                    data: signed_data,
                });
        }
    }

    state.messages.push(ChatMessage {
        topic,
        author: truncate_peer_id(&peer_id.to_string()),
        body,
        is_local: true,
    });
    state.messages_dirty = true;
}

pub(crate) fn handle_network_events(
    mut reader: MessageReader<NetworkBridgeEvent>,
    mut state: ResMut<ChatState>,
    key_store: Res<ChannelKeyStore>,
    server_state: Res<ServerState>,
) {
    for event in reader.read() {
        match event {
            NetworkBridgeEvent::MessageReceived {
                topic,
                data,
                source: _,
            } => {
                // Verify Ed25519 signature and extract the signed envelope bytes.
                let Ok((envelope_data, signer)) = willow_identity::unpack::<Vec<u8>>(data) else {
                    continue;
                };

                let Ok((msg, MessageType::Chat)) = unpack_envelope::<Message>(&envelope_data)
                else {
                    continue;
                };

                let _ = &signer;

                // Decrypt if encrypted, pass through if cleartext.
                let content = match &msg.content {
                    Content::Encrypted(sealed) => {
                        let Some(key) = key_store.keys.get(topic) else {
                            continue;
                        };
                        match willow_crypto::open_content(sealed, key) {
                            Ok(c) => c,
                            Err(_) => continue,
                        }
                    }
                    other => other.clone(),
                };

                if let Content::Text { ref body } = content {
                    let author = truncate_peer_id(&signer.to_string());

                    // Resolve channel name for display, fall back to topic.
                    let _display_name =
                        server_state.name_for_topic(topic).unwrap_or(topic.as_str());

                    state.messages.push(ChatMessage {
                        topic: topic.clone(),
                        author,
                        body: body.clone(),
                        is_local: false,
                    });
                    state.messages_dirty = true;
                    state.hlc.receive(msg.hlc);
                }
            }
            NetworkBridgeEvent::PeerConnected(peer) => {
                if !state.peers.contains(peer) {
                    state.peers.push(peer.clone());
                }
            }
            NetworkBridgeEvent::PeerDisconnected(peer) => {
                state.peers.retain(|p| p != peer);
            }
            NetworkBridgeEvent::Listening(addr) => {
                info!("Listening on {addr}");
            }
        }
    }
}

fn handle_channel_click(
    interaction_query: Query<(&Interaction, &ChannelButton), Changed<Interaction>>,
    mut state: ResMut<ChatState>,
) {
    for (interaction, button) in &interaction_query {
        if *interaction == Interaction::Pressed && state.current_channel != button.0 {
            state.current_channel = button.0.clone();
            state.messages_dirty = true;
        }
    }
}

fn sync_message_list(
    mut commands: Commands,
    mut state: ResMut<ChatState>,
    list_query: Query<Entity, With<MessageList>>,
    server_state: Res<ServerState>,
) {
    if !state.messages_dirty {
        return;
    }
    state.messages_dirty = false;

    let Ok(list_entity) = list_query.single() else {
        return;
    };

    commands.entity(list_entity).detach_all_children();

    // Find the topic for the current channel name.
    let current_topic = server_state
        .topic_for_name(&state.current_channel)
        .unwrap_or_default();

    let visible: Vec<_> = state
        .messages
        .iter()
        .filter(|m| m.topic == current_topic)
        .collect();

    if visible.is_empty() {
        commands.entity(list_entity).with_children(|parent| {
            parent.spawn((
                Text::new(format!(
                    "Welcome to #{}! This is a P2P chat — no servers, no middlemen.",
                    state.current_channel
                )),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.5, 0.5, 0.55)),
            ));
        });
        return;
    }

    commands.entity(list_entity).with_children(|parent| {
        for msg in &visible {
            let author_color = if msg.is_local {
                Color::srgb(0.5, 0.7, 1.0)
            } else {
                Color::srgb(0.9, 0.7, 0.4)
            };

            parent
                .spawn(Node {
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new(format!("{}: ", msg.author)),
                        TextFont::from_font_size(14.0),
                        TextColor(author_color),
                    ))
                    .with_child((
                        TextSpan::new(&msg.body),
                        TextFont::from_font_size(14.0),
                        TextColor(Color::srgb(0.85, 0.85, 0.85)),
                    ));
                });
        }
    });
}

fn sync_input_text(
    input: Res<InputState>,
    mut query: Query<(&mut Text, &mut TextColor), With<InputText>>,
) {
    if !input.is_changed() {
        return;
    }
    for (mut text, mut color) in &mut query {
        if input.text.is_empty() {
            **text = "Type a message...".to_string();
            *color = TextColor(Color::srgb(0.45, 0.45, 0.48));
        } else {
            **text = input.text.clone();
            *color = TextColor(Color::srgb(0.9, 0.9, 0.9));
        }
    }
}

fn update_peer_count(state: Res<ChatState>, mut query: Query<&mut Text, With<PeerCount>>) {
    if !state.is_changed() {
        return;
    }
    for mut text in &mut query {
        **text = format!("{} peer(s) connected", state.peers.len());
    }
}

fn update_channel_header(state: Res<ChatState>, mut query: Query<&mut Text, With<ChannelHeader>>) {
    if !state.is_changed() {
        return;
    }
    for mut text in &mut query {
        **text = format!("# {}", state.current_channel);
    }
}

fn update_channel_highlights(
    state: Res<ChatState>,
    query: Query<(&ChannelButton, &Children)>,
    mut text_query: Query<&mut TextColor>,
) {
    if !state.is_changed() {
        return;
    }
    for (button, children) in &query {
        let is_active = button.0 == state.current_channel;
        for child in children.iter() {
            if let Ok(mut color) = text_query.get_mut(child) {
                *color = if is_active {
                    TextColor(Color::WHITE)
                } else {
                    TextColor(Color::srgb(0.7, 0.7, 0.7))
                };
            }
        }
    }
}

// ───── Settings Systems ─────────────────────────────────────────────────────

/// Toggle between Chat and Settings when the settings button is clicked.
fn handle_settings_button(
    query: Query<&Interaction, (Changed<Interaction>, With<SettingsButton>)>,
    mut view: ResMut<AppView>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            *view = match *view {
                AppView::Chat => AppView::Settings,
                AppView::Settings => AppView::Chat,
            };
        }
    }
}

/// Handle "Save & Reconnect" button in settings.
fn handle_save_settings(
    query: Query<&Interaction, (Changed<Interaction>, With<SaveSettingsButton>)>,
    settings_input: Res<SettingsInput>,
    mut connect_writer: MessageWriter<ConnectCommand>,
    mut view: ResMut<AppView>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            let relay = if settings_input.relay_addr.trim().is_empty() {
                None
            } else {
                Some(settings_input.relay_addr.trim().to_string())
            };

            connect_writer.write(ConnectCommand { relay_addr: relay });
            *view = AppView::Chat;
            info!("settings saved, reconnecting");
        }
    }
}

/// Show/hide chat and settings panels based on current view.
#[allow(clippy::type_complexity)]
fn toggle_view(
    view: Res<AppView>,
    mut chat_query: Query<&mut Node, (With<ChatPanel>, Without<SettingsPanel>)>,
    mut settings_query: Query<&mut Node, (With<SettingsPanel>, Without<ChatPanel>)>,
    mut settings_input: ResMut<SettingsInput>,
    mut relay_text_query: Query<
        (&mut Text, &mut TextColor),
        (With<SettingsRelayText>, Without<InputText>),
    >,
) {
    if !view.is_changed() {
        return;
    }

    for mut node in &mut chat_query {
        node.display = if *view == AppView::Chat {
            Display::Flex
        } else {
            Display::None
        };
    }

    for mut node in &mut settings_query {
        node.display = if *view == AppView::Settings {
            Display::Flex
        } else {
            Display::None
        };
    }

    // When entering settings, reload the saved relay address.
    if *view == AppView::Settings {
        let saved = crate::storage::load_settings().unwrap_or_default();
        settings_input.relay_addr = saved.relay_addr.unwrap_or_default();

        for (mut text, mut color) in &mut relay_text_query {
            if settings_input.relay_addr.is_empty() {
                **text = "/ip4/.../tcp/9091/ws/p2p/12D3KooW...".to_string();
                *color = TextColor(Color::srgb(0.4, 0.4, 0.45));
            } else {
                **text = settings_input.relay_addr.clone();
                *color = TextColor(Color::srgb(0.85, 0.85, 0.85));
            }
        }
    }
}

// ───── Settings Keyboard Input ──────────────────────────────────────────────

// The existing handle_keyboard_input routes to the chat input. When settings
// is active, keyboard input goes to the relay address field instead. Let's
// update handle_keyboard_input to be view-aware.
