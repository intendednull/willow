use std::collections::{HashMap, HashSet};

use leptos::prelude::*;
use willow_client::DisplayMessage;

#[derive(Clone, Copy, PartialEq)]
pub enum VideoSource {
    Camera,
    Screen,
}

#[derive(Clone, PartialEq, Default)]
pub enum CallLayout {
    #[default]
    Grid,
    Focus(String), // focused peer_id
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum SettingsTab {
    #[default]
    Profile,
    Server,
}

/// Per-channel UI state. Extensible for future needs (drafts, scroll pos).
#[derive(Clone, Default, PartialEq)]
pub struct ChannelViewState {
    pub typing: Vec<String>,
}

// ── Read signals (provided via context) ──────────────────────────────

#[derive(Clone, Copy)]
pub struct AppState {
    pub chat: ChatState,
    pub network: NetworkState,
    pub server: ServerState,
    pub ui: UiState,
    pub voice: VoiceState,
}

#[derive(Clone, Copy)]
pub struct ChatState {
    pub messages: ReadSignal<Vec<DisplayMessage>>,
    pub current_channel: ReadSignal<String>,
    pub channels: ReadSignal<Vec<String>>,
    pub replying_to: ReadSignal<Option<DisplayMessage>>,
    pub editing: ReadSignal<Option<DisplayMessage>>,
    pub pinned_messages: ReadSignal<Vec<DisplayMessage>>,
    pub pin_labels: ReadSignal<HashMap<String, String>>,
    pub channel_views: ReadSignal<HashMap<String, ChannelViewState>>,
}

#[derive(Clone, Copy)]
pub struct NetworkState {
    pub peers: ReadSignal<Vec<(String, String, bool)>>,
    pub peer_count: ReadSignal<usize>,
    pub peer_id: ReadSignal<String>,
    pub connection_status: ReadSignal<String>,
    pub loading: ReadSignal<bool>,
}

#[derive(Clone, Copy)]
pub struct ServerState {
    pub servers: ReadSignal<Vec<(String, String)>>,
    pub active_server_id: ReadSignal<String>,
    pub active_server_name: ReadSignal<String>,
    pub unread: ReadSignal<HashMap<String, usize>>,
    pub roles: ReadSignal<Vec<(String, String, Vec<String>)>>,
    pub display_name: ReadSignal<String>,
}

#[derive(Clone, Copy)]
pub struct UiState {
    pub show_settings: ReadSignal<bool>,
    #[allow(dead_code)]
    pub show_server_settings: ReadSignal<bool>,
    pub show_sidebar: ReadSignal<bool>,
    pub show_members: ReadSignal<bool>,
    pub show_add_server: ReadSignal<bool>,
    pub show_pinned: ReadSignal<bool>,
    pub show_call_page: ReadSignal<bool>,
    pub show_palette: ReadSignal<bool>,
    pub call_layout: ReadSignal<CallLayout>,
    #[allow(dead_code)]
    pub settings_tab: ReadSignal<SettingsTab>,
}

#[derive(Clone, Copy)]
pub struct VoiceState {
    pub voice_channel: ReadSignal<Option<String>>,
    pub voice_muted: ReadSignal<bool>,
    pub voice_deafened: ReadSignal<bool>,
    /// Participants per voice channel. Not yet rendered but tracked for future use.
    #[allow(dead_code)]
    pub voice_participants_map: ReadSignal<HashMap<String, Vec<String>>>,
    pub voice_channel_name: ReadSignal<String>,
    pub video_source: ReadSignal<Option<VideoSource>>,
    pub speaking_peers: ReadSignal<HashSet<String>>,
    pub remote_video_streams:
        ReadSignal<HashMap<String, send_wrapper::SendWrapper<web_sys::MediaStream>>>,
    /// Local video stream (camera or screen share). Stored globally so it
    /// survives call-page component remounts.
    pub local_video_stream:
        ReadSignal<Option<send_wrapper::SendWrapper<web_sys::MediaStream>>>,
}

// ── Write signals (NOT in context — held by event processing) ────────

#[derive(Clone, Copy)]
pub struct AppWriteSignals {
    pub chat: ChatWriteSignals,
    pub network: NetworkWriteSignals,
    pub server: ServerWriteSignals,
    pub ui: UiWriteSignals,
    pub voice: VoiceWriteSignals,
}

#[derive(Clone, Copy)]
pub struct ChatWriteSignals {
    pub set_messages: WriteSignal<Vec<DisplayMessage>>,
    pub set_current_channel: WriteSignal<String>,
    pub set_channels: WriteSignal<Vec<String>>,
    pub set_replying_to: WriteSignal<Option<DisplayMessage>>,
    pub set_editing: WriteSignal<Option<DisplayMessage>>,
    pub set_pinned_messages: WriteSignal<Vec<DisplayMessage>>,
    pub set_pin_labels: WriteSignal<HashMap<String, String>>,
    pub set_channel_views: WriteSignal<HashMap<String, ChannelViewState>>,
}

#[derive(Clone, Copy)]
pub struct NetworkWriteSignals {
    pub set_peers: WriteSignal<Vec<(String, String, bool)>>,
    pub set_peer_count: WriteSignal<usize>,
    pub set_peer_id: WriteSignal<String>,
    pub set_connection_status: WriteSignal<String>,
    pub set_loading: WriteSignal<bool>,
}

#[derive(Clone, Copy)]
pub struct ServerWriteSignals {
    pub set_servers: WriteSignal<Vec<(String, String)>>,
    pub set_active_server_id: WriteSignal<String>,
    pub set_active_server_name: WriteSignal<String>,
    pub set_unread: WriteSignal<HashMap<String, usize>>,
    pub set_roles: WriteSignal<Vec<(String, String, Vec<String>)>>,
    pub set_display_name: WriteSignal<String>,
}

#[derive(Clone, Copy)]
pub struct UiWriteSignals {
    pub set_show_settings: WriteSignal<bool>,
    pub set_show_server_settings: WriteSignal<bool>,
    pub set_show_sidebar: WriteSignal<bool>,
    pub set_show_members: WriteSignal<bool>,
    pub set_show_add_server: WriteSignal<bool>,
    pub set_show_pinned: WriteSignal<bool>,
    pub set_show_call_page: WriteSignal<bool>,
    pub set_show_palette: WriteSignal<bool>,
    pub set_call_layout: WriteSignal<CallLayout>,
    pub set_settings_tab: WriteSignal<SettingsTab>,
}

#[derive(Clone, Copy)]
pub struct VoiceWriteSignals {
    pub set_voice_channel: WriteSignal<Option<String>>,
    pub set_voice_muted: WriteSignal<bool>,
    pub set_voice_deafened: WriteSignal<bool>,
    pub set_voice_participants_map: WriteSignal<HashMap<String, Vec<String>>>,
    pub set_voice_channel_name: WriteSignal<String>,
    pub set_video_source: WriteSignal<Option<VideoSource>>,
    pub set_speaking_peers: WriteSignal<HashSet<String>>,
    pub set_remote_video_streams:
        WriteSignal<HashMap<String, send_wrapper::SendWrapper<web_sys::MediaStream>>>,
    pub set_local_video_stream:
        WriteSignal<Option<send_wrapper::SendWrapper<web_sys::MediaStream>>>,
}

/// Create all signal pairs and return the read/write halves.
pub fn create_signals() -> (AppState, AppWriteSignals) {
    // Chat signals
    let (messages, set_messages) = signal(Vec::<DisplayMessage>::new());
    let (current_channel, set_current_channel) = signal(String::from("general"));
    let (channels, set_channels) = signal(Vec::<String>::new());
    let (replying_to, set_replying_to) = signal(Option::<DisplayMessage>::None);
    let (editing, set_editing) = signal(Option::<DisplayMessage>::None);
    let (pinned_messages, set_pinned_messages) = signal(Vec::<DisplayMessage>::new());
    let (pin_labels, set_pin_labels) = signal(HashMap::<String, String>::new());
    let (channel_views, set_channel_views) = signal(HashMap::<String, ChannelViewState>::new());

    // Network signals
    let (peers, set_peers) = signal(Vec::<(String, String, bool)>::new());
    let (peer_count, set_peer_count) = signal(0usize);
    let (peer_id, set_peer_id) = signal(String::new());
    let (connection_status, set_connection_status) = signal("connecting".to_string());
    let (loading, set_loading) = signal(true);

    // Server signals
    let (servers, set_servers) = signal(Vec::<(String, String)>::new());
    let (active_server_id, set_active_server_id) = signal(String::new());
    let (active_server_name, set_active_server_name) = signal(String::new());
    let (unread, set_unread) = signal(HashMap::<String, usize>::new());
    let (roles, set_roles) = signal(Vec::<(String, String, Vec<String>)>::new());
    let (display_name, set_display_name) = signal(String::new());

    // UI panel signals
    let (show_settings, set_show_settings) = signal(false);
    let (show_server_settings, set_show_server_settings) = signal(false);
    let (show_sidebar, set_show_sidebar) = signal(false);
    let (show_members, set_show_members) = signal(false);
    let (show_add_server, set_show_add_server) = signal(false);
    let (show_pinned, set_show_pinned) = signal(false);
    let (show_call_page, set_show_call_page) = signal(false);
    let (show_palette, set_show_palette) = signal(false);
    let (call_layout, set_call_layout) = signal(CallLayout::default());
    let (settings_tab, set_settings_tab) = signal(SettingsTab::default());

    // Voice signals
    let (voice_channel, set_voice_channel) = signal(Option::<String>::None);
    let (voice_muted, set_voice_muted) = signal(false);
    let (voice_deafened, set_voice_deafened) = signal(false);
    let (voice_participants_map, set_voice_participants_map) =
        signal(HashMap::<String, Vec<String>>::new());
    let (voice_channel_name, set_voice_channel_name) = signal(String::new());
    let (video_source, set_video_source) = signal(Option::<VideoSource>::None);
    let (speaking_peers, set_speaking_peers) = signal(HashSet::<String>::new());
    let (remote_video_streams, set_remote_video_streams) = signal(HashMap::<
        String,
        send_wrapper::SendWrapper<web_sys::MediaStream>,
    >::new());
    let (local_video_stream, set_local_video_stream) = signal(
        Option::<send_wrapper::SendWrapper<web_sys::MediaStream>>::None,
    );

    let app_state = AppState {
        chat: ChatState {
            messages,
            current_channel,
            channels,
            replying_to,
            editing,
            pinned_messages,
            pin_labels,
            channel_views,
        },
        network: NetworkState {
            peers,
            peer_count,
            peer_id,
            connection_status,
            loading,
        },
        server: ServerState {
            servers,
            active_server_id,
            active_server_name,
            unread,
            roles,
            display_name,
        },
        ui: UiState {
            show_settings,
            show_server_settings,
            show_sidebar,
            show_members,
            show_add_server,
            show_pinned,
            show_call_page,
            show_palette,
            call_layout,
            settings_tab,
        },
        voice: VoiceState {
            voice_channel,
            voice_muted,
            voice_deafened,
            voice_participants_map,
            voice_channel_name,
            video_source,
            speaking_peers,
            remote_video_streams,
            local_video_stream,
        },
    };

    let write_signals = AppWriteSignals {
        chat: ChatWriteSignals {
            set_messages,
            set_current_channel,
            set_channels,
            set_replying_to,
            set_editing,
            set_pinned_messages,
            set_pin_labels,
            set_channel_views,
        },
        network: NetworkWriteSignals {
            set_peers,
            set_peer_count,
            set_peer_id,
            set_connection_status,
            set_loading,
        },
        server: ServerWriteSignals {
            set_servers,
            set_active_server_id,
            set_active_server_name,
            set_unread,
            set_roles,
            set_display_name,
        },
        ui: UiWriteSignals {
            set_show_settings,
            set_show_server_settings,
            set_show_sidebar,
            set_show_members,
            set_show_add_server,
            set_show_pinned,
            set_show_call_page,
            set_show_palette,
            set_call_layout,
            set_settings_tab,
        },
        voice: VoiceWriteSignals {
            set_voice_channel,
            set_voice_muted,
            set_voice_deafened,
            set_voice_participants_map,
            set_voice_channel_name,
            set_video_source,
            set_speaking_peers,
            set_remote_video_streams,
            set_local_video_stream,
        },
    };

    (app_state, write_signals)
}
