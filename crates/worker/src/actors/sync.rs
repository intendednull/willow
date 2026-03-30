//! Sync actor — periodically broadcasts SyncRequests for state convergence.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::debug;
use willow_network::TopicHandle;

use super::StateMsg;
use crate::types::{WorkerRequest, WorkerWireMessage};

/// Run the sync actor loop.
///
/// Every `interval`, queries the state actor for state hashes per server
/// and broadcasts SyncRequests so other peers/workers can send missing events.
pub async fn run<T: TopicHandle>(
    _peer_id: willow_identity::EndpointId,
    interval: Duration,
    state_tx: mpsc::Sender<StateMsg>,
    topic: T,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    debug!("sync actor started (interval: {:?})", interval);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    debug!("sync actor shutting down");
                    return;
                }
            }
        }

        // Query state actor for current state hashes.
        let (reply_tx, reply_rx) = oneshot::channel();
        if state_tx
            .send(StateMsg::GetStateHashes { reply: reply_tx })
            .await
            .is_err()
        {
            break;
        }

        let hashes = match reply_rx.await {
            Ok(h) => h,
            Err(_) => break,
        };

        // Broadcast a sync request for each server.
        for (server_id, state_hash) in hashes {
            let msg = WorkerWireMessage::Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                target_peer: _peer_id, // Self-addressed — peers match on topic
                payload: WorkerRequest::Sync {
                    server_id,
                    state_hash,
                },
            };
            if let Ok(bytes) = bincode::serialize(&msg) {
                let _ = topic.broadcast(bytes::Bytes::from(bytes)).await;
            }
        }
    }

    debug!("sync actor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WORKERS_TOPIC;
    use willow_identity::Identity;
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::Network;
    use willow_state::StateHash;

    /// Fake state actor that returns known state hashes.
    async fn fake_state_actor(mut rx: mpsc::Receiver<StateMsg>) {
        while let Some(msg) = rx.recv().await {
            match msg {
                StateMsg::GetStateHashes { reply } => {
                    let _ = reply.send(vec![
                        ("server-a".to_string(), StateHash::ZERO),
                        ("server-b".to_string(), StateHash::ZERO),
                    ]);
                }
                StateMsg::Shutdown => break,
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn sync_actor_broadcasts_sync_requests() {
        let hub = MemHub::new();
        let net_a = MemNetwork::new(&hub);
        let net_b = MemNetwork::new(&hub);

        let topic_id = willow_network::topic_id(WORKERS_TOPIC);
        let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
        let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

        let (state_tx, state_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        tokio::spawn(fake_state_actor(state_rx));

        let sync = tokio::spawn(run(
            Identity::generate().endpoint_id(),
            Duration::from_millis(50),
            state_tx,
            sender_a,
            shutdown_rx,
        ));

        // Collect 2 sync requests (one per server) — drain neighbor events.
        let mut server_ids = vec![];
        while server_ids.len() < 2 {
            let event = tokio::time::timeout(Duration::from_secs(2), events_b.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let willow_network::GossipEvent::Received(msg) = event {
                let decoded: WorkerWireMessage = bincode::deserialize(&msg.content).unwrap();
                match decoded {
                    WorkerWireMessage::Request { payload, .. } => match payload {
                        WorkerRequest::Sync { server_id, .. } => {
                            server_ids.push(server_id);
                        }
                        _ => panic!("expected Sync request"),
                    },
                    _ => panic!("expected Request"),
                }
            }
        }

        server_ids.sort();
        assert_eq!(server_ids, vec!["server-a", "server-b"]);

        shutdown_tx.send(true).unwrap();
        sync.await.unwrap();
    }

    #[tokio::test]
    async fn sync_actor_exits_on_shutdown() {
        let hub = MemHub::new();
        let net = MemNetwork::new(&hub);

        let topic_id = willow_network::topic_id(WORKERS_TOPIC);
        let (sender, _events) = net.subscribe(topic_id, vec![]).await.unwrap();

        let (state_tx, _state_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let sync = tokio::spawn(run(
            Identity::generate().endpoint_id(),
            Duration::from_secs(60),
            state_tx,
            sender,
            shutdown_rx,
        ));

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), sync)
            .await
            .unwrap()
            .unwrap();
    }
}
