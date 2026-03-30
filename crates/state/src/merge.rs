//! Divergence detection and merge strategy.
//!
//! When two peers have different event histories, they can merge by finding
//! their common ancestor state, collecting divergent events, sorting by
//! timestamp, and replaying from the common ancestor.

use crate::hash::StateHash;
use crate::server::ServerState;
use crate::{apply_lenient, Event};

/// Find the most recent common ancestor hash between two event logs.
///
/// Walks both logs and returns the last `parent_hash` that appears in both.
/// Returns `None` if the logs share no common ancestor (completely disjoint).
pub fn find_common_ancestor(our_events: &[Event], their_events: &[Event]) -> Option<StateHash> {
    // Collect all parent hashes from their events (including the implicit
    // genesis ZERO hash).
    let their_hashes: std::collections::HashSet<StateHash> =
        their_events.iter().map(|e| e.parent_hash.clone()).collect();

    // Walk our events in reverse to find the most recent shared parent hash.
    for event in our_events.iter().rev() {
        if their_hashes.contains(&event.parent_hash) {
            return Some(event.parent_hash.clone());
        }
    }

    // Check if both start from genesis.
    if our_events
        .first()
        .is_some_and(|e| e.parent_hash == StateHash::ZERO)
        && their_events
            .first()
            .is_some_and(|e| e.parent_hash == StateHash::ZERO)
    {
        return Some(StateHash::ZERO);
    }

    None
}

/// Merge two divergent event histories into a single canonical state.
///
/// Starting from `common_state` (the state at the common ancestor), this
/// function:
/// 1. Collects all events from both logs that come after the common ancestor.
/// 2. Deduplicates by event ID.
/// 3. Sorts by timestamp (wall-clock hint), with event ID as tiebreaker.
/// 4. Replays events leniently onto the common state.
///
/// Returns the merged state and the canonical event ordering.
pub fn merge(
    our_events: &[Event],
    their_events: &[Event],
    common_state: &ServerState,
) -> (ServerState, Vec<Event>) {
    let common_hash = common_state.hash();

    // Collect events after the common ancestor from both logs.
    let our_divergent = events_after(our_events, &common_hash);
    let their_divergent = events_after(their_events, &common_hash);

    // Merge and deduplicate.
    let mut seen = std::collections::HashSet::new();
    let mut merged_events: Vec<Event> = Vec::new();

    for event in our_divergent.into_iter().chain(their_divergent) {
        if seen.insert(event.id.clone()) {
            merged_events.push(event);
        }
    }

    // Sort by timestamp, then by event ID for deterministic ordering.
    merged_events.sort_by(|a, b| {
        a.timestamp_ms
            .cmp(&b.timestamp_ms)
            .then_with(|| a.id.cmp(&b.id))
    });

    // Replay onto the common state.
    let mut state = common_state.clone();
    for event in &merged_events {
        let _ = apply_lenient(&mut state, event);
    }

    (state, merged_events)
}

/// Return events that come after the given hash in the log.
///
/// An event is "after" the common ancestor if it appears at or after the
/// first event whose parent_hash matches `hash`.
fn events_after(events: &[Event], hash: &StateHash) -> Vec<Event> {
    let start = events
        .iter()
        .position(|e| e.parent_hash == *hash)
        .unwrap_or(events.len());
    events[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventKind;
    use willow_identity::Identity;

    fn owner_id() -> willow_identity::EndpointId {
        Identity::generate().endpoint_id()
    }

    fn make_event(
        id: &str,
        parent: StateHash,
        ts: u64,
        author: willow_identity::EndpointId,
    ) -> Event {
        Event {
            id: id.to_string(),
            parent_hash: parent,
            author,
            timestamp_ms: ts,
            kind: EventKind::CreateChannel {
                name: format!("ch-{id}"),
                channel_id: format!("chid-{id}"),
                kind: "text".to_string(),
            },
        }
    }

    #[test]
    fn find_common_ancestor_at_genesis() {
        let owner = owner_id();
        let our = vec![make_event("e1", StateHash::ZERO, 100, owner)];
        let their = vec![make_event("e2", StateHash::ZERO, 200, owner)];

        let ancestor = find_common_ancestor(&our, &their);
        assert_eq!(ancestor, Some(StateHash::ZERO));
    }

    #[test]
    fn find_common_ancestor_after_shared_prefix() {
        let owner = owner_id();
        let shared_hash = StateHash::from_bytes(b"shared");
        let our = vec![
            make_event("e1", StateHash::ZERO, 100, owner),
            make_event("e2", shared_hash.clone(), 200, owner),
        ];
        let their = vec![
            make_event("e1", StateHash::ZERO, 100, owner),
            make_event("e3", shared_hash.clone(), 300, owner),
        ];

        let ancestor = find_common_ancestor(&our, &their);
        assert_eq!(ancestor, Some(shared_hash));
    }

    #[test]
    fn find_common_ancestor_disjoint() {
        let owner = owner_id();
        let our = vec![make_event("e1", StateHash::from_bytes(b"a"), 100, owner)];
        let their = vec![make_event("e2", StateHash::from_bytes(b"b"), 200, owner)];

        let ancestor = find_common_ancestor(&our, &their);
        assert_eq!(ancestor, None);
    }

    #[test]
    fn merge_deduplicates_events() {
        let owner = owner_id();
        let common = ServerState::new("s1", "Test", owner);
        let common_hash = common.hash();

        let shared = make_event("e1", common_hash.clone(), 100, owner);
        let our = vec![shared.clone()];
        let their = vec![shared];

        let (_, events) = merge(&our, &their, &common);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn merge_sorts_by_timestamp() {
        let owner = owner_id();
        let common = ServerState::new("s1", "Test", owner);
        let common_hash = common.hash();

        let our = vec![make_event("e2", common_hash.clone(), 200, owner)];
        let their = vec![make_event("e1", common_hash, 100, owner)];

        let (_, events) = merge(&our, &their, &common);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, "e1");
        assert_eq!(events[1].id, "e2");
    }
}
