use super::*;
use willow_network::TopicHandle as _;

/// Spawn the per-connection presence tick driver.
///
/// Advances [`PresenceMeta::now`](state_actors::PresenceMeta) by one
/// every second, refreshes `last_seen` for each reachable peer so their
/// derived state stays `here` while online, and sweeps stale entries
/// from [`NetworkMeta::typing_peers`](state_actors::NetworkMeta) older
/// than [`crate::TYPING_INDICATOR_TTL_MS`]. Piggy-backing the typing
/// sweep on the existing presence cadence (1 Hz) avoids a second timer
/// task: the TTL is 5 s so 1 Hz drains entries with at most 1 s of
/// extra dwell, far below the user-visible threshold. Followup to
/// issue #429 ([SEC-V-05]).
///
/// On native we use `tokio::spawn`; on wasm we use
/// `wasm_bindgen_futures::spawn_local` with `gloo-timers` for sleep.
fn spawn_presence_tick(
    presence_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::PresenceMeta>>,
    chat_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
    network_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::NetworkMeta>>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    tick_once(&presence_meta_addr, &chat_meta_addr, &network_meta_addr).await;
                }
            });
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(1_000).await;
                tick_once(&presence_meta_addr, &chat_meta_addr, &network_meta_addr).await;
            }
        });
    }
}

/// Single tick: advance `now`, stamp `last_seen` for every peer in
/// `chat_meta.peers`, and sweep stale typing entries. Kept separate so
/// it can be unit-tested by driving it directly without spawning a
/// timer task.
#[cfg(any(test, feature = "test-utils"))]
pub async fn tick_once_for_test(
    presence_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::PresenceMeta>>,
    chat_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
    network_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::NetworkMeta>>,
) {
    tick_once(presence_meta_addr, chat_meta_addr, network_meta_addr).await;
}

async fn tick_once(
    presence_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::PresenceMeta>>,
    chat_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
    network_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::NetworkMeta>>,
) {
    let reachable = willow_actor::state::select(chat_meta_addr, |c| c.peers.clone()).await;
    willow_actor::state::mutate(presence_meta_addr, move |pm| {
        pm.now = pm.now.saturating_add(1);
        for pid in &reachable {
            pm.last_seen.insert(*pid, pm.now);
        }
    })
    .await;
    // Drain stale typing entries on the same cadence so the map cannot
    // accumulate dead peers indefinitely (#429). The accessor + view
    // layers also filter on read, but only the sweep removes entries.
    let now_ms = crate::util::current_time_ms();
    willow_actor::state::mutate(network_meta_addr, move |n| {
        n.sweep_typing(now_ms, crate::TYPING_INDICATOR_TTL_MS);
    })
    .await;
}

/// Phase 2b — queue tick driver. 1 tick / s. Advances
/// `QueueMeta::now`, then decays `recent_arrivals` entries older than
/// 24 h.
fn spawn_queue_tick(
    queue_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::QueueMeta>>,
) {
    const DECAY_TICKS: crate::presence::Tick = 86_400; // 24 h in seconds
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    queue_tick_once(&queue_meta_addr, DECAY_TICKS).await;
                }
            });
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(1_000).await;
                queue_tick_once(&queue_meta_addr, DECAY_TICKS).await;
            }
        });
    }
}

async fn queue_tick_once(
    queue_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::QueueMeta>>,
    decay_ticks: crate::presence::Tick,
) {
    willow_actor::state::mutate(queue_meta_addr, move |qm| {
        qm.now = qm.now.saturating_add(1);
        qm.decay_arrivals(decay_ticks);
    })
    .await;
}

/// WASM-only: listen for `window.online` + `window.offline` events and
/// route them through `ClientMutations::set_device_online`. Called from
/// [`ClientHandle::connect`] once per connection.
#[cfg(target_arch = "wasm32")]
fn spawn_wasm_online_listener<N: willow_network::Network>(
    mutations: crate::mutations::ClientMutations<N>,
) {
    let Some(window) = web_sys::window() else {
        return;
    };
    // Prime from `navigator.onLine`.
    let online_now = window.navigator().on_line();
    {
        let mutations = mutations.clone();
        wasm_bindgen_futures::spawn_local(async move {
            mutations.set_device_online(online_now).await;
        });
    }
    // Online listener.
    let online_mutations = mutations.clone();
    let online_cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        let mutations = online_mutations.clone();
        wasm_bindgen_futures::spawn_local(async move {
            mutations.set_device_online(true).await;
        });
    });
    // Offline listener.
    let offline_mutations = mutations;
    let offline_cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        let mutations = offline_mutations.clone();
        wasm_bindgen_futures::spawn_local(async move {
            mutations.set_device_online(false).await;
        });
    });
    use wasm_bindgen::JsCast;
    let _ = window.add_event_listener_with_callback("online", online_cb.as_ref().unchecked_ref());
    let _ = window.add_event_listener_with_callback("offline", offline_cb.as_ref().unchecked_ref());
    online_cb.forget();
    offline_cb.forget();
}

impl<N: willow_network::Network> ClientHandle<N> {
    /// Connect to the P2P network.
    pub async fn connect(
        &mut self,
        network: N,
    ) -> willow_actor::Addr<willow_actor::Broker<ClientEvent>> {
        let network = Arc::new(network);
        self.network = Some(Arc::clone(&network));

        if self.persistence_enabled {
            // Save settings (relay addr is from config, already stored).
            storage::save_settings(&storage::NetworkSettings { relay_addr: None });
        }

        // One stable `stream_generation` per connect() call, shared by every
        // listener context built below (ops + profile + channel). The gossip
        // `SyncRequestV2` responder stamps it on every `HistorySyncComplete`
        // marker it emits, so repeated serves to a reconnecting peer dedup on
        // the receiver's `(provider, stream_generation)` table instead of
        // re-emitting `HistorySynced`. `uuid` is WASM-safe (workspace `js`
        // feature); `as_u64_pair().0` extracts a random `u64` without `std` RNG.
        let history_stream_generation = uuid::Uuid::new_v4().as_u64_pair().0;

        let listener_ctx = listeners::ListenerCtx {
            event_state: self.event_state_addr.clone(),
            chat_meta: self.chat_meta_addr.clone(),
            profiles: self.profile_state_addr.clone(),
            network: self.network_meta_addr.clone(),
            voice: self.voice_state_addr.clone(),
            persistence: self.persistence_addr.clone(),
            persistence_enabled: self.persistence_enabled,
            event_broker: self.event_broker.clone(),
            identity: self.identity.clone(),
            join_links: Arc::clone(&self.join_links),
            pending_joins: Arc::clone(&self.pending_joins),
            dag: self.dag_addr.clone(),
            server_registry: self.server_registry_addr.clone(),
            on_neighbor_up: None,
            history_stream_generation,
        };

        // Subscribe to the server ops topic.
        // On NeighborUp, re-send a SyncRequest so peers that join the gossip
        // mesh after the initial SyncRequest (sent at the end of connect())
        // can still trigger a full state sync. This is critical for
        // cross-browser scenarios where the relay connection is established
        // after accept_invite() has already sent its SyncRequest.
        let ops_topic_str = ops::SERVER_OPS_TOPIC;
        let bootstrap = self.bootstrap_peers.clone();
        if let Ok((sender, events)) = network
            .subscribe(willow_network::topic_id(ops_topic_str), bootstrap)
            .await
        {
            self.topics
                .write()
                .unwrap()
                .insert(ops_topic_str.to_string(), sender.clone());
            let ops_topics_for_retry = Arc::clone(&self.topics);
            let ops_identity_for_retry = self.identity.clone();
            let ops_dag_for_retry = self.dag_addr.clone();
            // On NeighborUp re-emit the heads-based `SyncRequestV2` (PR 4
            // cutover) so a peer that joins the mesh after the initial join
            // request still triggers a delta sync. Heads + server_id are read
            // from the DAG inside the spawned task because this callback is
            // synchronous; the broadcast was already async-spawned before.
            let ops_on_neighbor_up: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                let topics = ops_topics_for_retry
                    .read()
                    .unwrap_or_else(|e| e.into_inner());
                let Some(handle) = topics.get(ops::SERVER_OPS_TOPIC).cloned() else {
                    return;
                };
                drop(topics);
                let identity = ops_identity_for_retry.clone();
                let dag = ops_dag_for_retry.clone();
                let build_and_broadcast = async move {
                    let heads =
                        willow_actor::state::select(&dag, |ds| ds.managed.dag().heads_summary())
                            .await;
                    let server_id = willow_actor::state::select(&dag, |ds| {
                        ds.managed.dag().server_id().unwrap_or_default()
                    })
                    .await;
                    let msg = ops::WireMessage::SyncRequestV2 {
                        request_id: uuid::Uuid::new_v4().to_string(),
                        heads,
                        filter: willow_common::SyncFilter {
                            server_id,
                            channels: None,
                            authors: None,
                            event_kinds: None,
                            since_ms: None,
                        },
                    };
                    if let Some(data) = ops::pack_wire(&msg, &identity) {
                        handle.broadcast(bytes::Bytes::from(data)).await.ok();
                    }
                };
                #[cfg(not(target_arch = "wasm32"))]
                {
                    if let Ok(rt) = tokio::runtime::Handle::try_current() {
                        rt.spawn(build_and_broadcast);
                    }
                }
                #[cfg(target_arch = "wasm32")]
                {
                    wasm_bindgen_futures::spawn_local(build_and_broadcast);
                }
            });
            let ops_listener_ctx = listeners::ListenerCtx {
                on_neighbor_up: Some(ops_on_neighbor_up),
                ..listener_ctx.clone()
            };
            listeners::spawn_topic_listener(events, sender, ops_listener_ctx);
        }

        // Subscribe to the global profile broadcast topic.
        // On NeighborUp, re-broadcast the local profile so that late-joining
        // peers receive our display name even if they missed the initial burst.
        let profile_topic_str = ops::PROFILE_TOPIC;
        let bootstrap = self.bootstrap_peers.clone();
        if let Ok((sender, events)) = network
            .subscribe(willow_network::topic_id(profile_topic_str), bootstrap)
            .await
        {
            self.topics
                .write()
                .unwrap()
                .insert(profile_topic_str.to_string(), sender.clone());
            let identity_for_cb = self.identity.clone();
            let topics_for_cb = Arc::clone(&self.topics);
            let profile_ctx = listeners::ListenerCtx {
                on_neighbor_up: Some(Arc::new(move || {
                    let saved = crate::storage::load_profile().unwrap_or_default();
                    if saved.display_name.is_empty() {
                        return;
                    }
                    let profile = willow_identity::UserProfile::new(
                        identity_for_cb.endpoint_id(),
                        saved.display_name,
                    );
                    let Ok(data) = willow_identity::pack_profile(&profile, &identity_for_cb) else {
                        return;
                    };
                    let topics = topics_for_cb.read().unwrap_or_else(|e| e.into_inner());
                    let Some(handle) = topics.get(ops::PROFILE_TOPIC).cloned() else {
                        return;
                    };
                    drop(topics);
                    let bytes = bytes::Bytes::from(data);
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if let Ok(rt) = tokio::runtime::Handle::try_current() {
                            rt.spawn(async move {
                                handle.broadcast(bytes).await.ok();
                            });
                        }
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        wasm_bindgen_futures::spawn_local(async move {
                            handle.broadcast(bytes).await.ok();
                        });
                    }
                })),
                ..listener_ctx.clone()
            };
            listeners::spawn_topic_listener(events, sender, profile_ctx);
        }

        // Subscribe to channel topics from all servers.
        // Derive topic strings from event_state channels + server registry IDs.
        let channel_topics: Vec<String> = {
            let es = willow_actor::state::get(&self.event_state_addr).await;
            willow_actor::state::select(&self.server_registry_addr, move |reg| {
                reg.servers
                    .values()
                    .flat_map(|entry| entry.channel_topics(&es))
                    .collect()
            })
            .await
        };

        for topic_str in &channel_topics {
            let bootstrap = self.bootstrap_peers.clone();
            if let Ok((sender, events)) = network
                .subscribe(willow_network::topic_id(topic_str), bootstrap)
                .await
            {
                self.topics
                    .write()
                    .unwrap()
                    .insert(topic_str.clone(), sender.clone());
                listeners::spawn_topic_listener(events, sender, listener_ctx.clone());
            }
        }

        // Announce channel topics so the relay can subscribe and serve
        // as bootstrap for those topics.
        if !channel_topics.is_empty() {
            let msg = ops::WireMessage::TopicAnnounce {
                topics: channel_topics,
            };
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.mutation_handle
                    .broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
            }
        }

        // Presence tick driver (phase 1e). 1 tick = 1 s. Advances the
        // PresenceMeta `now` counter and refreshes `last_seen` for every
        // currently-reachable peer so their presence state stays `here`
        // while reachable. When a peer drops out of `chat_meta.peers`
        // their last_seen stays frozen so elapsed = now - last_seen
        // climbs past the idle / gone thresholds in due course.
        spawn_presence_tick(
            self.presence_meta_addr.clone(),
            self.chat_meta_addr.clone(),
            self.network_meta_addr.clone(),
        );

        // Sync-queue tick driver (Phase 2b). Advances `QueueMeta::now`
        // and decays `recent_arrivals` entries older than 24 h so the
        // sync-queue screen's Recent section rolls forward even when
        // nothing else mutates the actor.
        spawn_queue_tick(self.queue_meta_addr.clone());

        // WASM-only: bridge `window.online` / `window.offline` events to
        // `QueueMeta::set_device_online`. Native iroh doesn't expose a
        // connectivity probe yet; `Network::device_online` default-stays
        // `true` there.
        #[cfg(target_arch = "wasm32")]
        spawn_wasm_online_listener(self.mutation_handle.clone());

        self.broadcast_profile_via_network();
        // Also announce via SERVER_OPS_TOPIC for peers that have a sync path
        // but may not have received the PROFILE_TOPIC broadcast.
        {
            let saved = storage::load_profile().unwrap_or_default();
            if !saved.display_name.is_empty() {
                let announce = ops::WireMessage::ProfileAnnounce {
                    display_name: saved.display_name,
                };
                if let Some(data) = ops::pack_wire(&announce, &self.identity) {
                    self.mutation_handle
                        .broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
                }
            }
        }
        self.request_sync_via_network().await;

        self.mutation_handle.set_connected(true).await;
        self.event_broker.clone()
    }

    pub(crate) fn broadcast_profile_via_network(&self) {
        let saved = storage::load_profile().unwrap_or_default();
        if saved.display_name.is_empty() {
            return;
        }
        let profile =
            willow_identity::UserProfile::new(self.identity.endpoint_id(), saved.display_name);
        if let Ok(data) = willow_identity::pack_profile(&profile, &self.identity) {
            self.mutation_handle
                .broadcast_on_topic(ops::PROFILE_TOPIC, data);
        }
    }

    pub(crate) async fn request_sync_via_network(&self) {
        let state_hash = willow_state::EventHash::ZERO;
        let msg = ops::WireMessage::SyncRequest {
            state_hash,
            topic: None,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.mutation_handle
                .broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }

        let channel_topics: Vec<String> = {
            let es = willow_actor::state::get(&self.event_state_addr).await;
            willow_actor::state::select(&self.server_registry_addr, move |reg| {
                reg.servers
                    .values()
                    .flat_map(|entry| entry.channel_topics(&es))
                    .collect()
            })
            .await
        };
        for topic_str in channel_topics {
            let msg = ops::WireMessage::SyncRequest {
                state_hash,
                topic: Some(topic_str.clone()),
            };
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.mutation_handle.broadcast_on_topic(&topic_str, data);
            }
        }
    }
}
