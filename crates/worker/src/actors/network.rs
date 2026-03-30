//! Network actor — bridges gossip to other actors.
//!
//! TODO: Phase 3 will rewrite `run()` to use the Network trait from willow-network.
//! The pure parse functions below are kept and tested independently.

use willow_identity::EndpointId;

use crate::types::WorkerWireMessage;

/// Action produced by parsing an incoming worker topic message.
#[derive(Debug)]
pub enum WorkerMessageAction {
    /// Forward a request to the state actor and publish the response.
    HandleRequest {
        request_id: String,
        payload: willow_common::WorkerRequest,
    },
    /// No action needed (message not for us, or announcement/departure).
    Ignore,
    /// Could not deserialize the message.
    DeserializeError(String),
}

/// Parse a worker topic message and decide what action to take.
///
/// This is a pure function — no I/O, no channels — so it's easily
/// testable. The caller handles the actual I/O.
pub fn parse_worker_message(data: &[u8], local_peer_id: &EndpointId) -> WorkerMessageAction {
    let msg = match bincode::deserialize::<WorkerWireMessage>(data) {
        Ok(m) => m,
        Err(e) => return WorkerMessageAction::DeserializeError(e.to_string()),
    };

    match msg {
        WorkerWireMessage::Request {
            target_peer,
            payload,
            request_id,
        } => {
            if target_peer == *local_peer_id {
                WorkerMessageAction::HandleRequest {
                    request_id,
                    payload,
                }
            } else {
                WorkerMessageAction::Ignore
            }
        }
        WorkerWireMessage::Response { .. }
        | WorkerWireMessage::Announcement(_)
        | WorkerWireMessage::Departure { .. } => WorkerMessageAction::Ignore,
    }
}

/// Action produced by parsing a server ops / channel topic message.
#[derive(Debug)]
pub enum ServerMessageAction {
    /// One or more events to forward to the state actor.
    Events(Vec<willow_state::Event>),
    /// Could not parse the message (not an error — could be typing, voice, etc).
    Ignore,
}

/// Parse a signed server ops message and extract events.
///
/// Pure function — no I/O. Uses `willow_common::unpack_wire` to verify
/// the signature and deserialize.
pub fn parse_server_message(data: &[u8]) -> ServerMessageAction {
    if let Some((wire_msg, _signer)) = willow_common::unpack_wire(data) {
        match wire_msg {
            willow_common::WireMessage::Event(event) => ServerMessageAction::Events(vec![event]),
            willow_common::WireMessage::SyncBatch { events } => ServerMessageAction::Events(events),
            _ => ServerMessageAction::Ignore,
        }
    } else {
        ServerMessageAction::Ignore
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_common::{WorkerRequest, WorkerResponse};
    use willow_identity::Identity;
    use willow_state::StateHash;

    fn gen_id() -> EndpointId {
        Identity::generate().endpoint_id()
    }

    #[test]
    fn parse_worker_request_targeted_at_us() {
        let my_id = gen_id();
        let msg = WorkerWireMessage::Request {
            request_id: "req-1".to_string(),
            target_peer: my_id,
            payload: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                state_hash: StateHash::ZERO,
            },
        };
        let data = bincode::serialize(&msg).unwrap();

        match parse_worker_message(&data, &my_id) {
            WorkerMessageAction::HandleRequest { request_id, .. } => {
                assert_eq!(request_id, "req-1");
            }
            other => panic!("expected HandleRequest, got {:?}", other),
        }
    }

    #[test]
    fn parse_worker_request_not_for_us() {
        let my_id = gen_id();
        let other_id = gen_id();
        let msg = WorkerWireMessage::Request {
            request_id: "req-3".to_string(),
            target_peer: other_id,
            payload: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                state_hash: StateHash::ZERO,
            },
        };
        let data = bincode::serialize(&msg).unwrap();

        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_announcement_ignored() {
        let my_id = gen_id();
        let msg = WorkerWireMessage::Announcement(willow_common::WorkerAnnouncement {
            peer_id: gen_id(),
            role: willow_common::WorkerRoleInfo::Replay {
                servers_loaded: 1,
                events_buffered: 0,
                max_events: 1000,
            },
            servers: vec![],
            timestamp: 0,
        });
        let data = bincode::serialize(&msg).unwrap();

        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_departure_ignored() {
        let my_id = gen_id();
        let msg = WorkerWireMessage::Departure {
            peer_id: gen_id(),
        };
        let data = bincode::serialize(&msg).unwrap();

        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_response_ignored() {
        let my_id = gen_id();
        let msg = WorkerWireMessage::Response {
            request_id: "r1".to_string(),
            target_peer: my_id,
            payload: Box::new(WorkerResponse::Denied {
                reason: "test".to_string(),
            }),
        };
        let data = bincode::serialize(&msg).unwrap();

        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_garbage_data() {
        let my_id = gen_id();
        assert!(matches!(
            parse_worker_message(b"not valid bincode", &my_id),
            WorkerMessageAction::DeserializeError(_)
        ));
    }

    #[test]
    fn parse_worker_empty_data() {
        let my_id = gen_id();
        assert!(matches!(
            parse_worker_message(&[], &my_id),
            WorkerMessageAction::DeserializeError(_)
        ));
    }

    #[test]
    fn parse_server_message_with_signed_event() {
        let id = willow_identity::Identity::generate();
        let event = willow_state::Event {
            id: "e1".to_string(),
            parent_hash: StateHash::ZERO,
            author: id.endpoint_id(),
            timestamp_ms: 1000,
            kind: willow_state::EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello".to_string(),
                reply_to: None,
            },
        };

        let data = willow_common::pack_wire(&willow_common::WireMessage::Event(event.clone()), &id)
            .unwrap();

        match parse_server_message(&data) {
            ServerMessageAction::Events(events) => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].id, "e1");
            }
            ServerMessageAction::Ignore => panic!("expected Events"),
        }
    }

    #[test]
    fn parse_server_message_with_sync_batch() {
        let id = willow_identity::Identity::generate();
        let events = vec![
            willow_state::Event {
                id: "e1".to_string(),
                parent_hash: StateHash::ZERO,
                author: id.endpoint_id(),
                timestamp_ms: 100,
                kind: willow_state::EventKind::CreateChannel {
                    name: "ch".to_string(),
                    channel_id: "c1".to_string(),
                    kind: "text".to_string(),
                },
            },
            willow_state::Event {
                id: "e2".to_string(),
                parent_hash: StateHash::ZERO,
                author: id.endpoint_id(),
                timestamp_ms: 200,
                kind: willow_state::EventKind::Message {
                    channel_id: "c1".to_string(),
                    body: "msg".to_string(),
                    reply_to: None,
                },
            },
        ];

        let data = willow_common::pack_wire(&willow_common::WireMessage::SyncBatch { events }, &id)
            .unwrap();

        match parse_server_message(&data) {
            ServerMessageAction::Events(events) => assert_eq!(events.len(), 2),
            ServerMessageAction::Ignore => panic!("expected Events"),
        }
    }

    #[test]
    fn parse_server_message_typing_indicator_ignored() {
        let id = willow_identity::Identity::generate();
        let data = willow_common::pack_wire(
            &willow_common::WireMessage::TypingIndicator {
                channel: "general".to_string(),
            },
            &id,
        )
        .unwrap();

        assert!(matches!(
            parse_server_message(&data),
            ServerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_server_message_garbage_ignored() {
        assert!(matches!(
            parse_server_message(b"garbage data"),
            ServerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_server_message_empty_ignored() {
        assert!(matches!(
            parse_server_message(&[]),
            ServerMessageAction::Ignore
        ));
    }
}
