//! Integration tests for the worker node system.
//!
//! Tests the full actor flow: state actor with a real WorkerRole,
//! heartbeat interaction, and concurrent request handling.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use willow_common::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};
use willow_network::TopicEvents;
use willow_state::{Event, EventKind, ServerState, StateHash};
use willow_worker::actors::state;
use willow_worker::actors::StateMsg;

/// Full replay role that tracks a single server.
struct TestReplayRole {
    state: ServerState,
    events: Vec<Event>,
    max_events: usize,
}

impl TestReplayRole {
    fn new(server_id: &str, _owner: &str, max_events: usize) -> Self {
        let owner_id = willow_identity::Identity::generate().endpoint_id();
        Self {
            state: ServerState::new(server_id, server_id, owner_id),
            events: Vec::new(),
            max_events,
        }
    }
}

impl WorkerRole for TestReplayRole {
    fn role_info(&self) -> WorkerRoleInfo {
        WorkerRoleInfo::Replay {
            servers_loaded: 1,
            events_buffered: self.events.len() as u32,
            max_events: self.max_events as u32,
        }
    }

    fn on_event(&mut self, event: &Event) {
        willow_state::apply_lenient(&mut self.state, event);
        self.events.push(event.clone());
        while self.events.len() > self.max_events {
            self.events.remove(0);
        }
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::Sync { state_hash, .. } => {
                if state_hash == StateHash::ZERO {
                    WorkerResponse::SyncBatch {
                        events: self.events.clone(),
                    }
                } else {
                    WorkerResponse::Snapshot {
                        state: Box::new(self.state.clone()),
                    }
                }
            }
            WorkerRequest::History { .. } => WorkerResponse::Denied {
                reason: "not a storage node".to_string(),
            },
        }
    }
}

fn make_message(id: &str, ts: u64) -> Event {
    Event {
        id: id.to_string(),
        parent_hash: StateHash::ZERO,
        author: willow_identity::Identity::generate().endpoint_id(),
        timestamp_ms: ts,
        kind: EventKind::Message {
            channel_id: "general".to_string(),
            body: format!("message {id}"),
            reply_to: None,
        },
    }
}

#[tokio::test]
async fn state_actor_with_replay_role_full_flow() {
    let (tx, rx) = mpsc::channel(64);
    let role = Box::new(TestReplayRole::new("srv-1", "owner", 100));

    let handle = tokio::spawn(state::run(role, rx));

    // 1. Ingest 5 events.
    for i in 0..5u64 {
        tx.send(StateMsg::Event(make_message(
            &format!("e{i}"),
            (i + 1) * 1000,
        )))
        .await
        .unwrap();
    }

    // 2. Verify role info shows 5 buffered events.
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(StateMsg::GetRoleInfo { reply: reply_tx })
        .await
        .unwrap();
    match reply_rx.await.unwrap() {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => assert_eq!(events_buffered, 5),
        _ => panic!("expected Replay"),
    }

    // 3. Sync request with ZERO hash — should return all events.
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(StateMsg::Request {
        req: WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        },
        reply: reply_tx,
    })
    .await
    .unwrap();
    match reply_rx.await.unwrap() {
        WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 5),
        _ => panic!("expected SyncBatch"),
    }

    // 4. History request — should be denied.
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(StateMsg::Request {
        req: WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "general".to_string(),
            before_timestamp: None,
            limit: 10,
        },
        reply: reply_tx,
    })
    .await
    .unwrap();
    match reply_rx.await.unwrap() {
        WorkerResponse::Denied { .. } => {}
        _ => panic!("expected Denied"),
    }

    tx.send(StateMsg::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn heartbeat_and_state_actor_interaction() {
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::Network;
    use willow_worker::actors::heartbeat;

    let hub = MemHub::new();
    let net_a = MemNetwork::new(&hub);
    let net_b = MemNetwork::new(&hub);

    let topic_id = willow_network::topic_id(willow_common::WORKERS_TOPIC);
    let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
    let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

    let (state_tx, state_rx) = mpsc::channel(64);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let role = Box::new(TestReplayRole::new("srv-1", "owner", 100));
    let state_handle = tokio::spawn(state::run(role, state_rx));

    let test_worker_id = net_a.id();
    let hb_handle = tokio::spawn(heartbeat::run(
        test_worker_id,
        Duration::from_millis(50),
        state_tx.clone(),
        sender_a,
        shutdown_rx,
    ));

    // Wait for a heartbeat — drain neighbor events first.
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

    let decoded: willow_common::WorkerWireMessage = bincode::deserialize(&data).unwrap();
    match decoded {
        willow_common::WorkerWireMessage::Announcement(a) => {
            assert_eq!(a.peer_id, test_worker_id);
            match a.role {
                WorkerRoleInfo::Replay {
                    events_buffered, ..
                } => assert_eq!(events_buffered, 0),
                _ => panic!("expected Replay"),
            }
        }
        _ => panic!("expected Announcement"),
    }

    shutdown_tx.send(true).unwrap();
    let _ = state_tx.send(StateMsg::Shutdown).await;
    let _ = tokio::join!(state_handle, hb_handle);
}

#[tokio::test]
async fn concurrent_requests_all_resolve() {
    let (tx, rx) = mpsc::channel(256);
    let role = Box::new(TestReplayRole::new("srv-1", "owner", 100));

    let handle = tokio::spawn(state::run(role, rx));

    // Fire 50 concurrent requests.
    let mut reply_rxs = vec![];
    for _ in 0..50 {
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(StateMsg::Request {
            req: WorkerRequest::Sync {
                server_id: "srv-1".to_string(),
                state_hash: StateHash::ZERO,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();
        reply_rxs.push(reply_rx);
    }

    // All 50 should resolve.
    for rx in reply_rxs {
        let resp = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(resp, WorkerResponse::SyncBatch { .. }));
    }

    tx.send(StateMsg::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn events_applied_then_queried_via_request() {
    let (tx, rx) = mpsc::channel(64);
    let role = Box::new(TestReplayRole::new("srv-1", "owner", 5));

    let handle = tokio::spawn(state::run(role, rx));

    // Ingest 10 events into a buffer of size 5.
    for i in 0..10u64 {
        tx.send(StateMsg::Event(make_message(
            &format!("e{i}"),
            (i + 1) * 1000,
        )))
        .await
        .unwrap();
    }

    // Query — should only get 5 (buffer evicted oldest).
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(StateMsg::Request {
        req: WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        },
        reply: reply_tx,
    })
    .await
    .unwrap();
    match reply_rx.await.unwrap() {
        WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 5),
        _ => panic!("expected SyncBatch"),
    }

    tx.send(StateMsg::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn graceful_shutdown_sends_departure() {
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::Network;
    use willow_worker::actors::heartbeat;

    let hub = MemHub::new();
    let net_a = MemNetwork::new(&hub);
    let net_b = MemNetwork::new(&hub);

    let topic_id = willow_network::topic_id(willow_common::WORKERS_TOPIC);
    let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
    let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

    let (state_tx, state_rx) = mpsc::channel(64);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let role = Box::new(TestReplayRole::new("srv-1", "owner", 100));
    let state_handle = tokio::spawn(state::run(role, state_rx));

    let departing_id = net_a.id();
    let hb_handle = tokio::spawn(heartbeat::run(
        departing_id,
        Duration::from_secs(60), // Long interval — won't fire naturally
        state_tx.clone(),
        sender_a,
        shutdown_rx,
    ));

    // Immediately signal shutdown.
    shutdown_tx.send(true).unwrap();
    hb_handle.await.unwrap();

    // Should have a departure message — drain neighbor events.
    let departure_data = loop {
        match tokio::time::timeout(Duration::from_millis(500), events_b.next()).await {
            Ok(Some(Ok(willow_network::GossipEvent::Received(msg)))) => {
                break msg.content;
            }
            Ok(Some(Ok(_))) => continue,
            _ => panic!("expected departure message"),
        }
    };

    let decoded: willow_common::WorkerWireMessage = bincode::deserialize(&departure_data).unwrap();
    match decoded {
        willow_common::WorkerWireMessage::Departure { peer_id } => {
            assert_eq!(peer_id, departing_id);
        }
        _ => panic!("expected Departure"),
    }

    let _ = state_tx.send(StateMsg::Shutdown).await;
    state_handle.await.unwrap();
}

/// Tests the full actor wiring pattern used by runtime::run() —
/// state + heartbeat + sync actors coordinating via channels.
/// Validates that the orchestration pattern works without needing
/// a real network node.
#[tokio::test]
async fn full_actor_orchestration_without_network() {
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::Network;
    use willow_worker::actors::{heartbeat, sync};

    let hub = MemHub::new();
    let net_a = MemNetwork::new(&hub);
    let net_b = MemNetwork::new(&hub);

    let topic_id = willow_network::topic_id(willow_common::WORKERS_TOPIC);
    let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
    let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

    let (state_tx, state_rx) = mpsc::channel(256);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let role = Box::new(TestReplayRole::new("srv-1", "owner", 100));

    // Spawn all three non-network actors.
    let state_handle = tokio::spawn(state::run(role, state_rx));
    let orch_id = net_a.id();
    let hb_handle = tokio::spawn(heartbeat::run(
        orch_id,
        Duration::from_millis(50),
        state_tx.clone(),
        sender_a.clone(),
        shutdown_rx.clone(),
    ));
    let sync_handle = tokio::spawn(sync::run(
        orch_id,
        Duration::from_millis(80),
        state_tx.clone(),
        sender_a,
        shutdown_rx,
    ));

    // Ingest some events.
    for i in 0..3u64 {
        state_tx
            .send(StateMsg::Event(make_message(
                &format!("orch-{i}"),
                (i + 1) * 1000,
            )))
            .await
            .unwrap();
    }

    // Collect messages from heartbeat and sync for a bit.
    let mut announcement_count = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(30), events_b.next()).await {
            Ok(Some(Ok(willow_network::GossipEvent::Received(msg)))) => {
                if let Ok(decoded) =
                    bincode::deserialize::<willow_common::WorkerWireMessage>(&msg.content)
                {
                    match decoded {
                        willow_common::WorkerWireMessage::Announcement(_) => {
                            announcement_count += 1;
                        }
                        willow_common::WorkerWireMessage::Request { .. } => {
                            // Sync requests are expected too.
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    assert!(
        announcement_count >= 1,
        "expected at least 1 heartbeat, got {announcement_count}"
    );

    // Verify state actor still responds after all this.
    let (reply_tx, reply_rx) = oneshot::channel();
    state_tx
        .send(StateMsg::GetRoleInfo { reply: reply_tx })
        .await
        .unwrap();
    match reply_rx.await.unwrap() {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => assert_eq!(events_buffered, 3),
        _ => panic!("expected Replay"),
    }

    // Shutdown all actors.
    shutdown_tx.send(true).unwrap();
    let _ = state_tx.send(StateMsg::Shutdown).await;
    let _ = tokio::join!(state_handle, hb_handle, sync_handle);
}

/// Tests that the member list component's worker detection logic
/// (checking SyncProvider permission) correctly separates workers
/// from regular members at the data level.
#[test]
fn sync_provider_permission_identifies_workers() {
    use willow_identity::Identity;
    use willow_state::{EventKind, Permission, ServerState};

    let owner = Identity::generate().endpoint_id();
    let worker = Identity::generate().endpoint_id();
    let random = Identity::generate().endpoint_id();

    let mut state = ServerState::new("srv", "Test", owner);

    // Grant SyncProvider to a worker.
    let grant = willow_state::Event {
        id: "grant-1".to_string(),
        parent_hash: state.hash(),
        author: owner,
        timestamp_ms: 1000,
        kind: EventKind::GrantPermission {
            peer_id: worker,
            permission: Permission::SyncProvider,
        },
    };
    willow_state::apply_lenient(&mut state, &grant);

    // Worker should have SyncProvider.
    assert!(state.has_permission(&worker, &Permission::SyncProvider));
    // Owner has implicit all-permissions (root of trust).
    assert!(state.has_permission(&owner, &Permission::SyncProvider));
    // Random peer should not.
    assert!(!state.has_permission(&random, &Permission::SyncProvider));
    // The member list excludes owner from the infra section even though
    // they have the permission — that filtering is in the component, not state.
}
