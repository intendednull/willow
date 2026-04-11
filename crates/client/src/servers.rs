use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    pub async fn switch_server(&self, server_id: &str) {
        let target_sid = server_id.to_string();

        // Check the target exists and get the current active server ID.
        let (target_exists, current_sid) =
            willow_actor::state::select(&self.server_registry_addr, {
                let tid = target_sid.clone();
                move |reg| (reg.servers.contains_key(&tid), reg.active_server.clone())
            })
            .await;

        if !target_exists {
            return;
        }

        // Stash the current server's DAG and restore the target's.
        let tid = target_sid.clone();
        let state = willow_actor::state::mutate(&self.dag_addr, move |ds| {
            // Stash current DAG under the old server ID.
            if let Some(old_id) = current_sid {
                ds.stashed.insert(old_id, ds.managed.clone());
            }
            // Restore the target server's DAG (or create an empty one).
            if let Some(restored) = ds.stashed.remove(&tid) {
                ds.managed = restored;
            } else {
                ds.managed = willow_state::ManagedDag::empty(5000);
            }
            ds.managed.state().clone()
        })
        .await;

        // Update event_state from the restored DAG's materialized state.
        willow_actor::state::mutate(&self.event_state_addr, move |es| {
            *es = state;
        })
        .await;

        // Update active server in registry.
        let sid = target_sid.clone();
        willow_actor::state::mutate(&self.server_registry_addr, move |reg| {
            reg.active_server = Some(sid);
        })
        .await;

        // Reset current channel to "general" (may not exist on new server).
        willow_actor::state::mutate(&self.chat_meta_addr, |chat| {
            chat.current_channel = "general".to_string();
        })
        .await;
    }

    pub async fn server_list(&self) -> Vec<(String, String)> {
        willow_actor::state::select(&self.server_registry_addr, |reg| reg.server_list()).await
    }

    pub async fn active_server_name(&self) -> String {
        willow_actor::state::select(&self.server_registry_addr, |reg| {
            reg.active()
                .map(|e| e.name.clone())
                .unwrap_or_else(|| "No Server".to_string())
        })
        .await
    }

    pub async fn active_server_id(&self) -> Option<String> {
        willow_actor::state::select(&self.server_registry_addr, |reg| reg.active_server.clone())
            .await
    }

    pub async fn has_servers(&self) -> bool {
        willow_actor::state::select(&self.server_registry_addr, |reg| !reg.servers.is_empty()).await
    }

    pub async fn leave_server(&self, server_id: &str) {
        let sid = server_id.to_string();
        willow_actor::state::mutate(&self.server_registry_addr, move |reg| {
            reg.servers.remove(&sid);
            if reg.active_server.as_deref() == Some(&sid) {
                reg.active_server = reg.servers.keys().next().cloned();
            }
        })
        .await;
    }

    pub async fn create_server(&self, name: &str) -> anyhow::Result<String> {
        let name = name.to_string();
        let name_for_state = name.clone();
        let peer_id = self.identity.endpoint_id();

        let (server_id, ch_id_str) = willow_actor::state::mutate(
            &self.server_registry_addr,
            move |reg| -> anyhow::Result<(String, String)> {
                let mut server = willow_channel::Server::new(&name, peer_id);
                let server_id = server.id().to_string();
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
                topic_map.insert(topic, ("general".to_string(), ch_id));
                reg.servers.insert(
                    server_id.clone(),
                    state_actors::ServerEntry {
                        server,
                        name: name.to_string(),
                        topic_map,
                        keys,
                        unread: HashMap::new(),
                    },
                );
                reg.active_server = Some(server_id.clone());
                Ok((server_id, ch_id_str))
            },
        )
        .await?;

        // Stash the current server's DAG before seeding the new one.
        let old_sid = self.active_server_id().await;
        if let Some(old_id) = old_sid {
            let oid = old_id.clone();
            willow_actor::state::mutate(&self.dag_addr, move |ds| {
                ds.stashed.insert(oid, ds.managed.clone());
                // Reset managed to empty so seed_genesis creates fresh state.
                ds.managed = willow_state::ManagedDag::empty(5000);
            })
            .await;
        }

        // Seed the DAG with a genesis event and materialize initial state.
        self.mutation_handle.seed_genesis(&name_for_state).await;

        // Switch current channel.
        self.mutation_handle.switch_channel("general").await;

        // Create channel via event (DAG is now seeded, so insert will succeed).
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: ch_id_str,
                kind: "text".to_string(),
            })
            .await?;
        self.mutation_handle.apply_event(&event).await;

        // Open event store on persistence actor.
        let _ = self
            .persistence_addr
            .do_send(persistence_actor::OpenEventStore {
                server_id: server_id.clone(),
            });

        Ok(server_id)
    }

    pub async fn authorize_workers(
        &self,
        worker_peer_ids: &[willow_identity::EndpointId],
    ) -> anyhow::Result<()> {
        for worker_pid in worker_peer_ids {
            let event = self
                .mutation_handle
                .build_event(willow_state::EventKind::GrantPermission {
                    peer_id: *worker_pid,
                    permission: willow_state::Permission::SyncProvider,
                })
                .await?;
            self.mutation_handle.apply_event(&event).await;
            self.mutation_handle.broadcast_event(&event);
        }
        Ok(())
    }

    pub async fn set_server_display_name(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::SetProfile {
                display_name: name.clone(),
            })
            .await?;
        self.mutation_handle.apply_event(&event).await;

        // Also update global profile.
        let pid = self.identity.endpoint_id();
        willow_actor::state::mutate(&self.profile_state_addr, move |p| {
            p.names.insert(pid, name);
        })
        .await;

        self.mutation_handle.broadcast_event(&event);
        self.broadcast_profile_via_network();
        Ok(())
    }

    pub async fn server_display_name(&self) -> String {
        self.display_name().await
    }

    pub async fn rename_server(&self, new_name: &str) -> anyhow::Result<()> {
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::RenameServer {
                new_name: new_name.to_string(),
            })
            .await?;
        self.mutation_handle.apply_event(&event).await;
        self.mutation_handle.broadcast_event(&event);
        Ok(())
    }

    pub async fn set_server_description(&self, desc: &str) -> anyhow::Result<()> {
        let event = self
            .mutation_handle
            .build_event(willow_state::EventKind::SetServerDescription {
                description: desc.to_string(),
            })
            .await?;
        self.mutation_handle.apply_event(&event).await;
        self.mutation_handle.broadcast_event(&event);
        Ok(())
    }
}
