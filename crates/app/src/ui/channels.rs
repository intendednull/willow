//! Channel management systems — create, delete, and sidebar updates.

use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use crate::network_bridge::NetworkCommandSender;
use crate::theme;
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

fn create_channel(
    name: &str,
    server_state: &mut ResMut<ServerState>,
    key_store: &mut ResMut<ChannelKeyStore>,
    state: &mut ResMut<ChatState>,
    net_cmd: &Res<NetworkCommandSender>,
    commands: &mut Commands,
    list_query: &Query<Entity, With<ChannelList>>,
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
    server_state
        .topic_map
        .insert(topic.clone(), (name.to_string(), ch_id));

    // Subscribe to the new topic.
    let _ = net_cmd
        .0
        .send(crate::network_bridge::NetworkBridgeCommand::Subscribe(
            topic,
        ));

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
