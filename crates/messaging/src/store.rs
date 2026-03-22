//! # Message Storage
//!
//! Pluggable backends for persisting and querying messages.
//!
//! The [`MessageStore`] trait defines the interface that every storage backend
//! must implement. [`InMemoryStore`] is a simple reference implementation that
//! keeps everything in RAM — perfect for tests and lightweight nodes.

use std::collections::HashMap;

use crate::{ChannelId, Message, MessageId};

/// Errors that can occur during storage operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Attempted to insert a message whose ID already exists.
    #[error("duplicate message id: {0}")]
    DuplicateId(MessageId),

    /// The requested message was not found.
    #[error("message not found: {0}")]
    NotFound(MessageId),
}

/// Trait for message storage backends.
///
/// Implementations must support inserting, retrieving, and listing messages by
/// channel. All operations are synchronous; async wrappers can be layered on
/// top via `tokio::task::spawn_blocking` if needed.
pub trait MessageStore: Send + Sync {
    /// Insert a message. Returns an error if the message ID already exists.
    fn insert(&mut self, message: Message) -> Result<(), StoreError>;

    /// Retrieve a single message by ID.
    fn get(&self, id: &MessageId) -> Result<&Message, StoreError>;

    /// List all messages in a channel, ordered by HLC timestamp.
    fn list_channel(&self, channel_id: &ChannelId) -> Vec<&Message>;

    /// Total number of stored messages.
    fn len(&self) -> usize;

    /// Whether the store is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A simple in-memory message store.
///
/// Messages are stored in a `HashMap` keyed by [`MessageId`] and indexed by
/// [`ChannelId`] for fast channel listing.
///
/// **Not persistent** — all data is lost when the process exits. Use this for
/// testing or as a starting point before implementing a disk-backed store.
///
/// # Examples
///
/// ```
/// use willow_messaging::{Message, ChannelId, hlc::HLC, store::{InMemoryStore, MessageStore}};
/// use willow_identity::Identity;
///
/// let mut store = InMemoryStore::new();
/// let mut hlc = HLC::new();
/// let peer = Identity::generate().peer_id();
/// let channel = ChannelId::new();
///
/// let msg = Message::text(channel.clone(), peer, "hello", &mut hlc);
/// store.insert(msg).unwrap();
///
/// assert_eq!(store.list_channel(&channel).len(), 1);
/// ```
#[derive(Debug, Default)]
pub struct InMemoryStore {
    /// All messages keyed by their unique ID.
    messages: HashMap<MessageId, Message>,
    /// Index: channel ID → ordered list of message IDs.
    channel_index: HashMap<ChannelId, Vec<MessageId>>,
}

impl InMemoryStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl MessageStore for InMemoryStore {
    fn insert(&mut self, message: Message) -> Result<(), StoreError> {
        if self.messages.contains_key(&message.id) {
            return Err(StoreError::DuplicateId(message.id));
        }

        let id = message.id.clone();
        let channel_id = message.channel_id.clone();

        self.messages.insert(id.clone(), message);

        let ids = self.channel_index.entry(channel_id).or_default();
        ids.push(id);

        // Keep the channel index sorted by HLC timestamp.
        ids.sort_by(|a, b| {
            let ma = &self.messages[a];
            let mb = &self.messages[b];
            ma.hlc.cmp(&mb.hlc)
        });

        Ok(())
    }

    fn get(&self, id: &MessageId) -> Result<&Message, StoreError> {
        self.messages
            .get(id)
            .ok_or_else(|| StoreError::NotFound(id.clone()))
    }

    fn list_channel(&self, channel_id: &ChannelId) -> Vec<&Message> {
        self.channel_index
            .get(channel_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.messages.get(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn len(&self) -> usize {
        self.messages.len()
    }
}

// ───── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::HLC;
    use willow_identity::Identity;

    fn make_text_msg(channel: &ChannelId, hlc: &mut HLC) -> Message {
        let peer = Identity::generate().peer_id();
        Message::text(channel.clone(), peer, "test", hlc)
    }

    #[test]
    fn insert_and_get() {
        let mut store = InMemoryStore::new();
        let mut hlc = HLC::new();
        let channel = ChannelId::new();
        let msg = make_text_msg(&channel, &mut hlc);
        let id = msg.id.clone();

        store.insert(msg).unwrap();

        let retrieved = store.get(&id).unwrap();
        assert_eq!(retrieved.id, id);
    }

    #[test]
    fn duplicate_insert_rejected() {
        let mut store = InMemoryStore::new();
        let mut hlc = HLC::new();
        let channel = ChannelId::new();
        let msg = make_text_msg(&channel, &mut hlc);

        store.insert(msg.clone()).unwrap();
        let result = store.insert(msg);

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), StoreError::DuplicateId(_)));
    }

    #[test]
    fn get_missing_returns_not_found() {
        let store = InMemoryStore::new();
        let result = store.get(&MessageId::new());
        assert!(matches!(result.unwrap_err(), StoreError::NotFound(_)));
    }

    #[test]
    fn list_channel_returns_ordered() {
        let mut store = InMemoryStore::new();
        let mut hlc = HLC::new();
        let channel = ChannelId::new();

        let m1 = make_text_msg(&channel, &mut hlc);
        let m2 = make_text_msg(&channel, &mut hlc);
        let m3 = make_text_msg(&channel, &mut hlc);

        // Insert out of order.
        store.insert(m3.clone()).unwrap();
        store.insert(m1.clone()).unwrap();
        store.insert(m2.clone()).unwrap();

        let listed = store.list_channel(&channel);
        assert_eq!(listed.len(), 3);
        assert!(listed[0].hlc <= listed[1].hlc);
        assert!(listed[1].hlc <= listed[2].hlc);
    }

    #[test]
    fn list_empty_channel() {
        let store = InMemoryStore::new();
        let channel = ChannelId::new();
        assert!(store.list_channel(&channel).is_empty());
    }

    #[test]
    fn messages_are_channel_isolated() {
        let mut store = InMemoryStore::new();
        let mut hlc = HLC::new();
        let ch_a = ChannelId::new();
        let ch_b = ChannelId::new();

        store.insert(make_text_msg(&ch_a, &mut hlc)).unwrap();
        store.insert(make_text_msg(&ch_a, &mut hlc)).unwrap();
        store.insert(make_text_msg(&ch_b, &mut hlc)).unwrap();

        assert_eq!(store.list_channel(&ch_a).len(), 2);
        assert_eq!(store.list_channel(&ch_b).len(), 1);
    }

    #[test]
    fn len_and_is_empty() {
        let mut store = InMemoryStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        let mut hlc = HLC::new();
        let channel = ChannelId::new();
        store.insert(make_text_msg(&channel, &mut hlc)).unwrap();

        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
    }
}
