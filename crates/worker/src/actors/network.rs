//! Network actor — owns the libp2p swarm, bridges gossipsub to other actors.

use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use willow_network::{NetworkEvent, NetworkNode};

use super::{NetworkOutMsg, StateMsg};
use crate::types::{WorkerWireMessage, WORKERS_TOPIC};

/// Run the network actor loop.
///
/// Receives gossipsub events from the swarm, dispatches to the state
/// actor. Receives outbound messages from other actors and publishes
/// to gossipsub.
pub async fn run(
    node: NetworkNode,
    mut events: mpsc::UnboundedReceiver<NetworkEvent>,
    state_tx: mpsc::Sender<StateMsg>,
    mut outbound_rx: mpsc::Receiver<NetworkOutMsg>,
    local_peer_id: String,
) {
    debug!("network actor started");

    // Subscribe to the workers topic.
    if let Err(e) = node.subscribe(WORKERS_TOPIC) {
        warn!(%e, "failed to subscribe to workers topic");
    }

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                match event {
                    NetworkEvent::Message { topic, data, .. } => {
                        handle_incoming_message(
                            &topic,
                            &data,
                            &state_tx,
                            &node,
                            &local_peer_id,
                        )
                        .await;
                    }
                    NetworkEvent::PeerConnected(peer) => {
                        debug!(%peer, "peer connected");
                    }
                    NetworkEvent::PeerDisconnected(peer) => {
                        debug!(%peer, "peer disconnected");
                    }
                    _ => {}
                }
            }
            msg = outbound_rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    NetworkOutMsg::Publish { topic, data } => {
                        if let Err(e) = node.publish(&topic, data) {
                            trace!(%e, %topic, "failed to publish");
                        }
                    }
                    NetworkOutMsg::Subscribe(topic) => {
                        if let Err(e) = node.subscribe(&topic) {
                            warn!(%e, %topic, "failed to subscribe");
                        }
                    }
                }
            }
        }
    }

    debug!("network actor stopped");
}

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
pub fn parse_worker_message(data: &[u8], local_peer_id: &str) -> WorkerMessageAction {
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
            if target_peer.is_empty() || target_peer == local_peer_id {
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
            willow_common::WireMessage::SyncBatch { events } => {
                ServerMessageAction::Events(events)
            }
            _ => ServerMessageAction::Ignore,
        }
    } else {
        ServerMessageAction::Ignore
    }
}

async fn handle_incoming_message(
    topic: &str,
    data: &[u8],
    state_tx: &mpsc::Sender<StateMsg>,
    node: &NetworkNode,
    local_peer_id: &str,
) {
    if topic == WORKERS_TOPIC {
        match parse_worker_message(data, local_peer_id) {
            WorkerMessageAction::HandleRequest {
                request_id,
                payload,
            } => {
                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                if state_tx
                    .send(StateMsg::Request {
                        req: payload,
                        reply: reply_tx,
                    })
                    .await
                    .is_err()
                {
                    warn!("state actor unavailable for request {request_id}");
                    return;
                }

                let resp = match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    reply_rx,
                )
                .await
                {
                    Ok(Ok(resp)) => resp,
                    Ok(Err(_)) => {
                        warn!(%request_id, "state actor dropped reply channel");
                        return;
                    }
                    Err(_) => {
                        warn!(%request_id, "request timed out after 5s");
                        return;
                    }
                };

                let response_msg = WorkerWireMessage::Response {
                    request_id: request_id.clone(),
                    target_peer: String::new(),
                    payload: resp,
                };
                if let Ok(bytes) = bincode::serialize(&response_msg) {
                    if let Err(e) = node.publish(WORKERS_TOPIC, bytes) {
                        debug!(%e, %request_id, "failed to publish response");
                    }
                }
            }
            WorkerMessageAction::Ignore => {}
            WorkerMessageAction::DeserializeError(e) => {
                debug!(%e, "failed to deserialize worker message");
            }
        }
    } else {
        match parse_server_message(data) {
            ServerMessageAction::Events(events) => {
                for event in events {
                    let _ = state_tx.send(StateMsg::Event(event)).await;
                }
            }
            ServerMessageAction::Ignore => {
                trace!(topic, bytes = data.len(), "unrecognized message on topic");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_common::{WorkerRequest, WorkerResponse};
    use willow_state::StateHash;

    #[test]
    fn parse_worker_request_targeted_at_us() {
        let msg = WorkerWireMessage::Request {
            request_id: "req-1".to_string(),
            target_peer: "my-peer".to_string(),
            payload: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                state_hash: StateHash::ZERO,
            },
        };
        let data = bincode::serialize(&msg).unwrap();

        match parse_worker_message(&data, "my-peer") {
            WorkerMessageAction::HandleRequest { request_id, .. } => {
                assert_eq!(request_id, "req-1");
            }
            other => panic!("expected HandleRequest, got {:?}", other),
        }
    }

    #[test]
    fn parse_worker_request_broadcast() {
        let msg = WorkerWireMessage::Request {
            request_id: "req-2".to_string(),
            target_peer: String::new(), // broadcast
            payload: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                state_hash: StateHash::ZERO,
            },
        };
        let data = bincode::serialize(&msg).unwrap();

        match parse_worker_message(&data, "any-peer") {
            WorkerMessageAction::HandleRequest { request_id, .. } => {
                assert_eq!(request_id, "req-2");
            }
            other => panic!("expected HandleRequest, got {:?}", other),
        }
    }

    #[test]
    fn parse_worker_request_not_for_us() {
        let msg = WorkerWireMessage::Request {
            request_id: "req-3".to_string(),
            target_peer: "other-peer".to_string(),
            payload: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                state_hash: StateHash::ZERO,
            },
        };
        let data = bincode::serialize(&msg).unwrap();

        assert!(matches!(
            parse_worker_message(&data, "my-peer"),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_announcement_ignored() {
        let msg = WorkerWireMessage::Announcement(willow_common::WorkerAnnouncement {
            peer_id: "w1".to_string(),
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
            parse_worker_message(&data, "my-peer"),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_departure_ignored() {
        let msg = WorkerWireMessage::Departure {
            peer_id: "w1".to_string(),
        };
        let data = bincode::serialize(&msg).unwrap();

        assert!(matches!(
            parse_worker_message(&data, "my-peer"),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_response_ignored() {
        let msg = WorkerWireMessage::Response {
            request_id: "r1".to_string(),
            target_peer: "my-peer".to_string(),
            payload: WorkerResponse::Denied {
                reason: "test".to_string(),
            },
        };
        let data = bincode::serialize(&msg).unwrap();

        assert!(matches!(
            parse_worker_message(&data, "my-peer"),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_garbage_data() {
        assert!(matches!(
            parse_worker_message(b"not valid bincode", "peer"),
            WorkerMessageAction::DeserializeError(_)
        ));
    }

    #[test]
    fn parse_worker_empty_data() {
        assert!(matches!(
            parse_worker_message(&[], "peer"),
            WorkerMessageAction::DeserializeError(_)
        ));
    }

    #[test]
    fn parse_server_message_with_signed_event() {
        let id = willow_identity::Identity::generate();
        let event = willow_state::Event {
            id: "e1".to_string(),
            parent_hash: StateHash::ZERO,
            author: id.peer_id().to_string(),
            timestamp_ms: 1000,
            kind: willow_state::EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello".to_string(),
                reply_to: None,
            },
        };

        let data =
            willow_common::pack_wire(&willow_common::WireMessage::Event(event.clone()), &id)
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
                author: "p".to_string(),
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
                author: "p".to_string(),
                timestamp_ms: 200,
                kind: willow_state::EventKind::Message {
                    channel_id: "c1".to_string(),
                    body: "msg".to_string(),
                    reply_to: None,
                },
            },
        ];

        let data =
            willow_common::pack_wire(&willow_common::WireMessage::SyncBatch { events }, &id)
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
