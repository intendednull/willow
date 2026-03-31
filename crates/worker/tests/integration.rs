//! Integration tests for the worker node system.
//!
//! Tests the full actor flow: state actor with a real WorkerRole,
//! heartbeat interaction, and concurrent request handling.

use std::time::Duration;

use willow_actor::System;
use willow_common::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};
use willow_state::{Event, EventKind, ServerState, StateHash};
use willow_worker::actors::state::StateActor;
use willow_worker::actors::{EventMsg, GetRoleInfoMsg, WorkerRequestMsg};

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn state_actor_with_replay_role_full_flow() {
    let system = System::new();
    let addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", "owner", 100)),
    });

    // 1. Ingest 5 events.
    for i in 0..5u64 {
        addr.do_send(EventMsg(make_message(&format!("e{i}"), (i + 1) * 1000)))
            .unwrap();
    }

    // 2. Verify role info shows 5 buffered events.
    let info = addr.ask(GetRoleInfoMsg).await.unwrap();
    match info {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => assert_eq!(events_buffered, 5),
        _ => panic!("expected Replay"),
    }

    // 3. Sync request with ZERO hash — should return all events.
    let resp = addr
        .ask(WorkerRequestMsg(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        }))
        .await
        .unwrap();
    match resp {
        WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 5),
        _ => panic!("expected SyncBatch"),
    }

    // 4. History request — should be denied.
    let resp = addr
        .ask(WorkerRequestMsg(WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "general".to_string(),
            before_timestamp: None,
            limit: 10,
        }))
        .await
        .unwrap();
    match resp {
        WorkerResponse::Denied { .. } => {}
        _ => panic!("expected Denied"),
    }

    system.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_and_state_actor_interaction() {
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::{Network, TopicEvents};
    use willow_worker::actors::heartbeat::HeartbeatActor;

    let hub = MemHub::new();
    let net_a = MemNetwork::new(&hub);
    let net_b = MemNetwork::new(&hub);

    let topic_id = willow_network::topic_id(willow_common::WORKERS_TOPIC);
    let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
    let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

    let system = System::new();

    let state_addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", "owner", 100)),
    });

    let test_worker_id = net_a.id();
    let _hb = system.spawn(HeartbeatActor::new(
        test_worker_id,
        Duration::from_millis(50),
        state_addr.clone(),
        sender_a,
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

    system.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_requests_all_resolve() {
    let system = System::new();
    let addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", "owner", 100)),
    });

    // Fire 50 concurrent requests.
    let mut futs = vec![];
    for _ in 0..50 {
        let f = addr.ask(WorkerRequestMsg(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        }));
        futs.push(f);
    }

    // All 50 should resolve.
    for f in futs {
        let resp = tokio::time::timeout(Duration::from_secs(5), f)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(resp, WorkerResponse::SyncBatch { .. }));
    }

    system.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_applied_then_queried_via_request() {
    let system = System::new();
    let addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", "owner", 5)),
    });

    // Ingest 10 events into a buffer of size 5.
    for i in 0..10u64 {
        addr.do_send(EventMsg(make_message(&format!("e{i}"), (i + 1) * 1000)))
            .unwrap();
    }

    // Query — should only get 5 (buffer evicted oldest).
    let resp = addr
        .ask(WorkerRequestMsg(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        }))
        .await
        .unwrap();
    match resp {
        WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 5),
        _ => panic!("expected SyncBatch"),
    }

    system.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_sends_departure() {
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::{Network, TopicEvents};
    use willow_worker::actors::heartbeat::HeartbeatActor;

    let hub = MemHub::new();
    let net_a = MemNetwork::new(&hub);
    let net_b = MemNetwork::new(&hub);

    let topic_id = willow_network::topic_id(willow_common::WORKERS_TOPIC);
    let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
    let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

    let system = System::new();

    let state_addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", "owner", 100)),
    });

    let departing_id = net_a.id();
    let _hb = system.spawn(HeartbeatActor::new(
        departing_id,
        Duration::from_secs(60), // Long interval — won't fire naturally
        state_addr,
        sender_a,
    ));

    // Immediately shut down.
    system.shutdown().await;

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
}

/// Tests the full actor wiring pattern used by runtime::run() —
/// state + heartbeat + sync actors coordinating via channels.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_actor_orchestration_without_network() {
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::{Network, TopicEvents};
    use willow_worker::actors::heartbeat::HeartbeatActor;
    use willow_worker::actors::sync::SyncActor;

    let hub = MemHub::new();
    let net_a = MemNetwork::new(&hub);
    let net_b = MemNetwork::new(&hub);

    let topic_id = willow_network::topic_id(willow_common::WORKERS_TOPIC);
    let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
    let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

    let system = System::new();

    let state_addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", "owner", 100)),
    });

    let orch_id = net_a.id();
    let _hb = system.spawn(HeartbeatActor::new(
        orch_id,
        Duration::from_millis(50),
        state_addr.clone(),
        sender_a.clone(),
    ));
    let _sync = system.spawn(SyncActor::new(
        orch_id,
        Duration::from_millis(80),
        state_addr.clone(),
        sender_a,
    ));

    // Ingest some events.
    for i in 0..3u64 {
        state_addr
            .do_send(EventMsg(make_message(&format!("orch-{i}"), (i + 1) * 1000)))
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
    let info = state_addr.ask(GetRoleInfoMsg).await.unwrap();
    match info {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => assert_eq!(events_buffered, 3),
        _ => panic!("expected Replay"),
    }

    system.shutdown().await;
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

    assert!(state.has_permission(&worker, &Permission::SyncProvider));
    assert!(state.has_permission(&owner, &Permission::SyncProvider));
    assert!(!state.has_permission(&random, &Permission::SyncProvider));
}
