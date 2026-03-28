//! Sync actor — periodically broadcasts SyncRequests for state convergence.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::debug;

use super::{NetworkOutMsg, StateMsg};
use crate::types::{WorkerRequest, WorkerWireMessage, WORKERS_TOPIC};

/// Run the sync actor loop.
///
/// Every `interval`, queries the state actor for state hashes per server
/// and broadcasts SyncRequests so other peers/workers can send missing events.
pub async fn run(
    peer_id: String,
    interval: Duration,
    state_tx: mpsc::Sender<StateMsg>,
    network_tx: mpsc::Sender<NetworkOutMsg>,
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
                target_peer: String::new(), // Broadcast — any peer can respond
                payload: WorkerRequest::Sync {
                    server_id,
                    state_hash,
                },
            };
            if let Ok(bytes) = bincode::serialize(&msg) {
                let _ = network_tx
                    .send(NetworkOutMsg::Publish {
                        topic: WORKERS_TOPIC.to_string(),
                        data: bytes,
                    })
                    .await;
            }
        }
    }

    debug!("sync actor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let (state_tx, state_rx) = mpsc::channel(32);
        let (network_tx, mut network_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        tokio::spawn(fake_state_actor(state_rx));

        let sync = tokio::spawn(run(
            "test-peer".to_string(),
            Duration::from_millis(50),
            state_tx,
            network_tx,
            shutdown_rx,
        ));

        // Should get 2 sync requests (one per server).
        let msg1 = tokio::time::timeout(Duration::from_secs(1), network_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let msg2 = tokio::time::timeout(Duration::from_secs(1), network_rx.recv())
            .await
            .unwrap()
            .unwrap();

        let mut server_ids = vec![];
        for msg in [msg1, msg2] {
            match msg {
                NetworkOutMsg::Publish { topic, data } => {
                    assert_eq!(topic, WORKERS_TOPIC);
                    let decoded: WorkerWireMessage = bincode::deserialize(&data).unwrap();
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
                _ => panic!("expected Publish"),
            }
        }

        server_ids.sort();
        assert_eq!(server_ids, vec!["server-a", "server-b"]);

        shutdown_tx.send(true).unwrap();
        sync.await.unwrap();
    }

    #[tokio::test]
    async fn sync_actor_exits_on_shutdown() {
        let (state_tx, _state_rx) = mpsc::channel(32);
        let (network_tx, _network_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let sync = tokio::spawn(run(
            "test-peer".to_string(),
            Duration::from_secs(60),
            state_tx,
            network_tx,
            shutdown_rx,
        ));

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), sync)
            .await
            .unwrap()
            .unwrap();
    }
}
