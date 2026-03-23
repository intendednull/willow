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
) {
    if !state.messages_dirty {
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

            let time_str = super::format_timestamp(msg.timestamp_ms);

            parent
                .spawn(Node {
                    margin: UiRect::bottom(Val::Px(4.0)),
                    ..default()
                })
                .with_children(|row| {
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
                        TextSpan::new(&msg.body),
                        TextFont::from_font_size(14.0),
                        TextColor(theme::TEXT_PRIMARY),
                    ));
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
    mut query: Query<&mut Text, With<ChannelHeader>>,
) {
    if !state.is_changed() {
        return;
    }
    for mut text in &mut query {
        **text = format!("# {}", state.current_channel);
    }
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
