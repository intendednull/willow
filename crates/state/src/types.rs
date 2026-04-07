//! Pure data types for the event-sourced state machine.
//!
//! These types hold the shared state of a server without any UI framework,
//! networking, or crypto dependency. They are the building blocks of
//! [`ServerState`](crate::server::ServerState).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;

use crate::hash::EventHash;

/// A named conversation space inside a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// Unique ID (UUID string).
    pub id: String,
    /// Display name (e.g. "general").
    pub name: String,
    /// Hashes of pinned messages in this channel.
    #[serde(default)]
    pub pinned_messages: BTreeSet<EventHash>,
    /// Channel kind: `"text"` or `"voice"`. Defaults to `"text"`.
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
    pub permissions: BTreeSet<String>,
}

/// A peer's membership record within a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// The peer's endpoint ID.
    pub peer_id: EndpointId,
    /// Role IDs assigned to this member.
    pub roles: BTreeSet<String>,
    /// Optional display name override.
    pub display_name: Option<String>,
}

/// A single chat message with metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique message ID — the EventHash of the Message event that created it.
    pub id: EventHash,
    /// The channel this message belongs to.
    pub channel_id: String,
    /// Author's endpoint ID.
    pub author: EndpointId,
    /// Message body text.
    pub body: String,
    /// Wall-clock timestamp hint in milliseconds (display only).
    pub timestamp_ms: u64,
    /// Whether this message has been edited.
    pub edited: bool,
    /// Whether this message has been soft-deleted.
    pub deleted: bool,
    /// Reactions: emoji string -> list of reactor endpoint IDs.
    pub reactions: BTreeMap<String, Vec<EndpointId>>,
    /// If this is a reply, the EventHash of the parent message.
    pub reply_to: Option<EventHash>,
}

/// A peer's display profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// The peer's endpoint ID.
    pub peer_id: EndpointId,
    /// Display name.
    pub display_name: String,
}
