//! State actor — owns the [`WorkerRole`] and processes messages sequentially.
//!
//! All mutable state access goes through this actor. No locks needed
//! because only this task touches the role.

use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::StateMsg;
use crate::WorkerRole;

/// Run the state actor loop.
///
/// Receives messages on `rx`, dispatches to the `role` implementation.
/// Exits when `rx` is closed (all senders dropped) or Shutdown is received.
pub async fn run(mut role: Box<dyn WorkerRole>, mut rx: mpsc::Receiver<StateMsg>) {
    debug!("state actor started");

    while let Some(msg) = rx.recv().await {
        match msg {
            StateMsg::Event(event) => {
                role.on_event(&event);
            }
            StateMsg::Request { req, reply } => {
                let response = role.handle_request(req);
                if reply.send(response).is_err() {
                    warn!("request reply channel closed");
                }
            }
            StateMsg::GetRoleInfo { reply } => {
                let info = role.role_info();
                let _ = reply.send(info);
            }
            StateMsg::GetStateHashes { reply } => {
                // Default: no state hashes. Replay nodes override this
                // behavior via their WorkerRole implementation.
                let _ = reply.send(vec![]);
            }
            StateMsg::ServerDiscovered { server_id } => {
                debug!(%server_id, "server discovered by state actor");
            }
            StateMsg::Shutdown => {
                debug!("state actor shutting down");
                break;
            }
        }
    }

    debug!("state actor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WorkerRequest, WorkerResponse, WorkerRoleInfo};
    use tokio::sync::oneshot;
    use willow_state::{Event, EventKind, StateHash};

    /// A minimal test role that counts events and echoes requests.
    struct TestRole {
        event_count: u32,
    }

    impl TestRole {
        fn new() -> Self {
            Self { event_count: 0 }
        }
    }

    impl WorkerRole for TestRole {
        fn role_info(&self) -> WorkerRoleInfo {
            WorkerRoleInfo::Replay {
                servers_loaded: 1,
                events_buffered: self.event_count,
                max_events: 100,
            }
        }

        fn on_event(&mut self, _event: &Event) {
            self.event_count += 1;
        }

        fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
            match req {
                WorkerRequest::Sync { .. } => WorkerResponse::SyncBatch { events: vec![] },
                WorkerRequest::History { .. } => WorkerResponse::Denied {
                    reason: "not a storage node".to_string(),
                },
            }
        }
    }

    fn make_test_event() -> Event {
        Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: StateHash::ZERO,
            author: willow_identity::Identity::generate().endpoint_id(),
            timestamp_ms: 1000,
            kind: EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello".to_string(),
                reply_to: None,
            },
        }
    }

    #[tokio::test]
    async fn state_actor_processes_events() {
        let (tx, rx) = mpsc::channel(32);
        let role = Box::new(TestRole::new());

        let handle = tokio::spawn(run(role, rx));

        for _ in 0..3 {
            tx.send(StateMsg::Event(make_test_event())).await.unwrap();
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(StateMsg::GetRoleInfo { reply: reply_tx })
            .await
            .unwrap();
        let info = reply_rx.await.unwrap();
        match info {
            WorkerRoleInfo::Replay {
                events_buffered, ..
            } => assert_eq!(events_buffered, 3),
            _ => panic!("expected Replay"),
        }

        tx.send(StateMsg::Shutdown).await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn state_actor_handles_requests() {
        let (tx, rx) = mpsc::channel(32);
        let role = Box::new(TestRole::new());

        let handle = tokio::spawn(run(role, rx));

        // Sync request.
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(StateMsg::Request {
            req: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                state_hash: StateHash::ZERO,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();
        match reply_rx.await.unwrap() {
            WorkerResponse::SyncBatch { events } => assert!(events.is_empty()),
            _ => panic!("expected SyncBatch"),
        }

        // History request (denied by replay role).
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(StateMsg::Request {
            req: WorkerRequest::History {
                server_id: "srv".to_string(),
                channel: "general".to_string(),
                before_timestamp: None,
                limit: 50,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();
        match reply_rx.await.unwrap() {
            WorkerResponse::Denied { reason } => assert!(reason.contains("not a storage")),
            _ => panic!("expected Denied"),
        }

        tx.send(StateMsg::Shutdown).await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn state_actor_exits_on_channel_close() {
        let (tx, rx) = mpsc::channel(32);
        let role = Box::new(TestRole::new());

        let handle = tokio::spawn(run(role, rx));

        drop(tx);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn state_actor_handles_multiple_concurrent_requests() {
        let (tx, rx) = mpsc::channel(64);
        let role = Box::new(TestRole::new());

        let handle = tokio::spawn(run(role, rx));

        let mut reply_rxs = vec![];
        for _ in 0..10 {
            let (reply_tx, reply_rx) = oneshot::channel();
            tx.send(StateMsg::Request {
                req: WorkerRequest::Sync {
                    server_id: "srv".to_string(),
                    state_hash: StateHash::ZERO,
                },
                reply: reply_tx,
            })
            .await
            .unwrap();
            reply_rxs.push(reply_rx);
        }

        for rx in reply_rxs {
            match rx.await.unwrap() {
                WorkerResponse::SyncBatch { .. } => {}
                _ => panic!("expected SyncBatch"),
            }
        }

        tx.send(StateMsg::Shutdown).await.unwrap();
        handle.await.unwrap();
    }
}
