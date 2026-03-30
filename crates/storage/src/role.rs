//! Storage node role implementation.
//!
//! Persists events to SQLite, serves paginated history queries.

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
            warn!(event_id = %event.id, %e, "failed to store event");
        }
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::History {
                server_id,
                channel,
                before_timestamp,
                limit,
            } => match self
                .store
                .history(&server_id, &channel, before_timestamp, limit)
            {
                Ok((events, has_more)) => WorkerResponse::HistoryPage { events, has_more },
                Err(e) => WorkerResponse::Denied {
                    reason: format!("query failed: {e}"),
                },
            },
            WorkerRequest::Sync { .. } => WorkerResponse::Denied {
                reason: "storage nodes do not serve sync requests".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StorageEventStore;
    use willow_state::{EventKind, StateHash};

    fn make_message(id: &str, channel: &str, ts: u64) -> Event {
        Event {
            id: id.to_string(),
            parent_hash: StateHash::ZERO,
            author: "peer-1".to_string(),
            timestamp_ms: ts,
            kind: EventKind::Message {
                channel_id: channel.to_string(),
                body: format!("msg {id}"),
                reply_to: None,
            },
        }
    }

    #[test]
    fn storage_role_stores_and_serves_history() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        for i in 0..5u64 {
            role.on_event(&make_message(&format!("e{i}"), "general", (i + 1) * 1000));
        }

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "general".to_string(),
            before_timestamp: None,
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
    fn storage_role_denies_sync_requests() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        });

        match resp {
            WorkerResponse::Denied { reason } => assert!(reason.contains("sync")),
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn role_info_reflects_stored_data() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        role.on_event(&make_message("e1", "general", 1000));

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
    fn on_event_deduplicates_same_id() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        let event = make_message("dup-1", "general", 1000);
        role.on_event(&event);
        role.on_event(&event); // Same event ID

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

        role.on_event(&make_message("e1", "general", 1000));

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "nonexistent".to_string(),
            channel: "general".to_string(),
            before_timestamp: None,
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
    fn history_for_unknown_channel_returns_empty() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        role.on_event(&make_message("e1", "general", 1000));

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "nonexistent".to_string(),
            before_timestamp: None,
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
