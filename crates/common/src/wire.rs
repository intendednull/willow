//! Gossipsub wire message format shared between clients and workers.
//!
//! All gossipsub messages are signed envelopes wrapping a [`WireMessage`].
//! Use [`pack_wire`] to serialize and sign, [`unpack_wire`] to verify
//! and deserialize.

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;

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
        peer_id: EndpointId,
    },
    /// A peer left a voice channel.
    VoiceLeave {
        /// The voice channel being left.
        channel_id: String,
        /// The peer who left.
        peer_id: EndpointId,
    },
    /// A WebRTC signaling message for voice chat.
    VoiceSignal {
        /// The voice channel this signal relates to.
        channel_id: String,
        /// The intended recipient peer.
        target_peer: EndpointId,
        /// The signaling payload.
        signal: VoiceSignalPayload,
    },
    /// A peer is requesting to join via a shareable link.
    JoinRequest {
        link_id: String,
        peer_id: EndpointId,
    },
    /// The inviter's response with an encrypted invite for the requester.
    JoinResponse {
        target_peer: EndpointId,
        invite_data: String,
    },
    /// The inviter denied the join request.
    JoinDenied {
        target_peer: EndpointId,
        reason: String,
    },
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
pub fn unpack_wire(data: &[u8]) -> Option<(WireMessage, willow_identity::EndpointId)> {
    let (envelope_bytes, signer) = willow_identity::unpack::<Vec<u8>>(data).ok()?;
    let (msg, willow_transport::MessageType::Channel) =
        willow_transport::unpack_envelope::<WireMessage>(&envelope_bytes).ok()?
    else {
        return None;
    };
    Some((msg, signer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;
    use willow_state::{EventKind, StateHash};

    #[test]
    fn pack_unpack_event_round_trip() {
        let id = Identity::generate();
        let event = willow_state::Event {
            id: "evt-1".to_string(),
            parent_hash: StateHash::ZERO,
            author: id.endpoint_id(),
            timestamp_ms: 1000,
            kind: EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello from common".to_string(),
                reply_to: None,
            },
        };

        let msg = WireMessage::Event(event);
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();

        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::Event(e) => assert_eq!(e.id, "evt-1"),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn pack_unpack_sync_batch_round_trip() {
        let id = Identity::generate();
        let events = vec![willow_state::Event {
            id: "e1".to_string(),
            parent_hash: StateHash::ZERO,
            author: id.endpoint_id(),
            timestamp_ms: 100,
            kind: EventKind::CreateChannel {
                name: "ch".to_string(),
                channel_id: "cid".to_string(),
                kind: "text".to_string(),
            },
        }];

        let msg = WireMessage::SyncBatch { events };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();

        match decoded {
            WireMessage::SyncBatch { events } => assert_eq!(events.len(), 1),
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn pack_unpack_sync_request_round_trip() {
        let id = Identity::generate();
        let msg = WireMessage::SyncRequest {
            state_hash: StateHash::from_bytes(b"test"),
            topic: Some("_willow_server_ops".to_string()),
        };

        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();

        match decoded {
            WireMessage::SyncRequest { state_hash, topic } => {
                assert_eq!(state_hash, StateHash::from_bytes(b"test"));
                assert_eq!(topic, Some("_willow_server_ops".to_string()));
            }
            _ => panic!("expected SyncRequest"),
        }
    }

    #[test]
    fn tampered_data_fails_unpack() {
        let id = Identity::generate();
        let msg = WireMessage::Event(willow_state::Event {
            id: "e".to_string(),
            parent_hash: StateHash::ZERO,
            author: id.endpoint_id(),
            timestamp_ms: 0,
            kind: EventKind::DeleteChannel {
                channel_id: "c".to_string(),
            },
        });

        let mut data = pack_wire(&msg, &id).unwrap();
        if let Some(b) = data.last_mut() {
            *b ^= 0xFF;
        }
        assert!(unpack_wire(&data).is_none());
    }

    #[test]
    fn empty_data_fails_unpack() {
        assert!(unpack_wire(&[]).is_none());
    }

    #[test]
    fn garbage_data_fails_unpack() {
        assert!(unpack_wire(b"not a valid message at all").is_none());
    }
}
