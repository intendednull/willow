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
