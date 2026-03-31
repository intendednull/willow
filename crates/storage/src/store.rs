//! SQLite-backed event store for the storage node.
//!
//! Stores all events indefinitely and serves paginated history queries.

use rusqlite::{params, Connection};
use willow_state::{Event, EventKind};

/// SQLite-backed event store.
pub struct StorageEventStore {
    conn: Connection,
}

impl StorageEventStore {
    /// Open or create the database at `path`.
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                server_id TEXT NOT NULL,
                channel_id TEXT NOT NULL DEFAULT '',
                author TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                event_data BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_server ON events(server_id);
            CREATE INDEX IF NOT EXISTS idx_events_channel_ts ON events(server_id, channel_id, timestamp_ms);
            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp_ms);",
        )?;
        Ok(Self { conn })
    }

    /// Store an event. Deduplicates by event ID. Returns true if inserted.
    pub fn store_event(&self, server_id: &str, event: &Event) -> anyhow::Result<bool> {
        let channel_id = extract_channel_id(event);
        let event_data = bincode::serialize(event)?;
        let rows = self.conn.execute(
            "INSERT OR IGNORE INTO events (id, server_id, channel_id, author, timestamp_ms, event_data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id,
                server_id,
                channel_id,
                event.author.to_string(),
                event.timestamp_ms,
                event_data,
            ],
        )?;
        Ok(rows > 0)
    }

    /// Query events for a channel, paginated by timestamp (descending).
    ///
    /// Returns events older than `before_timestamp`, limited to `limit`.
    /// The second return value indicates whether more pages exist.
    pub fn history(
        &self,
        server_id: &str,
        channel: &str,
        before_timestamp: Option<u64>,
        limit: u32,
    ) -> anyhow::Result<(Vec<Event>, bool)> {
        let fetch_limit = limit as usize + 1;

        let raw_rows: Vec<Vec<u8>> = if let Some(before) = before_timestamp {
            let mut stmt = self.conn.prepare(
                "SELECT event_data FROM events
                 WHERE server_id = ?1 AND channel_id = ?2 AND timestamp_ms < ?3
                 ORDER BY timestamp_ms DESC
                 LIMIT ?4",
            )?;
            let rows: Vec<Vec<u8>> = stmt
                .query_map(
                    params![server_id, channel, before as i64, fetch_limit as i64],
                    |row| row.get(0),
                )?
                .filter_map(|r| r.ok())
                .collect();
            rows
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT event_data FROM events
                 WHERE server_id = ?1 AND channel_id = ?2
                 ORDER BY timestamp_ms DESC
                 LIMIT ?3",
            )?;
            let rows: Vec<Vec<u8>> = stmt
                .query_map(params![server_id, channel, fetch_limit as i64], |row| {
                    row.get(0)
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        let events: Vec<Event> = raw_rows
            .iter()
            .filter_map(|data| bincode::deserialize(data).ok())
            .collect();

        let has_more = events.len() > limit as usize;
        let events: Vec<Event> = events.into_iter().take(limit as usize).collect();

        Ok((events, has_more))
    }

    /// Total number of stored events.
    pub fn count(&self) -> anyhow::Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    /// Total disk usage estimate in bytes.
    pub fn disk_usage_bytes(&self) -> anyhow::Result<u64> {
        let pages: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let page_size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))?;
        Ok((pages * page_size) as u64)
    }

    /// Number of distinct servers tracked.
    pub fn server_count(&self) -> anyhow::Result<u32> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(DISTINCT server_id) FROM events", [], |row| {
                    row.get(0)
                })?;
        Ok(count as u32)
    }
}

/// Extract channel_id from an event, if applicable.
fn extract_channel_id(event: &Event) -> String {
    match &event.kind {
        EventKind::Message { channel_id, .. }
        | EventKind::PinMessage { channel_id, .. }
        | EventKind::UnpinMessage { channel_id, .. }
        | EventKind::CreateChannel { channel_id, .. }
        | EventKind::DeleteChannel { channel_id, .. }
        | EventKind::RenameChannel { channel_id, .. }
        | EventKind::RotateChannelKey { channel_id, .. } => channel_id.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_state::StateHash;

    fn make_message(id: &str, channel: &str, ts: u64) -> Event {
        Event {
            id: id.to_string(),
            parent_hash: StateHash::ZERO,
            author: willow_identity::Identity::generate().endpoint_id(),
            timestamp_ms: ts,
            kind: EventKind::Message {
                channel_id: channel.to_string(),
                body: format!("msg {id}"),
                reply_to: None,
            },
        }
    }

    #[test]
    fn store_and_count() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let event = make_message("e1", "general", 1000);
        assert!(store.store_event("srv-1", &event).unwrap());
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn deduplicates_by_event_id() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let event = make_message("e1", "general", 1000);
        assert!(store.store_event("srv-1", &event).unwrap());
        assert!(!store.store_event("srv-1", &event).unwrap());
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn history_returns_newest_first() {
        let store = StorageEventStore::open(":memory:").unwrap();
        for i in 0..5u64 {
            store
                .store_event(
                    "srv-1",
                    &make_message(&format!("e{i}"), "general", (i + 1) * 1000),
                )
                .unwrap();
        }

        let (events, has_more) = store.history("srv-1", "general", None, 3).unwrap();
        assert_eq!(events.len(), 3);
        assert!(has_more);
        assert_eq!(events[0].timestamp_ms, 5000);
        assert_eq!(events[1].timestamp_ms, 4000);
        assert_eq!(events[2].timestamp_ms, 3000);
    }

    #[test]
    fn history_pagination_with_cursor() {
        let store = StorageEventStore::open(":memory:").unwrap();
        for i in 0..10u64 {
            store
                .store_event(
                    "srv-1",
                    &make_message(&format!("e{i}"), "general", (i + 1) * 1000),
                )
                .unwrap();
        }

        let (page1, has_more1) = store.history("srv-1", "general", None, 3).unwrap();
        assert_eq!(page1.len(), 3);
        assert!(has_more1);
        assert_eq!(page1[0].timestamp_ms, 10000);

        let cursor = page1.last().unwrap().timestamp_ms;
        let (page2, has_more2) = store.history("srv-1", "general", Some(cursor), 3).unwrap();
        assert_eq!(page2.len(), 3);
        assert!(has_more2);
        assert_eq!(page2[0].timestamp_ms, 7000);

        let cursor = page2.last().unwrap().timestamp_ms;
        let (page3, has_more3) = store.history("srv-1", "general", Some(cursor), 3).unwrap();
        assert_eq!(page3.len(), 3);
        assert!(has_more3);

        let cursor = page3.last().unwrap().timestamp_ms;
        let (page4, has_more4) = store.history("srv-1", "general", Some(cursor), 3).unwrap();
        assert_eq!(page4.len(), 1);
        assert!(!has_more4);
    }

    #[test]
    fn history_filters_by_channel() {
        let store = StorageEventStore::open(":memory:").unwrap();
        store
            .store_event("srv-1", &make_message("e1", "general", 1000))
            .unwrap();
        store
            .store_event("srv-1", &make_message("e2", "random", 2000))
            .unwrap();
        store
            .store_event("srv-1", &make_message("e3", "general", 3000))
            .unwrap();

        let (events, _) = store.history("srv-1", "general", None, 10).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn history_filters_by_server() {
        let store = StorageEventStore::open(":memory:").unwrap();
        store
            .store_event("srv-1", &make_message("e1", "general", 1000))
            .unwrap();
        store
            .store_event("srv-2", &make_message("e2", "general", 2000))
            .unwrap();

        let (events, _) = store.history("srv-1", "general", None, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "e1");
    }

    #[test]
    fn server_count_tracks_distinct_servers() {
        let store = StorageEventStore::open(":memory:").unwrap();
        store
            .store_event("srv-1", &make_message("e1", "general", 1000))
            .unwrap();
        store
            .store_event("srv-2", &make_message("e2", "general", 2000))
            .unwrap();
        store
            .store_event("srv-1", &make_message("e3", "general", 3000))
            .unwrap();

        assert_eq!(store.server_count().unwrap(), 2);
    }

    #[test]
    fn disk_usage_returns_nonzero_after_insert() {
        let store = StorageEventStore::open(":memory:").unwrap();
        store
            .store_event("srv-1", &make_message("e1", "general", 1000))
            .unwrap();
        assert!(store.disk_usage_bytes().unwrap() > 0);
    }

    #[test]
    fn empty_history_returns_no_events() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (events, has_more) = store.history("srv-1", "general", None, 10).unwrap();
        assert!(events.is_empty());
        assert!(!has_more);
    }
}
