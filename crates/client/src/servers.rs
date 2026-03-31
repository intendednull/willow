use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    /// Switch to a different server by ID.
    pub fn switch_server(&self, server_id: &str) {
        let mut shared = self.shared.write().unwrap();
        if shared.state.servers.contains_key(server_id) {
            shared.state.active_server = Some(server_id.to_string());
            init_event_state_on_shared(&mut shared, server_id);
            reconcile_topic_map(&mut shared.state);
        }
    }

    /// List all servers as (id, name) pairs.
    pub fn server_list(&self) -> Vec<(String, String)> {
        self.shared.read().unwrap().state.server_list()
    }

    /// Get the name of the currently active server.
    pub fn active_server_name(&self) -> String {
        let shared = self.shared.read().unwrap();
        shared
            .state
            .active()
            .map(|ctx| ctx.server.name.clone())
            .unwrap_or_else(|| "No Server".to_string())
    }

    /// Get the ID of the currently active server.
    pub fn active_server_id(&self) -> Option<String> {
        self.shared.read().unwrap().state.active_server.clone()
    }

    /// Check whether any servers exist.
    pub fn has_servers(&self) -> bool {
        !self.shared.read().unwrap().state.servers.is_empty()
    }

    /// Remove a server from the local state and persist the change.
    ///
    /// If the removed server was active, switches to the first remaining
    /// server (or clears the active server if none remain).
    pub fn leave_server(&self, server_id: &str) {
        let mut shared = self.shared.write().unwrap();
        shared.state.servers.remove(server_id);
        if shared.state.active_server.as_deref() == Some(server_id) {
            shared.state.active_server = shared.state.servers.keys().next().cloned();
        }
        // Persist updated server list.
        let ids: Vec<String> = shared.state.servers.keys().cloned().collect();
        storage::save_server_list(&ids);
    }

    /// Create a brand-new server with the local user as owner.
    ///
    /// Automatically creates a "general" text channel, initializes the
    /// event-sourced state, persists everything, and subscribes to the
    /// channel topic on the network.
    ///
    /// Returns the server ID.
    pub fn create_server(&self, name: &str) -> anyhow::Result<String> {
        let mut shared = self.shared.write().unwrap();
        let mut server = willow_channel::Server::new(name, shared.identity.endpoint_id());
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

        shared.state.servers.insert(server_id.clone(), ctx);
        shared.state.active_server = Some(server_id.clone());
        shared.state.chat.current_channel = "general".to_string();

        // Initialize event-sourced state for this server.
        let peer_id = shared.identity.endpoint_id();
        shared.state.event_state =
            willow_state::ServerState::new(server_id.clone(), name.to_string(), peer_id);

        // Open event store.
        if shared.config.persistence {
            if let Some(store) = storage::open_event_store(&server_id) {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    shared.state.event_store = state::PersistentEventStore::Sqlite(store);
                }
                #[cfg(target_arch = "wasm32")]
                {
                    shared.state.event_store = state::PersistentEventStore::LocalStorage(store);
                }
            }
        }

        // Create the general channel via event.
        let create_ch = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: ch_id_str,
                kind: "text".to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &create_ch);

        // Persist.
        if shared.config.persistence {
            persist_servers(&shared.state);
        }

        // Subscription to the channel topic will happen via connect() or
        // a future subscribe_topic() call. For now, just note it.

        Ok(server_id)
    }

    /// Grant SyncProvider permission to the given worker peer IDs.
    ///
    /// Called during server creation for each worker the user wants to
    /// authorize. Workers need SyncProvider to serve state.
    pub fn authorize_workers(&self, worker_peer_ids: &[willow_identity::EndpointId]) {
        let mut events_to_broadcast = Vec::new();
        {
            let mut shared = self.shared.write().unwrap();
            let peer_id = shared.identity.endpoint_id();
            for worker_pid in worker_peer_ids {
                let event = willow_state::Event {
                    id: uuid::Uuid::new_v4().to_string(),
                    parent_hash: shared.state.event_state.hash(),
                    author: peer_id,
                    timestamp_ms: util::current_time_ms(),
                    kind: willow_state::EventKind::GrantPermission {
                        peer_id: *worker_pid,
                        permission: willow_state::Permission::SyncProvider,
                    },
                };
                apply_event_on_shared(&mut shared, &event);
                events_to_broadcast.push(event);
            }
        }
        // Broadcast after releasing the borrow.
        for event in events_to_broadcast {
            self.broadcast_event(&event);
        }
    }

    /// Set display name for the active server via event-sourced state.
    pub fn set_server_display_name(&self, name: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.write().unwrap();
        if shared.state.active_server.is_none() {
            return Err(anyhow::anyhow!("no active server"));
        }
        let peer_id = shared.identity.endpoint_id();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author: peer_id,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::SetProfile {
                display_name: name.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        drop(shared);

        self.broadcast_event(&event);

        // Also update the global profile for backward compat.
        let mut shared = self.shared.write().unwrap();
        let pid = shared.identity.endpoint_id();
        shared.state.profiles.names.insert(pid, name.to_string());

        storage::save_profile(&storage::LocalProfile {
            display_name: name.to_string(),
        });
        drop(shared);

        self.broadcast_profile_via_network();

        Ok(())
    }

    /// Get the display name for the active server (from event-sourced state).
    pub fn server_display_name(&self) -> String {
        let shared = self.shared.read().unwrap();
        let peer_id = shared.identity.endpoint_id();
        shared
            .state
            .event_state
            .profiles
            .get(&peer_id)
            .map(|p| p.display_name.clone())
            .unwrap_or_else(|| {
                // Fall back to legacy profile store.
                if let Some(profile) = shared.state.event_state.profiles.get(&peer_id) {
                    return profile.display_name.clone();
                }
                shared.state.profiles.display_name(&peer_id)
            })
    }

    /// Rename the server. Only the owner can do this.
    pub fn rename_server(&self, new_name: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.write().unwrap();
        let author = shared.identity.endpoint_id();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::RenameServer {
                new_name: new_name.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        drop(shared);
        self.broadcast_event(&event);
        Ok(())
    }

    /// Set the server description. Only the owner can do this.
    pub fn set_server_description(&self, desc: &str) -> anyhow::Result<()> {
        let mut shared = self.shared.write().unwrap();
        let author = shared.identity.endpoint_id();
        let event = willow_state::Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: shared.state.event_state.hash(),
            author,
            timestamp_ms: util::current_time_ms(),
            kind: willow_state::EventKind::SetServerDescription {
                description: desc.to_string(),
            },
        };
        apply_event_on_shared(&mut shared, &event);
        drop(shared);
        self.broadcast_event(&event);
        Ok(())
    }
}
