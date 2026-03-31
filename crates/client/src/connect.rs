use super::*;
use crate::client_actor::{mutate_state, read_state};

impl<N: willow_network::Network> ClientHandle<N> {
    /// Connect to the P2P network.
    pub async fn connect(&mut self, network: N) -> futures_mpsc::UnboundedReceiver<ClientEvent> {
        let network = Arc::new(network);
        self.network = Some(Arc::clone(&network));

        let (event_tx, event_rx) = futures_mpsc::unbounded();
        self.event_tx = event_tx.clone();

        let (persistence, relay_addr) = read_state(&self.state_addr, |s| {
            (s.config.persistence, s.config.relay_addr.clone())
        })
        .await;
        if persistence {
            storage::save_settings(&storage::NetworkSettings { relay_addr });
        }

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
            listeners::spawn_topic_listener(
                events,
                sender,
                self.state_addr.clone(),
                event_tx.clone(),
            );
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
            listeners::spawn_topic_listener(
                events,
                sender,
                self.state_addr.clone(),
                event_tx.clone(),
            );
        }

        // Subscribe to channel topics from all servers.
        let channel_topics: Vec<String> = read_state(&self.state_addr, |s| {
            s.state
                .servers
                .values()
                .flat_map(|ctx| ctx.topic_map.keys().cloned())
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
                listeners::spawn_topic_listener(
                    events,
                    sender,
                    self.state_addr.clone(),
                    event_tx.clone(),
                );
            }
        }

        self.broadcast_profile_via_network();
        self.request_sync_via_network().await;

        mutate_state(&self.state_addr, |s| s.connected = true).await;
        event_rx
    }

    /// Broadcast our profile to peers via the profile topic.
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
            self.broadcast_on_topic(ops::PROFILE_TOPIC, data);
        }
    }

    /// Request sync from peers on the server ops topic.
    pub(crate) async fn request_sync_via_network(&self) {
        let state_hash = read_state(&self.state_addr, |s| s.state.event_store.latest_hash()).await;
        let msg = ops::WireMessage::SyncRequest {
            state_hash: state_hash.clone(),
            topic: None,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }

        let channel_topics: Vec<String> = read_state(&self.state_addr, |s| {
            s.state
                .servers
                .values()
                .flat_map(|ctx| ctx.topic_map.keys().cloned())
                .collect()
        })
        .await;
        for topic_str in channel_topics {
            let msg = ops::WireMessage::SyncRequest {
                state_hash: state_hash.clone(),
                topic: Some(topic_str.clone()),
            };
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.broadcast_on_topic(&topic_str, data);
            }
        }
    }
}
