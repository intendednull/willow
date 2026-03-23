//! Bevy resources for the Willow UI.

use bevy::prelude::*;
use std::collections::HashMap;

use willow_channel::Server;
use willow_crypto::ChannelKey;
use willow_messaging::hlc::HLC;

use super::constants;

/// The local server instance. Each peer auto-creates a server on first launch.
#[derive(Resource, Default)]
pub struct ServerState {
    pub server: Option<Server>,
    /// Maps gossipsub topic → (channel_name, channel_id) for display + key lookup.
    pub topic_map: HashMap<String, (String, willow_channel::ChannelId)>,
}

impl ServerState {
    /// Get the gossipsub topic for a channel by name.
    pub fn topic_for_name(&self, name: &str) -> Option<String> {
        self.topic_map
            .iter()
            .find(|(_, (n, _))| n == name)
            .map(|(topic, _)| topic.clone())
    }

    /// Get the channel name for a gossipsub topic.
    #[allow(dead_code)]
    pub fn name_for_topic(&self, topic: &str) -> Option<&str> {
        self.topic_map.get(topic).map(|(name, _)| name.as_str())
    }

    /// List all channel names in sidebar order.
    pub fn channel_names(&self) -> Vec<String> {
        let Some(server) = &self.server else {
            return Vec::new();
        };
        let mut names: Vec<_> = server.channels().iter().map(|ch| ch.name.clone()).collect();
        names.sort();
        names
    }
}

#[derive(Resource)]
pub struct ChatState {
    pub messages: Vec<ChatMessage>,
    /// The current channel *name* (human-readable, e.g. "general").
    pub current_channel: String,
    pub peers: Vec<String>,
    pub hlc: HLC,
    pub messages_dirty: bool,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            current_channel: constants::DEFAULT_CHANNEL.to_string(),
            peers: Vec::new(),
            hlc: HLC::new(),
            messages_dirty: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// The gossipsub topic this message belongs to.
    pub topic: String,
    pub author: String,
    pub body: String,
    pub is_local: bool,
    /// HLC timestamp in milliseconds (for display).
    pub timestamp_ms: u64,
}

/// Tracks unread message counts per channel topic.
#[derive(Resource, Default)]
pub struct UnreadCounts {
    pub counts: HashMap<String, usize>,
}

#[derive(Resource, Default)]
pub struct InputState {
    pub text: String,
    pub send_requested: bool,
}

/// Per-channel symmetric encryption keys, keyed by gossipsub topic.
#[derive(Resource, Default)]
pub struct ChannelKeyStore {
    pub keys: HashMap<String, ChannelKey>,
}

/// Persistent message database.
#[derive(Resource, Clone)]
pub struct MessageDbRes(pub Option<std::sync::Arc<std::sync::Mutex<crate::storage::MessageDb>>>);

/// Maps PeerId strings → display names. Updated from profile broadcasts.
#[derive(Resource, Default, Clone)]
pub struct ProfileStore {
    pub names: HashMap<String, String>,
}

impl ProfileStore {
    /// Look up a display name for a peer, falling back to truncated ID.
    pub fn display_name(&self, peer_id: &str) -> String {
        self.names
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| super::truncate_peer_id(peer_id))
    }
}

/// Which view is currently active.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub enum AppView {
    #[default]
    Chat,
    Settings,
}

/// Editable settings state.
#[derive(Resource)]
pub struct SettingsInput {
    pub relay_addr: String,
    pub display_name: String,
    /// Which field is currently focused in settings.
    pub focused_field: SettingsField,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    #[default]
    DisplayName,
    RelayAddr,
}

impl Default for SettingsInput {
    fn default() -> Self {
        let saved_settings = crate::storage::load_settings().unwrap_or_default();
        let saved_profile = crate::storage::load_profile().unwrap_or_default();
        Self {
            relay_addr: saved_settings.relay_addr.unwrap_or_default(),
            display_name: saved_profile.display_name,
            focused_field: SettingsField::DisplayName,
        }
    }
}

/// File data from the picker: (filename, mime_type, data).
type FilePickerResult = (String, String, Vec<u8>);

/// Tracks pending file picker operations.
#[derive(Resource, Clone)]
pub struct FilePicker {
    pub rx: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Receiver<FilePickerResult>>>>,
}

impl Default for FilePicker {
    fn default() -> Self {
        Self {
            rx: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }
}
