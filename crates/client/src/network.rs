//! # Network Types
//!
//! Command and event enums for communicating between the client logic and the
//! network layer. These are UI-framework-agnostic counterparts to the Bevy
//! bridge types in `willow-app`.

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
    PeerConnected(String),
    PeerDisconnected(String),
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
        peer_id: String,
        display_name: String,
    },
    /// A server operation was received from a peer.
    OpReceived {
        stamped_op: crate::ops::StampedOp,
        from: String,
    },
    /// A sync request was received from a peer.
    SyncRequested {
        latest_hlc: willow_messaging::hlc::HlcTimestamp,
        from: String,
        topic: Option<String>,
    },
    /// A batch of ops was received as a sync response.
    SyncBatchReceived {
        ops: Vec<crate::ops::StampedOp>,
        from: String,
    },
}

/// Commands flowing from the client to the network.
#[derive(Debug, Clone)]
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
    /// Broadcast a server state operation.
    BroadcastOp(crate::ops::StampedOp),
    /// Request missing ops from peers.
    /// If `topic` is set, request chat messages for that specific channel.
    RequestSync {
        latest_hlc: willow_messaging::hlc::HlcTimestamp,
        topic: Option<String>,
    },
    /// Send a batch of ops as a sync response.
    SendSyncBatch {
        ops: Vec<crate::ops::StampedOp>,
    },
}
