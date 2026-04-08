//! State actor — owns the [`WorkerRole`] and processes messages sequentially.
//!
//! All mutable state access goes through this actor. No locks needed
//! because only this task touches the role.

use tracing::debug;
use willow_actor::{Actor, Context, Handler};

use super::{
    EventMsg, GetHeadsSummariesMsg, GetRoleInfoMsg, ServerDiscoveredMsg, WorkerRequestMsg,
};
use crate::WorkerRole;

/// The state actor holds the worker's mutable role and processes messages sequentially.
pub struct StateActor {
    pub role: Box<dyn WorkerRole>,
    /// Optional ready signal — set to `true` when `started()` completes so
    /// other actors (e.g. `NetworkActor`) can wait before draining events.
    /// Uses `watch` channel so late subscribers see the value immediately.
    pub ready: Option<tokio::sync::watch::Sender<bool>>,
}

impl Actor for StateActor {
    fn started(
        &mut self,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        debug!("state actor started");
        if let Some(ready) = self.ready.take() {
            let _ = ready.send(true);
        }
        async {}
    }

    fn stopped(&mut self) -> impl std::future::Future<Output = ()> + Send {
        debug!("state actor stopped");
        async {}
    }
}

impl Handler<EventMsg> for StateActor {
    fn handle(
        &mut self,
        msg: EventMsg,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        self.role.on_event(&msg.0);
        async {}
    }
}

impl Handler<WorkerRequestMsg> for StateActor {
    fn handle(
        &mut self,
        msg: WorkerRequestMsg,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = crate::types::WorkerResponse> + Send {
        let response = self.role.handle_request(msg.0);
        async move { response }
    }
}

impl Handler<GetRoleInfoMsg> for StateActor {
    fn handle(
        &mut self,
        _msg: GetRoleInfoMsg,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = crate::types::WorkerRoleInfo> + Send {
        let info = self.role.role_info();
        async move { info }
    }
}

impl Handler<GetHeadsSummariesMsg> for StateActor {
    async fn handle(
        &mut self,
        _msg: GetHeadsSummariesMsg,
        _ctx: &mut Context<Self>,
    ) -> Vec<(String, willow_state::HeadsSummary)> {
        self.role.heads_summaries()
    }
}

impl Handler<ServerDiscoveredMsg> for StateActor {
    fn handle(
        &mut self,
        msg: ServerDiscoveredMsg,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        debug!(server_id = %msg.server_id, "server discovered by state actor");
        async {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WorkerRequest, WorkerResponse, WorkerRoleInfo};
    use willow_actor::System;
    use willow_state::{Event, EventHash, EventKind, HeadsSummary};

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
        let id = willow_identity::Identity::generate();
        Event::new(
            &id,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello".to_string(),
                reply_to: None,
            },
            1000,
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_actor_processes_events() {
        let system = System::new();
        let addr = system.spawn(StateActor {
            role: Box::new(TestRole::new()),
            ready: None,
        });

        for _ in 0..3 {
            addr.do_send(EventMsg(make_test_event())).unwrap();
        }

        let info = addr.ask(GetRoleInfoMsg).await.unwrap();
        match info {
            WorkerRoleInfo::Replay {
                events_buffered, ..
            } => assert_eq!(events_buffered, 3),
            _ => panic!("expected Replay"),
        }

        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_actor_handles_requests() {
        let system = System::new();
        let addr = system.spawn(StateActor {
            role: Box::new(TestRole::new()),
            ready: None,
        });

        // Sync request.
        let resp = addr
            .ask(WorkerRequestMsg(WorkerRequest::Sync {
                server_id: "srv".to_string(),
                heads: HeadsSummary::default(),
            }))
            .await
            .unwrap();
        match resp {
            WorkerResponse::SyncBatch { events } => assert!(events.is_empty()),
            _ => panic!("expected SyncBatch"),
        }

        // History request (denied by replay role).
        let resp = addr
            .ask(WorkerRequestMsg(WorkerRequest::History {
                server_id: "srv".to_string(),
                channel: Some("general".to_string()),
                before: None,
                limit: 50,
            }))
            .await
            .unwrap();
        match resp {
            WorkerResponse::Denied { reason } => assert!(reason.contains("not a storage")),
            _ => panic!("expected Denied"),
        }

        system.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_actor_exits_on_shutdown() {
        let system = System::new();
        let addr = system.spawn(StateActor {
            role: Box::new(TestRole::new()),
            ready: None,
        });

        assert!(addr.is_alive());
        system.shutdown().await;
        for _ in 0..20 {
            if !addr.is_alive() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert!(!addr.is_alive());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_actor_handles_multiple_concurrent_requests() {
        let system = System::new();
        let addr = system.spawn(StateActor {
            role: Box::new(TestRole::new()),
            ready: None,
        });

        let mut futs = vec![];
        for _ in 0..10 {
            let f = addr.ask(WorkerRequestMsg(WorkerRequest::Sync {
                server_id: "srv".to_string(),
                heads: HeadsSummary::default(),
            }));
            futs.push(f);
        }

        for f in futs {
            match f.await.unwrap() {
                WorkerResponse::SyncBatch { .. } => {}
                _ => panic!("expected SyncBatch"),
            }
        }

        system.shutdown().await;
    }
}
