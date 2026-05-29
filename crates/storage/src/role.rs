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
                // Byte-budget the delta so a single response envelope stays
                // within the gossip layer's 64 KiB ceiling. The worker path is
                // request/response (one `WorkerResponse` per `Sync`), so we
                // serve only the first budget-fitting batch and set
                // `more: true` when further events remain — the requester
                // re-issues `Sync` with advanced heads to drain the rest.
                // `sync_since`'s SQL `LIMIT SYNC_BATCH_LIMIT` is now only an
                // OOM guard on the materialized row set, not the wire bound
                // (see `docs/specs/2026-04-24-negentropy-sync.md`
                // § Wire protocol).
                Ok(delta) => {
                    let (events, more) = willow_common::pack_sync_batches(
                        delta,
                        willow_common::SYNC_ENVELOPE_BUDGET,
                    )
                    .into_iter()
                    .next()
                    // `pack_sync_batches` always yields at least one
                    // (possibly empty) terminator batch.
                    .expect("pack_sync_batches always returns ≥1 batch");
                    WorkerResponse::SyncBatch { events, more }
                }
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

    /// A message whose body is `body_bytes` long, so a known number of them
    /// overflows the per-envelope `SYNC_ENVELOPE_BUDGET`. Used to drive the
    /// byte-budgeted streaming + `more`-flag behaviour.
    fn make_big_message(id: &Identity, seq: u64, prev: EventHash, body_bytes: usize) -> Event {
        Event::new(
            id,
            seq,
            prev,
            vec![],
            EventKind::Message {
                channel_id: "general".to_string(),
                body: "x".repeat(body_bytes),
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

    // ── Byte-budgeted sync streaming + `more` flag (plan PR 4 Task 4.4) ──────
    //
    // The storage `Sync` arm must byte-budget the delta it serves so a single
    // `WorkerResponse::SyncBatch` envelope stays within the gossip layer's
    // 64 KiB ceiling (`willow_common::SYNC_ENVELOPE_BUDGET`), and set `more`:
    //   - `more: true`  when further events remain past this envelope (the
    //     requester re-issues `Sync` with advanced heads),
    //   - `more: false` on the final / only batch (end-of-stream marker).
    // The legacy `SYNC_BATCH_LIMIT` stays solely as an OOM guard on the SQL.

    use willow_common::SYNC_ENVELOPE_BUDGET;

    /// A small delta (well under one envelope) is served in a single batch
    /// with `more: false` — the terminator.
    #[test]
    fn sync_small_delta_is_single_batch_more_false() {
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

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
        });

        match resp {
            WorkerResponse::SyncBatch { events, more } => {
                assert_eq!(events.len(), 3);
                assert!(!more, "a small delta must terminate with more: false");
            }
            other => panic!("expected SyncBatch, got {other:?}"),
        }
    }

    /// When the delta exceeds one envelope's byte budget, the storage role
    /// returns only the budget-fitting first batch with `more: true`, so the
    /// framed envelope cannot exceed `SYNC_ENVELOPE_BUDGET` and the requester
    /// knows to re-issue `Sync` with advanced heads.
    #[test]
    fn sync_large_delta_byte_budgets_first_batch_more_true() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        // ~8 KiB body each → 9 events ≈ 72 KiB > 64 KiB, forcing a split.
        let id = Identity::generate();
        // Genesis first so the chain is well-formed.
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
        for seq in 2..=10 {
            let e = make_big_message(&id, seq, prev, 8 * 1024);
            prev = e.hash;
            role.on_event(&e);
        }

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
        });

        match resp {
            WorkerResponse::SyncBatch { events, more } => {
                assert!(
                    more,
                    "an over-budget delta must set more: true on the first batch"
                );
                assert!(
                    !events.is_empty(),
                    "the first batch must carry at least one event"
                );
                // The packed batch must fit the budget.
                let batch_bytes = bincode::serialized_size(&events).unwrap() as usize;
                assert!(
                    batch_bytes <= SYNC_ENVELOPE_BUDGET,
                    "first batch ({batch_bytes} B) must fit SYNC_ENVELOPE_BUDGET ({SYNC_ENVELOPE_BUDGET} B)"
                );
                // It must NOT contain the whole 10-event delta — that would
                // overflow the envelope.
                assert!(
                    events.len() < 10,
                    "byte-budgeted first batch must not contain all 10 over-budget events"
                );
            }
            other => panic!("expected SyncBatch, got {other:?}"),
        }
    }

    /// A fully caught-up requester still receives exactly one zero-event
    /// terminator (`more: false`) rather than nothing.
    #[test]
    fn sync_caught_up_requester_gets_empty_terminator() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        let id = Identity::generate();
        let mut last_hash = EventHash::ZERO;
        for seq in 1..=3 {
            let e = make_message(&id, seq, last_hash, "general");
            last_hash = e.hash;
            role.on_event(&e);
        }

        // Requester already knows everything (seq 3).
        let mut their_heads = std::collections::BTreeMap::new();
        their_heads.insert(
            id.endpoint_id(),
            willow_state::AuthorHead {
                seq: 3,
                hash: last_hash,
            },
        );

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary { heads: their_heads },
        });

        match resp {
            WorkerResponse::SyncBatch { events, more } => {
                assert!(events.is_empty(), "caught-up requester gets no events");
                assert!(!more, "the empty terminator must set more: false");
            }
            other => panic!("expected empty SyncBatch terminator, got {other:?}"),
        }
    }
}
