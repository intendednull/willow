use super::*;

impl<N: willow_network::Network> ClientHandle<N> {
    pub async fn generate_invite(
        &self,
        recipient_peer_id: &willow_identity::EndpointId,
    ) -> anyhow::Result<String> {
        // Enforce the CreateInvite permission up front so non-authorised
        // peers cannot mint invites, matching the guard used by
        // `create_join_link` (see issue #225 / SEC-A-02).
        let pid = self.identity.endpoint_id();
        let has_perm = willow_actor::state::select(&self.event_state_addr, move |es| {
            es.has_permission(&pid, &willow_state::Permission::CreateInvite)
        })
        .await;
        if !has_perm {
            anyhow::bail!("missing CreateInvite permission");
        }

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

        // Use the authoritative `state.genesis_author` recorded at server
        // creation rather than picking an arbitrary admin. Picking from
        // `admins.iter().next()` is non-deterministic and wrong whenever
        // additional admins have been promoted (see issue #241 / SEC-A-08).
        // Fall back to the local peer id only for legacy states that were
        // serialized before `genesis_author` existed.
        let my_id = self.identity.endpoint_id();
        let genesis_author = willow_actor::state::select(&self.event_state_addr, move |es| {
            es.genesis_author.unwrap_or(my_id)
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

    /// Broadcast a `JoinRequest` to the inviter for `link_id` and
    /// record the pending attempt so subsequent `JoinResponse` /
    /// `JoinDenied` messages can be authenticated against the expected
    /// `inviter_peer_id`.
    ///
    /// `inviter_peer_id` is taken straight from the
    /// [`ops::JoinToken::inviter_peer_id`] the caller decoded; the
    /// listener uses it to drop spoofed responses signed by anyone else
    /// (issue #309 / SEC-A-07).
    pub fn send_join_request(&self, link_id: &str, inviter_peer_id: willow_identity::EndpointId) {
        // Record the pending attempt BEFORE broadcasting. If the inviter
        // races us and replies before we've populated the map, the
        // listener would otherwise drop the legitimate response with a
        // "no outstanding join request" debug log.
        self.pending_joins
            .lock()
            .insert(link_id.to_string(), inviter_peer_id);
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
            p.insert_name(pid, name);
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
            // Keep map + recency in lockstep: helper drops both.
            n.sweep_typing(now, crate::TYPING_INDICATOR_TTL_MS);
            n.typing_peers
                .iter()
                .filter(|(pid, (ch, _))| ch == &channel && *pid != &my_id)
                .map(|(pid, _)| views::resolve_display_name(&es, &profiles, pid))
                .collect()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the client-side auth guards on invite generation.
    //!
    //! These cover the fixes for SEC-A-02 (#225) and SEC-A-08 (#241):
    //!   * `generate_invite` must refuse to run when the caller lacks the
    //!     `CreateInvite` permission.
    //!   * `generate_invite` must record the real `genesis_author` in the
    //!     invite payload rather than an arbitrary admin.
    use crate::test_client;
    use willow_state::Permission;

    /// #225 — callers without `CreateInvite` must be rejected.
    ///
    /// The owner of a server implicitly holds every permission, so we
    /// first strip that status by overwriting `event_state.admins` and
    /// `event_state.genesis_author` with an identity we do not control.
    /// After that, the test client is a plain member and should not be
    /// able to mint invites.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generate_invite_requires_create_invite_permission() {
        let (client, _rx) = test_client();

        // Install a foreign genesis_author and admin set so the local
        // identity no longer owns the server. Without this the caller is
        // the implicit owner and `has_permission` always returns true.
        let stranger = willow_identity::Identity::generate().endpoint_id();
        willow_actor::state::mutate(&client.event_state_addr, move |es| {
            es.genesis_author = Some(stranger);
            es.admins.clear();
            es.admins.insert(stranger);
            es.peer_permissions.clear();
        })
        .await;

        // Sanity check: the local peer really lacks CreateInvite now.
        let local_pid = client.identity.endpoint_id();
        let has_perm = willow_actor::state::select(&client.event_state_addr, move |es| {
            es.has_permission(&local_pid, &Permission::CreateInvite)
        })
        .await;
        assert!(
            !has_perm,
            "test setup failed: local peer should not hold CreateInvite"
        );

        // A synthetic recipient for whom we would generate the invite.
        let bob = willow_identity::Identity::generate().endpoint_id();
        let err = client
            .generate_invite(&bob)
            .await
            .expect_err("generate_invite must refuse without CreateInvite");
        assert!(
            err.to_string().contains("CreateInvite"),
            "error should name the missing permission, got: {err}"
        );
    }

    /// #241 — the invite's `genesis_author` must be the authoritative
    /// server owner recorded in state, not an arbitrarily-ordered admin.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generate_invite_uses_state_genesis_author() {
        let (client, _rx) = test_client();

        // The local identity is the real genesis author on the test
        // server. Promote a *second* peer to admin; then scramble the
        // admin set so that `admins.iter().next()` would pick that peer
        // rather than the real owner. A correct implementation reads
        // `state.genesis_author` and so is unaffected by this ordering.
        let owner_id = client.identity.endpoint_id();
        let other_admin = willow_identity::Identity::generate().endpoint_id();
        let fake_admin = willow_identity::Identity::generate().endpoint_id();
        willow_actor::state::mutate(&client.event_state_addr, move |es| {
            // Keep the real genesis_author untouched.
            es.admins.clear();
            // Insert admins such that `iter().next()` (BTreeSet
            // ordering by EndpointId) returns a non-owner. We insert
            // both; whichever sorts first will be picked by the buggy
            // code path — neither is the real owner.
            es.admins.insert(other_admin);
            es.admins.insert(fake_admin);
            // Grant the local peer the CreateInvite permission
            // explicitly since they are no longer an admin by membership.
            es.peer_permissions
                .entry(owner_id)
                .or_default()
                .insert(Permission::CreateInvite);
        })
        .await;

        // Confirm the buggy `admins.iter().next()` would have produced
        // a non-owner — otherwise the test wouldn't actually catch #241.
        let (would_be_wrong, real_owner) =
            willow_actor::state::select(&client.event_state_addr, |es| {
                (es.admins.iter().next().copied(), es.genesis_author)
            })
            .await;
        assert_ne!(
            would_be_wrong, real_owner,
            "test setup failed: admins.iter().next() must differ from genesis_author",
        );

        let bob = willow_identity::Identity::generate().endpoint_id();
        let code = client
            .generate_invite(&bob)
            .await
            .expect("owner with CreateInvite must mint invites");

        // Decode the invite and verify the embedded genesis_author is
        // the real one, not the first-by-iteration admin.
        let raw = crate::base64::decode(&code).expect("invite must base64-decode");
        let payload: crate::invite::InvitePayload =
            willow_transport::unpack(&raw).expect("invite must deserialize");
        assert_eq!(
            payload.genesis_author,
            real_owner.expect("real_owner must be set"),
            "invite must carry state.genesis_author, not admins.iter().next()"
        );
        assert_ne!(
            Some(payload.genesis_author),
            would_be_wrong,
            "invite must not carry the arbitrary admin picked by ordering"
        );
    }
}
