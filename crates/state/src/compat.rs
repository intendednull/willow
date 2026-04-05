//! Temporary compatibility shims for downstream crates.
//!
//! These re-export old names and provide stub implementations so the
//! workspace compiles during the migration to the new DAG model.
//! Each item here should be removed as downstream crates are updated.
//!
//! Tracked by: https://github.com/intendednull/willow/issues/24

use crate::event::Event;
use crate::hash::EventHash;
use crate::server::ServerState;

/// Backward-compatible alias for `EventHash`.
pub type StateHash = EventHash;

/// Backward-compatible no-op for `apply_lenient`.
///
/// In the new model, events are applied via `dag.insert()` +
/// `apply_incremental()`. This shim exists only to unblock compilation.
pub fn apply_lenient(_state: &mut ServerState, _event: &Event) -> ApplyResult {
    // No-op — downstream code will be migrated to use the DAG path.
    ApplyResult::Applied
}

/// Backward-compatible no-op for `apply`.
pub fn apply(_state: &mut ServerState, _event: &Event) -> ApplyResult {
    ApplyResult::Applied
}

/// Legacy apply result (matches old API shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyResult {
    Applied,
    AlreadySeen,
    ParentHashMismatch,
    Rejected(String),
}

/// Backward-compatible stub for the old EventStore trait.
pub trait EventStore {
    fn append(&mut self, event: Event);
    fn events_since(&self, hash: &EventHash) -> Vec<Event>;
    fn all_events(&self) -> Vec<Event>;
    fn latest_hash(&self) -> EventHash;
    fn set_latest_hash(&mut self, hash: EventHash);
    fn contains(&self, event_id: &str) -> bool;
}

/// Backward-compatible stub for InMemoryStore.
#[derive(Debug, Clone, Default)]
pub struct InMemoryStore {
    events: Vec<Event>,
    latest_hash: EventHash,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl EventStore for InMemoryStore {
    fn append(&mut self, event: Event) {
        self.events.push(event);
    }

    fn events_since(&self, _hash: &EventHash) -> Vec<Event> {
        self.events.clone()
    }

    fn all_events(&self) -> Vec<Event> {
        self.events.clone()
    }

    fn latest_hash(&self) -> EventHash {
        self.latest_hash
    }

    fn set_latest_hash(&mut self, hash: EventHash) {
        self.latest_hash = hash;
    }

    fn contains(&self, _event_id: &str) -> bool {
        false
    }
}
