use super::*;
use willow_network::TopicHandle as _;

/// Spawn the per-connection presence tick driver.
///
/// Advances [`PresenceMeta::now`](state_actors::PresenceMeta) by one
/// every second and refreshes `last_seen` for each reachable peer so
/// their derived state stays `here` while online.
///
/// On native we use `tokio::spawn`; on wasm we use
/// `wasm_bindgen_futures::spawn_local` with `gloo-timers` for sleep.
fn spawn_presence_tick(
    presence_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::PresenceMeta>>,
    chat_meta_addr: willow_actor::Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    tick_once(&presence_meta_addr, &chat_meta_addr).await;
                }
            });
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        wasm_bindgen_futures::spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(1_000).await;
                tick_once(&presence_meta_addr, &chat_meta_addr).await;
            }
        });
    }
}

/// Single tick: advance `now` and stamp `last_seen` for every peer in
/// `chat_meta.peers`. Kept separate so it can be unit-tested by driving
/// it directly without spawning a timer task.
#[cfg(any(test, feature = "test-utils"))]
pub async fn tick_once_for_test(
    presence_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::PresenceMeta>>,
    chat_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
) {
    tick_once(presence_meta_addr, chat_meta_addr).await;
}

async fn tick_once(
    presence_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::PresenceMeta>>,
    chat_meta_addr: &willow_actor::Addr<willow_actor::StateActor<state_actors::ChatMeta>>,
) {
    let reachable = willow_actor::state::select(chat_meta_addr, |c| c.peers.clone()).await;
    willow_actor::state::mutate(presence_meta_addr, move |pm| {
        pm.now = pm.now.saturating_add(1);
        for pid in &reachable {
            pm.last_seen.insert(*pid, pm.now);
        }
    })
    .await;
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
            dag: self.dag_addr.clone(),
            server_registry: self.server_registry_addr.clone(),
            on_neighbor_up: None,
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
            let ops_on_neighbor_up: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                let topics = ops_topics_for_retry
                    .read()
                    .unwrap_or_else(|e| e.into_inner());
                let Some(handle) = topics.get(ops::SERVER_OPS_TOPIC).cloned() else {
                    return;
                };
                drop(topics);
                let msg = ops::WireMessage::SyncRequest {
                    state_hash: willow_state::EventHash::ZERO,
                    topic: None,
                };
                let Some(data) = ops::pack_wire(&msg, &ops_identity_for_retry) else {
                    return;
                };
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
        spawn_presence_tick(self.presence_meta_addr.clone(), self.chat_meta_addr.clone());

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
