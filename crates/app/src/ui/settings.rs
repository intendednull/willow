//! Settings view systems — toggle, save, field sync.

use bevy::prelude::*;

use crate::network_bridge::{ConnectCommand, LocalIdentity};
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
pub fn handle_save_settings(
    query: Query<&Interaction, (Changed<Interaction>, With<SaveSettingsButton>)>,
    settings_input: Res<SettingsInput>,
    mut connect_writer: MessageWriter<ConnectCommand>,
    mut view: ResMut<AppView>,
    mut profiles: ResMut<ProfileStore>,
    identity: Res<LocalIdentity>,
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

    // When entering settings, reload saved values.
    if *view == AppView::Settings {
        let saved = crate::storage::load_settings().unwrap_or_default();
        settings_input.relay_addr = saved.relay_addr.unwrap_or_default();
        let saved_profile = crate::storage::load_profile().unwrap_or_default();
        settings_input.display_name = saved_profile.display_name;
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

    // Update text values.
    for (mut text, mut color) in &mut name_query {
        if settings_input.display_name.is_empty() {
            **text = constants::NAME_PLACEHOLDER.to_string();
            *color = TextColor(theme::TEXT_PLACEHOLDER);
        } else {
            **text = settings_input.display_name.clone();
            *color = TextColor(theme::TEXT_PRIMARY);
        }
    }

    for (mut text, mut color) in &mut relay_query {
        if settings_input.relay_addr.is_empty() {
            **text = constants::RELAY_PLACEHOLDER.to_string();
            *color = TextColor(theme::TEXT_PLACEHOLDER);
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
