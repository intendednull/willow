//! Palette recents — local storage, max 8 entries, toggleable.
//!
//! Read / write through the browser's `localStorage`. The toggle key is
//! consumed by a stub in settings-tweaks.md; until then, the default is
//! `true` (remember recents).

use serde::{Deserialize, Serialize};

pub const MAX_RECENTS: usize = 8;
const KEY: &str = "willow.palette.recents";
const TOGGLE_KEY: &str = "willow.palette.remember-recents";

/// One recent entry. `kind` is a stable string key; `id` is the thing
/// to activate; `label` is the display string at the time of capture.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Recent {
    pub kind: String,
    pub id: String,
    pub label: String,
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Whether recents are persisted. Defaults to `true`.
pub fn remember_enabled() -> bool {
    storage()
        .and_then(|s| s.get_item(TOGGLE_KEY).ok().flatten())
        .map(|v| v != "false")
        .unwrap_or(true)
}

/// Load all recents (up to `MAX_RECENTS`). Returns empty when the
/// remember-recents toggle is off or when localStorage is unavailable.
pub fn load() -> Vec<Recent> {
    if !remember_enabled() {
        return Vec::new();
    }
    storage()
        .and_then(|s| s.get_item(KEY).ok().flatten())
        .and_then(|j| serde_json::from_str::<Vec<Recent>>(&j).ok())
        .unwrap_or_default()
}

/// Push an entry to the front. Dedupes by `(kind, id)` and truncates to
/// `MAX_RECENTS`. No-op when the remember-recents toggle is off.
pub fn push(entry: Recent) {
    if !remember_enabled() {
        return;
    }
    let mut list = load();
    list.retain(|e| !(e.kind == entry.kind && e.id == entry.id));
    list.insert(0, entry);
    list.truncate(MAX_RECENTS);
    if let Some(s) = storage() {
        if let Ok(j) = serde_json::to_string(&list) {
            let _ = s.set_item(KEY, &j);
        }
    }
}

/// Clear all recents.
pub fn clear() {
    if let Some(s) = storage() {
        let _ = s.remove_item(KEY);
    }
}
