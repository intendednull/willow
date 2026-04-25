use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use leptos::prelude::*;
use willow_client::trust::{PeerTrust, TrustStoreHandle};
use willow_client::DisplayMessage;

use crate::trust_store::WebTrustStore;

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
    Roles,
    Presence,
    /// Per-identity notification preferences (phase 1f placeholder).
    Notifications,
}

/// Per-grove crypto-visibility tweak.
///
/// See `docs/specs/2026-04-19-ui-design/trust-verification.md` §Per-grove
/// crypto-visibility setting. Controls how prominently the channel-key
/// holder pill and related metadata surface in the channel header.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum CryptoVisibility {
    /// Holder pill visible only when the holder count is *less than*
    /// the grove's member count.
    Subtle,
    /// Holder pill always visible. Default.
    #[default]
    Default,
    /// Holder pill visible plus a one-line crypto strip below the header.
    Explicit,
}

/// Feature flag: is the `not sure` CTA wired in the compare dialog? Per
/// the implementation plan's ambiguity decisions this ships **off** in
/// Phase 1d and the slot is reserved for a future change.
pub const V1_ALLOW_UNSURE_CTA: bool = false;

/// Per-channel UI state. Extensible for future needs (drafts, scroll pos).
#[derive(Clone, Default, PartialEq)]
pub struct ChannelViewState {
    pub typing: Vec<String>,
}

/// Parsed join token data for the UI.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedJoinToken {
    /// Original base64 for re-encoding.
    pub raw: String,
    pub link_id: String,
    pub server_name: String,
    pub inviter_name: String,
}

// ── Read signals (provided via context) ──────────────────────────────

#[derive(Clone, Copy)]
pub struct AppState {
    pub chat: ChatState,
    pub network: NetworkState,
    pub server: ServerState,
    pub ui: UiState,
    pub voice: VoiceState,
    pub trust: TrustState,
    pub presence: PresenceUiState,
    pub profile: ProfileUiState,
}

/// Reactive profile-card bucket. `open` carries the currently-visible
/// profile card's state (merged view + anchor). `None` means "closed".
#[derive(Clone, Copy)]
pub struct ProfileUiState {
    pub open: ReadSignal<Option<crate::profile::ProfileState>>,
}

/// Reactive presence bucket. `per_peer` maps a peer's string id to the
/// derived [`willow_client::presence::PresenceState`]. `self_state`
/// carries the local user's own label (respects the override). Both
/// are reactive — UI surfaces subscribe directly without needing to
/// round-trip through the client handle.
#[derive(Clone, Copy)]
pub struct PresenceUiState {
    pub per_peer: ReadSignal<HashMap<String, willow_client::presence::PresenceState>>,
    pub self_state: ReadSignal<willow_client::presence::PresenceState>,
    pub self_override: ReadSignal<willow_client::presence::PresenceOverride>,
}

/// Reactive trust bucket. The `trust_map` signal mirrors the
/// [`TrustStoreHandle`] snapshot and rebuilds on every `set` via the
/// version token. `compare_target` drives the root-mounted
/// `<AddFriendDialog>`: `None` closed, `Some(peer_id)` open for the
/// given peer (including the self-peer, which renders a single card).
#[derive(Clone, Copy)]
pub struct TrustState {
    /// Map of peer-id → current trust belief.
    pub trust_map: ReadSignal<HashMap<String, PeerTrust>>,
    /// Incrementing token — bumps when the underlying
    /// [`TrustStoreHandle`] mutates. UIs don't read this directly, but
    /// it exists as a debug handle for tests.
    pub version: ReadSignal<u64>,
    /// When `Some`, the compare-fingerprints dialog is open for that
    /// peer. `None` closes it.
    pub compare_target: ReadSignal<Option<String>>,
    /// Per-grove crypto-visibility mode. Drives holder-pill + crypto
    /// strip rendering.
    pub crypto_visibility: ReadSignal<CryptoVisibility>,
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
    /// Per-surface unread stats (phase 1f) — drives the badge priority
    /// pipeline. Keyed by channel name for `SurfaceId::Channel`; other
    /// variants are stubbed empty until their phases land.
    pub unread_stats: ReadSignal<HashMap<String, willow_client::views::UnreadStats>>,
    pub roles: ReadSignal<Vec<(String, String, Vec<String>)>>,
    pub display_name: ReadSignal<String>,
    pub server_owner: ReadSignal<String>,
    pub channel_kinds: ReadSignal<Vec<(String, willow_state::ChannelKind)>>,
    /// Peer IDs that have the SyncProvider permission.
    pub sync_provider_ids: ReadSignal<HashSet<String>>,
    /// Peer IDs that have the Administrator permission.
    pub admin_ids: ReadSignal<HashSet<String>>,
}

#[derive(Clone, Copy)]
pub struct UiState {
    pub show_settings: ReadSignal<bool>,
    pub show_sidebar: ReadSignal<bool>,
    pub show_members: ReadSignal<bool>,
    pub show_add_server: ReadSignal<bool>,
    pub show_pinned: ReadSignal<bool>,
    pub show_call_page: ReadSignal<bool>,
    pub show_palette: ReadSignal<bool>,
    pub call_layout: ReadSignal<CallLayout>,
    #[allow(dead_code)]
    pub settings_tab: ReadSignal<SettingsTab>,
    pub join_token: ReadSignal<Option<ParsedJoinToken>>,
    /// "", "connecting", or "denied:<reason>".
    pub join_status: ReadSignal<String>,
}

#[derive(Clone, Copy)]
pub struct VoiceState {
    pub voice_channel: ReadSignal<Option<String>>,
    pub voice_muted: ReadSignal<bool>,
    pub voice_deafened: ReadSignal<bool>,
    /// Participants per voice channel.
    #[allow(dead_code)]
    pub voice_participants_map: ReadSignal<HashMap<String, Vec<String>>>,
    pub voice_channel_name: ReadSignal<String>,
    pub video_source: ReadSignal<Option<VideoSource>>,
    pub speaking_peers: ReadSignal<HashSet<String>>,
    pub remote_video_streams:
        ReadSignal<HashMap<String, send_wrapper::SendWrapper<web_sys::MediaStream>>>,
    pub local_video_stream: ReadSignal<Option<send_wrapper::SendWrapper<web_sys::MediaStream>>>,
}

// ── Write signals (for local UI state only) ─────────────────────────

#[derive(Clone, Copy)]
pub struct AppWriteSignals {
    pub chat: ChatWriteSignals,
    pub network: NetworkWriteSignals,
    pub server: ServerWriteSignals,
    pub ui: UiWriteSignals,
    pub voice: VoiceWriteSignals,
    pub trust: TrustWriteSignals,
    pub presence: PresenceWriteSignals,
    pub profile: ProfileWriteSignals,
}

#[derive(Clone, Copy)]
pub struct ProfileWriteSignals {
    pub set_open: WriteSignal<Option<crate::profile::ProfileState>>,
}

#[derive(Clone, Copy)]
pub struct PresenceWriteSignals {
    pub set_per_peer: WriteSignal<HashMap<String, willow_client::presence::PresenceState>>,
    pub set_self_state: WriteSignal<willow_client::presence::PresenceState>,
    pub set_self_override: WriteSignal<willow_client::presence::PresenceOverride>,
}

#[derive(Clone, Copy)]
pub struct TrustWriteSignals {
    pub set_trust_map: WriteSignal<HashMap<String, PeerTrust>>,
    pub set_version: WriteSignal<u64>,
    pub set_compare_target: WriteSignal<Option<String>>,
    pub set_crypto_visibility: WriteSignal<CryptoVisibility>,
}

#[derive(Clone, Copy)]
pub struct ChatWriteSignals {
    pub set_messages: WriteSignal<Vec<DisplayMessage>>,
    pub set_current_channel: WriteSignal<String>,
    pub set_channels: WriteSignal<Vec<String>>,
    pub set_replying_to: WriteSignal<Option<DisplayMessage>>,
    pub set_editing: WriteSignal<Option<DisplayMessage>>,
    #[allow(dead_code)]
    pub set_pinned_messages: WriteSignal<Vec<DisplayMessage>>,
    #[allow(dead_code)]
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
    pub set_unread_stats: WriteSignal<HashMap<String, willow_client::views::UnreadStats>>,
    pub set_roles: WriteSignal<Vec<(String, String, Vec<String>)>>,
    pub set_display_name: WriteSignal<String>,
    pub set_server_owner: WriteSignal<String>,
    pub set_channel_kinds: WriteSignal<Vec<(String, willow_state::ChannelKind)>>,
    pub set_sync_provider_ids: WriteSignal<HashSet<String>>,
    pub set_admin_ids: WriteSignal<HashSet<String>>,
}

#[derive(Clone, Copy)]
pub struct UiWriteSignals {
    pub set_show_settings: WriteSignal<bool>,
    pub set_show_sidebar: WriteSignal<bool>,
    pub set_show_members: WriteSignal<bool>,
    pub set_show_add_server: WriteSignal<bool>,
    pub set_show_pinned: WriteSignal<bool>,
    pub set_show_call_page: WriteSignal<bool>,
    pub set_show_palette: WriteSignal<bool>,
    pub set_call_layout: WriteSignal<CallLayout>,
    pub set_settings_tab: WriteSignal<SettingsTab>,
    pub set_join_token: WriteSignal<Option<ParsedJoinToken>>,
    pub set_join_status: WriteSignal<String>,
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

impl VoiceWriteSignals {
    /// Reset all voice-related UI signals to their defaults: no active
    /// voice channel, unmuted/undeafened, no local or remote media
    /// streams, no speaking peers, and an empty participants map.
    ///
    /// Used when leaving a voice channel, switching between channels,
    /// and when microphone permission is denied.
    pub fn reset(&self) {
        self.set_voice_channel.set(None);
        self.set_voice_channel_name.set(String::new());
        self.set_voice_muted.set(false);
        self.set_voice_deafened.set(false);
        self.set_video_source.set(None);
        self.set_local_video_stream.set(None);
        self.set_remote_video_streams.update(|m| m.clear());
        self.set_speaking_peers.set(HashSet::new());
        self.set_voice_participants_map.update(|m| m.clear());
    }
}

/// Bundle returned from [`create_signals`] — read/write halves of every
/// reactive signal plus the trust-store handle the app boots with.
pub struct InitialSignals {
    pub app_state: AppState,
    pub write: AppWriteSignals,
    /// The authoritative trust store. Cloned into [`ClientHandle::with_trust_store`]
    /// and into the effect that syncs `trust_map` on every mutation.
    pub trust_store: TrustStoreHandle,
}

/// Create all signal pairs, load the trust store, and return the
/// [`InitialSignals`] bundle.
///
/// Signals that reflect `SharedState` are created via `derived_signal()` when
/// a state actor is available; otherwise they fall back to regular signals
/// that are updated via `refresh_all_signals()`.
pub fn create_signals() -> InitialSignals {
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
    let (unread_stats, set_unread_stats) =
        signal(HashMap::<String, willow_client::views::UnreadStats>::new());
    let (roles, set_roles) = signal(Vec::<(String, String, Vec<String>)>::new());
    let (display_name, set_display_name) = signal(String::new());
    let (server_owner, set_server_owner) = signal(String::new());
    let (channel_kinds, set_channel_kinds) =
        signal(Vec::<(String, willow_state::ChannelKind)>::new());
    let (sync_provider_ids, set_sync_provider_ids) = signal(HashSet::<String>::new());
    let (admin_ids, set_admin_ids) = signal(HashSet::<String>::new());

    // UI panel signals (purely local — never derived)
    let (show_settings, set_show_settings) = signal(false);
    let (show_sidebar, set_show_sidebar) = signal(false);
    let (show_members, set_show_members) = signal(false);
    let (show_add_server, set_show_add_server) = signal(false);
    let (show_pinned, set_show_pinned) = signal(false);
    let (show_call_page, set_show_call_page) = signal(false);
    let (show_palette, set_show_palette) = signal(false);
    let (call_layout, set_call_layout) = signal(CallLayout::default());
    let (settings_tab, set_settings_tab) = signal(SettingsTab::default());
    let (join_token, set_join_token) = signal(Option::<ParsedJoinToken>::None);
    let (join_status, set_join_status) = signal(String::new());

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
    let (local_video_stream, set_local_video_stream) =
        signal(Option::<send_wrapper::SendWrapper<web_sys::MediaStream>>::None);

    // Trust: boot the localStorage-backed store and seed signals from
    // its snapshot. The root `<App>` wires an Effect that copies every
    // version bump back into `trust_map`.
    let trust_store: TrustStoreHandle = Arc::new(WebTrustStore::load());
    let initial_trust: HashMap<String, PeerTrust> = trust_store.snapshot().into_iter().collect();
    let (trust_map, set_trust_map) = signal(initial_trust);
    let (trust_version, set_trust_version) = signal(trust_store.version());
    let (compare_target, set_compare_target) = signal(Option::<String>::None);
    let (crypto_visibility, set_crypto_visibility) = signal(CryptoVisibility::default());

    // Presence signals (phase 1e)
    let (presence_per_peer, set_presence_per_peer) =
        signal(HashMap::<String, willow_client::presence::PresenceState>::new());
    let (presence_self_state, set_presence_self_state) =
        signal(willow_client::presence::PresenceState::Here);
    let (presence_self_override, set_presence_self_override) =
        signal(willow_client::presence::PresenceOverride::Auto);

    // Profile-card signals (phase 2c)
    let (profile_open, set_profile_open) = signal(Option::<crate::profile::ProfileState>::None);

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
            unread_stats,
            roles,
            display_name,
            server_owner,
            channel_kinds,
            sync_provider_ids,
            admin_ids,
        },
        ui: UiState {
            show_settings,
            show_sidebar,
            show_members,
            show_add_server,
            show_pinned,
            show_call_page,
            show_palette,
            call_layout,
            settings_tab,
            join_token,
            join_status,
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
        trust: TrustState {
            trust_map,
            version: trust_version,
            compare_target,
            crypto_visibility,
        },
        presence: PresenceUiState {
            per_peer: presence_per_peer,
            self_state: presence_self_state,
            self_override: presence_self_override,
        },
        profile: ProfileUiState { open: profile_open },
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
            set_unread_stats,
            set_roles,
            set_display_name,
            set_server_owner,
            set_channel_kinds,
            set_sync_provider_ids,
            set_admin_ids,
        },
        ui: UiWriteSignals {
            set_show_settings,
            set_show_sidebar,
            set_show_members,
            set_show_add_server,
            set_show_pinned,
            set_show_call_page,
            set_show_palette,
            set_call_layout,
            set_settings_tab,
            set_join_token,
            set_join_status,
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
        trust: TrustWriteSignals {
            set_trust_map,
            set_version: set_trust_version,
            set_compare_target,
            set_crypto_visibility,
        },
        presence: PresenceWriteSignals {
            set_per_peer: set_presence_per_peer,
            set_self_state: set_presence_self_state,
            set_self_override: set_presence_self_override,
        },
        profile: ProfileWriteSignals {
            set_open: set_profile_open,
        },
    };

    InitialSignals {
        app_state,
        write: write_signals,
        trust_store,
    }
}

/// Wire up derived signals that auto-update from the state actor.
///
/// Call this after the actor system is initialized. These derived signals
/// replace the manual `refresh_all_signals()` / `process_event_batch()` flow.
/// Each signal only re-renders when its selected value actually changes.
pub fn wire_derived_signals<N: willow_network::Network>(
    handle: &willow_client::ClientHandle<N>,
    system: &willow_actor::SystemHandle,
    write: &AppWriteSignals,
) {
    let write = *write;
    use crate::state_bridge::derived_signal;
    let views = handle.views();

    // ── Server-derived signals ──────────────────────────────────────
    let servers = derived_signal(&views.server_registry, system, |reg| reg.server_list());
    leptos::prelude::Effect::new(move || write.server.set_servers.set(servers.get()));

    let active_id = derived_signal(&views.server_registry, system, |reg| {
        reg.active_server.clone().unwrap_or_default()
    });
    leptos::prelude::Effect::new(move || write.server.set_active_server_id.set(active_id.get()));

    let active_name = derived_signal(&views.server_registry, system, |reg| {
        reg.active()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "No Server".to_string())
    });
    leptos::prelude::Effect::new(move || {
        write.server.set_active_server_name.set(active_name.get())
    });

    let local_pid = handle.identity().endpoint_id();
    let display_name = derived_signal(&views.profiles, system, move |p| p.display_name(&local_pid));
    leptos::prelude::Effect::new(move || write.server.set_display_name.set(display_name.get()));

    let channels_sig = derived_signal(&views.channels, system, |cv| {
        cv.channels
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
    });
    leptos::prelude::Effect::new(move || write.chat.set_channels.set(channels_sig.get()));

    let unread = derived_signal(&views.server_registry, system, |reg| {
        reg.active().map(|e| e.unread.clone()).unwrap_or_default()
    });
    leptos::prelude::Effect::new(move || write.server.set_unread.set(unread.get()));

    // Per-surface UnreadStats signal — projects `UnreadView.stats`
    // down to `{channel_name -> stats}` so the UI can render the
    // badge atom without knowing about `SurfaceId`.
    let unread_stats_signal = derived_signal(&views.unread, system, |uv| {
        uv.stats
            .iter()
            .filter_map(|(surface, stats)| match surface {
                willow_client::views::SurfaceId::Channel(name) => {
                    Some((name.clone(), stats.clone()))
                }
                _ => None,
            })
            .collect::<HashMap<String, willow_client::views::UnreadStats>>()
    });
    leptos::prelude::Effect::new(move || {
        write.server.set_unread_stats.set(unread_stats_signal.get())
    });

    let pid_str = handle.peer_id();
    let peer_id = derived_signal(&views.network, system, move |_| pid_str.clone());
    leptos::prelude::Effect::new(move || write.network.set_peer_id.set(peer_id.get()));

    let peer_count = derived_signal(&views.chat_meta, system, |c| c.peers.len());
    leptos::prelude::Effect::new(move || write.network.set_peer_count.set(peer_count.get()));

    let connection = derived_signal(&views.network, system, |n| {
        if n.connected {
            "connected".to_string()
        } else {
            "connecting".to_string()
        }
    });
    leptos::prelude::Effect::new(move || write.network.set_connection_status.set(connection.get()));

    let roles_sig = derived_signal(&views.event_state, system, |es| {
        es.roles
            .iter()
            .map(|(id, role)| {
                let perms: Vec<String> =
                    role.permissions.iter().map(|p| format!("{p:?}")).collect();
                (id.clone(), role.name.clone(), perms)
            })
            .collect::<Vec<_>>()
    });
    leptos::prelude::Effect::new(move || write.server.set_roles.set(roles_sig.get()));

    let owner = derived_signal(&views.event_state, system, |es| {
        // Use the genesis author as the permanent server owner.
        // Fall back to the lexicographically smallest admin for state
        // loaded from older localStorage snapshots that lack genesis_author.
        es.genesis_author
            .or_else(|| es.admins.iter().next().copied())
            .map(|a| a.to_string())
            .unwrap_or_default()
    });
    leptos::prelude::Effect::new(move || write.server.set_server_owner.set(owner.get()));

    let sync_provider_ids = derived_signal(&views.event_state, system, |es| {
        es.peer_permissions
            .iter()
            .filter(|(_, perms)| {
                perms.contains(&willow_client::willow_state::Permission::SyncProvider)
            })
            .map(|(pid, _)| pid.to_string())
            .collect::<std::collections::HashSet<String>>()
    });
    leptos::prelude::Effect::new(move || {
        write
            .server
            .set_sync_provider_ids
            .set(sync_provider_ids.get())
    });

    let admin_ids = derived_signal(&views.event_state, system, |es| {
        es.admins
            .iter()
            .map(|pid| pid.to_string())
            .collect::<std::collections::HashSet<String>>()
    });
    leptos::prelude::Effect::new(move || write.server.set_admin_ids.set(admin_ids.get()));

    let channel_kinds = derived_signal(&views.event_state, system, |es| {
        es.channels
            .values()
            .map(|ch| (ch.name.clone(), ch.kind.clone()))
            .collect::<Vec<_>>()
    });
    leptos::prelude::Effect::new(move || write.server.set_channel_kinds.set(channel_kinds.get()));

    let peers_sig = derived_signal(&views.members, system, |mv| {
        mv.members
            .iter()
            .map(|m| (m.peer_id.to_string(), m.display_name.clone(), m.is_online))
            .collect::<Vec<_>>()
    });
    leptos::prelude::Effect::new(move || write.network.set_peers.set(peers_sig.get()));

    let current_ch = derived_signal(&views.chat_meta, system, |c| c.current_channel.clone());
    leptos::prelude::Effect::new(move || write.chat.set_current_channel.set(current_ch.get()));

    let messages_sig = derived_signal(&views.messages, system, |mv| mv.messages.clone());
    leptos::prelude::Effect::new(move || write.chat.set_messages.set(messages_sig.get()));

    // ── Presence derived signals (phase 1e) ──────────────────────────
    let presence_per_peer_sig = derived_signal(&views.presence, system, |pv| {
        pv.per_peer
            .iter()
            .map(|(pid, st)| (pid.to_string(), *st))
            .collect::<HashMap<String, willow_client::presence::PresenceState>>()
    });
    leptos::prelude::Effect::new(move || {
        write.presence.set_per_peer.set(presence_per_peer_sig.get())
    });

    let presence_self_sig = derived_signal(&views.presence, system, |pv| pv.self_state);
    leptos::prelude::Effect::new(move || {
        write.presence.set_self_state.set(presence_self_sig.get())
    });

    let presence_override_sig = derived_signal(&views.presence_meta, system, |pm| pm.self_override);
    leptos::prelude::Effect::new(move || {
        write
            .presence
            .set_self_override
            .set(presence_override_sig.get())
    });
}
