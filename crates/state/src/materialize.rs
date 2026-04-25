//! State materialization — projecting the DAG into [`ServerState`].
//!
//! The [`materialize`] function is the ONLY way to derive state from a
//! DAG. It topologically sorts all events and replays them through
//! [`apply_event`], producing identical output on all peers given the
//! same DAG contents.

use std::collections::{BTreeMap, BTreeSet};

use willow_identity::EndpointId;

use crate::dag::EventDag;
use crate::event::{Event, EventKind, Permission, ProposedAction};
use crate::hash::EventHash;
use crate::server::{PendingProposal, ServerState};
use crate::types::{
    Channel, ChatMessage, Member, PinnedFragment, Profile, PROFILE_CAP_BIO,
    PROFILE_CAP_CREST_COLOR, PROFILE_CAP_ELSEWHERE_ENTRY, PROFILE_CAP_ELSEWHERE_LEN,
    PROFILE_CAP_PINNED_BODY, PROFILE_CAP_PRONOUNS, PROFILE_CAP_SINCE, PROFILE_CAP_TAGLINE,
};

/// Truncate `s` to at most `cap` UTF-8 characters.
///
/// Walks char boundaries so multi-byte graphemes are never split mid-
/// codepoint. Used on `UpdateProfile` apply to cap each field without
/// rejecting the entire event — misbehaving clients are rate-limited
/// rather than divergent.
fn truncate_chars(s: &str, cap: usize) -> String {
    s.chars().take(cap).collect()
}

/// Result of applying an event to state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyResult {
    /// The event was applied successfully.
    Applied,
    /// The event was rejected (e.g., insufficient permissions).
    Rejected(String),
    /// The event was already applied (idempotent dedup).
    AlreadyApplied,
}

/// Compute the current server state from the full event DAG.
///
/// Derives the genesis author, server_id, and server name from the genesis
/// event. Then topologically sorts all events and replays them.
pub fn materialize(dag: &EventDag) -> ServerState {
    let genesis = dag.genesis().expect("DAG must have a genesis event");
    let server_id = genesis.hash.to_string();
    let genesis_author = genesis.author;
    let name = match &genesis.kind {
        EventKind::CreateServer { name } => name.clone(),
        _ => panic!("genesis event must be CreateServer"),
    };

    let sorted = dag.topological_sort();
    let mut state = ServerState::new(&server_id, &name, genesis_author);
    for event in sorted {
        state.applied_events.insert(event.hash);
        apply_event(&mut state, event);
    }
    state
}

/// Apply a single new event to an existing materialized state.
///
/// Precondition: all causal parents of `event` are already reflected
/// in `state`. This is O(1) per event.
///
/// If the state was just deserialized from disk — recognized by a
/// populated `messages` vector paired with an empty `message_index` —
/// the index is transparently rebuilt once here. This prevents the
/// silent no-op on `EditMessage`/`DeleteMessage`/`Reaction` that would
/// otherwise happen because `message_index` is `#[serde(skip)]`.
pub fn apply_incremental(state: &mut ServerState, event: &Event) -> ApplyResult {
    // Lazy index rebuild for deserialized states. Triggers at most once per
    // loaded state (subsequent calls observe a populated index and skip).
    if !state.messages.is_empty() && state.message_index.len() != state.messages.len() {
        state.rebuild_message_index();
    }
    if !state.applied_events.insert(event.hash) {
        return ApplyResult::AlreadyApplied;
    }
    apply_event(state, event)
}

// ───── Permission pre-check ────────────────────────────────────────────────

/// Check whether an author is allowed to emit a given [`EventKind`]
/// against the current state.
///
/// This is the same authority logic used inside [`apply_event`],
/// extracted so callers can pre-check *before* signing an event.
/// Returns `Ok(())` if permitted, or an error string describing why
/// the author is not allowed.
///
/// See `docs/specs/2026-04-12-state-authority-and-mutations.md` for
/// the full permission tier breakdown.
pub fn check_permission(
    state: &ServerState,
    author: &EndpointId,
    kind: &EventKind,
) -> Result<(), String> {
    // Governance: Propose/Vote require admin.
    match kind {
        EventKind::Propose { .. } | EventKind::Vote { .. } => {
            if !state.is_admin(author) {
                return Err("not an admin".into());
            }
            return Ok(());
        }
        EventKind::CreateServer { .. } => return Ok(()),
        _ => {}
    }

    // Admin-only events.
    if matches!(
        kind,
        EventKind::GrantPermission { .. }
            | EventKind::RevokePermission { .. }
            | EventKind::RenameServer { .. }
            | EventKind::SetServerDescription { .. }
    ) && !state.is_admin(author)
    {
        return Err(format!("author '{}' is not an admin", author));
    }

    // Permission-gated events.
    if let Some(ref perm) = required_permission(kind) {
        if !state.has_permission(author, perm) {
            return Err(format!("author '{}' lacks {:?} permission", author, perm));
        }
    }

    Ok(())
}

// ───── Internal ────────────────────────────────────────────────────────────

/// Apply an event's mutation to state. Checks permissions via
/// [`check_permission`], then handles governance state mutations
/// (inserting proposals, recording votes) and delegates to
/// [`apply_mutation`] for everything else.
fn apply_event(state: &mut ServerState, event: &Event) -> ApplyResult {
    // Permission / authority check (read-only against state).
    if let Err(reason) = check_permission(state, &event.author, &event.kind) {
        return ApplyResult::Rejected(reason);
    }

    // Governance events mutate state after the permission check.
    match &event.kind {
        EventKind::CreateServer { .. } => {
            // No-op during replay — genesis data already extracted
            // by materialize() before the replay loop.
            return ApplyResult::Applied;
        }
        EventKind::Propose { action } => {
            state.pending_proposals.insert(
                event.hash,
                PendingProposal {
                    action: action.clone(),
                    proposer: event.author,
                    votes: BTreeMap::from([(event.author, true)]),
                },
            );
            check_and_apply_proposal(state, &event.hash);
            return ApplyResult::Applied;
        }
        EventKind::Vote { proposal, accept } => {
            match state.pending_proposals.get_mut(proposal) {
                Some(prop) => {
                    prop.votes.insert(event.author, *accept);
                }
                None => {
                    return ApplyResult::Rejected(format!("proposal {} not found", proposal));
                }
            }
            check_and_apply_proposal(state, proposal);
            return ApplyResult::Applied;
        }
        _ => {}
    }

    apply_mutation(state, event)
}

/// Check if a pending proposal has met the vote threshold.
fn check_and_apply_proposal(state: &mut ServerState, proposal: &EventHash) {
    let should_apply = state
        .pending_proposals
        .get(proposal)
        .map(|prop| {
            let yes_count = prop.votes.values().filter(|v| **v).count();
            // The genesis author (server owner) is the root of trust and can
            // push governance actions through unilaterally. This matches the
            // "Owner is root of trust" principle from the authority model spec.
            let owner_override = state
                .genesis_author
                .map(|owner| owner == prop.proposer)
                .unwrap_or(false);
            owner_override || state.meets_threshold(yes_count)
        })
        .unwrap_or(false);

    if should_apply {
        let prop = state.pending_proposals.remove(proposal).unwrap();
        apply_proposed_action(state, &prop.action);
    }
}

/// Apply a voted-on action to state.
fn apply_proposed_action(state: &mut ServerState, action: &ProposedAction) {
    match action {
        ProposedAction::GrantAdmin { peer_id } => {
            state.admins.insert(*peer_id);
            state.members.entry(*peer_id).or_insert_with(|| Member {
                peer_id: *peer_id,
                roles: BTreeSet::new(),
                display_name: None,
            });
        }
        ProposedAction::RevokeAdmin { peer_id } => {
            // Prevent 0-admin state — server becomes permanently ungovernable.
            if state.admins.len() == 1 && state.admins.contains(peer_id) {
                return;
            }
            state.admins.remove(peer_id);
            cleanup_votes_and_reevaluate(state, peer_id);
        }
        ProposedAction::KickMember { peer_id } => {
            // Prevent 0-admin state — server becomes permanently ungovernable.
            if state.admins.len() == 1 && state.admins.contains(peer_id) {
                return;
            }
            state.members.remove(peer_id);
            state.peer_permissions.remove(peer_id);
            state.admins.remove(peer_id);
            cleanup_votes_and_reevaluate(state, peer_id);
        }
        ProposedAction::SetVoteThreshold { threshold } => {
            state.vote_threshold = threshold.clone();
            reevaluate_all_proposals(state);
        }
    }
}

/// Remove a peer's votes from all pending proposals, then re-evaluate.
fn cleanup_votes_and_reevaluate(state: &mut ServerState, peer_id: &EndpointId) {
    for prop in state.pending_proposals.values_mut() {
        prop.votes.remove(peer_id);
    }
    reevaluate_all_proposals(state);
}

/// Re-check all pending proposals against the current threshold.
fn reevaluate_all_proposals(state: &mut ServerState) {
    let passing: Vec<EventHash> = state
        .pending_proposals
        .iter()
        .filter(|(_, prop)| {
            let yes_count = prop.votes.values().filter(|v| **v).count();
            state.meets_threshold(yes_count)
        })
        .map(|(hash, _)| *hash)
        .collect();

    for hash in passing {
        if let Some(prop) = state.pending_proposals.remove(&hash) {
            apply_proposed_action(state, &prop.action);
        }
    }
}

/// Map an EventKind to its required Permission (if any).
///
/// This is the permission-gated enforcement table. See
/// `docs/specs/2026-04-12-state-authority-and-mutations.md` for the full authority model,
/// including which variants are checked elsewhere (governance block,
/// admin-only block) and which are intentionally unrestricted.
fn required_permission(kind: &EventKind) -> Option<Permission> {
    match kind {
        EventKind::Message { .. }
        | EventKind::EditMessage { .. }
        | EventKind::DeleteMessage { .. }
        | EventKind::Reaction { .. } => Some(Permission::SendMessages),

        EventKind::CreateChannel { .. }
        | EventKind::DeleteChannel { .. }
        | EventKind::RenameChannel { .. }
        | EventKind::RotateChannelKey { .. } => Some(Permission::ManageChannels),

        EventKind::CreateRole { .. }
        | EventKind::DeleteRole { .. }
        | EventKind::SetPermission { .. }
        | EventKind::AssignRole { .. } => Some(Permission::ManageRoles),

        // Variants that intentionally return None:
        //   CreateServer        — genesis, checked structurally
        //   Propose, Vote       — governance, checked in the governance block above
        //   GrantPermission,
        //   RevokePermission,
        //   RenameServer,
        //   SetServerDescription — admin-only, checked in the admin block above
        //   SetProfile          — unrestricted (any member)
        //   UpdateProfile       — unrestricted (any member; self-authorship
        //                         is the only identity check)
        //   PinMessage,
        //   UnpinMessage        — unrestricted (any member)
        //   MuteChannel,
        //   MuteGrove           — per-identity preference, never gated
        //
        // If a new EventKind variant is added and is NOT listed here or
        // in an arm above, it will silently get no permission check.
        // That is a bug. See docs/specs/2026-04-12-state-authority-and-mutations.md § "Adding a
        // new event kind" for the required checklist.
        _ => None,
    }
}

/// Apply the state mutation for a non-governance event.
fn apply_mutation(state: &mut ServerState, event: &Event) -> ApplyResult {
    match &event.kind {
        EventKind::CreateChannel {
            name,
            channel_id,
            kind,
            ephemeral,
        } => {
            // Bound check on idle threshold — `[1h, 90d]`. Reject
            // out-of-range events so the wire cap and the create
            // dialog clamp share the same enforcement.
            if let Some(cfg) = ephemeral.as_ref() {
                if cfg.idle_threshold_ms < crate::ephemeral::IDLE_THRESHOLD_MIN_MS
                    || cfg.idle_threshold_ms > crate::ephemeral::IDLE_THRESHOLD_MAX_MS
                {
                    return ApplyResult::Rejected(format!(
                        "ephemeral idle_threshold_ms {} out of range [{}, {}]",
                        cfg.idle_threshold_ms,
                        crate::ephemeral::IDLE_THRESHOLD_MIN_MS,
                        crate::ephemeral::IDLE_THRESHOLD_MAX_MS,
                    ));
                }
            }
            if !state.channels.contains_key(channel_id) {
                state.channels.insert(
                    channel_id.clone(),
                    Channel {
                        id: channel_id.clone(),
                        name: name.clone(),
                        pinned_messages: BTreeSet::new(),
                        kind: kind.clone(),
                        ephemeral: ephemeral.clone(),
                        last_activity_hlc: None,
                    },
                );
            }
        }

        EventKind::DeleteChannel { channel_id } => {
            state.channels.remove(channel_id);
            let mut kept = Vec::with_capacity(state.messages.len());
            state.message_index.clear();
            for msg in state.messages.drain(..) {
                if msg.channel_id != *channel_id {
                    state.message_index.insert(msg.id, kept.len());
                    kept.push(msg);
                }
            }
            state.messages = kept;
        }

        EventKind::RenameChannel {
            channel_id,
            new_name,
        } => {
            if let Some(ch) = state.channels.get_mut(channel_id) {
                ch.name = new_name.clone();
            }
        }

        EventKind::CreateRole { name, role_id } => {
            if !state.roles.contains_key(role_id) {
                state.roles.insert(
                    role_id.clone(),
                    crate::types::Role {
                        id: role_id.clone(),
                        name: name.clone(),
                        permissions: BTreeSet::new(),
                    },
                );
            }
        }

        EventKind::DeleteRole { role_id } => {
            state.roles.remove(role_id);
            for member in state.members.values_mut() {
                member.roles.remove(role_id);
            }
        }

        EventKind::SetPermission {
            role_id,
            permission,
            granted,
        } => {
            if let Some(role) = state.roles.get_mut(role_id) {
                if *granted {
                    role.permissions.insert(permission.clone());
                } else {
                    role.permissions.remove(permission);
                }
            }
        }

        EventKind::AssignRole { peer_id, role_id } => {
            if state.roles.contains_key(role_id) {
                if let Some(member) = state.members.get_mut(peer_id) {
                    member.roles.insert(role_id.clone());
                }
            }
        }

        EventKind::GrantPermission {
            peer_id,
            permission,
        } => {
            state
                .peer_permissions
                .entry(*peer_id)
                .or_default()
                .insert(*permission);
            state.members.entry(*peer_id).or_insert_with(|| Member {
                peer_id: *peer_id,
                roles: BTreeSet::new(),
                display_name: None,
            });
        }

        EventKind::RevokePermission {
            peer_id,
            permission,
        } => {
            if let Some(perms) = state.peer_permissions.get_mut(peer_id) {
                perms.remove(permission);
                if perms.is_empty() {
                    state.peer_permissions.remove(peer_id);
                }
            }
        }

        EventKind::Message {
            channel_id,
            body,
            reply_to,
        } => {
            let idx = state.messages.len();
            state.messages.push(ChatMessage {
                id: event.hash,
                channel_id: channel_id.clone(),
                author: event.author,
                body: body.clone(),
                timestamp_ms: event.timestamp_hint_ms,
                edited: false,
                deleted: false,
                reactions: BTreeMap::new(),
                reply_to: *reply_to,
            });
            state.message_index.insert(event.hash, idx);
            // Advance the channel's last_activity_hlc on every Message.
            // Tracked unconditionally — permanent channels carry it too —
            // so the branch stays simple and a future feature can reuse
            // it. Spec: ephemeral-channels.md §Inactivity ladder.
            if let Some(ch) = state.channels.get_mut(channel_id) {
                ch.last_activity_hlc = Some(event.timestamp_hint_ms);
            }
        }

        EventKind::EditMessage {
            message_id,
            new_body,
        } => {
            if let Some(&idx) = state.message_index.get(message_id) {
                if let Some(msg) = state.messages.get_mut(idx) {
                    if msg.author == event.author {
                        msg.body = new_body.clone();
                        msg.edited = true;
                    }
                }
            }
        }

        EventKind::DeleteMessage { message_id } => {
            if let Some(&idx) = state.message_index.get(message_id) {
                if let Some(msg) = state.messages.get_mut(idx) {
                    if msg.author == event.author {
                        msg.deleted = true;
                        msg.body = "[message deleted]".to_string();
                        msg.reactions.clear();
                    }
                }
            }
        }

        EventKind::Reaction { message_id, emoji } => {
            if let Some(&idx) = state.message_index.get(message_id) {
                if let Some(msg) = state.messages.get_mut(idx) {
                    msg.reactions
                        .entry(emoji.clone())
                        .or_default()
                        .insert(event.author);
                }
            }
        }

        EventKind::SetProfile { display_name } => {
            let entry = state
                .profiles
                .entry(event.author)
                .or_insert_with(|| Profile::new(event.author));
            entry.display_name = display_name.clone();
            if let Some(member) = state.members.get_mut(&event.author) {
                member.display_name = Some(display_name.clone());
            }
        }

        EventKind::UpdateProfile(delta) => {
            let crate::types::ProfileDelta {
                display_name,
                pronouns,
                bio,
                tagline,
                crest_pattern,
                crest_color,
                pinned,
                elsewhere,
                since,
            } = delta.as_ref();
            let entry = state
                .profiles
                .entry(event.author)
                .or_insert_with(|| Profile::new(event.author));
            if let Some(name) = display_name {
                entry.display_name = name.clone();
                if let Some(member) = state.members.get_mut(&event.author) {
                    member.display_name = Some(name.clone());
                }
            }
            if let Some(v) = pronouns {
                entry.pronouns = v.as_ref().map(|s| truncate_chars(s, PROFILE_CAP_PRONOUNS));
            }
            if let Some(v) = bio {
                entry.bio = v.as_ref().map(|s| truncate_chars(s, PROFILE_CAP_BIO));
            }
            if let Some(v) = tagline {
                entry.tagline = v.as_ref().map(|s| truncate_chars(s, PROFILE_CAP_TAGLINE));
            }
            if let Some(v) = crest_pattern {
                entry.crest_pattern = *v;
            }
            if let Some(v) = crest_color {
                // Only accept valid `#RRGGBB` shapes; everything else drops
                // to `None` so the banner falls back to `--moss-2`.
                entry.crest_color = v.as_ref().and_then(|s| {
                    let t = truncate_chars(s, PROFILE_CAP_CREST_COLOR);
                    if t.len() == PROFILE_CAP_CREST_COLOR && t.starts_with('#') {
                        Some(t)
                    } else {
                        None
                    }
                });
            }
            if let Some(v) = pinned {
                entry.pinned = v.as_ref().map(|p| PinnedFragment {
                    kind: p.kind,
                    body: truncate_chars(&p.body, PROFILE_CAP_PINNED_BODY),
                });
            }
            if let Some(v) = elsewhere {
                entry.elsewhere = v
                    .iter()
                    .take(PROFILE_CAP_ELSEWHERE_LEN)
                    .map(|s| truncate_chars(s, PROFILE_CAP_ELSEWHERE_ENTRY))
                    .collect();
            }
            if let Some(v) = since {
                entry.since = v.as_ref().map(|s| truncate_chars(s, PROFILE_CAP_SINCE));
            }
        }

        EventKind::RotateChannelKey {
            channel_id,
            encrypted_keys,
        } => {
            // Defense-in-depth: reject if the author isn't a member of
            // the server. The ManageChannels permission check in
            // `required_permission` is the primary gate, but this guards
            // against any future code path that might grant permissions
            // without also adding the peer to `members`.
            if !state.members.contains_key(&event.author) {
                return ApplyResult::Rejected(format!("author '{}' is not a member", event.author));
            }
            if !encrypted_keys.is_empty() {
                let keys = state.channel_keys.entry(channel_id.clone()).or_default();
                for (peer_id, key_bytes) in encrypted_keys {
                    keys.insert(*peer_id, key_bytes.clone());
                }
            }
        }

        EventKind::PinMessage {
            channel_id,
            message_id,
        } => {
            if let Some(ch) = state.channels.get_mut(channel_id) {
                ch.pinned_messages.insert(*message_id);
            }
        }

        EventKind::UnpinMessage {
            channel_id,
            message_id,
        } => {
            if let Some(ch) = state.channels.get_mut(channel_id) {
                ch.pinned_messages.remove(message_id);
            }
        }

        EventKind::RenameServer { new_name } => {
            state.server_name = new_name.clone();
        }

        EventKind::SetServerDescription { description } => {
            state.description = description.clone();
        }

        EventKind::MuteChannel { channel_id, muted } => {
            // Per-identity preference — keyed by the event author so
            // each peer maintains its own view. No member check;
            // muting a channel the author isn't a member of is a
            // harmless idempotent no-op.
            let entry = state.mute_state.entry(event.author).or_default();
            if *muted {
                entry.channels.insert(channel_id.clone());
            } else {
                entry.channels.remove(channel_id);
            }
        }

        EventKind::MuteGrove { muted } => {
            let entry = state.mute_state.entry(event.author).or_default();
            entry.grove_muted = *muted;
        }

        // Governance events handled above in apply_event.
        EventKind::CreateServer { .. } | EventKind::Propose { .. } | EventKind::Vote { .. } => {}
    }

    ApplyResult::Applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::EventDag;
    use crate::event::{EventKind, ProposedAction, VoteThreshold};
    use willow_identity::Identity;

    fn genesis_kind() -> EventKind {
        EventKind::CreateServer {
            name: "Test Server".into(),
        }
    }

    fn test_dag(identity: &Identity) -> EventDag {
        let mut dag = EventDag::new();
        let genesis = dag.create_event(identity, genesis_kind(), vec![], 0);
        dag.insert(genesis).unwrap();
        dag
    }

    fn emit(dag: &mut EventDag, id: &Identity, kind: EventKind) -> Event {
        let event = dag.create_event(id, kind, vec![], 0);
        dag.insert(event.clone()).unwrap();
        event
    }

    fn emit_with_deps(
        dag: &mut EventDag,
        id: &Identity,
        kind: EventKind,
        deps: Vec<EventHash>,
    ) -> Event {
        let event = dag.create_event(id, kind, deps, 0);
        dag.insert(event.clone()).unwrap();
        event
    }

    // ── Basic materialization ──────────────────────────────────────

    #[test]
    fn materialize_empty_dag() {
        let id = Identity::generate();
        let dag = test_dag(&id);
        let state = materialize(&dag);
        assert!(state.is_admin(&id.endpoint_id()));
        assert!(state.members.contains_key(&id.endpoint_id()));
        assert_eq!(state.admins.len(), 1);
        assert!(state.channels.is_empty());
    }

    #[test]
    fn materialize_create_channel() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);
        emit(
            &mut dag,
            &id,
            EventKind::CreateChannel {
                name: "general".into(),
                channel_id: "ch-1".into(),
                kind: crate::types::ChannelKind::Text,
                ephemeral: None,
            },
        );
        let state = materialize(&dag);
        assert!(state.channels.contains_key("ch-1"));
        assert_eq!(state.channels["ch-1"].name, "general");
    }

    #[test]
    fn materialize_is_deterministic() {
        let id = Identity::generate();
        let mut dag = test_dag(&id);
        for i in 0..5 {
            emit(
                &mut dag,
                &id,
                EventKind::SetProfile {
                    display_name: format!("name{i}"),
                },
            );
        }
        let s1 = materialize(&dag);
        let s2 = materialize(&dag);
        assert_eq!(s1.profiles, s2.profiles);
        assert_eq!(s1.server_name, s2.server_name);
    }

    #[test]
    fn materialize_permission_enforcement() {
        let admin = Identity::generate();
        let stranger = Identity::generate();
        let mut dag = test_dag(&admin);

        // Stranger tries to create a channel without ManageChannels.
        emit(
            &mut dag,
            &stranger,
            EventKind::CreateChannel {
                name: "evil".into(),
                channel_id: "ch-evil".into(),
                kind: crate::types::ChannelKind::Text,
                ephemeral: None,
            },
        );
        let state = materialize(&dag);
        // Channel should not exist (stranger lacks permission).
        assert!(!state.channels.contains_key("ch-evil"));
    }

    #[test]
    fn materialize_genesis_author_is_admin() {
        let id = Identity::generate();
        let dag = test_dag(&id);
        let state = materialize(&dag);
        assert!(state.is_admin(&id.endpoint_id()));
        // Admin can do anything.
        assert!(state.has_permission(&id.endpoint_id(), &Permission::ManageChannels));
    }

    #[test]
    fn materialize_admin_has_all_permissions() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        // Admin creates a channel — should work.
        emit(
            &mut dag,
            &admin,
            EventKind::CreateChannel {
                name: "general".into(),
                channel_id: "ch-1".into(),
                kind: crate::types::ChannelKind::Text,
                ephemeral: None,
            },
        );
        let state = materialize(&dag);
        assert!(state.channels.contains_key("ch-1"));
    }

    #[test]
    fn materialize_message_in_channel() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        // Admin can send messages (admins have all permissions).
        let msg = emit(
            &mut dag,
            &admin,
            EventKind::Message {
                channel_id: "general".into(),
                body: "hello".into(),
                reply_to: None,
            },
        );
        let state = materialize(&dag);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].id, msg.hash);
        assert_eq!(state.messages[0].body, "hello");
    }

    #[test]
    fn materialize_edit_message() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        let msg = emit(
            &mut dag,
            &admin,
            EventKind::Message {
                channel_id: "general".into(),
                body: "typo".into(),
                reply_to: None,
            },
        );
        emit(
            &mut dag,
            &admin,
            EventKind::EditMessage {
                message_id: msg.hash,
                new_body: "fixed".into(),
            },
        );
        let state = materialize(&dag);
        assert_eq!(state.messages[0].body, "fixed");
        assert!(state.messages[0].edited);
    }

    #[test]
    fn materialize_delete_message() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        let msg = emit(
            &mut dag,
            &admin,
            EventKind::Message {
                channel_id: "general".into(),
                body: "to delete".into(),
                reply_to: None,
            },
        );
        emit(
            &mut dag,
            &admin,
            EventKind::DeleteMessage {
                message_id: msg.hash,
            },
        );
        let state = materialize(&dag);
        assert!(state.messages[0].deleted);
        assert_eq!(state.messages[0].body, "[message deleted]");
    }

    #[test]
    fn materialize_reaction() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        let msg = emit(
            &mut dag,
            &admin,
            EventKind::Message {
                channel_id: "general".into(),
                body: "react to me".into(),
                reply_to: None,
            },
        );
        emit(
            &mut dag,
            &admin,
            EventKind::Reaction {
                message_id: msg.hash,
                emoji: "👍".into(),
            },
        );
        let state = materialize(&dag);
        assert!(state.messages[0].reactions.contains_key("👍"));
    }

    #[test]
    fn materialize_set_profile() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        emit(
            &mut dag,
            &admin,
            EventKind::SetProfile {
                display_name: "Alice".into(),
            },
        );
        let state = materialize(&dag);
        assert_eq!(state.profiles[&admin.endpoint_id()].display_name, "Alice");
        assert_eq!(
            state.members[&admin.endpoint_id()].display_name,
            Some("Alice".into())
        );
    }

    #[test]
    fn materialize_rename_server_admin_only() {
        let admin = Identity::generate();
        let stranger = Identity::generate();
        let mut dag = test_dag(&admin);

        // Admin renames — works.
        emit(
            &mut dag,
            &admin,
            EventKind::RenameServer {
                new_name: "New Name".into(),
            },
        );
        // Stranger renames — rejected.
        emit(
            &mut dag,
            &stranger,
            EventKind::RenameServer {
                new_name: "Hacked".into(),
            },
        );
        let state = materialize(&dag);
        assert_eq!(state.server_name, "New Name");
    }

    #[test]
    fn materialize_server_description_admin_only() {
        let admin = Identity::generate();
        let stranger = Identity::generate();
        let mut dag = test_dag(&admin);

        emit(
            &mut dag,
            &admin,
            EventKind::SetServerDescription {
                description: "A great server".into(),
            },
        );
        emit(
            &mut dag,
            &stranger,
            EventKind::SetServerDescription {
                description: "Hacked".into(),
            },
        );
        let state = materialize(&dag);
        assert_eq!(state.description, "A great server");
    }

    #[test]
    fn materialize_delete_channel_cascades_messages() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        emit(
            &mut dag,
            &admin,
            EventKind::CreateChannel {
                name: "doomed".into(),
                channel_id: "ch-d".into(),
                kind: crate::types::ChannelKind::Text,
                ephemeral: None,
            },
        );
        emit(
            &mut dag,
            &admin,
            EventKind::Message {
                channel_id: "ch-d".into(),
                body: "will be deleted".into(),
                reply_to: None,
            },
        );
        emit(
            &mut dag,
            &admin,
            EventKind::DeleteChannel {
                channel_id: "ch-d".into(),
            },
        );
        let state = materialize(&dag);
        assert!(!state.channels.contains_key("ch-d"));
        assert!(state.messages.is_empty());
    }

    #[test]
    fn materialize_delete_role_cascades_members() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        emit(
            &mut dag,
            &admin,
            EventKind::CreateRole {
                name: "mod".into(),
                role_id: "r-1".into(),
            },
        );
        emit(
            &mut dag,
            &admin,
            EventKind::AssignRole {
                peer_id: admin.endpoint_id(),
                role_id: "r-1".into(),
            },
        );
        emit(
            &mut dag,
            &admin,
            EventKind::DeleteRole {
                role_id: "r-1".into(),
            },
        );
        let state = materialize(&dag);
        assert!(!state.roles.contains_key("r-1"));
        assert!(!state.members[&admin.endpoint_id()].roles.contains("r-1"));
    }

    #[test]
    fn materialize_grant_permission_adds_member() {
        let admin = Identity::generate();
        let new_peer = Identity::generate();
        let mut dag = test_dag(&admin);
        emit(
            &mut dag,
            &admin,
            EventKind::GrantPermission {
                peer_id: new_peer.endpoint_id(),
                permission: Permission::SendMessages,
            },
        );
        let state = materialize(&dag);
        assert!(state.members.contains_key(&new_peer.endpoint_id()));
        assert!(state.has_permission(&new_peer.endpoint_id(), &Permission::SendMessages));
    }

    // ── Incremental apply ──────────────────────────────────────────

    #[test]
    fn incremental_matches_full_materialize() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);

        // Build a state incrementally.
        let mut incremental = materialize(&dag);
        for i in 0..5 {
            let e = emit(
                &mut dag,
                &admin,
                EventKind::SetProfile {
                    display_name: format!("name{i}"),
                },
            );
            apply_incremental(&mut incremental, &e);
        }

        let full = materialize(&dag);
        assert_eq!(incremental.profiles, full.profiles);
    }

    // ── Governance tests ───────────────────────────────────────────

    #[test]
    fn propose_requires_admin() {
        let admin = Identity::generate();
        let stranger = Identity::generate();
        let mut dag = test_dag(&admin);
        emit(
            &mut dag,
            &stranger,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: stranger.endpoint_id(),
                },
            },
        );
        let state = materialize(&dag);
        // Stranger's proposal was rejected — they're not admin.
        assert!(!state.is_admin(&stranger.endpoint_id()));
    }

    #[test]
    fn vote_requires_admin() {
        let admin = Identity::generate();
        let stranger = Identity::generate();
        let bob = Identity::generate();
        let mut dag = test_dag(&admin);
        let prop = emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: bob.endpoint_id(),
                },
            },
        );
        // Stranger votes — should be rejected (not admin).
        emit_with_deps(
            &mut dag,
            &stranger,
            EventKind::Vote {
                proposal: prop.hash,
                accept: true,
            },
            vec![prop.hash],
        );
        let state = materialize(&dag);
        // With 1 admin and majority, the proposal auto-applied on Propose.
        // Stranger's vote was rejected but doesn't affect the outcome.
        assert!(state.is_admin(&bob.endpoint_id()));
    }

    #[test]
    fn sole_admin_propose_auto_applies() {
        let admin = Identity::generate();
        let alice = Identity::generate();
        let mut dag = test_dag(&admin);
        emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: alice.endpoint_id(),
                },
            },
        );
        let state = materialize(&dag);
        // Sole admin: majority of 1 = 1. Proposer is implicit yes.
        assert!(state.is_admin(&alice.endpoint_id()));
    }

    #[test]
    fn vote_auto_applies_on_threshold() {
        let admin = Identity::generate();
        let alice = Identity::generate();
        let bob = Identity::generate();
        let mut dag = test_dag(&admin);

        // Add alice as admin (sole admin, auto-applies).
        emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: alice.endpoint_id(),
                },
            },
        );

        // Now 2 admins. Propose to add bob.
        let prop = emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: bob.endpoint_id(),
                },
            },
        );

        // Majority of 2 = need 2. Admin proposed (1 yes). Alice votes yes.
        emit_with_deps(
            &mut dag,
            &alice,
            EventKind::Vote {
                proposal: prop.hash,
                accept: true,
            },
            vec![prop.hash],
        );

        let state = materialize(&dag);
        assert!(state.is_admin(&bob.endpoint_id()));
    }

    #[test]
    fn vote_does_not_apply_below_threshold() {
        // The genesis author can bypass the majority threshold, but a regular
        // (non-genesis) admin cannot — their proposals require majority vote.
        let genesis = Identity::generate();
        let alice = Identity::generate();
        let bob = Identity::generate();
        let carol = Identity::generate();
        let mut dag = test_dag(&genesis);

        // Genesis author promotes alice (sole admin — auto-applies with 1 vote).
        emit(
            &mut dag,
            &genesis,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: alice.endpoint_id(),
                },
            },
        );

        // Now 2 admins. Alice (non-genesis) proposes bob — 1/2 votes, stays pending.
        emit(
            &mut dag,
            &alice,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: bob.endpoint_id(),
                },
            },
        );

        // Alice also proposes carol — 1/2 votes, stays pending.
        emit(
            &mut dag,
            &alice,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: carol.endpoint_id(),
                },
            },
        );

        let state = materialize(&dag);
        // Carol should NOT be admin (alice is not genesis author — needs majority).
        assert!(!state.is_admin(&carol.endpoint_id()));
        // Bob also not admin (alice's vote alone doesn't satisfy majority of 2).
        assert!(!state.is_admin(&bob.endpoint_id()));
    }

    #[test]
    fn propose_grant_admin_full_flow() {
        let admin = Identity::generate();
        let alice = Identity::generate();
        let bob = Identity::generate();
        let mut dag = test_dag(&admin);

        // Add alice.
        emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: alice.endpoint_id(),
                },
            },
        );

        // Add bob: admin proposes, alice votes yes. 2/2 = passes.
        let prop = emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: bob.endpoint_id(),
                },
            },
        );
        // Alice's vote must causally depend on the proposal.
        emit_with_deps(
            &mut dag,
            &alice,
            EventKind::Vote {
                proposal: prop.hash,
                accept: true,
            },
            vec![prop.hash],
        );

        let state = materialize(&dag);
        assert!(state.is_admin(&admin.endpoint_id()));
        assert!(state.is_admin(&alice.endpoint_id()));
        assert!(state.is_admin(&bob.endpoint_id()));
        assert_eq!(state.admins.len(), 3);
    }

    #[test]
    fn propose_kick_member_full_flow() {
        let admin = Identity::generate();
        let alice = Identity::generate();
        let target = Identity::generate();
        let mut dag = test_dag(&admin);

        // Add alice as admin.
        emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: alice.endpoint_id(),
                },
            },
        );
        // Grant target SendMessages so they become a member.
        emit(
            &mut dag,
            &admin,
            EventKind::GrantPermission {
                peer_id: target.endpoint_id(),
                permission: Permission::SendMessages,
            },
        );

        // Kick target: admin proposes, alice votes.
        let prop = emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::KickMember {
                    peer_id: target.endpoint_id(),
                },
            },
        );
        emit_with_deps(
            &mut dag,
            &alice,
            EventKind::Vote {
                proposal: prop.hash,
                accept: true,
            },
            vec![prop.hash],
        );

        let state = materialize(&dag);
        assert!(!state.members.contains_key(&target.endpoint_id()));
        assert!(!state.peer_permissions.contains_key(&target.endpoint_id()));
    }

    #[test]
    fn propose_set_vote_threshold() {
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);
        emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::SetVoteThreshold {
                    threshold: VoteThreshold::Unanimous,
                },
            },
        );
        let state = materialize(&dag);
        assert_eq!(state.vote_threshold, VoteThreshold::Unanimous);
    }

    #[test]
    fn vote_on_passed_proposal_ignored() {
        let admin = Identity::generate();
        let alice = Identity::generate();
        let mut dag = test_dag(&admin);

        // Add alice (auto-applies with sole admin).
        let prop = emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: alice.endpoint_id(),
                },
            },
        );
        // Proposal already passed. Late vote from alice — no crash, no-op.
        emit_with_deps(
            &mut dag,
            &alice,
            EventKind::Vote {
                proposal: prop.hash,
                accept: true,
            },
            vec![prop.hash],
        );
        let state = materialize(&dag);
        assert!(state.is_admin(&alice.endpoint_id()));
    }

    #[test]
    fn grant_permission_requires_admin() {
        let admin = Identity::generate();
        let stranger = Identity::generate();
        let target = Identity::generate();
        let mut dag = test_dag(&admin);

        // Stranger tries to grant permission — rejected.
        emit(
            &mut dag,
            &stranger,
            EventKind::GrantPermission {
                peer_id: target.endpoint_id(),
                permission: Permission::SendMessages,
            },
        );
        let state = materialize(&dag);
        assert!(!state.peer_permissions.contains_key(&target.endpoint_id()));
    }

    #[test]
    fn kick_cleans_up_pending_votes() {
        let admin = Identity::generate();
        let alice = Identity::generate();
        let bob = Identity::generate();
        let mut dag = test_dag(&admin);

        // Add alice as admin (sole admin, auto-applies).
        emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: alice.endpoint_id(),
                },
            },
        );
        // After adding alice (2 admins), add bob needs 2 votes.
        let add_bob = emit(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::GrantAdmin {
                    peer_id: bob.endpoint_id(),
                },
            },
        );
        let alice_vote = emit_with_deps(
            &mut dag,
            &alice,
            EventKind::Vote {
                proposal: add_bob.hash,
                accept: true,
            },
            vec![add_bob.hash],
        );
        // Now 3 admins. Majority of 3 = 2.

        // Bob votes on a new proposal. The proposal must causally follow
        // alice's vote (which grants bob admin status), otherwise the
        // topo sort may place the proposal before alice's vote and bob
        // won't be an admin when his vote is processed.
        let prop = emit_with_deps(
            &mut dag,
            &admin,
            EventKind::Propose {
                action: ProposedAction::SetVoteThreshold {
                    threshold: VoteThreshold::Unanimous,
                },
            },
            vec![alice_vote.hash],
        );
        emit_with_deps(
            &mut dag,
            &bob,
            EventKind::Vote {
                proposal: prop.hash,
                accept: true,
            },
            vec![prop.hash],
        );
        // 2/3 voted yes (admin + bob). Majority met → threshold changes to Unanimous.

        let state = materialize(&dag);
        assert_eq!(state.vote_threshold, VoteThreshold::Unanimous);
    }
}
