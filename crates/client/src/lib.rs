//! # Willow Client
//!
//! UI-agnostic client library for the Willow P2P chat network.
//! Use this crate to build bots, CLIs, TUIs, or alternative frontends.
//!
//! ## Quick start
//!
//! ```no_run
//! use willow_client::{Client, ClientConfig};
//!
//! let mut client = Client::new(ClientConfig::default());
//! client.connect();
//! loop {
//!     for event in client.poll() {
//!         // handle events
//!     }
//!     client.send_message("general", "hello!").ok();
//! }
//! ```

pub mod base64;
pub mod emoji;
pub mod events;
pub mod files;
pub mod invite;
pub mod network;
pub mod ops;
pub mod state;
pub mod storage;
pub mod util;

// Re-export key types at crate root for convenience.
pub use events::ClientEvent;
pub use ops::{Op, StampedOp, SyncMessage};
pub use state::{
    ChannelKeyStore, ChatMessage, ChatState, ClientState, OpLog, ProfileStore, ServerState,
    UnreadCounts,
};

use std::collections::HashMap;
use std::sync::mpsc as std_mpsc;

use willow_identity::Identity;
use willow_messaging::Content;

/// Configuration for creating a [`Client`].
pub struct ClientConfig {
    /// Optional relay multiaddr string for NAT traversal.
    pub relay_addr: Option<String>,
    /// Initial display name for the local user.
    pub display_name: Option<String>,
    /// Whether to persist state to disk. Defaults to `true`.
    pub persistence: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            relay_addr: None,
            display_name: None,
            persistence: true,
        }
    }
}

type DeferredPair = (
    std_mpsc::Sender<network::NetworkEvent>,
    std_mpsc::Receiver<network::NetworkCommand>,
);

/// UI-agnostic P2P chat client.
///
/// Wraps identity, networking, and all chat state. Call [`Client::poll()`]
/// each frame (or in a loop) to drain network events and receive
/// [`ClientEvent`]s.
pub struct Client {
    pub(crate) state: ClientState,
    pub(crate) identity: Identity,
    pub(crate) cmd_tx: std_mpsc::Sender<network::NetworkCommand>,
    pub(crate) event_rx: std_mpsc::Receiver<network::NetworkEvent>,
    pub(crate) connected: bool,
    pub(crate) config: ClientConfig,
    /// Holds the event_tx and cmd_rx until connect() consumes them.
    pub(crate) deferred_channels: Option<std::sync::Arc<std::sync::Mutex<Option<DeferredPair>>>>,
    /// Whether we have performed initial channel subscriptions.
    pub(crate) connected_subscribed: bool,
}

impl Client {
    /// Create a new client. Loads or generates identity, loads or creates
    /// the server with default channels, loads persisted messages and op log.
    ///
    /// Does **not** connect to the network -- call [`Client::connect()`] for that.
    pub fn new(config: ClientConfig) -> Self {
        let identity = load_identity();

        let (cmd_tx, cmd_rx) = std_mpsc::channel();
        let (event_tx, event_rx) = std_mpsc::channel();

        // We hold cmd_rx and event_tx until connect() is called.
        // Store them in a side channel via Mutex.
        let deferred = std::sync::Arc::new(std::sync::Mutex::new(Some((event_tx, cmd_rx))));

        let mut state = ClientState::default();

        // Open message database.
        if config.persistence {
            if let Some(db) = storage::open_message_db() {
                state.message_db = Some(std::sync::Arc::new(std::sync::Mutex::new(db)));
            }
        }

        // Load or create server.
        let (server, keys) = if let Some((server, keys)) = storage::load_server() {
            (server, keys)
        } else {
            let mut server = willow_channel::Server::new("My Server", identity.peer_id());
            let mut keys = HashMap::new();

            let default_channels = ["general", "random", "voice"];
            for name in default_channels {
                let ch_id = server
                    .create_channel(name, willow_channel::ChannelKind::Text)
                    .expect("default channel creation should not fail");

                let topic = util::make_topic(&server, name);
                if let Some(key) = server.channel_key(&ch_id) {
                    keys.insert(topic, key.clone());
                }
            }

            (server, keys)
        };

        // Build topic map.
        for ch in server.channels() {
            let topic = util::make_topic(&server, &ch.name);
            state
                .server
                .topic_map
                .insert(topic.clone(), (ch.name.clone(), ch.id.clone()));
        }

        state.key_store.keys = keys;
        if config.persistence {
            storage::save_server(&server, &state.key_store.keys);
        }

        // Load persisted messages.
        if let Some(ref db_arc) = state.message_db {
            if let Ok(db_lock) = db_arc.lock() {
                for topic in state.server.topic_map.keys() {
                    let stored = db_lock.load_topic(topic, 500);
                    for sm in stored {
                        state.chat.messages.push(ChatMessage::new(
                            sm.topic,
                            sm.author,
                            sm.body,
                            sm.is_local,
                            sm.timestamp_ms,
                        ));
                    }
                }
            }
        }

        // Load op log.
        if let Some(ops) = storage::load_op_log() {
            for op in ops {
                state.op_log.record(op);
            }
        }

        // Set display name if provided.
        if let Some(ref name) = config.display_name {
            state
                .profiles
                .names
                .insert(identity.peer_id().to_string(), name.clone());
            if config.persistence {
                storage::save_profile(&storage::LocalProfile {
                    display_name: name.clone(),
                });
            }
        }

        state.server.server = Some(server);

        Self {
            state,
            identity,
            cmd_tx,
            event_rx,
            connected: false,
            config,
            deferred_channels: Some(deferred),
            connected_subscribed: false,
        }
    }

    /// Connect to the P2P network. Spawns the network task in the background.
    ///
    /// After connecting, call [`Client::poll()`] regularly to process events.
    pub fn connect(&mut self) {
        if self.connected {
            return;
        }

        let Some(deferred) = self.deferred_channels.take() else {
            return;
        };
        let Ok(mut guard) = deferred.lock() else {
            return;
        };
        let Some((event_tx, cmd_rx)) = guard.take() else {
            return;
        };

        let config = network::build_network_config(self.config.relay_addr.as_deref());

        if self.config.persistence {
            storage::save_settings(&storage::NetworkSettings {
                relay_addr: self.config.relay_addr.clone(),
            });
        }

        network::spawn_network(self.identity.clone(), event_tx, cmd_rx, config);
        self.connected = true;
    }

    /// Drain network events, apply them to state, and return a list of
    /// [`ClientEvent`]s for the caller to handle.
    pub fn poll(&mut self) -> Vec<ClientEvent> {
        let mut events = Vec::new();

        while let Ok(net_event) = self.event_rx.try_recv() {
            match net_event {
                network::NetworkEvent::OpReceived { stamped_op, from } => {
                    let applied =
                        self.state
                            .apply_op(&stamped_op, &from, &self.identity, &self.cmd_tx);

                    if applied {
                        // Emit op-specific events.
                        match &stamped_op.op {
                            ops::Op::ChatMessage {
                                topic,
                                content_data,
                            } => {
                                self.state.process_chat_message(
                                    topic,
                                    content_data,
                                    &stamped_op.author,
                                    &stamped_op.op_id,
                                    stamped_op.hlc.millis,
                                    &stamped_op,
                                );
                                // Emit a MessageReceived event for the last
                                // pushed message.
                                if let Some(msg) = self.state.chat.messages.last() {
                                    let channel = self
                                        .state
                                        .server
                                        .name_for_topic(&msg.topic)
                                        .unwrap_or("unknown")
                                        .to_string();
                                    events.push(ClientEvent::MessageReceived {
                                        channel,
                                        message: msg.clone(),
                                    });
                                }
                            }
                            ops::Op::CreateChannel { name, .. } => {
                                events.push(ClientEvent::ChannelCreated(name.clone()));
                            }
                            ops::Op::DeleteChannel { name } => {
                                events.push(ClientEvent::ChannelDeleted(name.clone()));
                            }
                            ops::Op::CreateRole { name, role_id } => {
                                events.push(ClientEvent::RoleCreated {
                                    name: name.clone(),
                                    role_id: role_id.clone(),
                                });
                            }
                            ops::Op::DeleteRole { role_id } => {
                                events.push(ClientEvent::RoleDeleted {
                                    role_id: role_id.clone(),
                                });
                            }
                            ops::Op::KickMember { peer_id, .. } => {
                                events.push(ClientEvent::MemberKicked(peer_id.clone()));
                            }
                            ops::Op::TrustPeer { peer_id } => {
                                events.push(ClientEvent::PeerTrusted(peer_id.clone()));
                            }
                            ops::Op::UntrustPeer { peer_id } => {
                                events.push(ClientEvent::PeerUntrusted(peer_id.clone()));
                            }
                            _ => {}
                        }
                    }
                }
                network::NetworkEvent::PeerConnected(peer) => {
                    if !self.state.chat.peers.contains(&peer) {
                        self.state.chat.peers.push(peer.clone());
                    }
                    // On first peer connect, subscribe to channels.
                    if !self.connected_subscribed {
                        self.on_connected();
                        self.connected_subscribed = true;
                    }
                    events.push(ClientEvent::PeerConnected(peer));
                }
                network::NetworkEvent::PeerDisconnected(peer) => {
                    self.state.chat.peers.retain(|p| p != &peer);
                    events.push(ClientEvent::PeerDisconnected(peer));
                }
                network::NetworkEvent::ProfileReceived {
                    peer_id,
                    display_name,
                } => {
                    self.state
                        .profiles
                        .names
                        .insert(peer_id.clone(), display_name.clone());
                    events.push(ClientEvent::ProfileUpdated {
                        peer_id,
                        display_name,
                    });
                }
                network::NetworkEvent::Listening(addr) => {
                    // On receiving a listening event, do initial subscriptions.
                    if !self.connected_subscribed {
                        self.on_connected();
                        self.connected_subscribed = true;
                    }
                    events.push(ClientEvent::Listening(addr));
                }
                network::NetworkEvent::FileAnnounced {
                    filename,
                    size,
                    from,
                    topic,
                    ..
                } => {
                    let author = self.state.profiles.display_name(&from);
                    let size_kb = size / 1024;
                    let body = format!("[shared file: {filename} ({size_kb} KB)]");
                    let ts = self.state.chat.hlc.latest().millis;
                    self.state.chat.messages.push(ChatMessage::new(
                        topic.clone(),
                        author,
                        body,
                        false,
                        ts,
                    ));
                    self.state.chat.messages_dirty = true;

                    let channel = self
                        .state
                        .server
                        .name_for_topic(&topic)
                        .unwrap_or("unknown")
                        .to_string();
                    events.push(ClientEvent::FileAnnounced {
                        channel,
                        filename,
                        size,
                        from,
                    });
                }
                network::NetworkEvent::FileDownloaded { .. } => {
                    // no-op for now
                }
                network::NetworkEvent::SyncRequested {
                    latest_hlc,
                    from,
                    topic,
                } => {
                    let owner = self
                        .state
                        .server
                        .server
                        .as_ref()
                        .map(|s| s.owner.to_string())
                        .unwrap_or_default();
                    if self.state.op_log.is_trusted(&from, &owner) {
                        if let Some(ref req_topic) = topic {
                            if let Some(ref db_arc) = self.state.message_db {
                                if let Ok(db_lock) = db_arc.lock() {
                                    let chat_ops = db_lock.load_chat_ops_since(
                                        req_topic,
                                        latest_hlc.millis,
                                        latest_hlc.counter,
                                        500,
                                    );
                                    if !chat_ops.is_empty() {
                                        let _ = self.cmd_tx.send(
                                            network::NetworkCommand::SendSyncBatch {
                                                ops: chat_ops,
                                            },
                                        );
                                    }
                                }
                            }
                        } else {
                            let missing: Vec<_> = self
                                .state
                                .op_log
                                .ops
                                .iter()
                                .filter(|op| op.hlc > latest_hlc)
                                .cloned()
                                .collect();
                            if !missing.is_empty() {
                                let _ = self
                                    .cmd_tx
                                    .send(network::NetworkCommand::SendSyncBatch { ops: missing });
                            }
                        }
                    }
                }
                network::NetworkEvent::SyncBatchReceived { ops, from } => {
                    let owner = self
                        .state
                        .server
                        .server
                        .as_ref()
                        .map(|s| s.owner.to_string())
                        .unwrap_or_default();
                    if !self.state.op_log.is_trusted(&from, &owner) {
                        continue;
                    }
                    let mut sorted_ops = ops;
                    sorted_ops.sort_by(|a, b| a.hlc.cmp(&b.hlc));
                    let count = sorted_ops.len();
                    for stamped_op in &sorted_ops {
                        let applied = self.state.apply_op(
                            stamped_op,
                            &stamped_op.author,
                            &self.identity,
                            &self.cmd_tx,
                        );

                        if applied {
                            if let ops::Op::ChatMessage {
                                topic,
                                content_data,
                            } = &stamped_op.op
                            {
                                self.state.process_chat_message(
                                    topic,
                                    content_data,
                                    &stamped_op.author,
                                    &stamped_op.op_id,
                                    stamped_op.hlc.millis,
                                    stamped_op,
                                );
                            }
                        }
                    }
                    if count > 0 {
                        events.push(ClientEvent::SyncCompleted { ops_applied: count });
                    }
                }
                network::NetworkEvent::MessageReceived { .. } => {
                    // Legacy message path -- all messages now go through OpReceived.
                }
            }
        }

        events
    }

    // ───── Action methods ───────────────────────────────────────────────────

    /// Send a text message to the given channel.
    pub fn send_message(&mut self, channel: &str, body: &str) -> anyhow::Result<()> {
        let content = Content::Text {
            body: body.to_string(),
        };
        self.send_content(channel, content, body, None)
    }

    /// Send a reply to a specific message.
    pub fn send_reply(&mut self, channel: &str, parent_id: &str, body: &str) -> anyhow::Result<()> {
        let parent =
            willow_messaging::MessageId(uuid::Uuid::parse_str(parent_id).unwrap_or_default());
        let content = Content::Reply {
            parent,
            body: body.to_string(),
        };

        // Build reply preview from existing messages.
        let preview = self
            .state
            .chat
            .messages
            .iter()
            .find(|m| m.id == parent_id)
            .map(|m| {
                let text = if m.body.len() > 50 {
                    format!("{}...", &m.body[..50])
                } else {
                    m.body.clone()
                };
                format!("{}: {text}", m.author)
            });

        self.send_content(channel, content, body, preview)
    }

    /// Edit an existing message.
    pub fn edit_message(
        &mut self,
        channel: &str,
        message_id: &str,
        new_body: &str,
    ) -> anyhow::Result<()> {
        let target =
            willow_messaging::MessageId(uuid::Uuid::parse_str(message_id).unwrap_or_default());
        let content = Content::Edit {
            target: target.clone(),
            new_body: new_body.to_string(),
        };

        let topic = self
            .state
            .server
            .topic_for_name(channel)
            .unwrap_or_else(|| channel.to_string());

        let wire_content = self.encrypt_content(&content, &topic);
        let content_data = willow_transport::pack(&wire_content).unwrap_or_default();

        let peer_id_str = self.identity.peer_id().to_string();
        let stamped = StampedOp::new(
            Op::ChatMessage {
                topic,
                content_data,
            },
            &mut self.state.chat.hlc,
            &peer_id_str,
        );
        self.state.op_log.record(stamped.clone());
        let _ = self
            .cmd_tx
            .send(network::NetworkCommand::BroadcastOp(stamped));

        // Apply locally.
        let target_str = target.to_string();
        for m in &mut self.state.chat.messages {
            if m.id == target_str {
                m.body = new_body.to_string();
                m.edited = true;
                self.state.chat.messages_dirty = true;
                break;
            }
        }

        Ok(())
    }

    /// Delete a message.
    pub fn delete_message(&mut self, channel: &str, message_id: &str) -> anyhow::Result<()> {
        let target =
            willow_messaging::MessageId(uuid::Uuid::parse_str(message_id).unwrap_or_default());
        let content = Content::Delete {
            target: target.clone(),
        };

        let topic = self
            .state
            .server
            .topic_for_name(channel)
            .unwrap_or_else(|| channel.to_string());

        let wire_content = self.encrypt_content(&content, &topic);
        let content_data = willow_transport::pack(&wire_content).unwrap_or_default();

        let peer_id_str = self.identity.peer_id().to_string();
        let stamped = StampedOp::new(
            Op::ChatMessage {
                topic,
                content_data,
            },
            &mut self.state.chat.hlc,
            &peer_id_str,
        );
        self.state.op_log.record(stamped.clone());
        let _ = self
            .cmd_tx
            .send(network::NetworkCommand::BroadcastOp(stamped));

        // Apply locally.
        let target_str = target.to_string();
        for m in &mut self.state.chat.messages {
            if m.id == target_str {
                m.body = "[message deleted]".to_string();
                m.deleted = true;
                m.reactions.clear();
                self.state.chat.messages_dirty = true;
                break;
            }
        }

        Ok(())
    }

    /// Add a reaction to a message.
    pub fn react(&mut self, channel: &str, message_id: &str, emoji: &str) -> anyhow::Result<()> {
        let target =
            willow_messaging::MessageId(uuid::Uuid::parse_str(message_id).unwrap_or_default());
        let content = Content::Reaction {
            target: target.clone(),
            emoji: emoji.to_string(),
        };

        let topic = self
            .state
            .server
            .topic_for_name(channel)
            .unwrap_or_else(|| channel.to_string());

        let wire_content = self.encrypt_content(&content, &topic);
        let content_data = willow_transport::pack(&wire_content).unwrap_or_default();

        let peer_id_str = self.identity.peer_id().to_string();
        let stamped = StampedOp::new(
            Op::ChatMessage {
                topic,
                content_data,
            },
            &mut self.state.chat.hlc,
            &peer_id_str,
        );
        self.state.op_log.record(stamped.clone());
        let _ = self
            .cmd_tx
            .send(network::NetworkCommand::BroadcastOp(stamped));

        // Apply locally.
        let target_str = target.to_string();
        let author = self.display_name();
        for m in &mut self.state.chat.messages {
            if m.id == target_str {
                m.reactions
                    .entry(emoji.to_string())
                    .or_default()
                    .push(author);
                self.state.chat.messages_dirty = true;
                break;
            }
        }

        Ok(())
    }

    /// Create a new channel.
    pub fn create_channel(&mut self, name: &str) -> anyhow::Result<()> {
        let (topic, ch_id) = {
            let Some(server) = &mut self.state.server.server else {
                anyhow::bail!("no server");
            };
            let ch_id = server.create_channel(name, willow_channel::ChannelKind::Text)?;
            let topic = util::make_topic(server, name);

            if let Some(key) = server.channel_key(&ch_id) {
                self.state.key_store.keys.insert(topic.clone(), key.clone());
            }
            storage::save_server(server, &self.state.key_store.keys);
            (topic, ch_id)
        };

        let ch_id_str = ch_id.to_string();
        self.state
            .server
            .topic_map
            .insert(topic.clone(), (name.to_string(), ch_id));

        let _ = self.cmd_tx.send(network::NetworkCommand::Subscribe(topic));

        self.broadcast_op(Op::CreateChannel {
            name: name.to_string(),
            channel_id: ch_id_str,
        });

        self.state.chat.current_channel = name.to_string();
        self.state.chat.messages_dirty = true;

        Ok(())
    }

    /// Delete a channel.
    pub fn delete_channel(&mut self, name: &str) -> anyhow::Result<()> {
        let Some((topic, (_ch_name, ch_id))) = self
            .state
            .server
            .topic_map
            .iter()
            .find(|(_, (n, _))| n == name)
            .map(|(t, v)| (t.clone(), v.clone()))
        else {
            anyhow::bail!("channel not found");
        };

        {
            let Some(server) = &mut self.state.server.server else {
                anyhow::bail!("no server");
            };
            server.delete_channel(&ch_id)?;
            storage::save_server(server, &self.state.key_store.keys);
        }

        self.state.server.topic_map.remove(&topic);
        self.state.key_store.keys.remove(&topic);

        if self.state.chat.current_channel == name {
            let names = self.state.server.channel_names();
            self.state.chat.current_channel = names.first().cloned().unwrap_or_default();
            self.state.chat.messages_dirty = true;
        }

        self.broadcast_op(Op::DeleteChannel {
            name: name.to_string(),
        });

        Ok(())
    }

    /// Trust a peer for server state operations.
    pub fn trust_peer(&mut self, peer_id: &str) {
        self.broadcast_op(Op::TrustPeer {
            peer_id: peer_id.to_string(),
        });
    }

    /// Revoke trust from a peer.
    pub fn untrust_peer(&mut self, peer_id: &str) {
        self.broadcast_op(Op::UntrustPeer {
            peer_id: peer_id.to_string(),
        });
    }

    /// Kick a member, rotating channel keys.
    pub fn kick_member(&mut self, peer_id: &str) -> anyhow::Result<()> {
        let rotated = {
            let Some(server) = &mut self.state.server.server else {
                anyhow::bail!("no server");
            };

            let member_peer = server
                .members()
                .iter()
                .find(|m| m.peer_id.to_string() == peer_id)
                .map(|m| m.peer_id.clone());

            let Some(peer) = member_peer else {
                anyhow::bail!("peer not found in server members");
            };

            let new_keys = server.remove_member(&peer)?;
            storage::save_server(server, &self.state.key_store.keys);
            new_keys
        };

        // Update key store with rotated keys.
        for (ch_id, key) in &rotated {
            for (topic, (_, tid)) in &self.state.server.topic_map {
                if tid == ch_id {
                    self.state.key_store.keys.insert(topic.clone(), key.clone());
                    break;
                }
            }
        }

        self.state.chat.peers.retain(|p| p != peer_id);

        // Encrypt rotated keys for remaining members.
        let mut rotated_key_entries = Vec::new();
        if let Some(server) = &self.state.server.server {
            for member in server.members() {
                let peer_str = member.peer_id.to_string();
                if let Some(pub_key) = invite::peer_id_to_ed25519_public(&peer_str) {
                    for (ch_id, key) in &rotated {
                        for (topic, (_, tid)) in &self.state.server.topic_map {
                            if tid == ch_id {
                                if let Ok(enc) =
                                    willow_crypto::encrypt_channel_key_for(key, &pub_key)
                                {
                                    rotated_key_entries.push((
                                        peer_str.clone(),
                                        topic.clone(),
                                        enc,
                                    ));
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }

        self.broadcast_op(Op::KickMember {
            peer_id: peer_id.to_string(),
            rotated_keys: rotated_key_entries,
        });

        Ok(())
    }

    /// Create a new role.
    pub fn create_role(&mut self, name: &str) -> anyhow::Result<()> {
        let role_id = willow_channel::RoleId::new();
        let role = willow_channel::Role::with_id(role_id.clone(), name);

        let Some(server) = &mut self.state.server.server else {
            anyhow::bail!("no server");
        };
        server.create_role(role);
        storage::save_server(server, &self.state.key_store.keys);

        self.broadcast_op(Op::CreateRole {
            name: name.to_string(),
            role_id: role_id.to_string(),
        });

        Ok(())
    }

    /// Delete a role by ID.
    pub fn delete_role(&mut self, role_id: &str) -> anyhow::Result<()> {
        let rid = willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());

        let Some(server) = &mut self.state.server.server else {
            anyhow::bail!("no server");
        };
        server.delete_role(&rid)?;
        storage::save_server(server, &self.state.key_store.keys);

        self.broadcast_op(Op::DeleteRole {
            role_id: role_id.to_string(),
        });

        Ok(())
    }

    /// Set a permission on a role.
    pub fn set_permission(
        &mut self,
        role_id: &str,
        permission: &str,
        granted: bool,
    ) -> anyhow::Result<()> {
        let rid = willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
        let perm = parse_permission(permission)?;

        let Some(server) = &mut self.state.server.server else {
            anyhow::bail!("no server");
        };
        server.set_permission(&rid, perm, granted)?;
        storage::save_server(server, &self.state.key_store.keys);

        self.broadcast_op(Op::SetPermission {
            role_id: role_id.to_string(),
            permission: permission.to_string(),
            granted,
        });

        Ok(())
    }

    /// Assign a role to a peer.
    pub fn assign_role(&mut self, peer_id: &str, role_id: &str) -> anyhow::Result<()> {
        let rid = willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());

        let Some(server) = &mut self.state.server.server else {
            anyhow::bail!("no server");
        };

        let member_peer = server
            .members()
            .iter()
            .find(|m| m.peer_id.to_string() == peer_id)
            .map(|m| m.peer_id.clone());

        let Some(peer) = member_peer else {
            anyhow::bail!("peer not found");
        };

        server.assign_role(&peer, &rid)?;
        storage::save_server(server, &self.state.key_store.keys);

        self.broadcast_op(Op::AssignRole {
            peer_id: peer_id.to_string(),
            role_id: role_id.to_string(),
        });

        Ok(())
    }

    /// Generate a secure invite code encrypted for the given recipient.
    pub fn generate_invite(&self, recipient_peer_id: &str) -> anyhow::Result<String> {
        let Some(pub_key) = invite::peer_id_to_ed25519_public(recipient_peer_id) else {
            anyhow::bail!("invalid recipient PeerId");
        };

        let Some(server) = &self.state.server.server else {
            anyhow::bail!("no server");
        };

        invite::generate_invite(
            server,
            &self.state.key_store.keys,
            &self.state.server.topic_map,
            &pub_key,
        )
        .ok_or_else(|| anyhow::anyhow!("invite generation failed"))
    }

    /// Accept an invite code and join the server.
    pub fn accept_invite(&mut self, code: &str) -> anyhow::Result<()> {
        let accepted = invite::accept_invite(code, &self.identity)
            .ok_or_else(|| anyhow::anyhow!("invalid invite code or not for us"))?;

        for (topic, (name, key)) in &accepted.channel_keys {
            self.state.key_store.keys.insert(topic.clone(), key.clone());

            if !self.state.server.topic_map.contains_key(topic) {
                self.state.server.topic_map.insert(
                    topic.clone(),
                    (name.clone(), willow_channel::ChannelId::new()),
                );
            }

            let _ = self
                .cmd_tx
                .send(network::NetworkCommand::Subscribe(topic.clone()));
        }

        if let Some((_, (name, _))) = accepted.channel_keys.iter().next() {
            self.state.chat.current_channel = name.clone();
            self.state.chat.messages_dirty = true;
        }

        if let Some(server) = &self.state.server.server {
            storage::save_server(server, &self.state.key_store.keys);
        }

        Ok(())
    }

    /// Set the local display name and broadcast to peers.
    pub fn set_display_name(&mut self, name: &str) {
        let peer_id = self.identity.peer_id().to_string();
        self.state.profiles.names.insert(peer_id, name.to_string());

        storage::save_profile(&storage::LocalProfile {
            display_name: name.to_string(),
        });

        let _ = self.cmd_tx.send(network::NetworkCommand::BroadcastProfile {
            display_name: name.to_string(),
        });
    }

    /// Switch the current channel.
    pub fn switch_channel(&mut self, name: &str) {
        if self.state.chat.current_channel != name {
            self.state.chat.current_channel = name.to_string();
            self.state.chat.messages_dirty = true;

            if let Some(topic) = self.state.server.topic_for_name(name) {
                self.state.unread.counts.remove(&topic);
            }
        }
    }

    // ───── Accessor methods ─────────────────────────────────────────────────

    /// Get a reference to the full client state.
    pub fn state(&self) -> &ClientState {
        &self.state
    }

    /// Get the local PeerId as a string.
    pub fn peer_id(&self) -> String {
        self.identity.peer_id().to_string()
    }

    /// Get the local display name.
    pub fn display_name(&self) -> String {
        self.state.profiles.display_name(&self.peer_id())
    }

    /// Get a peer's display name.
    pub fn peer_display_name(&self, peer_id: &str) -> String {
        self.state.profiles.display_name(peer_id)
    }

    /// Get messages for a channel, filtered by topic.
    pub fn messages(&self, channel: &str) -> Vec<&ChatMessage> {
        let topic = self
            .state
            .server
            .topic_for_name(channel)
            .unwrap_or_default();
        self.state
            .chat
            .messages
            .iter()
            .filter(|m| m.topic == topic)
            .collect()
    }

    /// List all channel names.
    pub fn channels(&self) -> Vec<String> {
        self.state.server.channel_names()
    }

    /// Get the list of connected peers.
    pub fn peers(&self) -> &[String] {
        &self.state.chat.peers
    }

    /// Whether the network is connected.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    // ───── Internal helpers ─────────────────────────────────────────────────

    /// Stamp, record, persist, and broadcast a server op.
    fn broadcast_op(&mut self, op: Op) {
        let peer_id_str = self.identity.peer_id().to_string();
        let stamped = StampedOp::new(op, &mut self.state.chat.hlc, &peer_id_str);
        self.state.op_log.record(stamped.clone());
        storage::save_op_log(&self.state.op_log.ops);
        let _ = self
            .cmd_tx
            .send(network::NetworkCommand::BroadcastOp(stamped));
    }

    /// Send chat content (text, reply, edit, delete, reaction) on a channel.
    fn send_content(
        &mut self,
        channel: &str,
        content: Content,
        body: &str,
        reply_preview: Option<String>,
    ) -> anyhow::Result<()> {
        let topic = self
            .state
            .server
            .topic_for_name(channel)
            .unwrap_or_else(|| channel.to_string());

        let wire_content = self.encrypt_content(&content, &topic);
        let content_data = willow_transport::pack(&wire_content).unwrap_or_default();

        let peer_id_str = self.identity.peer_id().to_string();
        let stamped = StampedOp::new(
            Op::ChatMessage {
                topic: topic.clone(),
                content_data,
            },
            &mut self.state.chat.hlc,
            &peer_id_str,
        );

        self.state.op_log.record(stamped.clone());

        // Persist the stamped op for catch-up sync.
        if let Some(ref db_arc) = self.state.message_db {
            if let Ok(db_lock) = db_arc.lock() {
                db_lock.insert_chat_op(&stamped, &topic);
            }
        }

        let _ = self
            .cmd_tx
            .send(network::NetworkCommand::BroadcastOp(stamped.clone()));

        // Add to local display.
        let author = self.state.profiles.display_name(&peer_id_str);
        let ts = stamped.hlc.millis;
        let mut chat_msg =
            ChatMessage::new(topic.clone(), author.clone(), body.to_string(), true, ts);
        chat_msg.id = stamped.op_id.clone();
        chat_msg.reply_preview = reply_preview;

        // Persist to MessageDb.
        if let Some(ref db_arc) = self.state.message_db {
            if let Ok(db_lock) = db_arc.lock() {
                db_lock.insert(&storage::StoredMessage {
                    topic: topic.clone(),
                    author,
                    body: body.to_string(),
                    is_local: true,
                    timestamp_ms: ts,
                });
            }
        }

        self.state.chat.messages.push(chat_msg);
        self.state.chat.messages_dirty = true;

        Ok(())
    }

    /// Encrypt content if a channel key exists for the topic.
    fn encrypt_content(&self, content: &Content, topic: &str) -> Content {
        if let Some(key) = self.state.key_store.keys.get(topic) {
            if let Ok(sealed) = willow_crypto::seal_content(content, key, 0) {
                return Content::Encrypted(sealed);
            }
        }
        content.clone()
    }

    /// Called when we first hear from the network (Listening or PeerConnected).
    /// Subscribes to all channel topics, profile topic, server ops topic,
    /// broadcasts profile, and requests sync.
    fn on_connected(&self) {
        // Subscribe to all channel topics.
        for topic in self.state.server.topic_map.keys() {
            let _ = self
                .cmd_tx
                .send(network::NetworkCommand::Subscribe(topic.clone()));
        }

        // Subscribe to the global profile broadcast topic.
        let _ = self.cmd_tx.send(network::NetworkCommand::Subscribe(
            network::PROFILE_TOPIC.to_string(),
        ));

        // Subscribe to server state operations topic.
        let _ = self.cmd_tx.send(network::NetworkCommand::Subscribe(
            ops::SERVER_OPS_TOPIC.to_string(),
        ));

        // Broadcast our profile.
        let saved_profile = storage::load_profile().unwrap_or_default();
        if !saved_profile.display_name.is_empty() {
            let _ = self.cmd_tx.send(network::NetworkCommand::BroadcastProfile {
                display_name: saved_profile.display_name,
            });
        }

        // Request missing server ops.
        let _ = self.cmd_tx.send(network::NetworkCommand::RequestSync {
            latest_hlc: self.state.op_log.latest_hlc(),
            topic: None,
        });

        // Request chat history for each channel.
        for topic in self.state.server.topic_map.keys() {
            let _ = self.cmd_tx.send(network::NetworkCommand::RequestSync {
                latest_hlc: willow_messaging::hlc::HlcTimestamp::ZERO,
                topic: Some(topic.clone()),
            });
        }
    }
}

// ───── Identity persistence ──────────────────────────────────────────────────

fn load_identity() -> Identity {
    if let Some(bytes) = storage::load_identity_bytes() {
        if let Some(id) = Identity::from_ed25519_bytes(&bytes) {
            return id;
        }
    }

    let identity = Identity::generate();
    if let Some(bytes) = identity.to_ed25519_bytes() {
        storage::save_identity_bytes(&bytes);
    }
    identity
}

/// Parse a permission string into a [`willow_channel::Permission`].
fn parse_permission(s: &str) -> anyhow::Result<willow_channel::Permission> {
    match s {
        "Administrator" => Ok(willow_channel::Permission::Administrator),
        "SendMessages" => Ok(willow_channel::Permission::SendMessages),
        "ReadMessages" => Ok(willow_channel::Permission::ReadMessages),
        "KickMembers" => Ok(willow_channel::Permission::KickMembers),
        "CreateInvite" => Ok(willow_channel::Permission::CreateInvite),
        "AttachFiles" => Ok(willow_channel::Permission::AttachFiles),
        "ManageChannels" => Ok(willow_channel::Permission::ManageChannels),
        _ => anyhow::bail!("unknown permission: {s}"),
    }
}

/// Create a test-only Client without connecting to the network.
/// The returned client has mpsc channels wired up but no background task.
#[cfg(test)]
pub(crate) fn test_client() -> (Client, std::sync::mpsc::Receiver<network::NetworkCommand>) {
    let identity = Identity::generate();
    let (cmd_tx, cmd_rx) = std_mpsc::channel();
    let (_event_tx, event_rx) = std_mpsc::channel();

    let mut state = ClientState::default();

    // Create a minimal server.
    let mut server = willow_channel::Server::new("Test Server", identity.peer_id());
    let ch_id = server
        .create_channel("general", willow_channel::ChannelKind::Text)
        .unwrap();
    let topic = util::make_topic(&server, "general");
    if let Some(key) = server.channel_key(&ch_id) {
        state.key_store.keys.insert(topic.clone(), key.clone());
    }
    state
        .server
        .topic_map
        .insert(topic, ("general".to_string(), ch_id));
    state.server.server = Some(server);

    let client = Client {
        state,
        identity,
        cmd_tx,
        event_rx,
        connected: false,
        config: ClientConfig {
            persistence: false,
            ..ClientConfig::default()
        },
        deferred_channels: None,
        connected_subscribed: false,
    };

    (client, cmd_rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_new_has_default_channels() {
        let (client, _rx) = test_client();
        let names = client.channels();
        assert!(names.contains(&"general".to_string()));
    }

    #[test]
    fn client_peer_id_is_stable() {
        let (client, _rx) = test_client();
        let id1 = client.peer_id();
        let id2 = client.peer_id();
        assert_eq!(id1, id2);
        assert!(!id1.is_empty());
    }

    #[test]
    fn send_message_adds_to_state() {
        let (mut client, _rx) = test_client();
        client.send_message("general", "hello").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "hello");
        assert!(msgs[0].is_local);
    }

    #[test]
    fn send_message_broadcasts_op() {
        let (mut client, rx) = test_client();
        client.send_message("general", "test").unwrap();

        let cmd = rx.try_recv().unwrap();
        assert!(matches!(cmd, network::NetworkCommand::BroadcastOp(_)));
    }

    #[test]
    fn send_reply_has_preview() {
        let (mut client, _rx) = test_client();
        client.send_message("general", "original").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.send_reply("general", &msg_id, "reply").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].body, "reply");
        assert!(msgs[1].reply_preview.is_some());
    }

    #[test]
    fn edit_message_updates_state() {
        let (mut client, _rx) = test_client();
        client.send_message("general", "original").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.edit_message("general", &msg_id, "edited").unwrap();

        let msgs = client.messages("general");
        assert_eq!(msgs[0].body, "edited");
        assert!(msgs[0].edited);
    }

    #[test]
    fn delete_message_marks_deleted() {
        let (mut client, _rx) = test_client();
        client.send_message("general", "to delete").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.delete_message("general", &msg_id).unwrap();

        let msgs = client.messages("general");
        assert!(msgs[0].deleted);
        assert_eq!(msgs[0].body, "[message deleted]");
    }

    #[test]
    fn react_adds_reaction() {
        let (mut client, _rx) = test_client();
        client.send_message("general", "react to me").unwrap();
        let msg_id = client.messages("general")[0].id.clone();

        client.react("general", &msg_id, "thumbsup").unwrap();

        let msgs = client.messages("general");
        assert!(msgs[0].reactions.contains_key("thumbsup"));
    }

    #[test]
    fn create_channel_adds_to_server() {
        let (mut client, _rx) = test_client();
        client.create_channel("new-channel").unwrap();

        let names = client.channels();
        assert!(names.contains(&"new-channel".to_string()));
        assert_eq!(client.state.chat.current_channel, "new-channel");
    }

    #[test]
    fn delete_channel_removes_from_server() {
        let (mut client, _rx) = test_client();
        client.create_channel("temp").unwrap();
        assert!(client.channels().contains(&"temp".to_string()));

        client.delete_channel("temp").unwrap();
        assert!(!client.channels().contains(&"temp".to_string()));
    }

    #[test]
    fn switch_channel_updates_current() {
        let (mut client, _rx) = test_client();
        client.create_channel("other").unwrap();
        client.switch_channel("general");

        assert_eq!(client.state.chat.current_channel, "general");
    }

    #[test]
    fn trust_untrust_broadcasts_ops() {
        let (mut client, rx) = test_client();
        client.trust_peer("some-peer");

        let cmd = rx.try_recv().unwrap();
        assert!(
            matches!(cmd, network::NetworkCommand::BroadcastOp(ref s) if matches!(s.op, Op::TrustPeer { .. }))
        );

        client.untrust_peer("some-peer");
        let cmd = rx.try_recv().unwrap();
        assert!(
            matches!(cmd, network::NetworkCommand::BroadcastOp(ref s) if matches!(s.op, Op::UntrustPeer { .. }))
        );
    }

    #[test]
    fn display_name_default_truncated() {
        let (client, _rx) = test_client();
        let name = client.display_name();
        // Default name should be truncated peer ID.
        assert!(!name.is_empty());
    }

    #[test]
    fn set_display_name_updates_profile() {
        let (mut client, rx) = test_client();
        client.set_display_name("Alice");

        assert_eq!(client.display_name(), "Alice");

        // Should broadcast profile.
        let cmd = rx.try_recv().unwrap();
        assert!(matches!(
            cmd,
            network::NetworkCommand::BroadcastProfile { .. }
        ));
    }

    #[test]
    fn messages_filters_by_channel() {
        let (mut client, _rx) = test_client();
        client.send_message("general", "msg1").unwrap();
        client.create_channel("other").unwrap();
        client.send_message("other", "msg2").unwrap();

        assert_eq!(client.messages("general").len(), 1);
        assert_eq!(client.messages("other").len(), 1);
        assert_eq!(client.messages("general")[0].body, "msg1");
        assert_eq!(client.messages("other")[0].body, "msg2");
    }

    #[test]
    fn apply_op_deduplicates() {
        let (mut client, _rx) = test_client();

        let mut hlc = willow_messaging::hlc::HLC::new();
        let stamped = StampedOp::new(
            Op::CreateChannel {
                name: "dedup-test".into(),
                channel_id: uuid::Uuid::new_v4().to_string(),
            },
            &mut hlc,
            &client.peer_id(),
        );

        let applied1 = client.state.apply_op(
            &stamped,
            &client.peer_id(),
            &client.identity.clone(),
            &client.cmd_tx,
        );
        let applied2 = client.state.apply_op(
            &stamped,
            &client.peer_id(),
            &client.identity.clone(),
            &client.cmd_tx,
        );

        assert!(applied1);
        assert!(!applied2);
    }

    #[test]
    fn generate_accept_invite_round_trip() {
        let (client, _rx) = test_client();
        let recipient = Identity::generate();
        let recipient_peer_id = recipient.peer_id().to_string();

        let code = client.generate_invite(&recipient_peer_id).unwrap();
        assert!(!code.is_empty());

        // Verify recipient can accept.
        let accepted = invite::accept_invite(&code, &recipient).unwrap();
        assert!(!accepted.channel_keys.is_empty());
    }
}
