use super::*;
use crate::client_actor::{mutate_state, read_state};

impl<N: willow_network::Network> ClientHandle<N> {
    /// Send a text message to the given channel.
    pub async fn send_message(&self, channel: &str, body: &str) -> anyhow::Result<()> {
        let content = Content::Text {
            body: body.to_string(),
        };
        self.send_content(channel, content, body, None, None).await
    }

    /// Send a reply to a specific message.
    pub async fn send_reply(
        &self,
        channel: &str,
        parent_id: &str,
        body: &str,
    ) -> anyhow::Result<()> {
        let parent =
            willow_messaging::MessageId(uuid::Uuid::parse_str(parent_id).unwrap_or_default());
        let content = Content::Reply {
            parent,
            body: body.to_string(),
        };

        // Build reply preview from event_state messages.
        let parent_id_owned = parent_id.to_string();
        let preview = read_state(&self.state_addr, move |s| {
            s.state
                .event_state
                .messages
                .iter()
                .find(|m| m.id == parent_id_owned)
                .map(|m| {
                    let text = if m.body.len() > 50 {
                        format!("{}...", &m.body[..50])
                    } else {
                        m.body.clone()
                    };
                    let author_name = s
                        .state
                        .event_state
                        .profiles
                        .get(&m.author)
                        .map(|p| p.display_name.clone())
                        .unwrap_or_else(|| s.state.profiles.display_name(&m.author));
                    format!("{author_name}: {text}")
                })
        })
        .await;

        self.send_content(channel, content, body, preview, Some(parent_id.to_string()))
            .await
    }

    /// Share a small file inline by base64-encoding it into a text message.
    ///
    /// The message body uses the format `[file:filename:base64data]` so the
    /// UI can detect it and render a download card. Files larger than 256 KB
    /// are rejected.
    pub async fn share_file_inline(
        &self,
        channel: &str,
        filename: &str,
        data: &[u8],
    ) -> anyhow::Result<()> {
        const MAX_INLINE_SIZE: usize = 256 * 1024;
        if data.len() > MAX_INLINE_SIZE {
            anyhow::bail!("file too large for inline sharing (max 256 KB)");
        }
        let encoded = base64::encode(data);
        let body = format!("[file:{}:{}]", filename, encoded);
        self.send_message(channel, &body).await
    }

    /// Edit an existing message.
    pub async fn edit_message(
        &self,
        _channel: &str,
        message_id: &str,
        new_body: &str,
    ) -> anyhow::Result<()> {
        let message_id = message_id.to_string();
        let new_body = new_body.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let _ = s
                .state
                .active()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            let peer_id_str = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id_str,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::EditMessage {
                    message_id,
                    new_body,
                },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Delete a message.
    pub async fn delete_message(&self, _channel: &str, message_id: &str) -> anyhow::Result<()> {
        let message_id = message_id.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let _ = s
                .state
                .active()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            let peer_id_str = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id_str,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::DeleteMessage { message_id },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Add a reaction to a message.
    pub async fn react(&self, _channel: &str, message_id: &str, emoji: &str) -> anyhow::Result<()> {
        let message_id = message_id.to_string();
        let emoji = emoji.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let _ = s
                .state
                .active()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            let peer_id_str = s.identity.endpoint_id();
            let reaction_event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id_str,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::Reaction { message_id, emoji },
            };
            apply_event_on_shared(s, &reaction_event);
            Ok::<_, anyhow::Error>(reaction_event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Pin a message in a channel.
    ///
    /// Creates a `PinMessage` event in the event-sourced state and broadcasts
    /// it to peers.
    pub async fn pin_message(&self, channel: &str, message_id: &str) -> anyhow::Result<()> {
        let channel = channel.to_string();
        let message_id = message_id.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let channel_id = resolve_channel_id_shared(&s.state, &channel)?;
            let peer_id = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::PinMessage {
                    channel_id,
                    message_id,
                },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Unpin a message from a channel.
    ///
    /// Creates an `UnpinMessage` event in the event-sourced state and
    /// broadcasts it to peers.
    pub async fn unpin_message(&self, channel: &str, message_id: &str) -> anyhow::Result<()> {
        let channel = channel.to_string();
        let message_id = message_id.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let channel_id = resolve_channel_id_shared(&s.state, &channel)?;
            let peer_id = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::UnpinMessage {
                    channel_id,
                    message_id,
                },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Get pinned message IDs for a channel from the event-sourced state.
    ///
    /// Returns a sorted `Vec` of message IDs that are pinned in the channel.
    pub async fn pinned_message_ids(&self, channel: &str) -> Vec<String> {
        let channel = channel.to_string();
        read_state(&self.state_addr, move |s| {
            // Find channel_id from event_state by name (authoritative).
            let channel_id = s
                .state
                .event_state
                .channels
                .iter()
                .find(|(_, ch)| ch.name == channel)
                .map(|(id, _)| id.clone())
                .or_else(|| {
                    s.state.active().and_then(|ctx| {
                        ctx.topic_map
                            .values()
                            .find(|(n, _)| n == &channel)
                            .map(|(_, cid)| cid.to_string())
                    })
                })
                .unwrap_or_default();

            s.state
                .event_state
                .channels
                .get(&channel_id)
                .map(|ch| {
                    let mut ids: Vec<String> = ch.pinned_messages.iter().cloned().collect();
                    ids.sort();
                    ids
                })
                .unwrap_or_default()
        })
        .await
    }

    /// Get pinned messages for a channel.
    ///
    /// Returns messages whose IDs are in the event-sourced pinned set.
    pub async fn pinned_messages(&self, channel: &str) -> Vec<state::DisplayMessage> {
        let pinned_ids = self.pinned_message_ids(channel).await;
        if pinned_ids.is_empty() {
            return vec![];
        }
        let pinned_set: std::collections::HashSet<&str> =
            pinned_ids.iter().map(|s| s.as_str()).collect();
        self.messages(channel)
            .await
            .into_iter()
            .filter(|m| pinned_set.contains(m.id.as_str()))
            .collect()
    }

    /// Check if a message is pinned in a channel.
    pub async fn is_pinned(&self, channel: &str, message_id: &str) -> bool {
        let pinned_ids = self.pinned_message_ids(channel).await;
        pinned_ids.iter().any(|id| id == message_id)
    }

    /// Create a new channel.
    pub async fn create_channel(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let ctx = s
                .state
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            let ch_id = ctx
                .server
                .create_channel(&name, willow_channel::ChannelKind::Text)?;
            let topic = util::make_topic(&ctx.server, &name);

            if let Some(key) = ctx.server.channel_key(&ch_id) {
                ctx.keys.insert(topic.clone(), key.clone());
            }
            storage::save_server(&ctx.server, &ctx.keys);

            let ch_id_str = ch_id.to_string();
            ctx.topic_map.insert(topic.clone(), (name.clone(), ch_id));

            // Topic subscription will be handled by connect() or a future
            // subscribe_topic() method. Noted for later wiring.
            let _ = &topic;

            // Create and apply event, then broadcast it.
            let peer_id_str = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id_str,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::CreateChannel {
                    name: name.clone(),
                    channel_id: ch_id_str,
                    kind: "text".to_string(),
                },
            };
            apply_event_on_shared(s, &event);
            s.state.chat.current_channel = name;
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Create a voice channel.
    pub async fn create_voice_channel(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let ctx = s
                .state
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            let ch_id = ctx
                .server
                .create_channel(&name, willow_channel::ChannelKind::Voice)?;
            let topic = util::make_topic(&ctx.server, &name);

            if let Some(key) = ctx.server.channel_key(&ch_id) {
                ctx.keys.insert(topic.clone(), key.clone());
            }
            storage::save_server(&ctx.server, &ctx.keys);

            let ch_id_str = ch_id.to_string();
            ctx.topic_map.insert(topic.clone(), (name.clone(), ch_id));

            // Topic subscription will be handled by connect() or a future
            // subscribe_topic() method. Noted for later wiring.
            let _ = &topic;

            let peer_id_str = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id_str,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::CreateChannel {
                    name: name.clone(),
                    channel_id: ch_id_str,
                    kind: "voice".to_string(),
                },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Delete a channel.
    pub async fn delete_channel(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let ctx = s
                .state
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            let Some((topic, (_ch_name, ch_id))) = ctx
                .topic_map
                .iter()
                .find(|(_, (n, _))| n == &name)
                .map(|(t, v)| (t.clone(), v.clone()))
            else {
                anyhow::bail!("channel not found");
            };

            let ch_id_str = ch_id.to_string();

            ctx.server.delete_channel(&ch_id)?;
            storage::save_server(&ctx.server, &ctx.keys);

            ctx.topic_map.remove(&topic);
            ctx.keys.remove(&topic);

            if s.state.chat.current_channel == name {
                let names = s
                    .state
                    .active()
                    .map(|ctx| ctx.channel_names())
                    .unwrap_or_default();
                s.state.chat.current_channel = names.first().cloned().unwrap_or_default();
            }

            // Create and apply event, then broadcast it.
            let peer_id_str = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id_str,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::DeleteChannel {
                    channel_id: ch_id_str,
                },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Trust a peer for server state operations.
    ///
    /// Applies a `GrantPermission(Administrator)` event to the event-sourced
    /// state and broadcasts the event on the wire.
    pub async fn trust_peer(&self, peer_id: willow_identity::EndpointId) {
        let event = mutate_state(&self.state_addr, move |s| {
            let author = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::GrantPermission {
                    peer_id,
                    permission: willow_state::Permission::Administrator,
                },
            };
            apply_event_on_shared(s, &event);
            event
        })
        .await;
        self.broadcast_event(&event);
    }

    /// Revoke trust from a peer.
    ///
    /// Applies a `RevokePermission(Administrator)` event to the event-sourced
    /// state and broadcasts the event on the wire.
    pub async fn untrust_peer(&self, peer_id: willow_identity::EndpointId) {
        let event = mutate_state(&self.state_addr, move |s| {
            let author = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::RevokePermission {
                    peer_id,
                    permission: willow_state::Permission::Administrator,
                },
            };
            apply_event_on_shared(s, &event);
            event
        })
        .await;
        self.broadcast_event(&event);
    }

    /// Kick a member, rotating channel keys.
    pub async fn kick_member(&self, peer_id: willow_identity::EndpointId) -> anyhow::Result<()> {
        let event = mutate_state(&self.state_addr, move |s| {
            let ctx = s
                .state
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            let member_peer = ctx
                .server
                .members()
                .iter()
                .find(|m| m.peer_id == peer_id)
                .map(|m| m.peer_id);

            let Some(peer) = member_peer else {
                anyhow::bail!("peer not found in server members");
            };

            let rotated = ctx.server.remove_member(&peer)?;
            storage::save_server(&ctx.server, &ctx.keys);

            // Update key store with rotated keys.
            for (ch_id, key) in &rotated {
                for (topic, (_, tid)) in &ctx.topic_map {
                    if tid == ch_id {
                        ctx.keys.insert(topic.clone(), key.clone());
                        break;
                    }
                }
            }

            s.state.chat.peers.retain(|p| *p != peer_id);

            // Create and apply event, then broadcast it.
            let author = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::KickMember { peer_id },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Create a new role.
    pub async fn create_role(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let role_id = willow_channel::RoleId::new();
            let role = willow_channel::Role::with_id(role_id.clone(), &name);

            let ctx = s
                .state
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;
            ctx.server.create_role(role);
            storage::save_server(&ctx.server, &ctx.keys);

            let peer_id_str = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id_str,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::CreateRole {
                    name,
                    role_id: role_id.to_string(),
                },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Delete a role by ID.
    pub async fn delete_role(&self, role_id: &str) -> anyhow::Result<()> {
        let role_id = role_id.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let rid = willow_channel::RoleId(uuid::Uuid::parse_str(&role_id).unwrap_or_default());

            let ctx = s
                .state
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;
            ctx.server.delete_role(&rid)?;
            storage::save_server(&ctx.server, &ctx.keys);

            let peer_id_str = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id_str,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::DeleteRole { role_id },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Set a permission on a role.
    pub async fn set_permission(
        &self,
        role_id: &str,
        permission: &str,
        granted: bool,
    ) -> anyhow::Result<()> {
        let role_id = role_id.to_string();
        let permission = permission.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let rid = willow_channel::RoleId(uuid::Uuid::parse_str(&role_id).unwrap_or_default());
            let perm = parse_permission(&permission)?;

            let ctx = s
                .state
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;
            ctx.server.set_permission(&rid, perm, granted)?;
            storage::save_server(&ctx.server, &ctx.keys);

            let peer_id_str = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id_str,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::SetPermission {
                    role_id,
                    permission,
                    granted,
                },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Assign a role to a peer.
    pub async fn assign_role(
        &self,
        peer_id: willow_identity::EndpointId,
        role_id: &str,
    ) -> anyhow::Result<()> {
        let role_id = role_id.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let rid = willow_channel::RoleId(uuid::Uuid::parse_str(&role_id).unwrap_or_default());

            let ctx = s
                .state
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;

            let member_peer = ctx
                .server
                .members()
                .iter()
                .find(|m| m.peer_id == peer_id)
                .map(|m| m.peer_id);

            let Some(peer) = member_peer else {
                anyhow::bail!("peer not found");
            };

            ctx.server.assign_role(&peer, &rid)?;
            storage::save_server(&ctx.server, &ctx.keys);

            let author = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::AssignRole { peer_id, role_id },
            };
            apply_event_on_shared(s, &event);
            Ok::<_, anyhow::Error>(event)
        })
        .await?;
        self.broadcast_event(&event);

        Ok(())
    }

    /// Broadcast a state verification event carrying this peer's current state hash.
    pub async fn verify_state(&self) -> anyhow::Result<()> {
        let event = mutate_state(&self.state_addr, move |s| {
            let author = s.identity.endpoint_id();
            let state_hash = s.state.event_state.hash();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::StateVerification { state_hash },
            };
            apply_event_on_shared(s, &event);
            event
        })
        .await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Returns (agreeing_peers, total_peers_reporting) based on collected
    /// StateVerification results.
    pub async fn state_hash_agreement(&self) -> (usize, usize) {
        read_state(&self.state_addr, |s| {
            let our_hash = s.state.event_state.hash();
            let total = s.state_verification_results.len();
            let agreeing = s
                .state_verification_results
                .values()
                .filter(|h| **h == our_hash)
                .count();
            (agreeing, total)
        })
        .await
    }
}
