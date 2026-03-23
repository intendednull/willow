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

        let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);

        if *view == AppView::Settings {
            // Ctrl+V pastes into the focused settings field.
            if event.key_code == KeyCode::KeyV && ctrl {
                if let Some(text) = crate::clipboard::read_clipboard() {
                    let mut cursor = settings_input.cursor;
                    let target = match settings_input.focused_field {
                        SettingsField::DisplayName => &mut settings_input.display_name,
                        SettingsField::RelayAddr => &mut settings_input.relay_addr,
                        SettingsField::InviteRecipient => &mut channel_mgmt.invite_recipient,
                        SettingsField::JoinCode => &mut channel_mgmt.join_code,
                    };
                    crate::text_edit::insert_str(target, &mut cursor, &text);
                    settings_input.cursor = cursor;
                }
                continue;
            }
            match event.key_code {
                KeyCode::Tab => {
                    settings_input.focused_field = match settings_input.focused_field {
                        SettingsField::DisplayName => SettingsField::RelayAddr,
                        SettingsField::RelayAddr => SettingsField::InviteRecipient,
                        SettingsField::InviteRecipient => SettingsField::JoinCode,
                        SettingsField::JoinCode => SettingsField::DisplayName,
                    };
                    // Reset cursor to end of new field.
                    let new_text = match settings_input.focused_field {
                        SettingsField::DisplayName => &settings_input.display_name,
                        SettingsField::RelayAddr => &settings_input.relay_addr,
                        SettingsField::InviteRecipient => &channel_mgmt.invite_recipient,
                        SettingsField::JoinCode => &channel_mgmt.join_code,
                    };
                    settings_input.cursor = crate::text_edit::char_len(new_text);
                }
                KeyCode::Backspace => {
                    let mut cursor = settings_input.cursor;
                    let target = match settings_input.focused_field {
                        SettingsField::DisplayName => &mut settings_input.display_name,
                        SettingsField::RelayAddr => &mut settings_input.relay_addr,
                        SettingsField::InviteRecipient => &mut channel_mgmt.invite_recipient,
                        SettingsField::JoinCode => &mut channel_mgmt.join_code,
                    };
                    if ctrl {
                        crate::text_edit::backspace_word(target, &mut cursor);
                    } else {
                        crate::text_edit::backspace(target, &mut cursor);
                    }
                    settings_input.cursor = cursor;
                }
                KeyCode::Delete => {
                    let mut cursor = settings_input.cursor;
                    let target = match settings_input.focused_field {
                        SettingsField::DisplayName => &mut settings_input.display_name,
                        SettingsField::RelayAddr => &mut settings_input.relay_addr,
                        SettingsField::InviteRecipient => &mut channel_mgmt.invite_recipient,
                        SettingsField::JoinCode => &mut channel_mgmt.join_code,
                    };
                    if ctrl {
                        crate::text_edit::delete_word(target, &mut cursor);
                    } else {
                        crate::text_edit::delete(target, &mut cursor);
                    }
                    settings_input.cursor = cursor;
                }
                KeyCode::ArrowLeft => {
                    let mut cursor = settings_input.cursor;
                    if ctrl {
                        let target = match settings_input.focused_field {
                            SettingsField::DisplayName => &settings_input.display_name,
                            SettingsField::RelayAddr => &settings_input.relay_addr,
                            SettingsField::InviteRecipient => &channel_mgmt.invite_recipient,
                            SettingsField::JoinCode => &channel_mgmt.join_code,
                        };
                        crate::text_edit::move_word_left(target, &mut cursor);
                    } else {
                        crate::text_edit::move_left(&mut cursor);
                    }
                    settings_input.cursor = cursor;
                }
                KeyCode::ArrowRight => {
                    let mut cursor = settings_input.cursor;
                    let target = match settings_input.focused_field {
                        SettingsField::DisplayName => &settings_input.display_name,
                        SettingsField::RelayAddr => &settings_input.relay_addr,
                        SettingsField::InviteRecipient => &channel_mgmt.invite_recipient,
                        SettingsField::JoinCode => &channel_mgmt.join_code,
                    };
                    if ctrl {
                        crate::text_edit::move_word_right(target, &mut cursor);
                    } else {
                        crate::text_edit::move_right(target, &mut cursor);
                    }
                    settings_input.cursor = cursor;
                }
                KeyCode::Home => {
                    crate::text_edit::move_home(&mut settings_input.cursor);
                }
                KeyCode::End => {
                    let mut cursor = settings_input.cursor;
                    let target = match settings_input.focused_field {
                        SettingsField::DisplayName => &settings_input.display_name,
                        SettingsField::RelayAddr => &settings_input.relay_addr,
                        SettingsField::InviteRecipient => &channel_mgmt.invite_recipient,
                        SettingsField::JoinCode => &channel_mgmt.join_code,
                    };
                    crate::text_edit::move_end(target, &mut cursor);
                    settings_input.cursor = cursor;
                }
                KeyCode::Escape => {
                    *view = AppView::Chat;
                }
                _ => {
                    if let Some(ref s) = event.text {
                        let mut cursor = settings_input.cursor;
                        let target = match settings_input.focused_field {
                            SettingsField::DisplayName => &mut settings_input.display_name,
                            SettingsField::RelayAddr => &mut settings_input.relay_addr,
                            SettingsField::InviteRecipient => &mut channel_mgmt.invite_recipient,
                            SettingsField::JoinCode => &mut channel_mgmt.join_code,
                        };
                        for c in s.chars() {
                            if !c.is_control() {
                                crate::text_edit::insert_char(target, &mut cursor, c);
                            }
                        }
                        settings_input.cursor = cursor;
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
            // Chat mode — any typing auto-focuses the input.
            if !input.focused {
                input.focused = true;
            }

            let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

            // Ctrl+A selects all.
            if event.key_code == KeyCode::KeyA && ctrl {
                input.select_all();
                continue;
            }

            // Ctrl+C copies selection to clipboard.
            if event.key_code == KeyCode::KeyC && ctrl {
                let selected = input.selected_text();
                if !selected.is_empty() {
                    crate::clipboard::copy_to_clipboard(&selected);
                }
                continue;
            }

            // Ctrl+X cuts selection to clipboard.
            if event.key_code == KeyCode::KeyX && ctrl {
                let selected = input.selected_text();
                if !selected.is_empty() {
                    crate::clipboard::copy_to_clipboard(&selected);
                }
                input.delete_selection();
                continue;
            }

            // Ctrl+V pastes from clipboard (replaces selection if any).
            if event.key_code == KeyCode::KeyV && ctrl {
                if let Some(text) = crate::clipboard::read_clipboard() {
                    input.insert_str(&text);
                }
                continue;
            }

            // Ctrl+F opens search.
            if event.key_code == KeyCode::KeyF && ctrl {
                search.active = true;
                search.query.clear();
                continue;
            }

            // Ctrl+R -> reply to last message.
            if event.key_code == KeyCode::KeyR && ctrl && input.replying_to.is_none() {
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

            // Up arrow with empty input -> edit last own message.
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
                    input.cursor = crate::text_edit::char_len(&input.text);
                }
                continue;
            }

            // Ctrl+Backspace while editing -> delete the message.
            if event.key_code == KeyCode::Backspace && ctrl && input.editing_message_id.is_some() {
                // Mark as delete request -- send_message will handle it.
                input.send_requested = true;
                input.text = "$$DELETE$$".to_string();
                input.cursor = 0;
                continue;
            }

            // Escape while editing -> cancel edit.
            if event.key_code == KeyCode::Escape && input.editing_message_id.is_some() {
                input.editing_message_id = None;
                input.text.clear();
                input.cursor = 0;
                continue;
            }

            match event.key_code {
                KeyCode::Enter => {
                    input.selection = None;
                    if !input.text.is_empty() {
                        input.send_requested = true;
                    } else if input.editing_message_id.is_some() {
                        input.editing_message_id = None;
                    }
                }
                KeyCode::Backspace => input.backspace(ctrl),
                KeyCode::Delete => input.delete(ctrl),
                KeyCode::ArrowLeft => input.move_left(ctrl, shift),
                KeyCode::ArrowRight => input.move_right(ctrl, shift),
                KeyCode::Home => input.move_home(shift),
                KeyCode::End => input.move_end(shift),
                _ => {
                    if let Some(ref s) = event.text {
                        for c in s.chars() {
                            if !c.is_control() {
                                input.insert_char(c);
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
    input.cursor = 0;

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

    // Build content -- either a reply or a regular text message.
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

    // Record for dedup (in-memory only -- chat messages are not persisted to op log file).
    op_log.record(stamped.clone());

    // Persist the stamped op for catch-up sync.
    if let Some(ref db_arc) = db.0 {
        if let Ok(db_lock) = db_arc.lock() {
            db_lock.insert_chat_op(&stamped, &topic);
        }
    }

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
                msg_id: String::new(),
            });
        }
    }

    state.messages.push(chat_msg);
    state.messages_dirty = true;
}

/// Handle clicks on the chat input area to toggle focus.
#[allow(clippy::type_complexity)]
pub fn handle_input_focus(
    input_query: Query<&Interaction, (Changed<Interaction>, With<ChatInputArea>)>,
    other_query: Query<&Interaction, (Changed<Interaction>, Without<ChatInputArea>, With<Button>)>,
    mut input: ResMut<InputState>,
    mut border_query: Query<&mut BorderColor, With<ChatInputArea>>,
) {
    // Clicking the input area focuses it.
    for interaction in &input_query {
        if *interaction == Interaction::Pressed {
            input.focused = true;
        }
    }

    // Clicking any other button unfocuses.
    if input.focused {
        for interaction in &other_query {
            if *interaction == Interaction::Pressed {
                input.focused = false;
            }
        }
    }

    // Update border color based on focus.
    for mut border in &mut border_query {
        *border = if input.focused {
            BorderColor::all(theme::ACCENT)
        } else {
            BorderColor::all(theme::TEXT_MUTED)
        };
    }
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
            if input.focused {
                let (before, after) = crate::text_edit::split_at_cursor(&input.text, input.cursor);
                **text = format!("[editing] {before}\u{25cf}{after}");
            } else {
                **text = format!("[editing] {}", input.text);
            }
            *color = TextColor(theme::UNREAD_HIGHLIGHT);
        } else if let Some((_, ref preview)) = input.replying_to {
            if input.text.is_empty() {
                **text = format!("> replying to {preview}");
                *color = TextColor(theme::TEXT_MUTED);
            } else if input.focused {
                let (before, after) = crate::text_edit::split_at_cursor(&input.text, input.cursor);
                **text = format!("\u{21b3} {before}\u{25cf}{after}");
                *color = TextColor(theme::TEXT_PRIMARY);
            } else {
                **text = format!("\u{21b3} {}", input.text);
                *color = TextColor(theme::TEXT_PRIMARY);
            }
        } else if input.text.is_empty() {
            let cursor_str = if input.focused { "|" } else { "" };
            **text = format!("{}{cursor_str}", constants::CHAT_PLACEHOLDER);
            *color = TextColor(theme::TEXT_PLACEHOLDER);
        } else if input.focused {
            let (before, after) = crate::text_edit::split_at_cursor(&input.text, input.cursor);
            **text = format!("{before}\u{2581}{after}");
            *color = TextColor(theme::TEXT_PRIMARY);
        } else {
            **text = input.text.clone();
            *color = TextColor(theme::TEXT_PRIMARY);
        }
    }
}

/// Poll the WASM paste buffer. On native, paste is handled synchronously
/// in `handle_keyboard_input` via Ctrl+V, so this system is a no-op.
pub fn poll_paste_buffer(
    mut input: ResMut<InputState>,
    view: Res<AppView>,
    mut settings_input: ResMut<SettingsInput>,
    mut channel_mgmt: ResMut<ChannelManagement>,
) {
    // On native, read_clipboard() always returns the current clipboard
    // contents, which would cause infinite pasting. Only poll on WASM.
    if cfg!(not(target_arch = "wasm32")) {
        return;
    }
    if let Some(text) = crate::clipboard::read_clipboard() {
        if *view == AppView::Settings {
            let mut cursor = settings_input.cursor;
            let target = match settings_input.focused_field {
                SettingsField::DisplayName => &mut settings_input.display_name,
                SettingsField::RelayAddr => &mut settings_input.relay_addr,
                SettingsField::InviteRecipient => &mut channel_mgmt.invite_recipient,
                SettingsField::JoinCode => &mut channel_mgmt.join_code,
            };
            crate::text_edit::insert_str(target, &mut cursor, &text);
            settings_input.cursor = cursor;
        } else if input.focused {
            let mut cursor = input.cursor;
            crate::text_edit::insert_str(&mut input.text, &mut cursor, &text);
            input.cursor = cursor;
        }
    }
}
