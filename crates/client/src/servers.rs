use super::*;
use crate::client_actor::{mutate_state, read_state};

impl<N: willow_network::Network> ClientHandle<N> {
    /// Switch to a different server by ID.
    pub async fn switch_server(&self, server_id: &str) {
        let sid = server_id.to_string();
        mutate_state(&self.state_addr, move |s| {
            if s.state.servers.contains_key(&sid) {
                s.state.active_server = Some(sid.clone());
                init_event_state_on_shared(s, &sid);
                reconcile_topic_map(&mut s.state);
            }
        })
        .await;
    }

    /// List all servers as (id, name) pairs.
    pub async fn server_list(&self) -> Vec<(String, String)> {
        read_state(&self.state_addr, |s| s.state.server_list()).await
    }

    /// Get the name of the currently active server.
    pub async fn active_server_name(&self) -> String {
        read_state(&self.state_addr, |s| {
            s.state
                .active()
                .map(|ctx| ctx.server.name.clone())
                .unwrap_or_else(|| "No Server".to_string())
        })
        .await
    }

    /// Get the ID of the currently active server.
    pub async fn active_server_id(&self) -> Option<String> {
        read_state(&self.state_addr, |s| s.state.active_server.clone()).await
    }

    /// Check whether any servers exist.
    pub async fn has_servers(&self) -> bool {
        read_state(&self.state_addr, |s| !s.state.servers.is_empty()).await
    }

    /// Remove a server from the local state and persist the change.
    ///
    /// If the removed server was active, switches to the first remaining
    /// server (or clears the active server if none remain).
    pub async fn leave_server(&self, server_id: &str) {
        let sid = server_id.to_string();
        mutate_state(&self.state_addr, move |s| {
            s.state.servers.remove(&sid);
            if s.state.active_server.as_deref() == Some(&sid) {
                s.state.active_server = s.state.servers.keys().next().cloned();
            }
            // Persist updated server list.
            let ids: Vec<String> = s.state.servers.keys().cloned().collect();
            storage::save_server_list(&ids);
        })
        .await;
    }

    /// Create a brand-new server with the local user as owner.
    ///
    /// Automatically creates a "general" text channel, initializes the
    /// event-sourced state, persists everything, and subscribes to the
    /// channel topic on the network.
    ///
    /// Returns the server ID.
    pub async fn create_server(&self, name: &str) -> anyhow::Result<String> {
        let name = name.to_string();
        mutate_state(&self.state_addr, move |s| {
            let mut server = willow_channel::Server::new(&name, s.identity.endpoint_id());
            let server_id = server.id.to_string();

            // Create default "general" channel.
            let ch_id = server
                .create_channel("general", willow_channel::ChannelKind::Text)
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            let topic = util::make_topic(&server, "general");

            let mut topic_map = HashMap::new();
            let mut keys = HashMap::new();

            if let Some(key) = server.channel_key(&ch_id) {
                keys.insert(topic.clone(), key.clone());
            }
            let ch_id_str = ch_id.to_string();
            topic_map.insert(topic.clone(), ("general".to_string(), ch_id));

            let ctx = ServerContext {
                server,
                topic_map,
                keys,
                unread: HashMap::new(),
            };

            s.state.servers.insert(server_id.clone(), ctx);
            s.state.active_server = Some(server_id.clone());
            s.state.chat.current_channel = "general".to_string();

            // Initialize event-sourced state for this server.
            let peer_id = s.identity.endpoint_id();
            s.state.event_state =
                willow_state::ServerState::new(server_id.clone(), name.to_string(), peer_id);

            // Open event store.
            if s.config.persistence {
                if let Some(store) = storage::open_event_store(&server_id) {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        s.state.event_store = state::PersistentEventStore::Sqlite(store);
                    }
                    #[cfg(target_arch = "wasm32")]
                    {
                        s.state.event_store = state::PersistentEventStore::LocalStorage(store);
                    }
                }
            }

            // Create the general channel via event.
            let create_ch = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::CreateChannel {
                    name: "general".to_string(),
                    channel_id: ch_id_str,
                    kind: "text".to_string(),
                },
            };
            apply_event_on_shared(s, &create_ch);

            // Persist.
            if s.config.persistence {
                persist_servers(&s.state);
            }

            Ok(server_id)
        })
        .await
    }

    /// Grant SyncProvider permission to the given worker peer IDs.
    ///
    /// Called during server creation for each worker the user wants to
    /// authorize. Workers need SyncProvider to serve state.
    pub async fn authorize_workers(&self, worker_peer_ids: &[willow_identity::EndpointId]) {
        let worker_peer_ids = worker_peer_ids.to_vec();
        let events_to_broadcast = mutate_state(&self.state_addr, move |s| {
            let peer_id = s.identity.endpoint_id();
            let mut events = Vec::new();
            for worker_pid in &worker_peer_ids {
                let event = willow_state::Event {
                    id: uuid::Uuid::new_v4().to_string(),
                    parent_hash: s.state.event_state.hash(),
                    author: peer_id,
                    timestamp_ms: util::current_time_ms(),
                    kind: willow_state::EventKind::GrantPermission {
                        peer_id: *worker_pid,
                        permission: willow_state::Permission::SyncProvider,
                    },
                };
                apply_event_on_shared(s, &event);
                events.push(event);
            }
            events
        })
        .await;
        // Broadcast after releasing the borrow.
        for event in events_to_broadcast {
            self.broadcast_event(&event);
        }
    }

    /// Set display name for the active server via event-sourced state.
    pub async fn set_server_display_name(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            if s.state.active_server.is_none() {
                return Err(anyhow::anyhow!("no active server"));
            }
            let peer_id = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author: peer_id,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::SetProfile {
                    display_name: name.clone(),
                },
            };
            apply_event_on_shared(s, &event);

            // Also update the global profile for backward compat.
            let pid = s.identity.endpoint_id();
            s.state.profiles.names.insert(pid, name.clone());

            storage::save_profile(&storage::LocalProfile { display_name: name });

            Ok(event)
        })
        .await?;

        self.broadcast_event(&event);
        self.broadcast_profile_via_network();

        Ok(())
    }

    /// Get the display name for the active server (from event-sourced state).
    pub async fn server_display_name(&self) -> String {
        read_state(&self.state_addr, |s| {
            let peer_id = s.identity.endpoint_id();
            s.state
                .event_state
                .profiles
                .get(&peer_id)
                .map(|p| p.display_name.clone())
                .unwrap_or_else(|| {
                    // Fall back to legacy profile store.
                    if let Some(profile) = s.state.event_state.profiles.get(&peer_id) {
                        return profile.display_name.clone();
                    }
                    s.state.profiles.display_name(&peer_id)
                })
        })
        .await
    }

    /// Rename the server. Only the owner can do this.
    pub async fn rename_server(&self, new_name: &str) -> anyhow::Result<()> {
        let new_name = new_name.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let author = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::RenameServer { new_name },
            };
            apply_event_on_shared(s, &event);
            event
        })
        .await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Set the server description. Only the owner can do this.
    pub async fn set_server_description(&self, desc: &str) -> anyhow::Result<()> {
        let desc = desc.to_string();
        let event = mutate_state(&self.state_addr, move |s| {
            let author = s.identity.endpoint_id();
            let event = willow_state::Event {
                id: uuid::Uuid::new_v4().to_string(),
                parent_hash: s.state.event_state.hash(),
                author,
                timestamp_ms: util::current_time_ms(),
                kind: willow_state::EventKind::SetServerDescription { description: desc },
            };
            apply_event_on_shared(s, &event);
            event
        })
        .await;
        self.broadcast_event(&event);
        Ok(())
    }
}
