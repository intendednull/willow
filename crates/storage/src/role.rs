//! Storage node role implementation.
//!
//! Persists events to SQLite, serves paginated history queries using
//! DAG-aware [`HeadsSummary`] cursors.

use tracing::warn;
use willow_state::Event;
use willow_worker::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};

use crate::store::StorageEventStore;

/// The storage node's WorkerRole implementation.
pub struct StorageRole {
    store: StorageEventStore,
    /// Server ID is determined by the gossipsub topic in the full
    /// runtime. For now, default to a fixed value.
    default_server_id: String,
}

impl StorageRole {
    /// Create a new storage role with the given event store.
    pub fn new(store: StorageEventStore) -> Self {
        Self {
            store,
            default_server_id: "default".to_string(),
        }
    }

    /// Set the default server ID (used when server can't be inferred).
    pub fn set_default_server(&mut self, id: String) {
        self.default_server_id = id;
    }
}

impl WorkerRole for StorageRole {
    fn role_info(&self) -> WorkerRoleInfo {
        let total = self.store.count().unwrap_or_else(|e| {
            warn!(%e, "failed to query event count");
            0
        });
        let disk = self.store.disk_usage_bytes().unwrap_or_else(|e| {
            warn!(%e, "failed to query disk usage");
            0
        });
        let servers = self.store.server_count().unwrap_or_else(|e| {
            warn!(%e, "failed to query server count");
            0
        });
        WorkerRoleInfo::Storage {
            servers_tracked: servers,
            total_events_stored: total,
            disk_used_bytes: disk,
        }
    }

    fn on_event(&mut self, event: &Event) {
        if let Err(e) = self.store.store_event(&self.default_server_id, event) {
            warn!(event_hash = ?event.hash, %e, "failed to store event");
        }
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::History {
                server_id,
                channel,
                before,
                limit,
            } => match self
                .store
                .history(&server_id, channel.as_deref(), before.as_ref(), limit)
            {
                Ok((events, has_more)) => WorkerResponse::HistoryPage { events, has_more },
                Err(e) => WorkerResponse::Denied {
                    reason: format!("query failed: {e}"),
                },
            },
            WorkerRequest::Sync {
                server_id, heads, ..
            } => match self.store.sync_since(&server_id, &heads) {
                Ok(events) => WorkerResponse::SyncBatch {
                    events,
                    more: false,
                },
                Err(e) => WorkerResponse::Denied {
                    reason: format!("sync query failed: {e}"),
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StorageEventStore;
    use willow_identity::Identity;
    use willow_state::{EventHash, EventKind, HeadsSummary};

    fn make_message(id: &Identity, seq: u64, prev: EventHash, channel: &str) -> Event {
        Event::new(
            id,
            seq,
            prev,
            vec![],
            EventKind::Message {
                channel_id: channel.to_string(),
                body: format!("msg seq={seq}"),
                reply_to: None,
            },
            seq * 1000,
        )
    }

    #[test]
    fn storage_role_stores_and_serves_history() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        let id = Identity::generate();
        let genesis = Event::new(
            &id,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer {
                name: "test".to_string(),
            },
            0,
        );

        role.on_event(&genesis);
        let mut prev = genesis.hash;
        for seq in 2..=6 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            role.on_event(&e);
        }

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: Some("general".to_string()),
            before: None,
            limit: 3,
        });

        match resp {
            WorkerResponse::HistoryPage { events, has_more } => {
                assert_eq!(events.len(), 3);
                assert!(has_more);
            }
            _ => panic!("expected HistoryPage"),
        }
    }

    #[test]
    fn storage_role_serves_sync_requests() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        let id = Identity::generate();
        let mut prev = EventHash::ZERO;
        for seq in 1..=3 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            role.on_event(&e);
        }

        // Empty heads = new peer, should get all events.
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
        });

        match resp {
            WorkerResponse::SyncBatch { events, .. } => assert_eq!(events.len(), 3),
            _ => panic!("expected SyncBatch, got {:?}", resp),
        }
    }

    #[test]
    fn storage_role_sync_returns_delta() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        let id = Identity::generate();
        let mut prev = EventHash::ZERO;
        let mut hashes = vec![];
        for seq in 1..=5 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            hashes.push((seq, e.hash));
            role.on_event(&e);
        }

        // Peer knows up to seq 3 — should get seq 4 and 5.
        let mut their_heads = std::collections::BTreeMap::new();
        their_heads.insert(
            id.endpoint_id(),
            willow_state::AuthorHead {
                seq: 3,
                hash: hashes[2].1,
            },
        );
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary { heads: their_heads },
        });

        match resp {
            WorkerResponse::SyncBatch { events, .. } => assert_eq!(events.len(), 2),
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn role_info_reflects_stored_data() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        let id = Identity::generate();
        let e = make_message(&id, 1, EventHash::ZERO, "general");
        role.on_event(&e);

        match role.role_info() {
            WorkerRoleInfo::Storage {
                total_events_stored,
                servers_tracked,
                ..
            } => {
                assert_eq!(total_events_stored, 1);
                assert_eq!(servers_tracked, 1);
            }
            _ => panic!("expected Storage"),
        }
    }

    #[test]
    fn on_event_deduplicates_same_hash() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        let id = Identity::generate();
        let event = make_message(&id, 1, EventHash::ZERO, "general");
        role.on_event(&event);
        role.on_event(&event); // Same event hash

        match role.role_info() {
            WorkerRoleInfo::Storage {
                total_events_stored,
                ..
            } => assert_eq!(total_events_stored, 1),
            _ => panic!("expected Storage"),
        }
    }

    #[test]
    fn history_for_unknown_server_returns_empty() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        let id = Identity::generate();
        let e = make_message(&id, 1, EventHash::ZERO, "general");
        role.on_event(&e);

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "nonexistent".to_string(),
            channel: Some("general".to_string()),
            before: None,
            limit: 10,
        });

        match resp {
            WorkerResponse::HistoryPage { events, has_more } => {
                assert!(events.is_empty());
                assert!(!has_more);
            }
            _ => panic!("expected empty HistoryPage"),
        }
    }

    #[test]
    fn set_default_server_routes_subsequent_events() {
        // Focused round-trip test for `set_default_server`.
        //
        // The setter mutates an in-memory field (no SQLite persistence — the
        // field exists only on `StorageRole`, not the `StorageEventStore`),
        // so we verify the change via the observable read path: `on_event`
        // routes events through the configured server ID, which `handle_request`
        // then queries by server ID.
        //
        // Asserts:
        //   1. Default ID is `"default"` (events stored without calling the
        //      setter are queryable under `"default"`).
        //   2. After `set_default_server("srv-A")`, new events route to
        //      `"srv-A"` and are NOT visible under the previous ID.
        //   3. Last-write-wins: after a second `set_default_server("srv-B")`,
        //      newer events route to `"srv-B"` and earlier ones remain under
        //      their original server.
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        let id = Identity::generate();

        // (1) Default value — no setter call yet.
        let e1 = make_message(&id, 1, EventHash::ZERO, "general");
        role.on_event(&e1);

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "default".to_string(),
            channel: Some("general".to_string()),
            before: None,
            limit: 10,
        });
        match resp {
            WorkerResponse::HistoryPage { events, .. } => assert_eq!(
                events.len(),
                1,
                "event before set_default_server should land under \"default\""
            ),
            _ => panic!("expected HistoryPage"),
        }

        // (2) Set to "srv-A" and verify routing changes.
        role.set_default_server("srv-A".to_string());
        let e2 = make_message(&id, 2, e1.hash, "general");
        role.on_event(&e2);

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-A".to_string(),
            channel: Some("general".to_string()),
            before: None,
            limit: 10,
        });
        match resp {
            WorkerResponse::HistoryPage { events, .. } => assert_eq!(
                events.len(),
                1,
                "event after set_default_server(\"srv-A\") should land under \"srv-A\""
            ),
            _ => panic!("expected HistoryPage"),
        }

        // The "default" bucket still has only the first event — proves the
        // setter took effect rather than aliasing both IDs.
        let resp = role.handle_request(WorkerRequest::History {
            server_id: "default".to_string(),
            channel: Some("general".to_string()),
            before: None,
            limit: 10,
        });
        match resp {
            WorkerResponse::HistoryPage { events, .. } => assert_eq!(
                events.len(),
                1,
                "\"default\" bucket should not gain events after switching servers"
            ),
            _ => panic!("expected HistoryPage"),
        }

        // (3) Last-write-wins: switch again to "srv-B".
        role.set_default_server("srv-B".to_string());
        let e3 = make_message(&id, 3, e2.hash, "general");
        role.on_event(&e3);

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-B".to_string(),
            channel: Some("general".to_string()),
            before: None,
            limit: 10,
        });
        match resp {
            WorkerResponse::HistoryPage { events, .. } => assert_eq!(
                events.len(),
                1,
                "event after second set_default_server should land under \"srv-B\""
            ),
            _ => panic!("expected HistoryPage"),
        }

        // "srv-A" bucket retains its single event.
        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-A".to_string(),
            channel: Some("general".to_string()),
            before: None,
            limit: 10,
        });
        match resp {
            WorkerResponse::HistoryPage { events, .. } => assert_eq!(
                events.len(),
                1,
                "\"srv-A\" bucket should be unchanged after switching to \"srv-B\""
            ),
            _ => panic!("expected HistoryPage"),
        }
    }

    #[test]
    fn history_for_unknown_channel_returns_empty() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        let id = Identity::generate();
        let e = make_message(&id, 1, EventHash::ZERO, "general");
        role.on_event(&e);

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: Some("nonexistent".to_string()),
            before: None,
            limit: 10,
        });

        match resp {
            WorkerResponse::HistoryPage { events, has_more } => {
                assert!(events.is_empty());
                assert!(!has_more);
            }
            _ => panic!("expected empty HistoryPage"),
        }
    }
}
