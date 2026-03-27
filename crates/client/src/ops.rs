//! # Server State Sync
//!
//! Wire-level message types for broadcasting server state mutations over
//! gossipsub. The wire format is [`WireMessage`], which wraps
//! [`willow_state::Event`]s directly.
//!
//! ## Topic
//!
//! All server ops are published on `_willow_server_ops`.
//!
//! ## Security
//!
//! Each message is wrapped in a signed envelope via `willow_identity::pack()`.
//! The receiver verifies the signature, checks that the event hasn't been
//! seen before, and validates permissions before applying.

use serde::{Deserialize, Serialize};

// ───── Join link types ──────────────────────────────────────────────────────

/// Token embedded in a shareable join URL. Contains enough
/// context to show the user what they're joining before connecting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinToken {
    pub inviter_peer_id: String,
    pub server_id: String,
    pub link_id: String,
    /// Human-readable server name for the join page.
    pub server_name: String,
    /// Display name of whoever generated the link.
    pub inviter_name: String,
}

impl JoinToken {
    /// Encode to a URL-safe base64 string.
    pub fn encode(&self) -> String {
        let bytes = willow_transport::pack(self).unwrap_or_default();
        crate::base64::encode(&bytes)
    }

    /// Decode from a base64 string.
    pub fn decode(s: &str) -> Option<Self> {
        let bytes = crate::base64::decode(s)?;
        willow_transport::unpack(&bytes).ok()
    }
}

/// Metadata for a generated join link, stored locally by the inviter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinLink {
    pub link_id: String,
    pub server_id: String,
    pub max_uses: u32,
    pub used: u32,
    pub active: bool,
    /// Timestamp in ms. None = never expires.
    pub expires_at: Option<u64>,
    /// When this link was created (ms since epoch).
    pub created_at: u64,
}

impl JoinLink {
    /// Check if this link can accept another join.
    pub fn is_valid(&self) -> bool {
        if !self.active || self.used >= self.max_uses {
            return false;
        }
        if let Some(expires) = self.expires_at {
            if crate::util::current_time_ms() > expires {
                return false;
            }
        }
        true
    }
}

// ───── Wire format ──────────────────────────────────────────────────────────

/// Wire-level message format for network communication.
///
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
    JoinRequest {
        link_id: String,
        peer_id: String,
    },
    /// The inviter's response with an encrypted invite for the requester.
    JoinResponse {
        target_peer: String,
        invite_data: String,
    },
    /// The inviter denied the join request.
    JoinDenied {
        target_peer: String,
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
pub fn unpack_wire(data: &[u8]) -> Option<(WireMessage, willow_identity::PeerId)> {
    let (envelope_bytes, signer) = willow_identity::unpack::<Vec<u8>>(data).ok()?;
    let (msg, willow_transport::MessageType::Channel) =
        willow_transport::unpack_envelope::<WireMessage>(&envelope_bytes).ok()?
    else {
        return None;
    };
    Some((msg, signer))
}

/// Re-export `willow_state::Event` for convenience.
pub use willow_state::Event;

/// The gossipsub topic for server operations.
pub const SERVER_OPS_TOPIC: &str = "_willow_server_ops";

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    #[test]
    fn wire_message_event_round_trip() {
        let id = Identity::generate();
        let event = willow_state::Event {
            id: "evt-1".to_string(),
            parent_hash: willow_state::StateHash::ZERO,
            author: id.peer_id().to_string(),
            timestamp_ms: 1000,
            kind: willow_state::EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: "ch-1".to_string(),
                kind: "text".to_string(),
            },
        };

        let msg = WireMessage::Event(event.clone());
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();

        assert_eq!(signer, id.peer_id());
        match decoded {
            WireMessage::Event(e) => {
                assert_eq!(e.id, "evt-1");
                assert_eq!(e.author, id.peer_id().to_string());
            }
            _ => panic!("expected WireMessage::Event"),
        }
    }

    #[test]
    fn wire_message_sync_request_round_trip() {
        let id = Identity::generate();
        let msg = WireMessage::SyncRequest {
            state_hash: willow_state::StateHash::from_bytes(b"test-hash"),
            topic: Some("my-topic".to_string()),
        };

        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();

        match decoded {
            WireMessage::SyncRequest { state_hash, topic } => {
                assert_eq!(
                    state_hash,
                    willow_state::StateHash::from_bytes(b"test-hash")
                );
                assert_eq!(topic, Some("my-topic".to_string()));
            }
            _ => panic!("expected WireMessage::SyncRequest"),
        }
    }

    #[test]
    fn wire_message_sync_batch_round_trip() {
        let id = Identity::generate();
        let events = vec![
            willow_state::Event {
                id: "e1".to_string(),
                parent_hash: willow_state::StateHash::ZERO,
                author: "peer-1".to_string(),
                timestamp_ms: 100,
                kind: willow_state::EventKind::CreateChannel {
                    name: "ch1".to_string(),
                    channel_id: "cid1".to_string(),
                    kind: "text".to_string(),
                },
            },
            willow_state::Event {
                id: "e2".to_string(),
                parent_hash: willow_state::StateHash::ZERO,
                author: "peer-1".to_string(),
                timestamp_ms: 200,
                kind: willow_state::EventKind::Message {
                    channel_id: "cid1".to_string(),
                    body: "hello".to_string(),
                    reply_to: None,
                },
            },
        ];

        let msg = WireMessage::SyncBatch {
            events: events.clone(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();

        match decoded {
            WireMessage::SyncBatch {
                events: decoded_events,
            } => {
                assert_eq!(decoded_events.len(), 2);
                assert_eq!(decoded_events[0].id, "e1");
                assert_eq!(decoded_events[1].id, "e2");
            }
            _ => panic!("expected WireMessage::SyncBatch"),
        }
    }

    #[test]
    fn wire_message_tampered_fails() {
        let id = Identity::generate();
        let event = willow_state::Event {
            id: "evt-x".to_string(),
            parent_hash: willow_state::StateHash::ZERO,
            author: "peer".to_string(),
            timestamp_ms: 500,
            kind: willow_state::EventKind::DeleteChannel {
                channel_id: "ch-1".to_string(),
            },
        };

        let mut data = pack_wire(&WireMessage::Event(event), &id).unwrap();
        if let Some(byte) = data.last_mut() {
            *byte ^= 0xFF;
        }

        assert!(unpack_wire(&data).is_none());
    }

    #[test]
    fn join_token_round_trip() {
        let token = JoinToken {
            inviter_peer_id: "12D3KooWTest".to_string(),
            server_id: "srv-1".to_string(),
            link_id: "link-abc".to_string(),
            server_name: "My Server".to_string(),
            inviter_name: "Alice".to_string(),
        };
        let encoded = token.encode();
        let decoded = JoinToken::decode(&encoded).unwrap();
        assert_eq!(decoded.inviter_peer_id, "12D3KooWTest");
        assert_eq!(decoded.server_id, "srv-1");
        assert_eq!(decoded.link_id, "link-abc");
        assert_eq!(decoded.server_name, "My Server");
        assert_eq!(decoded.inviter_name, "Alice");
    }

    #[test]
    fn join_token_decode_invalid_returns_none() {
        assert!(JoinToken::decode("not-valid!@#$").is_none());
        assert!(JoinToken::decode("").is_none());
    }

    #[test]
    fn join_link_is_valid_active_under_limit() {
        let link = JoinLink {
            link_id: "l1".into(),
            server_id: "s1".into(),
            max_uses: 5,
            used: 2,
            active: true,
            expires_at: None,
            created_at: 0,
        };
        assert!(link.is_valid());
    }

    #[test]
    fn join_link_is_valid_max_uses_reached() {
        let link = JoinLink {
            link_id: "l1".into(),
            server_id: "s1".into(),
            max_uses: 5,
            used: 5,
            active: true,
            expires_at: None,
            created_at: 0,
        };
        assert!(!link.is_valid());
    }

    #[test]
    fn join_link_is_valid_inactive() {
        let link = JoinLink {
            link_id: "l1".into(),
            server_id: "s1".into(),
            max_uses: 5,
            used: 0,
            active: false,
            expires_at: None,
            created_at: 0,
        };
        assert!(!link.is_valid());
    }

    #[test]
    fn join_link_is_valid_expired() {
        let link = JoinLink {
            link_id: "l1".into(),
            server_id: "s1".into(),
            max_uses: 5,
            used: 0,
            active: true,
            expires_at: Some(1),
            created_at: 0,
        };
        assert!(!link.is_valid());
    }

    #[test]
    fn wire_message_join_request_round_trip() {
        let id = Identity::generate();
        let msg = WireMessage::JoinRequest {
            link_id: "link-1".to_string(),
            peer_id: "12D3KooWJoiner".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.peer_id());
        match decoded {
            WireMessage::JoinRequest { link_id, peer_id } => {
                assert_eq!(link_id, "link-1");
                assert_eq!(peer_id, "12D3KooWJoiner");
            }
            _ => panic!("expected JoinRequest"),
        }
    }

    #[test]
    fn wire_message_join_response_round_trip() {
        let id = Identity::generate();
        let msg = WireMessage::JoinResponse {
            target_peer: "12D3KooWJoiner".to_string(),
            invite_data: "base64inviteblob".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::JoinResponse {
                target_peer,
                invite_data,
            } => {
                assert_eq!(target_peer, "12D3KooWJoiner");
                assert_eq!(invite_data, "base64inviteblob");
            }
            _ => panic!("expected JoinResponse"),
        }
    }

    #[test]
    fn wire_message_join_denied_round_trip() {
        let id = Identity::generate();
        let msg = WireMessage::JoinDenied {
            target_peer: "12D3KooWJoiner".to_string(),
            reason: "link_expired".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::JoinDenied {
                target_peer,
                reason,
            } => {
                assert_eq!(target_peer, "12D3KooWJoiner");
                assert_eq!(reason, "link_expired");
            }
            _ => panic!("expected JoinDenied"),
        }
    }
}
