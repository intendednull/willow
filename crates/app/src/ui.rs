//! # UI Module
//!
//! Top-level Bevy UI layout for the Willow chat client.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use std::collections::HashMap;

use crate::network_bridge::{LocalIdentity, NetworkBridgeEvent, NetworkCommandSender};
use willow_crypto::ChannelKey;
use willow_messaging::hlc::HLC;
use willow_messaging::{ChannelId, Content, Message};
use willow_transport::{pack_envelope, unpack_envelope, MessageType};

/// The gossipsub topic names for each channel.
const CHANNELS: &[&str] = &["general", "random", "voice"];

/// Plugin for all UI systems and resources.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChatState::new())
            .insert_resource(InputState::default())
            .insert_resource(ChannelKeyStore::default())
            .add_systems(Startup, setup_ui)
            // Split into two add_systems calls to stay within the 8-tuple limit.
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
                    sync_input_text,
                    update_peer_count,
                    update_channel_header,
                    update_channel_highlights,
                ),
            );
    }
}

// ───── Resources ─────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct ChatState {
    pub messages: Vec<ChatMessage>,
    pub current_channel: String,
    pub peers: Vec<String>,
    pub hlc: HLC,
    pub(crate) messages_dirty: bool,
}

impl ChatState {
    pub(crate) fn new() -> Self {
        Self {
            messages: Vec::new(),
            current_channel: CHANNELS[0].to_string(),
            peers: Vec::new(),
            hlc: HLC::new(),
            messages_dirty: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub channel: String,
    pub author: String,
    pub body: String,
    pub is_local: bool,
}

#[derive(Resource, Default)]
pub(crate) struct InputState {
    pub(crate) text: String,
    pub(crate) send_requested: bool,
}

/// Per-channel symmetric encryption keys.
///
/// Populated when creating/joining a server. Messages are encrypted before
/// sending and decrypted on receipt using the channel's key.
#[derive(Resource, Default)]
pub(crate) struct ChannelKeyStore {
    pub(crate) keys: HashMap<String, ChannelKey>,
}

// ───── Components ────────────────────────────────────────────────────────────

#[derive(Component)]
struct MessageList;

#[derive(Component)]
struct ChannelHeader;

#[derive(Component)]
struct PeerCount;

#[derive(Component)]
struct InputText;

#[derive(Component)]
struct ChannelButton(String);

// ───── Helpers ──────────────────────────────────────────────────────────────

fn truncate_peer_id(s: &str) -> String {
    if s.len() > 12 {
        format!("{}...", &s[..12])
    } else {
        s.to_string()
    }
}

// ───── Systems ───────────────────────────────────────────────────────────────

fn setup_ui(
    mut commands: Commands,
    identity: Res<LocalIdentity>,
    net_cmd: Res<NetworkCommandSender>,
) {
    commands.spawn(Camera2d);

    let peer_display = truncate_peer_id(&identity.0.peer_id().to_string());

    for ch in CHANNELS {
        let _ = net_cmd
            .0
            .send(crate::network_bridge::NetworkBridgeCommand::Subscribe(
                ch.to_string(),
            ));
    }

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
                    Text::new("Willow"),
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

                for name in CHANNELS {
                    sidebar
                        .spawn((
                            Button,
                            Node {
                                margin: UiRect::top(Val::Px(4.0)),
                                padding: UiRect::new(
                                    Val::Px(8.0),
                                    Val::Px(8.0),
                                    Val::Px(4.0),
                                    Val::Px(4.0),
                                ),
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

                sidebar.spawn(Node {
                    flex_grow: 1.0,
                    ..default()
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
            ))
            .with_children(|main| {
                // Channel header bar
                main.spawn((
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
                    header.spawn((
                        Text::new(format!("# {}", CHANNELS[0])),
                        TextFont::from_font_size(18.0),
                        TextColor(Color::srgb(0.9, 0.9, 0.9)),
                        ChannelHeader,
                    ));
                });

                // Message area
                main.spawn((
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
                main.spawn((
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
        });
}

pub(crate) fn handle_keyboard_input(
    mut key_events: MessageReader<KeyboardInput>,
    mut input: ResMut<InputState>,
) {
    for event in key_events.read() {
        if !event.state.is_pressed() {
            continue;
        }
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

pub(crate) fn send_message(
    mut input: ResMut<InputState>,
    mut state: ResMut<ChatState>,
    identity: Res<LocalIdentity>,
    net_cmd: Res<NetworkCommandSender>,
    key_store: Res<ChannelKeyStore>,
) {
    if !input.send_requested {
        return;
    }
    input.send_requested = false;

    let body = input.text.drain(..).collect::<String>();
    if body.is_empty() {
        return;
    }

    let channel = state.current_channel.clone();
    let peer_id = identity.0.peer_id();

    let channel_id = ChannelId::new();
    let mut msg = Message::text(channel_id, peer_id.clone(), &body, &mut state.hlc);

    // Encrypt content if we have a key for this channel.
    if let Some(key) = key_store.keys.get(&channel) {
        if let Ok(sealed) = willow_crypto::seal_content(&msg.content, key, 0) {
            msg.content = Content::Encrypted(sealed);
        }
    }

    if let Ok(envelope_data) = pack_envelope(MessageType::Chat, &msg) {
        // Sign the envelope with our Ed25519 key for author verification.
        if let Ok(signed_data) = willow_identity::pack(&envelope_data, &identity.0) {
            let _ = net_cmd
                .0
                .send(crate::network_bridge::NetworkBridgeCommand::Publish {
                    topic: channel.clone(),
                    data: signed_data,
                });
        }
    }

    state.messages.push(ChatMessage {
        channel,
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
                    continue; // invalid or missing signature
                };

                let Ok((msg, MessageType::Chat)) = unpack_envelope::<Message>(&envelope_data)
                else {
                    continue;
                };

                let _ = &signer; // verified author PeerId

                // Decrypt if encrypted, pass through if cleartext.
                let content = match &msg.content {
                    Content::Encrypted(sealed) => {
                        let Some(key) = key_store.keys.get(topic) else {
                            continue; // no key for this channel
                        };
                        match willow_crypto::open_content(sealed, key) {
                            Ok(c) => c,
                            Err(_) => continue, // decryption failed
                        }
                    }
                    other => other.clone(),
                };

                if let Content::Text { ref body } = content {
                    let author = truncate_peer_id(&signer.to_string());
                    state.messages.push(ChatMessage {
                        channel: topic.clone(),
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
) {
    if !state.messages_dirty {
        return;
    }
    state.messages_dirty = false;

    let Ok(list_entity) = list_query.single() else {
        return;
    };

    commands.entity(list_entity).detach_all_children();

    let current = &state.current_channel;
    let visible: Vec<_> = state
        .messages
        .iter()
        .filter(|m| m.channel == *current)
        .collect();

    if visible.is_empty() {
        commands.entity(list_entity).with_children(|parent| {
            parent.spawn((
                Text::new(format!(
                    "Welcome to #{}! This is a P2P chat — no servers, no middlemen.",
                    current
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
