//! Channel management systems — create, delete, and sidebar updates.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use crate::network_bridge::{LocalIdentity, NetworkCommandSender};
use crate::theme;
#[allow(unused_imports)]
use willow_channel::ChannelKind;

use super::components::*;
use super::layout::{make_topic, spawn_channel_button};
use super::resources::*;

/// Handle the "+" button click to start creating a new channel.
pub fn handle_create_channel_button(
    query: Query<&Interaction, (Changed<Interaction>, With<CreateChannelButton>)>,
    mut mgmt: ResMut<ChannelManagement>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            mgmt.creating_channel = true;
            mgmt.new_channel_name.clear();
        }
    }
}

/// Handle keyboard input when creating a new channel.
///
/// Enter confirms, Escape cancels. This runs before handle_keyboard_input
/// so it consumes events when active.
#[allow(clippy::too_many_arguments)]
pub fn handle_new_channel_input(
    mut key_events: MessageReader<KeyboardInput>,
    mut mgmt: ResMut<ChannelManagement>,
    mut server_state: ResMut<ServerState>,
    mut key_store: ResMut<ChannelKeyStore>,
    mut state: ResMut<ChatState>,
    net_cmd: Res<NetworkCommandSender>,
    mut commands: Commands,
    list_query: Query<Entity, With<ChannelList>>,
    identity: Res<LocalIdentity>,
    mut op_log: ResMut<OpLog>,
) {
    if !mgmt.creating_channel {
        return;
    }

    for event in key_events.read() {
        if !event.state.is_pressed() {
            continue;
        }

        match event.key_code {
            KeyCode::Enter => {
                let name = mgmt.new_channel_name.trim().to_string();
                if !name.is_empty() {
                    create_channel(
                        &name,
                        &mut server_state,
                        &mut key_store,
                        &mut state,
                        &net_cmd,
                        &mut commands,
                        &list_query,
                        &identity,
                        &mut op_log,
                    );
                }
                mgmt.creating_channel = false;
                mgmt.new_channel_name.clear();
            }
            KeyCode::Escape => {
                mgmt.creating_channel = false;
                mgmt.new_channel_name.clear();
            }
            KeyCode::Backspace => {
                mgmt.new_channel_name.pop();
            }
            _ => {
                if let Some(ref s) = event.text {
                    for c in s.chars() {
                        if !c.is_control() {
                            mgmt.new_channel_name.push(c);
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn create_channel(
    name: &str,
    server_state: &mut ResMut<ServerState>,
    key_store: &mut ResMut<ChannelKeyStore>,
    state: &mut ResMut<ChatState>,
    net_cmd: &Res<NetworkCommandSender>,
    commands: &mut Commands,
    list_query: &Query<Entity, With<ChannelList>>,
    identity: &Res<LocalIdentity>,
    op_log: &mut ResMut<OpLog>,
) {
    // Create channel on the server.
    let (topic, ch_id) = {
        let Some(server) = &mut server_state.server else {
            return;
        };
        let Ok(ch_id) = server.create_channel(name, ChannelKind::Text) else {
            warn!("failed to create channel '{name}' (duplicate name?)");
            return;
        };
        let topic = make_topic(server, name);

        if let Some(key) = server.channel_key(&ch_id) {
            key_store.keys.insert(topic.clone(), key.clone());
        }

        // Persist while we have the server borrowed.
        crate::storage::save_server(server, &key_store.keys);

        (topic, ch_id)
    };

    // Update topic map (server borrow is dropped now).
    let ch_id_str = ch_id.to_string();
    server_state
        .topic_map
        .insert(topic.clone(), (name.to_string(), ch_id));

    // Subscribe to the new topic.
    let _ = net_cmd
        .0
        .send(crate::network_bridge::NetworkBridgeCommand::Subscribe(
            topic,
        ));

    // Broadcast to peers.
    broadcast_op(
        crate::server_sync::Op::CreateChannel {
            name: name.to_string(),
            channel_id: ch_id_str,
            kind: "text".to_string(),
        },
        state,
        identity,
        op_log,
        net_cmd,
    );

    // Switch to the new channel.
    state.current_channel = name.to_string();
    state.messages_dirty = true;

    // Rebuild channel list in sidebar.
    rebuild_channel_list(commands, list_query, &server_state.channel_names());

    info!("created channel #{name}");
}

/// Rebuild the channel button list in the sidebar.
pub fn rebuild_channel_list(
    commands: &mut Commands,
    list_query: &Query<Entity, With<ChannelList>>,
    channel_names: &[String],
) {
    let Ok(list_entity) = list_query.single() else {
        return;
    };

    commands.entity(list_entity).detach_all_children();
    commands.entity(list_entity).with_children(|list| {
        for name in channel_names {
            spawn_channel_button(list, name);
        }
    });
}

/// Sync the new channel input display in the channel list.
pub fn sync_new_channel_input(
    mgmt: Res<ChannelManagement>,
    mut commands: Commands,
    existing_input: Query<Entity, With<NewChannelInput>>,
    list_query: Query<Entity, With<ChannelList>>,
) {
    if !mgmt.is_changed() {
        return;
    }

    // Remove existing input if any.
    for entity in &existing_input {
        commands.entity(entity).despawn();
    }

    if !mgmt.creating_channel {
        return;
    }

    let Ok(list_entity) = list_query.single() else {
        return;
    };

    // Add input field at the end of the channel list.
    let display = if mgmt.new_channel_name.is_empty() {
        "channel-name".to_string()
    } else {
        mgmt.new_channel_name.clone()
    };
    let color = if mgmt.new_channel_name.is_empty() {
        theme::TEXT_PLACEHOLDER
    } else {
        theme::TEXT_PRIMARY
    };

    commands.entity(list_entity).with_children(|list| {
        list.spawn((
            Node {
                margin: UiRect::top(Val::Px(4.0)),
                padding: UiRect::new(Val::Px(8.0), Val::Px(8.0), Val::Px(4.0), Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(theme::INPUT_FIELD_BG),
            NewChannelInput,
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(format!("# {display}")),
                TextFont::from_font_size(15.0),
                TextColor(color),
            ));
        });
    });
}

// ───── Channel Deletion ─────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
/// Handle delete channel button clicks.
pub fn handle_delete_channel(
    query: Query<(&Interaction, &DeleteChannelButton), Changed<Interaction>>,
    mut server_state: ResMut<ServerState>,
    mut key_store: ResMut<ChannelKeyStore>,
    mut state: ResMut<ChatState>,
    mut commands: Commands,
    list_query: Query<Entity, With<ChannelList>>,
    net_cmd: Res<NetworkCommandSender>,
    identity: Res<LocalIdentity>,
    mut op_log: ResMut<OpLog>,
) {
    for (interaction, button) in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let name = &button.0;

        // Find the channel ID and topic for this name.
        let Some((topic, (_ch_name, ch_id))) = server_state
            .topic_map
            .iter()
            .find(|(_, (n, _))| n == name)
            .map(|(t, v)| (t.clone(), v.clone()))
        else {
            continue;
        };

        // Delete from the server.
        {
            let Some(server) = &mut server_state.server else {
                continue;
            };
            if let Err(e) = server.delete_channel(&ch_id) {
                warn!("failed to delete channel '{name}': {e}");
                continue;
            }
            crate::storage::save_server(server, &key_store.keys);
        }

        // Remove from topic map and key store.
        server_state.topic_map.remove(&topic);
        key_store.keys.remove(&topic);

        // If we deleted the current channel, switch to another.
        if state.current_channel == *name {
            let names = server_state.channel_names();
            state.current_channel = names.first().cloned().unwrap_or_default();
            state.messages_dirty = true;
        }

        // Rebuild sidebar.
        rebuild_channel_list(&mut commands, &list_query, &server_state.channel_names());

        // Broadcast to peers.
        broadcast_op(
            crate::server_sync::Op::DeleteChannel { name: name.clone() },
            &mut state,
            &identity,
            &mut op_log,
            &net_cmd,
        );

        info!("deleted channel #{name}");
    }
}

// ───── Invite Systems ───────────────────────────────────────────────────────

/// Handle "Generate Invite" button — creates a secure invite encrypted
/// for the recipient PeerId entered in the settings.
pub fn handle_generate_invite(
    query: Query<&Interaction, (Changed<Interaction>, With<GenerateInviteButton>)>,
    mut mgmt: ResMut<ChannelManagement>,
    server_state: Res<ServerState>,
    key_store: Res<ChannelKeyStore>,
    mut invite_display: Query<&mut Text, With<InviteCodeDisplay>>,
) {
    for interaction in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let recipient_str = mgmt.invite_recipient.trim();
        if recipient_str.is_empty() {
            mgmt.invite_code = Some("[enter recipient PeerId above]".into());
            for mut text in &mut invite_display {
                **text = "[enter recipient PeerId above]".to_string();
            }
            continue;
        }

        let Some(recipient_pub) = crate::invite::peer_id_to_ed25519_public(recipient_str) else {
            mgmt.invite_code = Some("[invalid PeerId]".into());
            for mut text in &mut invite_display {
                **text = "[invalid PeerId]".to_string();
            }
            continue;
        };

        let Some(server) = &server_state.server else {
            continue;
        };

        match crate::invite::generate_invite(
            server,
            &key_store.keys,
            &server_state.topic_map,
            &recipient_pub,
        ) {
            Some(code) => {
                info!(
                    "generated secure invite for {}",
                    &recipient_str[..12.min(recipient_str.len())]
                );
                mgmt.invite_code = Some(code.clone());
                for mut text in &mut invite_display {
                    let preview = if code.len() > 40 {
                        format!("{}... ({}B)", &code[..40], code.len())
                    } else {
                        code.clone()
                    };
                    **text = preview;
                }
            }
            None => {
                mgmt.invite_code = Some("[encryption failed]".into());
                for mut text in &mut invite_display {
                    **text = "[encryption failed]".to_string();
                }
            }
        }
    }
}

/// Handle "Join Server" button — decrypts an invite code and joins.
#[allow(clippy::too_many_arguments)]
pub fn handle_join_server(
    query: Query<&Interaction, (Changed<Interaction>, With<JoinServerButton>)>,
    mut mgmt: ResMut<ChannelManagement>,
    identity: Res<LocalIdentity>,
    mut key_store: ResMut<ChannelKeyStore>,
    mut server_state: ResMut<ServerState>,
    mut state: ResMut<ChatState>,
    net_cmd: Res<NetworkCommandSender>,
    mut commands: Commands,
    list_query: Query<Entity, With<ChannelList>>,
) {
    for interaction in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let code = mgmt.join_code.trim().to_string();
        if code.is_empty() {
            continue;
        }

        let Some(accepted) = crate::invite::accept_invite(&code, &identity.0) else {
            warn!("failed to accept invite — invalid code or not intended for us");
            mgmt.join_code.clear();
            continue;
        };

        info!(
            "accepted invite for server '{}' with {} channels",
            accepted.server_name,
            accepted.channel_keys.len()
        );

        // Subscribe to channels and store keys.
        for (topic, (name, key)) in &accepted.channel_keys {
            key_store.keys.insert(topic.clone(), key.clone());

            if !server_state.topic_map.contains_key(topic) {
                server_state.topic_map.insert(
                    topic.clone(),
                    (name.clone(), willow_channel::ChannelId::new()),
                );
            }

            let _ = net_cmd
                .0
                .send(crate::network_bridge::NetworkBridgeCommand::Subscribe(
                    topic.clone(),
                ));
        }

        // Switch to the first new channel.
        if let Some((_, (name, _))) = accepted.channel_keys.iter().next() {
            state.current_channel = name.clone();
            state.messages_dirty = true;
        }

        // Rebuild sidebar.
        let names = server_state.channel_names();
        rebuild_channel_list(&mut commands, &list_query, &names);

        // Persist keys.
        if let Some(server) = &server_state.server {
            crate::storage::save_server(server, &key_store.keys);
        }

        mgmt.join_code.clear();
    }
}

/// Sync the invite-related text fields in settings.
pub fn sync_invite_fields(
    mgmt: Res<ChannelManagement>,
    mut join_query: Query<(&mut Text, &mut TextColor), With<JoinCodeInput>>,
) {
    if !mgmt.is_changed() {
        return;
    }

    for (mut text, mut color) in &mut join_query {
        if mgmt.join_code.is_empty() {
            **text = "Paste invite code...".to_string();
            *color = TextColor(theme::TEXT_PLACEHOLDER);
        } else {
            **text = mgmt.join_code.clone();
            *color = TextColor(theme::TEXT_PRIMARY);
        }
    }
}

// ───── Member List ──────────────────────────────────────────────────────────

/// Rebuild the member list in settings when peers change.
#[allow(clippy::too_many_arguments)]
pub fn sync_member_list(
    mut commands: Commands,
    state: Res<ChatState>,
    profiles: Res<ProfileStore>,
    server_state: Res<ServerState>,
    list_query: Query<Entity, With<MemberList>>,
    identity: Res<LocalIdentity>,
    op_log: Res<OpLog>,
) {
    if !state.is_changed() && !server_state.is_changed() && !op_log.is_changed() {
        return;
    }

    let Ok(list_entity) = list_query.single() else {
        return;
    };

    commands.entity(list_entity).detach_all_children();

    let local_peer = identity.0.peer_id().to_string();
    let local_name = profiles.display_name(&local_peer);
    let owner = server_state
        .server
        .as_ref()
        .map(|s| s.owner.to_string())
        .unwrap_or_default();

    commands.entity(list_entity).with_children(|list| {
        // Show the local user first.
        spawn_member_row(list, &local_name, &local_peer, true, true, true);

        // Show connected peers.
        for peer_id in &state.peers {
            let name = profiles.display_name(peer_id);
            let is_owner = *peer_id == owner;
            let is_trusted = op_log.is_trusted(peer_id, &owner);
            spawn_member_row(list, &name, peer_id, false, is_owner, is_trusted);
        }
    });
}

fn spawn_member_row(
    parent: &mut ChildSpawnerCommands,
    name: &str,
    peer_id: &str,
    is_self: bool,
    is_owner: bool,
    is_trusted: bool,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            margin: UiRect::bottom(Val::Px(4.0)),
            padding: UiRect::horizontal(Val::Px(4.0)),
            ..default()
        })
        .with_children(|row| {
            // Online indicator dot
            row.spawn((
                Node {
                    width: Val::Px(8.0),
                    height: Val::Px(8.0),
                    margin: UiRect::right(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(theme::STATUS_ONLINE),
            ));

            // Name + trust badge
            let label = if is_self {
                format!("{name} (you)")
            } else if is_owner {
                format!("{name} [owner]")
            } else if is_trusted {
                format!("{name} [trusted]")
            } else {
                name.to_string()
            };
            row.spawn((
                Text::new(label),
                TextFont::from_font_size(13.0),
                TextColor(theme::TEXT_PRIMARY),
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
            ));

            // Trust/Untrust button (not for self or owner)
            if !is_self && !is_owner {
                let (trust_label, trust_color) = if is_trusted {
                    ("Untrust", theme::TEXT_MUTED)
                } else {
                    ("Trust", theme::ACCENT)
                };
                row.spawn((
                    Button,
                    Node {
                        padding: UiRect::horizontal(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    TrustMemberButton(peer_id.to_string()),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new(trust_label),
                        TextFont::from_font_size(11.0),
                        TextColor(trust_color),
                    ));
                });
            }

            // Kick button (only for others, not self)
            if !is_self {
                row.spawn((
                    Button,
                    Node {
                        padding: UiRect::horizontal(Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    KickMemberButton(peer_id.to_string()),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("Kick"),
                        TextFont::from_font_size(11.0),
                        TextColor(theme::DANGER),
                    ));
                });
            }
        });
}

#[allow(clippy::too_many_arguments)]
/// Handle kick button clicks.
pub fn handle_kick_member(
    query: Query<(&Interaction, &KickMemberButton), Changed<Interaction>>,
    mut server_state: ResMut<ServerState>,
    mut key_store: ResMut<ChannelKeyStore>,
    mut state: ResMut<ChatState>,
    net_cmd: Res<NetworkCommandSender>,
    identity: Res<LocalIdentity>,
    mut op_log: ResMut<OpLog>,
) {
    for (interaction, button) in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let kicked_peer = &button.0;

        // Find the PeerId in the server and remove.
        let rotated = {
            let Some(server) = &mut server_state.server else {
                continue;
            };

            // We need the PeerId to call remove_member. Try to find it.
            let member_peer = server
                .members()
                .iter()
                .find(|m| m.peer_id.to_string() == *kicked_peer)
                .map(|m| m.peer_id.clone());

            let Some(peer_id) = member_peer else {
                warn!("peer {} not found in server members", kicked_peer);
                continue;
            };

            match server.remove_member(&peer_id) {
                Ok(new_keys) => {
                    crate::storage::save_server(server, &key_store.keys);
                    Some(new_keys)
                }
                Err(e) => {
                    warn!("failed to kick {}: {e}", kicked_peer);
                    None
                }
            }
        };

        if let Some(new_keys) = rotated {
            // Update key store with rotated keys.
            for (ch_id, key) in &new_keys {
                // Find the topic for this channel ID.
                for (topic, (_, tid)) in &server_state.topic_map {
                    if tid == ch_id {
                        key_store.keys.insert(topic.clone(), key.clone());
                        break;
                    }
                }
            }

            // Remove from peers list.
            state.peers.retain(|p| p != kicked_peer);

            // Add system message.
            let ts = state.hlc.latest().millis;
            state.messages.push(ChatMessage::new(
                String::new(),
                "System".into(),
                format!("Kicked {kicked_peer} — channel keys rotated"),
                false,
                ts,
            ));
            state.messages_dirty = true;

            // Encrypt rotated keys for remaining members.
            let mut rotated_key_entries = Vec::new();
            if let Some(server) = &server_state.server {
                for member in server.members() {
                    let peer_str = member.peer_id.to_string();
                    if let Some(pub_key) = crate::invite::peer_id_to_ed25519_public(&peer_str) {
                        for (ch_id, key) in &new_keys {
                            for (topic, (_, tid)) in &server_state.topic_map {
                                if tid == ch_id {
                                    if let Ok(enc) =
                                        willow_crypto::encrypt_channel_key_for(key, &pub_key)
                                    {
                                        rotated_key_entries.push((
                                            peer_str.clone(),
                                            topic.clone(),
                                            enc,
                                        ));
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // Broadcast to peers.
            broadcast_op(
                crate::server_sync::Op::KickMember {
                    peer_id: kicked_peer.clone(),
                    rotated_keys: rotated_key_entries,
                },
                &mut state,
                &identity,
                &mut op_log,
                &net_cmd,
            );

            info!("kicked peer {kicked_peer}, keys rotated");
        }
    }
}

// ───── Role Management ──────────────────────────────────────────────────────

/// Handle "Create Role" button click.
pub fn handle_create_role_button(
    query: Query<&Interaction, (Changed<Interaction>, With<CreateRoleButton>)>,
    mut mgmt: ResMut<ChannelManagement>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            mgmt.creating_role = !mgmt.creating_role;
            mgmt.new_role_name.clear();
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Handle keyboard input for new role creation.
pub fn handle_new_role_input(
    mut key_events: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut mgmt: ResMut<ChannelManagement>,
    mut server_state: ResMut<ServerState>,
    key_store: Res<ChannelKeyStore>,
    net_cmd: Res<NetworkCommandSender>,
    identity: Res<LocalIdentity>,
    mut state: ResMut<ChatState>,
    mut op_log: ResMut<OpLog>,
) {
    if !mgmt.creating_role {
        return;
    }

    for event in key_events.read() {
        if !event.state.is_pressed() {
            continue;
        }

        match event.key_code {
            KeyCode::Enter => {
                let name = mgmt.new_role_name.trim().to_string();
                if !name.is_empty() {
                    if let Some(server) = &mut server_state.server {
                        let role_id = willow_channel::RoleId::new();
                        let role = willow_channel::Role::with_id(role_id.clone(), &name);
                        server.create_role(role);
                        crate::storage::save_server(server, &key_store.keys);
                        broadcast_op(
                            crate::server_sync::Op::CreateRole {
                                name: name.clone(),
                                role_id: role_id.to_string(),
                            },
                            &mut state,
                            &identity,
                            &mut op_log,
                            &net_cmd,
                        );
                        info!("created role '{name}'");
                    }
                }
                mgmt.creating_role = false;
                mgmt.new_role_name.clear();
            }
            KeyCode::Escape => {
                mgmt.creating_role = false;
                mgmt.new_role_name.clear();
            }
            KeyCode::Backspace => {
                mgmt.new_role_name.pop();
            }
            _ => {
                if let Some(ref s) = event.text {
                    for c in s.chars() {
                        if !c.is_control() {
                            mgmt.new_role_name.push(c);
                        }
                    }
                }
            }
        }
    }
}

/// Rebuild the role list display in settings.
pub fn sync_role_list(
    mut commands: Commands,
    server_state: Res<ServerState>,
    mgmt: Res<ChannelManagement>,
    list_query: Query<Entity, With<RoleList>>,
) {
    if !server_state.is_changed() && !mgmt.is_changed() {
        return;
    }

    let Ok(list_entity) = list_query.single() else {
        return;
    };

    commands.entity(list_entity).detach_all_children();

    let Some(server) = &server_state.server else {
        return;
    };

    commands.entity(list_entity).with_children(|list| {
        let roles = server.roles();
        if roles.is_empty() {
            list.spawn((
                Text::new("No roles defined"),
                TextFont::from_font_size(11.0),
                TextColor(theme::TEXT_PLACEHOLDER),
            ));
        }

        let key_perms = [
            ("Admin", willow_channel::Permission::Administrator),
            ("Send", willow_channel::Permission::SendMessages),
            ("Read", willow_channel::Permission::ReadMessages),
            ("Kick", willow_channel::Permission::KickMembers),
            ("Invite", willow_channel::Permission::CreateInvite),
            ("Files", willow_channel::Permission::AttachFiles),
            ("ManageCh", willow_channel::Permission::ManageChannels),
        ];

        for role in &roles {
            let role_id_str = role.id.to_string();

            list.spawn(Node {
                margin: UiRect::bottom(Val::Px(8.0)),
                flex_direction: FlexDirection::Column,
                ..default()
            })
            .with_children(|col| {
                // Role name + delete button row
                col.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    margin: UiRect::bottom(Val::Px(2.0)),
                    ..default()
                })
                .with_children(|name_row| {
                    name_row.spawn((
                        Text::new(&role.name),
                        TextFont::from_font_size(13.0),
                        TextColor(theme::TEXT_PRIMARY),
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                    ));
                    name_row
                        .spawn((
                            Button,
                            Node {
                                padding: UiRect::horizontal(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                            DeleteRoleButton(role_id_str.clone()),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new("x"),
                                TextFont::from_font_size(14.0),
                                TextColor(theme::DANGER),
                            ));
                        });
                });

                // Permission toggle badges
                col.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                })
                .with_children(|perm_row| {
                    for (label, perm) in &key_perms {
                        let active = role.permissions.contains(perm);
                        let bg = if active {
                            theme::ACCENT
                        } else {
                            theme::INPUT_FIELD_BG
                        };
                        let text_color = if active {
                            theme::TEXT_PRIMARY
                        } else {
                            theme::TEXT_MUTED
                        };

                        perm_row
                            .spawn((
                                Button,
                                Node {
                                    padding: UiRect::new(
                                        Val::Px(6.0),
                                        Val::Px(6.0),
                                        Val::Px(2.0),
                                        Val::Px(2.0),
                                    ),
                                    margin: UiRect::new(
                                        Val::Px(0.0),
                                        Val::Px(4.0),
                                        Val::Px(0.0),
                                        Val::Px(4.0),
                                    ),
                                    ..default()
                                },
                                BackgroundColor(bg),
                                TogglePermButton(role_id_str.clone(), format!("{perm:?}")),
                            ))
                            .with_children(|btn| {
                                btn.spawn((
                                    Text::new(*label),
                                    TextFont::from_font_size(10.0),
                                    TextColor(text_color),
                                ));
                            });
                    }
                });
            });
        }

        // Show new role input if creating.
        if mgmt.creating_role {
            let display = if mgmt.new_role_name.is_empty() {
                "Role name...".to_string()
            } else {
                mgmt.new_role_name.clone()
            };
            let color = if mgmt.new_role_name.is_empty() {
                theme::TEXT_PLACEHOLDER
            } else {
                theme::TEXT_PRIMARY
            };

            list.spawn((
                Node {
                    padding: UiRect::all(Val::Px(4.0)),
                    margin: UiRect::top(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(theme::INPUT_FIELD_BG),
            ))
            .with_children(|field| {
                field.spawn((
                    Text::new(display),
                    TextFont::from_font_size(13.0),
                    TextColor(color),
                    RoleNameInput,
                ));
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
/// Handle permission toggle button clicks.
pub fn handle_toggle_permission(
    query: Query<(&Interaction, &TogglePermButton), Changed<Interaction>>,
    mut server_state: ResMut<ServerState>,
    key_store: Res<ChannelKeyStore>,
    net_cmd: Res<NetworkCommandSender>,
    identity: Res<LocalIdentity>,
    mut state: ResMut<ChatState>,
    mut op_log: ResMut<OpLog>,
) {
    for (interaction, button) in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let Some(server) = &mut server_state.server else {
            continue;
        };

        let role_id = willow_channel::RoleId(uuid::Uuid::parse_str(&button.0).unwrap_or_default());
        let perm = match button.1.as_str() {
            "Administrator" => willow_channel::Permission::Administrator,
            "SendMessages" => willow_channel::Permission::SendMessages,
            "ReadMessages" => willow_channel::Permission::ReadMessages,
            "KickMembers" => willow_channel::Permission::KickMembers,
            "CreateInvite" => willow_channel::Permission::CreateInvite,
            "AttachFiles" => willow_channel::Permission::AttachFiles,
            "ManageChannels" => willow_channel::Permission::ManageChannels,
            _ => continue,
        };

        let granted = !server
            .role(&role_id)
            .map(|r| r.permissions.contains(&perm))
            .unwrap_or(false);

        if let Err(e) = server.set_permission(&role_id, perm, granted) {
            warn!("failed to set permission: {e}");
        } else {
            crate::storage::save_server(server, &key_store.keys);
            broadcast_op(
                crate::server_sync::Op::SetPermission {
                    role_id: button.0.clone(),
                    permission: button.1.clone(),
                    granted,
                },
                &mut state,
                &identity,
                &mut op_log,
                &net_cmd,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Handle delete role button clicks.
pub fn handle_delete_role(
    query: Query<(&Interaction, &DeleteRoleButton), Changed<Interaction>>,
    mut server_state: ResMut<ServerState>,
    key_store: Res<ChannelKeyStore>,
    net_cmd: Res<NetworkCommandSender>,
    identity: Res<LocalIdentity>,
    mut state: ResMut<ChatState>,
    mut op_log: ResMut<OpLog>,
) {
    for (interaction, button) in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let Some(server) = &mut server_state.server else {
            continue;
        };

        let role_id = willow_channel::RoleId(uuid::Uuid::parse_str(&button.0).unwrap_or_default());
        if let Err(e) = server.delete_role(&role_id) {
            warn!("failed to delete role: {e}");
        } else {
            crate::storage::save_server(server, &key_store.keys);
            broadcast_op(
                crate::server_sync::Op::DeleteRole {
                    role_id: button.0.clone(),
                },
                &mut state,
                &identity,
                &mut op_log,
                &net_cmd,
            );
            info!("deleted role");
        }
    }
}

#[allow(clippy::too_many_arguments)]
/// Handle role assignment button clicks — assigns the role to the member.
pub fn handle_assign_role(
    query: Query<(&Interaction, &AssignRoleButton), Changed<Interaction>>,
    mut server_state: ResMut<ServerState>,
    key_store: Res<ChannelKeyStore>,
    net_cmd: Res<NetworkCommandSender>,
    identity: Res<LocalIdentity>,
    mut state: ResMut<ChatState>,
    mut op_log: ResMut<OpLog>,
) {
    for (interaction, button) in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let Some(server) = &mut server_state.server else {
            continue;
        };

        let peer_id_str = &button.0;
        let role_id = willow_channel::RoleId(uuid::Uuid::parse_str(&button.1).unwrap_or_default());

        let member_peer = server
            .members()
            .iter()
            .find(|m| m.peer_id.to_string() == *peer_id_str)
            .map(|m| m.peer_id.clone());

        let Some(peer) = member_peer else {
            continue;
        };

        if let Err(e) = server.assign_role(&peer, &role_id) {
            warn!("failed to assign role: {e}");
        } else {
            crate::storage::save_server(server, &key_store.keys);
            broadcast_op(
                crate::server_sync::Op::AssignRole {
                    peer_id: peer_id_str.clone(),
                    role_id: button.1.clone(),
                },
                &mut state,
                &identity,
                &mut op_log,
                &net_cmd,
            );
            info!("assigned role to {peer_id_str}");
        }
    }
}

// ───── Clipboard Systems ────────────────────────────────────────────────────

/// Copy the local PeerId to clipboard when the "ID" button is clicked.
pub fn handle_copy_peer_id(
    query: Query<&Interaction, (Changed<Interaction>, With<CopyPeerIdButton>)>,
    identity: Res<LocalIdentity>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            let peer_id = identity.0.peer_id().to_string();
            crate::clipboard::copy_to_clipboard(&peer_id);
            info!("copied PeerId to clipboard");
        }
    }
}

/// Copy the generated invite code to clipboard.
pub fn handle_copy_invite(
    query: Query<&Interaction, (Changed<Interaction>, With<CopyInviteButton>)>,
    mgmt: Res<ChannelManagement>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            if let Some(ref code) = mgmt.invite_code {
                crate::clipboard::copy_to_clipboard(code);
                info!("copied invite code to clipboard");
            }
        }
    }
}

/// Handle trust/untrust button clicks.
pub fn handle_trust_member(
    query: Query<(&Interaction, &TrustMemberButton), Changed<Interaction>>,
    mut state: ResMut<ChatState>,
    identity: Res<LocalIdentity>,
    mut op_log: ResMut<OpLog>,
    net_cmd: Res<NetworkCommandSender>,
) {
    for (interaction, button) in &query {
        if *interaction != Interaction::Pressed {
            continue;
        }

        let peer_id = &button.0;
        let owner = identity.0.peer_id().to_string();

        if op_log.is_trusted(peer_id, &owner) {
            // Untrust
            broadcast_op(
                crate::server_sync::Op::UntrustPeer {
                    peer_id: peer_id.clone(),
                },
                &mut state,
                &identity,
                &mut op_log,
                &net_cmd,
            );
            info!("untrusted peer {peer_id}");
        } else {
            // Trust
            broadcast_op(
                crate::server_sync::Op::TrustPeer {
                    peer_id: peer_id.clone(),
                },
                &mut state,
                &identity,
                &mut op_log,
                &net_cmd,
            );
            info!("trusted peer {peer_id}");
        }
    }
}

/// Helper to stamp, record, persist, and broadcast a server op.
fn broadcast_op(
    op: crate::server_sync::Op,
    state: &mut ChatState,
    identity: &crate::network_bridge::LocalIdentity,
    op_log: &mut OpLog,
    net_cmd: &crate::network_bridge::NetworkCommandSender,
) {
    let stamped =
        crate::server_sync::StampedOp::new(op, &mut state.hlc, &identity.0.peer_id().to_string());
    op_log.record(stamped.clone());
    crate::storage::save_op_log(&op_log.ops);
    let _ = net_cmd
        .0
        .send(crate::network_bridge::NetworkBridgeCommand::BroadcastOp(
            stamped,
        ));
}
