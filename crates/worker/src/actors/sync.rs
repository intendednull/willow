//! Sync actor — periodically broadcasts SyncRequests for state convergence.

use std::time::Duration;

use tracing::debug;
use willow_actor::{Actor, Addr, Context, Handler, IntervalHandle, Message};
use willow_network::TopicHandle;

use super::state::StateActor;
use super::GetStateHashesMsg;
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
            let hashes = match state_addr.ask(GetStateHashesMsg).await {
                Ok(h) => h,
                Err(_) => return,
            };

            for (server_id, state_hash) in hashes {
                let msg = WorkerWireMessage::Request {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    target_peer: peer_id,
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

    /// A test role that provides known state hashes.
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

    // Override GetStateHashes to return test data.
    // Since we can't override a handler on StateActor, we need to test
    // with a state actor that returns empty hashes. The sync actor sends
    // requests only for hashes returned by GetStateHashesMsg.
    // StateActor's GetStateHashesMsg handler returns vec![] by default,
    // so the sync actor won't broadcast any sync requests.
    // Let's test just that the actor starts and shuts down cleanly.

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn sync_actor_exits_on_shutdown() {
        let hub = MemHub::new();
        let net = MemNetwork::new(&hub);

        let topic_id = willow_network::topic_id(WORKERS_TOPIC);
        let (sender, _events) = net.subscribe(topic_id, vec![]).await.unwrap();

        let system = System::new();
        let state_addr = system.spawn(StateActor {
            role: Box::new(TestSyncRole),
        });

        let addr = system.spawn(SyncActor::new(
            Identity::generate().endpoint_id(),
            Duration::from_secs(60),
            state_addr,
            sender,
        ));

        assert!(addr.is_alive());
        system.shutdown().await;
        assert!(!addr.is_alive());
    }
}
