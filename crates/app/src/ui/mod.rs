//! # UI Module
//!
//! Top-level Bevy UI for the Willow chat client.
//!
//! ## Modules
//!
//! - [`components`] — marker components for UI entities
//! - [`constants`] — shared placeholder strings and defaults
//! - [`resources`] — ECS resources (state, config, stores)
//! - [`layout`] — entity spawning for the initial UI hierarchy
//! - [`init`] — server initialization and channel subscription
//! - [`input`] — keyboard input handling and message sending
//! - [`chat`] — network events, message rendering, channel navigation
//! - [`settings`] — settings view systems
//! - [`files`] — file sharing UI

pub mod channels;
pub mod chat;
pub mod components;
pub mod constants;
pub mod files;
pub mod init;
pub mod input;
pub mod layout;
pub mod resources;
pub mod settings;

use bevy::prelude::*;

// Re-export commonly used types so tests and other modules can use
// `crate::ui::ChatState` etc. without reaching into sub-modules.
pub use components::*;
pub use constants::*;
pub use resources::*;

// Re-export specific items needed by tests.
pub use layout::make_topic;

// Re-export system functions used by the headless test app.
pub use chat::handle_network_events;
pub use input::{handle_keyboard_input, send_message};

/// Plugin for all UI systems and resources.
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChatState::default())
            .insert_resource(InputState::default())
            .insert_resource(ChannelKeyStore::default())
            .insert_resource(ServerState::default())
            .insert_resource(AppView::default())
            .insert_resource(SettingsInput::default())
            .insert_resource(ProfileStore::default())
            .insert_resource(FilePicker::default())
            .insert_resource(UnreadCounts::default())
            .insert_resource(SearchFilter::default())
            .insert_resource(MessageDbRes(
                crate::storage::open_message_db()
                    .map(|db| std::sync::Arc::new(std::sync::Mutex::new(db))),
            ))
            .add_systems(Startup, (init::init_server, layout::setup_ui).chain())
            .insert_resource(ChannelManagement::default())
            .add_systems(
                Update,
                (
                    channels::handle_create_channel_button,
                    channels::handle_new_channel_input,
                    channels::handle_delete_channel,
                    channels::sync_new_channel_input,
                    input::handle_keyboard_input,
                    input::send_message,
                    chat::handle_network_events,
                    chat::handle_channel_click,
                    chat::sync_message_list,
                ),
            )
            .add_systems(
                Update,
                (
                    init::subscribe_channels,
                    input::sync_input_text,
                    chat::update_peer_count,
                    chat::update_channel_header,
                    chat::update_channel_highlights,
                    chat::prune_messages,
                ),
            )
            .add_systems(
                Update,
                (
                    settings::handle_settings_button,
                    settings::handle_save_settings,
                    settings::toggle_view,
                    settings::sync_settings_fields,
                    channels::handle_generate_invite,
                    channels::handle_join_server,
                    channels::sync_invite_fields,
                    channels::handle_copy_peer_id,
                    channels::handle_copy_invite,
                    files::handle_share_file_button,
                    files::poll_file_picker,
                ),
            );
    }
}

// ───── Helpers ──────────────────────────────────────────────────────────────

pub fn truncate_peer_id(s: &str) -> String {
    if s.len() > 12 {
        format!("{}...", &s[..12])
    } else {
        s.to_string()
    }
}

/// Format a millisecond timestamp as "HH:MM".
pub fn format_timestamp(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    let secs = ms / 1000;
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    format!("{hours:02}:{minutes:02}")
}
