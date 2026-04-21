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

use crate::mentions::{extract_mention_peers, parse_mentions, PeerRef};
use crate::presence::{derive_peer_presence, derive_self_presence, PresenceInputs, PresenceState};
use crate::state::{DisplayMessage, QueueNote};
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

    // Build a PeerRef list for mention resolution. Willow does not yet
    // track a distinct `@handle` (see `profile-card.md` for the target
    // profile data model); as a stand-in we derive a handle from the
    // display name via `display_name.to_lowercase().replace(' ', '.')`.
    // TODO(profile-card.md): replace the display-name-derived handle
    // with the real handle field once profile data is plumbed.
    let peer_refs: Vec<PeerRef> = events
        .members
        .keys()
        .map(|pid| {
            let display = resolve_display_name(events, profiles, pid);
            let handle = display.to_lowercase().replace(' ', ".");
            PeerRef {
                peer_id: *pid,
                handle,
                display_name: display,
            }
        })
        .collect();

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
            let mention_segments = parse_mentions(&m.body, &peer_refs, &local_peer_id);
            let mentions = extract_mention_peers(&mention_segments);
            // Stamp pinned from the channel's pinned-message set.
            // `ServerState::channels[cid].pinned_messages` is a
            // `BTreeSet<EventHash>` owned by the pin-event projection in
            // willow-state; message-row.md §Pins consumes it here as a
            // quiet 1 px amber row marker + `pinned` badge.
            let pinned = events
                .channels
                .get(&m.channel_id)
                .map(|ch| ch.pinned_messages.contains(&m.id))
                .unwrap_or(false);
            // Queue-note derivation. Phase 2a Task 7 wires the field
            // end-to-end but defers real detection to sync-queue.md:
            // today there is no `MessageStore::delivery_state`, no
            // per-peer presence history, and no ack set — so both
            // `Pending` (local author, unacked) and `LateArrival` (peer
            // offline at authoring) fall back to `None`. The renderer
            // is ready for the full tri-state; once sync-queue lands
            // the only change needed here is replacing `None` with
            // the real lookups.
            // TODO(sync-queue.md): derive Pending from
            // `MessageStore::delivery_state(&m.id)` for `m.is_local`
            // and LateArrival from a peer-presence-history oracle
            // (was-peer-offline-near(author, ts, 30_000)).
            let queue_note = QueueNote::None;
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
                mentions,
                pinned,
                queue_note,
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
/// Phase 1f seeded `mentioned` as a stub. Phase 2a Task 4 replaces the
/// stub with `mentions_me`: for each channel with `count > 0`, scan
/// the tail of that channel's messages (up to `count`, capped at 500
/// per spec §Self-mention row highlight) and flip `mentioned = true`
/// if any of them mentions the local peer.
///
/// The projection uses `DisplayMessage.mentions` via `parse_mentions`,
/// so this respects the same resolver order as the row-level highlight
/// (exact handle → first-segment → display-name → `@you`).
pub fn compute_unread_view(
    registry: &Arc<ServerRegistry>,
    event_state: &Arc<willow_state::ServerState>,
    local_peer_id: EndpointId,
) -> UnreadView {
    let mut stats: HashMap<SurfaceId, UnreadStats> = HashMap::new();
    let mute = event_state.mute_state.get(&local_peer_id).cloned();

    // Build a PeerRef list once for the mention parser. Mirrors the
    // build in `compute_messages_view`; TODO(profile-card.md) tracks
    // swapping display-name-derived handles for real profile handles.
    //
    // `resolve_display_name` needs a `ProfileState` — we only have the
    // event-state profiles here, so fall back to the event-state entry
    // (and the truncated peer id) via a small inline helper. This
    // keeps `compute_unread_view`'s signature stable for 1f's callers.
    let peer_refs: Vec<PeerRef> = event_state
        .members
        .keys()
        .map(|pid| {
            let display = event_state
                .profiles
                .get(pid)
                .map(|p| p.display_name.clone())
                .unwrap_or_else(|| crate::util::truncate_peer_id(&pid.to_string()));
            let handle = display.to_lowercase().replace(' ', ".");
            PeerRef {
                peer_id: *pid,
                handle,
                display_name: display,
            }
        })
        .collect();

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

                // Heuristic: "unread mention" = any message in the
                // tail-slice of this channel mentions the local peer.
                // Cap at 500 per spec §Self-mention row highlight.
                let mentioned = if *count > 0 {
                    if let Some(cid) = &channel_id {
                        let tail_len = (*count).min(500);
                        // Collect channel messages in arrival order
                        // (already ordered in ServerState) and scan
                        // the last `tail_len` of them.
                        let channel_msgs: Vec<&willow_state::ChatMessage> = event_state
                            .messages
                            .iter()
                            .filter(|msg| &msg.channel_id == cid)
                            .collect();
                        let start = channel_msgs.len().saturating_sub(tail_len);
                        channel_msgs[start..].iter().any(|msg| {
                            let segs = parse_mentions(&msg.body, &peer_refs, &local_peer_id);
                            extract_mention_peers(&segs).contains(&local_peer_id)
                        })
                    } else {
                        false
                    }
                } else {
                    false
                };

                let s = UnreadStats {
                    count: *count as u32,
                    mentioned,
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

#[cfg(test)]
mod tests {
    //! Projection tests for Phase 2a Task 4 — populated `mentions` in
    //! `DisplayMessage` and `mentioned` flag in `UnreadStats`.
    use super::*;
    use std::collections::{BTreeMap, HashMap};
    use willow_identity::Identity;
    use willow_state::{Channel, ChannelKind, ChatMessage, Member, Profile, ServerState};

    fn fresh_state(owner: EndpointId) -> ServerState {
        ServerState::new("srv", "Test", owner)
    }

    fn add_member(state: &mut ServerState, peer_id: EndpointId, display: &str) {
        state.members.insert(
            peer_id,
            Member {
                peer_id,
                roles: Default::default(),
                display_name: None,
            },
        );
        state.profiles.insert(
            peer_id,
            Profile {
                peer_id,
                display_name: display.into(),
            },
        );
    }

    fn push_channel(state: &mut ServerState, id: &str, name: &str) {
        state.channels.insert(
            id.into(),
            Channel {
                id: id.into(),
                name: name.into(),
                pinned_messages: Default::default(),
                kind: ChannelKind::Text,
            },
        );
    }

    fn push_message(
        state: &mut ServerState,
        channel_id: &str,
        author: EndpointId,
        body: &str,
        ts_ms: u64,
    ) -> willow_state::EventHash {
        // Derive a unique-ish EventHash from message index + body so
        // repeat calls within one test don't collide.
        let seed = format!("test-msg-{}-{}", state.messages.len(), body);
        let id = willow_state::EventHash::from_bytes(seed.as_bytes());
        state.messages.push(ChatMessage {
            id,
            channel_id: channel_id.into(),
            author,
            body: body.into(),
            timestamp_ms: ts_ms,
            edited: false,
            deleted: false,
            reactions: BTreeMap::new(),
            reply_to: None,
        });
        id
    }

    #[test]
    fn projection_populates_mentions_for_body_at_mira() {
        // Spec: body "hi @mira" → DisplayMessage.mentions contains
        // Mira's endpoint id. The handle-from-display-name stand-in
        // lowercases the display name and maps spaces → dots, so
        // `@mira` resolves by the first-segment-of-handle path.
        let owner = Identity::generate().endpoint_id();
        let mira = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        add_member(&mut state, mira, "Mira");
        push_channel(&mut state, "ch-1", "general");
        push_message(&mut state, "ch-1", owner, "hi @mira", 1_000);

        let events = Arc::new(state);
        let registry = Arc::new(ServerRegistry::default());
        let chat = Arc::new(ChatMeta {
            current_channel: "general".into(),
            peers: Vec::new(),
        });
        let profiles = Arc::new(ProfileState::default());
        let view = compute_messages_view(&events, &registry, &chat, &profiles, owner);
        assert_eq!(view.messages.len(), 1);
        assert_eq!(
            view.messages[0].mentions,
            vec![mira],
            "`@mira` must resolve Mira's endpoint id into msg.mentions"
        );
    }

    #[test]
    fn projection_mentions_empty_when_no_at_tokens() {
        let owner = Identity::generate().endpoint_id();
        let mira = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        add_member(&mut state, mira, "Mira");
        push_channel(&mut state, "ch-1", "general");
        push_message(&mut state, "ch-1", owner, "no mentions here", 1_000);

        let events = Arc::new(state);
        let registry = Arc::new(ServerRegistry::default());
        let chat = Arc::new(ChatMeta {
            current_channel: "general".into(),
            peers: Vec::new(),
        });
        let profiles = Arc::new(ProfileState::default());
        let view = compute_messages_view(&events, &registry, &chat, &profiles, owner);
        assert_eq!(view.messages.len(), 1);
        assert!(view.messages[0].mentions.is_empty());
    }

    fn make_registry(server_id: &str, unread_topic: &str, unread_count: usize) -> ServerRegistry {
        let mut unread = HashMap::new();
        unread.insert(unread_topic.into(), unread_count);
        let entry = ServerEntry {
            server_id: server_id.into(),
            name: "Test".into(),
            keys: HashMap::new(),
            unread,
        };
        let mut servers = HashMap::new();
        servers.insert(server_id.into(), entry);
        ServerRegistry {
            servers,
            active_server: Some(server_id.into()),
        }
    }

    #[test]
    fn unread_mentioned_counter_counts_at_me() {
        // A channel with one unread message that mentions the local
        // peer (via @you alias) must flip `mentioned = true`.
        let owner = Identity::generate().endpoint_id();
        let other = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        add_member(&mut state, other, "Rin");
        push_channel(&mut state, "ch-1", "general");
        push_message(&mut state, "ch-1", other, "ping @you", 1_000);
        let events = Arc::new(state);

        let registry = Arc::new(make_registry("srv", "srv/general", 1));
        let view = compute_unread_view(&registry, &events, owner);
        let stats = view
            .stats
            .get(&SurfaceId::Channel("general".into()))
            .expect("general surface must exist");
        assert_eq!(stats.count, 1);
        assert!(stats.mentioned, "@you in unread must flip mentioned=true");
    }

    #[test]
    fn unread_mentioned_stays_false_when_no_self_mention() {
        let owner = Identity::generate().endpoint_id();
        let other = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        add_member(&mut state, other, "Rin");
        push_channel(&mut state, "ch-1", "general");
        push_message(&mut state, "ch-1", other, "just a normal message", 1_000);
        let events = Arc::new(state);

        let registry = Arc::new(make_registry("srv", "srv/general", 1));
        let view = compute_unread_view(&registry, &events, owner);
        let stats = view
            .stats
            .get(&SurfaceId::Channel("general".into()))
            .expect("general surface must exist");
        assert!(!stats.mentioned);
    }

    // ── Phase 2a Task 6 — pinned projection ────────────────────────────

    #[test]
    fn projection_pinned_false_when_not_pinned() {
        // A message with no corresponding entry in
        // `Channel::pinned_messages` must project to `pinned: false`.
        let owner = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        push_channel(&mut state, "ch-1", "general");
        push_message(&mut state, "ch-1", owner, "hello world", 1_000);

        let events = Arc::new(state);
        let registry = Arc::new(ServerRegistry::default());
        let chat = Arc::new(ChatMeta {
            current_channel: "general".into(),
            peers: Vec::new(),
        });
        let profiles = Arc::new(ProfileState::default());
        let view = compute_messages_view(&events, &registry, &chat, &profiles, owner);
        assert_eq!(view.messages.len(), 1);
        assert!(
            !view.messages[0].pinned,
            "unpinned messages must project to `pinned: false`"
        );
    }

    #[test]
    fn projection_pinned_true_when_channel_lists_message() {
        // Spec (message-row.md §Pins / §Data dependencies): `pinned` is
        // `true` whenever the channel's `pinned_messages` set contains
        // this message's id. The projection reads straight off the
        // server state — no separate pin-projection cache.
        let owner = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        push_channel(&mut state, "ch-1", "general");
        let msg_hash = push_message(&mut state, "ch-1", owner, "pin me", 1_000);
        // Simulate the PinMessage event's effect on ServerState.
        state
            .channels
            .get_mut("ch-1")
            .unwrap()
            .pinned_messages
            .insert(msg_hash);

        let events = Arc::new(state);
        let registry = Arc::new(ServerRegistry::default());
        let chat = Arc::new(ChatMeta {
            current_channel: "general".into(),
            peers: Vec::new(),
        });
        let profiles = Arc::new(ProfileState::default());
        let view = compute_messages_view(&events, &registry, &chat, &profiles, owner);
        assert_eq!(view.messages.len(), 1);
        assert!(
            view.messages[0].pinned,
            "messages in `channel.pinned_messages` must project to `pinned: true`"
        );
    }

    #[test]
    fn projection_pinned_flips_back_false_on_unpin() {
        // Unpinning (UnpinMessage) removes the hash from
        // `channel.pinned_messages`, which must flip `pinned` back to
        // false on the next projection pass.
        let owner = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        push_channel(&mut state, "ch-1", "general");
        let msg_hash = push_message(&mut state, "ch-1", owner, "pin me", 1_000);
        let ch = state.channels.get_mut("ch-1").unwrap();
        ch.pinned_messages.insert(msg_hash);
        ch.pinned_messages.remove(&msg_hash);

        let events = Arc::new(state);
        let registry = Arc::new(ServerRegistry::default());
        let chat = Arc::new(ChatMeta {
            current_channel: "general".into(),
            peers: Vec::new(),
        });
        let profiles = Arc::new(ProfileState::default());
        let view = compute_messages_view(&events, &registry, &chat, &profiles, owner);
        assert_eq!(view.messages.len(), 1);
        assert!(
            !view.messages[0].pinned,
            "UnpinMessage must flip `pinned` back to false on projection"
        );
    }

    // ── Phase 2a Task 7 — queue_note projection ────────────────────────
    //
    // The projection carries `queue_note` end-to-end but defers real
    // detection (delivery_state + peer presence history) to
    // sync-queue.md. Today it always emits `QueueNote::None`. These
    // tests pin that baseline so the UX stays non-broken until
    // sync-queue lands — when real detection arrives, new tests
    // covering Pending / LateArrival join these.

    #[test]
    fn projection_queue_note_none_by_default() {
        // A fresh message (local author, no sync-queue hooks) must
        // project with `queue_note == None`. See the
        // `TODO(sync-queue.md)` marker in `compute_messages_view`.
        let owner = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        push_channel(&mut state, "ch-1", "general");
        push_message(&mut state, "ch-1", owner, "hello", 1_000);

        let events = Arc::new(state);
        let registry = Arc::new(ServerRegistry::default());
        let chat = Arc::new(ChatMeta {
            current_channel: "general".into(),
            peers: Vec::new(),
        });
        let profiles = Arc::new(ProfileState::default());
        let view = compute_messages_view(&events, &registry, &chat, &profiles, owner);
        assert_eq!(view.messages.len(), 1);
        assert_eq!(
            view.messages[0].queue_note,
            QueueNote::None,
            "default projection must emit QueueNote::None (sync-queue.md wires real detection)"
        );
    }

    #[test]
    fn projection_queue_note_none_for_local_author_pending_stub() {
        // Until `MessageStore::delivery_state` lands, local-author
        // messages cannot be detected as Pending — the fallback must
        // still be None so the UX stays coherent. This test pins the
        // fallback so the day detection lands we notice the flip.
        let owner = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        push_channel(&mut state, "ch-1", "general");
        push_message(&mut state, "ch-1", owner, "sent while offline", 1_000);

        let events = Arc::new(state);
        let registry = Arc::new(ServerRegistry::default());
        let chat = Arc::new(ChatMeta {
            current_channel: "general".into(),
            peers: Vec::new(),
        });
        let profiles = Arc::new(ProfileState::default());
        let view = compute_messages_view(&events, &registry, &chat, &profiles, owner);
        assert_eq!(view.messages.len(), 1);
        assert!(view.messages[0].is_local, "author must be local");
        assert_eq!(
            view.messages[0].queue_note,
            QueueNote::None,
            "sync-queue.md fallback: local-author messages project as None today"
        );
    }

    #[test]
    fn projection_queue_note_none_for_peer_offline_late_arrival_stub() {
        // Mirror of the above for the LateArrival fallback. Until a
        // peer-presence-history oracle exists, peer-authored messages
        // cannot be flagged as LateArrival — the projection must
        // still emit None to keep the row render stable.
        let owner = Identity::generate().endpoint_id();
        let other = Identity::generate().endpoint_id();
        let mut state = fresh_state(owner);
        add_member(&mut state, other, "Rin");
        push_channel(&mut state, "ch-1", "general");
        push_message(&mut state, "ch-1", other, "from offline peer", 1_000);

        let events = Arc::new(state);
        let registry = Arc::new(ServerRegistry::default());
        let chat = Arc::new(ChatMeta {
            current_channel: "general".into(),
            peers: Vec::new(),
        });
        let profiles = Arc::new(ProfileState::default());
        let view = compute_messages_view(&events, &registry, &chat, &profiles, owner);
        assert_eq!(view.messages.len(), 1);
        assert!(!view.messages[0].is_local, "author must be peer");
        assert_eq!(
            view.messages[0].queue_note,
            QueueNote::None,
            "sync-queue.md fallback: peer-authored messages project as None today"
        );
    }
}
