//! # UI Module
//!
//! Top-level Bevy UI layout for the Willow chat client.
//!
//! ## Layout (Bevy 0.14)
//!
//! ```text
//! ┌──────────┬──────────────────────────────────┐
//! │          │  #general                         │
//! │ Servers  │                                   │
//! │          │  Alice: hey everyone!             │
//! │ ──────── │  Bob: what's up?                  │
//! │ #general │                                   │
//! │ #random  │                                   │
//! │ #voice   │                                   │
//! │          │ ┌──────────────────────────────┐  │
//! │          │ │ Type a message...            │  │
//! │          │ └──────────────────────────────┘  │
//! └──────────┴──────────────────────────────────┘
//! ```

use bevy::prelude::*;

use crate::network_bridge::{LocalIdentity, NetworkBridgeEvent};

/// Plugin for all UI systems and resources.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChatState::default())
            .add_systems(Startup, setup_ui)
            .add_systems(Update, (handle_network_events, update_peer_count));
    }
}

// ───── Resources ─────────────────────────────────────────────────────────────

/// Holds the current chat state visible to the UI.
#[derive(Resource, Default)]
pub struct ChatState {
    /// Messages in the currently active channel.
    pub messages: Vec<ChatMessage>,
    /// Name of the current channel.
    pub current_channel: String,
    /// Connected peers.
    pub peers: Vec<String>,
}

/// A rendered chat message.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub author: String,
    pub body: String,
}

// ───── Components ────────────────────────────────────────────────────────────

/// Marker for the message list container.
#[derive(Component)]
struct MessageList;

/// Marker for the channel name header.
#[derive(Component)]
struct ChannelHeader;

/// Marker for the peer count display.
#[derive(Component)]
struct PeerCount;

// ───── Systems ───────────────────────────────────────────────────────────────

/// Build the initial UI layout.
fn setup_ui(mut commands: Commands, identity: Res<LocalIdentity>) {
    // Camera
    commands.spawn(Camera2dBundle::default());

    let peer_id_str = format!("{}", identity.0.peer_id());
    let peer_display = if peer_id_str.len() > 12 {
        format!("{}...", &peer_id_str[..12])
    } else {
        peer_id_str
    };

    // Root container — fills the window, horizontal flex.
    commands
        .spawn(NodeBundle {
            style: Style {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                ..default()
            },
            ..default()
        })
        .with_children(|root| {
            // ── Left sidebar ──
            root.spawn(NodeBundle {
                style: Style {
                    width: Val::Px(220.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(12.0)),
                    ..default()
                },
                background_color: Color::srgb(0.15, 0.15, 0.18).into(),
                ..default()
            })
            .with_children(|sidebar| {
                // App title.
                sidebar.spawn(TextBundle::from_section(
                    "Willow",
                    TextStyle {
                        font_size: 24.0,
                        color: Color::srgb(0.9, 0.9, 0.9),
                        ..default()
                    },
                ));

                // Peer ID.
                sidebar.spawn(TextBundle::from_section(
                    format!("You: {peer_display}"),
                    TextStyle {
                        font_size: 11.0,
                        color: Color::srgb(0.5, 0.5, 0.5),
                        ..default()
                    },
                ));

                // Spacer.
                sidebar.spawn(NodeBundle {
                    style: Style {
                        height: Val::Px(20.0),
                        ..default()
                    },
                    ..default()
                });

                // Channel list header.
                sidebar.spawn(TextBundle::from_section(
                    "CHANNELS",
                    TextStyle {
                        font_size: 11.0,
                        color: Color::srgb(0.5, 0.5, 0.55),
                        ..default()
                    },
                ));

                // Channel entries.
                for name in ["# general", "# random", "# voice"] {
                    sidebar.spawn(TextBundle {
                        text: Text::from_section(
                            name,
                            TextStyle {
                                font_size: 15.0,
                                color: Color::srgb(0.7, 0.7, 0.7),
                                ..default()
                            },
                        ),
                        style: Style {
                            margin: UiRect::top(Val::Px(6.0)),
                            ..default()
                        },
                        ..default()
                    });
                }

                // Flexible spacer.
                sidebar.spawn(NodeBundle {
                    style: Style {
                        flex_grow: 1.0,
                        ..default()
                    },
                    ..default()
                });

                // Peer count.
                sidebar.spawn((
                    TextBundle::from_section(
                        "0 peers connected",
                        TextStyle {
                            font_size: 11.0,
                            color: Color::srgb(0.4, 0.8, 0.4),
                            ..default()
                        },
                    ),
                    PeerCount,
                ));
            });

            // ── Main content area ──
            root.spawn(NodeBundle {
                style: Style {
                    flex_grow: 1.0,
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                background_color: Color::srgb(0.2, 0.2, 0.22).into(),
                ..default()
            })
            .with_children(|main| {
                // Channel header bar.
                main.spawn(NodeBundle {
                    style: Style {
                        width: Val::Percent(100.0),
                        height: Val::Px(48.0),
                        padding: UiRect::horizontal(Val::Px(16.0)),
                        align_items: AlignItems::Center,
                        border: UiRect::bottom(Val::Px(1.0)),
                        ..default()
                    },
                    border_color: Color::srgb(0.15, 0.15, 0.18).into(),
                    ..default()
                })
                .with_children(|header| {
                    header.spawn((
                        TextBundle::from_section(
                            "# general",
                            TextStyle {
                                font_size: 18.0,
                                color: Color::srgb(0.9, 0.9, 0.9),
                                ..default()
                            },
                        ),
                        ChannelHeader,
                    ));
                });

                // Message area.
                main.spawn((
                    NodeBundle {
                        style: Style {
                            flex_grow: 1.0,
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::all(Val::Px(16.0)),
                            overflow: Overflow::clip_y(),
                            ..default()
                        },
                        ..default()
                    },
                    MessageList,
                ))
                .with_children(|messages| {
                    messages.spawn(TextBundle::from_section(
                        "Welcome to Willow! This is a P2P chat — no servers, no middlemen.",
                        TextStyle {
                            font_size: 14.0,
                            color: Color::srgb(0.5, 0.5, 0.55),
                            ..default()
                        },
                    ));
                });

                // Input area.
                main.spawn(NodeBundle {
                    style: Style {
                        width: Val::Percent(100.0),
                        height: Val::Px(56.0),
                        padding: UiRect::all(Val::Px(12.0)),
                        ..default()
                    },
                    background_color: Color::srgb(0.17, 0.17, 0.19).into(),
                    ..default()
                })
                .with_children(|input_area| {
                    input_area
                        .spawn(NodeBundle {
                            style: Style {
                                width: Val::Percent(100.0),
                                height: Val::Percent(100.0),
                                padding: UiRect::horizontal(Val::Px(12.0)),
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            background_color: Color::srgb(0.25, 0.25, 0.28).into(),
                            ..default()
                        })
                        .with_children(|input| {
                            input.spawn(TextBundle::from_section(
                                "Type a message... (input coming soon)",
                                TextStyle {
                                    font_size: 14.0,
                                    color: Color::srgb(0.45, 0.45, 0.48),
                                    ..default()
                                },
                            ));
                        });
                });
            });
        });
}

/// Process incoming network events and update the chat state.
fn handle_network_events(
    mut events: EventReader<NetworkBridgeEvent>,
    mut state: ResMut<ChatState>,
) {
    for event in events.read() {
        match event {
            NetworkBridgeEvent::MessageReceived { data, source, .. } => {
                if let Ok(body) = String::from_utf8(data.clone()) {
                    state.messages.push(ChatMessage {
                        author: source
                            .as_ref()
                            .map(|s| {
                                if s.len() > 12 {
                                    format!("{}...", &s[..12])
                                } else {
                                    s.clone()
                                }
                            })
                            .unwrap_or_else(|| "unknown".into()),
                        body,
                    });
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
            _ => {}
        }
    }
}

/// Update the peer count label when state changes.
fn update_peer_count(state: Res<ChatState>, mut query: Query<&mut Text, With<PeerCount>>) {
    if !state.is_changed() {
        return;
    }
    for mut text in &mut query {
        text.sections[0].value = format!("{} peer(s) connected", state.peers.len());
    }
}
