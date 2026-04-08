//! Integration tests for the worker node system.
//!
//! Tests the full actor flow: state actor with a real WorkerRole,
//! heartbeat interaction, and concurrent request handling.

use std::time::Duration;

use willow_actor::System;
use willow_common::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};
use willow_identity::Identity;
use willow_network::traits::{GossipEvent, GossipMessage, TopicEvents, TopicHandle};
use willow_state::{Event, EventHash, EventKind, HeadsSummary, Snapshot};
use willow_worker::actors::network::NetworkActor;
use willow_worker::actors::state::StateActor;
use willow_worker::actors::{EventMsg, GetRoleInfoMsg, WorkerRequestMsg};

/// Full replay role that tracks a single server using an EventDag.
struct TestReplayRole {
    dag: willow_state::EventDag,
    state: willow_state::ServerState,
    events: Vec<Event>,
    max_events: usize,
}

impl TestReplayRole {
    fn new(server_id: &str, max_events: usize) -> Self {
        let owner_id = Identity::generate().endpoint_id();
        Self {
            dag: willow_state::EventDag::new(),
            state: willow_state::ServerState::new(server_id, server_id, owner_id),
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
        if self.dag.insert(event.clone()).is_ok() {
            willow_state::apply_incremental(&mut self.state, event);
            self.events.push(event.clone());
            while self.events.len() > self.max_events {
                self.events.remove(0);
            }
        }
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::Sync { heads, .. } => {
                if heads.heads.is_empty() {
                    WorkerResponse::SyncBatch {
                        events: self.events.clone(),
                    }
                } else {
                    let snapshot = Snapshot::new(self.state.clone(), self.dag.heads_summary());
                    WorkerResponse::Snapshot {
                        snapshot: Box::new(snapshot),
                        post_snapshot_events: vec![],
                    }
                }
            }
            WorkerRequest::History { .. } => WorkerResponse::Denied {
                reason: "not a storage node".to_string(),
            },
        }
    }
}

fn make_message(identity: &Identity, seq: u64, prev: EventHash) -> Event {
    Event::new(
        identity,
        seq,
        prev,
        vec![],
        EventKind::Message {
            channel_id: "general".to_string(),
            body: format!("message seq={seq}"),
            reply_to: None,
        },
        seq * 1000,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn state_actor_with_replay_role_full_flow() {
    let system = System::new();
    let addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", 100)),
        ready: None,
    });

    // 1. Ingest 5 events.
    let id = Identity::generate();
    let genesis = Event::new(
        &id,
        1,
        EventHash::ZERO,
        vec![],
        EventKind::CreateServer {
            name: "srv-1".to_string(),
        },
        0,
    );
    addr.do_send(EventMsg(genesis.clone())).unwrap();

    let mut prev = genesis.hash;
    for seq in 2..=5 {
        let e = make_message(&id, seq, prev);
        prev = e.hash;
        addr.do_send(EventMsg(e)).unwrap();
    }

    // 2. Verify role info shows 5 buffered events.
    let info = addr.ask(GetRoleInfoMsg).await.unwrap();
    match info {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => assert_eq!(events_buffered, 5),
        _ => panic!("expected Replay"),
    }

    // 3. Sync request with empty heads — should return all events.
    let resp = addr
        .ask(WorkerRequestMsg(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
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
            channel: Some("general".to_string()),
            before: None,
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
        role: Box::new(TestReplayRole::new("srv-1", 100)),
        ready: None,
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
        role: Box::new(TestReplayRole::new("srv-1", 100)),
        ready: None,
    });

    // Fire 50 concurrent requests.
    let mut futs = vec![];
    for _ in 0..50 {
        let f = addr.ask(WorkerRequestMsg(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
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
        role: Box::new(TestReplayRole::new("srv-1", 5)),
        ready: None,
    });

    // Ingest 10 events into a buffer of size 5.
    let id = Identity::generate();
    let genesis = Event::new(
        &id,
        1,
        EventHash::ZERO,
        vec![],
        EventKind::CreateServer {
            name: "srv-1".to_string(),
        },
        0,
    );
    addr.do_send(EventMsg(genesis.clone())).unwrap();

    let mut prev = genesis.hash;
    for seq in 2..=10 {
        let e = make_message(&id, seq, prev);
        prev = e.hash;
        addr.do_send(EventMsg(e)).unwrap();
    }

    // Query — should only get 5 (buffer evicted oldest).
    let resp = addr
        .ask(WorkerRequestMsg(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
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
        role: Box::new(TestReplayRole::new("srv-1", 100)),
        ready: None,
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
        role: Box::new(TestReplayRole::new("srv-1", 100)),
        ready: None,
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
    let id = Identity::generate();
    let genesis = Event::new(
        &id,
        1,
        EventHash::ZERO,
        vec![],
        EventKind::CreateServer {
            name: "orch".to_string(),
        },
        0,
    );
    state_addr.do_send(EventMsg(genesis.clone())).unwrap();

    let mut prev = genesis.hash;
    for seq in 2..=3 {
        let e = make_message(&id, seq, prev);
        prev = e.hash;
        state_addr.do_send(EventMsg(e)).unwrap();
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
    use willow_state::{Permission, ServerState};

    let owner_identity = Identity::generate();
    let owner = owner_identity.endpoint_id();
    let worker = Identity::generate().endpoint_id();
    let random = Identity::generate().endpoint_id();

    let mut state = ServerState::new("srv", "Test", owner);

    // Grant SyncProvider to a worker — must be signed by the owner (admin).
    let grant = Event::new(
        &owner_identity,
        1,
        EventHash::ZERO,
        vec![],
        EventKind::GrantPermission {
            peer_id: worker,
            permission: Permission::SyncProvider,
        },
        1000,
    );
    willow_state::apply_incremental(&mut state, &grant);

    assert!(state.has_permission(&worker, &Permission::SyncProvider));
    assert!(state.has_permission(&owner, &Permission::SyncProvider));
    assert!(!state.has_permission(&random, &Permission::SyncProvider));
}

// ───── Mock types for NetworkActor tests ──────────────────────────────────

/// A minimal TopicEvents mock backed by a tokio mpsc channel.
struct MockTopicEvents {
    rx: tokio::sync::mpsc::Receiver<GossipEvent>,
}

#[async_trait::async_trait]
impl TopicEvents for MockTopicEvents {
    async fn next(&mut self) -> Option<anyhow::Result<GossipEvent>> {
        self.rx.recv().await.map(Ok)
    }
    async fn joined(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// A minimal TopicHandle mock that discards broadcasts.
#[derive(Clone)]
struct MockTopicHandle;

#[async_trait::async_trait]
impl TopicHandle for MockTopicHandle {
    async fn broadcast(&self, _data: bytes::Bytes) -> anyhow::Result<()> {
        Ok(())
    }
    async fn broadcast_neighbors(&self, _data: bytes::Bytes) -> anyhow::Result<()> {
        Ok(())
    }
    fn neighbors(&self) -> Vec<willow_identity::EndpointId> {
        vec![]
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_ops_events_forwarded_to_state() {
    let system = System::new();

    // State actor with a replay role that tracks ingested events.
    let state_addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", 100)),
        ready: None,
    });

    // Create channels for both WORKERS and SERVER_OPS streams.
    let (_workers_tx, workers_rx) = tokio::sync::mpsc::channel::<GossipEvent>(16);
    let (ops_tx, ops_rx) = tokio::sync::mpsc::channel::<GossipEvent>(16);

    let worker_id = Identity::generate();
    let peer_id = worker_id.endpoint_id();

    let workers_events = MockTopicEvents { rx: workers_rx };
    let ops_events = MockTopicEvents { rx: ops_rx };

    let _network = system.spawn(
        NetworkActor::new(workers_events, state_addr.clone(), peer_id, MockTopicHandle)
            .with_ops_events(ops_events),
    );

    // Allow the actor to start.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Create a signed server event and send it on the ops stream.
    let sender_id = Identity::generate();
    let event = Event::new(
        &sender_id,
        1,
        EventHash::ZERO,
        vec![],
        EventKind::CreateServer {
            name: "srv-1".to_string(),
        },
        0,
    );

    let data = willow_common::pack_wire(
        &willow_common::WireMessage::Event(event.clone()),
        &sender_id,
    )
    .unwrap();

    ops_tx
        .send(GossipEvent::Received(GossipMessage {
            content: bytes::Bytes::from(data),
            sender: sender_id.endpoint_id(),
        }))
        .await
        .unwrap();

    // Allow the event to be processed.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the state actor received the event.
    let info = state_addr.ask(GetRoleInfoMsg).await.unwrap();
    match info {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => {
            assert_eq!(
                events_buffered, 1,
                "server ops event should have been forwarded to state actor"
            );
        }
        _ => panic!("expected Replay"),
    }

    system.shutdown().await;
}

/// Issue #79: Verify that the ready signal prevents NetworkActor from draining
/// gossip events before StateActor has completed initialization.
///
/// Without the ready signal, pre-buffered events could arrive at StateActor
/// before its `started()` hook completes. The fix gates the drain tasks on
/// a `watch` channel that StateActor sets to `true` after `started()`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pre_buffered_events_wait_for_state_ready_signal() {
    let system = System::new();
    let (ready_tx, ready_rx) = tokio::sync::watch::channel(false);

    // State actor — we pass the ready sender so it fires on started().
    let state_addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", 100)),
        ready: Some(ready_tx),
    });

    // Pre-buffer 3 signed events in the ops channel BEFORE spawning NetworkActor.
    let (_workers_tx, workers_rx) = tokio::sync::mpsc::channel::<GossipEvent>(16);
    let (ops_tx, ops_rx) = tokio::sync::mpsc::channel::<GossipEvent>(16);

    let sender_id = Identity::generate();

    // EventDag requires CreateServer as genesis event (seq=1).
    let genesis = Event::new(
        &sender_id,
        1,
        EventHash::ZERO,
        vec![],
        EventKind::CreateServer {
            name: "srv-1".to_string(),
        },
        0,
    );
    let mut prev_hash = genesis.hash;

    let genesis_data =
        willow_common::pack_wire(&willow_common::WireMessage::Event(genesis), &sender_id).unwrap();
    ops_tx
        .send(GossipEvent::Received(GossipMessage {
            content: bytes::Bytes::from(genesis_data),
            sender: sender_id.endpoint_id(),
        }))
        .await
        .unwrap();

    // Buffer 2 more events (messages).
    for seq in 2..=3 {
        let event = Event::new(
            &sender_id,
            seq,
            prev_hash,
            vec![],
            EventKind::Message {
                channel_id: "general".to_string(),
                body: format!("msg-{seq}"),
                reply_to: None,
            },
            seq * 1000,
        );
        prev_hash = event.hash;

        let data = willow_common::pack_wire(&willow_common::WireMessage::Event(event), &sender_id)
            .unwrap();

        ops_tx
            .send(GossipEvent::Received(GossipMessage {
                content: bytes::Bytes::from(data),
                sender: sender_id.endpoint_id(),
            }))
            .await
            .unwrap();
    }

    let worker_id = Identity::generate();
    let peer_id = worker_id.endpoint_id();

    // Spawn NetworkActor with the ready signal — drain tasks should wait.
    let _network = system.spawn(
        NetworkActor::new(
            MockTopicEvents { rx: workers_rx },
            state_addr.clone(),
            peer_id,
            MockTopicHandle,
        )
        .with_ops_events(MockTopicEvents { rx: ops_rx })
        .with_ready_signal(ready_rx),
    );

    // StateActor's started() fires the watch channel, so the
    // drain tasks should begin and process all pre-buffered events.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify all 3 pre-buffered events were processed by StateActor.
    let info = state_addr.ask(GetRoleInfoMsg).await.unwrap();
    match info {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => {
            assert_eq!(
                events_buffered, 3,
                "all pre-buffered events should have been processed after ready signal"
            );
        }
        _ => panic!("expected Replay"),
    }

    system.shutdown().await;
}

/// Issue #79: Verify that without a ready signal, NetworkActor drains
/// immediately (backward compatibility for tests and simple setups).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn network_actor_drains_immediately_without_ready_signal() {
    let system = System::new();

    let state_addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", 100)),
        ready: None,
    });

    let (_workers_tx, workers_rx) = tokio::sync::mpsc::channel::<GossipEvent>(16);
    let (ops_tx, ops_rx) = tokio::sync::mpsc::channel::<GossipEvent>(16);

    let worker_id = Identity::generate();
    let peer_id = worker_id.endpoint_id();

    let _network = system.spawn(
        NetworkActor::new(
            MockTopicEvents { rx: workers_rx },
            state_addr.clone(),
            peer_id,
            MockTopicHandle,
        )
        .with_ops_events(MockTopicEvents { rx: ops_rx }),
        // No ready signal — drain starts immediately.
    );

    // Allow actors to start.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send a CreateServer event (required as genesis for EventDag).
    let sender_id = Identity::generate();
    let event = Event::new(
        &sender_id,
        1,
        EventHash::ZERO,
        vec![],
        EventKind::CreateServer {
            name: "srv-1".to_string(),
        },
        0,
    );
    let data =
        willow_common::pack_wire(&willow_common::WireMessage::Event(event), &sender_id).unwrap();
    ops_tx
        .send(GossipEvent::Received(GossipMessage {
            content: bytes::Bytes::from(data),
            sender: sender_id.endpoint_id(),
        }))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let info = state_addr.ask(GetRoleInfoMsg).await.unwrap();
    match info {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => {
            assert_eq!(
                events_buffered, 1,
                "event should be processed without ready signal"
            );
        }
        _ => panic!("expected Replay"),
    }

    system.shutdown().await;
}
