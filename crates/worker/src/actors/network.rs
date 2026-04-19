//! Network actor — bridges gossip topic events to the state actor.
//!
//! Uses [`StreamHandler`] to receive gossip events and dispatches
//! parsed messages to the state actor via typed [`Addr`] messages.

use willow_actor::{Actor, Addr, Context, Handler};
use willow_identity::EndpointId;
use willow_network::traits::{GossipEvent, TopicEvents};
use willow_network::TopicHandle;

use super::state::StateActor;
use super::{EventMsg, WorkerRequestMsg};
use crate::types::WorkerWireMessage;

/// Network actor that streams gossip events and forwards them to the state actor.
pub struct NetworkActor<E: TopicEvents + 'static, T: TopicHandle + 'static> {
    state_addr: Addr<StateActor>,
    local_peer_id: EndpointId,
    identity: willow_identity::Identity,
    events: Option<E>,
    /// Optional SERVER_OPS topic events stream.
    ops_events: Option<E>,
    reply_topic: T,
    /// Optional ready signal — drain tasks wait for `true` before pulling events.
    /// Uses `watch` so late subscribers see the value even if StateActor started first.
    ready: Option<tokio::sync::watch::Receiver<bool>>,
}

impl<E: TopicEvents + 'static, T: TopicHandle + 'static> NetworkActor<E, T> {
    pub fn new(
        events: E,
        state_addr: Addr<StateActor>,
        local_peer_id: EndpointId,
        reply_topic: T,
        identity: willow_identity::Identity,
    ) -> Self {
        Self {
            state_addr,
            local_peer_id,
            identity,
            events: Some(events),
            ops_events: None,
            reply_topic,
            ready: None,
        }
    }

    /// Attach a SERVER_OPS topic events stream. Events from this stream are
    /// parsed with [`parse_server_message`] and forwarded to the state actor.
    pub fn with_ops_events(mut self, ops_events: E) -> Self {
        self.ops_events = Some(ops_events);
        self
    }

    /// Attach a ready signal. Drain tasks will wait for `true` before pulling
    /// events, ensuring the `StateActor` has completed initialization.
    pub fn with_ready_signal(mut self, ready: tokio::sync::watch::Receiver<bool>) -> Self {
        self.ready = Some(ready);
        self
    }
}

/// Internal message wrapping a gossip event for the network actor.
struct GossipEventMsg(GossipEvent);
impl willow_actor::Message for GossipEventMsg {
    type Result = ();
}

/// Internal message wrapping a server ops gossip event.
struct ServerOpsEventMsg(GossipEvent);
impl willow_actor::Message for ServerOpsEventMsg {
    type Result = ();
}

impl<E: TopicEvents + 'static, T: TopicHandle + 'static> Actor for NetworkActor<E, T> {
    fn started(&mut self, ctx: &mut Context<Self>) -> impl std::future::Future<Output = ()> + Send {
        let ready = self.ready.take();

        // Spawn a task that drains WORKERS topic events.
        if let Some(mut events) = self.events.take() {
            let addr = ctx.address();
            let mut ready = ready.clone();
            willow_actor::runtime::spawn(async move {
                // Wait for StateActor to be ready before draining events.
                if let Some(ref mut rx) = ready {
                    rx.wait_for(|v| *v).await.ok();
                }
                while let Some(Ok(event)) = events.next().await {
                    if addr.do_send(GossipEventMsg(event)).is_err() {
                        break;
                    }
                }
            });
        }
        // Spawn a second task that drains SERVER_OPS topic events.
        if let Some(mut ops_events) = self.ops_events.take() {
            let addr = ctx.address();
            let mut ready = ready;
            willow_actor::runtime::spawn(async move {
                // Wait for StateActor to be ready before draining events.
                if let Some(ref mut rx) = ready {
                    rx.wait_for(|v| *v).await.ok();
                }
                while let Some(Ok(event)) = ops_events.next().await {
                    if addr.do_send(ServerOpsEventMsg(event)).is_err() {
                        break;
                    }
                }
            });
        }
        async {}
    }
}

impl<E: TopicEvents + 'static, T: TopicHandle + 'static> Handler<GossipEventMsg>
    for NetworkActor<E, T>
{
    fn handle(
        &mut self,
        msg: GossipEventMsg,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        let state_addr = self.state_addr.clone();
        let local_peer_id = self.local_peer_id;
        let identity = self.identity.clone();
        let event = msg.0;
        let reply_topic = self.reply_topic.clone();

        async move {
            if let GossipEvent::Received(msg) = event {
                let requester = msg.sender;
                match parse_worker_message(&msg.content, &local_peer_id) {
                    WorkerMessageAction::HandleRequest {
                        request_id,
                        payload,
                    } => {
                        if let Ok(response) = state_addr.ask(WorkerRequestMsg(payload)).await {
                            // target_peer identifies the original requester so
                            // clients can filter responses addressed to them.
                            let reply = WorkerWireMessage::Response {
                                request_id,
                                target_peer: requester,
                                payload: Box::new(response),
                            };
                            let wire = willow_common::WireMessage::Worker(reply);
                            if let Some(bytes) = willow_common::pack_wire(&wire, &identity) {
                                reply_topic.broadcast(bytes::Bytes::from(bytes)).await.ok();
                            }
                        }
                    }
                    WorkerMessageAction::Ignore => {}
                    WorkerMessageAction::DeserializeError(_) => {
                        match parse_server_message(&msg.content) {
                            ServerMessageAction::Events(events) => {
                                for event in events {
                                    state_addr.do_send(EventMsg(event)).ok();
                                }
                            }
                            ServerMessageAction::Ignore => {}
                        }
                    }
                }
            }
        }
    }
}

impl<E: TopicEvents + 'static, T: TopicHandle + 'static> Handler<ServerOpsEventMsg>
    for NetworkActor<E, T>
{
    fn handle(
        &mut self,
        msg: ServerOpsEventMsg,
        _ctx: &mut Context<Self>,
    ) -> impl std::future::Future<Output = ()> + Send {
        let state_addr = self.state_addr.clone();
        let event = msg.0;

        async move {
            if let GossipEvent::Received(msg) = event {
                match parse_server_message(&msg.content) {
                    ServerMessageAction::Events(events) => {
                        for event in events {
                            state_addr.do_send(EventMsg(event)).ok();
                        }
                    }
                    ServerMessageAction::Ignore => {}
                }
            }
        }
    }
}

// ───── Pure parse functions (unchanged) ────────────────────────────────────

/// Action produced by parsing an incoming worker topic message.
#[derive(Debug)]
pub enum WorkerMessageAction {
    /// Forward a request to the state actor and publish the response.
    HandleRequest {
        request_id: String,
        payload: willow_common::WorkerRequest,
    },
    /// No action needed (message not for us, or announcement/departure).
    Ignore,
    /// Could not deserialize the message.
    DeserializeError(String),
}

/// Parse a worker topic message and decide what action to take.
///
/// This is a pure function — no I/O, no channels — so it's easily
/// testable. The caller handles the actual I/O.
pub fn parse_worker_message(data: &[u8], local_peer_id: &EndpointId) -> WorkerMessageAction {
    let msg = match willow_common::unpack_wire(data) {
        Some((willow_common::WireMessage::Worker(m), _signer)) => m,
        Some(_) => return WorkerMessageAction::Ignore,
        None => {
            return WorkerMessageAction::DeserializeError(
                "invalid or unsigned worker message".to_string(),
            )
        }
    };

    match msg {
        WorkerWireMessage::Request {
            target_peer,
            payload,
            request_id,
        } => {
            // Accept Sync requests from any peer (broadcast protocol).
            // For other request types, only accept if targeted at us.
            let is_sync = matches!(payload, willow_common::WorkerRequest::Sync { .. });
            if target_peer == *local_peer_id || is_sync {
                WorkerMessageAction::HandleRequest {
                    request_id,
                    payload,
                }
            } else {
                WorkerMessageAction::Ignore
            }
        }
        WorkerWireMessage::Response { .. }
        | WorkerWireMessage::Announcement(_)
        | WorkerWireMessage::Departure { .. } => WorkerMessageAction::Ignore,
    }
}

/// Action produced by parsing a server ops / channel topic message.
#[derive(Debug)]
pub enum ServerMessageAction {
    /// One or more events to forward to the state actor.
    Events(Vec<willow_state::Event>),
    /// Could not parse the message (not an error — could be typing, voice, etc).
    Ignore,
}

/// Parse a signed server ops message and extract events.
///
/// Pure function — no I/O. Uses `willow_common::unpack_wire` to verify
/// the signature and deserialize.
pub fn parse_server_message(data: &[u8]) -> ServerMessageAction {
    if let Some((wire_msg, _signer)) = willow_common::unpack_wire(data) {
        match wire_msg {
            willow_common::WireMessage::Event(event) => ServerMessageAction::Events(vec![event]),
            willow_common::WireMessage::SyncBatch { events } => ServerMessageAction::Events(events),
            _ => ServerMessageAction::Ignore,
        }
    } else {
        ServerMessageAction::Ignore
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_common::{WorkerRequest, WorkerResponse, WorkerWireMessage};
    use willow_identity::Identity;
    use willow_state::HeadsSummary;

    fn gen_id() -> EndpointId {
        Identity::generate().endpoint_id()
    }

    fn pack_worker(msg: WorkerWireMessage, signer: &Identity) -> Vec<u8> {
        willow_common::pack_wire(&willow_common::WireMessage::Worker(msg), signer).unwrap()
    }

    #[test]
    fn parse_worker_request_targeted_at_us() {
        let signer = Identity::generate();
        let my_id = gen_id();
        let msg = WorkerWireMessage::Request {
            request_id: "req-1".to_string(),
            target_peer: my_id,
            payload: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                heads: HeadsSummary::default(),
            },
        };
        let data = pack_worker(msg, &signer);

        match parse_worker_message(&data, &my_id) {
            WorkerMessageAction::HandleRequest { request_id, .. } => {
                assert_eq!(request_id, "req-1");
            }
            other => panic!("expected HandleRequest, got {:?}", other),
        }
    }

    #[test]
    fn sync_request_accepted_regardless_of_target() {
        // Sync requests are broadcast — accepted even if target_peer differs.
        let signer = Identity::generate();
        let my_id = gen_id();
        let other_id = gen_id();
        let msg = WorkerWireMessage::Request {
            request_id: "req-3".to_string(),
            target_peer: other_id,
            payload: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                heads: HeadsSummary::default(),
            },
        };
        let data = pack_worker(msg, &signer);

        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::HandleRequest { .. }
        ));
    }

    #[test]
    fn history_request_not_for_us_ignored() {
        // Non-Sync requests targeted at another peer are ignored.
        let signer = Identity::generate();
        let my_id = gen_id();
        let other_id = gen_id();
        let msg = WorkerWireMessage::Request {
            request_id: "req-4".to_string(),
            target_peer: other_id,
            payload: WorkerRequest::History {
                server_id: "srv".to_string(),
                channel: None,
                before: None,
                limit: 50,
            },
        };
        let data = pack_worker(msg, &signer);

        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_announcement_ignored() {
        let signer = Identity::generate();
        let my_id = gen_id();
        let msg = WorkerWireMessage::Announcement(willow_common::WorkerAnnouncement {
            peer_id: gen_id(),
            role: willow_common::WorkerRoleInfo::Replay {
                servers_loaded: 1,
                events_buffered: 0,
                max_events: 1000,
                pending_count: 0,
            },
            servers: vec![],
            timestamp: 0,
        });
        let data = pack_worker(msg, &signer);

        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_departure_ignored() {
        let signer = Identity::generate();
        let my_id = gen_id();
        let msg = WorkerWireMessage::Departure { peer_id: gen_id() };
        let data = pack_worker(msg, &signer);

        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_response_ignored() {
        let signer = Identity::generate();
        let my_id = gen_id();
        let msg = WorkerWireMessage::Response {
            request_id: "r1".to_string(),
            target_peer: my_id,
            payload: Box::new(WorkerResponse::Denied {
                reason: "test".to_string(),
            }),
        };
        let data = pack_worker(msg, &signer);

        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn unsigned_bytes_rejected() {
        // Raw bincode (old unsigned path) must be rejected.
        let my_id = gen_id();
        let msg = WorkerWireMessage::Announcement(willow_common::WorkerAnnouncement {
            peer_id: gen_id(),
            role: willow_common::WorkerRoleInfo::Replay {
                servers_loaded: 0,
                events_buffered: 0,
                max_events: 1000,
                pending_count: 0,
            },
            servers: vec![],
            timestamp: 0,
        });
        let data = bincode::serialize(&msg).unwrap();
        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::DeserializeError(_)
        ));
    }

    #[test]
    fn non_worker_wire_message_ignored() {
        // A signed WireMessage that is not a Worker variant → Ignore.
        let id = Identity::generate();
        let my_id = gen_id();
        let event = willow_state::Event::new(
            &id,
            1,
            willow_state::EventHash::ZERO,
            vec![],
            willow_state::EventKind::Message {
                channel_id: "ch".to_string(),
                body: "hi".to_string(),
                reply_to: None,
            },
            1000,
        );
        let data =
            willow_common::pack_wire(&willow_common::WireMessage::Event(event), &id).unwrap();
        assert!(matches!(
            parse_worker_message(&data, &my_id),
            WorkerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_worker_garbage_data() {
        let my_id = gen_id();
        assert!(matches!(
            parse_worker_message(b"not valid bincode", &my_id),
            WorkerMessageAction::DeserializeError(_)
        ));
    }

    #[test]
    fn parse_worker_empty_data() {
        let my_id = gen_id();
        assert!(matches!(
            parse_worker_message(&[], &my_id),
            WorkerMessageAction::DeserializeError(_)
        ));
    }

    #[test]
    fn parse_server_message_with_signed_event() {
        let id = willow_identity::Identity::generate();
        let event = willow_state::Event::new(
            &id,
            1,
            willow_state::EventHash::ZERO,
            vec![],
            willow_state::EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello".to_string(),
                reply_to: None,
            },
            1000,
        );
        let expected_hash = event.hash;

        let data =
            willow_common::pack_wire(&willow_common::WireMessage::Event(event), &id).unwrap();

        match parse_server_message(&data) {
            ServerMessageAction::Events(events) => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].hash, expected_hash);
            }
            ServerMessageAction::Ignore => panic!("expected Events"),
        }
    }

    #[test]
    fn parse_server_message_with_sync_batch() {
        let id = willow_identity::Identity::generate();
        let e1 = willow_state::Event::new(
            &id,
            1,
            willow_state::EventHash::ZERO,
            vec![],
            willow_state::EventKind::CreateChannel {
                name: "ch".to_string(),
                channel_id: "c1".to_string(),
                kind: willow_state::ChannelKind::Text,
            },
            100,
        );
        let e2 = willow_state::Event::new(
            &id,
            2,
            e1.hash,
            vec![],
            willow_state::EventKind::Message {
                channel_id: "c1".to_string(),
                body: "msg".to_string(),
                reply_to: None,
            },
            200,
        );
        let events = vec![e1, e2];

        let data = willow_common::pack_wire(&willow_common::WireMessage::SyncBatch { events }, &id)
            .unwrap();

        match parse_server_message(&data) {
            ServerMessageAction::Events(events) => assert_eq!(events.len(), 2),
            ServerMessageAction::Ignore => panic!("expected Events"),
        }
    }

    #[test]
    fn parse_server_message_typing_indicator_ignored() {
        let id = willow_identity::Identity::generate();
        let data = willow_common::pack_wire(
            &willow_common::WireMessage::TypingIndicator {
                channel: "general".to_string(),
            },
            &id,
        )
        .unwrap();

        assert!(matches!(
            parse_server_message(&data),
            ServerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_server_message_garbage_ignored() {
        assert!(matches!(
            parse_server_message(b"garbage data"),
            ServerMessageAction::Ignore
        ));
    }

    #[test]
    fn parse_server_message_empty_ignored() {
        assert!(matches!(
            parse_server_message(&[]),
            ServerMessageAction::Ignore
        ));
    }
}
