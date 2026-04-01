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
    pub kind: String,
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

/// Unread badge counts per channel name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnreadView {
    pub counts: HashMap<String, usize>,
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
    pub server_owner: Option<EndpointId>,
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

    // Layer 1 — raw source state
    pub event_state: willow_actor::state::StateRef<willow_state::ServerState>,
    pub server_registry: willow_actor::state::StateRef<ServerRegistry>,
    pub chat_meta: willow_actor::state::StateRef<ChatMeta>,
    pub profiles: willow_actor::state::StateRef<ProfileState>,
    pub network: willow_actor::state::StateRef<NetworkMeta>,
    pub voice: willow_actor::state::StateRef<VoiceState>,
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
            event_state: self.event_state.clone(),
            server_registry: self.server_registry.clone(),
            chat_meta: self.chat_meta.clone(),
            profiles: self.profiles.clone(),
            network: self.network.clone(),
            voice: self.voice.clone(),
        }
    }
}

// ───── Compute functions (pure) ─────────────────────────────────────────

/// Compute the messages view for the current channel.
pub fn compute_messages_view(
    events: &Arc<willow_state::ServerState>,
    registry: &Arc<ServerRegistry>,
    chat: &Arc<ChatMeta>,
    profiles: &Arc<ProfileState>,
    local_peer_id: EndpointId,
) -> MessagesView {
    let ch = &chat.current_channel;
    let mut channel_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    if let Some(entry) = registry.active() {
        for (name, cid) in entry.topic_map.values() {
            if name == ch {
                channel_ids.insert(cid.to_string());
            }
        }
    }
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
            let reply_preview = m.reply_to.as_ref().and_then(|parent_id| {
                events
                    .messages
                    .iter()
                    .find(|pm| pm.id == *parent_id)
                    .map(|pm| {
                        let parent_name = resolve_display_name(events, profiles, &pm.author);
                        let text = if pm.body.len() > 50 {
                            format!("{}...", &pm.body[..50])
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
                id: m.id.clone(),
                channel_id: m.channel_id.clone(),
                author_peer_id: m.author,
                author_display_name: author_name,
                body: m.body.clone(),
                is_local: m.author == local_peer_id,
                timestamp_ms: m.timestamp_ms,
                reactions,
                edited: m.edited,
                deleted: m.deleted,
                reply_to: m.reply_to.clone(),
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
    let mut seen = std::collections::HashSet::new();
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
        seen.insert(*pid);
    }
    for pid in &chat.peers {
        if !seen.contains(pid) {
            result.push(MemberInfo {
                peer_id: *pid,
                display_name: resolve_display_name(events, profiles, pid),
                is_online: true,
            });
        }
    }
    MembersView { members: result }
}

/// Compute the channels view.
pub fn compute_channels_view(
    events: &Arc<willow_state::ServerState>,
    registry: &Arc<ServerRegistry>,
) -> ChannelsView {
    let mut names: Vec<ChannelInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // From server registry (authoritative channel list).
    if let Some(entry) = registry.active() {
        for ch in entry.server.channels() {
            if seen.insert(ch.name.clone()) {
                let kind = match ch.kind {
                    willow_channel::ChannelKind::Text => "text",
                    willow_channel::ChannelKind::Voice => "voice",
                };
                names.push(ChannelInfo {
                    name: ch.name.clone(),
                    kind: kind.to_string(),
                });
            }
        }
    }

    // From event state (may have channels not yet in registry).
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

/// Compute unread counts.
pub fn compute_unread_view(registry: &Arc<ServerRegistry>) -> UnreadView {
    let mut counts = HashMap::new();
    if let Some(entry) = registry.active() {
        for (topic, count) in &entry.unread {
            if let Some(name) = entry.name_for_topic(topic) {
                counts.insert(name.to_string(), *count);
            }
        }
    }
    UnreadView { counts }
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

/// Compute connection view.
pub fn compute_connection_view(
    network: &Arc<NetworkMeta>,
    chat: &Arc<ChatMeta>,
) -> ConnectionView {
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
        seen_message_ids: std::collections::HashSet::new(),
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
