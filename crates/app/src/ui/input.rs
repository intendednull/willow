//! Keyboard input handling and message sending.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use crate::network_bridge::{LocalIdentity, NetworkCommandSender};
use crate::theme;
use willow_messaging::{Content, Message};
use willow_transport::{pack_envelope, MessageType};

use super::components::*;
use super::constants;
use super::resources::*;

/// Capture keyboard input and route to the appropriate field based on view.
///
/// This system only mutates resources (`InputState`, `SettingsInput`).
/// UI text updates are handled reactively by `sync_input_text` and
/// `sync_settings_fields`.
#[allow(clippy::type_complexity)]
pub fn handle_keyboard_input(
    mut key_events: MessageReader<KeyboardInput>,
    mut input: ResMut<InputState>,
    mut view: ResMut<AppView>,
    mut settings_input: ResMut<SettingsInput>,
    mut search: ResMut<SearchFilter>,
    keys: Res<ButtonInput<KeyCode>>,
    channel_mgmt: Res<ChannelManagement>,
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
                        SettingsField::RelayAddr => SettingsField::DisplayName,
                    };
                }
                KeyCode::Backspace => {
                    let target = match settings_input.focused_field {
                        SettingsField::DisplayName => &mut settings_input.display_name,
                        SettingsField::RelayAddr => &mut settings_input.relay_addr,
                    };
                    target.pop();
                }
                KeyCode::Escape => {
                    *view = AppView::Chat;
                }
                _ => {
                    if let Some(ref s) = event.text {
                        let target = match settings_input.focused_field {
                            SettingsField::DisplayName => &mut settings_input.display_name,
                            SettingsField::RelayAddr => &mut settings_input.relay_addr,
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
    if body.is_empty() {
        return;
    }

    let channel_name = state.current_channel.clone();
    let peer_id = identity.0.peer_id();

    let topic = match server_state.topic_for_name(&channel_name) {
        Some(t) => t,
        None => channel_name.clone(),
    };

    let channel_id = willow_messaging::ChannelId::new();
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
    let chat_msg = ChatMessage {
        topic,
        author: author.clone(),
        body: body.clone(),
        is_local: true,
        timestamp_ms: ts,
    };

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
        if input.text.is_empty() {
            **text = constants::CHAT_PLACEHOLDER.to_string();
            *color = TextColor(theme::TEXT_PLACEHOLDER);
        } else {
            **text = input.text.clone();
            *color = TextColor(theme::TEXT_PRIMARY);
        }
    }
}
