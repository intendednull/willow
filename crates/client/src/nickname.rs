//! Local-only peer nicknames.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
//! §Private nickname. Nicknames never propagate — they live alongside
//! the trust store in browser localStorage. This crate owns the trait;
//! the web crate ships the `WebNicknameStore` impl.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Cap on nickname length in UTF-8 characters. Spec §Private nickname.
pub const NICKNAME_CAP: usize = 32;

/// Trait for an opaque, local-only per-peer nickname store.
///
/// Implementations MUST persist writes durably within the lifetime of
/// the session (e.g. localStorage on web, on-disk file natively). The
/// `version` counter increments on every successful mutation so
/// reactive UIs can bump a signal.
pub trait NicknameStore: Send + Sync {
    /// Return the stored nickname for `peer_id`, or `None`.
    fn get(&self, peer_id: &str) -> Option<String>;
    /// Persist `value` (truncated to [`NICKNAME_CAP`]). Pass empty to clear.
    fn set(&self, peer_id: &str, value: &str);
    /// Remove the entry for `peer_id`. Equivalent to `set(peer_id, "")`.
    fn clear(&self, peer_id: &str);
    /// Current version counter — bumps on every mutation.
    fn version(&self) -> u64;
    /// Full snapshot as `(peer_id, nickname)` pairs.
    fn snapshot(&self) -> Vec<(String, String)>;
}

/// Handle type matching the `TrustStoreHandle` shape.
pub type NicknameStoreHandle = Arc<dyn NicknameStore>;

/// In-memory implementation for tests + native builds.
#[derive(Default)]
pub struct MemNicknameStore {
    inner: RwLock<HashMap<String, String>>,
    version: RwLock<u64>,
}

impl NicknameStore for MemNicknameStore {
    fn get(&self, peer_id: &str) -> Option<String> {
        self.inner.read().ok()?.get(peer_id).cloned()
    }

    fn set(&self, peer_id: &str, value: &str) {
        let trimmed: String = value.chars().take(NICKNAME_CAP).collect();
        if trimmed.is_empty() {
            self.clear(peer_id);
            return;
        }
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(peer_id.to_string(), trimmed);
        }
        if let Ok(mut v) = self.version.write() {
            *v += 1;
        }
    }

    fn clear(&self, peer_id: &str) {
        let mut did_remove = false;
        if let Ok(mut guard) = self.inner.write() {
            did_remove = guard.remove(peer_id).is_some();
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
        self.inner
            .read()
            .map(|g| g.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_store_set_and_get_round_trip() {
        let s = MemNicknameStore::default();
        s.set("alice", "mira");
        assert_eq!(s.get("alice").as_deref(), Some("mira"));
    }

    #[test]
    fn mem_store_get_missing_is_none() {
        let s = MemNicknameStore::default();
        assert!(s.get("ghost").is_none());
    }

    #[test]
    fn mem_store_clear_removes_entry() {
        let s = MemNicknameStore::default();
        s.set("alice", "mira");
        s.clear("alice");
        assert_eq!(s.get("alice"), None);
    }

    #[test]
    fn mem_store_empty_value_clears() {
        let s = MemNicknameStore::default();
        s.set("alice", "mira");
        s.set("alice", "");
        assert!(s.get("alice").is_none());
    }

    #[test]
    fn mem_store_version_bumps_on_mutation() {
        let s = MemNicknameStore::default();
        let v0 = s.version();
        s.set("alice", "mira");
        let v1 = s.version();
        s.clear("alice");
        let v2 = s.version();
        assert!(v1 > v0);
        assert!(v2 > v1);
    }

    #[test]
    fn mem_store_caps_at_nickname_cap_chars() {
        let s = MemNicknameStore::default();
        // 100 x 'a' — should truncate to NICKNAME_CAP chars on set.
        let long = "a".repeat(100);
        s.set("alice", &long);
        assert_eq!(s.get("alice").unwrap().chars().count(), NICKNAME_CAP);
    }

    #[test]
    fn mem_store_snapshot_returns_all_entries() {
        let s = MemNicknameStore::default();
        s.set("alice", "mira");
        s.set("bob", "rob");
        let mut snap = s.snapshot();
        snap.sort();
        assert_eq!(
            snap,
            vec![
                ("alice".to_string(), "mira".to_string()),
                ("bob".to_string(), "rob".to_string()),
            ]
        );
    }
}
