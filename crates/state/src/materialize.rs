//! State materialization — projecting the DAG into [`ServerState`].
//!
//! The [`materialize`] function is the ONLY way to derive state from a
//! DAG. It topologically sorts all events and replays them through
//! [`apply_event`], producing identical output on all peers given the
//! same DAG contents.

use std::collections::{BTreeMap, BTreeSet};

use willow_identity::EndpointId;

use crate::dag::EventDag;
use crate::event::{Event, EventKind, Permission, ProposedAction, MAX_ENCRYPTED_KEYS_OVER_MEMBERS};
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

/// Maximum UTF-8 byte length of a `Reaction.emoji` string.
///
/// Emoji codepoints are short — even multi-codepoint ZWJ sequences fit
/// in ~28 bytes. Capping at 32 bytes prevents a peer with
/// `SendMessages` permission from broadcasting multi-MB strings as
/// reaction keys, which would replicate to every receiver. Measured in
/// bytes (not chars) because byte length is what drives wire-format
/// storage cost. See issue #615.
pub const MAX_REACTION_EMOJI_BYTES: usize = 32;

/// Maximum number of distinct reaction keys per message.
///
/// Without this cap, a single peer can issue N events with N distinct
/// emoji to grow `ChatMessage.reactions` cardinality unboundedly,
/// since each unique key clones at every replay. See issue #615.
pub const MAX_REACTIONS_PER_MESSAGE: usize = 32;

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
        //   SetProfile          — any current member; membership gate
        //                         lives in apply_mutation (issue #177)
        //   UpdateProfile       — any current member; membership gate
        //                         lives in apply_mutation. Self-
        //                         authorship is enforced structurally
        //                         (only the author's own profile is
        //                         mutated). See issue #177.
        //   PinMessage,
        //   UnpinMessage        — any current member; membership gate
        //                         lives in apply_mutation (issue #177)
        //   ChannelRevive       — membership check lives in apply_mutation
        //                         (does not require SendMessages so a muted
        //                         member can still un-archive)
        //   MuteChannel,
        //   MuteGrove           — per-identity preference, never gated
        //                         (preferences are not server state and
        //                         survive a kick)
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
            if name.chars().count() > 100 {
                return ApplyResult::Rejected(format!(
                    "channel name exceeds 100 chars ({} chars)",
                    name.chars().count()
                ));
            }
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
            if new_name.chars().count() > 100 {
                return ApplyResult::Rejected(format!(
                    "channel name exceeds 100 chars ({} chars)",
                    new_name.chars().count()
                ));
            }
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
            // Drop the unknown-legacy sentinel produced by the
            // back-compat deserialize path so a rogue / future client
            // can never inject an unrecognised permission name into a
            // role's permission set.
            if matches!(permission, Permission::__UnknownLegacy) {
                tracing::warn!(
                    role_id = %role_id,
                    "SetPermission with unknown legacy permission; dropping",
                );
                return ApplyResult::Applied;
            }
            if let Some(role) = state.roles.get_mut(role_id) {
                if *granted {
                    role.permissions.insert(*permission);
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
            // Reject oversized emoji strings — bounds DAG-time storage
            // cost, since each unique emoji is cloned as a BTreeMap key
            // on every receiver. See issue #615.
            if emoji.len() > MAX_REACTION_EMOJI_BYTES {
                return ApplyResult::Rejected(format!(
                    "reaction emoji exceeds {} bytes ({} bytes)",
                    MAX_REACTION_EMOJI_BYTES,
                    emoji.len()
                ));
            }
            if let Some(&idx) = state.message_index.get(message_id) {
                if let Some(msg) = state.messages.get_mut(idx) {
                    // Reject if adding a *new* reaction key would push
                    // cardinality past the cap. Adding an author to an
                    // existing emoji key is fine — that doesn't grow
                    // cardinality. See issue #615.
                    if !msg.reactions.contains_key(emoji)
                        && msg.reactions.len() >= MAX_REACTIONS_PER_MESSAGE
                    {
                        return ApplyResult::Rejected(format!(
                            "message already has {} distinct reactions (cap)",
                            MAX_REACTIONS_PER_MESSAGE
                        ));
                    }
                    msg.reactions
                        .entry(emoji.clone())
                        .or_default()
                        .insert(event.author);
                }
            }
        }

        EventKind::SetProfile { display_name } => {
            // Defense-in-depth: reject if the author isn't a member of
            // the server. `required_permission()` returns `None` for
            // SetProfile (it's "any current member"), so the membership
            // gate must live here. Without it, late-arriving events
            // from a kicked or never-joined signer would silently
            // mutate `state.profiles`. See issue #177.
            if !state.members.contains_key(&event.author) {
                return ApplyResult::Rejected(format!("author '{}' is not a member", event.author));
            }
            if display_name.chars().count() > 64 {
                return ApplyResult::Rejected(format!(
                    "display name exceeds 64 chars ({} chars)",
                    display_name.chars().count()
                ));
            }
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
            // Defense-in-depth: reject if the author isn't a member of
            // the server. `required_permission()` returns `None` for
            // UpdateProfile (it's "any current member"; self-authorship
            // is already structurally enforced by mutating only the
            // author's own profile). The membership gate must live
            // here so late-arriving events from a kicked or never-
            // joined signer cannot silently mutate `state.profiles`.
            // See issue #177.
            if !state.members.contains_key(&event.author) {
                return ApplyResult::Rejected(format!("author '{}' is not a member", event.author));
            }
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
            // Mirror SetProfile's display_name cap: reject the entire
            // event if display_name exceeds 64 chars. Rejection (not
            // silent truncation like the sibling profile fields) is
            // chosen for parity with SetProfile and because display_name
            // is identity-tier — silent truncation could produce
            // confusing duplicate names ("alice" vs a truncated
            // "alice…"). See issue #614.
            if let Some(name) = display_name {
                if name.chars().count() > 64 {
                    return ApplyResult::Rejected(format!(
                        "display name exceeds 64 chars ({} chars)",
                        name.chars().count()
                    ));
                }
            }
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
            // Anti-DoS cap (SEC-V-07). A legitimate RotateChannelKey
            // carries at most one entry per current member; epsilon
            // absorbs benign races between membership changes and key
            // rotation. Anything beyond that is a fabricated-id flood —
            // every entry would otherwise `.clone()` into the per-server
            // `BTreeMap<EndpointId, Vec<u8>>` on every peer.
            let cap = state
                .members
                .len()
                .saturating_add(MAX_ENCRYPTED_KEYS_OVER_MEMBERS);
            if encrypted_keys.len() > cap {
                return ApplyResult::Rejected(format!(
                    "RotateChannelKey: {} encrypted_keys exceeds cap {} (members={} + epsilon={})",
                    encrypted_keys.len(),
                    cap,
                    state.members.len(),
                    MAX_ENCRYPTED_KEYS_OVER_MEMBERS,
                ));
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
            // Defense-in-depth: reject if the author isn't a member of
            // the server. `required_permission()` returns `None` for
            // PinMessage (it's "any current member"), so the membership
            // gate must live here. Without it, late-arriving events
            // from a kicked or never-joined signer would silently
            // mutate `pinned_messages`. See issue #177.
            if !state.members.contains_key(&event.author) {
                return ApplyResult::Rejected(format!("author '{}' is not a member", event.author));
            }
            if let Some(ch) = state.channels.get_mut(channel_id) {
                ch.pinned_messages.insert(*message_id);
            }
        }

        EventKind::UnpinMessage {
            channel_id,
            message_id,
        } => {
            // Defense-in-depth: reject if the author isn't a member of
            // the server. `required_permission()` returns `None` for
            // UnpinMessage (it's "any current member"), so the
            // membership gate must live here. Without it, late-
            // arriving events from a kicked or never-joined signer
            // would silently mutate `pinned_messages`. See issue #177.
            if !state.members.contains_key(&event.author) {
                return ApplyResult::Rejected(format!("author '{}' is not a member", event.author));
            }
            if let Some(ch) = state.channels.get_mut(channel_id) {
                ch.pinned_messages.remove(message_id);
            }
        }

        EventKind::ChannelRevive { channel_id } => {
            // Member gate — same contract as Message emission, but
            // without the SendMessages permission requirement. A
            // muted member can still revive a channel they belong to,
            // even if they cannot post.
            if !state.members.contains_key(&event.author) {
                return ApplyResult::Rejected(format!(
                    "ChannelRevive: author '{}' is not a member",
                    event.author,
                ));
            }
            let ch = match state.channels.get_mut(channel_id) {
                Some(ch) => ch,
                None => {
                    return ApplyResult::Rejected(format!(
                        "ChannelRevive: channel '{channel_id}' not found"
                    ));
                }
            };
            // Idempotent on already-active channels — still advances
            // last_activity_hlc, which is a harmless no-op when the
            // channel was already active.
            ch.last_activity_hlc = Some(event.timestamp_hint_ms);
        }

        EventKind::RenameServer { new_name } => {
            if new_name.chars().count() > 100 {
                return ApplyResult::Rejected(format!(
                    "server name exceeds 100 chars ({} chars)",
                    new_name.chars().count()
                ));
            }
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

    // ── Name length caps (issue #189) ──────────────────────────────

    #[test]
    fn name_length_caps_are_utf8_aware() {
        // 100 crab emoji ('🦀') is 100 chars but 400 bytes — must be accepted
        // since the cap is on .chars().count(), not .len(). 101 crabs must be
        // rejected.
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);

        let ok_name: String = "🦀".repeat(100);
        let too_long: String = "🦀".repeat(101);

        // 100-char channel name accepted.
        emit(
            &mut dag,
            &admin,
            EventKind::CreateChannel {
                name: ok_name.clone(),
                channel_id: "ch-ok".into(),
                kind: crate::types::ChannelKind::Text,
                ephemeral: None,
            },
        );
        // 101-char channel name rejected.
        emit(
            &mut dag,
            &admin,
            EventKind::CreateChannel {
                name: too_long.clone(),
                channel_id: "ch-bad".into(),
                kind: crate::types::ChannelKind::Text,
                ephemeral: None,
            },
        );

        // 64-char display name accepted; 65-char rejected.
        let ok_display: String = "🦀".repeat(64);
        let bad_display: String = "🦀".repeat(65);
        emit(
            &mut dag,
            &admin,
            EventKind::SetProfile {
                display_name: ok_display.clone(),
            },
        );
        let state_after_ok = materialize(&dag);
        assert_eq!(
            state_after_ok.profiles[&admin.endpoint_id()].display_name,
            ok_display
        );

        emit(
            &mut dag,
            &admin,
            EventKind::SetProfile {
                display_name: bad_display,
            },
        );

        let state = materialize(&dag);
        // 100-char crab channel survived.
        assert!(state.channels.contains_key("ch-ok"));
        assert_eq!(state.channels["ch-ok"].name, ok_name);
        // 101-char channel was rejected.
        assert!(!state.channels.contains_key("ch-bad"));
        // Display name pinned at the 64-char value (rejected event left it intact).
        assert_eq!(
            state.profiles[&admin.endpoint_id()].display_name,
            ok_display
        );
    }

    #[test]
    fn update_profile_display_name_over_64_chars_rejected() {
        // Mirrors the SetProfile cap (issue #614): an UpdateProfile event
        // carrying a >64-char display_name must be rejected so a
        // misbehaving peer cannot DoS receivers by broadcasting a
        // multi-megabyte name (`Some("a".repeat(10_000_000))`). At the
        // same time, a 64-char display name must still apply, and the
        // sibling fields (which use silent truncation) must not regress.
        let admin = Identity::generate();
        let mut dag = test_dag(&admin);

        // First, set a known-good display name via SetProfile so we have
        // a baseline value to assert against after the rejected event.
        let baseline = "alice".to_string();
        emit(
            &mut dag,
            &admin,
            EventKind::SetProfile {
                display_name: baseline.clone(),
            },
        );

        // 64 crabs (256 bytes) must apply — char-count, not byte-length.
        let ok_display: String = "🦀".repeat(64);
        emit(
            &mut dag,
            &admin,
            EventKind::UpdateProfile(Box::new(crate::types::ProfileDelta {
                display_name: Some(ok_display.clone()),
                ..Default::default()
            })),
        );
        let state_after_ok = materialize(&dag);
        assert_eq!(
            state_after_ok.profiles[&admin.endpoint_id()].display_name,
            ok_display,
            "64-char UpdateProfile display_name must apply",
        );
        assert_eq!(
            state_after_ok.members[&admin.endpoint_id()].display_name,
            Some(ok_display.clone()),
            "members entry must mirror profiles entry",
        );

        // 65 crabs must be rejected — and the prior value must remain.
        let bad_display: String = "🦀".repeat(65);
        emit(
            &mut dag,
            &admin,
            EventKind::UpdateProfile(Box::new(crate::types::ProfileDelta {
                display_name: Some(bad_display),
                // Pair with a benign sibling field; if the event were
                // (incorrectly) applied with truncation instead of
                // rejection, the pronouns side-effect would still land.
                // Rejection must drop the *whole* event.
                pronouns: Some(Some("they/them".into())),
                ..Default::default()
            })),
        );
        let state = materialize(&dag);
        // Display name pinned at the prior 64-char value.
        assert_eq!(
            state.profiles[&admin.endpoint_id()].display_name,
            ok_display,
            "rejected UpdateProfile must leave display_name unchanged",
        );
        // Sibling field must NOT have been applied — rejection drops the
        // whole event, not just the offending field.
        assert_eq!(
            state.profiles[&admin.endpoint_id()].pronouns,
            None,
            "rejected UpdateProfile must not apply sibling fields",
        );
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

    #[test]
    fn reaction_emoji_over_32_bytes_rejected() {
        // Issue #615: an oversized Reaction.emoji must be rejected, not
        // applied as a BTreeMap key, otherwise a peer with SendMessages
        // permission can broadcast a multi-MB string and force every
        // receiver to clone it on every replay.
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

        // 33 bytes of ASCII — one byte over the 32-byte cap.
        let bad_emoji: String = "a".repeat(MAX_REACTION_EMOJI_BYTES + 1);
        assert_eq!(bad_emoji.len(), MAX_REACTION_EMOJI_BYTES + 1);
        emit(
            &mut dag,
            &admin,
            EventKind::Reaction {
                message_id: msg.hash,
                emoji: bad_emoji.clone(),
            },
        );

        let state = materialize(&dag);
        assert!(
            !state.messages[0].reactions.contains_key(&bad_emoji),
            "oversized emoji must not appear as a reaction key",
        );
        assert!(
            state.messages[0].reactions.is_empty(),
            "no reactions should land when the only Reaction is rejected",
        );
    }

    #[test]
    fn reaction_cardinality_over_32_per_message_rejected() {
        // Issue #615: distinct reaction keys per message are capped at
        // MAX_REACTIONS_PER_MESSAGE. A 33rd distinct emoji must be
        // rejected; the first 32 must remain.
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

        // 32 distinct ASCII emojis (use 2-char strings so they're
        // unambiguously distinct keys, well under 32 bytes each).
        for i in 0..MAX_REACTIONS_PER_MESSAGE {
            let emoji = format!("e{i:02}");
            emit(
                &mut dag,
                &admin,
                EventKind::Reaction {
                    message_id: msg.hash,
                    emoji,
                },
            );
        }

        // 33rd distinct key — must be rejected.
        let overflow_emoji = "EXTRA".to_string();
        emit(
            &mut dag,
            &admin,
            EventKind::Reaction {
                message_id: msg.hash,
                emoji: overflow_emoji.clone(),
            },
        );

        let state = materialize(&dag);
        assert_eq!(
            state.messages[0].reactions.len(),
            MAX_REACTIONS_PER_MESSAGE,
            "exactly MAX_REACTIONS_PER_MESSAGE distinct keys must remain",
        );
        assert!(
            !state.messages[0].reactions.contains_key(&overflow_emoji),
            "33rd distinct reaction must not be added",
        );
    }

    #[test]
    fn reaction_existing_key_not_blocked_by_cardinality_cap() {
        // Issue #615: at the cap, applying a Reaction with an emoji
        // *already* present must still succeed — it only adds an author
        // to the existing set; cardinality does not grow.
        let admin = Identity::generate();
        let bob = Identity::generate();
        let mut dag = test_dag(&admin);

        // Trust bob so he can react.
        let grant = emit(
            &mut dag,
            &admin,
            EventKind::GrantPermission {
                peer_id: bob.endpoint_id(),
                permission: crate::event::Permission::SendMessages,
            },
        );
        let msg = emit(
            &mut dag,
            &admin,
            EventKind::Message {
                channel_id: "general".into(),
                body: "react to me".into(),
                reply_to: None,
            },
        );

        // Fill to the cap with admin's reactions.
        for i in 0..MAX_REACTIONS_PER_MESSAGE {
            let emoji = format!("e{i:02}");
            emit(
                &mut dag,
                &admin,
                EventKind::Reaction {
                    message_id: msg.hash,
                    emoji,
                },
            );
        }

        // Bob reacts with an *existing* emoji key — must apply, even
        // though the message is at MAX_REACTIONS_PER_MESSAGE distinct
        // reactions, because cardinality does not grow. Bob's event
        // must causally depend on the grant + message so the topo sort
        // places it after both, otherwise his reaction may be processed
        // before he has SendMessages.
        let existing_emoji = "e00".to_string();
        emit_with_deps(
            &mut dag,
            &bob,
            EventKind::Reaction {
                message_id: msg.hash,
                emoji: existing_emoji.clone(),
            },
            vec![grant.hash, msg.hash],
        );

        let state = materialize(&dag);
        assert_eq!(
            state.messages[0].reactions.len(),
            MAX_REACTIONS_PER_MESSAGE,
            "cardinality unchanged",
        );
        let authors = state.messages[0]
            .reactions
            .get(&existing_emoji)
            .expect("existing emoji key must still be present");
        assert!(
            authors.contains(&admin.endpoint_id()),
            "admin's original reaction must remain",
        );
        assert!(
            authors.contains(&bob.endpoint_id()),
            "bob's reaction with existing emoji must be added",
        );
    }
}
