//! # Server State Sync
//!
//! Wire-level message types for broadcasting server state mutations over
//! gossipsub. The primary wire format is [`WireMessage`], which wraps
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
//!
//! ## Legacy types
//!
//! The [`Op`], [`StampedOp`], and [`SyncMessage`] types are kept for backward
//! compatibility with `willow-app`. New code should use [`WireMessage`]
//! instead.

use serde::{Deserialize, Serialize};
use willow_messaging::hlc::HlcTimestamp;

// ───── New wire format ─────────────────────────────────────────────────────

/// Wire-level message format. Replaces the legacy [`SyncMessage`].
///
/// All network communication now uses `WireMessage` wrappers around
/// [`willow_state::Event`]s instead of the legacy `Op`/`StampedOp` types.
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

// ───── Legacy types (deprecated, kept for willow-app compat) ───────────────

/// A signed, timestamped server state mutation.
///
/// Wraps an [`Op`] with metadata for deduplication and ordering.
#[deprecated(note = "use WireMessage with willow_state::Event instead")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(deprecated)]
pub struct StampedOp {
    /// Unique ID for deduplication.
    pub op_id: String,
    /// HLC timestamp for causal ordering.
    pub hlc: HlcTimestamp,
    /// PeerId of the author (verified against signature on receive).
    pub author: String,
    /// The actual mutation.
    pub op: Op,
}

#[allow(deprecated)]
impl StampedOp {
    /// Create a new stamped op with a fresh UUID and HLC timestamp.
    pub fn new(op: Op, hlc: &mut willow_messaging::hlc::HLC, author_peer_id: &str) -> Self {
        Self {
            op_id: uuid::Uuid::new_v4().to_string(),
            hlc: hlc.now(),
            author: author_peer_id.to_string(),
            op,
        }
    }
}

/// A state mutation (server ops and chat messages).
#[deprecated(note = "use willow_state::EventKind instead")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Op {
    CreateChannel {
        name: String,
        channel_id: String,
    },
    DeleteChannel {
        name: String,
    },
    CreateRole {
        name: String,
        role_id: String,
    },
    DeleteRole {
        role_id: String,
    },
    SetPermission {
        role_id: String,
        permission: String,
        granted: bool,
    },
    AssignRole {
        peer_id: String,
        role_id: String,
    },
    KickMember {
        peer_id: String,
        /// Encrypted rotated channel keys for remaining members.
        /// Each entry: (recipient_peer_id, topic, encrypted_key).
        #[serde(default)]
        rotated_keys: Vec<(String, String, willow_crypto::EncryptedChannelKey)>,
    },
    /// Mark a peer as trusted (can broadcast ops, sync state).
    TrustPeer {
        peer_id: String,
    },
    /// Remove trust from a peer.
    UntrustPeer {
        peer_id: String,
    },
    /// A chat message (text, reaction, edit, delete, reply, file).
    /// `content_data` is the serialized `Content` enum (may be encrypted).
    ChatMessage {
        topic: String,
        content_data: Vec<u8>,
    },
}

/// Wire-level message on the server ops topic.
#[deprecated(note = "use WireMessage instead")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(deprecated)]
pub enum SyncMessage {
    /// A single server operation.
    Op(StampedOp),
    /// Request ops newer than the given HLC timestamp.
    /// If `topic` is set, request chat messages for that specific channel.
    SyncRequest {
        latest_hlc: HlcTimestamp,
        #[serde(default)]
        topic: Option<String>,
    },
    /// Batch of ops in response to a sync request.
    SyncBatch { ops: Vec<StampedOp> },
}

/// The gossipsub topic for server operations.
pub const SERVER_OPS_TOPIC: &str = "_willow_server_ops";

/// Serialize a SyncMessage into a signed envelope ready for gossipsub.
#[deprecated(note = "use pack_wire instead")]
#[allow(deprecated)]
pub fn pack_sync(msg: &SyncMessage, identity: &willow_identity::Identity) -> Option<Vec<u8>> {
    let envelope =
        willow_transport::pack_envelope(willow_transport::MessageType::Channel, msg).ok()?;
    willow_identity::pack(&envelope, identity).ok()
}

/// Verify and deserialize a SyncMessage from a signed envelope.
#[deprecated(note = "use unpack_wire instead")]
#[allow(deprecated)]
pub fn unpack_sync(data: &[u8]) -> Option<(SyncMessage, willow_identity::PeerId)> {
    let (envelope_bytes, signer) = willow_identity::unpack::<Vec<u8>>(data).ok()?;
    let (msg, willow_transport::MessageType::Channel) =
        willow_transport::unpack_envelope::<SyncMessage>(&envelope_bytes).ok()?
    else {
        return None;
    };
    Some((msg, signer))
}

/// Pack a single op as a SyncMessage::Op.
#[deprecated(note = "use pack_wire with WireMessage::Event instead")]
#[allow(deprecated)]
pub fn pack_op(stamped: &StampedOp, identity: &willow_identity::Identity) -> Option<Vec<u8>> {
    pack_sync(&SyncMessage::Op(stamped.clone()), identity)
}

/// Unpack a single op (returns None if the message is not a SyncMessage::Op).
#[deprecated(note = "use unpack_wire instead")]
#[allow(deprecated)]
pub fn unpack_op(data: &[u8]) -> Option<(StampedOp, willow_identity::PeerId)> {
    let (msg, signer) = unpack_sync(data)?;
    match msg {
        SyncMessage::Op(stamped) => Some((stamped, signer)),
        _ => None,
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use willow_identity::Identity;
    use willow_messaging::hlc::HLC;

    fn make_stamped(op: Op) -> StampedOp {
        let mut hlc = HLC::new();
        StampedOp::new(op, &mut hlc, "test-peer")
    }

    #[test]
    fn pack_unpack_round_trip() {
        let id = Identity::generate();
        let stamped = make_stamped(Op::CreateChannel {
            name: "general".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        });

        let data = pack_op(&stamped, &id).unwrap();
        let (decoded, signer) = unpack_op(&data).unwrap();

        assert_eq!(signer, id.peer_id());
        assert_eq!(decoded.op_id, stamped.op_id);
        assert!(matches!(decoded.op, Op::CreateChannel { ref name, .. } if name == "general"));
    }

    #[test]
    fn wrong_signer_still_verifies() {
        let id = Identity::generate();
        let stamped = make_stamped(Op::KickMember {
            peer_id: "someone".into(),
            rotated_keys: vec![],
        });

        let data = pack_op(&stamped, &id).unwrap();
        let (_, signer) = unpack_op(&data).unwrap();
        assert_eq!(signer, id.peer_id());
    }

    #[test]
    fn tampered_data_fails() {
        let id = Identity::generate();
        let stamped = make_stamped(Op::CreateRole {
            name: "admin".into(),
            role_id: uuid::Uuid::new_v4().to_string(),
        });

        let mut data = pack_op(&stamped, &id).unwrap();
        if let Some(byte) = data.last_mut() {
            *byte ^= 0xFF;
        }

        assert!(unpack_op(&data).is_none());
    }

    #[test]
    fn all_op_variants_serialize() {
        let id = Identity::generate();
        let ops = vec![
            Op::CreateChannel {
                name: "test".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            Op::DeleteChannel {
                name: "test".into(),
            },
            Op::CreateRole {
                name: "mod".into(),
                role_id: uuid::Uuid::new_v4().to_string(),
            },
            Op::DeleteRole {
                role_id: "abc".into(),
            },
            Op::SetPermission {
                role_id: "abc".into(),
                permission: "Administrator".into(),
                granted: true,
            },
            Op::AssignRole {
                peer_id: "peer1".into(),
                role_id: "role1".into(),
            },
            Op::KickMember {
                peer_id: "peer1".into(),
                rotated_keys: vec![],
            },
            Op::TrustPeer {
                peer_id: "peer1".into(),
            },
            Op::UntrustPeer {
                peer_id: "peer1".into(),
            },
        ];

        for op in ops {
            let stamped = make_stamped(op);
            let data = pack_op(&stamped, &id).unwrap();
            let (decoded, _) = unpack_op(&data).unwrap();
            let _ = format!("{:?}", decoded.op);
        }
    }

    #[test]
    fn stamped_op_has_unique_ids() {
        let mut hlc = HLC::new();
        let a = StampedOp::new(
            Op::CreateChannel {
                name: "a".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            &mut hlc,
            "peer",
        );
        let b = StampedOp::new(
            Op::CreateChannel {
                name: "b".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            &mut hlc,
            "peer",
        );
        assert_ne!(a.op_id, b.op_id);
    }

    #[test]
    fn stamped_op_hlc_advances() {
        let mut hlc = HLC::new();
        let a = StampedOp::new(
            Op::CreateChannel {
                name: "a".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            &mut hlc,
            "peer",
        );
        let b = StampedOp::new(
            Op::CreateChannel {
                name: "b".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            &mut hlc,
            "peer",
        );
        assert!(b.hlc > a.hlc);
    }

    // ───── WireMessage tests ───────────────────────────────────────────────

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
