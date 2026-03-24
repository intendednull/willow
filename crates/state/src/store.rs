//! Append-only event store trait and in-memory implementation.
//!
//! The [`EventStore`] trait defines the interface for persisting events.
//! `willow-state` provides [`InMemoryStore`] for testing; production
//! backends (SQLite, localStorage) live in `willow-client`.

use crate::hash::StateHash;
use crate::Event;

/// Append-only event log.
///
/// The state crate defines the trait; storage backends implement it.
pub trait EventStore {
    /// Append an event to the log.
    fn append(&mut self, event: Event);

    /// Return all events whose parent hash is at or after the given hash.
    ///
    /// If the hash is not found, returns an empty vec.
    fn events_since(&self, hash: &StateHash) -> Vec<Event>;

    /// Return all events in insertion order.
    fn all_events(&self) -> Vec<Event>;

    /// The hash of the state after the most recent event was applied.
    fn latest_hash(&self) -> StateHash;

    /// Update the latest hash (called after applying an event).
    fn set_latest_hash(&mut self, hash: StateHash);

    /// Check whether an event with the given ID exists in the store.
    fn contains(&self, event_id: &str) -> bool;
}

/// An in-memory event store for testing and ephemeral use.
#[derive(Debug, Clone, Default)]
pub struct InMemoryStore {
    events: Vec<Event>,
    latest_hash: StateHash,
}

impl InMemoryStore {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            latest_hash: StateHash::ZERO,
        }
    }
}

impl EventStore for InMemoryStore {
    fn append(&mut self, event: Event) {
        self.events.push(event);
    }

    fn events_since(&self, hash: &StateHash) -> Vec<Event> {
        // Find the first event whose parent_hash matches, then return
        // that event and everything after it.
        let start = self
            .events
            .iter()
            .position(|e| e.parent_hash == *hash)
            .unwrap_or(self.events.len());
        self.events[start..].to_vec()
    }

    fn all_events(&self) -> Vec<Event> {
        self.events.clone()
    }

    fn latest_hash(&self) -> StateHash {
        self.latest_hash.clone()
    }

    fn set_latest_hash(&mut self, hash: StateHash) {
        self.latest_hash = hash;
    }

    fn contains(&self, event_id: &str) -> bool {
        self.events.iter().any(|e| e.id == event_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventKind;

    fn make_event(id: &str, parent: StateHash) -> Event {
        Event {
            id: id.to_string(),
            parent_hash: parent,
            author: "peer-1".to_string(),
            timestamp_ms: 1000,
            kind: EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: "ch-1".to_string(),
                kind: "text".to_string(),
            },
        }
    }

    #[test]
    fn append_and_retrieve() {
        let mut store = InMemoryStore::new();
        let event = make_event("e1", StateHash::ZERO);
        store.append(event.clone());

        let all = store.all_events();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "e1");
    }

    #[test]
    fn contains_check() {
        let mut store = InMemoryStore::new();
        assert!(!store.contains("e1"));

        store.append(make_event("e1", StateHash::ZERO));
        assert!(store.contains("e1"));
        assert!(!store.contains("e2"));
    }

    #[test]
    fn events_since_returns_from_matching_parent() {
        let mut store = InMemoryStore::new();
        let hash_a = StateHash::from_bytes(b"state-a");
        let hash_b = StateHash::from_bytes(b"state-b");

        store.append(make_event("e1", StateHash::ZERO));
        store.append(make_event("e2", hash_a.clone()));
        store.append(make_event("e3", hash_b));

        let since = store.events_since(&hash_a);
        assert_eq!(since.len(), 2);
        assert_eq!(since[0].id, "e2");
        assert_eq!(since[1].id, "e3");
    }

    #[test]
    fn latest_hash_default_is_zero() {
        let store = InMemoryStore::new();
        assert_eq!(store.latest_hash(), StateHash::ZERO);
    }

    #[test]
    fn set_latest_hash() {
        let mut store = InMemoryStore::new();
        let hash = StateHash::from_bytes(b"new-state");
        store.set_latest_hash(hash.clone());
        assert_eq!(store.latest_hash(), hash);
    }
}
