//! localStorage-backed [`NicknameStore`].
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
//! §Private nickname. Key format: `willow.profile.nickname.<peer_id>`.
//!
//! Native builds fall back to an in-memory HashMap so the web crate
//! still tests without a browser.

use std::collections::HashMap;
use std::sync::Mutex;

use willow_client::{NicknameStore, NICKNAME_CAP};

const KEY_PREFIX: &str = "willow.profile.nickname.";

/// Inner state under the [`WebNicknameStore`] guard. Held as a single
/// unit so cache writes and version bumps stay atomic.
#[derive(Default)]
struct WebNicknameInner {
    cache: HashMap<String, String>,
    version: u64,
}

/// localStorage-backed nickname store.
///
/// On native (tests, CI) the backing store is an in-memory `HashMap`
/// — there is no browser localStorage. On wasm32 every write is
/// mirrored to `localStorage` so a page reload rehydrates it.
#[derive(Default)]
pub struct WebNicknameStore {
    // state: lock-ok — `NicknameStore` trait is sync; trait elimination
    // tracked in docs/specs/2026-04-26-state-management-model-design.md
    // § Follow-up work F1. Single guard groups cache + version for
    // cross-field atomicity.
    inner: Mutex<WebNicknameInner>,
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
                    if let Ok(mut guard) = store.inner.lock() {
                        guard.cache = cache;
                    }
                }
            }
        }
        store
    }
}

impl NicknameStore for WebNicknameStore {
    fn get(&self, peer_id: &str) -> Option<String> {
        self.inner.lock().ok()?.cache.get(peer_id).cloned()
    }

    fn set(&self, peer_id: &str, value: &str) {
        let trimmed: String = value.chars().take(NICKNAME_CAP).collect();
        if trimmed.is_empty() {
            self.clear(peer_id);
            return;
        }
        // Persist to localStorage *inside* the guard so a subscriber
        // observing `version()` cannot see the bump before the write
        // is durable. `localStorage.set_item` is sync — no await — so
        // there's no risk of holding across async boundaries.
        if let Ok(mut guard) = self.inner.lock() {
            #[cfg(target_arch = "wasm32")]
            {
                if let Some(win) = web_sys::window() {
                    if let Ok(Some(ls)) = win.local_storage() {
                        ls.set_item(&format!("{KEY_PREFIX}{peer_id}"), &trimmed)
                            .ok();
                    }
                }
            }
            guard.cache.insert(peer_id.to_string(), trimmed);
            guard.version += 1;
        }
    }

    fn clear(&self, peer_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            if guard.cache.remove(peer_id).is_some() {
                #[cfg(target_arch = "wasm32")]
                {
                    if let Some(win) = web_sys::window() {
                        if let Ok(Some(ls)) = win.local_storage() {
                            ls.remove_item(&format!("{KEY_PREFIX}{peer_id}")).ok();
                        }
                    }
                }
                guard.version += 1;
            }
        }
    }

    fn version(&self) -> u64 {
        self.inner.lock().map(|g| g.version).unwrap_or(0)
    }

    fn snapshot(&self) -> Vec<(String, String)> {
        self.inner
            .lock()
            .map(|g| {
                g.cache
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
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

    #[test]
    fn web_store_clear_missing_does_not_bump_version() {
        let s = WebNicknameStore::default();
        let v0 = s.version();
        s.clear("never_set");
        assert_eq!(s.version(), v0);
    }
}
