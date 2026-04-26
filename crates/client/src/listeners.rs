//! Per-topic listener tasks that stream GossipEvents and mutate state via domain actors.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::events::ClientEvent;
use crate::mutations;
use crate::persistence_actor;
use crate::state_actors;
use willow_actor::Addr;
use willow_common::SYNC_BATCH_LIMIT;
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
    /// Whether persistence to disk is enabled.
    pub persistence_enabled: bool,
    pub event_broker: Addr<willow_actor::Broker<ClientEvent>>,
    pub identity: willow_identity::Identity,
    // state: lock-ok — mirrors `ClientHandle.join_links`; actor migration
    // tracked in docs/specs/2026-04-26-state-management-model-design.md § F4.
    pub join_links: Arc<Mutex<Vec<crate::ops::JoinLink>>>,
    pub dag: Addr<willow_actor::StateActor<state_actors::DagState>>,
    pub server_registry: Addr<willow_actor::StateActor<state_actors::ServerRegistry>>,
    /// Optional callback invoked on `NeighborUp`. Used by the profile topic
    /// listener to re-broadcast the local profile when a new peer joins the
    /// gossip mesh — ensuring late-arriving peers see the display name even
    /// when the initial broadcast happened before the link was established.
    pub on_neighbor_up: Option<Arc<dyn Fn() + Send + Sync>>,
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
            persistence_enabled: self.persistence_enabled,
            event_broker: self.event_broker.clone(),
            identity: self.identity.clone(),
            join_links: Arc::clone(&self.join_links),
            dag: self.dag.clone(),
            server_registry: self.server_registry.clone(),
            on_neighbor_up: self.on_neighbor_up.clone(),
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
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::PeerConnected(id)))
                    .ok();
                // Re-broadcast local profile so the newly joined peer gets
                // our display name even if they missed the initial broadcast.
                if let Some(cb) = &ctx.on_neighbor_up {
                    cb();
                }
            }
            GossipEvent::NeighborDown(id) => {
                let id2 = id;
                willow_actor::state::mutate(&ctx.chat_meta, move |c| {
                    c.peers.retain(|p| p != &id2);
                })
                .await;
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::PeerDisconnected(id)))
                    .ok();
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
            ctx.persistence
                .do_send(persistence_actor::PersistEvent { event: ev.clone() })
                .ok();
            let client_events = mutations::derive_client_events(ev);
            for e in client_events {
                ctx.event_broker.do_send(willow_actor::Publish(e)).ok();
            }
        }
        // Persist the state snapshot so messages survive a page reload
        // without requiring a network sync round-trip. Use a direct synchronous
        // storage write so the snapshot is on disk before this task yields —
        // fire-and-forget actor messages may be delayed past a page reload.
        // Key by registry UUID (not ServerState.server_id which is the genesis
        // hash) so that load_server_state() can find the saved snapshot.
        if ctx.persistence_enabled {
            let snapshot_state =
                willow_actor::state::select(&ctx.dag, |ds| ds.managed.state().clone()).await;
            let registry_id =
                willow_actor::state::select(&ctx.server_registry, |reg| reg.active_server.clone())
                    .await;
            if let Some(id) = registry_id {
                crate::storage::save_server_state(&id, &snapshot_state);
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
    // Try profile broadcast first. Profile envelopes are Ed25519-signed
    // and we reject anything where the signer doesn't match the claimed
    // `peer_id` — otherwise any peer could spoof another peer's display
    // name. See issue #145.
    match willow_identity::unpack_profile(data) {
        Ok(profile) => {
            let peer_id = profile.peer_id;
            let display_name = profile.display_name.clone();
            willow_actor::state::mutate(&ctx.profiles, move |p| {
                p.names.insert(peer_id, display_name);
            })
            .await;
            ctx.event_broker
                .do_send(willow_actor::Publish(ClientEvent::ProfileUpdated {
                    peer_id: profile.peer_id,
                    display_name: profile.display_name,
                }))
                .ok();
            return;
        }
        Err(willow_identity::IdentityError::PeerMismatch { claimed, signer }) => {
            tracing::warn!(
                %claimed,
                %signer,
                "rejecting spoofed profile broadcast: signer does not match claimed peer_id"
            );
            return;
        }
        Err(_) => {
            // Not a profile envelope — fall through to the WireMessage
            // path below.
        }
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
            // Bound matches the producer side in `willow-storage`; the
            // shared constant lives in `willow_common::SYNC_BATCH_LIMIT`.
            if batch.len() > SYNC_BATCH_LIMIT {
                tracing::warn!(
                    size = batch.len(),
                    "rejecting oversized sync batch (max {})",
                    SYNC_BATCH_LIMIT
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

                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::SyncCompleted {
                        ops_applied: count,
                    }))
                    .ok();
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
                    topic.broadcast(bytes::Bytes::from(data)).await.ok();
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
            ctx.event_broker
                .do_send(willow_actor::Publish(ClientEvent::VoiceJoined {
                    channel_id,
                    peer_id,
                }))
                .ok();
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
            ctx.event_broker
                .do_send(willow_actor::Publish(ClientEvent::VoiceLeft {
                    channel_id,
                    peer_id,
                }))
                .ok();
        }
        crate::ops::WireMessage::VoiceSignal {
            channel_id,
            target_peer,
            signal,
        } => {
            if target_peer == ctx.identity.endpoint_id() {
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::VoiceSignal {
                        channel_id,
                        from_peer: signer,
                        signal,
                    }))
                    .ok();
            }
        }
        crate::ops::WireMessage::JoinRequest { link_id, peer_id } => {
            // Security: the wire-supplied `peer_id` is attacker-controlled
            // and must never be trusted as a grant target. The only
            // authenticated peer identity we have is `signer`, which was
            // verified against the envelope's Ed25519 signature by
            // `unpack_wire`. Reject any request where the two disagree,
            // matching the ProfileAnnounce spoof guard above. See issue
            // #239 / SEC-A-03.
            if peer_id != signer {
                tracing::warn!(
                    %peer_id,
                    %signer,
                    "rejecting JoinRequest: wire-supplied peer_id does not match verified signer"
                );
                return;
            }
            let _ = peer_id; // the verified `signer` is the grant target below
            let should_respond = {
                let mut links = ctx.join_links.lock();
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
                // Generate invite for the requesting peer using event_state + registry.
                let server_registry = ctx.server_registry.clone();
                let event_state = ctx.event_state.clone();
                // Use the verified signer as the grant target — never the
                // attacker-supplied `peer_id` from the wire message.
                let peer_endpoint = signer;

                // Get server info from registry.
                let reg_info = willow_actor::state::select(&server_registry, move |reg| {
                    reg.active().map(|entry| {
                        (
                            entry.name.clone(),
                            entry.server_id.clone(),
                            entry.keys.clone(),
                        )
                    })
                })
                .await;

                let invite_result = if let Some((server_name, server_id, keys)) = reg_info {
                    // Build topic_names and get owner from event_state.
                    let sid = server_id.clone();
                    let fallback_id = ctx.identity.endpoint_id();
                    let (topic_names, genesis_author) =
                        willow_actor::state::select(&event_state, move |es| {
                            let names: std::collections::HashMap<String, String> = es
                                .channels
                                .values()
                                .map(|ch| {
                                    let topic = crate::util::make_topic(&sid, &ch.name);
                                    (topic, ch.name.clone())
                                })
                                .collect();
                            // Same correctness fix as in `generate_invite`
                            // (issue #241): pick the authoritative genesis
                            // author rather than an arbitrary admin.
                            let author = es.genesis_author.unwrap_or(fallback_id);
                            (names, author)
                        })
                        .await;

                    let pub_key = crate::invite::endpoint_id_to_ed25519_public(&peer_endpoint);
                    crate::invite::generate_invite(
                        &server_name,
                        &server_id,
                        genesis_author,
                        &keys,
                        &topic_names,
                        &pub_key,
                    )
                } else {
                    None
                };
                if let Some(invite_data) = invite_result {
                    let msg = crate::ops::WireMessage::JoinResponse {
                        target_peer: peer_endpoint,
                        invite_data,
                    };
                    if let Some(data) = crate::ops::pack_wire(&msg, &ctx.identity) {
                        topic.broadcast(bytes::Bytes::from(data)).await.ok();
                    }

                    // Grant SendMessages permission to the joining peer so
                    // they can send messages, reactions, etc. This creates
                    // a signed event in the inviter's DAG and broadcasts it
                    // so all peers (including the new joiner) learn that
                    // the new member has send permission.
                    let identity = ctx.identity.clone();
                    // Grant permission to the authenticated signer — never
                    // the wire-supplied peer_id (issue #239 / SEC-A-03).
                    let granted_peer = peer_endpoint;
                    let ts = crate::util::current_time_ms();
                    let grant_event = willow_actor::state::mutate(&ctx.dag, move |ds| {
                        ds.managed
                            .create_and_insert(
                                &identity,
                                willow_state::EventKind::GrantPermission {
                                    peer_id: granted_peer,
                                    permission: willow_state::Permission::SendMessages,
                                },
                                ts,
                            )
                            .ok()
                    })
                    .await;
                    if let Some(event) = grant_event {
                        // Sync event_state mirror from the DAG.
                        let new_state =
                            willow_actor::state::select(&ctx.dag, |ds| ds.managed.state().clone())
                                .await;
                        willow_actor::state::mutate(&ctx.event_state, move |es| {
                            *es = new_state;
                        })
                        .await;
                        // Persist.
                        ctx.persistence
                            .do_send(crate::persistence_actor::PersistEvent {
                                event: event.clone(),
                            })
                            .ok();
                        // Broadcast to other peers.
                        if let Some(data) = crate::ops::pack_wire(
                            &crate::ops::WireMessage::Event(event),
                            &ctx.identity,
                        ) {
                            topic.broadcast(bytes::Bytes::from(data)).await.ok();
                        }
                    }
                }
            }
        }
        crate::ops::WireMessage::JoinResponse {
            target_peer,
            invite_data,
        } => {
            if target_peer == ctx.identity.endpoint_id() {
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::JoinLinkResponse {
                        invite_data,
                    }))
                    .ok();
            }
        }
        crate::ops::WireMessage::JoinDenied {
            target_peer,
            reason,
        } => {
            if target_peer == ctx.identity.endpoint_id() {
                ctx.event_broker
                    .do_send(willow_actor::Publish(ClientEvent::JoinLinkDenied {
                        reason,
                    }))
                    .ok();
            }
        }
        // TopicAnnounce is consumed by the relay; clients ignore it.
        crate::ops::WireMessage::TopicAnnounce { .. } => {}
        // Update the local ProfileState with the sender's display name.
        // Sent on SERVER_OPS_TOPIC so delivery is guaranteed via the sync path.
        crate::ops::WireMessage::ProfileAnnounce { display_name } => {
            let peer_id = signer;
            let name = display_name.clone();
            willow_actor::state::mutate(&ctx.profiles, move |p| {
                p.names.insert(peer_id, name);
            })
            .await;
            ctx.event_broker
                .do_send(willow_actor::Publish(ClientEvent::ProfileUpdated {
                    peer_id,
                    display_name,
                }))
                .ok();
        }
        // Worker messages travel on the _willow_workers topic and are never
        // delivered to the client's server-ops listener.
        crate::ops::WireMessage::Worker(_) => {}
    }
}

#[cfg(test)]
mod tests {
    //! Listener tests for the JoinRequest signer guard (SEC-A-03 / #239).
    use super::*;
    use crate::test_client;
    use std::sync::Arc;
    use willow_network::Network;

    /// A `JoinRequest` whose wire-supplied `peer_id` disagrees with the
    /// Ed25519 `signer` must be rejected. Concretely: no `GrantPermission`
    /// event should be emitted for either identity, and no `JoinResponse`
    /// should be broadcast.
    ///
    /// Without the fix, the listener trusts `peer_id` from the wire and
    /// hands the authoritative invite + `SendMessages` grant to whichever
    /// identity the attacker names — even though the packet was signed by
    /// someone else entirely.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_request_with_mismatched_peer_id_is_rejected() {
        let (client, _rx) = test_client();

        // Inject a join link so the listener would otherwise be willing
        // to respond. Expires far in the future.
        let link_id = "test-link-id".to_string();
        {
            let mut links = client.join_links.lock();
            links.push(crate::ops::JoinLink {
                link_id: link_id.clone(),
                server_id: "unused".into(),
                max_uses: 10,
                used: 0,
                active: true,
                expires_at: None,
                created_at: 0,
            });
        }

        // Construct a MemNetwork + topic handle to satisfy the
        // TopicHandle generic expected by process_received_message.
        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-topic"), vec![])
            .await
            .expect("subscribe must succeed in test");

        // Build a ListenerCtx from the ClientHandle.
        let ctx = ListenerCtx {
            event_state: client.event_state_addr.clone(),
            chat_meta: client.chat_meta_addr.clone(),
            profiles: client.profile_state_addr.clone(),
            network: client.network_meta_addr.clone(),
            voice: client.voice_state_addr.clone(),
            persistence: client.persistence_addr.clone(),
            persistence_enabled: false,
            event_broker: client.event_broker.clone(),
            identity: client.identity.clone(),
            join_links: Arc::clone(&client.join_links),
            dag: client.dag_addr.clone(),
            server_registry: client.server_registry_addr.clone(),
            on_neighbor_up: None,
        };

        // Mallory signs the packet, but claims it's from Bob. Neither
        // identity is the inviter/client.
        let mallory = willow_identity::Identity::generate();
        let bob_id = willow_identity::Identity::generate().endpoint_id();
        let msg = crate::ops::WireMessage::JoinRequest {
            link_id: link_id.clone(),
            peer_id: bob_id,
        };
        let bytes = crate::ops::pack_wire(&msg, &mallory).expect("pack_wire must succeed");

        let sender = mallory.endpoint_id();
        process_received_message(&bytes, sender, &ctx, &topic_handle).await;

        // Give any actor tasks a moment to settle.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Neither Bob (the attacker-named target) nor Mallory (the real
        // signer) should have been granted SendMessages — the listener
        // must drop the packet entirely on signer/peer_id mismatch.
        let mallory_id = mallory.endpoint_id();
        let (bob_has, mallory_has) =
            willow_actor::state::select(&client.event_state_addr, move |es| {
                (
                    es.has_permission(&bob_id, &willow_state::Permission::SendMessages),
                    es.has_permission(&mallory_id, &willow_state::Permission::SendMessages),
                )
            })
            .await;
        assert!(
            !bob_has,
            "spoofed peer_id must not receive SendMessages grant"
        );
        assert!(
            !mallory_has,
            "signer must not receive SendMessages grant when peer_id mismatches"
        );

        // The join link's `used` counter must not have been incremented
        // since the request was rejected before the link was consumed.
        let used = {
            let links = client.join_links.lock();
            links
                .iter()
                .find(|l| l.link_id == link_id)
                .map(|l| l.used)
                .expect("link must still be present")
        };
        assert_eq!(used, 0, "rejected requests must not consume the join link");
    }

    /// When signer and wire peer_id agree, the listener grants
    /// SendMessages to that verified peer. This is the positive
    /// counterpart of the mismatch test above.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn join_request_with_matching_signer_grants_send_messages() {
        let (client, _rx) = test_client();

        let link_id = "match-link".to_string();
        {
            let mut links = client.join_links.lock();
            links.push(crate::ops::JoinLink {
                link_id: link_id.clone(),
                server_id: "unused".into(),
                max_uses: 10,
                used: 0,
                active: true,
                expires_at: None,
                created_at: 0,
            });
        }

        let hub = willow_network::mem::MemHub::new();
        let net = willow_network::mem::MemNetwork::new(&hub);
        let (topic_handle, _events) = net
            .subscribe(willow_network::topic_id("unit-test-topic-2"), vec![])
            .await
            .expect("subscribe must succeed");

        let ctx = ListenerCtx {
            event_state: client.event_state_addr.clone(),
            chat_meta: client.chat_meta_addr.clone(),
            profiles: client.profile_state_addr.clone(),
            network: client.network_meta_addr.clone(),
            voice: client.voice_state_addr.clone(),
            persistence: client.persistence_addr.clone(),
            persistence_enabled: false,
            event_broker: client.event_broker.clone(),
            identity: client.identity.clone(),
            join_links: Arc::clone(&client.join_links),
            dag: client.dag_addr.clone(),
            server_registry: client.server_registry_addr.clone(),
            on_neighbor_up: None,
        };

        let bob = willow_identity::Identity::generate();
        let bob_id = bob.endpoint_id();
        let msg = crate::ops::WireMessage::JoinRequest {
            link_id: link_id.clone(),
            peer_id: bob_id,
        };
        let bytes = crate::ops::pack_wire(&msg, &bob).expect("pack_wire must succeed");

        process_received_message(&bytes, bob_id, &ctx, &topic_handle).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let bob_has = willow_actor::state::select(&client.event_state_addr, move |es| {
            es.has_permission(&bob_id, &willow_state::Permission::SendMessages)
        })
        .await;
        assert!(
            bob_has,
            "verified signer must be granted SendMessages via join link"
        );
    }
}
