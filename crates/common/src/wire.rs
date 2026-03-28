//! Gossipsub wire message format shared between clients and workers.
//!
//! All gossipsub messages are signed envelopes wrapping a [`WireMessage`].
//! Use [`pack_wire`] to serialize and sign, [`unpack_wire`] to verify
//! and deserialize.

use serde::{Deserialize, Serialize};

/// All network communication uses `WireMessage` wrappers around
/// [`willow_state::Event`]s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireMessage {
    /// A single event.
    Event(willow_state::Event),
    /// Request events since a state hash.
    SyncRequest {
        /// The state hash the sender has — the responder returns events
        /// that the sender is missing.
        state_hash: willow_state::StateHash,
        /// If set, request events for a specific topic (channel).
        topic: Option<String>,
    },
    /// Batch of events in response to a sync request.
    SyncBatch {
        /// The events the responder is sending.
        events: Vec<willow_state::Event>,
    },
    /// Ephemeral typing indicator — not stored or persisted.
    TypingIndicator {
        /// The channel name the peer is typing in.
        channel: String,
    },
    /// A peer joined a voice channel.
    VoiceJoin {
        /// The voice channel being joined.
        channel_id: String,
        /// The peer who joined.
        peer_id: String,
    },
    /// A peer left a voice channel.
    VoiceLeave {
        /// The voice channel being left.
        channel_id: String,
        /// The peer who left.
        peer_id: String,
    },
    /// A WebRTC signaling message for voice chat.
    VoiceSignal {
        /// The voice channel this signal relates to.
        channel_id: String,
        /// The intended recipient peer.
        target_peer: String,
        /// The signaling payload.
        signal: VoiceSignalPayload,
    },
    /// A peer is requesting to join via a shareable link.
    JoinRequest { link_id: String, peer_id: String },
    /// The inviter's response with an encrypted invite for the requester.
    JoinResponse {
        target_peer: String,
        invite_data: String,
    },
    /// The inviter denied the join request.
    JoinDenied { target_peer: String, reason: String },
}

/// WebRTC signaling payload for voice chat negotiation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VoiceSignalPayload {
    /// SDP offer for initiating a connection.
    Offer(String),
    /// SDP answer in response to an offer.
    Answer(String),
    /// ICE candidate for connection establishment.
    IceCandidate(String),
}

/// Serialize a [`WireMessage`] into a signed envelope ready for gossipsub.
pub fn pack_wire(msg: &WireMessage, identity: &willow_identity::Identity) -> Option<Vec<u8>> {
    let envelope =
        willow_transport::pack_envelope(willow_transport::MessageType::Channel, msg).ok()?;
    willow_identity::pack(&envelope, identity).ok()
}

/// Verify and deserialize a [`WireMessage`] from a signed envelope.
pub fn unpack_wire(data: &[u8]) -> Option<(WireMessage, willow_identity::PeerId)> {
    let (envelope_bytes, signer) = willow_identity::unpack::<Vec<u8>>(data).ok()?;
    let (msg, willow_transport::MessageType::Channel) =
        willow_transport::unpack_envelope::<WireMessage>(&envelope_bytes).ok()?
    else {
        return None;
    };
    Some((msg, signer))
}
