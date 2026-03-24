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
}
