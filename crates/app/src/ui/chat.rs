//! Chat systems — network events, message rendering, channel navigation.

use bevy::prelude::*;

use crate::network_bridge::NetworkBridgeEvent;
use crate::theme;
use willow_messaging::{Content, Message};
use willow_transport::{unpack_envelope, MessageType};

use super::components::*;
use super::resources::*;

/// Process incoming network events and update the chat state.
pub fn handle_network_events(
    mut reader: MessageReader<NetworkBridgeEvent>,
    mut state: ResMut<ChatState>,
    key_store: Res<ChannelKeyStore>,
    db: Res<MessageDbRes>,
    mut profiles: ResMut<ProfileStore>,
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
                    // Message text row
                    col.spawn(Node::default()).with_children(|row| {
                        row.spawn((
                            Text::new(format!("{time_str} ")),
                            TextFont::from_font_size(11.0),
                            TextColor(theme::TEXT_PLACEHOLDER),
                        ));

                        row.spawn((
                            Text::new(format!("{}: ", msg.author)),
                            TextFont::from_font_size(14.0),
                            TextColor(author_color),
                        ))
                        .with_child((
                            TextSpan::new(emoji_reg.0.expand(&msg.body)),
                            TextFont::from_font_size(14.0),
                            TextColor(theme::TEXT_PRIMARY),
                        ));
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
