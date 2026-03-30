//! Heartbeat actor — broadcasts [`WorkerAnnouncement`] periodically.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};
use willow_identity::EndpointId;

use super::{NetworkOutMsg, StateMsg};
use crate::types::{WorkerAnnouncement, WorkerWireMessage, WORKERS_TOPIC};

/// Run the heartbeat actor loop.
///
/// Every `interval`, queries the state actor for role info
/// and broadcasts an announcement via the network actor.
pub async fn run(
    peer_id: EndpointId,
    interval: Duration,
    state_tx: mpsc::Sender<StateMsg>,
    network_tx: mpsc::Sender<NetworkOutMsg>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    debug!("heartbeat actor started (interval: {:?})", interval);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    // Send departure before exiting.
                    let departure = WorkerWireMessage::Departure {
                        peer_id: peer_id.clone(),
                    };
                    if let Ok(bytes) = bincode::serialize(&departure) {
                        if let Err(e) = network_tx
                            .send(NetworkOutMsg::Publish {
                                topic: WORKERS_TOPIC.to_string(),
                                data: bytes,
                            })
                            .await
                        {
                            warn!(%e, "failed to send departure message");
                        }
                    }
                    debug!("heartbeat actor shutting down");
                    return;
                }
            }
        }

        // Query state actor for role info.
        let (reply_tx, reply_rx) = oneshot::channel();
        if state_tx
            .send(StateMsg::GetRoleInfo { reply: reply_tx })
            .await
            .is_err()
        {
            warn!("state actor unavailable, heartbeat stopping");
            break;
        }

        let role_info = match reply_rx.await {
            Ok(info) => info,
            Err(_) => {
                warn!("state actor dropped reply, heartbeat stopping");
                break;
            }
        };

        let announcement = WorkerAnnouncement {
            peer_id: peer_id.clone(),
            role: role_info,
            servers: vec![], // Populated by state actor in the full runtime
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };

        let msg = WorkerWireMessage::Announcement(announcement);
        if let Ok(bytes) = bincode::serialize(&msg) {
            let _ = network_tx
                .send(NetworkOutMsg::Publish {
                    topic: WORKERS_TOPIC.to_string(),
                    data: bytes,
                })
                .await;
        }
    }

    debug!("heartbeat actor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WorkerRoleInfo;
    use std::time::Duration;
    use willow_identity::Identity;

    /// Minimal role info responder for testing.
    async fn fake_state_actor(mut rx: mpsc::Receiver<StateMsg>) {
        while let Some(msg) = rx.recv().await {
            match msg {
                StateMsg::GetRoleInfo { reply } => {
                    let _ = reply.send(WorkerRoleInfo::Replay {
                        servers_loaded: 1,
                        events_buffered: 42,
                        max_events: 1000,
                    });
                }
                StateMsg::Shutdown => break,
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn heartbeat_sends_announcements() {
        let (state_tx, state_rx) = mpsc::channel(32);
        let (network_tx, mut network_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        tokio::spawn(fake_state_actor(state_rx));

        let test_peer = Identity::generate().endpoint_id();
        let hb = tokio::spawn(run(
            test_peer,
            Duration::from_millis(50),
            state_tx,
            network_tx,
            shutdown_rx,
        ));

        // Wait for at least 1 announcement.
        let msg = tokio::time::timeout(Duration::from_secs(1), network_rx.recv())
            .await
            .unwrap()
            .unwrap();

        match msg {
            NetworkOutMsg::Publish { topic, data } => {
                assert_eq!(topic, WORKERS_TOPIC);
                let decoded: WorkerWireMessage = bincode::deserialize(&data).unwrap();
                match decoded {
                    WorkerWireMessage::Announcement(a) => {
                        assert_eq!(a.peer_id, test_peer);
                    }
                    _ => panic!("expected Announcement"),
                }
            }
            _ => panic!("expected Publish"),
        }

        shutdown_tx.send(true).unwrap();
        hb.await.unwrap();

        // Check departure message was sent.
        let departure = tokio::time::timeout(Duration::from_millis(100), network_rx.recv())
            .await
            .unwrap()
            .unwrap();
        match departure {
            NetworkOutMsg::Publish { data, .. } => {
                let decoded: WorkerWireMessage = bincode::deserialize(&data).unwrap();
                assert!(matches!(decoded, WorkerWireMessage::Departure { .. }));
            }
            _ => panic!("expected departure Publish"),
        }
    }
}
