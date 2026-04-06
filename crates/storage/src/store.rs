//! SQLite-backed event store for the storage node.
//!
//! Stores all events indefinitely and serves paginated history queries.
//! Events are keyed by their content hash and indexed by author/seq for
//! DAG-aware pagination.

use rusqlite::{params, Connection};
use willow_state::{Event, EventKind, HeadsSummary};

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
                hash BLOB PRIMARY KEY,
                server_id TEXT NOT NULL,
                channel_id TEXT NOT NULL DEFAULT '',
                author BLOB NOT NULL,
                seq INTEGER NOT NULL,
                timestamp_hint_ms INTEGER NOT NULL,
                event_data BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_server ON events(server_id);
            CREATE INDEX IF NOT EXISTS idx_events_channel ON events(server_id, channel_id);
            CREATE INDEX IF NOT EXISTS idx_events_author_seq ON events(author, seq);",
        )?;
        Ok(Self { conn })
    }

    /// Store an event. Deduplicates by event hash. Returns true if inserted.
    pub fn store_event(&self, server_id: &str, event: &Event) -> anyhow::Result<bool> {
        let channel_id = extract_channel_id(event);
        let event_data = bincode::serialize(event)?;
        let rows = self.conn.execute(
            "INSERT OR IGNORE INTO events (hash, server_id, channel_id, author, seq, timestamp_hint_ms, event_data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.hash.0.as_slice(),
                server_id,
                channel_id,
                event.author.as_bytes(),
                event.seq as i64,
                event.timestamp_hint_ms as i64,
                event_data,
            ],
        )?;
        Ok(rows > 0)
    }

    /// Query events for a server, optionally filtered by channel, paginated
    /// by HeadsSummary (events before the given heads).
    ///
    /// Returns events whose (author, seq) pairs are before the given heads,
    /// limited to `limit`. The second return value indicates whether more
    /// pages exist.
    pub fn history(
        &self,
        server_id: &str,
        channel: Option<&str>,
        before: Option<&HeadsSummary>,
        limit: u32,
    ) -> anyhow::Result<(Vec<Event>, bool)> {
        let fetch_limit = limit as usize + 1;

        // Build the query dynamically based on filters.
        let mut sql = String::from("SELECT event_data FROM events WHERE server_id = ?1");
        let mut param_idx = 2;

        // Channel filter
        let channel_param: String;
        if let Some(ch) = channel {
            sql.push_str(&format!(" AND channel_id = ?{param_idx}"));
            channel_param = ch.to_string();
            param_idx += 1;
        } else {
            channel_param = String::new();
        }

        // HeadsSummary cursor: exclude events at or after the given heads.
        // For each author in the cursor, only include events with seq < their_seq.
        // For authors NOT in the cursor, include all events.
        // All values are parameterized to prevent SQL injection.
        let mut extra_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        if let Some(heads) = before {
            if !heads.heads.is_empty() {
                let mut conditions = Vec::new();
                let mut author_placeholders = Vec::new();
                for (author, head) in &heads.heads {
                    conditions.push(format!(
                        "(author = ?{} AND seq < ?{})",
                        param_idx,
                        param_idx + 1
                    ));
                    extra_params.push(Box::new(author.as_bytes().to_vec()));
                    extra_params.push(Box::new(head.seq as i64));
                    author_placeholders.push(format!("?{param_idx}"));
                    param_idx += 2;
                }
                let known_authors = author_placeholders.join(", ");
                let cond = format!(
                    " AND (({}) OR author NOT IN ({}))",
                    conditions.join(" OR "),
                    known_authors,
                );
                sql.push_str(&cond);
            }
        }

        sql.push_str(&format!(
            " ORDER BY timestamp_hint_ms DESC, seq DESC LIMIT ?{param_idx}"
        ));

        let mut stmt = self.conn.prepare(&sql)?;

        // Build the full parameter list dynamically.
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(server_id.to_string()));
        if channel.is_some() {
            all_params.push(Box::new(channel_param.clone()));
        }
        all_params.extend(extra_params);
        all_params.push(Box::new(fetch_limit as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| &**p).collect();
        let raw_rows: Vec<Vec<u8>> = stmt
            .query_map(param_refs.as_slice(), |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let events: Vec<Event> = raw_rows
            .iter()
            .filter_map(|data| bincode::deserialize(data).ok())
            .collect();

        let has_more = events.len() > limit as usize;
        let events: Vec<Event> = events.into_iter().take(limit as usize).collect();

        Ok((events, has_more))
    }

    /// Return events the requester is missing based on their HeadsSummary.
    ///
    /// For each author in our store: if the requester has a lower seq (or
    /// doesn't know the author at all), return events with seq > their_seq.
    /// If heads is empty, returns all events for the server.
    /// Maximum events returned in a single sync batch to prevent OOM.
    const SYNC_BATCH_LIMIT: usize = 10_000;

    pub fn sync_since(&self, server_id: &str, heads: &HeadsSummary) -> anyhow::Result<Vec<Event>> {
        if heads.heads.is_empty() {
            // New peer — send up to SYNC_BATCH_LIMIT events for this server.
            let mut stmt = self.conn.prepare(
                "SELECT event_data FROM events WHERE server_id = ?1 ORDER BY seq ASC LIMIT ?2",
            )?;
            let rows: Vec<Vec<u8>> = stmt
                .query_map(
                    rusqlite::params![server_id, Self::SYNC_BATCH_LIMIT as i64],
                    |row| row.get(0),
                )?
                .filter_map(|r| r.ok())
                .collect();
            let events: Vec<Event> = rows
                .iter()
                .filter_map(|data| bincode::deserialize(data).ok())
                .collect();
            return Ok(events);
        }

        // Build a parameterized query for events after the requester's known heads.
        // For known authors: seq > their_seq. For unknown authors: all events.
        let mut sql = String::from("SELECT event_data FROM events WHERE server_id = ?1 AND (");
        let mut param_idx = 2u32;
        let mut extra_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        let mut conditions = Vec::new();
        let mut author_placeholders = Vec::new();
        for (author, head) in &heads.heads {
            conditions.push(format!(
                "(author = ?{} AND seq > ?{})",
                param_idx,
                param_idx + 1
            ));
            extra_params.push(Box::new(author.as_bytes().to_vec()));
            extra_params.push(Box::new(head.seq as i64));
            author_placeholders.push(format!("?{param_idx}"));
            param_idx += 2;
        }

        // Events from authors not in the heads (they don't know about them).
        let known_authors = author_placeholders.join(", ");
        conditions.push(format!("author NOT IN ({known_authors})"));

        sql.push_str(&conditions.join(" OR "));
        sql.push_str(&format!(
            ") ORDER BY seq ASC LIMIT {}",
            Self::SYNC_BATCH_LIMIT
        ));

        let mut stmt = self.conn.prepare(&sql)?;

        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(server_id.to_string()));
        all_params.extend(extra_params);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| &**p).collect();

        let rows: Vec<Vec<u8>> = stmt
            .query_map(param_refs.as_slice(), |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        let events: Vec<Event> = rows
            .iter()
            .filter_map(|data| bincode::deserialize(data).ok())
            .collect();

        Ok(events)
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
    use willow_identity::Identity;
    use willow_state::{EventHash, EventKind};

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

    fn setup_identity_and_genesis(channel: &str) -> (Identity, Event) {
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
        let _ = channel; // channel used by caller
        (id, genesis)
    }

    #[test]
    fn store_and_count() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis("general");
        let event = make_message(&id, 2, genesis.hash, "general");
        assert!(store.store_event("srv-1", &event).unwrap());
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn deduplicates_by_event_hash() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis("general");
        let event = make_message(&id, 2, genesis.hash, "general");
        assert!(store.store_event("srv-1", &event).unwrap());
        assert!(!store.store_event("srv-1", &event).unwrap());
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn history_returns_newest_first() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis("general");

        let mut prev = genesis.hash;
        for seq in 2..=6 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            store.store_event("srv-1", &e).unwrap();
        }

        let (events, has_more) = store.history("srv-1", Some("general"), None, 3).unwrap();
        assert_eq!(events.len(), 3);
        assert!(has_more);
        // Ordered by seq DESC
        assert_eq!(events[0].seq, 6);
        assert_eq!(events[1].seq, 5);
        assert_eq!(events[2].seq, 4);
    }

    #[test]
    fn history_pagination_with_heads_cursor() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis("general");

        let mut prev = genesis.hash;
        let mut event_hashes = vec![];
        for seq in 2..=11 {
            let e = make_message(&id, seq, prev, "general");
            prev = e.hash;
            event_hashes.push((seq, e.hash));
            store.store_event("srv-1", &e).unwrap();
        }

        // First page: no cursor.
        let (page1, has_more1) = store.history("srv-1", Some("general"), None, 3).unwrap();
        assert_eq!(page1.len(), 3);
        assert!(has_more1);
        assert_eq!(page1[0].seq, 11);

        // Second page: cursor at the last event of page 1.
        let last_seq = page1.last().unwrap().seq;
        let last_hash = page1.last().unwrap().hash;
        let mut cursor_heads = std::collections::HashMap::new();
        cursor_heads.insert(
            id.endpoint_id(),
            willow_state::AuthorHead {
                seq: last_seq,
                hash: last_hash,
            },
        );
        let cursor = HeadsSummary {
            heads: cursor_heads,
        };
        let (page2, _) = store
            .history("srv-1", Some("general"), Some(&cursor), 3)
            .unwrap();
        assert_eq!(page2.len(), 3);
        // All events in page2 should have seq < last_seq
        for e in &page2 {
            assert!(e.seq < last_seq);
        }
    }

    #[test]
    fn history_filters_by_channel() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis("general");

        let e1 = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e1).unwrap();

        let e2 = make_message(&id, 3, e1.hash, "random");
        store.store_event("srv-1", &e2).unwrap();

        let e3 = make_message(&id, 4, e2.hash, "general");
        store.store_event("srv-1", &e3).unwrap();

        let (events, _) = store.history("srv-1", Some("general"), None, 10).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn history_all_channels() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis("general");

        let e1 = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e1).unwrap();

        let e2 = make_message(&id, 3, e1.hash, "random");
        store.store_event("srv-1", &e2).unwrap();

        let (events, _) = store.history("srv-1", None, None, 10).unwrap();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn history_filters_by_server() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis("general");

        let e1 = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e1).unwrap();

        let e2 = make_message(&id, 3, e1.hash, "general");
        store.store_event("srv-2", &e2).unwrap();

        let (events, _) = store.history("srv-1", Some("general"), None, 10).unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn server_count_tracks_distinct_servers() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis("general");

        let e1 = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e1).unwrap();

        let e2 = make_message(&id, 3, e1.hash, "general");
        store.store_event("srv-2", &e2).unwrap();

        let e3 = make_message(&id, 4, e2.hash, "general");
        store.store_event("srv-1", &e3).unwrap();

        assert_eq!(store.server_count().unwrap(), 2);
    }

    #[test]
    fn disk_usage_returns_nonzero_after_insert() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (id, genesis) = setup_identity_and_genesis("general");

        let e = make_message(&id, 2, genesis.hash, "general");
        store.store_event("srv-1", &e).unwrap();
        assert!(store.disk_usage_bytes().unwrap() > 0);
    }

    #[test]
    fn empty_history_returns_no_events() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (events, has_more) = store.history("srv-1", Some("general"), None, 10).unwrap();
        assert!(events.is_empty());
        assert!(!has_more);
    }
}
