//! # Server State Sync
//!
//! Broadcasts server mutations as signed, HLC-stamped operations over
//! gossipsub so all members converge on the same state. Each operation is
//! wrapped in a [`StampedOp`] with a unique ID (for deduplication) and an
//! HLC timestamp (for causal ordering), then signed with Ed25519.
//!
//! ## Topic
//!
//! All server ops are published on `_willow_server_ops`.
//!
//! ## Security
//!
//! Each op is wrapped in a signed envelope via `willow_identity::pack()`.
//! The receiver verifies the signature, checks that the op hasn't been
//! seen before, and validates that the signer is a trusted peer before
//! applying the operation.

use serde::{Deserialize, Serialize};
use willow_messaging::hlc::HlcTimestamp;

/// A signed, timestamped server state mutation.
///
/// Wraps a [`ServerOp`] with metadata for deduplication and ordering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StampedOp {
    /// Unique ID for deduplication.
    pub op_id: String,
    /// HLC timestamp for causal ordering.
    pub hlc: HlcTimestamp,
    /// PeerId of the author (verified against signature on receive).
    pub author: String,
    /// The actual mutation.
    pub op: ServerOp,
}

impl StampedOp {
    /// Create a new stamped op with a fresh UUID and HLC timestamp.
    pub fn new(op: ServerOp, hlc: &mut willow_messaging::hlc::HLC, author_peer_id: &str) -> Self {
        Self {
            op_id: uuid::Uuid::new_v4().to_string(),
            hlc: hlc.now(),
            author: author_peer_id.to_string(),
            op,
        }
    }
}

/// A server state mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerOp {
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
}

/// Wire-level message on the server ops topic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncMessage {
    /// A single server operation.
    Op(StampedOp),
    /// Request ops newer than the given HLC timestamp.
    SyncRequest { latest_hlc: HlcTimestamp },
    /// Batch of ops in response to a sync request.
    SyncBatch { ops: Vec<StampedOp> },
}

/// The gossipsub topic for server operations.
pub const SERVER_OPS_TOPIC: &str = "_willow_server_ops";

/// Serialize a SyncMessage into a signed envelope ready for gossipsub.
pub fn pack_sync(msg: &SyncMessage, identity: &willow_identity::Identity) -> Option<Vec<u8>> {
    let envelope =
        willow_transport::pack_envelope(willow_transport::MessageType::Channel, msg).ok()?;
    willow_identity::pack(&envelope, identity).ok()
}

/// Verify and deserialize a SyncMessage from a signed envelope.
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
pub fn pack_op(stamped: &StampedOp, identity: &willow_identity::Identity) -> Option<Vec<u8>> {
    pack_sync(&SyncMessage::Op(stamped.clone()), identity)
}

/// Unpack a single op (returns None if the message is not a SyncMessage::Op).
pub fn unpack_op(data: &[u8]) -> Option<(StampedOp, willow_identity::PeerId)> {
    let (msg, signer) = unpack_sync(data)?;
    match msg {
        SyncMessage::Op(stamped) => Some((stamped, signer)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;
    use willow_messaging::hlc::HLC;

    fn make_stamped(op: ServerOp) -> StampedOp {
        let mut hlc = HLC::new();
        StampedOp::new(op, &mut hlc, "test-peer")
    }

    #[test]
    fn pack_unpack_round_trip() {
        let id = Identity::generate();
        let stamped = make_stamped(ServerOp::CreateChannel {
            name: "general".into(),
            channel_id: uuid::Uuid::new_v4().to_string(),
        });

        let data = pack_op(&stamped, &id).unwrap();
        let (decoded, signer) = unpack_op(&data).unwrap();

        assert_eq!(signer, id.peer_id());
        assert_eq!(decoded.op_id, stamped.op_id);
        assert!(
            matches!(decoded.op, ServerOp::CreateChannel { ref name, .. } if name == "general")
        );
    }

    #[test]
    fn wrong_signer_still_verifies() {
        let id = Identity::generate();
        let stamped = make_stamped(ServerOp::KickMember {
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
        let stamped = make_stamped(ServerOp::CreateRole {
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
            ServerOp::CreateChannel {
                name: "test".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            ServerOp::DeleteChannel {
                name: "test".into(),
            },
            ServerOp::CreateRole {
                name: "mod".into(),
                role_id: uuid::Uuid::new_v4().to_string(),
            },
            ServerOp::DeleteRole {
                role_id: "abc".into(),
            },
            ServerOp::SetPermission {
                role_id: "abc".into(),
                permission: "Administrator".into(),
                granted: true,
            },
            ServerOp::AssignRole {
                peer_id: "peer1".into(),
                role_id: "role1".into(),
            },
            ServerOp::KickMember {
                peer_id: "peer1".into(),
                rotated_keys: vec![],
            },
            ServerOp::TrustPeer {
                peer_id: "peer1".into(),
            },
            ServerOp::UntrustPeer {
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
            ServerOp::CreateChannel {
                name: "a".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            &mut hlc,
            "peer",
        );
        let b = StampedOp::new(
            ServerOp::CreateChannel {
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
            ServerOp::CreateChannel {
                name: "a".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            &mut hlc,
            "peer",
        );
        let b = StampedOp::new(
            ServerOp::CreateChannel {
                name: "b".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            &mut hlc,
            "peer",
        );
        assert!(b.hlc > a.hlc);
    }
}
