//! Heartbeat actor — broadcasts [`WorkerAnnouncement`] periodically.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::{debug, warn};
use willow_identity::EndpointId;
use willow_network::TopicHandle;

use super::StateMsg;
use crate::types::{WorkerAnnouncement, WorkerWireMessage};

/// Run the heartbeat actor loop.
///
/// Every `interval`, queries the state actor for role info
/// and broadcasts an announcement via the topic handle.
pub async fn run<T: TopicHandle>(
    peer_id: EndpointId,
    interval: Duration,
    state_tx: mpsc::Sender<StateMsg>,
    topic: T,
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
                        if let Err(e) = topic.broadcast(bytes::Bytes::from(bytes)).await {
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
            let _ = topic.broadcast(bytes::Bytes::from(bytes)).await;
        }
    }

    debug!("heartbeat actor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WorkerRoleInfo, WORKERS_TOPIC};
    use std::time::Duration;
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::{Network, TopicEvents};

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
        let hub = MemHub::new();
        let net_a = MemNetwork::new(&hub);
        let net_b = MemNetwork::new(&hub);

        let topic_id = willow_network::topic_id(WORKERS_TOPIC);
        let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
        let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

        let (state_tx, state_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        tokio::spawn(fake_state_actor(state_rx));

        let test_peer = net_a.id();
        let hb = tokio::spawn(run(
            test_peer,
            Duration::from_millis(50),
            state_tx,
            sender_a,
            shutdown_rx,
        ));

        // Wait for at least 1 announcement — drain neighbor events first.
        let data = loop {
            let event = tokio::time::timeout(Duration::from_secs(2), events_b.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let willow_network::GossipEvent::Received(msg) = event {
                break msg.content;
            }
        };

        let decoded: WorkerWireMessage = bincode::deserialize(&data).unwrap();
        match decoded {
            WorkerWireMessage::Announcement(a) => {
                assert_eq!(a.peer_id, test_peer);
            }
            _ => panic!("expected Announcement"),
        }

        shutdown_tx.send(true).unwrap();
        hb.await.unwrap();

        // Check departure message was sent — drain any neighbor events.
        let departure_data = loop {
            let event = tokio::time::timeout(Duration::from_millis(500), events_b.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let willow_network::GossipEvent::Received(msg) = event {
                break msg.content;
            }
        };
        let decoded: WorkerWireMessage = bincode::deserialize(&departure_data).unwrap();
        assert!(matches!(decoded, WorkerWireMessage::Departure { .. }));
    }
}
