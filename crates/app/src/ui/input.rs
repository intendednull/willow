//! Keyboard input handling and message sending.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use crate::network_bridge::{LocalIdentity, NetworkCommandSender};
use crate::theme;
use willow_messaging::Content;

use super::components::*;
use super::constants;
use super::resources::*;

/// Capture keyboard input and route to the appropriate field based on view.
///
/// This system only mutates resources (`InputState`, `SettingsInput`).
/// UI text updates are handled reactively by `sync_input_text` and
/// `sync_settings_fields`.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn handle_keyboard_input(
    mut key_events: MessageReader<KeyboardInput>,
    mut input: ResMut<InputState>,
    mut view: ResMut<AppView>,
    mut settings_input: ResMut<SettingsInput>,
    mut search: ResMut<SearchFilter>,
    keys: Res<ButtonInput<KeyCode>>,
    mut channel_mgmt: ResMut<ChannelManagement>,
    state: Res<ChatState>,
) {
    for event in key_events.read() {
        if !event.state.is_pressed() {
            continue;
        }

        // Skip when channel creation input is active.
        if channel_mgmt.creating_channel {
            continue;
        }

        if *view == AppView::Settings {
            match event.key_code {
                KeyCode::Tab => {
                    settings_input.focused_field = match settings_input.focused_field {
                        SettingsField::DisplayName => SettingsField::RelayAddr,
                        SettingsField::RelayAddr => SettingsField::InviteRecipient,
                        SettingsField::InviteRecipient => SettingsField::JoinCode,
                        SettingsField::JoinCode => SettingsField::DisplayName,
                    };
                }
                KeyCode::Backspace => match settings_input.focused_field {
                    SettingsField::DisplayName => {
                        settings_input.display_name.pop();
                    }
                    SettingsField::RelayAddr => {
                        settings_input.relay_addr.pop();
                    }
                    SettingsField::InviteRecipient => {
                        channel_mgmt.invite_recipient.pop();
                    }
                    SettingsField::JoinCode => {
                        channel_mgmt.join_code.pop();
                    }
                },
                KeyCode::Escape => {
                    *view = AppView::Chat;
                }
                _ => {
                    if let Some(ref s) = event.text {
                        let target = match settings_input.focused_field {
                            SettingsField::DisplayName => &mut settings_input.display_name,
                            SettingsField::RelayAddr => &mut settings_input.relay_addr,
                            SettingsField::InviteRecipient => &mut channel_mgmt.invite_recipient,
                            SettingsField::JoinCode => &mut channel_mgmt.join_code,
                        };
                        for c in s.chars() {
                            if !c.is_control() {
                                target.push(c);
                            }
                        }
                    }
                }
            }
        } else if search.active {
            // Search mode.
            match event.key_code {
                KeyCode::Escape | KeyCode::Enter => {
                    search.active = false;
                    search.query.clear();
                }
                KeyCode::Backspace => {
                    search.query.pop();
                }
                _ => {
                    if let Some(ref s) = event.text {
                        for c in s.chars() {
                            if !c.is_control() {
                                search.query.push(c);
                            }
                        }
                    }
                }
            }
        } else {
            // Chat mode.
            // Ctrl+F opens search.
            if event.key_code == KeyCode::KeyF
                && (keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight))
            {
                search.active = true;
                search.query.clear();
                continue;
            }

            // Ctrl+R → reply to last message.
            if event.key_code == KeyCode::KeyR
                && (keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight))
                && input.replying_to.is_none()
            {
                if let Some(last_msg) = state.messages.iter().rev().find(|m| !m.deleted) {
                    let preview = if last_msg.body.len() > 40 {
                        format!("{}: {}...", last_msg.author, &last_msg.body[..40])
                    } else {
                        format!("{}: {}", last_msg.author, last_msg.body)
                    };
                    input.replying_to = Some((last_msg.id.clone(), preview));
                }
                continue;
            }

            // Escape cancels reply.
            if event.key_code == KeyCode::Escape && input.replying_to.is_some() {
                input.replying_to = None;
                continue;
            }

            // Up arrow with empty input → edit last own message.
            if event.key_code == KeyCode::ArrowUp
                && input.text.is_empty()
                && input.editing_message_id.is_none()
            {
                if let Some(last_own) = state
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.is_local && !m.deleted)
                {
                    input.editing_message_id = Some(last_own.id.clone());
                    input.text = last_own.body.clone();
                }
                continue;
            }

            // Ctrl+Backspace while editing → delete the message.
            if event.key_code == KeyCode::Backspace
                && (keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight))
                && input.editing_message_id.is_some()
            {
                // Mark as delete request — send_message will handle it.
                input.send_requested = true;
                input.text = "$$DELETE$$".to_string();
                continue;
            }

            // Escape while editing → cancel edit.
            if event.key_code == KeyCode::Escape && input.editing_message_id.is_some() {
                input.editing_message_id = None;
                input.text.clear();
                continue;
            }

            match event.key_code {
                KeyCode::Enter => {
                    if !input.text.is_empty() {
                        input.send_requested = true;
                    } else if input.editing_message_id.is_some() {
                        // Enter with empty text while editing → cancel.
                        input.editing_message_id = None;
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

/// Send a message when Enter was pressed.
///
/// Messages are routed through the unified Op pipeline: the content is
/// serialized into a [`crate::server_sync::Op::ChatMessage`], wrapped in a
/// [`crate::server_sync::StampedOp`], recorded in the [`OpLog`] for dedup,
/// and broadcast via [`crate::network_bridge::NetworkBridgeCommand::BroadcastOp`].
#[allow(clippy::too_many_arguments)]
pub fn send_message(
    mut input: ResMut<InputState>,
    mut state: ResMut<ChatState>,
    identity: Res<LocalIdentity>,
    net_cmd: Res<NetworkCommandSender>,
    key_store: Res<ChannelKeyStore>,
    server_state: Res<ServerState>,
    db: Res<MessageDbRes>,
    profiles: Res<ProfileStore>,
    mut op_log: ResMut<OpLog>,
) {
    if !input.send_requested {
        return;
    }
    input.send_requested = false;

    let body = input.text.drain(..).collect::<String>();
    let editing_id = input.editing_message_id.take();

    if body.is_empty() && editing_id.is_none() {
        return;
    }

    let channel_name = state.current_channel.clone();
    let peer_id_str = identity.0.peer_id().to_string();

    let topic = match server_state.topic_for_name(&channel_name) {
        Some(t) => t,
        None => channel_name.clone(),
    };

    // Handle edit or delete of an existing message.
    if let Some(ref target_id) = editing_id {
        let target_msg_id =
            willow_messaging::MessageId(uuid::Uuid::parse_str(target_id).unwrap_or_default());

        let content = if body == "$$DELETE$$" {
            Content::Delete {
                target: target_msg_id.clone(),
            }
        } else {
            Content::Edit {
                target: target_msg_id.clone(),
                new_body: body.clone(),
            }
        };

        // Encrypt if channel key exists.
        let wire_content = if let Some(key) = key_store.keys.get(&topic) {
            if let Ok(sealed) = willow_crypto::seal_content(&content, key, 0) {
                Content::Encrypted(sealed)
            } else {
                content.clone()
            }
        } else {
            content.clone()
        };

        let content_data = willow_transport::pack(&wire_content).unwrap_or_default();
        let stamped = crate::server_sync::StampedOp::new(
            crate::server_sync::Op::ChatMessage {
                topic,
                content_data,
            },
            &mut state.hlc,
            &peer_id_str,
        );

        op_log.record(stamped.clone());

        let _ = net_cmd
            .0
            .send(crate::network_bridge::NetworkBridgeCommand::BroadcastOp(
                stamped,
            ));

        // Apply locally.
        let target_str = target_msg_id.to_string();
        for m in &mut state.messages {
            if m.id == target_str {
                match content {
                    Content::Edit { ref new_body, .. } => {
                        m.body = new_body.clone();
                        m.edited = true;
                    }
                    Content::Delete { .. } => {
                        m.body = "[message deleted]".to_string();
                        m.deleted = true;
                        m.reactions.clear();
                    }
                    _ => {}
                }
                state.messages_dirty = true;
                break;
            }
        }

        return;
    }

    if body.is_empty() {
        return;
    }

    let replying = input.replying_to.take();

    // Build content — either a reply or a regular text message.
    let content = if let Some((ref parent_id, _)) = replying {
        let parent =
            willow_messaging::MessageId(uuid::Uuid::parse_str(parent_id).unwrap_or_default());
        Content::Reply {
            parent,
            body: body.clone(),
        }
    } else {
        Content::Text { body: body.clone() }
    };

    // Encrypt if channel key exists.
    let wire_content = if let Some(key) = key_store.keys.get(&topic) {
        if let Ok(sealed) = willow_crypto::seal_content(&content, key, 0) {
            Content::Encrypted(sealed)
        } else {
            content.clone()
        }
    } else {
        content
    };

    // Serialize and create StampedOp.
    let content_data = willow_transport::pack(&wire_content).unwrap_or_default();
    let stamped = crate::server_sync::StampedOp::new(
        crate::server_sync::Op::ChatMessage {
            topic: topic.clone(),
            content_data,
        },
        &mut state.hlc,
        &peer_id_str,
    );

    // Record for dedup (in-memory only — chat messages are not persisted to op log file).
    op_log.record(stamped.clone());

    // Broadcast via the unified op pipeline.
    let _ = net_cmd
        .0
        .send(crate::network_bridge::NetworkBridgeCommand::BroadcastOp(
            stamped.clone(),
        ));

    // Add to local display.
    let author = profiles.display_name(&peer_id_str);
    let ts = stamped.hlc.millis;
    let mut chat_msg = ChatMessage::new(topic, author.clone(), body.clone(), true, ts);
    chat_msg.id = stamped.op_id.clone();
    if let Some((_, ref preview)) = replying {
        chat_msg.reply_preview = Some(preview.clone());
    }

    // Persist to MessageDb.
    if let Some(ref db) = db.0 {
        if let Ok(db) = db.lock() {
            db.insert(&crate::storage::StoredMessage {
                topic: chat_msg.topic.clone(),
                author,
                body,
                is_local: true,
                timestamp_ms: ts,
            });
        }
    }

    state.messages.push(chat_msg);
    state.messages_dirty = true;
}

/// Sync the chat input text display reactively from InputState.
pub fn sync_input_text(
    input: Res<InputState>,
    mut query: Query<(&mut Text, &mut TextColor), With<InputText>>,
) {
    if !input.is_changed() {
        return;
    }
    for (mut text, mut color) in &mut query {
        if input.editing_message_id.is_some() {
            **text = format!("[editing] {}", input.text);
            *color = TextColor(theme::UNREAD_HIGHLIGHT);
        } else if let Some((_, ref preview)) = input.replying_to {
            if input.text.is_empty() {
                **text = format!("↳ replying to {preview}");
                *color = TextColor(theme::TEXT_MUTED);
            } else {
                **text = format!("↳ {}", input.text);
                *color = TextColor(theme::TEXT_PRIMARY);
            }
        } else if input.text.is_empty() {
            **text = constants::CHAT_PLACEHOLDER.to_string();
            *color = TextColor(theme::TEXT_PLACEHOLDER);
        } else {
            **text = input.text.clone();
            *color = TextColor(theme::TEXT_PRIMARY);
        }
    }
}
