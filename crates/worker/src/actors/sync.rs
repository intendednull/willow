//! Sync actor — periodically broadcasts SyncRequests for state convergence.

use std::time::Duration;

use tracing::debug;
use willow_actor::{Actor, Addr, Context, Handler, IntervalHandle, Message};
use willow_network::TopicHandle;

use super::state::StateActor;
use super::GetHeadsSummariesMsg;
use crate::types::{WorkerRequest, WorkerWireMessage};

/// Sync actor that periodically queries state hashes and broadcasts sync requests.
pub struct SyncActor<T: TopicHandle + 'static> {
    peer_id: willow_identity::EndpointId,
    interval: Duration,
    state_addr: Addr<StateActor>,
    topic: T,
    _interval_handle: Option<IntervalHandle>,
}

impl<T: TopicHandle + 'static> SyncActor<T> {
    pub fn new(
        peer_id: willow_identity::EndpointId,
        interval: Duration,
        state_addr: Addr<StateActor>,
        topic: T,
    ) -> Self {
        Self {
            peer_id,
            interval,
            state_addr,
            topic,
            _interval_handle: None,
        }
    }
}

struct SyncTick;
impl Message for SyncTick {
    type Result = ();
}

impl<T: TopicHandle + 'static> Actor for SyncActor<T> {
    fn started(&mut self, ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        debug!("sync actor started (interval: {:?})", self.interval);
        self._interval_handle = Some(ctx.run_interval(self.interval, || SyncTick));
        async {}
    }
}

impl<T: TopicHandle + 'static> Handler<SyncTick> for SyncActor<T> {
    fn handle(
        &mut self,
        _msg: SyncTick,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        let state_addr = self.state_addr.clone();
        let peer_id = self.peer_id;
        let topic = self.topic.clone();

        async move {
            let summaries = match state_addr.ask(GetHeadsSummariesMsg).await {
                Ok(s) => s,
                Err(_) => return,
            };

            for (server_id, heads) in summaries {
                // target_peer is set to the sender's own ID to indicate a
                // broadcast request. The NetworkActor accepts these because
                // it checks target_peer == local_peer_id (the receiver also
                // has their own ID, which won't match) OR handles Sync
                // requests specially.
                let msg = WorkerWireMessage::Request {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    target_peer: peer_id,
                    payload: WorkerRequest::Sync { server_id, heads },
                };
                if let Ok(bytes) = bincode::serialize(&msg) {
                    let _ = topic.broadcast(bytes::Bytes::from(bytes)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WORKERS_TOPIC;
    use willow_actor::System;
    use willow_identity::Identity;
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::Network;

    use super::super::state::StateActor;

    /// A test role that provides known heads summaries.
    struct TestSyncRole;
    impl crate::WorkerRole for TestSyncRole {
        fn role_info(&self) -> crate::types::WorkerRoleInfo {
            crate::types::WorkerRoleInfo::Replay {
                servers_loaded: 0,
                events_buffered: 0,
                max_events: 0,
            }
        }
        fn on_event(&mut self, _event: &willow_state::Event) {}
        fn handle_request(
            &mut self,
            _req: crate::types::WorkerRequest,
        ) -> crate::types::WorkerResponse {
            crate::types::WorkerResponse::Denied {
                reason: "test".to_string(),
            }
        }
    }

    // Since we can't override the GetHeadsSummariesMsg handler on StateActor,
    // we test with a state actor that returns empty summaries. The sync actor
    // sends requests only for summaries returned by GetHeadsSummariesMsg.
    // StateActor returns vec![] by default, so no sync requests are broadcast.
    // We test that the actor starts and shuts down cleanly.

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_actor_exits_on_shutdown() {
        let hub = MemHub::new();
        let net = MemNetwork::new(&hub);

        let topic_id = willow_network::topic_id(WORKERS_TOPIC);
        let (sender, _events) = net.subscribe(topic_id, vec![]).await.unwrap();

        let system = System::new();
        let state_addr = system.spawn(StateActor {
            role: Box::new(TestSyncRole),
            ready: None,
        });

        let addr = system.spawn(SyncActor::new(
            Identity::generate().endpoint_id(),
            Duration::from_secs(60),
            state_addr,
            sender,
        ));

        assert!(addr.is_alive());
        system.shutdown().await;
        // The SyncActor owns a timer interval — shutdown may take a few
        // ticks to fully propagate through the runtime.
        for _ in 0..20 {
            if !addr.is_alive() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
        assert!(!addr.is_alive());
    }
}
