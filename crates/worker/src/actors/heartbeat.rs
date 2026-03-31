//! Heartbeat actor — broadcasts [`WorkerAnnouncement`] periodically.

use std::time::Duration;

use tracing::{debug, warn};
use willow_actor::{Actor, Addr, Context, Handler, IntervalHandle, Message};
use willow_identity::EndpointId;
use willow_network::TopicHandle;

use super::state::StateActor;
use super::GetRoleInfoMsg;
use crate::types::{WorkerAnnouncement, WorkerWireMessage};

/// Heartbeat actor that periodically queries state and broadcasts announcements.
pub struct HeartbeatActor<T: TopicHandle + 'static> {
    peer_id: EndpointId,
    interval: Duration,
    state_addr: Addr<StateActor>,
    topic: T,
    _interval_handle: Option<IntervalHandle>,
}

impl<T: TopicHandle + 'static> HeartbeatActor<T> {
    pub fn new(
        peer_id: EndpointId,
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

struct HeartbeatTick;
impl Message for HeartbeatTick {
    type Result = ();
}

impl<T: TopicHandle + 'static> Actor for HeartbeatActor<T> {
    fn started(&mut self, ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        debug!("heartbeat actor started (interval: {:?})", self.interval);
        self._interval_handle = Some(ctx.run_interval(self.interval, || HeartbeatTick));
        async {}
    }

    fn stopped(&mut self) -> impl std::future::Future<Output = ()> + Send {
        debug!("heartbeat actor shutting down");
        let peer_id = self.peer_id;
        let topic = self.topic.clone();
        async move {
            // Send departure before exiting.
            let departure = WorkerWireMessage::Departure { peer_id };
            if let Ok(bytes) = bincode::serialize(&departure) {
                if let Err(e) = topic.broadcast(bytes::Bytes::from(bytes)).await {
                    warn!(%e, "failed to send departure message");
                }
            }
        }
    }
}

impl<T: TopicHandle + 'static> Handler<HeartbeatTick> for HeartbeatActor<T> {
    fn handle(
        &mut self,
        _msg: HeartbeatTick,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        let state_addr = self.state_addr.clone();
        let peer_id = self.peer_id;
        let topic = self.topic.clone();

        async move {
            let role_info = match state_addr.ask(GetRoleInfoMsg).await {
                Ok(info) => info,
                Err(_) => {
                    warn!("state actor unavailable, skipping heartbeat");
                    return;
                }
            };

            let announcement = WorkerAnnouncement {
                peer_id,
                role: role_info,
                servers: vec![],
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            };

            let msg = WorkerWireMessage::Announcement(announcement);
            if let Ok(bytes) = bincode::serialize(&msg) {
                let _ = topic.broadcast(bytes::Bytes::from(bytes)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{WorkerRoleInfo, WORKERS_TOPIC};
    use willow_actor::System;
    use willow_network::mem::{MemHub, MemNetwork};
    use willow_network::{Network, TopicEvents};

    /// A minimal test role for the state actor.
    struct TestRole;
    impl crate::WorkerRole for TestRole {
        fn role_info(&self) -> WorkerRoleInfo {
            WorkerRoleInfo::Replay {
                servers_loaded: 1,
                events_buffered: 42,
                max_events: 1000,
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn heartbeat_sends_announcements() {
        let hub = MemHub::new();
        let net_a = MemNetwork::new(&hub);
        let net_b = MemNetwork::new(&hub);

        let topic_id = willow_network::topic_id(WORKERS_TOPIC);
        let (sender_a, _events_a) = net_a.subscribe(topic_id, vec![]).await.unwrap();
        let (_sender_b, mut events_b) = net_b.subscribe(topic_id, vec![]).await.unwrap();

        let system = System::new();

        let state_addr = system.spawn(StateActor {
            role: Box::new(TestRole),
        });

        let test_peer = net_a.id();
        let _hb = system.spawn(HeartbeatActor::new(
            test_peer,
            Duration::from_millis(50),
            state_addr,
            sender_a,
        ));

        // Wait for at least 1 announcement — drain neighbor events first.
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

        let decoded: WorkerWireMessage = bincode::deserialize(&data).unwrap();
        match decoded {
            WorkerWireMessage::Announcement(a) => {
                assert_eq!(a.peer_id, test_peer);
            }
            _ => panic!("expected Announcement"),
        }

        system.shutdown().await;

        // Check departure message was sent — drain any neighbor events.
        let departure_data = loop {
            let event = tokio::time::timeout(Duration::from_millis(500), events_b.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let willow_network::GossipEvent::Received(msg) = event {
                break msg.content;
            }
        };
        let decoded: WorkerWireMessage = bincode::deserialize(&departure_data).unwrap();
        assert!(matches!(decoded, WorkerWireMessage::Departure { .. }));
    }
}
