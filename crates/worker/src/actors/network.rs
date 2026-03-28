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

async fn handle_incoming_message(
    topic: &str,
    data: &[u8],
    state_tx: &mpsc::Sender<StateMsg>,
    node: &NetworkNode,
    local_peer_id: &str,
) {
    if topic == WORKERS_TOPIC {
        // Try to decode as WorkerWireMessage.
        let msg = match bincode::deserialize::<WorkerWireMessage>(data) {
            Ok(m) => m,
            Err(e) => {
                debug!(%e, "failed to deserialize worker message");
                return;
            }
        };

        match msg {
            WorkerWireMessage::Request {
                target_peer,
                payload,
                request_id,
            } => {
                // Only handle if targeted at us or broadcast (empty target).
                if target_peer.is_empty() || target_peer == local_peer_id {
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

                    // Wait for response with a timeout.
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

                    // Publish the response back via gossipsub.
                    let response_msg = WorkerWireMessage::Response {
                        request_id: request_id.clone(),
                        target_peer: String::new(), // Requester filters by request_id
                        payload: resp,
                    };
                    if let Ok(bytes) = bincode::serialize(&response_msg) {
                        if let Err(e) = node.publish(WORKERS_TOPIC, bytes) {
                            debug!(%e, %request_id, "failed to publish response");
                        }
                    }
                }
            }
            WorkerWireMessage::Response { target_peer, .. } => {
                if target_peer == local_peer_id {
                    debug!("received response to our request");
                }
            }
            WorkerWireMessage::Announcement(_) | WorkerWireMessage::Departure { .. } => {
                // Workers don't track other workers' announcements.
                // Clients handle this via worker_cache.
            }
        }
    } else {
        // Server ops or channel topic — try to decode as a state event.
        if let Ok(event) = bincode::deserialize::<willow_state::Event>(data) {
            let _ = state_tx.send(StateMsg::Event(event)).await;
        } else {
            trace!(topic, bytes = data.len(), "unrecognized message on topic");
        }
    }
}
