//! # Client Events
//!
//! Events emitted by the client when state changes. These provide a
//! UI-framework-agnostic notification mechanism for frontends to react to.

/// Events emitted by the client when state changes.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    MessageReceived {
        channel: String,
        message_id: String,
        is_local: bool,
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
    /// A peer joined a voice channel.
    VoiceJoined {
        channel_id: String,
        peer_id: String,
    },
    /// A peer left a voice channel.
    VoiceLeft {
        channel_id: String,
        peer_id: String,
    },
    /// A voice signaling message was received.
    VoiceSignal {
        channel_id: String,
        from_peer: String,
        signal: crate::ops::VoiceSignalPayload,
    },
    /// A join-via-link response was received — auto-join can proceed.
    JoinLinkResponse {
        invite_data: String,
    },
    /// A join-via-link request was denied.
    JoinLinkDenied {
        reason: String,
    },
}
