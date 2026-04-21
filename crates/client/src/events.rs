//! # Client Events
//!
//! Events emitted by the client when state changes. These provide a
//! UI-framework-agnostic notification mechanism for frontends to react to.

use willow_identity::EndpointId;

/// Scope of a mute toggle for `ClientEvent::MuteChanged`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MuteScope {
    /// The entire active grove was muted / unmuted.
    Grove,
    /// A single channel (by channel_id) was muted / unmuted.
    Channel(String),
}

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
        author: EndpointId,
    },
    PeerConnected(EndpointId),
    PeerDisconnected(EndpointId),
    ChannelCreated(String),
    ChannelDeleted(String),
    PeerTrusted(EndpointId),
    PeerUntrusted(EndpointId),
    ProfileUpdated {
        peer_id: EndpointId,
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
    /// A governance proposal was created.
    ProposalCreated {
        proposal_hash: String,
        action_description: String,
    },
    /// A vote was cast on a governance proposal.
    VoteCast {
        proposal_hash: String,
        accept: bool,
        voter: EndpointId,
    },
    /// A peer joined a voice channel.
    VoiceJoined {
        channel_id: String,
        peer_id: EndpointId,
    },
    /// A peer left a voice channel.
    VoiceLeft {
        channel_id: String,
        peer_id: EndpointId,
    },
    /// A voice signaling message was received.
    VoiceSignal {
        channel_id: String,
        from_peer: EndpointId,
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
    /// Local identity's per-surface mute state changed. Emitted after
    /// `mutate_channel_mute` / `mutate_grove_mute` succeed so the
    /// Notifier can refresh its gating table and the UI can flip the
    /// badge variant.
    MuteChanged {
        scope: MuteScope,
        muted: bool,
    },
    /// Sync-queue aggregate snapshot changed (Phase 2b). Re-emitted
    /// after any `QueueMeta` mutation the UI surfaces care about
    /// (enqueue, ack, retry, arrival bucket, relay / device signal).
    ///
    /// Payload is the fresh `QueueView`. The web crate pipes this into
    /// `AppState.queue.view` via `event_processing.rs`.
    QueueChanged(crate::views::QueueView),
    /// Relay reachability transitioned (Phase 2b).
    RelayStatusChanged(crate::queue::RelayStatus),
    /// Device-online signal transitioned (Phase 2b). Consumed by the
    /// reconnection-toast + welcome-back-banner components.
    DeviceOnlineChanged(bool),
}

impl willow_actor::Message for ClientEvent {
    type Result = ();
}
