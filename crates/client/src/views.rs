//! # Derived View Types
//!
//! Reactive view types computed by `DerivedActor`s from Layer 1 source
//! state. Each view subscribes to its sources and only recomputes when
//! they change. `PartialEq` prevents spurious downstream notifications.
//!
//! ## Adding a new view
//!
//! 1. Define a `#[derive(Clone, Debug, PartialEq)]` struct here
//! 2. Write a `compute_*` pure function
//! 3. Call `derived()` in `ClientHandle::new()` to spawn the DerivedActor
//! 4. Add its `StateRef<T>` to `ClientViewHandle`

use std::collections::HashMap;
use std::sync::Arc;

use willow_identity::EndpointId;

use crate::presence::{derive_peer_presence, derive_self_presence, PresenceInputs, PresenceState};
use crate::state::DisplayMessage;
use crate::state_actors::*;

// ───── Layer 2: Derived view types ──────────────────────────────────────

/// Precomputed message list for the current channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MessagesView {
    pub messages: Vec<DisplayMessage>,
    pub channel: String,
}

/// Channel list for the active server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChannelsView {
    pub channels: Vec<ChannelInfo>,
}

/// Channel metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelInfo {
    pub name: String,
    pub kind: willow_state::ChannelKind,
}

/// Member list with online status.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MembersView {
    pub members: Vec<MemberInfo>,
}

/// Member metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberInfo {
    pub peer_id: EndpointId,
    pub display_name: String,
    pub is_online: bool,
}

/// Identifier for a surface that can carry unread state.
///
/// Phase 1f introduces per-surface `UnreadStats` keyed by `SurfaceId`.
/// Only `Channel` is populated today — `Letter` and `Grove` are defined
/// for forward compatibility with the letters and aggregate-grove
/// surfaces in later phases.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceId {
    /// A named channel inside the active server.
    Channel(String),
    /// A 1:1 letter with the given peer.
    Letter(EndpointId),
    /// Aggregate unread for an entire grove (server id).
    Grove(String),
}

/// Per-surface unread stats. Drives the badge priority pipeline —
/// whisper > mentioned > announce-only > muted > default.
///
/// `count` tracks real unread event volume; the flags are independent
/// signals the UI layers in (e.g. a muted channel still increments
/// `count` so the user sees *something is here*).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnreadStats {
    /// Raw unread count. Capped only at render-time (`99+`).
    pub count: u32,
    /// Latest unread mentions the local peer.
    pub mentioned: bool,
    /// Surface is a whisper-marked letter.
    pub whisper: bool,
    /// Every unread event in this surface is governance / announce-only.
    pub announce_only: bool,
    /// Surface is currently muted (channel / grove / letter scope).
    pub muted: bool,
}

/// Unread badge counts per surface.
///
/// The legacy `counts: HashMap<String, usize>` channel-name map is
/// preserved via the `counts()` back-compat shim so phase 1a..1e
/// callers keep compiling. New callers read `stats` directly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnreadView {
    /// Full per-surface stats. Primary source from phase 1f onward.
    pub stats: HashMap<SurfaceId, UnreadStats>,
}

impl UnreadView {
    /// Back-compat channel-name-keyed count map.
    ///
    /// Projects `stats` down to `{channel_name -> count}` for legacy
    /// consumers. New code should read `stats` and `UnreadStats`
    /// variants directly.
    pub fn counts(&self) -> HashMap<String, usize> {
        self.stats
            .iter()
            .filter_map(|(id, s)| match id {
                SurfaceId::Channel(name) => Some((name.clone(), s.count as usize)),
                _ => None,
            })
            .collect()
    }

    /// Aggregate unread for a whole server (sum of channel stats,
    /// highest-priority variant wins).
    ///
    /// Phase 1f places this on `UnreadView` so `server_list` / grove
    /// tiles can render a single pill per grove with the strongest
    /// variant present.
    pub fn for_server(&self, _server_id: &str) -> UnreadStats {
        // All channels in the active-server registry are included today
        // because the registry only tracks one active server at a time.
        // A multi-grove world will key by `_server_id` directly.
        let mut out = UnreadStats::default();
        for (id, s) in &self.stats {
            if matches!(id, SurfaceId::Channel(_)) {
                out.count = out.count.saturating_add(s.count);
                out.mentioned |= s.mentioned;
                out.whisper |= s.whisper;
                out.announce_only |= s.announce_only;
                out.muted |= s.muted;
            }
        }
        out
    }
}

/// Role definitions with permissions.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RolesView {
    pub roles: Vec<RoleEntry>,
}

/// A single role.
#[derive(Clone, Debug, PartialEq)]
pub struct RoleEntry {
    pub id: String,
    pub name: String,
    pub permissions: Vec<String>,
}

/// Connection and presence info.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConnectionView {
    pub connected: bool,
    pub peer_count: usize,
    pub typing_peers: Vec<(EndpointId, String)>,
}

/// Derived presence snapshot — per-peer [`PresenceState`] plus the
/// local user's self-state.
///
/// Recomputed by the presence derived actor whenever any of its source
/// actors ([`PresenceMeta`], [`ChatMeta`], [`VoiceState`]) change. The
/// `PartialEq` derive keeps downstream signals quiet across identical
/// frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresenceView {
    /// Per-peer presence state (peers currently in ChatMeta::peers or
    /// ever seen via last_seen map).
    pub per_peer: HashMap<EndpointId, PresenceState>,
    /// Local user's presence state (respects self-override).
    pub self_state: PresenceState,
}

// ───── Layer 3: Grouped views ───────────────────────────────────────────

/// Chat-related views grouped for terminal composition.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatViews {
    pub messages: MessagesView,
    pub channels: ChannelsView,
    pub unread: UnreadView,
}

/// Social-related views grouped for terminal composition.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SocialViews {
    pub members: MembersView,
    pub roles: RolesView,
    pub connection: ConnectionView,
}

/// Terminal composite — everything the UI needs in one snapshot.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientView {
    pub chat: ChatViews,
    pub social: SocialViews,
    pub voice: VoiceState,
    pub server_name: Option<String>,
    pub server_admins: Vec<EndpointId>,
    pub current_channel: String,
}

// ───── ClientViewHandle ─────────────────────────────────────────────────

/// Bundle of all reactive state references at every granularity.
///
/// Consumers pick their subscription level:
/// - `view` — terminal, everything in one snapshot
/// - `messages`, `members`, etc. — individual Layer 2 views
/// - `event_state`, `chat_meta`, etc. — raw Layer 1 source state
pub struct ClientViewHandle {
    // Terminal
    pub view: willow_actor::state::StateRef<ClientView>,

    // Layer 2 — derived views
    pub messages: willow_actor::state::StateRef<MessagesView>,
    pub members: willow_actor::state::StateRef<MembersView>,
    pub channels: willow_actor::state::StateRef<ChannelsView>,
    pub unread: willow_actor::state::StateRef<UnreadView>,
    pub roles: willow_actor::state::StateRef<RolesView>,
    pub connection: willow_actor::state::StateRef<ConnectionView>,
    pub presence: willow_actor::state::StateRef<PresenceView>,

    // Layer 1 — raw source state
    pub event_state: willow_actor::state::StateRef<willow_state::ServerState>,
    pub server_registry: willow_actor::state::StateRef<ServerRegistry>,
    pub chat_meta: willow_actor::state::StateRef<ChatMeta>,
    pub profiles: willow_actor::state::StateRef<ProfileState>,
    pub network: willow_actor::state::StateRef<NetworkMeta>,
    pub voice: willow_actor::state::StateRef<VoiceState>,
    pub presence_meta: willow_actor::state::StateRef<PresenceMeta>,
}

impl Clone for ClientViewHandle {
    fn clone(&self) -> Self {
        Self {
            view: self.view.clone(),
            messages: self.messages.clone(),
            members: self.members.clone(),
            channels: self.channels.clone(),
            unread: self.unread.clone(),
            roles: self.roles.clone(),
            connection: self.connection.clone(),
            presence: self.presence.clone(),
            event_state: self.event_state.clone(),
            server_registry: self.server_registry.clone(),
            chat_meta: self.chat_meta.clone(),
            profiles: self.profiles.clone(),
            network: self.network.clone(),
            voice: self.voice.clone(),
            presence_meta: self.presence_meta.clone(),
        }
    }
}

// ───── Compute functions (pure) ─────────────────────────────────────────

/// Compute the messages view for the current channel.
pub fn compute_messages_view(
    events: &Arc<willow_state::ServerState>,
    _registry: &Arc<ServerRegistry>,
    chat: &Arc<ChatMeta>,
    profiles: &Arc<ProfileState>,
    local_peer_id: EndpointId,
) -> MessagesView {
    let ch = &chat.current_channel;
    let mut channel_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (id, c) in &events.channels {
        if c.name == *ch {
            channel_ids.insert(id.clone());
        }
    }

    if channel_ids.is_empty() {
        return MessagesView {
            messages: vec![],
            channel: ch.clone(),
        };
    }

    let mut msgs: Vec<DisplayMessage> = events
        .messages
        .iter()
        .filter(|m| channel_ids.contains(&m.channel_id))
        .map(|m| {
            let author_name = resolve_display_name(events, profiles, &m.author);
            let reply_preview = m.reply_to.as_ref().and_then(|parent_hash| {
                events
                    .messages
                    .iter()
                    .find(|pm| pm.id == *parent_hash)
                    .map(|pm| {
                        let parent_name = resolve_display_name(events, profiles, &pm.author);
                        let text = if pm.body.chars().count() > 50 {
                            let truncated: String = pm.body.chars().take(50).collect();
                            format!("{truncated}...")
                        } else {
                            pm.body.clone()
                        };
                        format!("{parent_name}: {text}")
                    })
            });
            let reactions = m
                .reactions
                .iter()
                .map(|(emoji, peer_ids)| {
                    let names: Vec<String> = peer_ids
                        .iter()
                        .map(|pid| resolve_display_name(events, profiles, pid))
                        .collect();
                    (emoji.clone(), names)
                })
                .collect();
            DisplayMessage {
                id: m.id.to_string(),
                channel_id: m.channel_id.clone(),
                author_peer_id: m.author,
                author_display_name: author_name,
                body: m.body.clone(),
                is_local: m.author == local_peer_id,
                timestamp_ms: m.timestamp_ms,
                reactions,
                edited: m.edited,
                deleted: m.deleted,
                reply_to: m.reply_to.as_ref().map(|h| h.to_string()),
                reply_preview,
            }
        })
        .collect();
    msgs.sort_by_key(|m| m.timestamp_ms);
    MessagesView {
        messages: msgs,
        channel: ch.clone(),
    }
}

/// Compute the members view.
pub fn compute_members_view(
    events: &Arc<willow_state::ServerState>,
    chat: &Arc<ChatMeta>,
    profiles: &Arc<ProfileState>,
    local_peer_id: EndpointId,
) -> MembersView {
    let online: std::collections::HashSet<EndpointId> = chat.peers.iter().copied().collect();
    let mut result = Vec::new();
    for (pid, member) in &events.members {
        let name = member
            .display_name
            .clone()
            .or_else(|| events.profiles.get(pid).map(|p| p.display_name.clone()))
            .unwrap_or_else(|| resolve_display_name(events, profiles, pid));
        let is_online = *pid == local_peer_id || online.contains(pid);
        result.push(MemberInfo {
            peer_id: *pid,
            display_name: name,
            is_online,
        });
    }
    MembersView { members: result }
}

/// Compute the channels view.
pub fn compute_channels_view(
    events: &Arc<willow_state::ServerState>,
    _registry: &Arc<ServerRegistry>,
) -> ChannelsView {
    let mut names: Vec<ChannelInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // From event state (the single source of truth for channels).
    for ch in events.channels.values() {
        if seen.insert(ch.name.clone()) {
            names.push(ChannelInfo {
                name: ch.name.clone(),
                kind: ch.kind.clone(),
            });
        }
    }

    names.sort_by(|a, b| a.name.cmp(&b.name));
    ChannelsView { channels: names }
}

/// Compute unread stats per surface.
///
/// Phase 1f: consumes the active server's `unread` topic map plus the
/// event-sourced `mute_state` to build per-channel `UnreadStats`.
/// `mentioned` is stubbed as `false` until the last-500-message
/// substring pass lands — the render pipeline already handles the
/// variant, so turning it on is a single-line change.
pub fn compute_unread_view(
    registry: &Arc<ServerRegistry>,
    event_state: &Arc<willow_state::ServerState>,
    local_peer_id: EndpointId,
) -> UnreadView {
    let mut stats: HashMap<SurfaceId, UnreadStats> = HashMap::new();
    let mute = event_state.mute_state.get(&local_peer_id).cloned();
    if let Some(entry) = registry.active() {
        for (topic, count) in &entry.unread {
            if let Some(name) = entry.name_for_topic(topic) {
                let channel_id = event_state
                    .channels
                    .values()
                    .find(|c| c.name == name)
                    .map(|c| c.id.clone());
                let muted = mute
                    .as_ref()
                    .map(|m| {
                        m.grove_muted
                            || channel_id
                                .as_ref()
                                .map(|id| m.channels.contains(id))
                                .unwrap_or(false)
                    })
                    .unwrap_or(false);
                let s = UnreadStats {
                    count: *count as u32,
                    mentioned: false,
                    whisper: false,
                    announce_only: false,
                    muted,
                };
                stats.insert(SurfaceId::Channel(name.to_string()), s);
            }
        }
    }
    UnreadView { stats }
}

/// Compute roles view.
pub fn compute_roles_view(events: &Arc<willow_state::ServerState>) -> RolesView {
    let mut roles: Vec<RoleEntry> = events
        .roles
        .values()
        .map(|role| RoleEntry {
            id: role.id.clone(),
            name: role.name.clone(),
            permissions: role.permissions.iter().cloned().collect(),
        })
        .collect();
    roles.sort_by(|a, b| a.name.cmp(&b.name));
    RolesView { roles }
}

/// Compute the presence view from source actors.
///
/// Inputs:
///   - `presence_meta` — tick counter, last-seen map, queue depth,
///     whisper/invisibility sets, self-override, thresholds.
///   - `chat` — online peer list (proxy for network reachability).
///   - `voice` — per-channel participant sets (any non-empty set means
///     the peer is in a call).
///   - `local_peer_id` — used to compute self-state and to exclude self
///     from the per-peer map.
pub fn compute_presence_view(
    presence_meta: &Arc<PresenceMeta>,
    chat: &Arc<ChatMeta>,
    voice: &Arc<VoiceState>,
    local_peer_id: EndpointId,
) -> PresenceView {
    let reachable: std::collections::HashSet<EndpointId> = chat.peers.iter().copied().collect();
    let in_call: std::collections::HashSet<EndpointId> = voice
        .participants
        .values()
        .flat_map(|set| set.iter().copied())
        .collect();

    // Build a union of peer IDs across all inputs so we emit a state for
    // every peer we've heard of, not just ones currently reachable.
    let mut peer_ids: std::collections::HashSet<EndpointId> = std::collections::HashSet::new();
    peer_ids.extend(reachable.iter().copied());
    peer_ids.extend(presence_meta.last_seen.keys().copied());
    peer_ids.extend(presence_meta.queue_depth.keys().copied());
    peer_ids.extend(presence_meta.whispering_with.iter().copied());
    peer_ids.extend(in_call.iter().copied());
    peer_ids.remove(&local_peer_id);

    let mut per_peer = HashMap::with_capacity(peer_ids.len());
    for pid in peer_ids {
        let inputs = PresenceInputs {
            now: presence_meta.now,
            last_seen: presence_meta.last_seen.get(&pid).copied().unwrap_or(0),
            reachable: reachable.contains(&pid),
            in_call: in_call.contains(&pid) && !presence_meta.whispering_with.contains(&pid),
            whispering: presence_meta.whispering_with.contains(&pid),
            queue_depth: presence_meta.queue_depth.get(&pid).copied().unwrap_or(0),
            invisible_to_me: presence_meta.invisible_to_me.contains(&pid),
            idle_ticks: presence_meta.idle_ticks,
            gone_ticks: presence_meta.gone_ticks,
        };
        per_peer.insert(pid, derive_peer_presence(&inputs));
    }

    // Self is always reachable if we have a connected network; approx
    // by checking whether self appears in our own chat.peers list or the
    // chat meta is non-empty. In phase 1e we just use `true` for now —
    // the override takes precedence anyway.
    let self_reachable = true;
    let self_in_call = voice.active_channel.is_some();
    let self_whispering = false; // stub; real signal in whisper-mode phase.
    let self_state = derive_self_presence(
        presence_meta.self_override,
        self_reachable,
        self_in_call,
        self_whispering,
    );

    PresenceView {
        per_peer,
        self_state,
    }
}

/// Compute connection view.
pub fn compute_connection_view(network: &Arc<NetworkMeta>, chat: &Arc<ChatMeta>) -> ConnectionView {
    let typing = network
        .typing_peers
        .iter()
        .map(|(id, (ch, _))| (*id, ch.clone()))
        .collect();
    ConnectionView {
        connected: network.connected,
        peer_count: chat.peers.len(),
        typing_peers: typing,
    }
}

// ───── Helper ───────────────────────────────────────────────────────────

/// Compute messages for a specific channel (not necessarily the current one).
pub fn compute_messages_view_for_channel(
    events: &Arc<willow_state::ServerState>,
    registry: &Arc<ServerRegistry>,
    profiles: &Arc<ProfileState>,
    channel: &str,
    local_peer_id: EndpointId,
) -> MessagesView {
    let chat = Arc::new(ChatMeta {
        current_channel: channel.to_string(),
        peers: Vec::new(),
    });
    compute_messages_view(events, registry, &chat, profiles, local_peer_id)
}

pub fn resolve_display_name(
    event_state: &willow_state::ServerState,
    profiles: &ProfileState,
    peer_id: &EndpointId,
) -> String {
    event_state
        .profiles
        .get(peer_id)
        .map(|p| p.display_name.clone())
        .unwrap_or_else(|| profiles.display_name(peer_id))
}
