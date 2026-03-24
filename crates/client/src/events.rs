//! # Client Events
//!
//! Events emitted by the client when state changes. These provide a
//! UI-framework-agnostic notification mechanism for frontends to react to.

use crate::state::ChatMessage;

/// Events emitted by the client when state changes.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    MessageReceived {
        channel: String,
        message: ChatMessage,
    },
    MessageEdited {
        channel: String,
        message_id: String,
        new_body: String,
    },
    MessageDeleted {
        channel: String,
        message_id: String,
    },
    ReactionAdded {
        channel: String,
        message_id: String,
        emoji: String,
        author: String,
    },
    PeerConnected(String),
    PeerDisconnected(String),
    ChannelCreated(String),
    ChannelDeleted(String),
    MemberKicked(String),
    PeerTrusted(String),
    PeerUntrusted(String),
    ProfileUpdated {
        peer_id: String,
        display_name: String,
    },
    FileAnnounced {
        channel: String,
        filename: String,
        size: u64,
        from: String,
    },
    Listening(String),
    SyncCompleted {
        ops_applied: usize,
    },
    RoleCreated {
        name: String,
        role_id: String,
    },
    RoleDeleted {
        role_id: String,
    },
    StateHashMismatch {
        peer_id: String,
        our_hash: String,
        their_hash: String,
    },
    ServerRenamed {
        new_name: String,
    },
    ServerDescriptionChanged {
        description: String,
    },
    MessagePinned {
        channel: String,
        message_id: String,
    },
    MessageUnpinned {
        channel: String,
        message_id: String,
    },
}

/// Push notifications sent through the optional notification channel.
///
/// These are an addition alongside the existing [`ClientEvent`] poll model.
/// UIs can subscribe to this channel for reactive updates without polling.
#[derive(Debug, Clone)]
pub enum ClientNotification {
    /// A `willow_state::Event` was applied to the event-sourced state.
    EventApplied(willow_state::Event),
    /// A new peer connected to the network.
    PeerConnected(String),
    /// A peer disconnected from the network.
    PeerDisconnected(String),
    /// The event-sourced state changed (generic notification).
    StateChanged,
}
