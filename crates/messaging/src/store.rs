//! # Message Storage
//!
//! Pluggable backends for persisting and querying messages.
//!
//! The [`MessageStore`] trait defines the interface that every storage backend
//! must implement. [`InMemoryStore`] is a simple reference implementation that
//! keeps everything in RAM — perfect for tests and lightweight nodes.

use std::collections::{BTreeMap, HashMap, HashSet};

use willow_identity::EndpointId;

use crate::{hlc::HlcTimestamp, ChannelId, Message, MessageId, MessageValidationError};

/// Delivery state for a single message.
///
/// Used by the sync-queue UI in [`willow_client`] to classify a message
/// as `Pending` (not yet acked by at least one recipient) or `Delivered`
/// (every expected recipient acked).
///
/// The per-recipient ack mechanism lives in the client-layer actor
/// bus — this enum is the storage-facing snapshot the UI queries.
///
/// See [`docs/specs/2026-04-19-ui-design/sync-queue.md`] §Data shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryState {
    /// Every expected recipient has acknowledged this message.
    Delivered,
    /// No recipient has acknowledged yet; the inner set is the
    /// pending-recipient cohort.
    PendingAllRecipients(HashSet<EndpointId>),
    /// Some recipients have acked; others have not.
    PendingSomeRecipients {
        /// Recipients that have acknowledged.
        acked: HashSet<EndpointId>,
        /// Recipients that have not yet acknowledged.
        pending: HashSet<EndpointId>,
    },
}

/// Errors that can occur during storage operations.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Attempted to insert a message whose ID already exists.
    #[error("duplicate message id: {0}")]
    DuplicateId(MessageId),

    /// The requested message was not found.
    #[error("message not found: {0}")]
    NotFound(MessageId),

    /// Attempted to insert a message that failed structural validation
    /// (e.g. peer-supplied `Content::File` with an oversized filename
    /// or MIME type). See [`MessageValidationError`] for the variants.
    #[error("invalid message: {0}")]
    Invalid(#[from] MessageValidationError),
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

    /// Returns the delivery state for `id`, or `None` when unknown.
    ///
    /// Default impl returns `Some(DeliveryState::Delivered)` so stores
    /// without delivery tracking behave as "everything delivered" — the
    /// Phase 2b sync-queue work only upgrades `InMemoryStore` here.
    fn delivery_state(&self, _id: &MessageId) -> Option<DeliveryState> {
        Some(DeliveryState::Delivered)
    }
}

/// A simple in-memory message store.
///
/// Messages are stored in a `HashMap` keyed by [`MessageId`] for O(1) lookup,
/// and indexed by [`ChannelId`] using a `BTreeMap<HlcTimestamp, Vec<MessageId>>`
/// for naturally sorted channel iteration without per-insert sorting.
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
/// let peer = Identity::generate().endpoint_id();
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
    /// Index: channel ID → BTreeMap of HLC timestamp → message IDs.
    ///
    /// Using `BTreeMap` gives naturally sorted iteration by timestamp, so
    /// `insert` is O(log N) instead of the previous O(N log N) sort-on-every-insert.
    /// The `Vec<MessageId>` value handles the (rare) case where two messages share
    /// the exact same HLC timestamp.
    channel_index: HashMap<ChannelId, BTreeMap<HlcTimestamp, Vec<MessageId>>>,
    /// Per-message delivery tracking.
    ///
    /// - `acked[id]`   = recipients that have acknowledged the message.
    /// - `pending[id]` = recipients the message is still pending for.
    ///
    /// When a message has no entry in either map the store reports
    /// [`DeliveryState::Delivered`] (the permissive default expected by
    /// the trait).
    acked: HashMap<MessageId, HashSet<EndpointId>>,
    pending: HashMap<MessageId, HashSet<EndpointId>>,
}

impl InMemoryStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the cohort of expected recipients for a newly-sent
    /// message. Every recipient starts in the `pending` set.
    ///
    /// Call this after [`insert`](Self::insert) when the sender knows
    /// which peers must ack the message for it to count as delivered.
    pub fn mark_pending<I>(&mut self, id: MessageId, recipients: I)
    where
        I: IntoIterator<Item = EndpointId>,
    {
        let set: HashSet<EndpointId> = recipients.into_iter().collect();
        if set.is_empty() {
            self.pending.remove(&id);
            self.acked.remove(&id);
            return;
        }
        self.pending.insert(id.clone(), set);
        self.acked.entry(id).or_default();
    }

    /// Mark a single `peer` as having acknowledged `id`. Moves the peer
    /// from the pending → acked sets. A no-op when the message has no
    /// tracking entry or the peer is not in the pending set.
    pub fn ack(&mut self, id: &MessageId, peer: EndpointId) {
        if let Some(pending) = self.pending.get_mut(id) {
            if pending.remove(&peer) {
                self.acked.entry(id.clone()).or_default().insert(peer);
                if pending.is_empty() {
                    // Terminal state: every recipient acked. Drop the
                    // tracking entry so `delivery_state` falls back to
                    // the permissive `Delivered` default.
                    self.pending.remove(id);
                    self.acked.remove(id);
                }
            }
        }
    }

    /// Mark every expected recipient for `id` as acknowledged. Clears
    /// the tracking entry so the message reports as `Delivered`.
    pub fn ack_all(&mut self, id: &MessageId) {
        self.pending.remove(id);
        self.acked.remove(id);
    }
}

impl MessageStore for InMemoryStore {
    fn insert(&mut self, message: Message) -> Result<(), StoreError> {
        // Structural validation guards against unbounded peer-supplied
        // `Content::File` strings (filename, mime_type) being persisted
        // without ever passing through a length check. Tracked in #583.
        //
        // NOTE: Ideally validation also gates earlier, at the network
        // ingress in `crates/client/src/listeners.rs`, before a `Message`
        // ever reaches a store. Those files are in flight under PR #566
        // (coordinator decision); revisit after merge so peers can't
        // pin oversized payloads in client memory even when the store
        // is bypassed.
        message.validate()?;

        if self.messages.contains_key(&message.id) {
            return Err(StoreError::DuplicateId(message.id));
        }

        let id = message.id.clone();
        let channel_id = message.channel_id.clone();
        let hlc = message.hlc;

        self.messages.insert(id.clone(), message);

        self.channel_index
            .entry(channel_id)
            .or_default()
            .entry(hlc)
            .or_default()
            .push(id);

        Ok(())
    }

    fn get(&self, id: &MessageId) -> Result<&Message, StoreError> {
        self.messages
            .get(id)
            .ok_or_else(|| StoreError::NotFound(id.clone()))
    }

    fn list_channel(&self, channel_id: &ChannelId) -> Vec<&Message> {
        match self.channel_index.get(channel_id) {
            None => Vec::new(),
            Some(by_ts) => by_ts
                .values()
                .flat_map(|ids| ids.iter().filter_map(|id| self.messages.get(id)))
                .collect(),
        }
    }

    fn len(&self) -> usize {
        self.messages.len()
    }

    fn delivery_state(&self, id: &MessageId) -> Option<DeliveryState> {
        // Message we have never heard of — `None`.
        if !self.messages.contains_key(id) {
            return None;
        }
        // No tracking entry at all — treat as delivered (permissive
        // default matching the trait docstring).
        let pending = match self.pending.get(id) {
            Some(set) if !set.is_empty() => set,
            _ => return Some(DeliveryState::Delivered),
        };
        let acked = self.acked.get(id).cloned().unwrap_or_default();
        if acked.is_empty() {
            Some(DeliveryState::PendingAllRecipients(pending.clone()))
        } else {
            Some(DeliveryState::PendingSomeRecipients {
                acked,
                pending: pending.clone(),
            })
        }
    }
}

// ───── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hlc::HLC;
    use willow_identity::Identity;

    fn make_text_msg(channel: &ChannelId, hlc: &mut HLC) -> Message {
        let peer = Identity::generate().endpoint_id();
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

    #[test]
    fn messages_returned_in_hlc_order_after_random_insertion() {
        let mut store = InMemoryStore::new();
        let channel = ChannelId::new();
        let peer = Identity::generate().endpoint_id();

        // Construct messages with explicit out-of-order HLC timestamps.
        let make_msg = |millis: u64, counter: u32| -> Message {
            let mut m = Message::text(channel.clone(), peer, "test", &mut HLC::new());
            m.hlc = HlcTimestamp { millis, counter };
            m
        };

        let msg_t5 = make_msg(1000, 5);
        let msg_t1 = make_msg(1000, 1);
        let msg_t3 = make_msg(1000, 3);
        let msg_t200 = make_msg(2000, 0);
        let msg_t0 = make_msg(500, 0);

        // Insert in deliberately scrambled order.
        store.insert(msg_t5.clone()).unwrap();
        store.insert(msg_t200.clone()).unwrap();
        store.insert(msg_t1.clone()).unwrap();
        store.insert(msg_t0.clone()).unwrap();
        store.insert(msg_t3.clone()).unwrap();

        let listed = store.list_channel(&channel);
        assert_eq!(listed.len(), 5);

        // Verify strictly ascending HLC order.
        for window in listed.windows(2) {
            assert!(
                window[0].hlc <= window[1].hlc,
                "expected {:?} <= {:?}",
                window[0].hlc,
                window[1].hlc
            );
        }

        // Verify the exact order: t0, t1, t3, t5, t200.
        assert_eq!(listed[0].id, msg_t0.id);
        assert_eq!(listed[1].id, msg_t1.id);
        assert_eq!(listed[2].id, msg_t3.id);
        assert_eq!(listed[3].id, msg_t5.id);
        assert_eq!(listed[4].id, msg_t200.id);
    }

    #[test]
    fn delivery_state_defaults_to_delivered() {
        // A freshly stored message with no tracking entry reports as
        // `Delivered` — the permissive default per the trait docstring.
        let mut store = InMemoryStore::new();
        let mut hlc = HLC::new();
        let channel = ChannelId::new();
        let msg = make_text_msg(&channel, &mut hlc);
        let id = msg.id.clone();
        store.insert(msg).unwrap();
        assert_eq!(store.delivery_state(&id), Some(DeliveryState::Delivered));
    }

    #[test]
    fn delivery_state_unknown_message_returns_none() {
        let store = InMemoryStore::new();
        assert_eq!(store.delivery_state(&MessageId::new()), None);
    }

    #[test]
    fn mark_pending_then_ack_one_moves_to_pending_some() {
        let mut store = InMemoryStore::new();
        let mut hlc = HLC::new();
        let channel = ChannelId::new();
        let msg = make_text_msg(&channel, &mut hlc);
        let id = msg.id.clone();
        store.insert(msg).unwrap();

        let alice = Identity::generate().endpoint_id();
        let bob = Identity::generate().endpoint_id();
        store.mark_pending(id.clone(), [alice, bob]);

        // Both pending, nothing acked → PendingAllRecipients.
        match store.delivery_state(&id) {
            Some(DeliveryState::PendingAllRecipients(set)) => {
                assert_eq!(set.len(), 2);
                assert!(set.contains(&alice));
                assert!(set.contains(&bob));
            }
            other => panic!("expected PendingAllRecipients, got {other:?}"),
        }

        store.ack(&id, alice);

        // One acked, one pending → PendingSomeRecipients.
        match store.delivery_state(&id) {
            Some(DeliveryState::PendingSomeRecipients { acked, pending }) => {
                assert!(acked.contains(&alice));
                assert!(pending.contains(&bob));
                assert_eq!(acked.len(), 1);
                assert_eq!(pending.len(), 1);
            }
            other => panic!("expected PendingSomeRecipients, got {other:?}"),
        }
    }

    #[test]
    fn ack_all_transitions_to_delivered() {
        let mut store = InMemoryStore::new();
        let mut hlc = HLC::new();
        let channel = ChannelId::new();
        let msg = make_text_msg(&channel, &mut hlc);
        let id = msg.id.clone();
        store.insert(msg).unwrap();

        let alice = Identity::generate().endpoint_id();
        let bob = Identity::generate().endpoint_id();
        store.mark_pending(id.clone(), [alice, bob]);

        // Ack each recipient individually — also drains to Delivered.
        store.ack(&id, alice);
        store.ack(&id, bob);
        assert_eq!(store.delivery_state(&id), Some(DeliveryState::Delivered));

        // Using the `ack_all` fast path from a fresh pending state.
        let msg2 = make_text_msg(&channel, &mut hlc);
        let id2 = msg2.id.clone();
        store.insert(msg2).unwrap();
        store.mark_pending(id2.clone(), [alice, bob]);
        store.ack_all(&id2);
        assert_eq!(store.delivery_state(&id2), Some(DeliveryState::Delivered));
    }

    #[test]
    fn duplicate_hlc_timestamps_handled_gracefully() {
        let mut store = InMemoryStore::new();
        let channel = ChannelId::new();
        let peer = Identity::generate().endpoint_id();

        let ts = HlcTimestamp {
            millis: 9999,
            counter: 0,
        };

        // Two different messages sharing the exact same HLC timestamp.
        let mut m1 = Message::text(channel.clone(), peer, "first", &mut HLC::new());
        m1.hlc = ts;
        let mut m2 = Message::text(channel.clone(), peer, "second", &mut HLC::new());
        m2.hlc = ts;

        store.insert(m1).unwrap();
        store.insert(m2).unwrap();

        let listed = store.list_channel(&channel);
        assert_eq!(listed.len(), 2, "both messages must be present");
        assert_eq!(listed[0].hlc, ts);
        assert_eq!(listed[1].hlc, ts);
    }
}
