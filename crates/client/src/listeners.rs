//! Per-topic listener tasks that stream GossipEvents and mutate state via domain actors.

use std::sync::{Arc, Mutex};

use crate::events::ClientEvent;
use crate::mutations;
use crate::persistence_actor;
use crate::state_actors;
use willow_actor::Addr;
use willow_identity::EndpointId;
use willow_network::traits::TopicHandle;
use willow_network::traits::{GossipEvent, TopicEvents};

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
    pub dag: Addr<willow_actor::StateActor<state_actors::DagState>>,
    pub server_registry: Addr<willow_actor::StateActor<state_actors::ServerRegistry>>,
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
            dag: self.dag.clone(),
            server_registry: self.server_registry.clone(),
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

/// Try to insert an event into the DAG. On success, ManagedDag atomically
/// applies it to state and resolves pending events. On chain gap, the
/// event is buffered. Duplicates are silently ignored.
///
/// Atomicity is guaranteed by ManagedDag::insert_and_apply — DAG insert
/// and state materialization happen in the same call.
async fn try_insert_event(ctx: &ListenerCtx, event: willow_state::Event) {
    // ManagedDag handles insert + apply + pending resolution atomically.
    let (applied, all_applied) = willow_actor::state::mutate(&ctx.dag, move |ds| {
        match ds.managed.insert_and_apply(event) {
            Ok(outcome) => {
                let mut all = Vec::new();
                if let Some(ref ev) = outcome.applied {
                    all.push(ev.clone());
                }
                for r in &outcome.resolved {
                    all.push(r.clone());
                }
                (outcome.applied, all)
            }
            Err(willow_state::InsertError::PrevMismatch {
                author,
                expected,
                got,
            }) => {
                tracing::warn!(
                    %author, %expected, %got,
                    "PrevMismatch: equivocation or conflicting chain — dropping event"
                );
                (None, vec![])
            }
            Err(err) => {
                tracing::warn!("DAG insert error: {err}");
                (None, vec![])
            }
        }
    })
    .await;

    // Sync state and emit side-effects for all applied events.
    if applied.is_some() {
        // Sync state once (ManagedDag already applied all resolved events).
        let state = willow_actor::state::select(&ctx.dag, |ds| ds.managed.state().clone()).await;
        willow_actor::state::mutate(&ctx.event_state, move |es| {
            *es = state;
        })
        .await;

        // Persist and emit for each applied event.
        for ev in &all_applied {
            let _ = ctx
                .persistence
                .do_send(persistence_actor::PersistEvent { event: ev.clone() });
            let client_events = mutations::derive_client_events(ev);
            for e in client_events {
                let _ = ctx.event_broker.do_send(willow_actor::Publish(e));
            }
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
            // Reject oversized batches to prevent memory exhaustion.
            const MAX_SYNC_BATCH_SIZE: usize = 10_000;
            if batch.len() > MAX_SYNC_BATCH_SIZE {
                tracing::warn!(
                    size = batch.len(),
                    "rejecting oversized sync batch (max {})",
                    MAX_SYNC_BATCH_SIZE
                );
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
            let count = batch.len();
            // Insert each event into the DAG. The DAG enforces per-author
            // chain ordering; out-of-order events are buffered automatically.
            for event in batch {
                try_insert_event(ctx, event).await;
            }
            if count > 0 {
                // ManagedDag automatically marks synced when genesis arrives.
                // Just check if we need to signal completion.
                let _is_synced =
                    willow_actor::state::select(&ctx.dag, |ds| ds.managed.is_synced()).await;

                let _ =
                    ctx.event_broker
                        .do_send(willow_actor::Publish(ClientEvent::SyncCompleted {
                            ops_applied: count,
                        }));
            }
        }
        crate::ops::WireMessage::SyncRequest { state_hash, .. } => {
            let _ = state_hash; // Legacy field — can't filter by state hash in DAG model.
                                // TODO: Migrate clients to worker's heads-based sync protocol
                                // (WorkerRequest::Sync { heads }) for efficient delta sync.
                                // For now, send the first 500 events from topological sort.
                                // Receiver will dedup via InsertError::Duplicate.
            let events: Vec<willow_state::Event> = willow_actor::state::select(&ctx.dag, |ds| {
                ds.managed
                    .dag()
                    .topological_sort()
                    .into_iter()
                    .take(500)
                    .cloned()
                    .collect()
            })
            .await;
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
                // Generate invite for the requesting peer using the server registry.
                let server_registry = ctx.server_registry.clone();
                let peer_endpoint = peer_id;
                let invite_result = willow_actor::state::select(&server_registry, move |reg| {
                    let entry = reg.active()?;
                    let pub_key = crate::invite::endpoint_id_to_ed25519_public(&peer_endpoint);
                    crate::invite::generate_invite(
                        &entry.server,
                        &entry.keys,
                        &entry.topic_map,
                        &pub_key,
                    )
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
        // TopicAnnounce is consumed by the relay; clients ignore it.
        crate::ops::WireMessage::TopicAnnounce { .. } => {}
    }
}
