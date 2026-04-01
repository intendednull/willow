//! Per-topic listener tasks that stream GossipEvents and mutate state via the actor.

use willow_actor::Addr;
use willow_identity::EndpointId;
use willow_network::traits::{GossipEvent, TopicEvents};

use willow_network::traits::TopicHandle;

use crate::client_actor::ClientStateActor;
use crate::events::ClientEvent;
use crate::mutations;

/// Spawn an async task that listens for gossip events on a topic.
pub fn spawn_topic_listener<T: TopicHandle + 'static, E: TopicEvents + 'static>(
    events: E,
    topic: T,
    state_addr: Addr<ClientStateActor>,
    event_broker: Addr<willow_actor::Broker<ClientEvent>>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::task::spawn_local(topic_listener_loop(events, topic, state_addr, event_broker));

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(topic_listener_loop(events, topic, state_addr, event_broker));
}

async fn topic_listener_loop<T: TopicHandle, E: TopicEvents>(
    mut events: E,
    topic: T,
    state_addr: Addr<ClientStateActor>,
    event_broker: Addr<willow_actor::Broker<ClientEvent>>,
) {
    while let Some(Ok(gossip_event)) = events.next().await {
        match gossip_event {
            GossipEvent::Received(msg) => {
                process_received_message(&msg.content, msg.sender, &state_addr, &event_broker, &topic)
                    .await;
            }
            GossipEvent::NeighborUp(id) => {
                let id2 = id;
                let _ = crate::client_actor::mutate_state(&state_addr, move |s| {
                    if !s.state.chat.peers.contains(&id2) {
                        s.state.chat.peers.push(id2);
                    }
                })
                .await;
                let _ = event_broker.do_send(willow_actor::Publish(ClientEvent::PeerConnected(id)));
            }
            GossipEvent::NeighborDown(id) => {
                let id2 = id;
                let _ = crate::client_actor::mutate_state(&state_addr, move |s| {
                    s.state.chat.peers.retain(|p| p != &id2);
                })
                .await;
                let _ = event_broker.do_send(willow_actor::Publish(ClientEvent::PeerDisconnected(id)));
            }
        }
    }
}

async fn process_received_message<T: TopicHandle>(
    data: &[u8],
    _sender: EndpointId,
    state_addr: &Addr<ClientStateActor>,
    event_broker: &Addr<willow_actor::Broker<ClientEvent>>,
    topic: &T,
) {
    // Try profile broadcast first.
    if let Ok((profile, willow_transport::MessageType::Identity)) =
        willow_transport::unpack_envelope::<willow_identity::UserProfile>(data)
    {
        let peer_id = profile.peer_id;
        let display_name = profile.display_name.clone();
        let _ = crate::client_actor::mutate_state(state_addr, move |s| {
            s.state.profiles.names.insert(peer_id, display_name.clone());
        })
        .await;
        let _ = event_broker.do_send(willow_actor::Publish(ClientEvent::ProfileUpdated {
            peer_id: profile.peer_id,
            display_name: profile.display_name,
        }));
        return;
    }

    let Some((wire_msg, signer)) = crate::ops::unpack_wire(data) else {
        return;
    };

    match wire_msg {
        crate::ops::WireMessage::Event(event) => {
            if event.author != signer {
                return;
            }
            let event_clone = event.clone();
            let client_events = crate::client_actor::mutate_state(state_addr, move |s| {
                if !s.state.chat.peers.contains(&signer) {
                    s.state.chat.peers.push(signer);
                }
                let result = willow_state::apply_lenient(&mut s.state.event_state, &event_clone);
                if matches!(result, willow_state::ApplyResult::Applied) {
                    use willow_state::EventStore as _;
                    s.state.event_store.append(event_clone.clone());
                    let hash = s.state.event_state.hash();
                    s.state.event_store.set_latest_hash(hash);
                    if s.config.persistence {
                        if let Some(sid) = &s.state.active_server {
                            crate::storage::save_server_state(sid, &s.state.event_state);
                        }
                    }
                    mutations::derive_client_events(&event_clone)
                } else {
                    vec![]
                }
            })
            .await;
            for e in client_events {
                let _ = event_broker.do_send(willow_actor::Publish(e));
            }
        }
        crate::ops::WireMessage::SyncBatch { events: batch } => {
            let client_events = crate::client_actor::mutate_state(state_addr, move |s| {
                if !s.state.chat.peers.contains(&signer) {
                    s.state.chat.peers.push(signer);
                }
                let mut sorted = batch;
                sorted.sort_by_key(|e| e.timestamp_ms);
                let count = sorted.len();
                let mut client_events = Vec::new();
                for event in &sorted {
                    let result = willow_state::apply_lenient(&mut s.state.event_state, event);
                    if matches!(result, willow_state::ApplyResult::Applied) {
                        use willow_state::EventStore as _;
                        s.state.event_store.append(event.clone());
                        let hash = s.state.event_state.hash();
                        s.state.event_store.set_latest_hash(hash);
                        client_events.extend(mutations::derive_client_events(event));
                    }
                }
                if count > 0 {
                    if s.config.persistence {
                        if let Some(sid) = &s.state.active_server {
                            crate::storage::save_server_state(sid, &s.state.event_state);
                        }
                    }
                    crate::reconcile_topic_map(&mut s.state);
                    client_events.push(ClientEvent::SyncCompleted { ops_applied: count });
                }
                client_events
            })
            .await;
            for e in client_events {
                let _ = event_broker.do_send(willow_actor::Publish(e));
            }
        }
        crate::ops::WireMessage::SyncRequest { state_hash, .. } => {
            use willow_state::EventStore as _;
            let (missing, identity) = crate::client_actor::read_state(state_addr, move |s| {
                let m = s.state.event_store.events_since(&state_hash);
                let id = s.identity.clone();
                (m, id)
            })
            .await;
            if !missing.is_empty() {
                let msg = crate::ops::WireMessage::SyncBatch { events: missing };
                if let Some(data) = crate::ops::pack_wire(&msg, &identity) {
                    let _ = topic.broadcast(bytes::Bytes::from(data)).await;
                }
            }
        }
        crate::ops::WireMessage::TypingIndicator { channel } => {
            let _ = crate::client_actor::mutate_state(state_addr, move |s| {
                let now = crate::util::current_time_ms();
                s.typing_peers.insert(signer, (channel, now));
                if !s.state.chat.peers.contains(&signer) {
                    s.state.chat.peers.push(signer);
                }
            })
            .await;
        }
        crate::ops::WireMessage::VoiceJoin {
            channel_id,
            peer_id,
        } => {
            let ch = channel_id.clone();
            let _ = crate::client_actor::mutate_state(state_addr, move |s| {
                s.voice_participants.entry(ch).or_default().insert(peer_id);
            })
            .await;
            let _ = event_broker.do_send(willow_actor::Publish(ClientEvent::VoiceJoined {
                channel_id,
                peer_id,
            }));
        }
        crate::ops::WireMessage::VoiceLeave {
            channel_id,
            peer_id,
        } => {
            let ch = channel_id.clone();
            let _ = crate::client_actor::mutate_state(state_addr, move |s| {
                if let Some(p) = s.voice_participants.get_mut(&ch) {
                    p.remove(&peer_id);
                }
            })
            .await;
            let _ = event_broker.do_send(willow_actor::Publish(ClientEvent::VoiceLeft {
                channel_id,
                peer_id,
            }));
        }
        crate::ops::WireMessage::VoiceSignal {
            channel_id,
            target_peer,
            signal,
        } => {
            let our_id =
                crate::client_actor::read_state(state_addr, |s| s.identity.endpoint_id()).await;
            if target_peer == our_id {
                let _ = event_broker.do_send(willow_actor::Publish(ClientEvent::VoiceSignal {
                    channel_id,
                    from_peer: signer,
                    signal,
                }));
            }
        }
        crate::ops::WireMessage::JoinRequest { link_id, peer_id } => {
            let should_respond = crate::client_actor::mutate_state(state_addr, move |s| {
                let valid = s
                    .join_links
                    .iter_mut()
                    .find(|l| l.link_id == link_id && l.is_valid());
                if let Some(link) = valid {
                    link.used += 1;
                    if s.config.persistence {
                        crate::storage::save_join_links(
                            s.state.active_server.as_deref().unwrap_or(""),
                            &s.join_links,
                        );
                    }
                    Some(s.identity.clone())
                } else {
                    None
                }
            })
            .await;
            if let Some(identity) = should_respond {
                match crate::generate_invite_via_actor(state_addr, &peer_id).await {
                    Ok(invite_data) => {
                        let msg = crate::ops::WireMessage::JoinResponse {
                            target_peer: peer_id,
                            invite_data,
                        };
                        if let Some(data) = crate::ops::pack_wire(&msg, &identity) {
                            let _ = topic.broadcast(bytes::Bytes::from(data)).await;
                        }
                    }
                    Err(e) => tracing::warn!(%e, "failed to generate invite for join link"),
                }
            }
        }
        crate::ops::WireMessage::JoinResponse {
            target_peer,
            invite_data,
        } => {
            let our_id =
                crate::client_actor::read_state(state_addr, |s| s.identity.endpoint_id()).await;
            if target_peer == our_id {
                let _ = event_broker.do_send(willow_actor::Publish(ClientEvent::JoinLinkResponse { invite_data }));
            }
        }
        crate::ops::WireMessage::JoinDenied {
            target_peer,
            reason,
        } => {
            let our_id =
                crate::client_actor::read_state(state_addr, |s| s.identity.endpoint_id()).await;
            if target_peer == our_id {
                let _ = event_broker.do_send(willow_actor::Publish(ClientEvent::JoinLinkDenied { reason }));
            }
        }
    }
}
