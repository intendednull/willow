use super::*;

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
            storage::save_settings(&storage::NetworkSettings {
                relay_addr: None,
            });
        }

        let listener_ctx = listeners::ListenerCtx {
            event_state: self.event_state_addr.clone(),
            chat_meta: self.chat_meta_addr.clone(),
            profiles: self.profile_state_addr.clone(),
            network: self.network_meta_addr.clone(),
            voice: self.voice_state_addr.clone(),
            persistence: self.persistence_addr.clone(),
            event_broker: self.event_broker.clone(),
            identity: self.identity.clone(),
            join_links: Arc::clone(&self.join_links),
        };

        // Subscribe to the server ops topic.
        let ops_topic_str = ops::SERVER_OPS_TOPIC;
        if let Ok((sender, events)) = network
            .subscribe(willow_network::topic_id(ops_topic_str), vec![])
            .await
        {
            self.topics
                .write()
                .unwrap()
                .insert(ops_topic_str.to_string(), sender.clone());
            listeners::spawn_topic_listener(events, sender, listener_ctx.clone());
        }

        // Subscribe to the global profile broadcast topic.
        let profile_topic_str = ops::PROFILE_TOPIC;
        if let Ok((sender, events)) = network
            .subscribe(willow_network::topic_id(profile_topic_str), vec![])
            .await
        {
            self.topics
                .write()
                .unwrap()
                .insert(profile_topic_str.to_string(), sender.clone());
            listeners::spawn_topic_listener(events, sender, listener_ctx.clone());
        }

        // Subscribe to channel topics from all servers.
        let channel_topics: Vec<String> =
            willow_actor::state::select(&self.server_registry_addr, |reg| {
                reg.servers
                    .values()
                    .flat_map(|entry| entry.topic_map.keys().cloned())
                    .collect()
            })
            .await;

        for topic_str in channel_topics {
            if let Ok((sender, events)) = network
                .subscribe(willow_network::topic_id(&topic_str), vec![])
                .await
            {
                self.topics
                    .write()
                    .unwrap()
                    .insert(topic_str, sender.clone());
                listeners::spawn_topic_listener(events, sender, listener_ctx.clone());
            }
        }

        self.broadcast_profile_via_network();
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
        if let Ok(data) =
            willow_transport::pack_envelope(willow_transport::MessageType::Identity, &profile)
        {
            self.mutation_handle
                .broadcast_on_topic(ops::PROFILE_TOPIC, data);
        }
    }

    pub(crate) async fn request_sync_via_network(&self) {
        let state_hash = self
            .persistence_addr
            .ask(persistence_actor::GetLatestHash)
            .await
            .unwrap_or(willow_state::StateHash::ZERO);
        let msg = ops::WireMessage::SyncRequest {
            state_hash: state_hash.clone(),
            topic: None,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.mutation_handle
                .broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }

        let channel_topics: Vec<String> =
            willow_actor::state::select(&self.server_registry_addr, |reg| {
                reg.servers
                    .values()
                    .flat_map(|entry| entry.topic_map.keys().cloned())
                    .collect()
            })
            .await;
        for topic_str in channel_topics {
            let msg = ops::WireMessage::SyncRequest {
                state_hash: state_hash.clone(),
                topic: Some(topic_str.clone()),
            };
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.mutation_handle.broadcast_on_topic(&topic_str, data);
            }
        }
    }
}
