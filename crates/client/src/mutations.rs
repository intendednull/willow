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
use std::sync::{Arc, Mutex, RwLock};

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
    pub(crate) identity: Identity,
    pub(crate) dag: Addr<StateActor<DagState>>,
    pub(crate) join_links: Arc<Mutex<Vec<crate::ops::JoinLink>>>,
    pub(crate) topics: Arc<RwLock<HashMap<String, N::Topic>>>,
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
            identity: self.identity.clone(),
            dag: self.dag.clone(),
            join_links: Arc::clone(&self.join_links),
            topics: Arc::clone(&self.topics),
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
            let genesis =
                ds.dag
                    .create_event(&identity, EventKind::CreateServer { name }, vec![], ts);
            ds.dag
                .insert(genesis)
                .expect("genesis event must insert successfully");
            willow_state::materialize(&ds.dag)
        })
        .await;
        willow_actor::state::mutate(&self.event_state, move |es| {
            *es = state;
        })
        .await;
    }

    /// Build an event and atomically insert it into the DAG.
    ///
    /// Atomicity is guaranteed by the actor mailbox — only one mutation
    /// runs at a time, so no concurrent mutation can produce events with
    /// the same seq/prev.
    pub(crate) async fn build_event(&self, kind: EventKind) -> anyhow::Result<willow_state::Event> {
        let identity = self.identity.clone();
        let ts = util::current_time_ms();
        willow_actor::state::mutate(&self.dag, move |ds| {
            let my_id = identity.endpoint_id();
            let deps: Vec<EventHash> = ds
                .dag
                .authors()
                .filter(|a| **a != my_id)
                .filter_map(|a| ds.dag.head(a).copied())
                .collect();
            let event = ds.dag.create_event(&identity, kind, deps, ts);
            ds.dag
                .insert(event.clone())
                .map_err(|e| anyhow::anyhow!("DAG insert failed: {e:?}"))?;
            Ok(event)
        })
        .await
    }

    /// Resolve channel name → channel ID via event state + server registry.
    pub(crate) async fn resolve_channel_id(&self, channel: &str) -> anyhow::Result<String> {
        let ch = channel.to_string();
        let channel_id = willow_actor::state::select(&self.event_state, move |es| {
            es.channels
                .iter()
                .find(|(_, c)| c.name == ch)
                .map(|(id, _)| id.clone())
        })
        .await;
        if let Some(id) = channel_id {
            return Ok(id);
        }
        // Fall back to server registry topic_map.
        let ch2 = channel.to_string();
        let from_registry = willow_actor::state::select(&self.server_registry, move |reg| {
            reg.active().and_then(|entry| {
                entry
                    .topic_map
                    .values()
                    .find(|(n, _)| n == &ch2)
                    .map(|(_, cid)| cid.to_string())
            })
        })
        .await;
        from_registry.ok_or_else(|| anyhow::anyhow!("channel not found: {channel}"))
    }

    /// Apply an already-inserted event: update materialized state, persist,
    /// and emit ClientEvents. The event MUST already be in the DAG.
    pub(crate) async fn apply_event(&self, event: &willow_state::Event) {
        let event_clone = event.clone();
        willow_actor::state::mutate(&self.event_state, move |es| {
            willow_state::apply_incremental(es, &event_clone);
        })
        .await;
        let _ = self.persistence.do_send(persistence_actor::PersistEvent {
            event: event.clone(),
        });
        let client_events = derive_client_events(event);
        for e in client_events {
            let _ = self.event_broker.do_send(Publish(e));
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
            return;
        };
        drop(topics);
        let bytes = bytes::Bytes::from(data);
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move {
                    let _ = handle.broadcast(bytes).await;
                });
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local(async move {
                let _ = handle.broadcast(bytes).await;
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
        let name_for_event = name.clone();
        let name_for_switch = name.clone();

        // Create channel in Server object and update topic_map.
        let ch_id_str = willow_actor::state::mutate(
            &self.server_registry,
            move |reg| -> anyhow::Result<String> {
                let entry = reg
                    .active_mut()
                    .ok_or_else(|| anyhow::anyhow!("no active server"))?;
                let ch_id = entry
                    .server
                    .create_channel(&name, willow_channel::ChannelKind::Text)?;
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
            .build_event(EventKind::CreateChannel {
                name: name_for_event,
                channel_id: ch_id_str,
                kind: "text".to_string(),
            })
            .await?;
        self.apply_event(&event).await;

        // Switch to new channel.
        self.switch_channel(&name_for_switch).await;

        self.broadcast_event(&event);
        Ok(())
    }

    /// Delete a channel.
    pub async fn delete_channel(&self, name: &str) -> anyhow::Result<()> {
        let name = name.to_string();
        let name_for_check = name.clone();
        let ch_id_str = willow_actor::state::mutate(
            &self.server_registry,
            move |reg| -> anyhow::Result<String> {
                let entry = reg
                    .active_mut()
                    .ok_or_else(|| anyhow::anyhow!("no active server"))?;
                let (topic, ch_id) = entry
                    .topic_map
                    .iter()
                    .find(|(_, (n, _))| n == &name)
                    .map(|(t, (_, cid))| (t.clone(), cid.clone()))
                    .ok_or_else(|| anyhow::anyhow!("channel not found"))?;
                let ch_id_str = ch_id.to_string();
                entry.server.delete_channel(&ch_id)?;
                entry.topic_map.remove(&topic);
                entry.keys.remove(&topic);
                Ok(ch_id_str)
            },
        )
        .await?;

        let event = self
            .build_event(EventKind::DeleteChannel {
                channel_id: ch_id_str,
            })
            .await?;
        self.apply_event(&event).await;

        // Switch to first remaining channel if we deleted the current one.
        let current =
            willow_actor::state::select(&self.chat_meta, |c| c.current_channel.clone()).await;
        if current == name_for_check {
            let first = willow_actor::state::select(&self.server_registry, |reg| {
                reg.active()
                    .map(|e| e.channel_names())
                    .and_then(|names| names.into_iter().next())
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
    pub async fn propose_revoke_admin(&self, peer_id: EndpointId) -> anyhow::Result<()> {
        let event = self
            .build_event(EventKind::Propose {
                action: willow_state::ProposedAction::RevokeAdmin { peer_id },
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
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
        let role_id = willow_channel::RoleId::new();
        let role = willow_channel::Role::with_id(role_id.clone(), &name);
        willow_actor::state::mutate(&self.server_registry, |reg| -> anyhow::Result<()> {
            let entry = reg
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;
            entry.server.create_role(role);
            Ok(())
        })
        .await?;
        let event = self
            .build_event(EventKind::CreateRole {
                name,
                role_id: role_id.to_string(),
            })
            .await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Delete a role.
    pub async fn delete_role(&self, role_id: &str) -> anyhow::Result<()> {
        let role_id = role_id.to_string();
        let rid = willow_channel::RoleId(
            uuid::Uuid::parse_str(&role_id).map_err(|e| anyhow::anyhow!("invalid role_id: {e}"))?,
        );
        willow_actor::state::mutate(&self.server_registry, move |reg| -> anyhow::Result<()> {
            let entry = reg
                .active_mut()
                .ok_or_else(|| anyhow::anyhow!("no active server"))?;
            entry.server.delete_role(&rid)?;
            Ok(())
        })
        .await?;
        let event = self.build_event(EventKind::DeleteRole { role_id }).await?;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
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
        let _ = self
            .event_broker
            .do_send(Publish(ClientEvent::PeerConnected(peer_id)));
    }

    /// Track a peer as offline.
    pub async fn peer_disconnected(&self, peer_id: EndpointId) {
        willow_actor::state::mutate(&self.chat_meta, move |c| {
            c.peers.retain(|p| p != &peer_id);
        })
        .await;
        let _ = self
            .event_broker
            .do_send(Publish(ClientEvent::PeerDisconnected(peer_id)));
    }

    /// Update a peer's display name from a profile broadcast.
    pub async fn update_profile(&self, peer_id: EndpointId, display_name: String) {
        let name = display_name.clone();
        willow_actor::state::mutate(&self.profiles, move |p| {
            p.names.insert(peer_id, name);
        })
        .await;
        let _ = self
            .event_broker
            .do_send(Publish(ClientEvent::ProfileUpdated {
                peer_id,
                display_name,
            }));
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
        let _ = self.event_broker.do_send(Publish(ClientEvent::VoiceJoined {
            channel_id,
            peer_id,
        }));
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
        let _ = self.event_broker.do_send(Publish(ClientEvent::VoiceLeft {
            channel_id,
            peer_id,
        }));
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
        _ => {}
    }
    out
}
