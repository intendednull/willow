//! UI layout — entity spawning for the initial UI hierarchy.

use bevy::prelude::*;

use crate::network_bridge::LocalIdentity;
use crate::theme;

use super::components::*;
use super::constants;
use super::resources::*;

/// Build a gossipsub topic string from a server ID and channel name.
pub fn make_topic(server: &willow_channel::Server, channel_name: &str) -> String {
    format!("{}/{}", server.id, channel_name)
}

pub fn setup_ui(
    mut commands: Commands,
    identity: Res<LocalIdentity>,
    server_state: Res<ServerState>,
    settings_input: Res<SettingsInput>,
    mut profiles: ResMut<ProfileStore>,
) {
    commands.spawn(Camera2d);

    let peer_id_str = identity.0.peer_id().to_string();
    let local_name = if settings_input.display_name.is_empty() {
        super::truncate_peer_id(&peer_id_str)
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

    commands
        .spawn(Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            ..default()
        })
        .with_children(|root| {
            spawn_sidebar(root, server_name, &peer_display, &channel_names);
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

fn spawn_sidebar(
    root: &mut ChildSpawnerCommands,
    server_name: &str,
    peer_display: &str,
    channel_names: &[String],
) {
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

        // Channel section header with "+" button
        sidebar
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                padding: UiRect::new(Val::Px(16.0), Val::Px(8.0), Val::Px(16.0), Val::Px(4.0)),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Text::new("TEXT CHANNELS"),
                    TextFont::from_font_size(11.0),
                    TextColor(theme::TEXT_HEADER),
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                ));
                row.spawn((
                    Button,
                    Node {
                        width: Val::Px(20.0),
                        height: Val::Px(20.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    CreateChannelButton,
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("+"),
                        TextFont::from_font_size(16.0),
                        TextColor(theme::TEXT_HEADER),
                    ));
                });
            });

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
                for name in channel_names {
                    spawn_channel_button(list, name);
                }
            });

        sidebar.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });

        // User area
        spawn_user_area(sidebar, peer_display);
    });
}

fn spawn_user_area(sidebar: &mut ChildSpawnerCommands, peer_display: &str) {
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

            // Name + peer count
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

            // Copy PeerId button
            user_area
                .spawn((
                    Button,
                    Node {
                        width: Val::Px(32.0),
                        height: Val::Px(32.0),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        margin: UiRect::right(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    CopyPeerIdButton,
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("ID"),
                        TextFont::from_font_size(10.0),
                        TextColor(theme::TEXT_MUTED),
                    ));
                });

            // Settings gear
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
}

pub fn spawn_chat_panel(parent: &mut ChildSpawnerCommands, channel_names: &[String]) {
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
            // Channel header
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
                    .unwrap_or(constants::DEFAULT_CHANNEL);
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
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::End,
                    padding: UiRect::new(Val::Px(16.0), Val::Px(16.0), Val::Px(8.0), Val::Px(8.0)),
                    overflow: Overflow::clip_y(),
                    ..default()
                },
                MessageList,
            ));

            // Input area
            chat.spawn(Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(68.0),
                padding: UiRect::new(Val::Px(16.0), Val::Px(16.0), Val::Px(0.0), Val::Px(16.0)),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|input_area| {
                // Share file button
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

                // Text input
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
                            Text::new(constants::CHAT_PLACEHOLDER),
                            TextFont::from_font_size(14.0),
                            TextColor(theme::TEXT_PLACEHOLDER),
                            InputText,
                        ));
                    });
            });
        });
}

pub fn spawn_settings_panel(parent: &mut ChildSpawnerCommands, settings: &SettingsInput) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(24.0)),
                display: Display::None,
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

            spawn_settings_field(
                panel,
                "Display Name",
                &settings.display_name,
                constants::NAME_PLACEHOLDER,
                SettingsNameText,
                SettingsFieldContainer(super::resources::SettingsField::DisplayName),
            );

            panel.spawn(Node {
                height: Val::Px(12.0),
                ..default()
            });

            // Relay section with description
            panel.spawn((
                Text::new("Relay Address"),
                TextFont::from_font_size(13.0),
                TextColor(theme::TEXT_SECONDARY),
            ));
            panel.spawn((
                Text::new(
                    "Connect to a relay server for peer discovery. Leave empty for LAN-only (mDNS).",
                ),
                TextFont::from_font_size(11.0),
                TextColor(theme::TEXT_PLACEHOLDER),
                Node {
                    margin: UiRect::vertical(Val::Px(4.0)),
                    ..default()
                },
            ));
            spawn_settings_input_field(
                panel,
                &settings.relay_addr,
                constants::RELAY_PLACEHOLDER,
                SettingsRelayText,
                SettingsFieldContainer(super::resources::SettingsField::RelayAddr),
            );

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
                Text::new(constants::RELAY_EXAMPLE),
                TextFont::from_font_size(11.0),
                TextColor(theme::TEXT_PLACEHOLDER),
            ));

            // ── Invite Section ──
            panel.spawn(Node {
                height: Val::Px(24.0),
                ..default()
            });

            panel.spawn((
                Text::new("Server Invites"),
                TextFont::from_font_size(13.0),
                TextColor(theme::TEXT_SECONDARY),
            ));

            panel.spawn(Node {
                height: Val::Px(8.0),
                ..default()
            });

            // Recipient PeerId input
            panel.spawn((
                Text::new("Recipient PeerId (ask them to copy from their sidebar)"),
                TextFont::from_font_size(11.0),
                TextColor(theme::TEXT_PLACEHOLDER),
                Node {
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                },
            ));

            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        min_height: Val::Px(36.0),
                        padding: UiRect::horizontal(Val::Px(12.0)),
                        align_items: AlignItems::Center,
                        margin: UiRect::vertical(Val::Px(4.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(theme::INPUT_FIELD_BG),
                    BorderColor::all(Color::NONE),
                    SettingsFieldContainer(super::resources::SettingsField::InviteRecipient),
                ))
                .with_children(|field| {
                    field.spawn((
                        Text::new("12D3KooW..."),
                        TextFont::from_font_size(13.0),
                        TextColor(theme::TEXT_PLACEHOLDER),
                        // Reuse JoinCodeInput as a generic text display; we'll add a dedicated one.
                    ));
                });

            panel.spawn(Node {
                height: Val::Px(4.0),
                ..default()
            });

            // Generate invite button + code display
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Button,
                        Node {
                            padding: UiRect::new(
                                Val::Px(12.0),
                                Val::Px(12.0),
                                Val::Px(6.0),
                                Val::Px(6.0),
                            ),
                            margin: UiRect::right(Val::Px(8.0)),
                            ..default()
                        },
                        BackgroundColor(theme::ACCENT),
                        GenerateInviteButton,
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new("Generate Invite"),
                            TextFont::from_font_size(12.0),
                            TextColor(theme::TEXT_PRIMARY),
                        ));
                    });

                    row.spawn((
                        Text::new(""),
                        TextFont::from_font_size(11.0),
                        TextColor(theme::STATUS_ONLINE),
                        InviteCodeDisplay,
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                    ));

                    row.spawn((
                        Button,
                        Node {
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        CopyInviteButton,
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new("Copy"),
                            TextFont::from_font_size(11.0),
                            TextColor(theme::TEXT_MUTED),
                        ));
                    });
                });

            // Join server section
            panel.spawn(Node {
                height: Val::Px(16.0),
                ..default()
            });

            panel.spawn((
                Text::new("Join a Server"),
                TextFont::from_font_size(13.0),
                TextColor(theme::TEXT_SECONDARY),
            ));

            panel.spawn((
                Text::new("Paste an invite code from another user."),
                TextFont::from_font_size(11.0),
                TextColor(theme::TEXT_PLACEHOLDER),
                Node {
                    margin: UiRect::vertical(Val::Px(4.0)),
                    ..default()
                },
            ));

            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        min_height: Val::Px(36.0),
                        padding: UiRect::horizontal(Val::Px(12.0)),
                        align_items: AlignItems::Center,
                        margin: UiRect::vertical(Val::Px(4.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(theme::INPUT_FIELD_BG),
                    BorderColor::all(Color::NONE),
                    SettingsFieldContainer(super::resources::SettingsField::JoinCode),
                ))
                .with_children(|field| {
                    field.spawn((
                        Text::new("Paste invite code..."),
                        TextFont::from_font_size(13.0),
                        TextColor(theme::TEXT_PLACEHOLDER),
                        JoinCodeInput,
                    ));
                });


            panel.spawn(Node {
                height: Val::Px(8.0),
                ..default()
            });

            panel
                .spawn((
                    Button,
                    Node {
                        padding: UiRect::new(
                            Val::Px(12.0),
                            Val::Px(12.0),
                            Val::Px(6.0),
                            Val::Px(6.0),
                        ),
                        ..default()
                    },
                    BackgroundColor(theme::ACCENT),
                    JoinServerButton,
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("Join Server"),
                        TextFont::from_font_size(12.0),
                        TextColor(theme::TEXT_PRIMARY),
                    ));
                });

            // ── Members Section ──
            panel.spawn(Node {
                height: Val::Px(24.0),
                ..default()
            });

            panel.spawn((
                Text::new("Members"),
                TextFont::from_font_size(13.0),
                TextColor(theme::TEXT_SECONDARY),
            ));

            panel.spawn(Node {
                height: Val::Px(4.0),
                ..default()
            });

            // Member list container — populated dynamically by sync_member_list.
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                MemberList,
            ));

            // ── Roles Section ──
            panel.spawn(Node {
                height: Val::Px(24.0),
                ..default()
            });

            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("Roles"),
                        TextFont::from_font_size(13.0),
                        TextColor(theme::TEXT_SECONDARY),
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                    ));
                    row.spawn((
                        Button,
                        Node {
                            padding: UiRect::horizontal(Val::Px(8.0)),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        CreateRoleButton,
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new("+"),
                            TextFont::from_font_size(16.0),
                            TextColor(theme::TEXT_HEADER),
                        ));
                    });
                });

            panel.spawn(Node {
                height: Val::Px(4.0),
                ..default()
            });

            // Role list — populated dynamically by sync_role_list.
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                RoleList,
            ));
        });
}

/// Spawn a labeled settings text input field.
fn spawn_settings_field(
    panel: &mut ChildSpawnerCommands,
    label: &str,
    value: &str,
    placeholder: &str,
    marker: impl Component,
    container: SettingsFieldContainer,
) {
    panel.spawn((
        Text::new(label),
        TextFont::from_font_size(13.0),
        TextColor(theme::TEXT_SECONDARY),
    ));
    spawn_settings_input_field(panel, value, placeholder, marker, container);
}

fn spawn_settings_input_field(
    panel: &mut ChildSpawnerCommands,
    value: &str,
    placeholder: &str,
    marker: impl Component,
    container: SettingsFieldContainer,
) {
    let (display, color) = if value.is_empty() {
        (placeholder, theme::TEXT_PLACEHOLDER)
    } else {
        (value, theme::TEXT_PRIMARY)
    };

    panel
        .spawn((
            Node {
                width: Val::Percent(100.0),
                min_height: Val::Px(36.0),
                padding: UiRect::horizontal(Val::Px(12.0)),
                align_items: AlignItems::Center,
                margin: UiRect::vertical(Val::Px(4.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(theme::INPUT_FIELD_BG),
            BorderColor::all(Color::NONE),
            container,
        ))
        .with_children(|field| {
            field.spawn((
                Text::new(display),
                TextFont::from_font_size(13.0),
                TextColor(color),
                marker,
            ));
        });
}

pub fn spawn_channel_button(parent: &mut ChildSpawnerCommands, name: &str) {
    parent
        .spawn(Node {
            margin: UiRect::top(Val::Px(2.0)),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            // Channel name button (takes most of the row).
            row.spawn((
                Button,
                Node {
                    flex_grow: 1.0,
                    padding: UiRect::new(Val::Px(8.0), Val::Px(4.0), Val::Px(4.0), Val::Px(4.0)),
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

            // Delete button (small "×" on the right).
            row.spawn((
                Button,
                Node {
                    width: Val::Px(20.0),
                    height: Val::Px(20.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::NONE),
                DeleteChannelButton(name.to_string()),
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("×"),
                    TextFont::from_font_size(14.0),
                    TextColor(theme::TEXT_PLACEHOLDER),
                ));
            });
        });
}
