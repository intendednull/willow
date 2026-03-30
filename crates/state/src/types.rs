//! Pure data types for the event-sourced state machine.
//!
//! These types hold the shared state of a server without any UI framework,
//! networking, or crypto dependency. They are the building blocks of
//! [`ServerState`](crate::server::ServerState).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;

/// A named conversation space inside a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// Unique ID (UUID string).
    pub id: String,
    /// Display name (e.g. "general").
    pub name: String,
    /// IDs of pinned messages in this channel.
    #[serde(default)]
    pub pinned_messages: HashSet<String>,
    /// Channel kind: `"text"` or `"voice"`. Defaults to `"text"` for backward compat.
    #[serde(default = "default_channel_kind")]
    pub kind: String,
}

/// Default channel kind for deserialization backward compatibility.
fn default_channel_kind() -> String {
    "text".to_string()
}

/// A named bundle of permissions that can be assigned to members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    /// Unique ID (UUID string).
    pub id: String,
    /// Human-readable name (e.g. "Moderator").
    pub name: String,
    /// The set of permission strings this role grants.
    pub permissions: HashSet<String>,
}

/// A peer's membership record within a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// The peer's endpoint ID.
    pub peer_id: EndpointId,
    /// Role IDs assigned to this member.
    pub roles: HashSet<String>,
    /// Optional display name override.
    pub display_name: Option<String>,
}

/// A single chat message with metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique message ID (matches the event ID that created it).
    pub id: String,
    /// The channel this message belongs to.
    pub channel_id: String,
    /// Author's endpoint ID.
    pub author: EndpointId,
    /// Message body text.
    pub body: String,
    /// Wall-clock timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Whether this message has been edited.
    pub edited: bool,
    /// Whether this message has been soft-deleted.
    pub deleted: bool,
    /// Reactions: emoji string -> list of reactor endpoint IDs.
    pub reactions: HashMap<String, Vec<EndpointId>>,
    /// If this is a reply, the ID of the parent message.
    pub reply_to: Option<String>,
}

/// Fine-grained permissions that can be granted to individual peers.
///
/// The owner always has all permissions implicitly. Non-owner peers must be
/// explicitly granted permissions via [`GrantPermission`](crate::EventKind::GrantPermission).
/// The [`Administrator`](Permission::Administrator) permission implies all others.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Can sync/provide full history to other peers.
    SyncProvider,
    /// Can manage channels (create, delete, rename).
    ManageChannels,
    /// Can manage roles and permissions.
    ManageRoles,
    /// Can kick members.
    KickMembers,
    /// Can send messages.
    SendMessages,
    /// Can create invites.
    CreateInvite,
    /// Full admin access (implies all permissions).
    Administrator,
}

/// A peer's display profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// The peer's endpoint ID.
    pub peer_id: EndpointId,
    /// Display name.
    pub display_name: String,
}
