//! Settings view systems — toggle, save, field sync.

use bevy::prelude::*;

use crate::network_bridge::{ConnectCommand, LocalIdentity, NetworkCommandSender};
use crate::theme;

use super::components::*;
use super::constants;
use super::resources::*;

/// Toggle between Chat and Settings when the settings button is clicked.
pub fn handle_settings_button(
    query: Query<&Interaction, (Changed<Interaction>, With<SettingsButton>)>,
    mut view: ResMut<AppView>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            *view = match *view {
                AppView::Chat => AppView::Settings,
                AppView::Settings => AppView::Chat,
            };
        }
    }
}

/// Handle "Save & Reconnect" button in settings.
#[allow(clippy::too_many_arguments)]
pub fn handle_save_settings(
    query: Query<&Interaction, (Changed<Interaction>, With<SaveSettingsButton>)>,
    settings_input: Res<SettingsInput>,
    mut connect_writer: MessageWriter<ConnectCommand>,
    mut view: ResMut<AppView>,
    mut profiles: ResMut<ProfileStore>,
    identity: Res<LocalIdentity>,
    net_cmd: Res<NetworkCommandSender>,
    mut user_display_query: Query<&mut Text, With<LocalUserDisplay>>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            let name = settings_input.display_name.trim().to_string();
            crate::storage::save_profile(&crate::storage::LocalProfile {
                display_name: name.clone(),
            });

            let peer_id_str = identity.0.peer_id().to_string();
            let display = if name.is_empty() {
                super::truncate_peer_id(&peer_id_str)
            } else {
                name
            };
            profiles.names.insert(peer_id_str, display.clone());

            for mut text in &mut user_display_query {
                **text = display.clone();
            }

            let relay = if settings_input.relay_addr.trim().is_empty() {
                None
            } else {
                Some(settings_input.relay_addr.trim().to_string())
            };

            // Broadcast profile to peers.
            if !display.is_empty() {
                let _ = net_cmd.0.send(
                    crate::network_bridge::NetworkBridgeCommand::BroadcastProfile {
                        display_name: display.clone(),
                    },
                );
            }

            connect_writer.write(ConnectCommand { relay_addr: relay });
            *view = AppView::Chat;
            info!("settings saved, reconnecting");
        }
    }
}

/// Show/hide chat and settings panels based on current view.
#[allow(clippy::type_complexity)]
pub fn toggle_view(
    view: Res<AppView>,
    mut chat_query: Query<&mut Node, (With<ChatPanel>, Without<SettingsPanel>)>,
    mut settings_query: Query<&mut Node, (With<SettingsPanel>, Without<ChatPanel>)>,
    mut settings_input: ResMut<SettingsInput>,
) {
    if !view.is_changed() {
        return;
    }

    for mut node in &mut chat_query {
        node.display = if *view == AppView::Chat {
            Display::Flex
        } else {
            Display::None
        };
    }

    for mut node in &mut settings_query {
        node.display = if *view == AppView::Settings {
            Display::Flex
        } else {
            Display::None
        };
    }

    // When entering settings, reload saved values and reset cursor.
    if *view == AppView::Settings {
        let saved = crate::storage::load_settings().unwrap_or_default();
        settings_input.relay_addr = saved.relay_addr.unwrap_or_default();
        let saved_profile = crate::storage::load_profile().unwrap_or_default();
        settings_input.display_name = saved_profile.display_name;
        // Place cursor at end of the focused field.
        let focused_text = match settings_input.focused_field {
            SettingsField::DisplayName => &settings_input.display_name,
            SettingsField::RelayAddr => &settings_input.relay_addr,
            _ => "",
        };
        settings_input.cursor = crate::text_edit::char_len(focused_text);
    }
}

/// Handle clicks on settings input fields to focus them.
pub fn handle_settings_field_click(
    query: Query<(&Interaction, &SettingsFieldContainer), Changed<Interaction>>,
    mut settings_input: ResMut<SettingsInput>,
    channel_mgmt: Res<ChannelManagement>,
) {
    for (interaction, container) in &query {
        if *interaction == Interaction::Pressed {
            settings_input.focused_field = container.0;
            // Place cursor at end of the new field.
            let field_text = match container.0 {
                SettingsField::DisplayName => &settings_input.display_name,
                SettingsField::RelayAddr => &settings_input.relay_addr,
                SettingsField::InviteRecipient => &channel_mgmt.invite_recipient,
                SettingsField::JoinCode => &channel_mgmt.join_code,
            };
            settings_input.cursor = crate::text_edit::char_len(field_text);
        }
    }
}

/// Reactively sync the settings text fields and focus indicator from SettingsInput.
#[allow(clippy::type_complexity)]
pub fn sync_settings_fields(
    settings_input: Res<SettingsInput>,
    mut name_query: Query<
        (&mut Text, &mut TextColor),
        (
            With<SettingsNameText>,
            Without<InputText>,
            Without<SettingsRelayText>,
        ),
    >,
    mut relay_query: Query<
        (&mut Text, &mut TextColor),
        (
            With<SettingsRelayText>,
            Without<InputText>,
            Without<SettingsNameText>,
        ),
    >,
    mut container_query: Query<(&SettingsFieldContainer, &mut BorderColor)>,
) {
    if !settings_input.is_changed() {
        return;
    }

    // Update text values, showing cursor in the focused field.
    let name_focused = settings_input.focused_field == SettingsField::DisplayName;
    for (mut text, mut color) in &mut name_query {
        if settings_input.display_name.is_empty() {
            let cursor_str = if name_focused { "\u{2581}" } else { "" };
            **text = format!("{}{cursor_str}", constants::NAME_PLACEHOLDER);
            *color = TextColor(theme::TEXT_PLACEHOLDER);
        } else if name_focused {
            let (before, after) = crate::text_edit::split_at_cursor(
                &settings_input.display_name,
                settings_input.cursor,
            );
            **text = format!("{before}\u{2581}{after}");
            *color = TextColor(theme::TEXT_PRIMARY);
        } else {
            **text = settings_input.display_name.clone();
            *color = TextColor(theme::TEXT_PRIMARY);
        }
    }

    let relay_focused = settings_input.focused_field == SettingsField::RelayAddr;
    for (mut text, mut color) in &mut relay_query {
        if settings_input.relay_addr.is_empty() {
            let cursor_str = if relay_focused { "\u{2581}" } else { "" };
            **text = format!("{}{cursor_str}", constants::RELAY_PLACEHOLDER);
            *color = TextColor(theme::TEXT_PLACEHOLDER);
        } else if relay_focused {
            let (before, after) = crate::text_edit::split_at_cursor(
                &settings_input.relay_addr,
                settings_input.cursor,
            );
            **text = format!("{before}\u{2581}{after}");
            *color = TextColor(theme::TEXT_PRIMARY);
        } else {
            **text = settings_input.relay_addr.clone();
            *color = TextColor(theme::TEXT_PRIMARY);
        }
    }

    // Highlight the focused field container with a border.
    for (container, mut border) in &mut container_query {
        *border = if container.0 == settings_input.focused_field {
            BorderColor::all(theme::ACCENT)
        } else {
            BorderColor::all(Color::NONE)
        };
    }
}
