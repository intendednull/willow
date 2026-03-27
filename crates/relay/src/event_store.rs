//! Relay event store — stores events as they pass through gossipsub
//! and responds to sync requests so peers can catch up from the relay.

use rusqlite::params;
use willow_state::{Event, EventStore, StateHash};

/// SQLite-backed event store for the relay server.
pub struct RelayEventStore {
    conn: rusqlite::Connection,
}

impl RelayEventStore {
    /// Open or create the event store database.
    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;

        // Create base tables (without parent_hash — existing DBs may lack it).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                topic TEXT NOT NULL,
                author TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                raw_data BLOB NOT NULL,
                event_data BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_topic ON events(topic);
            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp_ms);
            CREATE TABLE IF NOT EXISTS event_meta (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );",
        )?;

        // Migrate: add parent_hash column if missing (idempotent).
        let _ = conn.execute_batch(
            "ALTER TABLE events ADD COLUMN parent_hash BLOB NOT NULL DEFAULT X''",
        );
        // Index on parent_hash (safe now that the column exists).
        let _ = conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_events_parent_hash ON events(parent_hash)",
        );

        Ok(Self { conn })
    }

    /// Store a raw gossipsub message alongside the parsed event.
    /// The raw_data is kept so we can re-publish exact bytes for sync responses.
    pub fn store_event(&mut self, topic: &str, event: &Event, raw_data: &[u8]) {
        let event_data = willow_transport::pack(event).unwrap_or_default();
        let parent_hash = event.parent_hash.0.as_slice();
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO events (id, topic, author, timestamp_ms, raw_data, event_data, parent_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.id,
                topic,
                event.author,
                event.timestamp_ms as i64,
                raw_data,
                event_data,
                parent_hash,
            ],
        );
    }

    /// Load events for a specific topic that the requester is missing,
    /// based on their current state hash.
    ///
    /// If `hash` is ZERO the requester has no state — return everything.
    /// Otherwise find the first event whose `parent_hash` matches (the
    /// first event the requester is missing) and return it plus all
    /// subsequent events for the topic.
    pub fn events_for_topic_since_hash(&self, topic: &str, hash: &StateHash) -> Vec<Event> {
        if *hash == StateHash::ZERO {
            return self.all_events_for_topic(topic);
        }

        // Find the timestamp of the first event whose parent_hash matches.
        let since_ts: Option<i64> = self
            .conn
            .query_row(
                "SELECT MIN(timestamp_ms) FROM events WHERE parent_hash = ?1",
                params![hash.0.as_slice()],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        match since_ts {
            Some(ts) => {
                // Return events for this topic at or after that timestamp.
                let mut stmt = match self.conn.prepare(
                    "SELECT event_data FROM events WHERE topic = ?1 AND timestamp_ms >= ?2 ORDER BY timestamp_ms",
                ) {
                    Ok(s) => s,
                    Err(_) => return Vec::new(),
                };
                stmt.query_map(params![topic, ts], |row| row.get::<_, Vec<u8>>(0))
                    .ok()
                    .map(|rows| {
                        rows.filter_map(|r| r.ok())
                            .filter_map(|data| willow_transport::unpack::<Event>(&data).ok())
                            .collect()
                    })
                    .unwrap_or_default()
            }
            // Hash not found — the relay doesn't have the requester's
            // state checkpoint. Return everything so the client can
            // deduplicate on its end.
            None => self.all_events_for_topic(topic),
        }
    }

    /// Return all events for a given topic, ordered by timestamp.
    fn all_events_for_topic(&self, topic: &str) -> Vec<Event> {
        let mut stmt = match self.conn.prepare(
            "SELECT event_data FROM events WHERE topic = ?1 ORDER BY timestamp_ms",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![topic], |row| row.get::<_, Vec<u8>>(0))
            .ok()
            .map(|rows| {
                rows.filter_map(|r| r.ok())
                    .filter_map(|data| willow_transport::unpack::<Event>(&data).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Load all events across all topics since a given timestamp.
    pub fn all_events_since(&self, since_ms: u64) -> Vec<Event> {
        let mut stmt = match self.conn.prepare(
            "SELECT event_data FROM events WHERE timestamp_ms > ?1 ORDER BY timestamp_ms",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![since_ms as i64], |row| row.get::<_, Vec<u8>>(0))
            .ok()
            .map(|rows| {
                rows.filter_map(|r| r.ok())
                    .filter_map(|data| willow_transport::unpack::<Event>(&data).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Total number of stored events.
    pub fn count(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize
    }
}

// Implement EventStore trait for compatibility.
impl EventStore for RelayEventStore {
    fn append(&mut self, event: Event) {
        let event_data = willow_transport::pack(&event).unwrap_or_default();
        let parent_hash = event.parent_hash.0.as_slice();
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO events (id, topic, author, timestamp_ms, raw_data, event_data, parent_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.id,
                "", // no topic context in trait method
                event.author,
                event.timestamp_ms as i64,
                &event_data, // raw_data same as event_data
                event_data,
                parent_hash,
            ],
        );
    }

    fn events_since(&self, hash: &StateHash) -> Vec<Event> {
        if *hash == StateHash::ZERO {
            return self.all_events_since(0);
        }

        // Find the timestamp of the first event whose parent_hash matches.
        let since_ts: Option<i64> = self
            .conn
            .query_row(
                "SELECT MIN(timestamp_ms) FROM events WHERE parent_hash = ?1",
                params![hash.0.as_slice()],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        match since_ts {
            Some(ts) => {
                let mut stmt = match self.conn.prepare(
                    "SELECT event_data FROM events WHERE timestamp_ms >= ?1 ORDER BY timestamp_ms",
                ) {
                    Ok(s) => s,
                    Err(_) => return Vec::new(),
                };
                stmt.query_map(params![ts], |row| row.get::<_, Vec<u8>>(0))
                    .ok()
                    .map(|rows| {
                        rows.filter_map(|r| r.ok())
                            .filter_map(|data| willow_transport::unpack::<Event>(&data).ok())
                            .collect()
                    })
                    .unwrap_or_default()
            }
            None => self.all_events_since(0),
        }
    }

    fn all_events(&self) -> Vec<Event> {
        self.all_events_since(0)
    }

    fn latest_hash(&self) -> StateHash {
        self.conn
            .query_row(
                "SELECT value FROM event_meta WHERE key = 'latest_hash'",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .ok()
            .and_then(|bytes| {
                if bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    Some(StateHash(arr))
                } else {
                    None
                }
            })
            .unwrap_or(StateHash::ZERO)
    }

    fn set_latest_hash(&mut self, hash: StateHash) {
        let _ = self.conn.execute(
            "INSERT OR REPLACE INTO event_meta (key, value) VALUES ('latest_hash', ?1)",
            params![hash.0.as_slice()],
        );
    }

    fn contains(&self, event_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM events WHERE id = ?1",
                params![event_id],
                |_| Ok(()),
            )
            .is_ok()
    }
}
