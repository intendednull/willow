//! Per-topic listener tasks that stream GossipEvents and mutate state via domain actors.

use std::sync::{Arc, Mutex, RwLock};

use willow_actor::Addr;
use willow_identity::EndpointId;
use willow_network::traits::TopicHandle;
use willow_network::traits::{GossipEvent, TopicEvents};
use willow_state::{EventDag, InsertError, PendingBuffer};

use crate::events::ClientEvent;
use crate::mutations;
use crate::persistence_actor;
use crate::state_actors;

/// Context passed to listener tasks with all the actor addresses they need.
pub struct ListenerCtx {
    pub event_state: Addr<willow_actor::StateActor<willow_state::ServerState>>,
    pub chat_meta: Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
    pub profiles: Addr<willow_actor::StateActor<state_actors::ProfileState>>,
    pub network: Addr<willow_actor::StateActor<state_actors::NetworkMeta>>,
    pub voice: Addr<willow_actor::StateActor<state_actors::VoiceState>>,
    pub persistence: Addr<persistence_actor::PersistenceActor>,
    pub event_broker: Addr<willow_actor::Broker<ClientEvent>>,
    pub identity: willow_identity::Identity,
    pub join_links: Arc<Mutex<Vec<crate::ops::JoinLink>>>,
    pub dag: Arc<RwLock<EventDag>>,
    pub pending: Arc<Mutex<PendingBuffer>>,
}

impl Clone for ListenerCtx {
    fn clone(&self) -> Self {
        Self {
            event_state: self.event_state.clone(),
            chat_meta: self.chat_meta.clone(),
            profiles: self.profiles.clone(),
            network: self.network.clone(),
            voice: self.voice.clone(),
            persistence: self.persistence.clone(),
            event_broker: self.event_broker.clone(),
            identity: self.identity.clone(),
            join_links: Arc::clone(&self.join_links),
            dag: Arc::clone(&self.dag),
            pending: Arc::clone(&self.pending),
        }
    }
}

/// Spawn an async task that listens for gossip events on a topic.
pub fn spawn_topic_listener<T: TopicHandle + 'static, E: TopicEvents + 'static>(
    events: E,
    topic: T,
    ctx: ListenerCtx,
) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::task::spawn_local(topic_listener_loop(events, topic, ctx));

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(topic_listener_loop(events, topic, ctx));
}

async fn topic_listener_loop<T: TopicHandle, E: TopicEvents>(
    mut events: E,
    topic: T,
    ctx: ListenerCtx,
) {
    while let Some(Ok(gossip_event)) = events.next().await {
        match gossip_event {
            GossipEvent::Received(msg) => {
                process_received_message(&msg.content, msg.sender, &ctx, &topic).await;
            }
            GossipEvent::NeighborUp(id) => {
                let id2 = id;
                willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                    if !c.peers.contains(&id2) {
                        c.peers.push(id2);
                    }
                })
                .await;
                let _ = ctx
                    .event_broker
                    .do_send(willow_actor::Publish(ClientEvent::PeerConnected(id)));
            }
            GossipEvent::NeighborDown(id) => {
                let id2 = id;
                willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                    c.peers.retain(|p| p != &id2);
                })
                .await;
                let _ = ctx
                    .event_broker
                    .do_send(willow_actor::Publish(ClientEvent::PeerDisconnected(id)));
            }
        }
    }
}

// ───── DAG helpers ──────────────────────────────────────────────────────────

/// Apply an event incrementally to materialized state, persist it, and emit
/// client events via the broker.
async fn apply_and_emit(ctx: &ListenerCtx, event: &willow_state::Event) {
    // Apply incrementally to the materialized ServerState.
    let event_clone = event.clone();
    willow_actor::state::mutate(&ctx.event_state, move |state| {
        willow_state::apply_incremental(state, &event_clone);
    })
    .await;

    // Persist event to the event store.
    let _ = ctx.persistence.do_send(persistence_actor::PersistEvent {
        event: event.clone(),
    });

    // Emit client events.
    let client_events = mutations::derive_client_events(event);
    for e in client_events {
        let _ = ctx.event_broker.do_send(willow_actor::Publish(e));
    }
}

/// Try to insert an event into the DAG. On success, apply and emit, then
/// drain any pending events whose `prev` is now satisfied. On chain gap
/// or prev mismatch, buffer the event for later. Duplicates are silently
/// ignored.
async fn try_insert_event(ctx: &ListenerCtx, event: willow_state::Event) {
    let insert_result = {
        let mut dag = ctx.dag.write().unwrap();
        dag.insert(event.clone())
    };

    match insert_result {
        Ok(()) => {
            apply_and_emit(ctx, &event).await;

            // Drain any events that were waiting on this event's hash.
            let resolved = {
                let mut pending = ctx.pending.lock().unwrap();
                pending.resolve(&event.hash)
            };
            // Recursively try inserting resolved events — they may unblock
            // further pending events in turn.
            for resolved_event in resolved {
                // Use Box::pin to allow async recursion without growing the
                // stack frame type infinitely.
                Box::pin(try_insert_event(ctx, resolved_event)).await;
            }
        }
        Err(InsertError::SeqGap { .. }) | Err(InsertError::PrevMismatch { .. }) => {
            let prev = event.prev;
            // Buffer the event, then check for TOCTOU: the predecessor
            // may have been inserted between our failed dag.insert and now.
            let resolved = {
                let mut pending = ctx.pending.lock().unwrap();
                pending.buffer_for_prev(prev, event);
                let predecessor_exists = ctx.dag.read().unwrap().get(&prev).is_some();
                if predecessor_exists {
                    pending.resolve(&prev)
                } else {
                    vec![]
                }
            };
            for resolved_event in resolved {
                Box::pin(try_insert_event(ctx, resolved_event)).await;
            }
        }
        Err(InsertError::Duplicate) => {
            // Already have this event — nothing to do.
        }
        Err(err) => {
            eprintln!("DAG insert error: {err}");
        }
    }
}

// ───── Message processing ───────────────────────────────────────────────────

async fn process_received_message<T: TopicHandle>(
    data: &[u8],
    _sender: EndpointId,
    ctx: &ListenerCtx,
    topic: &T,
) {
    // Try profile broadcast first.
    if let Ok((profile, willow_transport::MessageType::Identity)) =
        willow_transport::unpack_envelope::<willow_identity::UserProfile>(data)
    {
        let peer_id = profile.peer_id;
        let display_name = profile.display_name.clone();
        willow_actor::state::mutate(&ctx.profiles, move |p| {
            p.names.insert(peer_id, display_name);
        })
        .await;
        let _ = ctx
            .event_broker
            .do_send(willow_actor::Publish(ClientEvent::ProfileUpdated {
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
            // Verify the event author matches the signer.
            if event.author != signer {
                return;
            }
            // Track peer.
            let signer2 = signer;
            willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                if !c.peers.contains(&signer2) {
                    c.peers.push(signer2);
                }
            })
            .await;
            // Insert into DAG (handles dedup, ordering, buffering).
            try_insert_event(ctx, event).await;
        }
        crate::ops::WireMessage::SyncBatch { events: batch } => {
            // Track peer.
            let signer2 = signer;
            willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                if !c.peers.contains(&signer2) {
                    c.peers.push(signer2);
                }
            })
            .await;
            let count = batch.len();
            // Insert each event into the DAG. The DAG enforces per-author
            // chain ordering; out-of-order events are buffered automatically.
            for event in batch {
                try_insert_event(ctx, event).await;
            }
            if count > 0 {
                let _ =
                    ctx.event_broker
                        .do_send(willow_actor::Publish(ClientEvent::SyncCompleted {
                            ops_applied: count,
                        }));
            }
        }
        crate::ops::WireMessage::SyncRequest { state_hash, .. } => {
            // Use the DAG to get all events for the responder.
            // If state_hash is ZERO, send everything. Otherwise also send
            // everything (best effort — the receiver will dedup).
            let events: Vec<willow_state::Event> = {
                let dag = ctx.dag.read().unwrap();
                let _ = state_hash; // Acknowledged but not used for filtering yet.
                dag.topological_sort().into_iter().cloned().collect()
            };
            if !events.is_empty() {
                let msg = crate::ops::WireMessage::SyncBatch { events };
                if let Some(data) = crate::ops::pack_wire(&msg, &ctx.identity) {
                    let _ = topic.broadcast(bytes::Bytes::from(data)).await;
                }
            }
        }
        crate::ops::WireMessage::TypingIndicator { channel } => {
            let now = crate::util::current_time_ms();
            willow_actor::state::mutate(&ctx.network, move |n| {
                n.typing_peers.insert(signer, (channel, now));
            })
            .await;
            let signer2 = signer;
            willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                if !c.peers.contains(&signer2) {
                    c.peers.push(signer2);
                }
            })
            .await;
        }
        crate::ops::WireMessage::VoiceJoin {
            channel_id,
            peer_id,
        } => {
            let ch = channel_id.clone();
            willow_actor::state::mutate(&ctx.voice, move |v| {
                v.participants.entry(ch).or_default().insert(peer_id);
            })
            .await;
            let _ = ctx
                .event_broker
                .do_send(willow_actor::Publish(ClientEvent::VoiceJoined {
                    channel_id,
                    peer_id,
                }));
        }
        crate::ops::WireMessage::VoiceLeave {
            channel_id,
            peer_id,
        } => {
            let ch = channel_id.clone();
            willow_actor::state::mutate(&ctx.voice, move |v| {
                if let Some(p) = v.participants.get_mut(&ch) {
                    p.remove(&peer_id);
                }
            })
            .await;
            let _ = ctx
                .event_broker
                .do_send(willow_actor::Publish(ClientEvent::VoiceLeft {
                    channel_id,
                    peer_id,
                }));
        }
        crate::ops::WireMessage::VoiceSignal {
            channel_id,
            target_peer,
            signal,
        } => {
            if target_peer == ctx.identity.endpoint_id() {
                let _ = ctx
                    .event_broker
                    .do_send(willow_actor::Publish(ClientEvent::VoiceSignal {
                        channel_id,
                        from_peer: signer,
                        signal,
                    }));
            }
        }
        crate::ops::WireMessage::JoinRequest { link_id, peer_id } => {
            let should_respond = {
                let mut links = ctx.join_links.lock().unwrap();
                let valid = links
                    .iter_mut()
                    .find(|l| l.link_id == link_id && l.is_valid());
                if let Some(link) = valid {
                    link.used += 1;
                    true
                } else {
                    false
                }
            };
            if should_respond {
                // Generate invite for the requesting peer.
                let invite_result = willow_actor::state::select(&ctx.event_state, move |_es| {
                    // We need the server registry to generate the invite.
                    // For now, just return None — full invite generation needs refactoring.
                    None::<String>
                })
                .await;
                if let Some(invite_data) = invite_result {
                    let msg = crate::ops::WireMessage::JoinResponse {
                        target_peer: peer_id,
                        invite_data,
                    };
                    if let Some(data) = crate::ops::pack_wire(&msg, &ctx.identity) {
                        let _ = topic.broadcast(bytes::Bytes::from(data)).await;
                    }
                }
            }
        }
        crate::ops::WireMessage::JoinResponse {
            target_peer,
            invite_data,
        } => {
            if target_peer == ctx.identity.endpoint_id() {
                let _ = ctx.event_broker.do_send(willow_actor::Publish(
                    ClientEvent::JoinLinkResponse { invite_data },
                ));
            }
        }
        crate::ops::WireMessage::JoinDenied {
            target_peer,
            reason,
        } => {
            if target_peer == ctx.identity.endpoint_id() {
                let _ =
                    ctx.event_broker
                        .do_send(willow_actor::Publish(ClientEvent::JoinLinkDenied {
                            reason,
                        }));
            }
        }
    }
}
