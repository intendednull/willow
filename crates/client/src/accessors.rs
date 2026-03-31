use super::*;
use crate::client_actor::read_state;

impl<N: willow_network::Network> ClientHandle<N> {
    pub fn identity(&self) -> Identity {
        self.identity.clone()
    }

    pub fn state_addr(&self) -> &willow_actor::Addr<client_actor::ClientStateActor> {
        &self.state_addr
    }

    pub fn actor_system(&self) -> &willow_actor::SystemHandle {
        &self.system
    }

    pub fn notify_mutation(&self) {
        let _ = self.state_addr.do_send(client_actor::NotifyMutation);
    }

    pub fn peer_id(&self) -> String {
        self.identity.endpoint_id().to_string()
    }

    pub async fn display_name(&self) -> String {
        read_state(&self.state_addr, |s| {
            let pid = s.identity.endpoint_id();
            if let Some(profile) = s.state.event_state.profiles.get(&pid) {
                return profile.display_name.clone();
            }
            s.state.profiles.display_name(&pid)
        })
        .await
    }

    pub async fn peer_display_name(&self, peer_id: &willow_identity::EndpointId) -> String {
        let pid = *peer_id;
        read_state(&self.state_addr, move |s| peer_display_name_shared(s, &pid)).await
    }

    pub async fn messages(&self, channel: &str) -> Vec<state::DisplayMessage> {
        let ch = channel.to_string();
        read_state(&self.state_addr, move |s| {
            let mut channel_ids: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            if let Some(ctx) = s.state.active() {
                for (name, cid) in ctx.topic_map.values() {
                    if name == &ch {
                        channel_ids.insert(cid.to_string());
                    }
                }
            }
            for (id, c) in &s.state.event_state.channels {
                if c.name == ch {
                    channel_ids.insert(id.clone());
                }
            }
            if channel_ids.is_empty() {
                return vec![];
            }
            let local_peer_id = s.identity.endpoint_id();
            let mut msgs: Vec<state::DisplayMessage> = s
                .state
                .event_state
                .messages
                .iter()
                .filter(|m| channel_ids.contains(&m.channel_id))
                .map(|m| {
                    let author_name = s
                        .state
                        .event_state
                        .profiles
                        .get(&m.author)
                        .map(|p| p.display_name.clone())
                        .unwrap_or_else(|| s.state.profiles.display_name(&m.author));
                    let reply_preview = m.reply_to.as_ref().and_then(|parent_id| {
                        s.state
                            .event_state
                            .messages
                            .iter()
                            .find(|pm| pm.id == *parent_id)
                            .map(|pm| {
                                let parent_name = s
                                    .state
                                    .event_state
                                    .profiles
                                    .get(&pm.author)
                                    .map(|p| p.display_name.clone())
                                    .unwrap_or_else(|| s.state.profiles.display_name(&pm.author));
                                let text = if pm.body.len() > 50 {
                                    format!("{}...", &pm.body[..50])
                                } else {
                                    pm.body.clone()
                                };
                                format!("{parent_name}: {text}")
                            })
                    });
                    let reactions = m
                        .reactions
                        .iter()
                        .map(|(emoji, peer_ids)| {
                            let names: Vec<String> = peer_ids
                                .iter()
                                .map(|pid| {
                                    s.state
                                        .event_state
                                        .profiles
                                        .get(pid)
                                        .map(|p| p.display_name.clone())
                                        .unwrap_or_else(|| s.state.profiles.display_name(pid))
                                })
                                .collect();
                            (emoji.clone(), names)
                        })
                        .collect();
                    state::DisplayMessage {
                        id: m.id.clone(),
                        channel_id: m.channel_id.clone(),
                        author_peer_id: m.author,
                        author_display_name: author_name,
                        body: m.body.clone(),
                        is_local: m.author == local_peer_id,
                        timestamp_ms: m.timestamp_ms,
                        reactions,
                        edited: m.edited,
                        deleted: m.deleted,
                        reply_to: m.reply_to.clone(),
                        reply_preview,
                    }
                })
                .collect();
            msgs.sort_by_key(|m| m.timestamp_ms);
            msgs
        })
        .await
    }

    pub async fn channels(&self) -> Vec<String> {
        read_state(&self.state_addr, |s| {
            let mut names = s.state.channel_names();
            for ch in s.state.event_state.channels.values() {
                if !names.contains(&ch.name) {
                    names.push(ch.name.clone());
                }
            }
            names.sort();
            names.dedup();
            names
        })
        .await
    }

    pub async fn event_messages(&self, channel_id: &str) -> Vec<willow_state::ChatMessage> {
        let cid = channel_id.to_string();
        read_state(&self.state_addr, move |s| {
            s.state
                .event_state
                .messages
                .iter()
                .filter(|m| m.channel_id == cid && !m.deleted)
                .cloned()
                .collect()
        })
        .await
    }

    pub async fn peers(&self) -> Vec<willow_identity::EndpointId> {
        read_state(&self.state_addr, |s| s.state.chat.peers.clone()).await
    }

    pub async fn server_members(&self) -> Vec<(willow_identity::EndpointId, String, bool)> {
        read_state(&self.state_addr, |s| {
            let local_id = s.identity.endpoint_id();
            let online: std::collections::HashSet<willow_identity::EndpointId> =
                s.state.chat.peers.iter().copied().collect();
            let mut result = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for (pid, member) in &s.state.event_state.members {
                let name = member
                    .display_name
                    .clone()
                    .or_else(|| {
                        s.state
                            .event_state
                            .profiles
                            .get(pid)
                            .map(|p| p.display_name.clone())
                    })
                    .unwrap_or_else(|| peer_display_name_shared(s, pid));
                let is_online = *pid == local_id || online.contains(pid);
                result.push((*pid, name, is_online));
                seen.insert(*pid);
            }
            for pid in &s.state.chat.peers {
                if !seen.contains(pid) {
                    let name = peer_display_name_shared(s, pid);
                    result.push((*pid, name, true));
                }
            }
            result
        })
        .await
    }

    pub async fn is_connected(&self) -> bool {
        read_state(&self.state_addr, |s| s.connected).await
    }

    pub async fn has_permission(
        &self,
        peer_id: &willow_identity::EndpointId,
        perm: &willow_state::Permission,
    ) -> bool {
        let pid = *peer_id;
        let p = perm.clone();
        read_state(&self.state_addr, move |s| {
            s.state.event_state.has_permission(&pid, &p)
        })
        .await
    }

    pub async fn roles_data(&self) -> Vec<(String, String, Vec<String>)> {
        read_state(&self.state_addr, |s| {
            let mut entries: Vec<(String, String, Vec<String>)> = s
                .state
                .event_state
                .roles
                .values()
                .map(|role| {
                    let perms: Vec<String> = role.permissions.iter().cloned().collect();
                    (role.id.clone(), role.name.clone(), perms)
                })
                .collect();
            entries.sort_by(|a, b| a.1.cmp(&b.1));
            entries
        })
        .await
    }

    pub async fn unread_counts(&self) -> HashMap<String, usize> {
        read_state(&self.state_addr, |s| {
            let mut unread_map = HashMap::new();
            if let Some(ctx) = s.state.active() {
                for (topic, count) in &ctx.unread {
                    if let Some(name) = ctx.name_for_topic(topic) {
                        unread_map.insert(name.to_string(), *count);
                    }
                }
            }
            unread_map
        })
        .await
    }

    pub async fn server_owner(&self) -> willow_identity::EndpointId {
        read_state(&self.state_addr, |s| s.state.event_state.owner).await
    }

    pub async fn channel_kinds(&self) -> Vec<(String, String)> {
        read_state(&self.state_addr, |s| {
            s.state
                .event_state
                .channels
                .values()
                .map(|ch| (ch.name.clone(), ch.kind.clone()))
                .collect()
        })
        .await
    }

    pub async fn current_channel(&self) -> String {
        read_state(&self.state_addr, |s| s.state.chat.current_channel.clone()).await
    }
}
