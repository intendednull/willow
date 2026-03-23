//! # Willow State
//!
//! Pure, deterministic event-sourced state machine for the Willow P2P chat
//! network. All state is derived from an ordered sequence of [`Event`]s via
//! the [`apply`] function. This crate has zero I/O, zero networking — just
//! `apply(state, event) -> ApplyResult`.
//!
//! ## Core concept
//!
//! Every mutation to shared state is represented as an [`Event`] with a
//! unique ID, author, timestamp, parent state hash, and an [`EventKind`]
//! describing the change. The [`apply`] function takes a mutable reference
//! to [`ServerState`] and an event, validates it, and applies the mutation
//! deterministically.
//!
//! ## Example
//!
//! ```
//! use willow_state::{Event, EventKind, ServerState, StateHash, apply};
//!
//! let mut state = ServerState::new("server-1", "My Server", "owner-peer");
//! let event = Event {
//!     id: "evt-1".to_string(),
//!     parent_hash: state.hash(),
//!     author: "owner-peer".to_string(),
//!     timestamp_ms: 1000,
//!     kind: EventKind::CreateChannel {
//!         name: "general".to_string(),
//!         channel_id: "ch-1".to_string(),
//!     },
//! };
//!
//! let result = apply(&mut state, &event);
//! assert!(matches!(result, willow_state::ApplyResult::Applied));
//! assert!(state.channels.contains_key("ch-1"));
//! ```

pub mod hash;
pub mod merge;
pub mod server;
pub mod store;
pub mod types;

#[cfg(test)]
mod tests;

// Re-exports for convenience.
pub use hash::StateHash;
pub use merge::{find_common_ancestor, merge};
pub use server::ServerState;
pub use store::{EventStore, InMemoryStore};
pub use types::{Channel, ChatMessage, Member, Profile, Role};

use serde::{Deserialize, Serialize};

/// An event that deterministically mutates shared state.
///
/// Each event carries the hash of the state it was applied against
/// (`parent_hash`), enabling divergence detection between peers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    /// Unique ID (UUID string).
    pub id: String,
    /// Hash of the state this event was applied against.
    pub parent_hash: StateHash,
    /// Author's peer ID.
    pub author: String,
    /// Wall-clock timestamp in milliseconds (display hint, not used for ordering).
    pub timestamp_ms: u64,
    /// The mutation to apply.
    pub kind: EventKind,
}

/// All possible state mutations — exhaustive, in one place.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EventKind {
    // -- Server structure --
    /// Create a new channel.
    CreateChannel {
        /// Human-readable name.
        name: String,
        /// Unique channel ID.
        channel_id: String,
    },
    /// Delete a channel by ID.
    DeleteChannel {
        /// The channel ID to delete.
        channel_id: String,
    },
    /// Rename a channel.
    RenameChannel {
        /// The channel ID to rename.
        channel_id: String,
        /// The new name.
        new_name: String,
    },

    // -- Roles & permissions --
    /// Create a new role.
    CreateRole {
        /// Human-readable name.
        name: String,
        /// Unique role ID.
        role_id: String,
    },
    /// Delete a role by ID.
    DeleteRole {
        /// The role ID to delete.
        role_id: String,
    },
    /// Set or clear a permission on a role.
    SetPermission {
        /// The role ID.
        role_id: String,
        /// Permission name string.
        permission: String,
        /// Whether to grant (true) or revoke (false).
        granted: bool,
    },
    /// Assign a role to a member.
    AssignRole {
        /// The member's peer ID.
        peer_id: String,
        /// The role ID to assign.
        role_id: String,
    },

    // -- Members --
    /// Mark a peer as trusted (can mutate server state).
    TrustPeer {
        /// The peer ID to trust.
        peer_id: String,
    },
    /// Remove trust from a peer.
    UntrustPeer {
        /// The peer ID to untrust.
        peer_id: String,
    },
    /// Remove a member from the server.
    KickMember {
        /// The peer ID to kick.
        peer_id: String,
    },

    // -- Chat --
    /// Send a chat message.
    Message {
        /// The channel this message belongs to.
        channel_id: String,
        /// Message body text.
        body: String,
    },
    /// Edit a previously sent message.
    EditMessage {
        /// The message ID to edit.
        message_id: String,
        /// The new body text.
        new_body: String,
    },
    /// Soft-delete a message (preserves history).
    DeleteMessage {
        /// The message ID to delete.
        message_id: String,
    },
    /// Add a reaction to a message.
    Reaction {
        /// The message ID to react to.
        message_id: String,
        /// The emoji string.
        emoji: String,
    },

    // -- Identity --
    /// Set or update the author's display name.
    SetProfile {
        /// The new display name.
        display_name: String,
    },

    // -- Encryption --
    /// Rotate a channel's encryption key.
    RotateChannelKey {
        /// The channel ID.
        channel_id: String,
        /// Encrypted key material for each recipient: (peer_id, encrypted_key_bytes).
        encrypted_keys: Vec<(String, Vec<u8>)>,
    },
}

/// Result of applying an event to state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyResult {
    /// The event was applied successfully.
    Applied,
    /// The event was already seen (duplicate event ID).
    AlreadySeen,
    /// The event's parent hash does not match the current state hash.
    ParentHashMismatch,
    /// The event was rejected (e.g., untrusted author for a privileged op).
    Rejected(String),
}

/// Apply an event to state with strict parent hash checking.
///
/// This is THE core function. It validates the event, checks the parent
/// hash, enforces trust, deduplicates, and then applies the mutation.
///
/// Returns [`ApplyResult`] indicating what happened.
pub fn apply(state: &mut ServerState, event: &Event) -> ApplyResult {
    // 1. Deduplication.
    if state.seen_event_ids.contains(&event.id) {
        return ApplyResult::AlreadySeen;
    }

    // 2. Parent hash check (strict).
    if event.parent_hash != state.hash() {
        return ApplyResult::ParentHashMismatch;
    }

    apply_inner(state, event)
}

/// Apply an event to state without strict parent hash checking.
///
/// This variant accepts events even when the parent hash doesn't match,
/// which is necessary during sync when peers are catching up and have
/// stale hashes. Deduplication and trust checks still apply.
pub fn apply_lenient(state: &mut ServerState, event: &Event) -> ApplyResult {
    // 1. Deduplication.
    if state.seen_event_ids.contains(&event.id) {
        return ApplyResult::AlreadySeen;
    }

    apply_inner(state, event)
}

/// Shared implementation for both strict and lenient apply.
fn apply_inner(state: &mut ServerState, event: &Event) -> ApplyResult {
    // Trust enforcement: only the owner and explicitly trusted peers can
    // mutate server structure. Chat messages from any member are accepted.
    let needs_trust = !matches!(
        event.kind,
        EventKind::Message { .. }
            | EventKind::EditMessage { .. }
            | EventKind::DeleteMessage { .. }
            | EventKind::Reaction { .. }
            | EventKind::SetProfile { .. }
    );

    if needs_trust && !state.is_trusted(&event.author) {
        state.seen_event_ids.insert(event.id.clone());
        return ApplyResult::Rejected(format!(
            "author '{}' is not trusted for {:?}",
            event.author, event.kind
        ));
    }

    // Mark as seen.
    state.seen_event_ids.insert(event.id.clone());

    // Apply the mutation.
    match &event.kind {
        EventKind::CreateChannel { name, channel_id } => {
            // Skip if channel already exists.
            if !state.channels.contains_key(channel_id) {
                state.channels.insert(
                    channel_id.clone(),
                    Channel {
                        id: channel_id.clone(),
                        name: name.clone(),
                    },
                );
            }
        }

        EventKind::DeleteChannel { channel_id } => {
            state.channels.remove(channel_id);
            // Also remove any messages in this channel.
            state.messages.retain(|m| m.channel_id != *channel_id);
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
                    Role {
                        id: role_id.clone(),
                        name: name.clone(),
                        permissions: std::collections::HashSet::new(),
                    },
                );
            }
        }

        EventKind::DeleteRole { role_id } => {
            state.roles.remove(role_id);
            // Remove the role from all members.
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
            // Only assign if both the role and member exist.
            if state.roles.contains_key(role_id) {
                if let Some(member) = state.members.get_mut(peer_id) {
                    member.roles.insert(role_id.clone());
                }
            }
        }

        EventKind::TrustPeer { peer_id } => {
            state.trusted_peers.insert(peer_id.clone());
            // Also ensure they are a member.
            state
                .members
                .entry(peer_id.clone())
                .or_insert_with(|| Member {
                    peer_id: peer_id.clone(),
                    roles: std::collections::HashSet::new(),
                    display_name: None,
                });
        }

        EventKind::UntrustPeer { peer_id } => {
            state.trusted_peers.remove(peer_id);
        }

        EventKind::KickMember { peer_id } => {
            // Cannot kick the owner.
            if *peer_id != state.owner {
                state.members.remove(peer_id);
                state.trusted_peers.remove(peer_id);
            }
        }

        EventKind::Message { channel_id, body } => {
            state.messages.push(ChatMessage {
                id: event.id.clone(),
                channel_id: channel_id.clone(),
                author: event.author.clone(),
                body: body.clone(),
                timestamp_ms: event.timestamp_ms,
                edited: false,
                deleted: false,
                reactions: std::collections::HashMap::new(),
                reply_to: None,
            });
        }

        EventKind::EditMessage {
            message_id,
            new_body,
        } => {
            if let Some(msg) = state.messages.iter_mut().find(|m| m.id == *message_id) {
                msg.body = new_body.clone();
                msg.edited = true;
            }
        }

        EventKind::DeleteMessage { message_id } => {
            if let Some(msg) = state.messages.iter_mut().find(|m| m.id == *message_id) {
                msg.deleted = true;
                msg.body = "[message deleted]".to_string();
                msg.reactions.clear();
            }
        }

        EventKind::Reaction { message_id, emoji } => {
            if let Some(msg) = state.messages.iter_mut().find(|m| m.id == *message_id) {
                msg.reactions
                    .entry(emoji.clone())
                    .or_default()
                    .push(event.author.clone());
            }
        }

        EventKind::SetProfile { display_name } => {
            state.profiles.insert(
                event.author.clone(),
                Profile {
                    peer_id: event.author.clone(),
                    display_name: display_name.clone(),
                },
            );
            // Also update the member's display name if they are a member.
            if let Some(member) = state.members.get_mut(&event.author) {
                member.display_name = Some(display_name.clone());
            }
        }

        EventKind::RotateChannelKey {
            channel_id,
            encrypted_keys,
        } => {
            // Store the first encrypted key as the channel key material.
            // In practice, the client layer decrypts the key for the local
            // peer and stores it.
            if let Some((_, key_bytes)) = encrypted_keys.first() {
                state
                    .channel_keys
                    .insert(channel_id.clone(), key_bytes.clone());
            }
        }
    }

    ApplyResult::Applied
}
