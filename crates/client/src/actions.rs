use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    pub async fn send_message(&self, channel: &str, body: &str) -> anyhow::Result<()> {
        self.mutation_handle.send_message(channel, body).await
    }

    pub async fn send_reply(
        &self,
        channel: &str,
        parent_hash: &willow_state::EventHash,
        body: &str,
    ) -> anyhow::Result<()> {
        self.mutation_handle
            .send_reply(channel, parent_hash, body)
            .await
    }

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

    pub async fn edit_message(
        &self,
        _channel: &str,
        message_id: &willow_state::EventHash,
        new_body: &str,
    ) -> anyhow::Result<()> {
        self.mutation_handle
            .edit_message(message_id, new_body)
            .await
    }

    pub async fn delete_message(
        &self,
        _channel: &str,
        message_id: &willow_state::EventHash,
    ) -> anyhow::Result<()> {
        self.mutation_handle.delete_message(message_id).await
    }

    pub async fn react(
        &self,
        _channel: &str,
        message_id: &willow_state::EventHash,
        emoji: &str,
    ) -> anyhow::Result<()> {
        self.mutation_handle.react(message_id, emoji).await
    }

    pub async fn pin_message(
        &self,
        channel: &str,
        message_id: &willow_state::EventHash,
    ) -> anyhow::Result<()> {
        self.mutation_handle.pin_message(channel, message_id).await
    }

    pub async fn unpin_message(
        &self,
        channel: &str,
        message_id: &willow_state::EventHash,
    ) -> anyhow::Result<()> {
        self.mutation_handle
            .unpin_message(channel, message_id)
            .await
    }

    pub async fn pinned_message_ids(&self, channel: &str) -> Vec<willow_state::EventHash> {
        let channel = channel.to_string();
        willow_actor::state::select(&self.event_state_addr, move |es| {
            let channel_id = es
                .channels
                .iter()
                .find(|(_, ch)| ch.name == channel)
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
            es.channels
                .get(&channel_id)
                .map(|ch| {
                    let mut ids: Vec<willow_state::EventHash> =
                        ch.pinned_messages.iter().cloned().collect();
                    ids.sort();
                    ids
                })
                .unwrap_or_default()
        })
        .await
    }

    pub async fn pinned_messages(&self, channel: &str) -> Vec<state::DisplayMessage> {
        let pinned_ids = self.pinned_message_ids(channel).await;
        if pinned_ids.is_empty() {
            return vec![];
        }
        let pinned_set: std::collections::HashSet<String> =
            pinned_ids.iter().map(|h| h.to_string()).collect();
        self.messages(channel)
            .await
            .into_iter()
            .filter(|m| pinned_set.contains(&m.id))
            .collect()
    }

    pub async fn is_pinned(&self, channel: &str, message_id: &willow_state::EventHash) -> bool {
        self.pinned_message_ids(channel)
            .await
            .iter()
            .any(|id| id == message_id)
    }

    pub async fn create_channel(&self, name: &str) -> anyhow::Result<()> {
        self.mutation_handle.create_channel(name).await
    }

    pub async fn create_voice_channel(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let name_for_event = name.clone();
        let ch_id_str = willow_actor::state::mutate(
            &self.server_registry_addr,
            move |reg| -> anyhow::Result<String> {
                let entry = reg
                    .active_mut()
                    .ok_or_else(|| anyhow::anyhow!("no active server"))?;
                let ch_id = entry
                    .server
                    .create_channel(&name, willow_channel::ChannelKind::Voice)?;
                let topic = util::make_topic(&entry.server, &name);
                if let Some(key) = entry.server.channel_key(&ch_id) {
                    entry.keys.insert(topic.clone(), key.clone());
                }
                let ch_id_str = ch_id.to_string();
                entry.topic_map.insert(topic, (name.clone(), ch_id));
                Ok(ch_id_str)
            },
        )
        .await?;
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::CreateChannel {
                name: name_for_event,
                channel_id: ch_id_str,
                kind: "voice".to_string(),
            })?;
        self.mutation_handle.apply_event(&event).await;
        self.mutation_handle.broadcast_event(&event);
        Ok(())
    }

    pub async fn delete_channel(&self, name: &str) -> anyhow::Result<()> {
        self.mutation_handle.delete_channel(name).await
    }

    pub async fn propose_kick_member(
        &self,
        peer_id: willow_identity::EndpointId,
    ) -> anyhow::Result<()> {
        self.mutation_handle.propose_kick_member(peer_id).await
    }

    pub async fn create_role(&self, name: &str) -> anyhow::Result<()> {
        self.mutation_handle.create_role(name).await
    }

    pub async fn delete_role(&self, role_id: &str) -> anyhow::Result<()> {
        self.mutation_handle.delete_role(role_id).await
    }

    pub async fn set_permission(
        &self,
        role_id: &str,
        permission: &str,
        granted: bool,
    ) -> anyhow::Result<()> {
        let role_id = role_id.to_string();
        let permission = permission.to_string();
        let rid = willow_channel::RoleId(
            uuid::Uuid::parse_str(&role_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        let perm = parse_permission(&permission)?;
        willow_actor::state::mutate(
            &self.server_registry_addr,
            move |reg| -> anyhow::Result<()> {
                let entry = reg
                    .active_mut()
                    .ok_or_else(|| anyhow::anyhow!("no active server"))?;
                entry.server.set_permission(&rid, perm, granted)?;
                Ok(())
            },
        )
        .await?;
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::SetPermission {
                role_id,
                permission,
                granted,
            })?;
        self.mutation_handle.apply_event(&event).await;
        self.mutation_handle.broadcast_event(&event);
        Ok(())
    }

    pub async fn assign_role(
        &self,
        peer_id: willow_identity::EndpointId,
        role_id: &str,
    ) -> anyhow::Result<()> {
        let role_id = role_id.to_string();
        let rid = willow_channel::RoleId(
            uuid::Uuid::parse_str(&role_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        );
        willow_actor::state::mutate(
            &self.server_registry_addr,
            move |reg| -> anyhow::Result<()> {
                let entry = reg
                    .active_mut()
                    .ok_or_else(|| anyhow::anyhow!("no active server"))?;
                let member_peer = entry
                    .server
                    .members()
                    .iter()
                    .find(|m| m.peer_id == peer_id)
                    .map(|m| m.peer_id);
                let Some(peer) = member_peer else {
                    anyhow::bail!("peer not found");
                };
                entry.server.assign_role(&peer, &rid)?;
                Ok(())
            },
        )
        .await?;
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::AssignRole { peer_id, role_id })?;
        self.mutation_handle.apply_event(&event).await;
        self.mutation_handle.broadcast_event(&event);
        Ok(())
    }

    /// Switch the current channel.
    pub async fn switch_channel(&self, channel: &str) {
        self.mutation_handle.switch_channel(channel).await;
    }
}
