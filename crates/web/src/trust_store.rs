//! `WebTrustStore` — the Leptos-side implementation of
//! [`TrustStore`](willow_client::trust::TrustStore).
//!
//! On wasm32 we persist in `localStorage` under a single key so the set
//! of peers fits in one read/write. On native (tests, CI) we degrade to
//! an in-memory mutex so the web crate still builds and tests without
//! a browser.
//!
//! Trust is a per-device belief (see `trust.rs` module docs) so this
//! store is never synced to the server DAG. Deleting `localStorage`
//! wipes verification — the UI treats every peer as
//! [`PeerTrust::Unknown`] after a clean browser.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use willow_client::trust::{PeerTrust, TrustStore, TrustStoreHandle};

/// Key under which the peer-trust map lives in `localStorage`. Versioned
/// so a schema change can safely invalidate older snapshots.
const STORAGE_KEY: &str = "willow.trust.v1";

/// Serialized form of the entire trust table.
///
/// We serialize a flat map of `{ peer_id -> PeerTrust }` as JSON so the
/// payload is human-readable when debugging and round-trips cleanly
/// through both `localStorage` (string-only) and bincode-aware tests.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct TrustSnapshot {
    /// Schema version; future migrations bump this.
    #[serde(default = "default_version")]
    version: u32,
    /// Each recorded peer's current belief.
    #[serde(default)]
    peers: HashMap<String, PeerTrust>,
}

fn default_version() -> u32 {
    1
}

/// Peer-trust store backed by the browser `localStorage` API (wasm32)
/// or an in-memory mutex (native).
pub struct WebTrustStore {
    inner: Mutex<Inner>,
}

struct Inner {
    peers: HashMap<String, PeerTrust>,
    version: u64,
}

impl WebTrustStore {
    /// Load the store from the current environment. On wasm this reads
    /// `localStorage`; on native it starts empty. A corrupted or absent
    /// snapshot produces an empty store (logged but non-fatal).
    pub fn load() -> Self {
        let peers = load_snapshot().unwrap_or_default().peers;
        Self {
            inner: Mutex::new(Inner { peers, version: 0 }),
        }
    }

    /// Convenience: build and wrap as the `Arc<dyn TrustStore>` handle
    /// the client expects.
    pub fn load_handle() -> TrustStoreHandle {
        Arc::new(Self::load())
    }

    /// Persist the in-memory map back to `localStorage`. Called after
    /// every `set`. No-op on native.
    fn persist(&self, peers: &HashMap<String, PeerTrust>) {
        let snapshot = TrustSnapshot {
            version: 1,
            peers: peers.clone(),
        };
        persist_snapshot(&snapshot);
    }
}

impl TrustStore for WebTrustStore {
    fn get(&self, peer_id: &str) -> PeerTrust {
        self.inner
            .lock()
            .expect("WebTrustStore mutex poisoned")
            .peers
            .get(peer_id)
            .cloned()
            .unwrap_or_default()
    }

    fn set(&self, peer_id: &str, trust: PeerTrust) {
        let mut guard = self.inner.lock().expect("WebTrustStore mutex poisoned");
        if matches!(trust, PeerTrust::Unknown) {
            guard.peers.remove(peer_id);
        } else {
            guard.peers.insert(peer_id.to_string(), trust);
        }
        guard.version = guard.version.wrapping_add(1);
        // Clone so the lock is released before we dip into `localStorage`.
        let snapshot = guard.peers.clone();
        drop(guard);
        self.persist(&snapshot);
    }

    fn snapshot(&self) -> Vec<(String, PeerTrust)> {
        self.inner
            .lock()
            .expect("WebTrustStore mutex poisoned")
            .peers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn version(&self) -> u64 {
        self.inner
            .lock()
            .expect("WebTrustStore mutex poisoned")
            .version
    }
}

// ── Platform: localStorage backend on wasm32 ──────────────────────────────

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

#[cfg(target_arch = "wasm32")]
fn load_snapshot() -> Option<TrustSnapshot> {
    let storage = local_storage()?;
    let raw = storage.get_item(STORAGE_KEY).ok().flatten()?;
    match serde_json::from_str::<TrustSnapshot>(&raw) {
        Ok(snap) => Some(snap),
        Err(err) => {
            tracing::warn!(%err, "willow.trust.v1 snapshot is corrupt — resetting");
            None
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn persist_snapshot(snapshot: &TrustSnapshot) {
    let Some(storage) = local_storage() else {
        return;
    };
    match serde_json::to_string(snapshot) {
        Ok(payload) => {
            if let Err(err) = storage.set_item(STORAGE_KEY, &payload) {
                tracing::warn!(?err, "failed to persist willow.trust.v1");
            }
        }
        Err(err) => tracing::warn!(%err, "failed to serialize willow.trust.v1"),
    }
}

// ── Platform: native fallback (used in cargo test for the web crate) ──────

#[cfg(not(target_arch = "wasm32"))]
fn load_snapshot() -> Option<TrustSnapshot> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn persist_snapshot(_snapshot: &TrustSnapshot) {
    // no-op on native
}

#[cfg(test)]
mod tests {
    // Native-only tests — wasm coverage lives in crates/web/tests/browser.rs.
    #[cfg(not(target_arch = "wasm32"))]
    mod native {
        use super::super::*;
        use willow_client::trust::{PeerTrust, UnverifiedReason};

        #[test]
        fn new_store_is_empty_and_unknown() {
            let store = WebTrustStore::load();
            assert_eq!(store.snapshot().len(), 0);
            assert_eq!(store.version(), 0);
            assert_eq!(store.get("anyone"), PeerTrust::Unknown);
        }

        #[test]
        fn set_round_trip_bumps_version() {
            let store = WebTrustStore::load();
            store.set(
                "alice",
                PeerTrust::Verified {
                    at_ms: 42,
                    pinned_key: [0xAAu8; 32],
                },
            );
            assert!(store.get("alice").is_verified());
            assert_eq!(store.version(), 1);

            store.set(
                "bob",
                PeerTrust::Unverified {
                    reason: UnverifiedReason::SasMismatch,
                },
            );
            assert_eq!(store.snapshot().len(), 2);
            assert_eq!(store.version(), 2);
        }

        #[test]
        fn setting_unknown_removes_entry() {
            let store = WebTrustStore::load();
            store.set("alice", PeerTrust::PendingVerify);
            assert_eq!(store.snapshot().len(), 1);

            store.set("alice", PeerTrust::Unknown);
            assert_eq!(store.snapshot().len(), 0);
            // version still bumps for every write
            assert_eq!(store.version(), 2);
        }
    }
}
