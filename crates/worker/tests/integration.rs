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
            pending_count: 0,
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
    let hb_identity = Identity::generate();
    let _hb = system.spawn(HeartbeatActor::new(
        test_worker_id,
        Duration::from_millis(50),
        state_addr.clone(),
        sender_a,
        hb_identity,
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

    let (wire, _) = willow_common::unpack_wire(&data).expect("signed announcement");
    match wire {
        willow_common::WireMessage::Worker(willow_common::WorkerWireMessage::Announcement(a)) => {
            assert_eq!(a.peer_id, test_worker_id);
            match a.role {
                WorkerRoleInfo::Replay {
                    events_buffered, ..
                } => assert_eq!(events_buffered, 0),
                _ => panic!("expected Replay"),
            }
        }
        _ => panic!("expected Worker(Announcement)"),
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
    let dep_identity = Identity::generate();
    let _hb = system.spawn(HeartbeatActor::new(
        departing_id,
        Duration::from_secs(60), // Long interval — won't fire naturally
        state_addr,
        sender_a,
        dep_identity,
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

    let (wire, _) = willow_common::unpack_wire(&departure_data).expect("signed departure");
    match wire {
        willow_common::WireMessage::Worker(willow_common::WorkerWireMessage::Departure {
            peer_id,
        }) => {
            assert_eq!(peer_id, departing_id);
        }
        _ => panic!("expected Worker(Departure)"),
    }
}

/// Tests the heartbeat actor wiring pattern used by runtime::run() —
/// state + heartbeat actors coordinating via channels.
///
/// Note: the `SyncActor` is spawned here but does **not** broadcast because
/// `TestReplayRole::heads_summaries()` returns empty. This test only verifies
/// that the heartbeat fires, that multiple actors can share the same
/// `state_addr` without deadlocking, and that `StateActor` remains responsive
/// after the 200 ms window. For sync-broadcast coverage see
/// `sync_actor_broadcasts_request_when_heads_nonempty`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heartbeat_actor_orchestration_without_network() {
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
    let orch_identity = Identity::generate();
    let _hb = system.spawn(HeartbeatActor::new(
        orch_id,
        Duration::from_millis(50),
        state_addr.clone(),
        sender_a.clone(),
        orch_identity.clone(),
    ));
    let _sync = system.spawn(SyncActor::new(
        orch_id,
        Duration::from_millis(80),
        state_addr.clone(),
        sender_a,
        orch_identity,
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
        if let Ok(Some(Ok(willow_network::GossipEvent::Received(msg)))) =
            tokio::time::timeout(Duration::from_millis(30), events_b.next()).await
        {
            if let Some((
                willow_common::WireMessage::Worker(willow_common::WorkerWireMessage::Announcement(
                    _,
                )),
                _,
            )) = willow_common::unpack_wire(&msg.content)
            {
                announcement_count += 1;
            }
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
        NetworkActor::new(
            workers_events,
            state_addr.clone(),
            peer_id,
            MockTopicHandle,
            worker_id.clone(),
        )
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
        &willow_common::WireMessage::Event(Box::new(event.clone())),
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

    let genesis_data = willow_common::pack_wire(
        &willow_common::WireMessage::Event(Box::new(genesis)),
        &sender_id,
    )
    .unwrap();
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

        let data = willow_common::pack_wire(
            &willow_common::WireMessage::Event(Box::new(event)),
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
            worker_id.clone(),
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
///
/// This test pre-buffers the event in the channel BEFORE spawning the
/// NetworkActor — confirming that the drain loop starts without waiting
/// for any ready signal when none is provided.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn network_actor_drains_immediately_without_ready_signal() {
    let system = System::new();

    let state_addr = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-1", 100)),
        ready: None,
    });

    let (_workers_tx, workers_rx) = tokio::sync::mpsc::channel::<GossipEvent>(16);
    let (ops_tx, ops_rx) = tokio::sync::mpsc::channel::<GossipEvent>(16);

    // Pre-buffer a CreateServer event BEFORE spawning the NetworkActor.
    // Without a ready signal the drain loop should process it immediately
    // after the actor starts — no 50ms sleep needed to "simulate arrival".
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
        &willow_common::WireMessage::Event(Box::new(event)),
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

    let worker_id = Identity::generate();
    let peer_id = worker_id.endpoint_id();

    // Spawn NetworkActor with no ready signal — drain must start immediately.
    let _network = system.spawn(
        NetworkActor::new(
            MockTopicEvents { rx: workers_rx },
            state_addr.clone(),
            peer_id,
            MockTopicHandle,
            worker_id.clone(),
        )
        .with_ops_events(MockTopicEvents { rx: ops_rx }),
    );

    // Allow the drain task to run and the event to be forwarded to StateActor.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let info = state_addr.ask(GetRoleInfoMsg).await.unwrap();
    match info {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => {
            assert_eq!(
                events_buffered, 1,
                "pre-buffered event should be processed without ready signal"
            );
        }
        _ => panic!("expected Replay"),
    }

    system.shutdown().await;
}

// ───── A role that overrides heads_summaries() ────────────────────────────

/// A WorkerRole that returns a non-empty [`HeadsSummary`] for one server,
/// so the SyncActor has something to broadcast.
struct TestHeadsRole {
    server_id: String,
    heads: willow_state::HeadsSummary,
}

impl TestHeadsRole {
    fn new(server_id: &str, heads: willow_state::HeadsSummary) -> Self {
        Self {
            server_id: server_id.to_string(),
            heads,
        }
    }
}

impl willow_common::WorkerRole for TestHeadsRole {
    fn role_info(&self) -> willow_common::WorkerRoleInfo {
        willow_common::WorkerRoleInfo::Replay {
            servers_loaded: 1,
            events_buffered: 1,
            max_events: 100,
            pending_count: 0,
        }
    }

    fn on_event(&mut self, _event: &willow_state::Event) {}

    fn handle_request(
        &mut self,
        _req: willow_common::WorkerRequest,
    ) -> willow_common::WorkerResponse {
        willow_common::WorkerResponse::Denied {
            reason: "test".to_string(),
        }
    }

    fn heads_summaries(&self) -> Vec<(String, willow_state::HeadsSummary)> {
        vec![(self.server_id.clone(), self.heads.clone())]
    }
}

/// Verify that SyncActor broadcasts a `WorkerWireMessage::Request` containing
/// a `WorkerRequest::Sync` when the role reports non-empty heads.
///
/// This tests the SyncActor's core behavior: query heads → build wire message →
/// call `topic.broadcast()`. Previously only shutdown was tested.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_actor_broadcasts_request_when_heads_nonempty() {
    use std::collections::BTreeMap;
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::{Network, TopicEvents};
    use willow_state::{AuthorHead, EventHash};
    use willow_worker::actors::sync::SyncActor;

    // Build a non-empty HeadsSummary with one author.
    let author_id = Identity::generate();
    let mut heads_map = BTreeMap::new();
    heads_map.insert(
        author_id.endpoint_id(),
        AuthorHead {
            seq: 3,
            hash: EventHash::from_bytes(b"fake-hash-for-test"),
        },
    );
    let heads = willow_state::HeadsSummary { heads: heads_map };

    let hub = MemHub::new();
    let net_a = MemNetwork::new(&hub);
    let net_b = MemNetwork::new(&hub);

    let topic_id = willow_network::topic_id(willow_common::WORKERS_TOPIC);
    // net_a is the SyncActor's network; net_b is the observer.
    let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
    let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

    let system = System::new();

    // StateActor backed by a role that returns non-empty heads_summaries.
    let state_addr = system.spawn(StateActor {
        role: Box::new(TestHeadsRole::new("srv-sync", heads)),
        ready: None,
    });

    let sync_identity = Identity::generate();
    let sync_peer_id = net_a.id();

    // Very short interval so the sync fires quickly in the test.
    let _sync = system.spawn(SyncActor::new(
        sync_peer_id,
        Duration::from_millis(30),
        state_addr,
        sender_a,
        sync_identity,
    ));

    // Drain events from net_b until we see a Request message or time out.
    let found_request = loop {
        let event = match tokio::time::timeout(Duration::from_secs(2), events_b.next()).await {
            Ok(Some(Ok(e))) => e,
            _ => break false,
        };
        if let willow_network::GossipEvent::Received(msg) = event {
            if let Some((
                willow_common::WireMessage::Worker(willow_common::WorkerWireMessage::Request {
                    payload: willow_common::WorkerRequest::Sync { server_id, heads },
                    ..
                }),
                _,
            )) = willow_common::unpack_wire(&msg.content)
            {
                assert_eq!(server_id, "srv-sync");
                assert_eq!(heads.heads.len(), 1, "heads should have the one author");
                break true;
            }
        }
    };

    assert!(
        found_request,
        "SyncActor should have broadcast a WorkerRequest::Sync message"
    );

    system.shutdown().await;
}

/// Verify the request→response cycle at the StateActor level:
/// inject a `WorkerRequest::Sync` into a StateActor that already holds
/// events and assert the response contains those events.
///
/// This covers the core worker convergence path without full gossip wiring.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_request_response_returns_known_events() {
    let system = System::new();

    // Build a TestReplayRole pre-loaded with genesis + two messages.
    let mut role = TestReplayRole::new("srv-conv", 100);
    let author = Identity::generate();

    let genesis = Event::new(
        &author,
        1,
        EventHash::ZERO,
        vec![],
        EventKind::CreateServer {
            name: "srv-conv".to_string(),
        },
        0,
    );
    role.on_event(&genesis);

    let msg1 = make_message(&author, 2, genesis.hash);
    role.on_event(&msg1);

    let msg2 = make_message(&author, 3, msg1.hash);
    role.on_event(&msg2);

    let state_addr = system.spawn(StateActor {
        role: Box::new(role),
        ready: None,
    });

    // A peer that has no events sends Sync with empty heads.
    let resp = state_addr
        .ask(WorkerRequestMsg(willow_common::WorkerRequest::Sync {
            server_id: "srv-conv".to_string(),
            heads: willow_state::HeadsSummary::default(),
        }))
        .await
        .unwrap();

    match resp {
        willow_common::WorkerResponse::SyncBatch { events } => {
            assert_eq!(
                events.len(),
                3,
                "SyncBatch should contain all three events (genesis + 2 messages)"
            );
            // Events should include genesis and both messages.
            let hashes: std::collections::HashSet<_> = events.iter().map(|e| e.hash).collect();
            assert!(hashes.contains(&genesis.hash), "genesis missing from batch");
            assert!(hashes.contains(&msg1.hash), "msg1 missing from batch");
            assert!(hashes.contains(&msg2.hash), "msg2 missing from batch");
        }
        other => panic!("expected SyncBatch, got {:?}", other),
    }

    system.shutdown().await;
}

/// Two-worker state convergence: verify that events held by Worker A
/// are delivered to Worker B via the sync request/response cycle routed
/// through real gossip (MemNetwork).
///
/// Flow:
///   1. Worker A's StateActor holds [genesis, msg1].
///   2. Worker B has no events; its NetworkActor receives a sync request
///      from the WORKERS topic (injected directly), forwards it to A's
///      StateActor, and A broadcasts the SyncBatch response.
///   3. Worker B's NetworkActor receives the Response and its StateActor
///      processes the events via `parse_server_message` fallback
///      (SyncBatch on the ops channel).
///
/// For simplicity we test the critical sub-path: inject a Sync request
/// into Worker A via gossip and verify a Response containing the events
/// is broadcast back on the topic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_workers_sync_state_via_gossip() {
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::{Network, TopicEvents};

    let hub = MemHub::new();
    let net_a = MemNetwork::new(&hub); // Worker A (has events)
    let net_b = MemNetwork::new(&hub); // Worker B (observer / requester)

    let topic_id = willow_network::topic_id(willow_common::WORKERS_TOPIC);

    // Worker A subscribes for reading requests and broadcasting responses.
    let (sender_a, events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
    // Worker B subscribes to observe responses.
    let (sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

    let system = System::new();

    // Worker A's StateActor has genesis + msg1.
    let mut role_a = TestReplayRole::new("srv-ab", 100);
    let author = Identity::generate();
    let genesis = Event::new(
        &author,
        1,
        EventHash::ZERO,
        vec![],
        EventKind::CreateServer {
            name: "srv-ab".to_string(),
        },
        0,
    );
    role_a.on_event(&genesis);
    let msg1 = make_message(&author, 2, genesis.hash);
    role_a.on_event(&msg1);

    let state_a = system.spawn(StateActor {
        role: Box::new(role_a),
        ready: None,
    });

    let worker_a_identity = Identity::generate();
    let worker_a_id = net_a.id();

    // Wire up Worker A's NetworkActor to process incoming requests.
    let _network_a = system.spawn(NetworkActor::new(
        events_a,
        state_a.clone(),
        worker_a_id,
        sender_a,
        worker_a_identity.clone(),
    ));

    // Allow actors to start.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Worker B sends a Sync request with empty heads — simulates a peer
    // that has no events yet.
    let requester_b_id = net_b.id();
    let request_id = "req-b-to-a".to_string();
    let sync_req = willow_common::WorkerWireMessage::Request {
        request_id: request_id.clone(),
        // Sync requests are accepted regardless of target_peer (broadcast protocol).
        target_peer: requester_b_id,
        payload: willow_common::WorkerRequest::Sync {
            server_id: "srv-ab".to_string(),
            heads: willow_state::HeadsSummary::default(),
        },
    };
    let b_identity = Identity::generate();
    let data = willow_common::pack_wire(&willow_common::WireMessage::Worker(sync_req), &b_identity)
        .unwrap();
    sender_b.broadcast(bytes::Bytes::from(data)).await.unwrap();

    // Drain events_b until we see a WorkerWireMessage::Response addressed
    // to requester_b_id containing a SyncBatch with the expected events.
    let mut found_response = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        let event = match tokio::time::timeout(Duration::from_millis(100), events_b.next()).await {
            Ok(Some(Ok(e))) => e,
            _ => break,
        };

        if let willow_network::GossipEvent::Received(msg) = event {
            if let Some((
                willow_common::WireMessage::Worker(willow_common::WorkerWireMessage::Response {
                    request_id: rid,
                    target_peer,
                    payload,
                }),
                _,
            )) = willow_common::unpack_wire(&msg.content)
            {
                if rid == request_id && target_peer == requester_b_id {
                    match *payload {
                        willow_common::WorkerResponse::SyncBatch { events } => {
                            assert_eq!(events.len(), 2, "Worker A should send genesis + msg1");
                            let hashes: std::collections::HashSet<_> =
                                events.iter().map(|e| e.hash).collect();
                            assert!(hashes.contains(&genesis.hash));
                            assert!(hashes.contains(&msg1.hash));
                            found_response = true;
                            break;
                        }
                        other => {
                            panic!("expected SyncBatch in response payload, got {:?}", other)
                        }
                    }
                }
            }
        }
    }

    assert!(
        found_response,
        "Worker B should have received a SyncBatch response from Worker A"
    );

    // ── Phase 2: verify Worker B's StateActor processes the SyncBatch ──────
    //
    // The `parse_server_message` path in NetworkActor converts a received
    // `WorkerWireMessage::Response` into events for the state actor (via the
    // DeserializeError fallback). We exercise that path manually here by:
    //   1. Re-serialising the two events as `WireMessage::SyncBatch`.
    //   2. Passing the bytes through `parse_server_message`.
    //   3. Feeding the resulting events into a fresh `state_b` actor.
    //   4. Asserting that `state_b` reports `events_buffered == 2`.

    use willow_worker::actors::network::parse_server_message;
    use willow_worker::actors::network::ServerMessageAction;

    // Build the same SyncBatch payload that Worker A would have sent.
    let batch_bytes = willow_common::pack_wire(
        &willow_common::WireMessage::SyncBatch {
            events: vec![genesis.clone(), msg1.clone()],
        },
        &worker_a_identity,
    )
    .unwrap();

    let state_b = system.spawn(StateActor {
        role: Box::new(TestReplayRole::new("srv-ab", 100)),
        ready: None,
    });

    match parse_server_message(&batch_bytes) {
        ServerMessageAction::Events(events) => {
            assert_eq!(
                events.len(),
                2,
                "parse_server_message must extract both events"
            );
            for event in events {
                state_b.do_send(EventMsg(event)).unwrap();
            }
        }
        ServerMessageAction::Ignore => panic!("parse_server_message should have returned Events"),
    }

    // Allow the EventMsg deliveries to be processed.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let info_b = state_b.ask(GetRoleInfoMsg).await.unwrap();
    match info_b {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => {
            assert_eq!(
                events_buffered, 2,
                "Worker B's StateActor should have buffered genesis + msg1"
            );
        }
        _ => panic!("expected Replay"),
    }

    system.shutdown().await;
}
