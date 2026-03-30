//! # Network Types
//!
//! Command and event enums for communicating between the client logic and the
//! network layer. These are UI-framework-agnostic counterparts to the Bevy
//! bridge types in `willow-app`.
//!
//! The network spawn/run functions have been removed as part of the iroh
//! migration. They will be re-added once the network layer is rebuilt on
//! iroh endpoints.

use willow_identity::EndpointId;

/// Global gossipsub topic for profile broadcasts.
pub const PROFILE_TOPIC: &str = "_willow_profiles";

/// Events flowing from the network into the client.
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    MessageReceived {
        topic: String,
        data: Vec<u8>,
        source: Option<String>,
    },
    PeerConnected(EndpointId),
    PeerDisconnected(EndpointId),
    Listening(String),
    /// A file was announced by a peer (manifest received via gossipsub).
    FileAnnounced {
        filename: String,
        mime_type: String,
        size: u64,
        file_hash: String,
        from: String,
        topic: String,
    },
    /// A file download completed.
    FileDownloaded {
        filename: String,
        file_hash: String,
    },
    /// A peer's profile was received.
    ProfileReceived {
        peer_id: EndpointId,
        display_name: String,
    },
    /// An event was received from a peer.
    EventReceived {
        event: willow_state::Event,
        from: EndpointId,
    },
    /// A sync request was received from a peer.
    SyncRequested {
        state_hash: willow_state::StateHash,
        from: EndpointId,
        topic: Option<String>,
    },
    /// A batch of events was received as a sync response.
    SyncBatchReceived {
        events: Vec<willow_state::Event>,
        from: EndpointId,
    },
    /// A typing indicator was received from a peer.
    TypingReceived {
        peer_id: EndpointId,
        channel: String,
    },
    /// A peer joined a voice channel.
    VoiceJoinReceived {
        /// The voice channel that was joined.
        channel_id: String,
        /// The peer who joined.
        peer_id: EndpointId,
    },
    /// A peer left a voice channel.
    VoiceLeaveReceived {
        /// The voice channel that was left.
        channel_id: String,
        /// The peer who left.
        peer_id: EndpointId,
    },
    /// A voice signaling message was received (targeted at us).
    VoiceSignalReceived {
        /// The voice channel this signal relates to.
        channel_id: String,
        /// The peer who sent the signal.
        from_peer: EndpointId,
        /// The signaling payload.
        signal: crate::ops::VoiceSignalPayload,
    },
    /// A peer wants to join via a shareable link.
    JoinLinkRequested {
        link_id: String,
        peer_id: EndpointId,
    },
    /// A join link response was received (targeted at us).
    JoinLinkResponseReceived {
        invite_data: String,
    },
    /// A join link request was denied.
    JoinLinkDenied {
        reason: String,
    },
}

/// Commands flowing from the client to the network.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum NetworkCommand {
    Subscribe(String),
    Publish {
        topic: String,
        data: Vec<u8>,
    },
    /// Share a file: split, store chunks, broadcast manifest on the given topic.
    ShareFile {
        topic: String,
        filename: String,
        mime_type: String,
        data: Vec<u8>,
    },
    /// Broadcast our profile to peers.
    BroadcastProfile {
        display_name: String,
    },
    /// Broadcast a typing indicator on the server ops topic.
    SendTyping {
        channel: String,
    },
    /// Broadcast an event.
    BroadcastEvent {
        event: willow_state::Event,
        topic: Option<String>,
    },
    /// Request missing events from peers.
    RequestSync {
        state_hash: willow_state::StateHash,
        topic: Option<String>,
    },
    /// Send a batch of events as a sync response.
    SendSyncBatch {
        events: Vec<willow_state::Event>,
    },
    /// Broadcast a voice join to all peers.
    SendVoiceJoin {
        /// The voice channel being joined.
        channel_id: String,
    },
    /// Broadcast a voice leave to all peers.
    SendVoiceLeave {
        /// The voice channel being left.
        channel_id: String,
    },
    /// Send a voice signaling message to a specific peer.
    SendVoiceSignal {
        /// The voice channel this signal relates to.
        channel_id: String,
        /// The intended recipient peer.
        target_peer: EndpointId,
        /// The signaling payload.
        signal: crate::ops::VoiceSignalPayload,
    },
}

// ───── Network spawn (stubbed) ────────────────────────────────────────────
//
// The network spawn/run functions that previously used libp2p's NetworkConfig,
// NetworkNode, and NetworkEvent have been removed. The iroh-based network
// layer will provide a different API. For now, connect() is a no-op.

/// Stub: spawn the network task. Currently a no-op during the iroh migration.
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_network(
    _identity: willow_identity::Identity,
    _event_tx: futures::channel::mpsc::UnboundedSender<NetworkEvent>,
    _cmd_rx: futures::channel::mpsc::UnboundedReceiver<NetworkCommand>,
) {
    tracing::warn!("network spawn is stubbed out during iroh migration");
}

/// Stub: spawn the network task (WASM). Currently a no-op during the iroh migration.
#[cfg(target_arch = "wasm32")]
pub fn spawn_network(
    _identity: willow_identity::Identity,
    _event_tx: futures::channel::mpsc::UnboundedSender<NetworkEvent>,
    _cmd_rx: futures::channel::mpsc::UnboundedReceiver<NetworkCommand>,
) {
    tracing::warn!("network spawn is stubbed out during iroh migration");
}
