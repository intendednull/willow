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

/// WASM stub — messages are in-memory only (IndexedDB is a future enhancement).
#[cfg(target_arch = "wasm32")]
pub struct MessageDb;

#[cfg(target_arch = "wasm32")]
impl MessageDb {
    pub fn insert(&self, _msg: &StoredMessage) {}
    pub fn load_topic(&self, _topic: &str, _limit: usize) -> Vec<StoredMessage> {
        Vec::new()
    }
    pub fn topics(&self) -> Vec<String> {
        Vec::new()
    }
    pub fn count_topic(&self, _topic: &str) -> usize {
        0
    }
}

#[cfg(target_arch = "wasm32")]
pub fn open_message_db() -> Option<MessageDb> {
    Some(MessageDb)
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
        let encoded = base64_encode(data);
        let _ = storage.set_item(&format!("willow_{key}"), &encoded);
    }
}

#[cfg(target_arch = "wasm32")]
fn load_raw(key: &str) -> Option<Vec<u8>> {
    let storage = local_storage()?;
    let encoded = storage.get_item(&format!("willow_{key}")).ok()??;
    base64_decode(&encoded)
}

// ───── Base64 (WASM only) ───────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[cfg(target_arch = "wasm32")]
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn char_val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes = input.as_bytes();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut result = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let a = char_val(chunk[0])?;
        let b = char_val(chunk[1])?;
        let n = (a << 18) | (b << 12);
        result.push((n >> 16) as u8);
        if chunk[2] != b'=' {
            let c = char_val(chunk[2])?;
            let n = n | (c << 6);
            result.push((n >> 8) as u8);
            if chunk[3] != b'=' {
                let d = char_val(chunk[3])?;
                let n = n | d;
                result.push(n as u8);
            }
        }
    }
    Some(result)
}
