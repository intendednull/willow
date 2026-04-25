//! localStorage-backed [`NicknameStore`].
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
//! §Private nickname. Key format: `willow.profile.nickname.<peer_id>`.
//!
//! Native builds fall back to an in-memory HashMap so the web crate
//! still tests without a browser.

use std::collections::HashMap;
use std::sync::RwLock;

use willow_client::{NicknameStore, NICKNAME_CAP};

const KEY_PREFIX: &str = "willow.profile.nickname.";

/// localStorage-backed nickname store.
///
/// On native (tests, CI) the backing store is an in-memory `HashMap`
/// — there is no browser localStorage. On wasm32 every write is
/// mirrored to `localStorage` so a page reload rehydrates it.
#[derive(Default)]
pub struct WebNicknameStore {
    cache: RwLock<HashMap<String, String>>,
    version: RwLock<u64>,
}

impl WebNicknameStore {
    /// Boot + hydrate from the `willow.profile.nickname.*` keys.
    pub fn load() -> Self {
        let store = Self::default();
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(win) = web_sys::window() {
                if let Ok(Some(ls)) = win.local_storage() {
                    let len = ls.length().unwrap_or(0);
                    let mut cache = HashMap::new();
                    for i in 0..len {
                        let Ok(Some(k)) = ls.key(i) else { continue };
                        if let Some(pid) = k.strip_prefix(KEY_PREFIX) {
                            if let Ok(Some(v)) = ls.get_item(&k) {
                                cache.insert(pid.to_string(), v);
                            }
                        }
                    }
                    *store.cache.write().unwrap() = cache;
                }
            }
        }
        store
    }
}

impl NicknameStore for WebNicknameStore {
    fn get(&self, peer_id: &str) -> Option<String> {
        self.cache.read().ok()?.get(peer_id).cloned()
    }

    fn set(&self, peer_id: &str, value: &str) {
        let trimmed: String = value.chars().take(NICKNAME_CAP).collect();
        if trimmed.is_empty() {
            self.clear(peer_id);
            return;
        }
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(peer_id.to_string(), trimmed.clone());
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(win) = web_sys::window() {
                if let Ok(Some(ls)) = win.local_storage() {
                    ls.set_item(&format!("{KEY_PREFIX}{peer_id}"), &trimmed)
                        .ok();
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = &trimmed; // silence unused warning on native
        }
        if let Ok(mut v) = self.version.write() {
            *v += 1;
        }
    }

    fn clear(&self, peer_id: &str) {
        let mut did_remove = false;
        if let Ok(mut cache) = self.cache.write() {
            did_remove = cache.remove(peer_id).is_some();
        }
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(win) = web_sys::window() {
                if let Ok(Some(ls)) = win.local_storage() {
                    ls.remove_item(&format!("{KEY_PREFIX}{peer_id}")).ok();
                }
            }
        }
        if did_remove {
            if let Ok(mut v) = self.version.write() {
                *v += 1;
            }
        }
    }

    fn version(&self) -> u64 {
        self.version.read().map(|g| *g).unwrap_or(0)
    }

    fn snapshot(&self) -> Vec<(String, String)> {
        self.cache
            .read()
            .map(|g| g.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_store_set_and_get_round_trip() {
        let s = WebNicknameStore::default();
        s.set("alice", "mira");
        assert_eq!(s.get("alice").as_deref(), Some("mira"));
    }

    #[test]
    fn web_store_empty_clears() {
        let s = WebNicknameStore::default();
        s.set("alice", "mira");
        s.set("alice", "");
        assert_eq!(s.get("alice"), None);
    }

    #[test]
    fn web_store_version_bumps() {
        let s = WebNicknameStore::default();
        let v0 = s.version();
        s.set("alice", "mira");
        assert!(s.version() > v0);
    }
}
