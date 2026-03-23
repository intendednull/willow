//! # Client State
//!
//! Pure state types for the Willow client, extracted from the Bevy UI resources.
//! These types hold the client's runtime state without any UI framework dependency.

use std::collections::{HashMap, HashSet};

use willow_channel::Server;
use willow_crypto::ChannelKey;
use willow_messaging::hlc::HLC;

/// Maximum messages kept in memory per topic to avoid unbounded growth.
pub const MAX_MESSAGES_IN_MEMORY: usize = 1000;

/// The default channel name used when no channels exist.
pub const DEFAULT_CHANNEL: &str = "general";

/// The local server instance. Each peer auto-creates a server on first launch.
#[derive(Default)]
pub struct ServerState {
    pub server: Option<Server>,
    /// Maps gossipsub topic -> (channel_name, channel_id) for display + key lookup.
    pub topic_map: HashMap<String, (String, willow_channel::ChannelId)>,
}

impl ServerState {
    /// Get the gossipsub topic for a channel by name.
    pub fn topic_for_name(&self, name: &str) -> Option<String> {
        self.topic_map
            .iter()
            .find(|(_, (n, _))| n == name)
            .map(|(topic, _)| topic.clone())
    }

    /// Get the channel name for a gossipsub topic.
    #[allow(dead_code)]
    pub fn name_for_topic(&self, topic: &str) -> Option<&str> {
        self.topic_map.get(topic).map(|(name, _)| name.as_str())
    }

    /// List all channel names in sidebar order.
    pub fn channel_names(&self) -> Vec<String> {
        let Some(server) = &self.server else {
            return Vec::new();
        };
        let mut names: Vec<_> = server.channels().iter().map(|ch| ch.name.clone()).collect();
        names.sort();
        names
    }
}

/// Chat state holding messages, current channel, peers, and the HLC clock.
pub struct ChatState {
    pub messages: Vec<ChatMessage>,
    /// The current channel *name* (human-readable, e.g. "general").
    pub current_channel: String,
    pub peers: Vec<String>,
    pub hlc: HLC,
    pub messages_dirty: bool,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            current_channel: DEFAULT_CHANNEL.to_string(),
            peers: Vec::new(),
            hlc: HLC::new(),
            messages_dirty: true,
        }
    }
}

impl ChatState {
    /// Prune old messages if total count exceeds the limit.
    pub fn prune_if_needed(&mut self) {
        if self.messages.len() > MAX_MESSAGES_IN_MEMORY {
            let excess = self.messages.len() - MAX_MESSAGES_IN_MEMORY;
            self.messages.drain(..excess);
        }
    }
}

/// A single chat message with metadata.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// The gossipsub topic this message belongs to.
    pub topic: String,
    /// Unique ID for this message (for reactions/edit/delete to target).
    pub id: String,
    pub author: String,
    pub body: String,
    pub is_local: bool,
    /// HLC timestamp in milliseconds (for display).
    pub timestamp_ms: u64,
    /// Reactions: emoji -> list of author names.
    pub reactions: HashMap<String, Vec<String>>,
    /// Whether this message has been edited.
    pub edited: bool,
    /// Whether this message has been deleted (shows "[deleted]").
    pub deleted: bool,
    /// If this is a reply, the parent message preview ("Author: text...").
    pub reply_preview: Option<String>,
}

impl ChatMessage {
    pub fn new(
        topic: String,
        author: String,
        body: String,
        is_local: bool,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            topic,
            id: uuid::Uuid::new_v4().to_string(),
            author,
            body,
            is_local,
            timestamp_ms,
            reactions: HashMap::new(),
            edited: false,
            deleted: false,
            reply_preview: None,
        }
    }
}

/// Tracks unread message counts per channel topic.
#[derive(Default)]
pub struct UnreadCounts {
    pub counts: HashMap<String, usize>,
}

/// Ordered log of server operations for deduplication, replay, and trust.
#[derive(Default)]
pub struct OpLog {
    /// All recorded operations in HLC order.
    pub ops: Vec<crate::ops::StampedOp>,
    /// Set of seen op IDs for deduplication.
    pub seen_ids: HashSet<String>,
    /// Set of trusted PeerIds (derived from TrustPeer/UntrustPeer ops).
    pub trusted_peers: HashSet<String>,
}

impl OpLog {
    /// Record a stamped op. Returns true if it was new (not a duplicate).
    ///
    /// Chat messages are tracked in `seen_ids` for dedup but are **not**
    /// stored in `ops` -- they are persisted via `MessageDb` instead.
    pub fn record(&mut self, stamped: crate::ops::StampedOp) -> bool {
        if !self.seen_ids.insert(stamped.op_id.clone()) {
            return false;
        }
        match &stamped.op {
            crate::ops::Op::TrustPeer { peer_id } => {
                self.trusted_peers.insert(peer_id.clone());
            }
            crate::ops::Op::UntrustPeer { peer_id } => {
                self.trusted_peers.remove(peer_id);
            }
            // Chat messages go to MessageDb, not the op log.
            crate::ops::Op::ChatMessage { .. } => return true,
            _ => {}
        }
        self.ops.push(stamped);
        true
    }

    /// Check whether a peer is trusted (owner is always trusted).
    pub fn is_trusted(&self, peer_id: &str, owner: &str) -> bool {
        peer_id == owner || self.trusted_peers.contains(peer_id)
    }

    /// Rebuild seen_ids and trusted_peers from the ops list (after loading).
    pub fn rebuild(&mut self) {
        self.seen_ids.clear();
        self.trusted_peers.clear();
        let ops = std::mem::take(&mut self.ops);
        for op in ops {
            self.record(op);
        }
    }

    /// The HLC timestamp of the most recent op.
    pub fn latest_hlc(&self) -> willow_messaging::hlc::HlcTimestamp {
        self.ops
            .last()
            .map(|op| op.hlc)
            .unwrap_or(willow_messaging::hlc::HlcTimestamp::ZERO)
    }
}

/// Per-channel symmetric encryption keys, keyed by gossipsub topic.
#[derive(Default)]
pub struct ChannelKeyStore {
    pub keys: HashMap<String, ChannelKey>,
}

/// Maps PeerId strings -> display names. Updated from profile broadcasts.
#[derive(Default, Clone)]
pub struct ProfileStore {
    pub names: HashMap<String, String>,
}

impl ProfileStore {
    /// Look up a display name for a peer, falling back to truncated ID.
    pub fn display_name(&self, peer_id: &str) -> String {
        self.names
            .get(peer_id)
            .cloned()
            .unwrap_or_else(|| crate::util::truncate_peer_id(peer_id))
    }
}

/// Aggregate client state bundle. Holds all runtime state for the client
/// without any UI framework dependency.
pub struct ClientState {
    /// Chat messages, current channel, peers, and HLC clock.
    pub chat: ChatState,
    /// The local server instance and topic map.
    pub server: ServerState,
    /// Ordered log of server operations.
    pub op_log: OpLog,
    /// Per-channel symmetric encryption keys.
    pub key_store: ChannelKeyStore,
    /// Peer display names.
    pub profiles: ProfileStore,
    /// Unread message counts per channel.
    pub unread: UnreadCounts,
    /// Emoji shortcode expansion registry.
    pub emoji: crate::emoji::EmojiRegistry,
    /// Persistent message database (native-only SQLite, WASM localStorage).
    pub message_db: Option<std::sync::Arc<std::sync::Mutex<crate::storage::MessageDb>>>,
}

impl Default for ClientState {
    fn default() -> Self {
        Self {
            chat: ChatState::default(),
            server: ServerState::default(),
            op_log: OpLog::default(),
            key_store: ChannelKeyStore::default(),
            profiles: ProfileStore::default(),
            unread: UnreadCounts::default(),
            emoji: crate::emoji::EmojiRegistry::new(),
            message_db: None,
        }
    }
}

impl ClientState {
    /// Apply a remote server operation to local state.
    ///
    /// Returns `true` if the op was new and accepted (not deduplicated or
    /// rejected). Chat messages bypass the trust check -- anyone who can
    /// subscribe to a channel topic can chat. Trust is enforced only for
    /// server state mutations.
    pub fn apply_op(
        &mut self,
        stamped: &crate::ops::StampedOp,
        from: &str,
        identity: &willow_identity::Identity,
        cmd_tx: &std::sync::mpsc::Sender<crate::network::NetworkCommand>,
    ) -> bool {
        use crate::ops::Op;

        // Dedup: skip if we've already seen this op.
        if self.op_log.seen_ids.contains(&stamped.op_id) {
            return false;
        }

        // Trust check: only for non-chat ops (server state mutations).
        let needs_trust = !matches!(stamped.op, Op::ChatMessage { .. });
        if needs_trust {
            let owner = self
                .server
                .server
                .as_ref()
                .map(|s| s.owner.to_string())
                .unwrap_or_default();
            if !self.op_log.is_trusted(from, &owner) {
                self.op_log.seen_ids.insert(stamped.op_id.clone());
                return false;
            }
        }

        // Advance local HLC.
        self.chat.hlc.receive(stamped.hlc);

        // Record (chat messages go to seen_ids only, not ops).
        self.op_log.record(stamped.clone());

        // Persist op log only for server ops (not chat messages).
        if needs_trust {
            crate::storage::save_op_log(&self.op_log.ops);
        }

        match &stamped.op {
            Op::CreateChannel { name, channel_id } => {
                let info = {
                    let Some(server) = &mut self.server.server else {
                        return true;
                    };
                    if server.channels().iter().any(|ch| ch.name == *name) {
                        return true;
                    }
                    let ch_uuid =
                        uuid::Uuid::parse_str(channel_id).unwrap_or_else(|_| uuid::Uuid::new_v4());
                    let ch_id = willow_channel::ChannelId(ch_uuid);
                    let Ok(ch_id) = server.create_channel_with_id(
                        ch_id,
                        name,
                        willow_channel::ChannelKind::Text,
                    ) else {
                        return true;
                    };
                    let topic = crate::util::make_topic(server, name);
                    let key = server.channel_key(&ch_id).cloned();
                    (topic, ch_id, key)
                };
                let (topic, ch_id, key) = info;
                if let Some(k) = key {
                    self.key_store.keys.insert(topic.clone(), k);
                }
                self.server
                    .topic_map
                    .insert(topic.clone(), (name.clone(), ch_id));
                if let Some(server) = &self.server.server {
                    crate::storage::save_server(server, &self.key_store.keys);
                }
                let _ = cmd_tx.send(crate::network::NetworkCommand::Subscribe(topic));
            }
            Op::DeleteChannel { name } => {
                let to_remove = self
                    .server
                    .topic_map
                    .iter()
                    .find(|(_, (n, _))| n == name)
                    .map(|(t, (_, id))| (t.clone(), id.clone()));

                if let Some((topic, ch_id)) = to_remove {
                    if let Some(server) = &mut self.server.server {
                        let _ = server.delete_channel(&ch_id);
                        crate::storage::save_server(server, &self.key_store.keys);
                    }
                    self.server.topic_map.remove(&topic);
                    self.key_store.keys.remove(&topic);

                    if self.chat.current_channel == *name {
                        let names = self.server.channel_names();
                        self.chat.current_channel = names.first().cloned().unwrap_or_default();
                        self.chat.messages_dirty = true;
                    }
                }
            }
            Op::CreateRole { name, role_id } => {
                if let Some(server) = &mut self.server.server {
                    if !server.roles().iter().any(|r| r.name == *name) {
                        let rid = willow_channel::RoleId(
                            uuid::Uuid::parse_str(role_id).unwrap_or_default(),
                        );
                        let role = willow_channel::Role::with_id(rid, name);
                        server.create_role(role);
                        crate::storage::save_server(server, &self.key_store.keys);
                    }
                }
            }
            Op::DeleteRole { role_id } => {
                if let Some(server) = &mut self.server.server {
                    let rid =
                        willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
                    let _ = server.delete_role(&rid);
                    crate::storage::save_server(server, &self.key_store.keys);
                }
            }
            Op::SetPermission {
                role_id,
                permission,
                granted,
            } => {
                if let Some(server) = &mut self.server.server {
                    let rid =
                        willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
                    let perm = match permission.as_str() {
                        "Administrator" => willow_channel::Permission::Administrator,
                        "SendMessages" => willow_channel::Permission::SendMessages,
                        "ReadMessages" => willow_channel::Permission::ReadMessages,
                        "KickMembers" => willow_channel::Permission::KickMembers,
                        "CreateInvite" => willow_channel::Permission::CreateInvite,
                        "AttachFiles" => willow_channel::Permission::AttachFiles,
                        "ManageChannels" => willow_channel::Permission::ManageChannels,
                        _ => return true,
                    };
                    let _ = server.set_permission(&rid, perm, *granted);
                    crate::storage::save_server(server, &self.key_store.keys);
                }
            }
            Op::AssignRole { peer_id, role_id } => {
                if let Some(server) = &mut self.server.server {
                    let rid =
                        willow_channel::RoleId(uuid::Uuid::parse_str(role_id).unwrap_or_default());
                    let member_peer = server
                        .members()
                        .iter()
                        .find(|m| m.peer_id.to_string() == *peer_id)
                        .map(|m| m.peer_id.clone());
                    if let Some(peer) = member_peer {
                        let _ = server.assign_role(&peer, &rid);
                        crate::storage::save_server(server, &self.key_store.keys);
                    }
                }
            }
            Op::KickMember {
                peer_id,
                rotated_keys,
            } => {
                self.chat.peers.retain(|p| p != peer_id);

                if let Some(server) = &mut self.server.server {
                    let member_peer = server
                        .members()
                        .iter()
                        .find(|m| m.peer_id.to_string() == *peer_id)
                        .map(|m| m.peer_id.clone());
                    if let Some(peer) = member_peer {
                        let _ = server.remove_member(&peer);
                    }
                }

                let our_peer_id = identity.peer_id().to_string();
                for (recipient, topic, encrypted) in rotated_keys {
                    if *recipient == our_peer_id {
                        if let Ok(key) = willow_crypto::decrypt_channel_key(encrypted, identity) {
                            self.key_store.keys.insert(topic.clone(), key.clone());
                            let ch_id = self.server.topic_map.get(topic).map(|(_, id)| id.clone());
                            if let (Some(ch_id), Some(server)) =
                                (ch_id, self.server.server.as_mut())
                            {
                                server.set_channel_key(ch_id, key);
                            }
                        }
                    }
                }

                if let Some(server) = &self.server.server {
                    crate::storage::save_server(server, &self.key_store.keys);
                }
            }
            Op::TrustPeer { .. } | Op::UntrustPeer { .. } => {
                // Trust changes are handled by OpLog::record above.
            }
            Op::ChatMessage { .. } => {
                // Chat message display is handled by the caller after apply_op.
            }
        }

        true
    }

    /// Process a ChatMessage op for display: deserialize content, decrypt if
    /// needed, and add to ChatState / persist to MessageDb.
    pub fn process_chat_message(
        &mut self,
        topic: &str,
        content_data: &[u8],
        author_peer_id: &str,
        op_id: &str,
        hlc_millis: u64,
        stamped: &crate::ops::StampedOp,
    ) {
        // Store the stamped op for catch-up sync.
        if let Some(ref db_arc) = self.message_db {
            if let Ok(db_lock) = db_arc.lock() {
                db_lock.insert_chat_op(stamped, topic);
            }
        }

        let Ok(content) = willow_transport::unpack::<willow_messaging::Content>(content_data)
        else {
            return;
        };

        // Decrypt if encrypted.
        let content = match &content {
            willow_messaging::Content::Encrypted(sealed) => {
                let Some(key) = self.key_store.keys.get(topic) else {
                    return;
                };
                match willow_crypto::open_content(sealed, key) {
                    Ok(c) => c,
                    Err(_) => return,
                }
            }
            other => other.clone(),
        };

        let author = self.profiles.display_name(author_peer_id);

        // Handle reactions.
        if let willow_messaging::Content::Reaction {
            ref target,
            ref emoji,
        } = content
        {
            let target_str = target.to_string();
            for m in &mut self.chat.messages {
                if m.id == target_str {
                    m.reactions
                        .entry(emoji.clone())
                        .or_default()
                        .push(author.clone());
                    self.chat.messages_dirty = true;
                    break;
                }
            }
            return;
        }

        // Handle edits.
        if let willow_messaging::Content::Edit {
            ref target,
            ref new_body,
        } = content
        {
            let target_str = target.to_string();
            for m in &mut self.chat.messages {
                if m.id == target_str {
                    m.body = new_body.clone();
                    m.edited = true;
                    self.chat.messages_dirty = true;
                    break;
                }
            }
            return;
        }

        // Handle deletes.
        if let willow_messaging::Content::Delete { ref target } = content {
            let target_str = target.to_string();
            for m in &mut self.chat.messages {
                if m.id == target_str {
                    m.body = "[message deleted]".to_string();
                    m.deleted = true;
                    m.reactions.clear();
                    self.chat.messages_dirty = true;
                    break;
                }
            }
            return;
        }

        // Handle replies.
        if let willow_messaging::Content::Reply {
            ref parent,
            ref body,
        } = content
        {
            let parent_str = parent.to_string();

            let preview = self
                .chat
                .messages
                .iter()
                .find(|m| m.id == parent_str)
                .map(|m| {
                    let text = if m.body.len() > 50 {
                        format!("{}...", &m.body[..50])
                    } else {
                        m.body.clone()
                    };
                    format!("{}: {text}", m.author)
                });

            let mut chat_msg =
                ChatMessage::new(topic.to_string(), author, body.clone(), false, hlc_millis);
            chat_msg.id = op_id.to_string();
            chat_msg.reply_preview = preview;

            self.chat.messages.push(chat_msg);
            self.chat.messages_dirty = true;
            return;
        }

        // Handle text messages.
        if let willow_messaging::Content::Text { ref body } = content {
            let mut chat_msg = ChatMessage::new(
                topic.to_string(),
                author.clone(),
                body.clone(),
                false,
                hlc_millis,
            );
            chat_msg.id = op_id.to_string();

            if let Some(ref db_arc) = self.message_db {
                if let Ok(db_lock) = db_arc.lock() {
                    db_lock.insert(&crate::storage::StoredMessage {
                        topic: topic.to_string(),
                        author: author.clone(),
                        body: body.clone(),
                        is_local: false,
                        timestamp_ms: hlc_millis,
                    });
                }
            }

            let current_topic = self
                .server
                .topic_for_name(&self.chat.current_channel)
                .unwrap_or_default();
            if chat_msg.topic != current_topic {
                *self
                    .unread
                    .counts
                    .entry(chat_msg.topic.clone())
                    .or_insert(0) += 1;
            }

            self.chat.messages.push(chat_msg);
            self.chat.messages_dirty = true;
        }
    }
}
