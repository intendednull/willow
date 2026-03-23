//! Integration test: relay serves event history to a new peer when all
//! original peers are offline.
//!
//! Flow:
//! 1. Start a relay node.
//! 2. Start peer A, connect to relay, publish events via gossipsub.
//! 3. Verify the relay stored the events.
//! 4. Drop peer A (simulate disconnect).
//! 5. Start peer B (new peer), connect to relay.
//! 6. Peer B sends a SyncRequest.
//! 7. The relay responds with a SyncBatch containing stored events.
//! 8. Peer B receives and validates the batch.

use std::time::Duration;

use libp2p::{
    futures::StreamExt, gossipsub, identify, kad, noise, swarm::SwarmEvent, tcp, yamux, Multiaddr,
    PeerId, Swarm, SwarmBuilder,
};
use willow_relay::WireMessage;
use willow_state::{Event, EventKind, StateHash};

/// Build a minimal peer swarm with GossipSub + Identify + Kademlia.
async fn build_peer() -> (Swarm<PeerBehaviour>, willow_identity::Identity) {
    let identity = willow_identity::Identity::generate();
    let keypair = identity.keypair().clone();
    let local_peer_id = PeerId::from(keypair.public());

    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .unwrap()
        .with_behaviour(|key| {
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(1))
                .validation_mode(gossipsub::ValidationMode::Strict)
                .message_id_fn(|msg: &gossipsub::Message| {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    msg.data.hash(&mut hasher);
                    msg.topic.hash(&mut hasher);
                    gossipsub::MessageId::from(hasher.finish().to_string())
                })
                .build()
                .expect("valid gossipsub config");

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gossipsub_config,
            )
            .expect("valid gossipsub behaviour");

            let kademlia =
                kad::Behaviour::new(local_peer_id, kad::store::MemoryStore::new(local_peer_id));

            let identify = identify::Behaviour::new(identify::Config::new(
                "/willow/1.0.0".to_string(),
                key.public(),
            ));

            Ok(PeerBehaviour {
                gossipsub,
                kademlia,
                identify,
            })
        })
        .unwrap()
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    (swarm, identity)
}

#[derive(libp2p::swarm::NetworkBehaviour)]
struct PeerBehaviour {
    gossipsub: gossipsub::Behaviour,
    kademlia: kad::Behaviour<kad::store::MemoryStore>,
    identify: identify::Behaviour,
}

/// Pack a WireMessage into signed bytes ready for gossipsub.
fn pack_wire(msg: &WireMessage, identity: &willow_identity::Identity) -> Vec<u8> {
    let envelope =
        willow_transport::pack_envelope(willow_transport::MessageType::Channel, msg).unwrap();
    willow_identity::pack(&envelope, identity).unwrap()
}

/// Unpack signed gossipsub bytes into a WireMessage.
fn unpack_wire(data: &[u8]) -> Option<WireMessage> {
    let (envelope_bytes, _signer) = willow_identity::unpack::<Vec<u8>>(data).ok()?;
    let (msg, willow_transport::MessageType::Channel) =
        willow_transport::unpack_envelope::<WireMessage>(&envelope_bytes).ok()?
    else {
        return None;
    };
    Some(msg)
}

/// Get the first TCP listen address from the relay.
async fn wait_for_listen_addr(relay: &mut willow_relay::Relay) -> Multiaddr {
    loop {
        let event = relay.swarm.select_next_some().await;
        if let SwarmEvent::NewListenAddr { address, .. } = &event {
            // Only return TCP addresses (not WebSocket).
            let addr_str = address.to_string();
            if !addr_str.contains("/ws") {
                let addr = address.clone();
                relay.handle_swarm_event(event);
                return addr;
            }
        }
        relay.handle_swarm_event(event);
    }
}

/// Pump a swarm for `duration`, processing all events.
async fn pump_relay(relay: &mut willow_relay::Relay, duration: Duration) {
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        tokio::select! {
            event = relay.swarm.select_next_some() => {
                relay.handle_swarm_event(event);
            }
            _ = tokio::time::sleep_until(deadline) => {
                return;
            }
        }
    }
}

/// Pump both a relay and a peer swarm concurrently for `duration`.
/// Returns gossipsub messages received by the peer.
async fn pump_both(
    relay: &mut willow_relay::Relay,
    peer: &mut Swarm<PeerBehaviour>,
    duration: Duration,
) -> Vec<Vec<u8>> {
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        tokio::select! {
            event = relay.swarm.select_next_some() => {
                relay.handle_swarm_event(event);
            }
            event = peer.select_next_some() => {
                if let SwarmEvent::Behaviour(PeerBehaviourEvent::Gossipsub(
                    gossipsub::Event::Message { message, .. },
                )) = event
                {
                    messages.push(message.data);
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                return messages;
            }
        }
    }
}

/// Pump relay + two peers concurrently.
/// Returns gossipsub messages received by peer A and peer B respectively.
async fn pump_relay_and_two_peers(
    relay: &mut willow_relay::Relay,
    peer_a: &mut Swarm<PeerBehaviour>,
    peer_b: &mut Swarm<PeerBehaviour>,
    duration: Duration,
) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let mut msgs_a = Vec::new();
    let mut msgs_b = Vec::new();
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        tokio::select! {
            event = relay.swarm.select_next_some() => {
                relay.handle_swarm_event(event);
            }
            event = peer_a.select_next_some() => {
                if let SwarmEvent::Behaviour(PeerBehaviourEvent::Gossipsub(
                    gossipsub::Event::Message { message, .. },
                )) = event
                {
                    msgs_a.push(message.data);
                }
            }
            event = peer_b.select_next_some() => {
                if let SwarmEvent::Behaviour(PeerBehaviourEvent::Gossipsub(
                    gossipsub::Event::Message { message, .. },
                )) = event
                {
                    msgs_b.push(message.data);
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                return (msgs_a, msgs_b);
            }
        }
    }
}

fn make_test_event(id: &str, author: &str, channel_id: &str, body: &str) -> Event {
    Event {
        id: id.to_string(),
        parent_hash: StateHash::ZERO,
        author: author.to_string(),
        timestamp_ms: 1000,
        kind: EventKind::Message {
            channel_id: channel_id.to_string(),
            body: body.to_string(),
        },
    }
}

fn make_channel_event(id: &str, author: &str, channel_id: &str, name: &str) -> Event {
    Event {
        id: id.to_string(),
        parent_hash: StateHash::ZERO,
        author: author.to_string(),
        timestamp_ms: 500,
        kind: EventKind::CreateChannel {
            name: name.to_string(),
            channel_id: channel_id.to_string(),
        },
    }
}

#[tokio::test]
async fn relay_serves_history_to_new_peer() {
    let _ = tracing_subscriber::fmt().with_env_filter("warn").try_init();

    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("events.db");

    // ── 1. Start the relay ─────────────────────────────────────────────
    let relay_kp = libp2p::identity::Keypair::generate_ed25519();
    let mut relay = willow_relay::Relay::start(relay_kp, &db_path)
        .await
        .expect("relay should start");

    // Listen on a random TCP port.
    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    relay.swarm.listen_on(listen_addr).unwrap();
    let relay_addr = wait_for_listen_addr(&mut relay).await;
    let _relay_peer_id = relay.peer_id;

    let topic_str = "test-server/general";
    let gs_topic = gossipsub::IdentTopic::new(topic_str);

    // Subscribe the relay to the topic so it can receive messages.
    relay
        .swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();

    // ── 2. Start peer A, connect to relay ──────────────────────────────
    let (mut peer_a, identity_a) = build_peer().await;
    let peer_a_author = identity_a.peer_id().to_string();

    // Subscribe peer A to the topic.
    peer_a
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();

    // Dial the relay.
    peer_a.dial(relay_addr.clone()).unwrap();

    // Pump both to establish connection and let GossipSub mesh form.
    pump_both(&mut relay, &mut peer_a, Duration::from_secs(3)).await;

    // ── 3. Peer A publishes events ─────────────────────────────────────
    let channel_event = make_channel_event("evt-ch-1", &peer_a_author, "ch-1", "general");
    let msg_event_1 = make_test_event("evt-msg-1", &peer_a_author, "ch-1", "hello from A");
    let msg_event_2 = make_test_event("evt-msg-2", &peer_a_author, "ch-1", "second message");

    let wire_ch = WireMessage::Event(channel_event.clone());
    let wire_m1 = WireMessage::Event(msg_event_1.clone());
    let wire_m2 = WireMessage::Event(msg_event_2.clone());

    let signed_ch = pack_wire(&wire_ch, &identity_a);
    let signed_m1 = pack_wire(&wire_m1, &identity_a);
    let signed_m2 = pack_wire(&wire_m2, &identity_a);

    peer_a
        .behaviour_mut()
        .gossipsub
        .publish(gs_topic.clone(), signed_ch)
        .expect("publish channel event");
    peer_a
        .behaviour_mut()
        .gossipsub
        .publish(gs_topic.clone(), signed_m1)
        .expect("publish msg 1");
    peer_a
        .behaviour_mut()
        .gossipsub
        .publish(gs_topic.clone(), signed_m2)
        .expect("publish msg 2");

    // Pump so the relay receives and stores the events.
    pump_both(&mut relay, &mut peer_a, Duration::from_secs(3)).await;

    // ── 4. Verify the relay stored the events ──────────────────────────
    assert_eq!(
        relay.event_store.count(),
        3,
        "relay should have stored 3 events (1 channel + 2 messages)"
    );

    // ── 5. Drop peer A (simulate going offline) ───────────────────────
    drop(peer_a);

    // Let the relay notice the disconnect.
    pump_relay(&mut relay, Duration::from_millis(500)).await;

    // ── 6. Start peer B (new peer), connect to relay ──────────────────
    let (mut peer_b, identity_b) = build_peer().await;

    // Subscribe peer B to the topic.
    peer_b
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();

    // Dial the relay.
    peer_b.dial(relay_addr.clone()).unwrap();

    // Let the mesh form between relay and peer B.
    pump_both(&mut relay, &mut peer_b, Duration::from_secs(3)).await;

    // ── 7. Peer B sends a SyncRequest ─────────────────────────────────
    let sync_req = WireMessage::SyncRequest {
        state_hash: StateHash::ZERO,
        topic: Some(topic_str.to_string()),
    };
    let signed_req = pack_wire(&sync_req, &identity_b);

    peer_b
        .behaviour_mut()
        .gossipsub
        .publish(gs_topic.clone(), signed_req)
        .expect("publish sync request");

    // ── 8. Pump relay + peer B so the relay responds and peer B receives ──
    let received = pump_both(&mut relay, &mut peer_b, Duration::from_secs(3)).await;

    // ── 9. Peer B should have received a SyncBatch ────────────────────
    let mut found_batch = false;
    let mut batch_events = Vec::new();

    for msg_data in &received {
        if let Some(WireMessage::SyncBatch { events }) = unpack_wire(msg_data) {
            found_batch = true;
            batch_events = events;
            break;
        }
    }

    assert!(
        found_batch,
        "peer B should receive a SyncBatch from the relay"
    );
    assert_eq!(
        batch_events.len(),
        3,
        "SyncBatch should contain all 3 stored events"
    );

    // Verify the events are the ones peer A published.
    let ids: Vec<&str> = batch_events.iter().map(|e| e.id.as_str()).collect();
    assert!(
        ids.contains(&"evt-ch-1"),
        "batch should contain channel event"
    );
    assert!(ids.contains(&"evt-msg-1"), "batch should contain msg 1");
    assert!(ids.contains(&"evt-msg-2"), "batch should contain msg 2");

    // Verify event contents.
    let ch_evt = batch_events.iter().find(|e| e.id == "evt-ch-1").unwrap();
    assert!(matches!(
        &ch_evt.kind,
        EventKind::CreateChannel { name, channel_id }
        if name == "general" && channel_id == "ch-1"
    ));

    let msg1 = batch_events.iter().find(|e| e.id == "evt-msg-1").unwrap();
    assert!(matches!(
        &msg1.kind,
        EventKind::Message { body, channel_id }
        if body == "hello from A" && channel_id == "ch-1"
    ));
}

#[tokio::test]
async fn relay_serves_history_to_multiple_new_peers() {
    let _ = tracing_subscriber::fmt().with_env_filter("warn").try_init();

    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("events.db");

    // Start relay.
    let relay_kp = libp2p::identity::Keypair::generate_ed25519();
    let mut relay = willow_relay::Relay::start(relay_kp, &db_path)
        .await
        .unwrap();

    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    relay.swarm.listen_on(listen_addr).unwrap();
    let relay_addr = wait_for_listen_addr(&mut relay).await;

    let topic_str = "test-server/general";
    let gs_topic = gossipsub::IdentTopic::new(topic_str);

    relay
        .swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();

    // Start peer A, connect, publish 2 messages.
    let (mut peer_a, identity_a) = build_peer().await;
    let author_a = identity_a.peer_id().to_string();
    peer_a
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();
    peer_a.dial(relay_addr.clone()).unwrap();
    pump_both(&mut relay, &mut peer_a, Duration::from_secs(3)).await;

    let evt1 = make_test_event("a-1", &author_a, "ch-1", "first");
    let evt2 = make_test_event("a-2", &author_a, "ch-1", "second");
    peer_a
        .behaviour_mut()
        .gossipsub
        .publish(
            gs_topic.clone(),
            pack_wire(&WireMessage::Event(evt1), &identity_a),
        )
        .unwrap();
    peer_a
        .behaviour_mut()
        .gossipsub
        .publish(
            gs_topic.clone(),
            pack_wire(&WireMessage::Event(evt2), &identity_a),
        )
        .unwrap();
    pump_both(&mut relay, &mut peer_a, Duration::from_secs(3)).await;

    assert_eq!(relay.event_store.count(), 2);

    // Drop peer A.
    drop(peer_a);
    pump_relay(&mut relay, Duration::from_millis(500)).await;

    // Start peer B and peer C, both connect.
    let (mut peer_b, identity_b) = build_peer().await;
    let (mut peer_c, identity_c) = build_peer().await;

    peer_b
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();
    peer_c
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();

    peer_b.dial(relay_addr.clone()).unwrap();
    peer_c.dial(relay_addr.clone()).unwrap();

    // Let mesh form with both peers.
    pump_relay_and_two_peers(&mut relay, &mut peer_b, &mut peer_c, Duration::from_secs(3)).await;

    // Peer B sends sync request.
    let sync_req = WireMessage::SyncRequest {
        state_hash: StateHash::ZERO,
        topic: Some(topic_str.to_string()),
    };
    peer_b
        .behaviour_mut()
        .gossipsub
        .publish(gs_topic.clone(), pack_wire(&sync_req, &identity_b))
        .unwrap();

    let (msgs_b, msgs_c) =
        pump_relay_and_two_peers(&mut relay, &mut peer_b, &mut peer_c, Duration::from_secs(3))
            .await;

    // Peer B should receive the batch.
    let batch_b: Vec<WireMessage> = msgs_b
        .iter()
        .filter_map(|d| unpack_wire(d))
        .filter(|m| matches!(m, WireMessage::SyncBatch { .. }))
        .collect();
    assert!(!batch_b.is_empty(), "peer B should receive a SyncBatch");

    if let WireMessage::SyncBatch { events } = &batch_b[0] {
        assert_eq!(events.len(), 2);
    }

    // Peer C also sends a sync request and should get the same data.
    let sync_req_c = WireMessage::SyncRequest {
        state_hash: StateHash::ZERO,
        topic: Some(topic_str.to_string()),
    };
    peer_c
        .behaviour_mut()
        .gossipsub
        .publish(gs_topic.clone(), pack_wire(&sync_req_c, &identity_c))
        .unwrap();

    let (_, msgs_c2) =
        pump_relay_and_two_peers(&mut relay, &mut peer_b, &mut peer_c, Duration::from_secs(3))
            .await;

    let batch_c: Vec<WireMessage> = msgs_c2
        .iter()
        .chain(msgs_c.iter())
        .filter_map(|d| unpack_wire(d))
        .filter(|m| matches!(m, WireMessage::SyncBatch { .. }))
        .collect();
    assert!(
        !batch_c.is_empty(),
        "peer C should also receive a SyncBatch"
    );

    if let WireMessage::SyncBatch { events } = &batch_c[0] {
        assert_eq!(events.len(), 2);
    }
}

#[tokio::test]
async fn relay_stores_events_from_multiple_peers() {
    let _ = tracing_subscriber::fmt().with_env_filter("warn").try_init();

    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("events.db");

    let relay_kp = libp2p::identity::Keypair::generate_ed25519();
    let mut relay = willow_relay::Relay::start(relay_kp, &db_path)
        .await
        .unwrap();

    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    relay.swarm.listen_on(listen_addr).unwrap();
    let relay_addr = wait_for_listen_addr(&mut relay).await;

    let topic_str = "test-server/general";
    let gs_topic = gossipsub::IdentTopic::new(topic_str);

    relay
        .swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();

    // Peer A publishes 1 event.
    let (mut peer_a, identity_a) = build_peer().await;
    let author_a = identity_a.peer_id().to_string();
    peer_a
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();
    peer_a.dial(relay_addr.clone()).unwrap();
    pump_both(&mut relay, &mut peer_a, Duration::from_secs(3)).await;

    let evt_a = make_test_event("from-a", &author_a, "ch-1", "hello from A");
    peer_a
        .behaviour_mut()
        .gossipsub
        .publish(
            gs_topic.clone(),
            pack_wire(&WireMessage::Event(evt_a), &identity_a),
        )
        .unwrap();
    pump_both(&mut relay, &mut peer_a, Duration::from_secs(2)).await;

    // Peer B connects and publishes 1 event.
    let (mut peer_b, identity_b) = build_peer().await;
    let author_b = identity_b.peer_id().to_string();
    peer_b
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();
    peer_b.dial(relay_addr.clone()).unwrap();

    pump_relay_and_two_peers(&mut relay, &mut peer_a, &mut peer_b, Duration::from_secs(3)).await;

    let evt_b = make_test_event("from-b", &author_b, "ch-1", "hello from B");
    peer_b
        .behaviour_mut()
        .gossipsub
        .publish(
            gs_topic.clone(),
            pack_wire(&WireMessage::Event(evt_b), &identity_b),
        )
        .unwrap();
    pump_relay_and_two_peers(&mut relay, &mut peer_a, &mut peer_b, Duration::from_secs(2)).await;

    assert_eq!(
        relay.event_store.count(),
        2,
        "relay should store events from both peers"
    );

    // Drop both peers.
    drop(peer_a);
    drop(peer_b);
    pump_relay(&mut relay, Duration::from_millis(500)).await;

    // Peer C connects and requests sync — should get both events.
    let (mut peer_c, identity_c) = build_peer().await;
    peer_c
        .behaviour_mut()
        .gossipsub
        .subscribe(&gs_topic)
        .unwrap();
    peer_c.dial(relay_addr.clone()).unwrap();
    pump_both(&mut relay, &mut peer_c, Duration::from_secs(3)).await;

    let sync_req = WireMessage::SyncRequest {
        state_hash: StateHash::ZERO,
        topic: Some(topic_str.to_string()),
    };
    peer_c
        .behaviour_mut()
        .gossipsub
        .publish(gs_topic.clone(), pack_wire(&sync_req, &identity_c))
        .unwrap();

    let received = pump_both(&mut relay, &mut peer_c, Duration::from_secs(3)).await;

    let batch: Vec<WireMessage> = received
        .iter()
        .filter_map(|d| unpack_wire(d))
        .filter(|m| matches!(m, WireMessage::SyncBatch { .. }))
        .collect();
    assert!(!batch.is_empty(), "peer C should receive a SyncBatch");

    if let WireMessage::SyncBatch { events } = &batch[0] {
        assert_eq!(events.len(), 2, "batch should contain events from A and B");
        let ids: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(&"from-a"));
        assert!(ids.contains(&"from-b"));
    }
}
