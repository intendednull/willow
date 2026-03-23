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
}
