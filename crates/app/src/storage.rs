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

// ───── Public types ─────────────────────────────────────────────────────────

/// Persisted channel key data: maps topic → key bytes.
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

// ───── Public API ───────────────────────────────────────────────────────────

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

// ───── Message Persistence ──────────────────────────────────────────────────

/// A stored chat message for display. Lightweight compared to the full
/// `willow_messaging::Message` — just what the UI needs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredMessage {
    pub topic: String,
    pub author: String,
    pub body: String,
    pub is_local: bool,
    pub timestamp_ms: u64,
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
                timestamp_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_topic ON messages(topic, timestamp_ms);",
        )
        .ok()?;
        Some(Self { conn })
    }

    /// Insert a message.
    pub fn insert(&self, msg: &StoredMessage) {
        let _ = self.conn.execute(
            "INSERT INTO messages (topic, author, body, is_local, timestamp_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![msg.topic, msg.author, msg.body, msg.is_local as i32, msg.timestamp_ms],
        );
    }

    /// Load messages for a topic, ordered by timestamp.
    pub fn load_topic(&self, topic: &str, limit: usize) -> Vec<StoredMessage> {
        let mut stmt = match self.conn.prepare(
            "SELECT topic, author, body, is_local, timestamp_ms FROM messages WHERE topic = ?1 ORDER BY timestamp_ms ASC LIMIT ?2",
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
        // Simple hash to keep keys short. Not cryptographic — just for storage.
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
        // Return empty — topic list comes from ServerState instead.
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

// ───── Native implementation ────────────────────────────────────────────────

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

// ───── WASM implementation (localStorage) ───────────────────────────────────

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
