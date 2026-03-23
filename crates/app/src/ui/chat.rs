//! Chat systems — network events, message rendering, channel navigation.

use bevy::prelude::*;

use crate::network_bridge::NetworkBridgeEvent;
use crate::theme;
use willow_messaging::{Content, Message};
use willow_transport::{unpack_envelope, MessageType};

use super::components::*;
use super::resources::*;

/// Process incoming network events and update the chat state.
#[allow(clippy::too_many_arguments)]
pub fn handle_network_events(
    mut reader: MessageReader<NetworkBridgeEvent>,
    mut state: ResMut<ChatState>,
    mut key_store: ResMut<ChannelKeyStore>,
    db: Res<MessageDbRes>,
    mut profiles: ResMut<ProfileStore>,
    mut server_state: ResMut<ServerState>,
    mut unread: ResMut<UnreadCounts>,
    net_cmd: Res<crate::network_bridge::NetworkCommandSender>,
    mut op_log: ResMut<OpLog>,
    identity: Res<crate::network_bridge::LocalIdentity>,
) {
    for event in reader.read() {
        match event {
            NetworkBridgeEvent::MessageReceived {
                topic,
                data,
                source: _,
            } => {
                let Ok((envelope_data, signer)) = willow_identity::unpack::<Vec<u8>>(data) else {
                    continue;
                };

                let Ok((msg, MessageType::Chat)) = unpack_envelope::<Message>(&envelope_data)
                else {
                    continue;
                };

                let _ = &signer;

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

                // Handle reactions by updating the target message.
                if let Content::Reaction {
                    ref target,
                    ref emoji,
                } = content
                {
                    let author = profiles.display_name(&signer.to_string());
                    let target_str = target.to_string();
                    for m in &mut state.messages {
                        if m.id == target_str {
                            m.reactions
                                .entry(emoji.clone())
                                .or_default()
                                .push(author.clone());
                            state.messages_dirty = true;
                            break;
                        }
                    }
                    continue;
                }

                // Handle edits by updating the target message body.
                if let Content::Edit {
                    ref target,
                    ref new_body,
                } = content
                {
                    let target_str = target.to_string();
                    for m in &mut state.messages {
                        if m.id == target_str {
                            m.body = new_body.clone();
                            m.edited = true;
                            state.messages_dirty = true;
                            break;
                        }
                    }
                    continue;
                }

                // Handle deletes by marking the target message.
                if let Content::Delete { ref target } = content {
                    let target_str = target.to_string();
                    for m in &mut state.messages {
                        if m.id == target_str {
                            m.body = "[message deleted]".to_string();
                            m.deleted = true;
                            m.reactions.clear();
                            state.messages_dirty = true;
                            break;
                        }
                    }
                    continue;
                }

                // Handle replies — create a message with reply_preview.
                if let Content::Reply {
                    ref parent,
                    ref body,
                } = content
                {
                    let author = profiles.display_name(&signer.to_string());
                    let parent_str = parent.to_string();

                    // Build reply preview from the parent message.
                    let preview = state.messages.iter().find(|m| m.id == parent_str).map(|m| {
                        let text = if m.body.len() > 50 {
                            format!("{}...", &m.body[..50])
                        } else {
                            m.body.clone()
                        };
                        format!("{}: {text}", m.author)
                    });

                    let mut chat_msg = ChatMessage::new(
                        topic.clone(),
                        author.clone(),
                        body.clone(),
                        false,
                        msg.hlc.millis,
                    );
                    chat_msg.id = msg.id.to_string();
                    chat_msg.reply_preview = preview;

                    state.messages.push(chat_msg);
                    state.messages_dirty = true;
                    state.hlc.receive(msg.hlc);
                    continue;
                }

                if let Content::Text { ref body } = content {
                    let author = profiles.display_name(&signer.to_string());

                    let mut chat_msg = ChatMessage::new(
                        topic.clone(),
                        author.clone(),
                        body.clone(),
                        false,
                        msg.hlc.millis,
                    );
                    // Use the real message ID so reactions can target it.
                    chat_msg.id = msg.id.to_string();

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

                    let current_topic = server_state
                        .topic_for_name(&state.current_channel)
                        .unwrap_or_default();
                    if chat_msg.topic != current_topic {
                        *unread.counts.entry(chat_msg.topic.clone()).or_insert(0) += 1;

                        // Show OS notification for messages on other channels.
                        let channel_name = server_state
                            .name_for_topic(&chat_msg.topic)
                            .unwrap_or("unknown");
                        crate::notify::send_notification(
                            &format!("#{channel_name}"),
                            &format!("{}: {}", chat_msg.author, chat_msg.body),
                        );
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
                state
                    .messages
                    .push(ChatMessage::new(topic.clone(), author, body, false, ts));
                state.messages_dirty = true;
            }
            NetworkBridgeEvent::FileDownloaded { filename, .. } => {
                info!("file downloaded: {filename}");
            }
            NetworkBridgeEvent::ProfileReceived {
                peer_id,
                display_name,
            } => {
                profiles.names.insert(peer_id.clone(), display_name.clone());
            }
            NetworkBridgeEvent::OpReceived { stamped_op, from } => {
                let applied = handle_op(
                    stamped_op,
                    from,
                    &mut server_state,
                    &mut key_store,
                    &mut state,
                    &net_cmd,
                    &mut op_log,
                    &identity,
                );

                // Process chat messages for display after the general op
                // handling (dedup, HLC advance) has completed.
                if applied {
                    if let crate::server_sync::Op::ChatMessage {
                        topic,
                        content_data,
                    } = &stamped_op.op
                    {
                        process_chat_message(
                            topic,
                            content_data,
                            &stamped_op.author,
                            &stamped_op.op_id,
                            stamped_op.hlc.millis,
                            &mut key_store,
                            &mut state,
                            &db,
                            &profiles,
                            &server_state,
                            &mut unread,
                        );
                    }
                }
            }
            NetworkBridgeEvent::SyncRequested { latest_hlc, from } => {
                // Only respond to trusted peers.
                let owner = server_state
                    .server
                    .as_ref()
                    .map(|s| s.owner.to_string())
                    .unwrap_or_default();
                if op_log.is_trusted(from, &owner) {
                    let missing: Vec<_> = op_log
                        .ops
                        .iter()
                        .filter(|op| op.hlc > *latest_hlc)
                        .cloned()
                        .collect();
                    let count = missing.len();
                    if !missing.is_empty() {
                        let _ = net_cmd.0.send(
                            crate::network_bridge::NetworkBridgeCommand::SendSyncBatch {
                                ops: missing,
                            },
                        );
                        info!("sent {count} ops to sync peer {from}");
                    }
                }
            }
            NetworkBridgeEvent::SyncBatchReceived { ops, from } => {
                // Verify batch sender is trusted.
                let owner = server_state
                    .server
                    .as_ref()
                    .map(|s| s.owner.to_string())
                    .unwrap_or_default();
                if !op_log.is_trusted(from, &owner) {
                    continue;
                }
                let mut sorted_ops = ops.clone();
                sorted_ops.sort_by(|a, b| a.hlc.cmp(&b.hlc));
                let count = sorted_ops.len();
                for stamped_op in &sorted_ops {
                    let applied = handle_op(
                        stamped_op,
                        &stamped_op.author,
                        &mut server_state,
                        &mut key_store,
                        &mut state,
                        &net_cmd,
                        &mut op_log,
                        &identity,
                    );

                    if applied {
                        if let crate::server_sync::Op::ChatMessage {
                            topic,
                            content_data,
                        } = &stamped_op.op
                        {
                            process_chat_message(
                                topic,
                                content_data,
                                &stamped_op.author,
                                &stamped_op.op_id,
                                stamped_op.hlc.millis,
                                &mut key_store,
                                &mut state,
                                &db,
                                &profiles,
                                &server_state,
                                &mut unread,
                            );
                        }
                    }
                }
                if count > 0 {
                    info!("applied sync batch of {count} ops from {from}");
                }
            }
        }
    }
}

/// Handle clicks on channel buttons in the sidebar.
pub fn handle_channel_click(
    interaction_query: Query<(&Interaction, &ChannelButton), Changed<Interaction>>,
    mut state: ResMut<ChatState>,
    server_state: Res<ServerState>,
    mut unread: ResMut<UnreadCounts>,
) {
    for (interaction, button) in &interaction_query {
        if *interaction == Interaction::Pressed && state.current_channel != button.0 {
            state.current_channel = button.0.clone();
            state.messages_dirty = true;

            if let Some(topic) = server_state.topic_for_name(&button.0) {
                unread.counts.remove(&topic);
            }
        }
    }
}

/// Re-render the message list when messages change or channel switches.
pub fn sync_message_list(
    mut commands: Commands,
    mut state: ResMut<ChatState>,
    list_query: Query<Entity, With<MessageList>>,
    server_state: Res<ServerState>,
    search: Res<SearchFilter>,
    emoji_reg: Res<EmojiRegistryRes>,
) {
    if !state.messages_dirty && !search.is_changed() {
        return;
    }
    state.messages_dirty = false;

    let Ok(list_entity) = list_query.single() else {
        return;
    };

    commands.entity(list_entity).detach_all_children();

    let current_topic = server_state
        .topic_for_name(&state.current_channel)
        .unwrap_or_default();

    let query_lower = search.query.to_lowercase();
    let visible: Vec<_> = state
        .messages
        .iter()
        .filter(|m| m.topic == current_topic)
        .filter(|m| {
            !search.active
                || query_lower.is_empty()
                || m.body.to_lowercase().contains(&query_lower)
                || m.author.to_lowercase().contains(&query_lower)
        })
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

            let time_str = super::format_timestamp(msg.timestamp_ms);

            parent
                .spawn(Node {
                    margin: UiRect::bottom(Val::Px(4.0)),
                    flex_direction: FlexDirection::Column,
                    ..default()
                })
                .with_children(|col| {
                    // Reply preview (if this is a reply)
                    if let Some(ref preview) = msg.reply_preview {
                        col.spawn(Node {
                            padding: UiRect::left(Val::Px(48.0)),
                            margin: UiRect::bottom(Val::Px(2.0)),
                            ..default()
                        })
                        .with_children(|reply_row| {
                            reply_row.spawn((
                                Text::new(format!("↳ {preview}")),
                                TextFont::from_font_size(11.0),
                                TextColor(theme::TEXT_MUTED),
                            ));
                        });
                    }

                    // Message text row
                    col.spawn(Node::default()).with_children(|row| {
                        row.spawn((
                            Text::new(format!("{time_str} ")),
                            TextFont::from_font_size(11.0),
                            TextColor(theme::TEXT_PLACEHOLDER),
                        ));

                        let body_color = if msg.deleted {
                            theme::TEXT_MUTED
                        } else {
                            theme::TEXT_PRIMARY
                        };

                        let mut author_text = row.spawn((
                            Text::new(format!("{}: ", msg.author)),
                            TextFont::from_font_size(14.0),
                            TextColor(author_color),
                        ));
                        author_text.with_child((
                            TextSpan::new(emoji_reg.0.expand(&msg.body)),
                            TextFont::from_font_size(14.0),
                            TextColor(body_color),
                        ));
                        if msg.edited && !msg.deleted {
                            author_text.with_child((
                                TextSpan::new(" (edited)"),
                                TextFont::from_font_size(10.0),
                                TextColor(theme::TEXT_PLACEHOLDER),
                            ));
                        }
                    });

                    // Reactions row (if any)
                    if !msg.reactions.is_empty() {
                        col.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            margin: UiRect::top(Val::Px(2.0)),
                            padding: UiRect::left(Val::Px(48.0)),
                            ..default()
                        })
                        .with_children(|reactions_row| {
                            for (emoji, authors) in &msg.reactions {
                                reactions_row
                                    .spawn((
                                        Node {
                                            padding: UiRect::new(
                                                Val::Px(6.0),
                                                Val::Px(6.0),
                                                Val::Px(2.0),
                                                Val::Px(2.0),
                                            ),
                                            margin: UiRect::right(Val::Px(4.0)),
                                            ..default()
                                        },
                                        BackgroundColor(theme::INPUT_FIELD_BG),
                                    ))
                                    .with_children(|badge| {
                                        badge.spawn((
                                            Text::new(format!("{emoji} {}", authors.len())),
                                            TextFont::from_font_size(12.0),
                                            TextColor(theme::TEXT_SECONDARY),
                                        ));
                                    });
                            }
                        });
                    }
                });
        }
    });
}

/// Update the peer count label.
pub fn update_peer_count(state: Res<ChatState>, mut query: Query<&mut Text, With<PeerCount>>) {
    if !state.is_changed() {
        return;
    }
    for mut text in &mut query {
        let n = state.peers.len();
        **text = if n == 1 {
            "1 peer".to_string()
        } else {
            format!("{n} peers")
        };
    }
}

/// Update the channel header text.
pub fn update_channel_header(
    state: Res<ChatState>,
    search: Res<SearchFilter>,
    mut query: Query<&mut Text, With<ChannelHeader>>,
) {
    if !state.is_changed() && !search.is_changed() {
        return;
    }
    for mut text in &mut query {
        if search.active {
            if search.query.is_empty() {
                **text = format!("# {} — Search...", state.current_channel);
            } else {
                **text = format!("# {} — Search: {}", state.current_channel, search.query);
            }
        } else {
            **text = format!("# {}", state.current_channel);
        }
    }
}

/// Prune old messages to prevent unbounded memory growth.
pub fn prune_messages(mut state: ResMut<ChatState>) {
    state.prune_if_needed();
}

/// Highlight the active channel and show unread counts.
pub fn update_channel_highlights(
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
                if count > 0 && !is_active {
                    **text = format!("# {} ({})", button.0, count);
                } else {
                    **text = format!("# {}", button.0);
                }

                *color = if is_active {
                    TextColor(theme::TEXT_PRIMARY)
                } else if count > 0 {
                    TextColor(theme::UNREAD_HIGHLIGHT)
                } else {
                    TextColor(theme::TEXT_MUTED)
                };
            }
        }
    }
}

/// Apply a remote server operation to local state.
///
/// Returns `true` if the op was new and accepted (not deduplicated or
/// rejected). Chat messages bypass the trust check — anyone who can
/// subscribe to a channel topic can chat. Trust is enforced only for
/// server state mutations.
#[allow(clippy::too_many_arguments)]
fn handle_op(
    stamped: &crate::server_sync::StampedOp,
    from: &str,
    server_state: &mut ResMut<ServerState>,
    key_store: &mut ResMut<ChannelKeyStore>,
    state: &mut ResMut<ChatState>,
    net_cmd: &Res<crate::network_bridge::NetworkCommandSender>,
    op_log: &mut ResMut<OpLog>,
    identity: &Res<crate::network_bridge::LocalIdentity>,
) -> bool {
    use crate::server_sync::Op;

    // Dedup: skip if we've already seen this op.
    if op_log.seen_ids.contains(&stamped.op_id) {
        return false;
    }

    // Trust check: only for non-chat ops (server state mutations).
    let needs_trust = !matches!(stamped.op, Op::ChatMessage { .. });
    if needs_trust {
        let owner = server_state
            .server
            .as_ref()
            .map(|s| s.owner.to_string())
            .unwrap_or_default();
        if !op_log.is_trusted(from, &owner) {
            warn!("untrusted op from {from}, recording id only");
            op_log.seen_ids.insert(stamped.op_id.clone());
            return false;
        }
    }

    // Advance local HLC.
    state.hlc.receive(stamped.hlc);

    // Record (chat messages go to seen_ids only, not ops — see OpLog::record).
    op_log.record(stamped.clone());

    // Persist op log only for server ops (not chat messages).
    if needs_trust {
        crate::storage::save_op_log(&op_log.ops);
    }

    match &stamped.op {
        Op::CreateChannel { name, channel_id } => {
            let info = {
                let Some(server) = &mut server_state.server else {
                    return true;
                };
                if server.channels().iter().any(|ch| ch.name == *name) {
                    return true;
                }
                let ch_uuid =
                    uuid::Uuid::parse_str(channel_id).unwrap_or_else(|_| uuid::Uuid::new_v4());
                let ch_id = willow_channel::ChannelId(ch_uuid);
                let Ok(ch_id) =
                    server.create_channel_with_id(ch_id, name, willow_channel::ChannelKind::Text)
                else {
                    return true;
                };
                let topic = super::make_topic(server, name);
                let key = server.channel_key(&ch_id).cloned();
                (topic, ch_id, key)
            };
            let (topic, ch_id, key) = info;
            if let Some(k) = key {
                key_store.keys.insert(topic.clone(), k);
            }
            server_state
                .topic_map
                .insert(topic.clone(), (name.clone(), ch_id));
            if let Some(server) = &server_state.server {
                crate::storage::save_server(server, &key_store.keys);
            }
            let _ = net_cmd
                .0
                .send(crate::network_bridge::NetworkBridgeCommand::Subscribe(
                    topic,
                ));
            info!("remote channel #{name} created by {from}");
        }
        Op::DeleteChannel { name } => {
            let to_remove = server_state
                .topic_map
                .iter()
                .find(|(_, (n, _))| n == name)
                .map(|(t, (_, id))| (t.clone(), id.clone()));

            if let Some((topic, ch_id)) = to_remove {
                if let Some(server) = &mut server_state.server {
                    let _ = server.delete_channel(&ch_id);
                    crate::storage::save_server(server, &key_store.keys);
                }
                server_state.topic_map.remove(&topic);
                key_store.keys.remove(&topic);

                if state.current_channel == *name {
                    let names = server_state.channel_names();
                    state.current_channel = names.first().cloned().unwrap_or_default();
                    state.messages_dirty = true;
                }
                info!("remote channel #{name} deleted by {from}");
            }
        }
        Op::CreateRole { name, role_id } => {
            if let Some(server) = &mut server_state.server {
                if !server.roles().iter().any(|r| r.name == *name) {
                    let rid =
                        willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
                    let role = willow_channel::Role::with_id(rid, name);
                    server.create_role(role);
                    crate::storage::save_server(server, &key_store.keys);
                    info!("remote role '{name}' created by {from}");
                }
            }
        }
        Op::DeleteRole { role_id } => {
            if let Some(server) = &mut server_state.server {
                let rid =
                    willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
                let _ = server.delete_role(&rid);
                crate::storage::save_server(server, &key_store.keys);
            }
        }
        Op::SetPermission {
            role_id,
            permission,
            granted,
        } => {
            if let Some(server) = &mut server_state.server {
                let rid =
                    willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
                let perm = match permission.as_str() {
                    "Administrator" => willow_channel::Permission::Administrator,
                    "SendMessages" => willow_channel::Permission::SendMessages,
                    "ReadMessages" => willow_channel::Permission::ReadMessages,
                    "KickMembers" => willow_channel::Permission::KickMembers,
                    "CreateInvite" => willow_channel::Permission::CreateInvite,
                    "AttachFiles" => willow_channel::Permission::AttachFiles,
                    "ManageChannels" => willow_channel::Permission::ManageChannels,
                    _ => return true,
                };
                let _ = server.set_permission(&rid, perm, *granted);
                crate::storage::save_server(server, &key_store.keys);
            }
        }
        Op::AssignRole { peer_id, role_id } => {
            if let Some(server) = &mut server_state.server {
                let rid =
                    willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
                let member_peer = server
                    .members()
                    .iter()
                    .find(|m| m.peer_id.to_string() == *peer_id)
                    .map(|m| m.peer_id.clone());
                if let Some(peer) = member_peer {
                    let _ = server.assign_role(&peer, &rid);
                    crate::storage::save_server(server, &key_store.keys);
                }
            }
        }
        Op::KickMember {
            peer_id,
            rotated_keys,
        } => {
            state.peers.retain(|p| p != peer_id);

            // Remove member from server (rotates keys locally).
            {
                if let Some(server) = &mut server_state.server {
                    let member_peer = server
                        .members()
                        .iter()
                        .find(|m| m.peer_id.to_string() == *peer_id)
                        .map(|m| m.peer_id.clone());
                    if let Some(peer) = member_peer {
                        let _ = server.remove_member(&peer);
                    }
                }
            }

            // Decrypt and apply rotated keys intended for us.
            let our_peer_id = identity.0.peer_id().to_string();
            for (recipient, topic, encrypted) in rotated_keys {
                if *recipient == our_peer_id {
                    if let Ok(key) = willow_crypto::decrypt_channel_key(encrypted, &identity.0) {
                        key_store.keys.insert(topic.clone(), key.clone());
                        let ch_id = server_state.topic_map.get(topic).map(|(_, id)| id.clone());
                        if let (Some(ch_id), Some(server)) = (ch_id, server_state.server.as_mut()) {
                            server.set_channel_key(ch_id, key);
                        }
                    }
                }
            }

            if let Some(server) = &server_state.server {
                crate::storage::save_server(server, &key_store.keys);
            }
            info!("peer {peer_id} kicked by {from}, keys rotated");
        }
        Op::TrustPeer { peer_id } => {
            info!("peer {peer_id} trusted by {from}");
        }
        Op::UntrustPeer { peer_id } => {
            info!("peer {peer_id} untrusted by {from}");
        }
        // Chat message display is handled by the caller after handle_op returns.
        Op::ChatMessage { .. } => {}
    }

    true
}

/// Process a ChatMessage op for display: deserialize content, decrypt if
/// needed, and add to ChatState / persist to MessageDb.
#[allow(clippy::too_many_arguments)]
fn process_chat_message(
    topic: &str,
    content_data: &[u8],
    author_peer_id: &str,
    op_id: &str,
    hlc_millis: u64,
    key_store: &mut ResMut<ChannelKeyStore>,
    state: &mut ResMut<ChatState>,
    db: &Res<MessageDbRes>,
    profiles: &ResMut<ProfileStore>,
    server_state: &ResMut<ServerState>,
    unread: &mut ResMut<UnreadCounts>,
) {
    let Ok(content) = willow_transport::unpack::<Content>(content_data) else {
        warn!("failed to deserialize ChatMessage content");
        return;
    };

    // Decrypt if encrypted.
    let content = match &content {
        Content::Encrypted(sealed) => {
            let Some(key) = key_store.keys.get(topic) else {
                return;
            };
            match willow_crypto::open_content(sealed, key) {
                Ok(c) => c,
                Err(_) => return,
            }
        }
        other => other.clone(),
    };

    let author = profiles.display_name(author_peer_id);

    // Handle reactions.
    if let Content::Reaction {
        ref target,
        ref emoji,
    } = content
    {
        let target_str = target.to_string();
        for m in &mut state.messages {
            if m.id == target_str {
                m.reactions
                    .entry(emoji.clone())
                    .or_default()
                    .push(author.clone());
                state.messages_dirty = true;
                break;
            }
        }
        return;
    }

    // Handle edits.
    if let Content::Edit {
        ref target,
        ref new_body,
    } = content
    {
        let target_str = target.to_string();
        for m in &mut state.messages {
            if m.id == target_str {
                m.body = new_body.clone();
                m.edited = true;
                state.messages_dirty = true;
                break;
            }
        }
        return;
    }

    // Handle deletes.
    if let Content::Delete { ref target } = content {
        let target_str = target.to_string();
        for m in &mut state.messages {
            if m.id == target_str {
                m.body = "[message deleted]".to_string();
                m.deleted = true;
                m.reactions.clear();
                state.messages_dirty = true;
                break;
            }
        }
        return;
    }

    // Handle replies.
    if let Content::Reply {
        ref parent,
        ref body,
    } = content
    {
        let parent_str = parent.to_string();

        let preview = state.messages.iter().find(|m| m.id == parent_str).map(|m| {
            let text = if m.body.len() > 50 {
                format!("{}...", &m.body[..50])
            } else {
                m.body.clone()
            };
            format!("{}: {text}", m.author)
        });

        let mut chat_msg =
            ChatMessage::new(topic.to_string(), author, body.clone(), false, hlc_millis);
        chat_msg.id = op_id.to_string();
        chat_msg.reply_preview = preview;

        state.messages.push(chat_msg);
        state.messages_dirty = true;
        return;
    }

    // Handle text messages.
    if let Content::Text { ref body } = content {
        let mut chat_msg = ChatMessage::new(
            topic.to_string(),
            author.clone(),
            body.clone(),
            false,
            hlc_millis,
        );
        chat_msg.id = op_id.to_string();

        if let Some(ref db_arc) = db.0 {
            if let Ok(db_lock) = db_arc.lock() {
                db_lock.insert(&crate::storage::StoredMessage {
                    topic: topic.to_string(),
                    author: author.clone(),
                    body: body.clone(),
                    is_local: false,
                    timestamp_ms: hlc_millis,
                });
            }
        }

        let current_topic = server_state
            .topic_for_name(&state.current_channel)
            .unwrap_or_default();
        if chat_msg.topic != current_topic {
            *unread.counts.entry(chat_msg.topic.clone()).or_insert(0) += 1;

            let channel_name = server_state
                .name_for_topic(&chat_msg.topic)
                .unwrap_or("unknown");
            crate::notify::send_notification(
                &format!("#{channel_name}"),
                &format!("{}: {}", chat_msg.author, chat_msg.body),
            );
        }

        state.messages.push(chat_msg);
        state.messages_dirty = true;
    }
}
