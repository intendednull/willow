//! Keyboard input handling and message sending.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use crate::network_bridge::{LocalIdentity, NetworkCommandSender};
use crate::theme;
use willow_messaging::{Content, Message};
use willow_transport::{pack_envelope, MessageType};

#[allow(unused_imports)]
use super::resources::ChatMessage;

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
    let peer_id = identity.0.peer_id();

    let topic = match server_state.topic_for_name(&channel_name) {
        Some(t) => t,
        None => channel_name.clone(),
    };

    let channel_id = willow_messaging::ChannelId::new();

    // Handle edit or delete of an existing message.
    if let Some(ref target_id) = editing_id {
        let target_msg_id =
            willow_messaging::MessageId(uuid::Uuid::parse_str(target_id).unwrap_or_default());

        let content = if body == "$$DELETE$$" {
            // Delete the message.
            Content::Delete {
                target: target_msg_id.clone(),
            }
        } else {
            // Edit the message.
            Content::Edit {
                target: target_msg_id.clone(),
                new_body: body.clone(),
            }
        };

        let msg = Message {
            id: willow_messaging::MessageId::new(),
            channel_id,
            author: peer_id.clone(),
            content: content.clone(),
            created_at: chrono::Utc::now(),
            hlc: state.hlc.now(),
        };

        if let Ok(envelope_data) = pack_envelope(MessageType::Chat, &msg) {
            if let Ok(signed_data) = willow_identity::pack(&envelope_data, &identity.0) {
                let _ = net_cmd
                    .0
                    .send(crate::network_bridge::NetworkBridgeCommand::Publish {
                        topic,
                        data: signed_data,
                    });
            }
        }

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

    let mut msg = Message::text(channel_id, peer_id.clone(), &body, &mut state.hlc);

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
    let mut chat_msg = ChatMessage::new(topic, author.clone(), body.clone(), true, ts);
    chat_msg.id = msg.id.to_string();

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

/// Sync the chat input text display reactively from InputState.
pub fn sync_input_text(
    input: Res<InputState>,
    mut query: Query<(&mut Text, &mut TextColor), With<InputText>>,
) {
    if !input.is_changed() {
        return;
    }
    for (mut text, mut color) in &mut query {
        if input.text.is_empty() && input.editing_message_id.is_none() {
            **text = constants::CHAT_PLACEHOLDER.to_string();
            *color = TextColor(theme::TEXT_PLACEHOLDER);
        } else if input.editing_message_id.is_some() {
            **text = format!("[editing] {}", input.text);
            *color = TextColor(theme::UNREAD_HIGHLIGHT);
        } else {
            **text = input.text.clone();
            *color = TextColor(theme::TEXT_PRIMARY);
        }
    }
}
