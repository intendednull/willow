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
        Ok(Self { conn })
    }

    /// Store a raw gossipsub message alongside the parsed event.
    /// The raw_data is kept so we can re-publish exact bytes for sync responses.
    pub fn store_event(&mut self, topic: &str, event: &Event, raw_data: &[u8]) {
        let event_data = willow_transport::pack(event).unwrap_or_default();
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO events (id, topic, author, timestamp_ms, raw_data, event_data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id,
                topic,
                event.author,
                event.timestamp_ms as i64,
                raw_data,
                event_data,
            ],
        );
    }

    /// Load events for a specific topic since a given timestamp.
    #[allow(dead_code)]
    pub fn events_for_topic_since(
        &self,
        topic: &str,
        since_ms: u64,
    ) -> Vec<Event> {
        let mut stmt = match self.conn.prepare(
            "SELECT event_data FROM events WHERE topic = ?1 AND timestamp_ms > ?2 ORDER BY timestamp_ms LIMIT 500",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![topic, since_ms as i64], |row| {
            row.get::<_, Vec<u8>>(0)
        })
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
            "SELECT event_data FROM events WHERE timestamp_ms > ?1 ORDER BY timestamp_ms LIMIT 1000",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(params![since_ms as i64], |row| {
            row.get::<_, Vec<u8>>(0)
        })
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
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO events (id, topic, author, timestamp_ms, raw_data, event_data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id,
                "", // no topic context in trait method
                event.author,
                event.timestamp_ms as i64,
                &event_data, // raw_data same as event_data
                event_data,
            ],
        );
    }

    fn events_since(&self, hash: &StateHash) -> Vec<Event> {
        // For the relay, we use timestamp-based lookup instead of hash-based.
        // Return all events (the relay doesn't track state hashes).
        let _ = hash;
        self.all_events_since(0)
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
