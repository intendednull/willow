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
use willow_identity::EndpointId;

// Re-export wire types from willow-common so existing imports still work.
pub use willow_common::{pack_wire, unpack_wire, VoiceSignalPayload, WireMessage};

// ───── Join link types ──────────────────────────────────────────────────────

/// Token embedded in a shareable join URL. Contains enough
/// context to show the user what they're joining before connecting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinToken {
    pub inviter_peer_id: EndpointId,
    pub server_id: String,
    pub link_id: String,
    /// Human-readable server name for the join page.
    pub server_name: String,
    /// Display name of whoever generated the link.
    pub inviter_name: String,
    /// Optional bootstrap endpoint IDs (e.g. relay/worker `SyncProvider`s) the
    /// joiner can resolve via pkarr before the relay fallback. Empty for links
    /// generated before this field existed.
    ///
    /// `#[serde(default)]` keeps the JSON/self-describing-format decode path
    /// backward-compatible; the base64 share-URL path (bincode, positional) is
    /// handled explicitly in [`JoinToken::decode`] by retrying the legacy shape.
    #[serde(default)]
    pub bootstrap_endpoint_ids: Vec<EndpointId>,
}

impl JoinToken {
    /// Encode to a URL-safe base64 string.
    pub fn encode(&self) -> String {
        let bytes = willow_transport::pack(self).unwrap_or_default();
        crate::base64::encode(&bytes)
    }

    /// Decode from a base64 string.
    ///
    /// The share-URL payload is positional bincode, which does not honour
    /// `#[serde(default)]` for missing trailing fields. To keep links generated
    /// before `bootstrap_endpoint_ids` existed decodable, a failed full-shape
    /// decode falls back to the legacy five-field shape, defaulting the new
    /// field to an empty list.
    pub fn decode(s: &str) -> Option<Self> {
        let bytes = crate::base64::decode(s)?;
        if let Ok(token) = willow_transport::unpack::<Self>(&bytes) {
            return Some(token);
        }
        // Legacy fallback: a pre-`bootstrap_endpoint_ids` token has no trailing
        // `Vec` length prefix, so the positional decode above runs off the end.
        let legacy: LegacyJoinToken = willow_transport::unpack(&bytes).ok()?;
        Some(JoinToken {
            inviter_peer_id: legacy.inviter_peer_id,
            server_id: legacy.server_id,
            link_id: legacy.link_id,
            server_name: legacy.server_name,
            inviter_name: legacy.inviter_name,
            bootstrap_endpoint_ids: Vec::new(),
        })
    }
}

/// Pre-`bootstrap_endpoint_ids` wire shape of [`JoinToken`], retained solely so
/// [`JoinToken::decode`] can parse share URLs generated before that field was
/// added (bincode is positional and ignores `#[serde(default)]` for missing
/// trailing fields).
#[derive(Deserialize)]
struct LegacyJoinToken {
    inviter_peer_id: EndpointId,
    server_id: String,
    link_id: String,
    server_name: String,
    inviter_name: String,
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

/// Re-export `willow_state::Event` for convenience.
pub use willow_state::Event;

/// The gossipsub topic for server operations.
pub const SERVER_OPS_TOPIC: &str = "_willow_server_ops";

/// Global gossipsub topic for profile broadcasts.
pub const PROFILE_TOPIC: &str = "_willow_profiles";

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use willow_identity::Identity;
    use willow_state::EventHash;

    /// Helper to create a signed event for testing.
    fn make_event(id: &Identity, kind: willow_state::EventKind) -> willow_state::Event {
        willow_state::Event::new(id, 1, EventHash::ZERO, vec![], kind, 1000)
    }

    #[test]
    fn wire_message_event_round_trip() {
        let id = Identity::generate();
        let event = make_event(
            &id,
            willow_state::EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: "ch-1".to_string(),
                kind: willow_state::ChannelKind::Text,
                ephemeral: None,
            },
        );

        let msg = WireMessage::Event(Box::new(event.clone()));
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();

        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::Event(e) => {
                assert_eq!(e.hash, event.hash);
                assert_eq!(e.author, id.endpoint_id());
            }
            _ => panic!("expected WireMessage::Event"),
        }
    }

    #[test]
    fn wire_message_sync_request_round_trip() {
        let id = Identity::generate();
        let msg = WireMessage::SyncRequest {
            state_hash: EventHash::from_bytes(b"test-hash"),
            topic: Some("my-topic".to_string()),
        };

        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();

        match decoded {
            WireMessage::SyncRequest { state_hash, topic } => {
                assert_eq!(state_hash, EventHash::from_bytes(b"test-hash"));
                assert_eq!(topic, Some("my-topic".to_string()));
            }
            _ => panic!("expected WireMessage::SyncRequest"),
        }
    }

    #[test]
    fn wire_message_sync_batch_round_trip() {
        let id = Identity::generate();
        let peer1 = Identity::generate();
        let e1 = make_event(
            &peer1,
            willow_state::EventKind::CreateChannel {
                name: "ch1".to_string(),
                channel_id: "cid1".to_string(),
                kind: willow_state::ChannelKind::Text,
                ephemeral: None,
            },
        );
        let e2 = willow_state::Event::new(
            &peer1,
            2,
            e1.hash,
            vec![],
            willow_state::EventKind::Message {
                channel_id: "cid1".to_string(),
                body: "hello".to_string(),
                reply_to: None,
            },
            200,
        );

        let events = vec![e1.clone(), e2.clone()];
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
                assert_eq!(decoded_events[0].hash, e1.hash);
                assert_eq!(decoded_events[1].hash, e2.hash);
            }
            _ => panic!("expected WireMessage::SyncBatch"),
        }
    }

    #[test]
    fn wire_message_tampered_fails() {
        let id = Identity::generate();
        let event = make_event(
            &id,
            willow_state::EventKind::DeleteChannel {
                channel_id: "ch-1".to_string(),
            },
        );

        let mut data = pack_wire(&WireMessage::Event(Box::new(event)), &id).unwrap();
        if let Some(byte) = data.last_mut() {
            *byte ^= 0xFF;
        }

        assert!(unpack_wire(&data).is_none());
    }

    #[test]
    fn join_token_round_trip() {
        let inviter = Identity::generate();
        let token = JoinToken {
            inviter_peer_id: inviter.endpoint_id(),
            server_id: "srv-1".to_string(),
            link_id: "link-abc".to_string(),
            server_name: "My Server".to_string(),
            inviter_name: "Alice".to_string(),
            bootstrap_endpoint_ids: vec![],
        };
        let encoded = token.encode();
        let decoded = JoinToken::decode(&encoded).unwrap();
        assert_eq!(decoded.inviter_peer_id, inviter.endpoint_id());
        assert_eq!(decoded.server_id, "srv-1");
        assert_eq!(decoded.link_id, "link-abc");
        assert_eq!(decoded.server_name, "My Server");
        assert_eq!(decoded.inviter_name, "Alice");
        assert!(decoded.bootstrap_endpoint_ids.is_empty());
    }

    #[test]
    fn join_token_with_bootstrap_ids_round_trip() {
        let inviter = Identity::generate();
        let boot1 = Identity::generate().endpoint_id();
        let boot2 = Identity::generate().endpoint_id();
        let token = JoinToken {
            inviter_peer_id: inviter.endpoint_id(),
            server_id: "srv-1".to_string(),
            link_id: "link-abc".to_string(),
            server_name: "My Server".to_string(),
            inviter_name: "Alice".to_string(),
            bootstrap_endpoint_ids: vec![boot1, boot2],
        };
        let encoded = token.encode();
        let decoded = JoinToken::decode(&encoded).unwrap();
        assert_eq!(decoded.bootstrap_endpoint_ids, vec![boot1, boot2]);
        assert_eq!(decoded.inviter_peer_id, inviter.endpoint_id());
    }

    /// An **old** join link generated before `bootstrap_endpoint_ids` existed —
    /// whose wire bytes carry only the original five fields — must still decode.
    /// This pins the backward-compatibility guarantee for in-the-wild share URLs.
    #[test]
    fn old_join_token_without_bootstrap_ids_still_decodes() {
        // Mirror of the pre-bootstrap-ids `JoinToken` to produce legacy bytes.
        #[derive(serde::Serialize)]
        struct OldJoinToken {
            inviter_peer_id: EndpointId,
            server_id: String,
            link_id: String,
            server_name: String,
            inviter_name: String,
        }

        let inviter = Identity::generate();
        let old = OldJoinToken {
            inviter_peer_id: inviter.endpoint_id(),
            server_id: "srv-1".to_string(),
            link_id: "link-abc".to_string(),
            server_name: "My Server".to_string(),
            inviter_name: "Alice".to_string(),
        };
        // Reproduce the exact `JoinToken::encode` pipeline against the old shape.
        let bytes = willow_transport::pack(&old).unwrap();
        let encoded = crate::base64::encode(&bytes);

        let decoded = JoinToken::decode(&encoded)
            .expect("legacy join token without bootstrap_endpoint_ids must decode");
        assert_eq!(decoded.inviter_peer_id, inviter.endpoint_id());
        assert_eq!(decoded.server_id, "srv-1");
        assert_eq!(decoded.link_id, "link-abc");
        assert_eq!(decoded.server_name, "My Server");
        assert_eq!(decoded.inviter_name, "Alice");
        // The missing field defaults to an empty list.
        assert!(decoded.bootstrap_endpoint_ids.is_empty());
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
        let joiner = Identity::generate();
        let msg = WireMessage::JoinRequest {
            link_id: "link-1".to_string(),
            peer_id: joiner.endpoint_id(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::JoinRequest { link_id, peer_id } => {
                assert_eq!(link_id, "link-1");
                assert_eq!(peer_id, joiner.endpoint_id());
            }
            _ => panic!("expected JoinRequest"),
        }
    }

    #[test]
    fn wire_message_join_response_round_trip() {
        let id = Identity::generate();
        let joiner = Identity::generate();
        let msg = WireMessage::JoinResponse {
            link_id: "link-1".to_string(),
            target_peer: joiner.endpoint_id(),
            invite_data: "base64inviteblob".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::JoinResponse {
                link_id,
                target_peer,
                invite_data,
            } => {
                assert_eq!(link_id, "link-1");
                assert_eq!(target_peer, joiner.endpoint_id());
                assert_eq!(invite_data, "base64inviteblob");
            }
            _ => panic!("expected JoinResponse"),
        }
    }

    #[test]
    fn wire_message_join_denied_round_trip() {
        let id = Identity::generate();
        let joiner = Identity::generate();
        let msg = WireMessage::JoinDenied {
            link_id: "link-1".to_string(),
            target_peer: joiner.endpoint_id(),
            reason: "link_expired".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::JoinDenied {
                link_id,
                target_peer,
                reason,
            } => {
                assert_eq!(link_id, "link-1");
                assert_eq!(target_peer, joiner.endpoint_id());
                assert_eq!(reason, "link_expired");
            }
            _ => panic!("expected JoinDenied"),
        }
    }
}
