use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    pub async fn generate_invite(
        &self,
        recipient_peer_id: &willow_identity::EndpointId,
    ) -> anyhow::Result<String> {
        let pub_key = invite::endpoint_id_to_ed25519_public(recipient_peer_id);

        // Gather server info from registry.
        let (server_name, server_id, keys) =
            willow_actor::state::select(&self.server_registry_addr, move |reg| {
                let entry = reg
                    .active()
                    .ok_or_else(|| anyhow::anyhow!("no active server"))?;
                Ok::<_, anyhow::Error>((
                    entry.name.clone(),
                    entry.server_id.clone(),
                    entry.keys.clone(),
                ))
            })
            .await?;

        // Build topic_names from event_state channels + server_id.
        let sid = server_id.clone();
        let topic_names: HashMap<String, String> =
            willow_actor::state::select(&self.event_state_addr, move |es| {
                es.channels
                    .values()
                    .map(|ch| {
                        let topic = crate::util::make_topic(&sid, &ch.name);
                        (topic, ch.name.clone())
                    })
                    .collect()
            })
            .await;

        // Get genesis author (first admin) from event_state.
        let my_id = self.identity.endpoint_id();
        let genesis_author = willow_actor::state::select(&self.event_state_addr, move |es| {
            es.admins.iter().next().copied().unwrap_or(my_id)
        })
        .await;

        let invite_code = invite::generate_invite(
            &server_name,
            &server_id,
            genesis_author,
            &keys,
            &topic_names,
            &pub_key,
        )
        .ok_or_else(|| anyhow::anyhow!("invite generation failed"))?;

        // Grant SendMessages permission to the joining peer so they can
        // actually send messages once they accept the invite. Without this,
        // the joined peer's messages are silently rejected by this (the
        // inviter's) apply_incremental permission check.
        if let Ok(grant_event) = self
            .mutation_handle
            .build_event(willow_state::EventKind::GrantPermission {
                peer_id: *recipient_peer_id,
                permission: willow_state::Permission::SendMessages,
            })
            .await
        {
            self.mutation_handle.apply_event(&grant_event).await;
            self.mutation_handle.broadcast_event(&grant_event);
        }

        Ok(invite_code)
    }

    pub async fn accept_invite(&self, code: &str) -> anyhow::Result<()> {
        let code = code.to_string();
        let identity = self.identity.clone();
        let accepted = invite::accept_invite(&code, &identity)
            .ok_or_else(|| anyhow::anyhow!("invalid invite code or not for us"))?;

        let server_id = accepted.server_id.clone();
        let genesis_author = accepted.genesis_author;
        let first_channel_name = accepted
            .channel_keys
            .values()
            .next()
            .map(|(name, _)| name.clone());

        // Validate the server id BEFORE we touch any actor state. If the
        // invite is malformed we want to surface a typed error to the
        // caller instead of silently inventing a fresh server id, which
        // would split-brain the joiner from the rest of the network
        // (issue #115).
        let _parsed_server_uuid = uuid::Uuid::parse_str(&server_id).map_err(|e| {
            crate::ClientError::MalformedInvite(format!("invalid server_id `{server_id}`: {e}"))
        })?;

        // Update server registry.
        let channel_topics = willow_actor::state::mutate(
            &self.server_registry_addr,
            move |reg| -> Result<Vec<String>, crate::ClientError> {
                if let Some(entry) = reg.servers.get_mut(&server_id) {
                    // Existing server — just merge in the channel keys.
                    for (topic, (_name, key)) in &accepted.channel_keys {
                        entry.keys.insert(topic.clone(), key.clone());
                    }
                } else {
                    // New server — build a minimal ServerEntry.
                    let mut keys = HashMap::new();
                    for (topic, (_name, key)) in &accepted.channel_keys {
                        keys.insert(topic.clone(), key.clone());
                    }
                    reg.servers.insert(
                        server_id.clone(),
                        state_actors::ServerEntry {
                            server_id: server_id.clone(),
                            name: accepted.server_name.clone(),
                            keys,
                            unread: HashMap::new(),
                        },
                    );
                }
                reg.active_server = Some(server_id.clone());
                // Derive channel topics from invite channel_keys.
                let topics: Vec<String> = accepted.channel_keys.keys().cloned().collect();
                Ok(topics)
            },
        )
        .await?;

        // Initialize event state for the joined server with a placeholder.
        // The DAG remains empty — it will be populated from the sync batch
        // which delivers the full event history including genesis. Local
        // mutations before sync completes will fail gracefully.
        let sid = accepted.server_id.clone();
        willow_actor::state::mutate(&self.event_state_addr, move |es| {
            *es = willow_state::ServerState::new(&sid, "", genesis_author);
        })
        .await;

        // Set current channel.
        if let Some(name) = &first_channel_name {
            self.mutation_handle.switch_channel(name).await;
        }

        // Open event store on persistence actor.
        self.persistence_addr
            .do_send(persistence_actor::OpenEventStore {
                server_id: accepted.server_id.clone(),
            })
            .ok();

        // Persist the server list and metadata so this joined server survives
        // a page reload without requiring a network sync round-trip.
        // Use synchronous direct storage writes so the data is guaranteed to
        // be in localStorage before this function returns — fire-and-forget
        // actor messages may be delayed past a page reload.
        if self.persistence_enabled {
            let (all_ids, entry_name, entry_keys) =
                willow_actor::state::select(&self.server_registry_addr, |reg| {
                    let ids = reg.servers.keys().cloned().collect::<Vec<_>>();
                    let (name, keys) = reg
                        .active()
                        .map(|e| (e.name.clone(), e.keys.clone()))
                        .unwrap_or_default();
                    (ids, name, keys)
                })
                .await;
            storage::save_server_list(&all_ids);
            let meta = storage::SavedServerMeta {
                server_id: accepted.server_id.clone(),
                name: entry_name,
            };
            storage::save_server_by_id(&accepted.server_id, &meta, &entry_keys);
        }

        // Request sync.
        let msg = ops::WireMessage::SyncRequest {
            state_hash: willow_state::EventHash::ZERO,
            topic: None,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.mutation_handle
                .broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
        for topic_str in channel_topics {
            let msg = ops::WireMessage::SyncRequest {
                state_hash: willow_state::EventHash::ZERO,
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
        let inviter_name = profiles.names.get(&pid).cloned().unwrap_or_default();

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
        self.join_links.lock().push(link);
        Ok(token.encode())
    }

    pub async fn join_links(&self) -> Vec<ops::JoinLink> {
        self.join_links.lock().clone()
    }

    pub async fn delete_join_link(&self, link_id: &str) {
        let link_id = link_id.to_string();
        self.join_links.lock().retain(|l| l.link_id != link_id);
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
        let should_send = willow_actor::state::mutate(&self.network_meta_addr, |n| {
            let now = util::current_time_ms();
            if now - n.last_typing_sent_ms < 3000 {
                return false;
            }
            n.last_typing_sent_ms = now;
            true
        })
        .await;
        if !should_send {
            return;
        }
        let channel =
            willow_actor::state::select(&self.chat_meta_addr, |c| c.current_channel.clone()).await;
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
            n.typing_peers
                .retain(|_, (_, ts)| now - *ts < crate::TYPING_INDICATOR_TTL_MS);
            n.typing_peers
                .iter()
                .filter(|(pid, (ch, _))| ch == &channel && *pid != &my_id)
                .map(|(pid, _)| views::resolve_display_name(&es, &profiles, pid))
                .collect()
        })
        .await
    }
}
