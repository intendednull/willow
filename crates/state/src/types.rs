//! Pure data types for the event-sourced state machine.
//!
//! These types hold the shared state of a server without any UI framework,
//! networking, or crypto dependency. They are the building blocks of
//! [`ServerState`](crate::server::ServerState).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;

use crate::hash::EventHash;

/// Channel kind — text chat or voice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    /// A text chat channel (default).
    #[default]
    #[serde(alias = "text")]
    Text,
    /// A voice (and optionally video/screenshare) channel.
    #[serde(alias = "voice")]
    Voice,
}

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
    /// Text or voice.
    #[serde(default)]
    pub kind: ChannelKind,
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
    /// Reactions: emoji string -> set of reactor endpoint IDs.
    /// Stored as a `BTreeSet` so each peer can only react once with a
    /// given emoji to a given message.
    pub reactions: BTreeMap<String, BTreeSet<EndpointId>>,
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

/// Per-identity mute state for one grove.
///
/// Stored on `ServerState::mute_state` keyed by `EndpointId`. Muting
/// silences the author's own notifications only — it is never
/// advertised to peers, so there is no authority check in
/// `apply_event` for `MuteChannel` / `MuteGrove`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuteState {
    /// Explicitly-muted channel IDs. Membership in this set means
    /// "suppress notifications for this channel."
    pub channels: std::collections::HashSet<String>,
    /// True if the entire grove is muted (supersedes per-channel
    /// entries). A muted grove still emits unread counts so the
    /// badge layer can render the outlined muted pill.
    pub grove_muted: bool,
}
