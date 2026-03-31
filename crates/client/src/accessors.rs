use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    /// Get a clone of the local identity.
    ///
    /// Useful for constructing network configurations that require the
    /// identity's secret key.
    pub fn identity(&self) -> Identity {
        self.identity.clone()
    }

    /// Get the state actor address.
    pub fn state_addr(&self) -> &willow_actor::Addr<client_actor::ClientStateActor> {
        &self.state_addr
    }
    /// Get the actor system handle.
    pub fn actor_system(&self) -> &willow_actor::SystemHandle {
        &self.system
    }
    /// Notify the state actor that shared state was mutated.
    pub fn notify_mutation(&self) {
        let _ = self.state_addr.do_send(client_actor::NotifyMutation);
    }
    /// Get the local PeerId as a string.
    pub fn peer_id(&self) -> String {
        self.identity.endpoint_id().to_string()
    }

    /// Get the local display name.
    ///
    /// Checks the event-sourced state profiles first, falling back to the
    /// legacy profile store.
    pub fn display_name(&self) -> String {
        let shared = self.shared.read().unwrap();
        let pid = shared.identity.endpoint_id();
        if let Some(profile) = shared.state.event_state.profiles.get(&pid) {
            return profile.display_name.clone();
        }
        shared.state.profiles.display_name(&pid)
    }

    /// Get a peer's display name.
    ///
    /// Checks the event-sourced state profiles first, falling back to the
    /// legacy profile store.
    pub fn peer_display_name(&self, peer_id: &willow_identity::EndpointId) -> String {
        let shared = self.shared.read().unwrap();
        peer_display_name_shared(&shared, peer_id)
    }

    /// Get messages for a channel, computed from the event-sourced state.
    pub fn messages(&self, channel: &str) -> Vec<state::DisplayMessage> {
        let shared = self.shared.read().unwrap();
        // Collect ALL channel_ids that map to this channel name.
        // This handles the ID mismatch between owner and joiner.
        let mut channel_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        // From topic_map (legacy).
        if let Some(ctx) = shared.state.active() {
            for (name, cid) in ctx.topic_map.values() {
                if name == channel {
                    channel_ids.insert(cid.to_string());
                }
            }
        }

        // From event_state.channels (authoritative).
        for (id, ch) in &shared.state.event_state.channels {
            if ch.name == channel {
                channel_ids.insert(id.clone());
            }
        }

        if channel_ids.is_empty() {
            return vec![];
        }

        let local_peer_id = shared.identity.endpoint_id();

        let mut msgs: Vec<state::DisplayMessage> = shared
            .state
            .event_state
            .messages
            .iter()
            .filter(|m| channel_ids.contains(&m.channel_id))
            .map(|m| {
                let author_name = shared
                    .state
                    .event_state
                    .profiles
                    .get(&m.author)
                    .map(|p| p.display_name.clone())
                    .unwrap_or_else(|| shared.state.profiles.display_name(&m.author));

                let reply_preview = m.reply_to.as_ref().and_then(|parent_id| {
                    shared
                        .state
                        .event_state
                        .messages
                        .iter()
                        .find(|pm| pm.id == *parent_id)
                        .map(|pm| {
                            let parent_name = shared
                                .state
                                .event_state
                                .profiles
                                .get(&pm.author)
                                .map(|p| p.display_name.clone())
                                .unwrap_or_else(|| shared.state.profiles.display_name(&pm.author));
                            let text = if pm.body.len() > 50 {
                                format!("{}...", &pm.body[..50])
                            } else {
                                pm.body.clone()
                            };
                            format!("{parent_name}: {text}")
                        })
                });

                // Resolve reaction author names.
                let reactions = m
                    .reactions
                    .iter()
                    .map(|(emoji, peer_ids)| {
                        let names: Vec<String> = peer_ids
                            .iter()
                            .map(|pid| {
                                shared
                                    .state
                                    .event_state
                                    .profiles
                                    .get(pid)
                                    .map(|p| p.display_name.clone())
                                    .unwrap_or_else(|| shared.state.profiles.display_name(pid))
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
    }

    /// List all channel names for the active server.
    ///
    /// Returns the union of channels from the server context and the
    /// event-sourced state, deduplicated and sorted.
    pub fn channels(&self) -> Vec<String> {
        let shared = self.shared.read().unwrap();
        let mut names = shared.state.channel_names();

        // Merge any channels from event_state that aren't yet in the list.
        for ch in shared.state.event_state.channels.values() {
            if !names.contains(&ch.name) {
                names.push(ch.name.clone());
            }
        }

        names.sort();
        names.dedup();
        names
    }

    /// Get messages from the event-sourced state for a channel by ID.
    ///
    /// Returns all non-deleted messages for the given channel in
    /// event-sequence order (owned copies).
    pub fn event_messages(&self, channel_id: &str) -> Vec<willow_state::ChatMessage> {
        let shared = self.shared.read().unwrap();
        shared
            .state
            .event_state
            .messages
            .iter()
            .filter(|m| m.channel_id == channel_id && !m.deleted)
            .cloned()
            .collect()
    }

    /// Get the list of connected peers (network-level connections).
    pub fn peers(&self) -> Vec<willow_identity::EndpointId> {
        self.shared.read().unwrap().state.chat.peers.clone()
    }

    /// Get all server members with online/offline status.
    ///
    /// Returns `(peer_id, display_name, is_online)` for each member.
    /// Falls back to `chat.peers` for peers not in the member list
    /// (e.g. connected before event sync completes).
    pub fn server_members(&self) -> Vec<(willow_identity::EndpointId, String, bool)> {
        let shared = self.shared.read().unwrap();
        let local_id = shared.identity.endpoint_id();
        let online: std::collections::HashSet<willow_identity::EndpointId> =
            shared.state.chat.peers.iter().copied().collect();

        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Members from event-sourced state.
        for (pid, member) in &shared.state.event_state.members {
            let name = member
                .display_name
                .clone()
                .or_else(|| {
                    shared
                        .state
                        .event_state
                        .profiles
                        .get(pid)
                        .map(|p| p.display_name.clone())
                })
                .unwrap_or_else(|| peer_display_name_shared(&shared, pid));
            // Local user is always online.
            let is_online = *pid == local_id || online.contains(pid);
            result.push((*pid, name, is_online));
            seen.insert(*pid);
        }

        // Connected peers not yet in the member list (pre-sync).
        for pid in &shared.state.chat.peers {
            if !seen.contains(pid) {
                let name = peer_display_name_shared(&shared, pid);
                result.push((*pid, name, true));
            }
        }

        result
    }

    /// Whether the network is connected.
    pub fn is_connected(&self) -> bool {
        self.shared.read().unwrap().connected
    }

    /// Check whether a peer has a specific permission.
    pub fn has_permission(
        &self,
        peer_id: &willow_identity::EndpointId,
        perm: &willow_state::Permission,
    ) -> bool {
        self.shared
            .read()
            .unwrap()
            .state
            .event_state
            .has_permission(peer_id, perm)
    }

    /// Returns role data from the event-sourced state as owned values.
    ///
    /// Each entry is `(role_id, role_name, permissions)`, sorted by name.
    pub fn roles_data(&self) -> Vec<(String, String, Vec<String>)> {
        let shared = self.shared.read().unwrap();
        let mut entries: Vec<(String, String, Vec<String>)> = shared
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
    }

    /// Returns unread counts keyed by channel name for the active server.
    pub fn unread_counts(&self) -> HashMap<String, usize> {
        let shared = self.shared.read().unwrap();
        let mut unread_map = HashMap::new();
        if let Some(ctx) = shared.state.active() {
            for (topic, count) in &ctx.unread {
                if let Some(name) = ctx.name_for_topic(topic) {
                    unread_map.insert(name.to_string(), *count);
                }
            }
        }
        unread_map
    }

    /// Returns the EndpointId of the server owner.
    pub fn server_owner(&self) -> willow_identity::EndpointId {
        self.shared.read().unwrap().state.event_state.owner
    }

    /// Returns `(channel_name, kind_str)` pairs for the active server's channels.
    /// `kind_str` is `"text"` or `"voice"`.
    pub fn channel_kinds(&self) -> Vec<(String, String)> {
        let shared = self.shared.read().unwrap();
        shared
            .state
            .event_state
            .channels
            .values()
            .map(|ch| (ch.name.clone(), ch.kind.clone()))
            .collect()
    }

    /// Returns the current channel name from the chat state.
    pub fn current_channel(&self) -> String {
        self.shared
            .read()
            .unwrap()
            .state
            .chat
            .current_channel
            .clone()
    }
}
