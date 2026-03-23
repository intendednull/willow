//! # UI Module
//!
//! Top-level Bevy UI layout for the Willow chat client.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use std::collections::HashMap;

use crate::network_bridge::{
    ConnectCommand, LocalIdentity, NetworkBridgeEvent, NetworkCommandSender,
};
use crate::theme;
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
            .insert_resource(ProfileStore::default())
            .insert_resource(FilePicker::default())
            .insert_resource(UnreadCounts::default())
            .insert_resource(MessageDbRes(
                crate::storage::open_message_db()
                    .map(|db| std::sync::Arc::new(std::sync::Mutex::new(db))),
            ))
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
                (
                    handle_settings_button,
                    handle_save_settings,
                    toggle_view,
                    handle_share_file_button,
                    poll_file_picker,
                ),
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
    #[allow(dead_code)]
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
    /// HLC timestamp in milliseconds (for display).
    pub timestamp_ms: u64,
}

/// Tracks unread message counts per channel topic.
#[derive(Resource, Default)]
pub(crate) struct UnreadCounts {
    pub(crate) counts: HashMap<String, usize>,
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

/// Persistent message database.
#[derive(Resource, Clone)]
pub(crate) struct MessageDbRes(
    pub(crate) Option<std::sync::Arc<std::sync::Mutex<crate::storage::MessageDb>>>,
);

/// Maps PeerId strings → display names. Updated from profile broadcasts.
#[derive(Resource, Default, Clone)]
pub(crate) struct ProfileStore {
    pub(crate) names: HashMap<String, String>,
}

impl ProfileStore {
    /// Look up a display name for a peer, falling back to truncated ID.
    pub(crate) fn display_name(&self, peer_id: &str) -> String {
        self.names
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| truncate_peer_id(peer_id))
    }
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
    pub(crate) display_name: String,
    /// Which field is currently focused in settings.
    pub(crate) focused_field: SettingsField,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsField {
    #[default]
    DisplayName,
    RelayAddr,
}

impl Default for SettingsInput {
    fn default() -> Self {
        let saved_settings = crate::storage::load_settings().unwrap_or_default();
        let saved_profile = crate::storage::load_profile().unwrap_or_default();
        Self {
            relay_addr: saved_settings.relay_addr.unwrap_or_default(),
            display_name: saved_profile.display_name,
            focused_field: SettingsField::DisplayName,
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

/// Display name text field in settings.
#[derive(Component)]
pub(crate) struct SettingsNameText;

/// The sidebar display of the local user's name.
#[derive(Component)]
struct LocalUserDisplay;

/// "Share File" button in the input area.
#[derive(Component)]
struct ShareFileButton;

/// File data from the picker: (filename, mime_type, data).
type FilePickerResult = (String, String, Vec<u8>);

/// Tracks pending file picker operations.
#[derive(Resource, Clone)]
pub(crate) struct FilePicker {
    rx: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<FilePickerResult>>>>,
}

impl Default for FilePicker {
    fn default() -> Self {
        Self {
            rx: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

// ───── Helpers ──────────────────────────────────────────────────────────────

fn truncate_peer_id(s: &str) -> String {
    if s.len() > 12 {
        format!("{}...", &s[..12])
    } else {
        s.to_string()
    }
}

/// Format a millisecond timestamp as "HH:MM".
pub(crate) fn format_timestamp(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    let secs = ms / 1000;
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    format!("{hours:02}:{minutes:02}")
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
    mut state: ResMut<ChatState>,
    db: Res<MessageDbRes>,
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

    // Load persisted messages from the database.
    if let Some(ref db_arc) = db.0 {
        if let Ok(db_lock) = db_arc.lock() {
            for topic in server_state.topic_map.keys() {
                let stored = db_lock.load_topic(topic, 500);
                for sm in stored {
                    state.messages.push(ChatMessage {
                        topic: sm.topic,
                        author: sm.author,
                        body: sm.body,
                        is_local: sm.is_local,
                        timestamp_ms: sm.timestamp_ms,
                    });
                }
            }
            if !state.messages.is_empty() {
                state.messages_dirty = true;
                info!("loaded {} messages from database", state.messages.len());
            }
        }
    }

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
    mut profiles: ResMut<ProfileStore>,
) {
    commands.spawn(Camera2d);

    // Register our own display name in the profile store.
    let peer_id_str = identity.0.peer_id().to_string();
    let local_name = if settings_input.display_name.is_empty() {
        truncate_peer_id(&peer_id_str)
    } else {
        settings_input.display_name.clone()
    };
    profiles
        .names
        .insert(peer_id_str.clone(), local_name.clone());
    let peer_display = local_name;
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
                    width: Val::Px(240.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                BackgroundColor(theme::SIDEBAR_BG),
            ))
            .with_children(|sidebar| {
                // Server name header
                sidebar
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(48.0),
                            padding: UiRect::horizontal(Val::Px(16.0)),
                            align_items: AlignItems::Center,
                            border: UiRect::bottom(Val::Px(1.0)),
                            ..default()
                        },
                        BorderColor::all(theme::DIVIDER),
                    ))
                    .with_children(|header| {
                        header.spawn((
                            Text::new(server_name),
                            TextFont::from_font_size(16.0),
                            TextColor(theme::TEXT_PRIMARY),
                        ));
                    });

                // Channel section
                sidebar.spawn((
                    Text::new("TEXT CHANNELS"),
                    TextFont::from_font_size(11.0),
                    TextColor(theme::TEXT_HEADER),
                    Node {
                        padding: UiRect::new(
                            Val::Px(16.0),
                            Val::Px(8.0),
                            Val::Px(16.0),
                            Val::Px(4.0),
                        ),
                        ..default()
                    },
                ));

                // Channel list
                sidebar
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            ..default()
                        },
                        ChannelList,
                    ))
                    .with_children(|list| {
                        for name in &channel_names {
                            spawn_channel_button(list, name);
                        }
                    });

                // Spacer
                sidebar.spawn(Node {
                    flex_grow: 1.0,
                    ..default()
                });

                // User area (bottom of sidebar)
                sidebar
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(52.0),
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            align_items: AlignItems::Center,
                            flex_direction: FlexDirection::Row,
                            ..default()
                        },
                        BackgroundColor(theme::USER_AREA_BG),
                    ))
                    .with_children(|user_area| {
                        // Avatar circle
                        user_area.spawn((
                            Node {
                                width: Val::Px(32.0),
                                height: Val::Px(32.0),
                                margin: UiRect::right(Val::Px(8.0)),
                                ..default()
                            },
                            BackgroundColor(theme::ACCENT),
                        ));

                        // Name + peer count column
                        user_area
                            .spawn(Node {
                                flex_direction: FlexDirection::Column,
                                flex_grow: 1.0,
                                ..default()
                            })
                            .with_children(|info| {
                                info.spawn((
                                    Text::new(peer_display),
                                    TextFont::from_font_size(13.0),
                                    TextColor(theme::TEXT_PRIMARY),
                                    LocalUserDisplay,
                                ));
                                info.spawn((
                                    Text::new("0 peers"),
                                    TextFont::from_font_size(11.0),
                                    TextColor(theme::STATUS_ONLINE),
                                    PeerCount,
                                ));
                            });

                        // Settings gear button
                        user_area
                            .spawn((
                                Button,
                                Node {
                                    width: Val::Px(32.0),
                                    height: Val::Px(32.0),
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::NONE),
                                SettingsButton,
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    Text::new("⚙"),
                                    TextFont::from_font_size(18.0),
                                    TextColor(theme::TEXT_MUTED),
                                ));
                            });
                    });
            });

            // ── Main content area ──
            root.spawn((
                Node {
                    flex_grow: 1.0,
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                BackgroundColor(theme::MAIN_BG),
                MainContent,
            ))
            .with_children(|main| {
                spawn_chat_panel(main, &channel_names);
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
                BackgroundColor(theme::INPUT_BG),
                BorderColor::all(theme::DIVIDER),
            ))
            .with_children(|header| {
                let first = channel_names
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("general");
                header.spawn((
                    Text::new(format!("# {first}")),
                    TextFont::from_font_size(16.0),
                    TextColor(theme::TEXT_PRIMARY),
                    ChannelHeader,
                ));
            });

            // Message area
            chat.spawn((
                Node {
                    flex_grow: 1.0,
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::ColumnReverse,
                    padding: UiRect::new(Val::Px(16.0), Val::Px(16.0), Val::Px(8.0), Val::Px(8.0)),
                    overflow: Overflow::clip_y(),
                    ..default()
                },
                MessageList,
            ));

            // Input area
            chat.spawn((Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(68.0),
                padding: UiRect::new(Val::Px(16.0), Val::Px(16.0), Val::Px(0.0), Val::Px(16.0)),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                ..default()
            },))
                .with_children(|input_area| {
                    // Share file button (left of input)
                    input_area
                        .spawn((
                            Button,
                            Node {
                                width: Val::Px(36.0),
                                height: Val::Px(36.0),
                                margin: UiRect::right(Val::Px(8.0)),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            BackgroundColor(theme::INPUT_FIELD_BG),
                            ShareFileButton,
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new("+"),
                                TextFont::from_font_size(20.0),
                                TextColor(theme::TEXT_MUTED),
                            ));
                        });

                    // Text input field
                    input_area
                        .spawn((
                            Node {
                                flex_grow: 1.0,
                                min_height: Val::Px(40.0),
                                padding: UiRect::horizontal(Val::Px(16.0)),
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(theme::INPUT_FIELD_BG),
                        ))
                        .with_children(|input| {
                            input.spawn((
                                Text::new("Type a message..."),
                                TextFont::from_font_size(14.0),
                                TextColor(theme::TEXT_PLACEHOLDER),
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
                TextColor(theme::TEXT_PRIMARY),
            ));

            panel.spawn(Node {
                height: Val::Px(20.0),
                ..default()
            });

            // Display name section
            panel.spawn((
                Text::new("Display Name"),
                TextFont::from_font_size(13.0),
                TextColor(theme::TEXT_SECONDARY),
            ));

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
                    BackgroundColor(theme::INPUT_FIELD_BG),
                ))
                .with_children(|field| {
                    let display = if settings.display_name.is_empty() {
                        "Enter your name..."
                    } else {
                        &settings.display_name
                    };
                    let color = if settings.display_name.is_empty() {
                        theme::TEXT_PLACEHOLDER
                    } else {
                        theme::TEXT_PRIMARY
                    };
                    field.spawn((
                        Text::new(display),
                        TextFont::from_font_size(13.0),
                        TextColor(color),
                        SettingsNameText,
                    ));
                });

            panel.spawn(Node {
                height: Val::Px(12.0),
                ..default()
            });

            // Relay address section
            panel.spawn((
                Text::new("Relay Address"),
                TextFont::from_font_size(13.0),
                TextColor(theme::TEXT_SECONDARY),
            ));

            panel.spawn((
                Text::new("Connect to a relay server for peer discovery. Leave empty for LAN-only (mDNS)."),
                TextFont::from_font_size(11.0),
                TextColor(theme::TEXT_PLACEHOLDER),
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
                    BackgroundColor(theme::INPUT_FIELD_BG),
                ))
                .with_children(|field| {
                    let display = if settings.relay_addr.is_empty() {
                        "/ip4/.../tcp/9091/ws/p2p/12D3KooW..."
                    } else {
                        &settings.relay_addr
                    };
                    let color = if settings.relay_addr.is_empty() {
                        theme::TEXT_PLACEHOLDER
                    } else {
                        theme::TEXT_PRIMARY
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
                    BackgroundColor(theme::ACCENT),
                    SaveSettingsButton,
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("Save & Reconnect"),
                        TextFont::from_font_size(14.0),
                        TextColor(theme::TEXT_PRIMARY),
                    ));
                });

            panel.spawn(Node {
                height: Val::Px(16.0),
                ..default()
            });

            panel.spawn((
                Text::new("Example: /ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooW..."),
                TextFont::from_font_size(11.0),
                TextColor(theme::TEXT_PLACEHOLDER),
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
                TextColor(theme::TEXT_MUTED),
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
        (
            With<SettingsRelayText>,
            Without<InputText>,
            Without<SettingsNameText>,
        ),
    >,
    mut name_text_query: Query<
        (&mut Text, &mut TextColor),
        (
            With<SettingsNameText>,
            Without<InputText>,
            Without<SettingsRelayText>,
        ),
    >,
) {
    for event in key_events.read() {
        if !event.state.is_pressed() {
            continue;
        }

        if *view == AppView::Settings {
            // Tab switches between fields.
            if event.key_code == KeyCode::Tab {
                settings_input.focused_field = match settings_input.focused_field {
                    SettingsField::DisplayName => SettingsField::RelayAddr,
                    SettingsField::RelayAddr => SettingsField::DisplayName,
                };
                continue;
            }

            // Route to the focused field.
            let target = match settings_input.focused_field {
                SettingsField::DisplayName => &mut settings_input.display_name,
                SettingsField::RelayAddr => &mut settings_input.relay_addr,
            };

            match event.key_code {
                KeyCode::Backspace => {
                    target.pop();
                }
                KeyCode::Escape => {}
                _ => {
                    if let Some(ref s) = event.text {
                        for c in s.chars() {
                            if !c.is_control() {
                                target.push(c);
                            }
                        }
                    }
                }
            }

            // Update display name text.
            for (mut text, mut color) in &mut name_text_query {
                if settings_input.display_name.is_empty() {
                    **text = "Enter your name...".to_string();
                    *color = TextColor(theme::TEXT_PLACEHOLDER);
                } else {
                    **text = settings_input.display_name.clone();
                    *color = TextColor(theme::TEXT_PRIMARY);
                }
            }

            // Update relay text.
            for (mut text, mut color) in &mut relay_text_query {
                if settings_input.relay_addr.is_empty() {
                    **text = "/ip4/.../tcp/9091/ws/p2p/12D3KooW...".to_string();
                    *color = TextColor(theme::TEXT_PLACEHOLDER);
                } else {
                    **text = settings_input.relay_addr.clone();
                    *color = TextColor(theme::TEXT_PRIMARY);
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn send_message(
    mut input: ResMut<InputState>,
    mut state: ResMut<ChatState>,
    identity: Res<LocalIdentity>,
    net_cmd: Res<NetworkCommandSender>,
    key_store: Res<ChannelKeyStore>,
    server_state: Res<ServerState>,
    db: Res<MessageDbRes>,
    profiles: Res<ProfileStore>,
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

    let author = profiles.display_name(&peer_id.to_string());
    let ts = state.hlc.latest().millis;
    let chat_msg = ChatMessage {
        topic,
        author: author.clone(),
        body: body.clone(),
        is_local: true,
        timestamp_ms: ts,
    };

    // Persist to database.
    if let Some(ref db) = db.0 {
        if let Ok(db) = db.lock() {
            db.insert(&crate::storage::StoredMessage {
                topic: chat_msg.topic.clone(),
                author,
                body,
                is_local: true,
                timestamp_ms: state.hlc.latest().millis,
            });
        }
    }

    state.messages.push(chat_msg);
    state.messages_dirty = true;
}

pub(crate) fn handle_network_events(
    mut reader: MessageReader<NetworkBridgeEvent>,
    mut state: ResMut<ChatState>,
    key_store: Res<ChannelKeyStore>,
    db: Res<MessageDbRes>,
    profiles: Res<ProfileStore>,
    server_state: Res<ServerState>,
    mut unread: ResMut<UnreadCounts>,
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
                    let author = profiles.display_name(&signer.to_string());

                    let chat_msg = ChatMessage {
                        topic: topic.clone(),
                        author: author.clone(),
                        body: body.clone(),
                        is_local: false,
                        timestamp_ms: msg.hlc.millis,
                    };

                    if let Some(ref db_arc) = db.0 {
                        if let Ok(db_lock) = db_arc.lock() {
                            db_lock.insert(&crate::storage::StoredMessage {
                                topic: topic.clone(),
                                author,
                                body: body.clone(),
                                is_local: false,
                                timestamp_ms: msg.hlc.millis,
                            });
                        }
                    }

                    // Track unread if this isn't the active channel.
                    let current_topic = server_state
                        .topic_for_name(&state.current_channel)
                        .unwrap_or_default();
                    if chat_msg.topic != current_topic {
                        *unread.counts.entry(chat_msg.topic.clone()).or_insert(0) += 1;
                    }

                    state.messages.push(chat_msg);
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
            NetworkBridgeEvent::FileAnnounced {
                filename,
                size,
                from,
                topic,
                ..
            } => {
                let author = profiles.display_name(from);
                let size_kb = size / 1024;
                let body = format!("[shared file: {filename} ({size_kb} KB)]");
                let ts = state.hlc.latest().millis;
                state.messages.push(ChatMessage {
                    topic: topic.clone(),
                    author,
                    body,
                    is_local: false,
                    timestamp_ms: ts,
                });
                state.messages_dirty = true;
            }
            NetworkBridgeEvent::FileDownloaded { filename, .. } => {
                info!("file downloaded: {filename}");
            }
        }
    }
}

fn handle_channel_click(
    interaction_query: Query<(&Interaction, &ChannelButton), Changed<Interaction>>,
    mut state: ResMut<ChatState>,
    server_state: Res<ServerState>,
    mut unread: ResMut<UnreadCounts>,
) {
    for (interaction, button) in &interaction_query {
        if *interaction == Interaction::Pressed && state.current_channel != button.0 {
            state.current_channel = button.0.clone();
            state.messages_dirty = true;

            // Clear unread count for the channel we just switched to.
            if let Some(topic) = server_state.topic_for_name(&button.0) {
                unread.counts.remove(&topic);
            }
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
                TextColor(theme::TEXT_MUTED),
            ));
        });
        return;
    }

    commands.entity(list_entity).with_children(|parent| {
        for msg in &visible {
            let author_color = if msg.is_local {
                theme::AUTHOR_LOCAL
            } else {
                theme::AUTHOR_REMOTE
            };

            // Format timestamp as HH:MM.
            let time_str = format_timestamp(msg.timestamp_ms);

            parent
                .spawn(Node {
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|row| {
                    // Timestamp
                    row.spawn((
                        Text::new(format!("{time_str} ")),
                        TextFont::from_font_size(11.0),
                        TextColor(theme::TEXT_PLACEHOLDER),
                    ));

                    // Author + body
                    row.spawn((
                        Text::new(format!("{}: ", msg.author)),
                        TextFont::from_font_size(14.0),
                        TextColor(author_color),
                    ))
                    .with_child((
                        TextSpan::new(&msg.body),
                        TextFont::from_font_size(14.0),
                        TextColor(theme::TEXT_PRIMARY),
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
            *color = TextColor(theme::TEXT_PLACEHOLDER);
        } else {
            **text = input.text.clone();
            *color = TextColor(theme::TEXT_PRIMARY);
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
    unread: Res<UnreadCounts>,
    server_state: Res<ServerState>,
    query: Query<(&ChannelButton, &Children)>,
    mut text_query: Query<(&mut Text, &mut TextColor)>,
) {
    if !state.is_changed() && !unread.is_changed() {
        return;
    }
    for (button, children) in &query {
        let is_active = button.0 == state.current_channel;
        let topic = server_state.topic_for_name(&button.0).unwrap_or_default();
        let count = unread.counts.get(&topic).copied().unwrap_or(0);

        for child in children.iter() {
            if let Ok((mut text, mut color)) = text_query.get_mut(child) {
                // Show unread count badge.
                if count > 0 && !is_active {
                    **text = format!("# {} ({})", button.0, count);
                } else {
                    **text = format!("# {}", button.0);
                }

                *color = if is_active {
                    TextColor(theme::TEXT_PRIMARY)
                } else if count > 0 {
                    TextColor(theme::UNREAD_HIGHLIGHT) // yellow for unread
                } else {
                    TextColor(theme::TEXT_MUTED)
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
    mut profiles: ResMut<ProfileStore>,
    identity: Res<LocalIdentity>,
    mut user_display_query: Query<&mut Text, With<LocalUserDisplay>>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            // Save profile.
            let name = settings_input.display_name.trim().to_string();
            crate::storage::save_profile(&crate::storage::LocalProfile {
                display_name: name.clone(),
            });

            // Update profile store and sidebar display.
            let peer_id_str = identity.0.peer_id().to_string();
            let display = if name.is_empty() {
                truncate_peer_id(&peer_id_str)
            } else {
                name
            };
            profiles.names.insert(peer_id_str, display.clone());

            for mut text in &mut user_display_query {
                **text = format!("You: {display}");
            }

            // Save relay and reconnect.
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
                *color = TextColor(theme::TEXT_PLACEHOLDER);
            } else {
                **text = settings_input.relay_addr.clone();
                *color = TextColor(theme::TEXT_PRIMARY);
            }
        }
    }
}

// ───── File Sharing ─────────────────────────────────────────────────────────

/// Open a file dialog when the "Share" button is clicked.
fn handle_share_file_button(
    query: Query<&Interaction, (Changed<Interaction>, With<ShareFileButton>)>,
    picker: Res<FilePicker>,
) {
    for interaction in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let rx_arc = picker.rx.clone();

        // Spawn the file dialog on a background thread (it's blocking).
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            if let Ok(mut guard) = rx_arc.lock() {
                *guard = Some(rx);
            }
            std::thread::spawn(move || {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    if let Ok(data) = std::fs::read(&path) {
                        let filename = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        let mime = mime_from_extension(&filename);
                        let _ = tx.send((filename, mime, data));
                    }
                }
            });
        }

        // WASM: file picker not yet implemented (would need web_sys input element)
        #[cfg(target_arch = "wasm32")]
        {
            let _ = rx_arc;
            info!("file picker not yet available on WASM");
        }
    }
}

/// Poll for file picker results and send the ShareFile command.
#[allow(clippy::too_many_arguments)]
fn poll_file_picker(
    picker: Res<FilePicker>,
    net_cmd: Res<NetworkCommandSender>,
    server_state: Res<ServerState>,
    mut chat_state: ResMut<ChatState>,
    profiles: Res<ProfileStore>,
    identity: Res<LocalIdentity>,
    db: Res<MessageDbRes>,
) {
    let Ok(mut guard) = picker.rx.lock() else {
        return;
    };
    let Some(rx) = guard.as_ref() else {
        return;
    };

    let Ok((filename, mime_type, data)) = rx.try_recv() else {
        return;
    };

    // Drop the receiver since we got the file.
    *guard = None;

    let channel_name = chat_state.current_channel.clone();
    let topic = server_state
        .topic_for_name(&channel_name)
        .unwrap_or(channel_name);

    let size_kb = data.len() / 1024;

    let _ = net_cmd
        .0
        .send(crate::network_bridge::NetworkBridgeCommand::ShareFile {
            topic: topic.clone(),
            filename: filename.clone(),
            mime_type,
            data,
        });

    // Show in local chat.
    let author = profiles.display_name(&identity.0.peer_id().to_string());
    let body = format!("[shared file: {filename} ({size_kb} KB)]");
    let chat_msg = ChatMessage {
        topic,
        author: author.clone(),
        body: body.clone(),
        is_local: true,
        timestamp_ms: chat_state.hlc.latest().millis,
    };

    if let Some(ref db_arc) = db.0 {
        if let Ok(db_lock) = db_arc.lock() {
            db_lock.insert(&crate::storage::StoredMessage {
                topic: chat_msg.topic.clone(),
                author,
                body,
                is_local: true,
                timestamp_ms: chat_state.hlc.latest().millis,
            });
        }
    }

    chat_state.messages.push(chat_msg);
    chat_state.messages_dirty = true;
}

/// Guess MIME type from file extension.
#[cfg(not(target_arch = "wasm32"))]
fn mime_from_extension(filename: &str) -> String {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "txt" => "text/plain",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "rs" => "text/x-rust",
        _ => "application/octet-stream",
    }
    .to_string()
}
