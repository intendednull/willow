//! # Storage
//!
//! Platform-specific persistence for identity, server state, channel keys,
//! and network settings.
//!
//! - **Native**: files in `~/.local/share/willow/`
//! - **WASM**: browser `localStorage`

use std::collections::HashMap;

use willow_channel::Server;
use willow_crypto::ChannelKey;

// ---- Public types -----------------------------------------------------------

/// Persisted channel key data: maps topic -> key bytes.
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct SavedKeys(pub Vec<(String, [u8; 32])>);

/// Persisted network settings.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct NetworkSettings {
    /// Relay multiaddr string, e.g. `/ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooW...`
    pub relay_addr: Option<String>,
}

/// Persisted local user profile.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct LocalProfile {
    pub display_name: String,
}

pub fn save_profile(profile: &LocalProfile) {
    if let Ok(bytes) = willow_transport::pack(profile) {
        save_raw("profile", &bytes);
    }
}

pub fn load_profile() -> Option<LocalProfile> {
    willow_transport::unpack(&load_raw("profile")?).ok()
}

// ---- Public API -------------------------------------------------------------

pub fn save_server(server: &Server, keys: &HashMap<String, ChannelKey>) {
    if let Ok(bytes) = willow_transport::pack(server) {
        save_raw("server", &bytes);
    }
    let saved = SavedKeys(
        keys.iter()
            .map(|(topic, key)| (topic.clone(), *key.as_bytes()))
            .collect(),
    );
    if let Ok(bytes) = willow_transport::pack(&saved) {
        save_raw("keys", &bytes);
    }
}

pub fn load_server() -> Option<(Server, HashMap<String, ChannelKey>)> {
    let server: Server = willow_transport::unpack(&load_raw("server")?).ok()?;
    let saved: SavedKeys = willow_transport::unpack(&load_raw("keys")?).ok()?;
    let keys = saved
        .0
        .into_iter()
        .map(|(topic, bytes)| (topic, ChannelKey::from_bytes(bytes)))
        .collect();
    Some((server, keys))
}

pub fn save_identity_bytes(bytes: &[u8]) {
    save_raw("identity", bytes);
}

pub fn load_identity_bytes() -> Option<Vec<u8>> {
    load_raw("identity")
}

pub fn save_settings(settings: &NetworkSettings) {
    if let Ok(bytes) = willow_transport::pack(settings) {
        save_raw("settings", &bytes);
    }
}

pub fn load_settings() -> Option<NetworkSettings> {
    willow_transport::unpack(&load_raw("settings")?).ok()
}

// ---- Multi-server persistence -----------------------------------------------

/// Save a single server context by ID.
pub fn save_server_by_id(id: &str, server: &Server, keys: &HashMap<String, ChannelKey>) {
    if let Ok(bytes) = willow_transport::pack(server) {
        save_raw(&format!("srv_{id}"), &bytes);
    }
    let saved = SavedKeys(
        keys.iter()
            .map(|(topic, key)| (topic.clone(), *key.as_bytes()))
            .collect(),
    );
    if let Ok(bytes) = willow_transport::pack(&saved) {
        save_raw(&format!("srvkeys_{id}"), &bytes);
    }
}

/// Load a single server context by ID.
pub fn load_server_by_id(id: &str) -> Option<(Server, HashMap<String, ChannelKey>)> {
    let server: Server = willow_transport::unpack(&load_raw(&format!("srv_{id}"))?).ok()?;
    let saved: SavedKeys = willow_transport::unpack(&load_raw(&format!("srvkeys_{id}"))?).ok()?;
    let keys = saved
        .0
        .into_iter()
        .map(|(topic, bytes)| (topic, ChannelKey::from_bytes(bytes)))
        .collect();
    Some((server, keys))
}

/// Save the list of known server IDs.
pub fn save_server_list(ids: &[String]) {
    if let Ok(bytes) = willow_transport::pack(&ids.to_vec()) {
        save_raw("server_list", &bytes);
    }
}

/// Load the list of known server IDs.
pub fn load_server_list() -> Option<Vec<String>> {
    willow_transport::unpack(&load_raw("server_list")?).ok()
}

/// Save the full event-sourced ServerState for a specific server.
pub fn save_server_state(id: &str, state: &willow_state::ServerState) {
    if let Ok(bytes) = willow_transport::pack(state) {
        save_raw(&format!("srv_state_{id}"), &bytes);
    }
}

/// Load the full event-sourced ServerState for a specific server.
pub fn load_server_state(id: &str) -> Option<willow_state::ServerState> {
    willow_transport::unpack(&load_raw(&format!("srv_state_{id}"))?).ok()
}

// ---- Message Persistence ----------------------------------------------------

/// A stored chat message for display. Lightweight compared to the full
/// `willow_messaging::Message` -- just what the UI needs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredMessage {
    pub topic: String,
    pub author: String,
    pub body: String,
    pub is_local: bool,
    pub timestamp_ms: u64,
    /// Unique message ID (op_id) for deduplication.
    #[serde(default)]
    pub msg_id: String,
}

/// Open (or create) the message database and return a handle.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_message_db() -> Option<MessageDb> {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("messages.db");
    MessageDb::open(&path)
}

/// SQLite-backed message storage (native only).
#[cfg(not(target_arch = "wasm32"))]
pub struct MessageDb {
    conn: rusqlite::Connection,
}

#[cfg(not(target_arch = "wasm32"))]
impl MessageDb {
    /// Open a database at a specific path (used by tests and `open_message_db`).
    pub fn open_path(path: impl AsRef<std::path::Path>) -> Option<Self> {
        Self::open(path.as_ref())
    }

    fn open(path: &std::path::Path) -> Option<Self> {
        let conn = rusqlite::Connection::open(path).ok()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                topic TEXT NOT NULL,
                author TEXT NOT NULL,
                body TEXT NOT NULL,
                is_local INTEGER NOT NULL DEFAULT 0,
                timestamp_ms INTEGER NOT NULL,
                msg_id TEXT NOT NULL DEFAULT ''
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_msg_id ON messages(msg_id) WHERE msg_id != '';
            CREATE INDEX IF NOT EXISTS idx_messages_topic ON messages(topic, timestamp_ms);",
        )
        .ok()?;
        // Migration: add msg_id column if it doesn't exist (existing DBs).
        let _ =
            conn.execute_batch("ALTER TABLE messages ADD COLUMN msg_id TEXT NOT NULL DEFAULT '';");
        Some(Self { conn })
    }

    /// Insert a message, deduplicating by msg_id.
    pub fn insert(&self, msg: &StoredMessage) {
        if msg.msg_id.is_empty() {
            let _ = self.conn.execute(
                "INSERT INTO messages (topic, author, body, is_local, timestamp_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![msg.topic, msg.author, msg.body, msg.is_local as i32, msg.timestamp_ms],
            );
        } else {
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO messages (topic, author, body, is_local, timestamp_ms, msg_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![msg.topic, msg.author, msg.body, msg.is_local as i32, msg.timestamp_ms, msg.msg_id],
            );
        }
    }

    /// Load messages for a topic, ordered by timestamp.
    pub fn load_topic(&self, topic: &str, limit: usize) -> Vec<StoredMessage> {
        let mut stmt = match self.conn.prepare(
            "SELECT topic, author, body, is_local, timestamp_ms, msg_id FROM messages WHERE topic = ?1 ORDER BY timestamp_ms ASC LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(rusqlite::params![topic, limit as i64], |row| {
            Ok(StoredMessage {
                topic: row.get(0)?,
                author: row.get(1)?,
                body: row.get(2)?,
                is_local: row.get::<_, i32>(3)? != 0,
                timestamp_ms: row.get(4)?,
                msg_id: row.get::<_, String>(5).unwrap_or_default(),
            })
        })
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Load all topics that have messages.
    pub fn topics(&self) -> Vec<String> {
        let mut stmt = match self.conn.prepare("SELECT DISTINCT topic FROM messages") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |row| row.get(0))
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    /// Count messages for a topic.
    pub fn count_topic(&self, topic: &str) -> usize {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE topic = ?1",
                rusqlite::params![topic],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
    }
}

/// WASM message storage backed by localStorage.
///
/// Each topic's messages are stored as a serialized `Vec<StoredMessage>` under
/// the key `willow_msgs_<topic_hash>`. The topic is hashed to avoid
/// localStorage key length issues with long multiaddr-based topic names.
#[cfg(target_arch = "wasm32")]
pub struct MessageDb;

#[cfg(target_arch = "wasm32")]
impl MessageDb {
    fn msg_key(topic: &str) -> String {
        // Simple hash to keep keys short. Not cryptographic -- just for storage.
        let mut h: u64 = 5381;
        for b in topic.bytes() {
            h = h.wrapping_mul(33).wrapping_add(b as u64);
        }
        format!("willow_msgs_{h:x}")
    }

    fn load_all(topic: &str) -> Vec<StoredMessage> {
        let Some(bytes) = load_raw(&Self::msg_key(topic)) else {
            return Vec::new();
        };
        willow_transport::unpack::<Vec<StoredMessage>>(&bytes).unwrap_or_default()
    }

    fn save_all(topic: &str, messages: &[StoredMessage]) {
        if let Ok(bytes) = willow_transport::pack(&messages.to_vec()) {
            save_raw(&Self::msg_key(topic), &bytes);
        }
    }

    pub fn insert(&self, msg: &StoredMessage) {
        let mut messages = Self::load_all(&msg.topic);
        // Dedup by msg_id.
        if !msg.msg_id.is_empty() && messages.iter().any(|m| m.msg_id == msg.msg_id) {
            return;
        }
        messages.push(msg.clone());
        // Keep at most 500 messages per topic to avoid localStorage limits.
        if messages.len() > 500 {
            messages.drain(..messages.len() - 500);
        }
        Self::save_all(&msg.topic, &messages);
    }

    pub fn load_topic(&self, topic: &str, limit: usize) -> Vec<StoredMessage> {
        let messages = Self::load_all(topic);
        if messages.len() > limit {
            messages[messages.len() - limit..].to_vec()
        } else {
            messages
        }
    }

    pub fn topics(&self) -> Vec<String> {
        // Can't enumerate localStorage keys by prefix efficiently.
        // Return empty -- topic list comes from ServerState instead.
        Vec::new()
    }

    pub fn count_topic(&self, topic: &str) -> usize {
        Self::load_all(topic).len()
    }
}

#[cfg(target_arch = "wasm32")]
pub fn open_message_db() -> Option<MessageDb> {
    Some(MessageDb)
}

// ---- Persistent EventStore backends -----------------------------------------

/// SQLite-backed event store for persistent event-sourced state (native only).
///
/// Events are stored in an `events` table with deduplication by event ID.
/// The `latest_hash` is stored as a single-row metadata table.
#[cfg(not(target_arch = "wasm32"))]
pub struct SqliteEventStore {
    conn: rusqlite::Connection,
}

#[cfg(not(target_arch = "wasm32"))]
impl SqliteEventStore {
    /// Open or create an event store database at the given path.
    pub fn open(path: &std::path::Path) -> Option<Self> {
        let conn = rusqlite::Connection::open(path).ok()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                parent_hash BLOB NOT NULL,
                author TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                event_data BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_parent ON events(parent_hash);
            CREATE TABLE IF NOT EXISTS event_meta (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );",
        )
        .ok()?;
        Some(Self { conn })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl willow_state::EventStore for SqliteEventStore {
    fn append(&mut self, event: willow_state::Event) {
        let event_data = willow_transport::pack(&event).unwrap_or_default();
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO events (id, parent_hash, author, timestamp_ms, event_data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                event.id,
                event.parent_hash.0.as_slice(),
                event.author,
                event.timestamp_ms as i64,
                event_data,
            ],
        );
    }

    fn events_since(&self, hash: &willow_state::StateHash) -> Vec<willow_state::Event> {
        // ZERO hash means "give me everything".
        if *hash == willow_state::StateHash::ZERO {
            return self.all_events();
        }

        // Find the rowid of the first event whose parent_hash matches,
        // then return that event and everything after it.
        let start_rowid: Option<i64> = self
            .conn
            .query_row(
                "SELECT MIN(rowid) FROM events WHERE parent_hash = ?1",
                rusqlite::params![hash.0.as_slice()],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        let Some(start) = start_rowid else {
            return Vec::new();
        };

        let mut stmt = match self
            .conn
            .prepare("SELECT event_data FROM events WHERE rowid >= ?1 ORDER BY rowid")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map(rusqlite::params![start], |row| row.get::<_, Vec<u8>>(0))
            .ok()
            .map(|rows| {
                rows.filter_map(|r| r.ok())
                    .filter_map(|data| willow_transport::unpack::<willow_state::Event>(&data).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn all_events(&self) -> Vec<willow_state::Event> {
        let mut stmt = match self
            .conn
            .prepare("SELECT event_data FROM events ORDER BY rowid")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))
            .ok()
            .map(|rows| {
                rows.filter_map(|r| r.ok())
                    .filter_map(|data| willow_transport::unpack::<willow_state::Event>(&data).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn latest_hash(&self) -> willow_state::StateHash {
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
                    Some(willow_state::StateHash(arr))
                } else {
                    None
                }
            })
            .unwrap_or(willow_state::StateHash::ZERO)
    }

    fn set_latest_hash(&mut self, hash: willow_state::StateHash) {
        let _ = self.conn.execute(
            "INSERT OR REPLACE INTO event_meta (key, value) VALUES ('latest_hash', ?1)",
            rusqlite::params![hash.0.as_slice()],
        );
    }

    fn contains(&self, event_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM events WHERE id = ?1",
                rusqlite::params![event_id],
                |_| Ok(()),
            )
            .is_ok()
    }
}

/// Open (or create) the event store database and return a handle.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_event_store(server_id: &str) -> Option<SqliteEventStore> {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(format!("events_{server_id}.db"));
    SqliteEventStore::open(&path)
}

/// localStorage-backed event store for persistent event-sourced state (WASM only).
///
/// Events are serialized as a `Vec<Event>` under a single localStorage key.
/// Capped at 2000 events to stay within browser storage limits.
#[cfg(target_arch = "wasm32")]
pub struct LocalStorageEventStore {
    /// localStorage key for this store's event data.
    key: String,
    /// localStorage key for this store's latest hash.
    hash_key: String,
}

#[cfg(target_arch = "wasm32")]
impl LocalStorageEventStore {
    /// Maximum events stored to avoid exceeding localStorage limits.
    const MAX_EVENTS: usize = 2000;

    /// Create a new store for the given server ID.
    pub fn new(server_id: &str) -> Self {
        Self {
            key: format!("willow_events_{server_id}"),
            hash_key: format!("willow_events_hash_{server_id}"),
        }
    }

    /// Load all stored events from localStorage.
    fn load_all(&self) -> Vec<willow_state::Event> {
        load_raw(&self.key)
            .and_then(|bytes| willow_transport::unpack(&bytes).ok())
            .unwrap_or_default()
    }

    /// Save all events to localStorage.
    fn save_all(&self, events: &[willow_state::Event]) {
        if let Ok(bytes) = willow_transport::pack(&events.to_vec()) {
            save_raw(&self.key, &bytes);
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl willow_state::EventStore for LocalStorageEventStore {
    fn append(&mut self, event: willow_state::Event) {
        let mut events = self.load_all();
        // Dedup by event ID.
        if events.iter().any(|e| e.id == event.id) {
            return;
        }
        events.push(event);
        // Cap to avoid exceeding localStorage limits.
        if events.len() > Self::MAX_EVENTS {
            events.drain(..events.len() - Self::MAX_EVENTS);
        }
        self.save_all(&events);
    }

    fn events_since(&self, hash: &willow_state::StateHash) -> Vec<willow_state::Event> {
        // ZERO hash means "give me everything".
        if *hash == willow_state::StateHash::ZERO {
            return self.all_events();
        }
        let events = self.load_all();
        let start = events
            .iter()
            .position(|e| e.parent_hash == *hash)
            .unwrap_or(events.len());
        events[start..].to_vec()
    }

    fn all_events(&self) -> Vec<willow_state::Event> {
        self.load_all()
    }

    fn latest_hash(&self) -> willow_state::StateHash {
        load_raw(&self.hash_key)
            .and_then(|bytes| {
                if bytes.len() == 32 {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    Some(willow_state::StateHash(arr))
                } else {
                    None
                }
            })
            .unwrap_or(willow_state::StateHash::ZERO)
    }

    fn set_latest_hash(&mut self, hash: willow_state::StateHash) {
        save_raw(&self.hash_key, &hash.0);
    }

    fn contains(&self, event_id: &str) -> bool {
        self.load_all().iter().any(|e| e.id == event_id)
    }
}

/// Open (or create) the localStorage event store.
#[cfg(target_arch = "wasm32")]
pub fn open_event_store(server_id: &str) -> Option<LocalStorageEventStore> {
    Some(LocalStorageEventStore::new(server_id))
}

/// Save a downloaded file to the downloads directory. Returns the path.
#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
pub fn save_download(filename: &str, data: &[u8]) -> Option<std::path::PathBuf> {
    let dir = dirs::download_dir().unwrap_or_else(|| data_dir().join("downloads"));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(filename);
    std::fs::write(&path, data).ok()?;
    Some(path)
}

// ---- Native implementation --------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn data_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("willow")
}

#[cfg(not(target_arch = "wasm32"))]
fn save_raw(key: &str, data: &[u8]) {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("{key}.bin")), data);
}

#[cfg(not(target_arch = "wasm32"))]
fn load_raw(key: &str) -> Option<Vec<u8>> {
    std::fs::read(data_dir().join(format!("{key}.bin"))).ok()
}

// ---- WASM implementation (localStorage) -------------------------------------

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
fn save_raw(key: &str, data: &[u8]) {
    if let Some(storage) = local_storage() {
        let encoded = crate::base64::encode(data);
        let _ = storage.set_item(&format!("willow_{key}"), &encoded);
    }
}

#[cfg(target_arch = "wasm32")]
fn load_raw(key: &str) -> Option<Vec<u8>> {
    let storage = local_storage()?;
    let encoded = storage.get_item(&format!("willow_{key}")).ok()??;
    crate::base64::decode(&encoded)
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use willow_state::{Event, EventKind, EventStore, StateHash};

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

    #[cfg(not(target_arch = "wasm32"))]
    mod sqlite_event_store {
        use super::*;

        fn temp_store() -> SqliteEventStore {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test_events.db");
            // Keep the dir alive by leaking it (test only).
            let store = SqliteEventStore::open(&path).unwrap();
            std::mem::forget(dir);
            store
        }

        #[test]
        fn append_and_all_events() {
            let mut store = temp_store();
            let e1 = make_event("e1", StateHash::ZERO);
            let e2 = make_event("e2", StateHash::from_bytes(b"state-a"));

            store.append(e1);
            store.append(e2);

            let all = store.all_events();
            assert_eq!(all.len(), 2);
            assert_eq!(all[0].id, "e1");
            assert_eq!(all[1].id, "e2");
        }

        #[test]
        fn append_deduplicates() {
            let mut store = temp_store();
            let event = make_event("e1", StateHash::ZERO);
            store.append(event.clone());
            store.append(event);

            assert_eq!(store.all_events().len(), 1);
        }

        #[test]
        fn contains_check() {
            let mut store = temp_store();
            assert!(!store.contains("e1"));

            store.append(make_event("e1", StateHash::ZERO));
            assert!(store.contains("e1"));
            assert!(!store.contains("e2"));
        }

        #[test]
        fn latest_hash_default_is_zero() {
            let store = temp_store();
            assert_eq!(store.latest_hash(), StateHash::ZERO);
        }

        #[test]
        fn set_and_get_latest_hash() {
            let mut store = temp_store();
            let hash = StateHash::from_bytes(b"new-state");
            store.set_latest_hash(hash.clone());
            assert_eq!(store.latest_hash(), hash);
        }

        #[test]
        fn events_since_returns_from_matching_parent() {
            let mut store = temp_store();
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
        fn events_since_missing_hash_returns_empty() {
            let mut store = temp_store();
            store.append(make_event("e1", StateHash::ZERO));

            let since = store.events_since(&StateHash::from_bytes(b"unknown"));
            assert!(since.is_empty());
        }

        #[test]
        fn persistent_event_store_enum_delegates() {
            let inner = temp_store();
            let mut store = crate::state::PersistentEventStore::Sqlite(inner);

            store.append(make_event("e1", StateHash::ZERO));
            assert_eq!(store.all_events().len(), 1);
            assert!(store.contains("e1"));

            let hash = StateHash::from_bytes(b"test");
            store.set_latest_hash(hash.clone());
            assert_eq!(store.latest_hash(), hash);
        }
    }

    #[test]
    fn persistent_event_store_in_memory_default() {
        let mut store = crate::state::PersistentEventStore::default();
        let event = make_event("e1", StateHash::ZERO);

        store.append(event);
        assert_eq!(store.all_events().len(), 1);
        assert!(store.contains("e1"));
        assert_eq!(store.latest_hash(), StateHash::ZERO);

        let hash = StateHash::from_bytes(b"test-hash");
        store.set_latest_hash(hash.clone());
        assert_eq!(store.latest_hash(), hash);
    }
}
