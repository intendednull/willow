use super::*;
use crate::state_actors;

/// Resolve display name from event-sourced profiles, falling back to global profile store.
pub(crate) fn resolve_display_name(
    event_state: &willow_state::ServerState,
    profiles: &state_actors::ProfileState,
    peer_id: &willow_identity::EndpointId,
) -> String {
    event_state
        .profiles
        .get(peer_id)
        .map(|p| p.display_name.clone())
        .unwrap_or_else(|| profiles.display_name(peer_id))
}

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

    /// Subscribe to client events. Returns an [`EventReceiver`](crate::EventReceiver)
    /// that can be polled for events.
    pub async fn subscribe_events(&self) -> crate::EventReceiver {
        crate::EventReceiver::subscribe(&self.event_broker, &self.system).await
    }

    /// Get the event broker address (for direct subscription).
    pub fn event_broker(&self) -> &willow_actor::Addr<willow_actor::Broker<ClientEvent>> {
        &self.event_broker
    }

    pub async fn display_name(&self) -> String {
        let pid = self.identity.endpoint_id();
        let es = willow_actor::state::get(&self.event_state_addr).await;
        if let Some(profile) = es.profiles.get(&pid) {
            return profile.display_name.clone();
        }
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        profiles.display_name(&pid)
    }

    pub async fn peer_display_name(&self, peer_id: &willow_identity::EndpointId) -> String {
        let pid = *peer_id;
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        resolve_display_name(&es, &profiles, &pid)
    }

    pub async fn messages(&self, channel: &str) -> Vec<state::DisplayMessage> {
        let ch = channel.to_string();
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let registry = willow_actor::state::get(&self.server_registry_addr).await;
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;
        let local_peer_id = self.identity.endpoint_id();

        let mut channel_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        if let Some(ctx) = registry.active() {
            for (name, cid) in ctx.topic_map.values() {
                if name == &ch {
                    channel_ids.insert(cid.to_string());
                }
            }
        }
        for (id, c) in &es.channels {
            if c.name == ch {
                channel_ids.insert(id.clone());
            }
        }
        if channel_ids.is_empty() {
            return vec![];
        }
        let mut msgs: Vec<state::DisplayMessage> = es
            .messages
            .iter()
            .filter(|m| channel_ids.contains(&m.channel_id))
            .map(|m| {
                let author_name = resolve_display_name(&es, &profiles, &m.author);
                let reply_preview = m.reply_to.as_ref().and_then(|parent_id| {
                    es.messages
                        .iter()
                        .find(|pm| pm.id == *parent_id)
                        .map(|pm| {
                            let parent_name =
                                resolve_display_name(&es, &profiles, &pm.author);
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
                            .map(|pid| resolve_display_name(&es, &profiles, pid))
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
    }

    pub async fn channels(&self) -> Vec<String> {
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let registry = willow_actor::state::get(&self.server_registry_addr).await;
        let mut names = registry
            .active()
            .map(|e| e.channel_names())
            .unwrap_or_default();
        for ch in es.channels.values() {
            if !names.contains(&ch.name) {
                names.push(ch.name.clone());
            }
        }
        names.sort();
        names.dedup();
        names
    }

    pub async fn event_messages(&self, channel_id: &str) -> Vec<willow_state::ChatMessage> {
        let cid = channel_id.to_string();
        willow_actor::state::select(&self.event_state_addr, move |es| {
            es.messages
                .iter()
                .filter(|m| m.channel_id == cid && !m.deleted)
                .cloned()
                .collect()
        })
        .await
    }

    pub async fn peers(&self) -> Vec<willow_identity::EndpointId> {
        willow_actor::state::select(&self.chat_meta_addr, |c| c.peers.clone()).await
    }

    pub async fn server_members(&self) -> Vec<(willow_identity::EndpointId, String, bool)> {
        let local_id = self.identity.endpoint_id();
        let es = willow_actor::state::get(&self.event_state_addr).await;
        let chat = willow_actor::state::get(&self.chat_meta_addr).await;
        let profiles = willow_actor::state::get(&self.profile_state_addr).await;

        let online: std::collections::HashSet<willow_identity::EndpointId> =
            chat.peers.iter().copied().collect();
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (pid, member) in &es.members {
            let name = member
                .display_name
                .clone()
                .or_else(|| es.profiles.get(pid).map(|p| p.display_name.clone()))
                .unwrap_or_else(|| resolve_display_name(&es, &profiles, pid));
            let is_online = *pid == local_id || online.contains(pid);
            result.push((*pid, name, is_online));
            seen.insert(*pid);
        }
        for pid in &chat.peers {
            if !seen.contains(pid) {
                let name = resolve_display_name(&es, &profiles, pid);
                result.push((*pid, name, true));
            }
        }
        result
    }

    pub async fn is_connected(&self) -> bool {
        willow_actor::state::select(&self.network_meta_addr, |n| n.connected).await
    }

    pub async fn has_permission(
        &self,
        peer_id: &willow_identity::EndpointId,
        perm: &willow_state::Permission,
    ) -> bool {
        let pid = *peer_id;
        let p = perm.clone();
        willow_actor::state::select(&self.event_state_addr, move |es| {
            es.has_permission(&pid, &p)
        })
        .await
    }

    pub async fn roles_data(&self) -> Vec<(String, String, Vec<String>)> {
        willow_actor::state::select(&self.event_state_addr, |es| {
            let mut entries: Vec<(String, String, Vec<String>)> = es
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
        willow_actor::state::select(&self.server_registry_addr, |reg| {
            let mut unread_map = HashMap::new();
            if let Some(entry) = reg.active() {
                for (topic, count) in &entry.unread {
                    if let Some(name) = entry.name_for_topic(topic) {
                        unread_map.insert(name.to_string(), *count);
                    }
                }
            }
            unread_map
        })
        .await
    }

    pub async fn server_owner(&self) -> willow_identity::EndpointId {
        willow_actor::state::select(&self.event_state_addr, |es| es.owner).await
    }

    pub async fn channel_kinds(&self) -> Vec<(String, String)> {
        willow_actor::state::select(&self.event_state_addr, |es| {
            es.channels
                .values()
                .map(|ch| (ch.name.clone(), ch.kind.clone()))
                .collect()
        })
        .await
    }

    pub async fn current_channel(&self) -> String {
        willow_actor::state::select(&self.chat_meta_addr, |c| c.current_channel.clone()).await
    }
}
