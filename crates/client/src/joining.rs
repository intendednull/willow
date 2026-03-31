use super::*;
use crate::client_actor::{mutate_state, read_state};

impl<N: willow_network::Network> ClientHandle<N> {
    /// Generate a secure invite code encrypted for the given recipient.
    pub async fn generate_invite(
        &self,
        recipient_peer_id: &willow_identity::EndpointId,
    ) -> anyhow::Result<String> {
        let pub_key = invite::endpoint_id_to_ed25519_public(recipient_peer_id);

        read_state(&self.state_addr, move |s| {
            let ctx = s
                .state
                .active()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            invite::generate_invite(&ctx.server, &ctx.keys, &ctx.topic_map, &pub_key)
                .ok_or_else(|| anyhow::anyhow!("invite generation failed"))
        })
        .await
    }

    /// Accept an invite code and join the server.
    pub async fn accept_invite(&self, code: &str) -> anyhow::Result<()> {
        let code = code.to_string();
        let identity = self.identity.clone();
        let channel_topics = mutate_state(&self.state_addr, move |s| {
            let accepted = invite::accept_invite(&code, &identity)
                .ok_or_else(|| anyhow::anyhow!("invalid invite code or not for us"))?;

            let server_id = accepted.server_id.clone();

            // Check if we already have this server.
            if let Some(ctx) = s.state.servers.get_mut(&server_id) {
                // Merge new channel keys into existing server context.
                for (topic, (name, key)) in &accepted.channel_keys {
                    ctx.keys.insert(topic.clone(), key.clone());
                    if !ctx.topic_map.contains_key(topic) {
                        ctx.topic_map.insert(
                            topic.clone(),
                            (name.clone(), willow_channel::ChannelId::new()),
                        );
                    }
                    // Topic subscription deferred to connect().
                    let _ = topic;
                }
            } else {
                // Create a new server context for this server.
                // Use the ORIGINAL server ID from the invite so topics match.
                // Use the ACTUAL owner from the invite, not the joiner's peer ID.
                // This is persisted and used on reload to initialize event_state --
                // if the owner is wrong, the actual owner's events get rejected.
                let mut server = willow_channel::Server::new(&accepted.server_name, accepted.owner);
                server.id = willow_channel::ServerId(
                    uuid::Uuid::parse_str(&server_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
                );

                let mut topic_map = HashMap::new();
                let mut keys = HashMap::new();

                for (topic, (name, key)) in &accepted.channel_keys {
                    // Create the channel on the server so it appears in channels().
                    let ch_id = server
                        .create_channel(name, willow_channel::ChannelKind::Text)
                        .unwrap_or_else(|_| willow_channel::ChannelId::new());

                    // Override the generated key with the one from the invite.
                    server.set_channel_key(ch_id.clone(), key.clone());

                    keys.insert(topic.clone(), key.clone());
                    topic_map.insert(topic.clone(), (name.clone(), ch_id));
                    // Topic subscription deferred to connect().
                    let _ = topic;
                }

                let ctx = ServerContext {
                    server,
                    topic_map,
                    keys,
                    unread: HashMap::new(),
                };

                s.state.servers.insert(server_id.clone(), ctx);
            }

            s.state.active_server = Some(server_id.clone());
            init_event_state_on_shared(s, &server_id);

            // Fix the event_state owner to be the ACTUAL server owner from the invite,
            // not the joining peer. This is critical for permission checks -- without it,
            // the actual owner's events (CreateChannel, etc.) get rejected.
            s.state.event_state.owner = accepted.owner;
            // Also add the owner as a member so permissions work.
            s.state
                .event_state
                .members
                .entry(accepted.owner)
                .or_insert_with(|| willow_state::Member {
                    peer_id: accepted.owner,
                    roles: std::collections::HashSet::new(),
                    display_name: None,
                });

            reconcile_topic_map(&mut s.state);

            if let Some((_, (name, _))) = accepted.channel_keys.iter().next() {
                s.state.chat.current_channel = name.clone();
            }

            // Persist all servers so the joined server survives refresh.
            persist_servers(&s.state);

            // Request sync for the new server -- get all events from peers.
            let channel_topics: Vec<String> = s
                .state
                .servers
                .get(&server_id)
                .map(|ctx| ctx.topic_map.keys().cloned().collect())
                .unwrap_or_default();

            Ok::<_, anyhow::Error>(channel_topics)
        })
        .await?;

        let msg = ops::WireMessage::SyncRequest {
            state_hash: willow_state::StateHash::ZERO,
            topic: None,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
        for topic_str in channel_topics {
            let msg = ops::WireMessage::SyncRequest {
                state_hash: willow_state::StateHash::ZERO,
                topic: Some(topic_str.clone()),
            };
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.broadcast_on_topic(&topic_str, data);
            }
        }

        Ok(())
    }

    /// Publish raw data on a gossipsub topic.
    pub fn publish(&self, topic: &str, data: Vec<u8>) {
        self.broadcast_on_topic(topic, data);
    }

    /// Send a JoinRequest for a link ID on the server ops topic.
    pub fn send_join_request(&self, link_id: &str) {
        let msg = ops::WireMessage::JoinRequest {
            link_id: link_id.to_string(),
            peer_id: self.identity.endpoint_id(),
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }

    /// Create a join link for the active server. Returns the encoded token string.
    /// Requires `CreateInvite` permission (owner has this implicitly).
    pub async fn create_join_link(
        &self,
        max_uses: u32,
        expires_at: Option<u64>,
    ) -> anyhow::Result<String> {
        mutate_state(&self.state_addr, move |s| {
            let server_id = s
                .state
                .active_server
                .clone()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            let peer_id = s.identity.endpoint_id();
            if !s
                .state
                .event_state
                .has_permission(&peer_id, &willow_state::Permission::CreateInvite)
            {
                return Err(anyhow::anyhow!("missing CreateInvite permission"));
            }

            let server_name = s
                .state
                .active()
                .map(|c| c.server.name.clone())
                .unwrap_or_default();
            let inviter_name = s
                .state
                .profiles
                .names
                .get(&peer_id)
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
                inviter_peer_id: peer_id,
                server_id,
                link_id: link.link_id.clone(),
                server_name,
                inviter_name,
            };

            s.join_links.push(link);
            if s.config.persistence {
                storage::save_join_links(
                    s.state.active_server.as_deref().unwrap_or(""),
                    &s.join_links,
                );
            }

            Ok(token.encode())
        })
        .await
    }

    /// Return all join links for the active server.
    pub async fn join_links(&self) -> Vec<ops::JoinLink> {
        read_state(&self.state_addr, |s| s.join_links.clone()).await
    }

    /// Delete a join link by ID.
    pub async fn delete_join_link(&self, link_id: &str) {
        let link_id = link_id.to_string();
        mutate_state(&self.state_addr, move |s| {
            s.join_links.retain(|l| l.link_id != link_id);
            if s.config.persistence {
                storage::save_join_links(
                    s.state.active_server.as_deref().unwrap_or(""),
                    &s.join_links,
                );
            }
        })
        .await;
    }

    /// Set the local display name and broadcast to peers.
    pub async fn set_display_name(&self, name: &str) {
        let name = name.to_string();
        mutate_state(&self.state_addr, move |s| {
            let peer_id = s.identity.endpoint_id();
            s.state.profiles.names.insert(peer_id, name.clone());

            storage::save_profile(&storage::LocalProfile { display_name: name });
        })
        .await;
        self.broadcast_profile_via_network();
    }

    /// Switch the current channel.
    pub async fn switch_channel(&self, name: &str) {
        let name = name.to_string();
        mutate_state(&self.state_addr, move |s| {
            if s.state.chat.current_channel != name {
                s.state.chat.current_channel = name.clone();

                if let Some(ctx) = s.state.active_mut() {
                    if let Some(topic) = ctx.topic_for_name(&name) {
                        ctx.unread.remove(&topic);
                    }
                }
            }
        })
        .await;
    }

    /// Notify peers that we are typing in the current channel.
    ///
    /// Debounced -- will not send more than once per 3 seconds.
    pub async fn send_typing(&self) {
        let channel = mutate_state(&self.state_addr, |s| {
            let now = util::current_time_ms();
            if now - s.last_typing_sent_ms < 3000 {
                return None; // debounce
            }
            s.last_typing_sent_ms = now;

            let channel = s.state.chat.current_channel.clone();
            if channel.is_empty() {
                None
            } else {
                Some(channel)
            }
        })
        .await;
        if let Some(channel) = channel {
            let msg = ops::WireMessage::TypingIndicator { channel };
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
            }
        }
    }

    /// Get display names of peers currently typing in the given channel.
    ///
    /// Automatically expires entries older than 5 seconds and excludes the
    /// local user.
    pub async fn typing_in(&self, channel: &str) -> Vec<String> {
        let channel = channel.to_string();
        mutate_state(&self.state_addr, move |s| {
            let now = util::current_time_ms();
            // Remove expired entries (older than 5 seconds).
            s.typing_peers.retain(|_, (_, ts)| now - *ts < 5000);

            let my_id = s.identity.endpoint_id();
            s.typing_peers
                .iter()
                .filter(|(pid, (ch, _))| ch == &channel && *pid != &my_id)
                .map(|(pid, _)| peer_display_name_shared(s, pid))
                .collect()
        })
        .await
    }
}
