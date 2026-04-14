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
    /// Request events since a given state.
    SyncRequest {
        /// The event hash the sender's state is at — the responder
        /// returns events that the sender is missing.
        state_hash: willow_state::EventHash,
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
    /// Announce channel topics this peer is subscribed to, so the relay
    /// can dynamically subscribe and serve as bootstrap for those topics.
    TopicAnnounce {
        /// Topic name strings (e.g. "{server_id}/{channel_name}").
        topics: Vec<String>,
    },
    /// A signed worker node message (announcement, departure, request, or response).
    ///
    /// Worker gossip messages travel on the `_willow_workers` topic.
    /// They are wrapped in this variant so they share the same Ed25519-signed
    /// envelope as all other gossipsub messages.
    Worker(crate::WorkerWireMessage),
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
    use willow_state::{EventHash, EventKind};

    fn make_event(id: &Identity, kind: EventKind) -> willow_state::Event {
        willow_state::Event::new(id, 1, EventHash::ZERO, vec![], kind, 1000)
    }

    #[test]
    fn pack_unpack_event_round_trip() {
        let id = Identity::generate();
        let event = make_event(
            &id,
            EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello from common".to_string(),
                reply_to: None,
            },
        );
        let event_hash = event.hash;

        let msg = WireMessage::Event(event);
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();

        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::Event(e) => assert_eq!(e.hash, event_hash),
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn pack_unpack_sync_batch_round_trip() {
        let id = Identity::generate();
        let events = vec![make_event(
            &id,
            EventKind::CreateChannel {
                name: "ch".to_string(),
                channel_id: "cid".to_string(),
                kind: willow_state::ChannelKind::Text,
            },
        )];

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
            state_hash: EventHash::from_bytes(b"test"),
            topic: Some("_willow_server_ops".to_string()),
        };

        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();

        match decoded {
            WireMessage::SyncRequest { state_hash, topic } => {
                assert_eq!(state_hash, EventHash::from_bytes(b"test"));
                assert_eq!(topic, Some("_willow_server_ops".to_string()));
            }
            _ => panic!("expected SyncRequest"),
        }
    }

    #[test]
    fn tampered_data_fails_unpack() {
        let id = Identity::generate();
        let event = make_event(
            &id,
            EventKind::DeleteChannel {
                channel_id: "c".to_string(),
            },
        );
        let msg = WireMessage::Event(event);

        let mut data = pack_wire(&msg, &id).unwrap();
        if let Some(b) = data.last_mut() {
            *b ^= 0xFF;
        }
        assert!(unpack_wire(&data).is_none());
    }

    #[test]
    fn pack_unpack_worker_message_round_trip() {
        use crate::{WorkerAnnouncement, WorkerRoleInfo, WorkerWireMessage};
        let id = Identity::generate();
        let announcement = WorkerAnnouncement {
            peer_id: id.endpoint_id(),
            role: WorkerRoleInfo::Replay {
                servers_loaded: 2,
                events_buffered: 100,
                max_events: 1000,
            },
            servers: vec!["srv-abc".to_string()],
            timestamp: 12345,
        };
        let msg = WireMessage::Worker(WorkerWireMessage::Announcement(announcement.clone()));
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::Worker(WorkerWireMessage::Announcement(a)) => {
                assert_eq!(a.peer_id, announcement.peer_id);
                assert_eq!(a.servers, announcement.servers);
                assert_eq!(a.timestamp, announcement.timestamp);
            }
            _ => panic!("expected Worker(Announcement)"),
        }
    }

    #[test]
    fn pack_unpack_typing_indicator_round_trip() {
        let id = Identity::generate();
        let msg = WireMessage::TypingIndicator {
            channel: "general".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::TypingIndicator { channel } => {
                assert_eq!(channel, "general");
            }
            _ => panic!("expected TypingIndicator"),
        }
    }

    #[test]
    fn pack_unpack_topic_announce_round_trip() {
        let id = Identity::generate();
        let topics = vec![
            "srv-abc/general".to_string(),
            "srv-abc/announcements".to_string(),
        ];
        let msg = WireMessage::TopicAnnounce {
            topics: topics.clone(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::TopicAnnounce {
                topics: decoded_topics,
            } => {
                assert_eq!(decoded_topics, topics);
            }
            _ => panic!("expected TopicAnnounce"),
        }
    }

    #[test]
    fn pack_unpack_join_request_round_trip() {
        use willow_identity::Identity;
        let id = Identity::generate();
        let peer = Identity::generate().endpoint_id();
        let msg = WireMessage::JoinRequest {
            link_id: "link-xyz".to_string(),
            peer_id: peer,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, signer) = unpack_wire(&data).unwrap();
        assert_eq!(signer, id.endpoint_id());
        match decoded {
            WireMessage::JoinRequest { link_id, peer_id } => {
                assert_eq!(link_id, "link-xyz");
                assert_eq!(peer_id, peer);
            }
            _ => panic!("expected JoinRequest"),
        }
    }

    #[test]
    fn pack_unpack_join_response_round_trip() {
        let id = Identity::generate();
        let target = Identity::generate().endpoint_id();
        let msg = WireMessage::JoinResponse {
            target_peer: target,
            invite_data: "encrypted-invite-payload".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::JoinResponse {
                target_peer,
                invite_data,
            } => {
                assert_eq!(target_peer, target);
                assert_eq!(invite_data, "encrypted-invite-payload");
            }
            _ => panic!("expected JoinResponse"),
        }
    }

    #[test]
    fn pack_unpack_join_denied_round_trip() {
        let id = Identity::generate();
        let target = Identity::generate().endpoint_id();
        let msg = WireMessage::JoinDenied {
            target_peer: target,
            reason: "invite expired".to_string(),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::JoinDenied {
                target_peer,
                reason,
            } => {
                assert_eq!(target_peer, target);
                assert_eq!(reason, "invite expired");
            }
            _ => panic!("expected JoinDenied"),
        }
    }

    #[test]
    fn pack_unpack_voice_join_round_trip() {
        let id = Identity::generate();
        let peer = Identity::generate().endpoint_id();
        let msg = WireMessage::VoiceJoin {
            channel_id: "voice-1".to_string(),
            peer_id: peer,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::VoiceJoin {
                channel_id,
                peer_id,
            } => {
                assert_eq!(channel_id, "voice-1");
                assert_eq!(peer_id, peer);
            }
            _ => panic!("expected VoiceJoin"),
        }
    }

    #[test]
    fn pack_unpack_voice_leave_round_trip() {
        let id = Identity::generate();
        let peer = Identity::generate().endpoint_id();
        let msg = WireMessage::VoiceLeave {
            channel_id: "voice-1".to_string(),
            peer_id: peer,
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::VoiceLeave {
                channel_id,
                peer_id,
            } => {
                assert_eq!(channel_id, "voice-1");
                assert_eq!(peer_id, peer);
            }
            _ => panic!("expected VoiceLeave"),
        }
    }

    #[test]
    fn pack_unpack_voice_signal_offer_round_trip() {
        let id = Identity::generate();
        let target = Identity::generate().endpoint_id();
        let msg = WireMessage::VoiceSignal {
            channel_id: "voice-2".to_string(),
            target_peer: target,
            signal: VoiceSignalPayload::Offer("sdp-offer-data".to_string()),
        };
        let data = pack_wire(&msg, &id).unwrap();
        let (decoded, _) = unpack_wire(&data).unwrap();
        match decoded {
            WireMessage::VoiceSignal {
                channel_id,
                target_peer,
                signal,
            } => {
                assert_eq!(channel_id, "voice-2");
                assert_eq!(target_peer, target);
                match signal {
                    VoiceSignalPayload::Offer(sdp) => assert_eq!(sdp, "sdp-offer-data"),
                    _ => panic!("expected Offer signal"),
                }
            }
            _ => panic!("expected VoiceSignal"),
        }
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
