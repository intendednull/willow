use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    pub async fn generate_invite(
        &self,
        recipient_peer_id: &willow_identity::EndpointId,
    ) -> anyhow::Result<String> {
        let pub_key = invite::endpoint_id_to_ed25519_public(recipient_peer_id);
        willow_actor::state::select(&self.server_registry_addr, move |reg| {
            let entry = reg
                .active()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;
            invite::generate_invite(&entry.server, &entry.keys, &entry.topic_map, &pub_key)
                .ok_or_else(|| anyhow::anyhow!("invite generation failed"))
        })
        .await
    }

    pub async fn accept_invite(&self, code: &str) -> anyhow::Result<()> {
        let code = code.to_string();
        let identity = self.identity.clone();
        let accepted = invite::accept_invite(&code, &identity)
            .ok_or_else(|| anyhow::anyhow!("invalid invite code or not for us"))?;

        let server_id = accepted.server_id.clone();
        let owner = accepted.owner;
        let first_channel_name = accepted
            .channel_keys
            .values()
            .next()
            .map(|(name, _)| name.clone());

        // Update server registry.
        let channel_topics = willow_actor::state::mutate(
            &self.server_registry_addr,
            move |reg| -> anyhow::Result<Vec<String>> {
                if let Some(entry) = reg.servers.get_mut(&server_id) {
                    for (topic, (name, key)) in &accepted.channel_keys {
                        entry.keys.insert(topic.clone(), key.clone());
                        if !entry.topic_map.contains_key(topic) {
                            entry.topic_map.insert(
                                topic.clone(),
                                (name.clone(), willow_channel::ChannelId::new()),
                            );
                        }
                    }
                } else {
                    let mut server =
                        willow_channel::Server::new(&accepted.server_name, accepted.owner);
                    server.id = willow_channel::ServerId(
                        uuid::Uuid::parse_str(&server_id)
                            .unwrap_or_else(|_| uuid::Uuid::new_v4()),
                    );
                    let mut topic_map = HashMap::new();
                    let mut keys = HashMap::new();
                    for (topic, (name, key)) in &accepted.channel_keys {
                        let ch_id = server
                            .create_channel(name, willow_channel::ChannelKind::Text)
                            .unwrap_or_else(|_| willow_channel::ChannelId::new());
                        server.set_channel_key(ch_id.clone(), key.clone());
                        keys.insert(topic.clone(), key.clone());
                        topic_map.insert(topic.clone(), (name.clone(), ch_id));
                    }
                    reg.servers.insert(
                        server_id.clone(),
                        state_actors::ServerEntry {
                            server,
                            name: accepted.server_name.clone(),
                            topic_map,
                            keys,
                            unread: HashMap::new(),
                        },
                    );
                }
                reg.active_server = Some(server_id.clone());
                let topics = reg
                    .servers
                    .get(&server_id)
                    .map(|e| e.topic_map.keys().cloned().collect())
                    .unwrap_or_default();
                Ok(topics)
            },
        )
        .await?;

        // Initialize event state for the new server.
        let sid = accepted.server_id.clone();
        willow_actor::state::mutate(&self.event_state_addr, move |es| {
            *es = willow_state::ServerState::new(&sid, "", owner);
            es.owner = owner;
            es.members
                .entry(owner)
                .or_insert_with(|| willow_state::Member {
                    peer_id: owner,
                    roles: std::collections::HashSet::new(),
                    display_name: None,
                });
        })
        .await;

        // Set current channel.
        if let Some(name) = &first_channel_name {
            self.mutation_handle.switch_channel(name).await;
        }

        // Open event store on persistence actor.
        let _ = self
            .persistence_addr
            .do_send(persistence_actor::OpenEventStore {
                server_id: accepted.server_id.clone(),
            });

        // Request sync.
        let msg = ops::WireMessage::SyncRequest {
            state_hash: willow_state::StateHash::ZERO,
            topic: None,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.mutation_handle
                .broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
        for topic_str in channel_topics {
            let msg = ops::WireMessage::SyncRequest {
                state_hash: willow_state::StateHash::ZERO,
                topic: Some(topic_str.clone()),
            };
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.mutation_handle.broadcast_on_topic(&topic_str, data);
            }
        }

        Ok(())
    }

    pub fn publish(&self, topic: &str, data: Vec<u8>) {
        self.mutation_handle.broadcast_on_topic(topic, data);
    }

    pub fn send_join_request(&self, link_id: &str) {
        let msg = ops::WireMessage::JoinRequest {
            link_id: link_id.to_string(),
            peer_id: self.identity.endpoint_id(),
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.mutation_handle
                .broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }

    pub async fn create_join_link(
        &self,
        max_uses: u32,
        expires_at: Option<u64>,
    ) -> anyhow::Result<String> {
        let pid = self.identity.endpoint_id();
        let has_perm = willow_actor::state::select(&self.event_state_addr, move |es| {
            es.has_permission(&pid, &willow_state::Permission::CreateInvite)
        })
        .await;
        if !has_perm {
            anyhow::bail!("missing CreateInvite permission");
        }

        let server_id = willow_actor::state::select(&self.server_registry_addr, |reg| {
            reg.active_server.clone()
        })
        .await
        .ok_or_else(|| anyhow::anyhow!("no active server"))?;

        let server_name = willow_actor::state::select(&self.server_registry_addr, |reg| {
            reg.active().map(|e| e.name.clone()).unwrap_or_default()
        })
        .await;

        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        let inviter_name = profiles
            .names
            .get(&pid)
            .cloned()
            .unwrap_or_default();

        let link = ops::JoinLink {
            link_id: uuid::Uuid::new_v4().to_string(),
            server_id: server_id.clone(),
            max_uses,
            used: 0,
            active: true,
            expires_at,
            created_at: util::current_time_ms(),
        };
        let token = ops::JoinToken {
            inviter_peer_id: pid,
            server_id,
            link_id: link.link_id.clone(),
            server_name,
            inviter_name,
        };
        self.join_links.lock().unwrap().push(link);
        Ok(token.encode())
    }

    pub async fn join_links(&self) -> Vec<ops::JoinLink> {
        self.join_links.lock().unwrap().clone()
    }

    pub async fn delete_join_link(&self, link_id: &str) {
        let link_id = link_id.to_string();
        self.join_links
            .lock()
            .unwrap()
            .retain(|l| l.link_id != link_id);
    }

    pub async fn set_display_name(&self, name: &str) {
        let pid = self.identity.endpoint_id();
        let name = name.to_string();
        willow_actor::state::mutate(&self.profile_state_addr, move |p| {
            p.names.insert(pid, name);
        })
        .await;
        self.broadcast_profile_via_network();
    }

    pub async fn send_typing(&self) {
        let (should_send, channel) = willow_actor::state::mutate(&self.network_meta_addr, |n| {
            let now = util::current_time_ms();
            if now - n.last_typing_sent_ms < 3000 {
                return (false, String::new());
            }
            n.last_typing_sent_ms = now;
            (true, String::new())
        })
        .await;
        if !should_send {
            return;
        }
        let channel =
            willow_actor::state::select(&self.chat_meta_addr, |c| c.current_channel.clone())
                .await;
        if channel.is_empty() {
            return;
        }
        let msg = ops::WireMessage::TypingIndicator { channel };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.mutation_handle
                .broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }

    pub async fn typing_in(&self, channel: &str) -> Vec<String> {
        let channel = channel.to_string();
        let my_id = self.identity.endpoint_id();
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        willow_actor::state::mutate(&self.network_meta_addr, move |n| {
            let now = util::current_time_ms();
            n.typing_peers.retain(|_, (_, ts)| now - *ts < 5000);
            n.typing_peers
                .iter()
                .filter(|(pid, (ch, _))| ch == &channel && *pid != &my_id)
                .map(|(pid, _)| views::resolve_display_name(&es, &profiles, pid))
                .collect()
        })
        .await
    }
}
