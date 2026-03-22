//! # Storage
//!
//! Platform-specific persistence for identity, server state, and channel keys.
//!
//! - **Native**: files in `~/.local/share/willow/`
//! - **WASM**: browser `localStorage`

use std::collections::HashMap;

use willow_channel::Server;
use willow_crypto::ChannelKey;

/// Persisted channel key data: maps topic → key bytes.
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct SavedKeys(pub Vec<(String, [u8; 32])>);

/// Save server state and channel keys.
pub fn save_server(server: &Server, keys: &HashMap<String, ChannelKey>) {
    save_impl(server, keys);
}

/// Load server state and channel keys. Returns None if not found.
pub fn load_server() -> Option<(Server, HashMap<String, ChannelKey>)> {
    load_impl()
}

/// Save identity key bytes.
pub fn save_identity_bytes(bytes: &[u8]) {
    save_identity_impl(bytes);
}

/// Load identity key bytes. Returns None if not found.
pub fn load_identity_bytes() -> Option<Vec<u8>> {
    load_identity_impl()
}

// ───── Native implementation ────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn data_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("willow")
}

#[cfg(not(target_arch = "wasm32"))]
fn save_impl(server: &Server, keys: &HashMap<String, ChannelKey>) {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);

    if let Ok(bytes) = willow_transport::pack(server) {
        let _ = std::fs::write(dir.join("server.bin"), bytes);
    }

    let saved = SavedKeys(
        keys.iter()
            .map(|(topic, key)| (topic.clone(), *key.as_bytes()))
            .collect(),
    );
    if let Ok(bytes) = willow_transport::pack(&saved) {
        let _ = std::fs::write(dir.join("keys.bin"), bytes);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_impl() -> Option<(Server, HashMap<String, ChannelKey>)> {
    let dir = data_dir();

    let server_bytes = std::fs::read(dir.join("server.bin")).ok()?;
    let server: Server = willow_transport::unpack(&server_bytes).ok()?;

    let key_bytes = std::fs::read(dir.join("keys.bin")).ok()?;
    let saved: SavedKeys = willow_transport::unpack(&key_bytes).ok()?;

    let keys = saved
        .0
        .into_iter()
        .map(|(topic, bytes)| (topic, ChannelKey::from_bytes(bytes)))
        .collect();

    Some((server, keys))
}

#[cfg(not(target_arch = "wasm32"))]
fn save_identity_impl(bytes: &[u8]) {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("identity.key"), bytes);
}

#[cfg(not(target_arch = "wasm32"))]
fn load_identity_impl() -> Option<Vec<u8>> {
    let dir = data_dir();
    std::fs::read(dir.join("identity.key")).ok()
}

// ───── WASM implementation (localStorage) ───────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

#[cfg(target_arch = "wasm32")]
fn save_to_storage(key: &str, data: &[u8]) {
    if let Some(storage) = local_storage() {
        // Base64-encode binary data for localStorage (which only stores strings).
        let encoded = base64_encode(data);
        let _ = storage.set_item(key, &encoded);
    }
}

#[cfg(target_arch = "wasm32")]
fn load_from_storage(key: &str) -> Option<Vec<u8>> {
    let storage = local_storage()?;
    let encoded = storage.get_item(key).ok()??;
    base64_decode(&encoded)
}

#[cfg(target_arch = "wasm32")]
fn save_impl(server: &Server, keys: &HashMap<String, ChannelKey>) {
    if let Ok(bytes) = willow_transport::pack(server) {
        save_to_storage("willow_server", &bytes);
    }

    let saved = SavedKeys(
        keys.iter()
            .map(|(topic, key)| (topic.clone(), *key.as_bytes()))
            .collect(),
    );
    if let Ok(bytes) = willow_transport::pack(&saved) {
        save_to_storage("willow_keys", &bytes);
    }
}

#[cfg(target_arch = "wasm32")]
fn load_impl() -> Option<(Server, HashMap<String, ChannelKey>)> {
    let server_bytes = load_from_storage("willow_server")?;
    let server: Server = willow_transport::unpack(&server_bytes).ok()?;

    let key_bytes = load_from_storage("willow_keys")?;
    let saved: SavedKeys = willow_transport::unpack(&key_bytes).ok()?;

    let keys = saved
        .0
        .into_iter()
        .map(|(topic, bytes)| (topic, ChannelKey::from_bytes(bytes)))
        .collect();

    Some((server, keys))
}

#[cfg(target_arch = "wasm32")]
fn save_identity_impl(bytes: &[u8]) {
    save_to_storage("willow_identity", bytes);
}

#[cfg(target_arch = "wasm32")]
fn load_identity_impl() -> Option<Vec<u8>> {
    load_from_storage("willow_identity")
}

// ───── Base64 (WASM only, avoids pulling in a crate) ────────────────────────

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
