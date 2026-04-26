//! # Client Mutations
//!
//! Typed mutation interface that routes operations to domain-specific
//! `StateActor`s. User code calls methods on this handle; it builds
//! events, applies them, broadcasts, and emits `ClientEvent`s.
//!
//! ```ignore
//! client.mutations().send_message("general", "hello").await?;
//! client.mutations().create_channel("dev").await?;
//! client.mutations().toggle_mute().await;
//! ```

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use parking_lot::Mutex;

use willow_actor::{Addr, Broker, Publish, StateActor};
use willow_identity::{EndpointId, Identity};
use willow_network::TopicHandle as _;
use willow_state::{EventHash, EventKind};

use crate::state_actors::DagState;

use crate::events::ClientEvent;
use crate::ops;
use crate::persistence_actor::{self, PersistenceActor};
use crate::state_actors::*;
use crate::util;

/// Typed mutation interface. Routes operations to domain actors.
///
/// Cloneable — pass to UI handlers, listener tasks, etc.
pub struct ClientMutations<N: willow_network::Network> {
    pub(crate) event_state: Addr<StateActor<willow_state::ServerState>>,
    pub(crate) server_registry: Addr<StateActor<ServerRegistry>>,
    pub(crate) chat_meta: Addr<StateActor<ChatMeta>>,
    pub(crate) profiles: Addr<StateActor<ProfileState>>,
    pub(crate) network: Addr<StateActor<NetworkMeta>>,
    pub(crate) voice: Addr<StateActor<VoiceState>>,
    pub(crate) event_broker: Addr<Broker<ClientEvent>>,
    pub(crate) persistence: Addr<PersistenceActor>,
    /// Whether persistence to disk is enabled.
    pub(crate) persistence_enabled: bool,
    pub(crate) identity: Identity,
    pub(crate) dag: Addr<StateActor<DagState>>,
    pub(crate) join_links: Arc<Mutex<Vec<crate::ops::JoinLink>>>,
    pub(crate) topics: Arc<RwLock<HashMap<String, N::Topic>>>,
    /// Phase 2b: shared handle to the sync-queue actor, so mutations
    /// that touch the outbound queue / inbound marks flow through the
    /// same bus as everything else.
    pub(crate) queue_meta: Addr<StateActor<QueueMeta>>,
}

impl<N: willow_network::Network> Clone for ClientMutations<N> {
    fn clone(&self) -> Self {
        Self {
            event_state: self.event_state.clone(),
            server_registry: self.server_registry.clone(),
            chat_meta: self.chat_meta.clone(),
            profiles: self.profiles.clone(),
            network: self.network.clone(),
            voice: self.voice.clone(),
            event_broker: self.event_broker.clone(),
            persistence: self.persistence.clone(),
            persistence_enabled: self.persistence_enabled,
            identity: self.identity.clone(),
            dag: self.dag.clone(),
            join_links: Arc::clone(&self.join_links),
            topics: Arc::clone(&self.topics),
            queue_meta: self.queue_meta.clone(),
        }
    }
}

// ───── Internal helpers ──────────────────────────────────────────────────

impl<N: willow_network::Network> ClientMutations<N> {
    /// Seed the DAG with a genesis `CreateServer` event and materialize
    /// the initial `ServerState`. Must be called once before any other
    /// events are created.
    pub(crate) async fn seed_genesis(&self, server_name: &str) {
        let identity = self.identity.clone();
        let name = server_name.to_string();
        let ts = util::current_time_ms();
        let state = willow_actor::state::mutate(&self.dag, move |ds| {
            // Idempotent: if genesis already exists, return current state.
            if ds.managed.dag().genesis().is_some() {
                return ds.managed.state().clone();
            }
            let genesis = ds.managed.dag().create_event(
                &identity,
                EventKind::CreateServer { name },
                vec![],
                ts,
            );
            ds.managed
                .insert_and_apply(genesis)
                .expect("genesis event must insert successfully");
            ds.managed.state().clone()
        })
        .await;
        willow_actor::state::mutate(&self.event_state, move |es| {
            *es = state;
        })
        .await;
    }

    /// Build an event and atomically insert it into the DAG and apply
    /// to state. ManagedDag ensures the DAG and ServerState are always
    /// in sync — no separate apply step needed.
    pub(crate) async fn build_event(&self, kind: EventKind) -> anyhow::Result<willow_state::Event> {
        let identity = self.identity.clone();
        let ts = util::current_time_ms();
        let dag = self.dag.clone();
        util::with_timeout("build_event", async move {
            willow_actor::state::mutate(&dag, move |ds| {
                ds.managed
                    .create_and_insert(&identity, kind, ts)
                    .map_err(|e| anyhow::anyhow!("DAG insert failed: {e:?}"))
            })
            .await
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?
    }

    /// Resolve channel name → channel ID via event state.
    pub(crate) async fn resolve_channel_id(&self, channel: &str) -> anyhow::Result<String> {
        let ch = channel.to_string();
        let event_state = self.event_state.clone();
        let channel_id = util::with_timeout("resolve_channel_id", async move {
            willow_actor::state::select(&event_state, move |es| {
                es.channels
                    .iter()
                    .find(|(_, c)| c.name == ch)
                    .map(|(id, _)| id.clone())
            })
            .await
        })
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        channel_id.ok_or_else(|| anyhow::anyhow!("channel not found: {channel}"))
    }

    /// Sync event_state mirror from ManagedDag, persist, and emit
    /// ClientEvents. Called after build_event succeeds — ManagedDag has
    /// already applied the event to its internal state atomically.
    pub(crate) async fn apply_event(&self, event: &willow_state::Event) {
        // Sync event_state mirror from ManagedDag's authoritative state.
        let dag = self.dag.clone();
        let state = match util::with_timeout("apply_event/select_dag", async move {
            willow_actor::state::select(&dag, |ds| ds.managed.state().clone()).await
        })
        .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "apply_event: dag state read timed out, skipping event_state sync");
                return;
            }
        };
        let event_state = self.event_state.clone();
        if util::with_timeout("apply_event/mutate_event_state", async move {
            willow_actor::state::mutate(&event_state, move |es| {
                *es = state;
            })
            .await
        })
        .await
        .is_err()
        {
            tracing::warn!("apply_event: event_state mutate timed out");
        }
        self.persistence
            .do_send(persistence_actor::PersistEvent {
                event: event.clone(),
            })
            .ok();
        // Persist the state snapshot so messages survive a page reload without
        // requiring a network sync round-trip. Call storage directly (synchronous)
        // so the snapshot is guaranteed to be on disk before this await point —
        // fire-and-forget actor messages may be delayed past a page reload.
        // Key by registry UUID (not ServerState.server_id which is the genesis
        // hash) so that load_server_state() can find the saved snapshot.
        if self.persistence_enabled {
            let snapshot_state =
                willow_actor::state::select(&self.dag, |ds| ds.managed.state().clone()).await;
            let registry_id =
                willow_actor::state::select(&self.server_registry, |reg| reg.active_server.clone())
                    .await;
            if let Some(id) = registry_id {
                crate::storage::save_server_state(&id, &snapshot_state);
            }
        }
        let client_events = derive_client_events(event);
        for e in client_events {
            self.event_broker.do_send(Publish(e)).ok();
        }
    }

    /// Broadcast a signed event to peers via the server ops topic.
    pub(crate) fn broadcast_event(&self, event: &willow_state::Event) {
        if let Some(data) = ops::pack_wire(&ops::WireMessage::Event(event.clone()), &self.identity)
        {
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }

    /// Fire-and-forget broadcast of raw data on a named topic.
    pub(crate) fn broadcast_on_topic(&self, topic: &str, data: Vec<u8>) {
        let topics = self.topics.read().unwrap_or_else(|e| e.into_inner());
        let Some(handle) = topics.get(topic).cloned() else {
            tracing::warn!(
                topic,
                "broadcast_on_topic: topic not subscribed, message dropped"
            );
            return;
        };
        drop(topics);
        let bytes = bytes::Bytes::from(data);
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move {
                    handle.broadcast(bytes).await.ok();
                });
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                handle.broadcast(bytes).await.ok();
            });
        }
    }
}

// ───── Chat mutations ────────────────────────────────────────────────────

impl<N: willow_network::Network> ClientMutations<N> {
    /// Send a text message to a channel.
    pub async fn send_message(&self, channel: &str, body: &str) -> anyhow::Result<()> {
        let channel_id = self.resolve_channel_id(channel).await?;
        let event = self
            .build_event(EventKind::Message {
                channel_id,
                body: body.to_string(),
                reply_to: None,
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Send a reply to a specific message.
    pub async fn send_reply(
        &self,
        channel: &str,
        parent_hash: &EventHash,
        body: &str,
    ) -> anyhow::Result<()> {
        let channel_id = self.resolve_channel_id(channel).await?;
        let event = self
            .build_event(EventKind::Message {
                channel_id,
                body: body.to_string(),
                reply_to: Some(*parent_hash),
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Edit an existing message.
    pub async fn edit_message(&self, message_id: &EventHash, new_body: &str) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::EditMessage {
                message_id: *message_id,
                new_body: new_body.to_string(),
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Delete a message.
    pub async fn delete_message(&self, message_id: &EventHash) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::DeleteMessage {
                message_id: *message_id,
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// React to a message.
    pub async fn react(&self, message_id: &EventHash, emoji: &str) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::Reaction {
                message_id: *message_id,
                emoji: emoji.to_string(),
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Pin a message.
    pub async fn pin_message(&self, channel: &str, message_id: &EventHash) -> anyhow::Result<()> {
        let channel_id = self.resolve_channel_id(channel).await?;
        let event = self
            .build_event(EventKind::PinMessage {
                channel_id,
                message_id: *message_id,
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Unpin a message.
    pub async fn unpin_message(&self, channel: &str, message_id: &EventHash) -> anyhow::Result<()> {
        let channel_id = self.resolve_channel_id(channel).await?;
        let event = self
            .build_event(EventKind::UnpinMessage {
                channel_id,
                message_id: *message_id,
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Switch the current channel.
    pub async fn switch_channel(&self, channel: &str) {
        let ch = channel.to_string();
        willow_actor::state::mutate(&self.chat_meta, move |c| {
            c.current_channel = ch;
        })
        .await;
    }
}

// ───── Server mutations ──────────────────────────────────────────────────

impl<N: willow_network::Network> ClientMutations<N> {
    /// Create a new text channel.
    pub async fn create_channel(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let name_for_switch = name.clone();

        let ch_id_str = uuid::Uuid::new_v4().to_string();

        let event = self
            .build_event(EventKind::CreateChannel {
                name: name.clone(),
                channel_id: ch_id_str,
                kind: willow_state::ChannelKind::Text,
                ephemeral: None,
            })
            .await?;
        self.apply_event(&event).await;

        // Switch to new channel.
        self.switch_channel(&name_for_switch).await;

        self.broadcast_event(&event);
        Ok(())
    }

    /// Create a non-permanent ("ephemeral") channel that
    /// auto-archives after `idle_threshold_ms` of inactivity.
    ///
    /// Spec: `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`.
    pub async fn create_ephemeral_channel(
        &self,
        name: &str,
        kind: willow_state::EphemeralKind,
        idle_threshold_ms: u64,
    ) -> anyhow::Result<()> {
        let name = name.to_string();
        let name_for_switch = name.clone();
        let ch_id_str = uuid::Uuid::new_v4().to_string();

        let event = self
            .build_event(EventKind::CreateChannel {
                name: name.clone(),
                channel_id: ch_id_str,
                kind: willow_state::ChannelKind::Text,
                ephemeral: Some(willow_state::EphemeralConfig {
                    kind,
                    idle_threshold_ms,
                }),
            })
            .await?;
        self.apply_event(&event).await;
        self.switch_channel(&name_for_switch).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Revive an auto-archived ephemeral channel by name without
    /// posting a message.
    pub async fn revive_channel(&self, name: &str) -> anyhow::Result<()> {
        let ch_id_str = self.resolve_channel_id(name).await?;
        let event = self
            .build_event(EventKind::ChannelRevive {
                channel_id: ch_id_str,
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Delete a channel.
    pub async fn delete_channel(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let name_for_check = name.clone();

        // Resolve channel ID from event state.
        let ch_id_str = self.resolve_channel_id(&name).await?;

        let event = self
            .build_event(EventKind::DeleteChannel {
                channel_id: ch_id_str,
            })
            .await?;
        self.apply_event(&event).await;

        // Remove the channel key from registry.
        let name_for_key = name.clone();
        willow_actor::state::mutate(&self.server_registry, move |reg| {
            if let Some(entry) = reg.active_mut() {
                let topic = crate::util::make_topic(&entry.server_id, &name_for_key);
                entry.keys.remove(&topic);
            }
        })
        .await;

        // Switch to first remaining channel if we deleted the current one.
        let current =
            willow_actor::state::select(&self.chat_meta, |c| c.current_channel.clone()).await;
        if current == name_for_check {
            let first = willow_actor::state::select(&self.event_state, |es| {
                es.channels
                    .values()
                    .map(|ch| ch.name.clone())
                    .next()
                    .unwrap_or_default()
            })
            .await;
            self.switch_channel(&first).await;
        }

        self.broadcast_event(&event);
        Ok(())
    }

    /// Grant a non-admin permission to a peer.
    pub async fn grant_permission(
        &self,
        peer_id: EndpointId,
        permission: willow_state::Permission,
    ) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::GrantPermission {
                peer_id,
                permission,
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Revoke a non-admin permission from a peer.
    pub async fn revoke_permission(
        &self,
        peer_id: EndpointId,
        permission: willow_state::Permission,
    ) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::RevokePermission {
                peer_id,
                permission,
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Propose granting admin status to a peer (requires admin vote).
    pub async fn propose_grant_admin(&self, peer_id: EndpointId) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::Propose {
                action: willow_state::ProposedAction::GrantAdmin { peer_id },
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Propose revoking admin status from a peer (requires admin vote).
    ///
    /// Also revokes the peer's `SendMessages` permission so their future
    /// messages are rejected by the state machine — i.e., a full "untrust".
    pub async fn propose_revoke_admin(&self, peer_id: EndpointId) -> anyhow::Result<()> {
        // Governance proposal: remove from admin set (may require majority).
        let event = self
            .build_event(EventKind::Propose {
                action: willow_state::ProposedAction::RevokeAdmin { peer_id },
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);

        // Immediate admin action: revoke message-sending rights so the peer
        // can no longer write to this server regardless of proposal timing.
        // Ignore errors (caller may not be admin, or peer had no SendMessages).
        let _ = self
            .revoke_permission(peer_id, willow_state::Permission::SendMessages)
            .await;

        Ok(())
    }

    /// Propose kicking a member (requires admin vote).
    pub async fn propose_kick_member(&self, peer_id: EndpointId) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::Propose {
                action: willow_state::ProposedAction::KickMember { peer_id },
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Propose changing the vote threshold (requires admin vote).
    pub async fn propose_set_threshold(
        &self,
        threshold: willow_state::VoteThreshold,
    ) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::Propose {
                action: willow_state::ProposedAction::SetVoteThreshold { threshold },
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Vote on a proposal (accept or reject).
    pub async fn vote(&self, proposal_hash: &EventHash, accept: bool) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::Vote {
                proposal: *proposal_hash,
                accept,
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Create a new role.
    pub async fn create_role(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let role_id = uuid::Uuid::new_v4().to_string();
        let event = self
            .build_event(EventKind::CreateRole { name, role_id })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Delete a role.
    pub async fn delete_role(&self, role_id: &str) -> anyhow::Result<()> {
        let role_id = role_id.to_string();
        let event = self.build_event(EventKind::DeleteRole { role_id }).await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Mute or unmute a channel for the local identity only.
    ///
    /// Not admin-gated — every member can silence their own
    /// notifications. Emits a `ClientEvent::MuteChanged` so the UI can
    /// update the pill outline and the Notifier can bypass the surface.
    pub async fn mutate_channel_mute(&self, channel: &str, muted: bool) -> anyhow::Result<()> {
        let channel_id = self.resolve_channel_id(channel).await?;
        let event = self
            .build_event(EventKind::MuteChannel {
                channel_id: channel_id.clone(),
                muted,
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        self.event_broker
            .do_send(Publish(ClientEvent::MuteChanged {
                scope: crate::events::MuteScope::Channel(channel_id),
                muted,
            }))
            .ok();
        Ok(())
    }

    /// Mute or unmute the entire grove for the local identity only.
    pub async fn mutate_grove_mute(&self, muted: bool) -> anyhow::Result<()> {
        let event = self.build_event(EventKind::MuteGrove { muted }).await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        self.event_broker
            .do_send(Publish(ClientEvent::MuteChanged {
                scope: crate::events::MuteScope::Grove,
                muted,
            }))
            .ok();
        Ok(())
    }
}

// ───── Voice mutations ───────────────────────────────────────────────────

impl<N: willow_network::Network> ClientMutations<N> {
    /// Join a voice channel.
    pub async fn join_voice(&self, channel_id: &str) {
        let in_voice =
            willow_actor::state::select(&self.voice, |v| v.active_channel.is_some()).await;
        if in_voice {
            self.leave_voice().await;
        }
        let ch = channel_id.to_string();
        let my_peer_id = self.identity.endpoint_id();
        willow_actor::state::mutate(&self.voice, move |v| {
            v.active_channel = Some(ch.clone());
            v.participants.entry(ch).or_default().insert(my_peer_id);
        })
        .await;
        let msg = ops::WireMessage::VoiceJoin {
            channel_id: channel_id.to_string(),
            peer_id: my_peer_id,
        };
        if let Some(data) = ops::pack_wire(&msg, &self.identity) {
            self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
        }
    }

    /// Leave the current voice channel.
    pub async fn leave_voice(&self) {
        let my_peer_id = self.identity.endpoint_id();
        let maybe_ch = willow_actor::state::mutate(&self.voice, move |v| {
            if let Some(ch) = v.active_channel.take() {
                if let Some(p) = v.participants.get_mut(&ch) {
                    p.remove(&my_peer_id);
                }
                Some(ch)
            } else {
                None
            }
        })
        .await;
        if let Some(ch) = maybe_ch {
            let msg = ops::WireMessage::VoiceLeave {
                channel_id: ch,
                peer_id: my_peer_id,
            };
            if let Some(data) = ops::pack_wire(&msg, &self.identity) {
                self.broadcast_on_topic(ops::SERVER_OPS_TOPIC, data);
            }
        }
    }

    /// Toggle mute. Returns new muted state.
    pub async fn toggle_mute(&self) -> bool {
        willow_actor::state::mutate(&self.voice, |v| {
            v.muted = !v.muted;
            v.muted
        })
        .await
    }

    /// Toggle deafen. Returns new deafened state.
    pub async fn toggle_deafen(&self) -> bool {
        willow_actor::state::mutate(&self.voice, |v| {
            v.deafened = !v.deafened;
            v.deafened
        })
        .await
    }
}

// ───── Network mutations (called by listeners) ──────────────────────────

impl<N: willow_network::Network> ClientMutations<N> {
    /// Track a peer as online.
    pub async fn peer_connected(&self, peer_id: EndpointId) {
        willow_actor::state::mutate(&self.chat_meta, move |c| {
            if !c.peers.contains(&peer_id) {
                c.peers.push(peer_id);
            }
        })
        .await;
        self.event_broker
            .do_send(Publish(ClientEvent::PeerConnected(peer_id)))
            .ok();
    }

    /// Track a peer as offline.
    pub async fn peer_disconnected(&self, peer_id: EndpointId) {
        willow_actor::state::mutate(&self.chat_meta, move |c| {
            c.peers.retain(|p| p != &peer_id);
        })
        .await;
        self.event_broker
            .do_send(Publish(ClientEvent::PeerDisconnected(peer_id)))
            .ok();
    }

    /// Build + apply + broadcast an [`EventKind::UpdateProfile`].
    ///
    /// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
    /// §Editing — self. Called from the Settings Profile tab; the
    /// popover itself never inlines edits.
    pub async fn update_profile_fields(
        &self,
        delta: crate::views::ProfileDelta,
    ) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::UpdateProfile(Box::new(delta)))
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Update a peer's display name from a profile broadcast.
    pub async fn update_profile(&self, peer_id: EndpointId, display_name: String) {
        let name = display_name.clone();
        willow_actor::state::mutate(&self.profiles, move |p| {
            p.names.insert(peer_id, name);
        })
        .await;
        self.event_broker
            .do_send(Publish(ClientEvent::ProfileUpdated {
                peer_id,
                display_name,
            }))
            .ok();
    }

    /// Record a typing indicator from a peer.
    pub async fn record_typing(&self, peer_id: EndpointId, channel: String) {
        let now = util::current_time_ms();
        willow_actor::state::mutate(&self.network, move |n| {
            n.typing_peers.insert(peer_id, (channel, now));
        })
        .await;
        // Also ensure peer is tracked.
        willow_actor::state::mutate(&self.chat_meta, move |c| {
            if !c.peers.contains(&peer_id) {
                c.peers.push(peer_id);
            }
        })
        .await;
    }

    /// Mark the network as connected.
    pub async fn set_connected(&self, connected: bool) {
        willow_actor::state::mutate(&self.network, move |n| {
            n.connected = connected;
        })
        .await;
    }

    /// Handle a voice join event from a peer.
    pub async fn voice_peer_joined(&self, channel_id: String, peer_id: EndpointId) {
        let ch = channel_id.clone();
        willow_actor::state::mutate(&self.voice, move |v| {
            v.participants.entry(ch).or_default().insert(peer_id);
        })
        .await;
        self.event_broker
            .do_send(Publish(ClientEvent::VoiceJoined {
                channel_id,
                peer_id,
            }))
            .ok();
    }

    /// Handle a voice leave event from a peer.
    pub async fn voice_peer_left(&self, channel_id: String, peer_id: EndpointId) {
        let ch = channel_id.clone();
        willow_actor::state::mutate(&self.voice, move |v| {
            if let Some(p) = v.participants.get_mut(&ch) {
                p.remove(&peer_id);
            }
        })
        .await;
        self.event_broker
            .do_send(Publish(ClientEvent::VoiceLeft {
                channel_id,
                peer_id,
            }))
            .ok();
    }
}

// ───── Sync-queue mutations (Phase 2b) ──────────────────────────────────

impl<N: willow_network::Network> ClientMutations<N> {
    /// Attempt a best-effort retry of every queued outbound message.
    ///
    /// Today this walks `QueueMeta::outbound` and stamps a fresh
    /// `last_attempt_at` tick on every entry. A real retry schedules a
    /// reconnect ping against each unique recipient via the network
    /// layer — that plumbing lands with the retry-schedule follow-up
    /// (plan §Scope — *Out: reachability probing / retry scheduling
    /// wire protocol*).
    ///
    /// No-op when the queue is empty. Always emits a `QueueChanged`
    /// event so the UI re-renders and the spinner clears.
    pub async fn retry_queue(&self) -> anyhow::Result<()> {
        willow_actor::state::mutate(&self.queue_meta, |qm| {
            let now = qm.now;
            for entry in qm.outbound.values_mut() {
                entry.last_attempt_at = Some(now);
                entry.last_attempt_error = None;
            }
        })
        .await;
        let view = willow_actor::state::get(&self.queue_meta).await;
        let snapshot = crate::views::compute_queue_view(&Arc::new((*view).clone()));
        self.event_broker
            .do_send(Publish(ClientEvent::QueueChanged(snapshot)))
            .ok();
        Ok(())
    }

    /// Stamp the `mark as read locally` marker for a single peer's
    /// inbound queue (Phase 2b sync-queue screen, inbound tab footer).
    ///
    /// Never touches message bodies — the stamp is a local-only tick
    /// annotation used by the UI to hide the inbound-hint badge.
    pub async fn mark_queue_read(&self, peer_id: EndpointId) -> anyhow::Result<()> {
        willow_actor::state::mutate(&self.queue_meta, move |qm| {
            qm.mark_read(peer_id);
        })
        .await;
        let view = willow_actor::state::get(&self.queue_meta).await;
        let snapshot = crate::views::compute_queue_view(&Arc::new((*view).clone()));
        self.event_broker
            .do_send(Publish(ClientEvent::QueueChanged(snapshot)))
            .ok();
        Ok(())
    }

    /// Update the relay-reachability snapshot. Called by the network
    /// layer + the (future) relay-probe listener.
    pub async fn set_relay_status(&self, status: crate::queue::RelayStatus) {
        willow_actor::state::mutate(&self.queue_meta, move |qm| {
            qm.set_relay_status(status);
        })
        .await;
        self.event_broker
            .do_send(Publish(ClientEvent::RelayStatusChanged(status)))
            .ok();
    }

    /// Update the device-online signal. Called by the WASM
    /// `window.addEventListener('online'/'offline')` path in
    /// `connect.rs`.
    pub async fn set_device_online(&self, online: bool) {
        willow_actor::state::mutate(&self.queue_meta, move |qm| {
            qm.set_device_online(online);
        })
        .await;
        self.event_broker
            .do_send(Publish(ClientEvent::DeviceOnlineChanged(online)))
            .ok();
    }
}

// ───── derive_client_events (pure function) ──────────────────────────────

/// Convert a [`willow_state::Event`] into zero or more [`ClientEvent`]s.
///
/// Pure function — no I/O. Maps event kinds to client-visible notifications.
pub(crate) fn derive_client_events(event: &willow_state::Event) -> Vec<ClientEvent> {
    let mut out = Vec::new();
    match &event.kind {
        EventKind::Message { channel_id, .. } => {
            out.push(ClientEvent::MessageReceived {
                channel: channel_id.clone(),
                message_id: event.hash.to_string(),
                is_local: false, // Caller can override based on identity check.
            });
        }
        EventKind::CreateChannel { name, .. } => {
            out.push(ClientEvent::ChannelCreated(name.clone()));
        }
        EventKind::DeleteChannel { channel_id } => {
            out.push(ClientEvent::ChannelDeleted(channel_id.clone()));
        }
        EventKind::CreateRole { name, role_id } => {
            out.push(ClientEvent::RoleCreated {
                name: name.clone(),
                role_id: role_id.clone(),
            });
        }
        EventKind::DeleteRole { role_id } => {
            out.push(ClientEvent::RoleDeleted {
                role_id: role_id.clone(),
            });
        }
        EventKind::Propose { action } => {
            out.push(ClientEvent::ProposalCreated {
                proposal_hash: event.hash.to_string(),
                action_description: format!("{action:?}"),
            });
        }
        EventKind::Vote { proposal, accept } => {
            out.push(ClientEvent::VoteCast {
                proposal_hash: proposal.to_string(),
                accept: *accept,
                voter: event.author,
            });
        }
        EventKind::GrantPermission { peer_id, .. } => {
            out.push(ClientEvent::PeerTrusted(*peer_id));
        }
        EventKind::RevokePermission { peer_id, .. } => {
            out.push(ClientEvent::PeerUntrusted(*peer_id));
        }
        EventKind::EditMessage {
            message_id,
            new_body,
        } => {
            out.push(ClientEvent::MessageEdited {
                channel: String::new(), // Resolved by view layer.
                message_id: message_id.to_string(),
                new_body: new_body.clone(),
            });
        }
        EventKind::DeleteMessage { message_id } => {
            out.push(ClientEvent::MessageDeleted {
                channel: String::new(),
                message_id: message_id.to_string(),
            });
        }
        EventKind::PinMessage {
            channel_id,
            message_id,
        } => {
            out.push(ClientEvent::MessagePinned {
                channel: channel_id.clone(),
                message_id: message_id.to_string(),
            });
        }
        EventKind::UnpinMessage {
            channel_id,
            message_id,
        } => {
            out.push(ClientEvent::MessageUnpinned {
                channel: channel_id.clone(),
                message_id: message_id.to_string(),
            });
        }
        EventKind::RenameServer { new_name } => {
            out.push(ClientEvent::ServerRenamed {
                new_name: new_name.clone(),
            });
        }
        EventKind::SetServerDescription { description } => {
            out.push(ClientEvent::ServerDescriptionChanged {
                description: description.clone(),
            });
        }
        EventKind::Reaction {
            message_id, emoji, ..
        } => {
            out.push(ClientEvent::ReactionAdded {
                channel: String::new(),
                message_id: message_id.to_string(),
                emoji: emoji.clone(),
                author: event.author,
            });
        }
        EventKind::MuteChannel { channel_id, muted } => {
            out.push(ClientEvent::MuteChanged {
                scope: crate::events::MuteScope::Channel(channel_id.clone()),
                muted: *muted,
            });
        }
        EventKind::MuteGrove { muted } => {
            out.push(ClientEvent::MuteChanged {
                scope: crate::events::MuteScope::Grove,
                muted: *muted,
            });
        }
        _ => {}
    }
    out
}
