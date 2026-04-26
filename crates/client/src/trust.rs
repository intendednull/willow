//! Local trust store — per-device belief about peer verification state.
//!
//! See `docs/specs/2026-04-19-ui-design/trust-verification.md` §Data
//! dependencies and the implementation plan Task 2.
//!
//! ## Scope
//!
//! Trust here is a **local belief** on a single device — never a
//! willow-state event, never gossiped. A user who compares fingerprints
//! with Alice on their laptop does not automatically verify Alice on
//! their phone. This keeps the trust surface tightly bounded: a
//! compromised device cannot silently un-verify a peer on another
//! device.
//!
//! A future `SharedTrust` EventKind (out of scope for v1) may layer on
//! top without changing this trait.
//!
//! ## Platforms
//!
//! The UI consumes a `Arc<dyn TrustStore>`. The `WebTrustStore`
//! implementation (native + wasm `localStorage`) lives in
//! `crates/web/src/trust_store.rs`. Tests and headless consumers can
//! plug in any other backend.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Per-device trust belief for a single peer.
///
/// ## Transitions
///
/// ```text
/// Unknown                (never seen)
///   ↓ first contact
/// PendingVerify          (first message but no SAS attempt yet)
///   ↓ they match        ↓ they don't match / no attempt over time
/// Verified               Unverified
///   ↓ key rotation
/// DowngradedFromVerified
///   ↓ re-verify
/// Verified
/// ```
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PeerTrust {
    /// Never met, never verified. Renders as `new peer` in the UI.
    #[default]
    Unknown,
    /// First contact, SAS not yet attempted.
    PendingVerify,
    /// SAS mismatch or user has explicitly marked the peer unverified.
    Unverified {
        /// Why the peer is unverified — drives copy on the downgrade
        /// banner and affects recovery paths (e.g. a `SasMismatch`
        /// requires an active re-compare, whereas `NeverCompared` just
        /// needs the user to open the dialog once).
        reason: UnverifiedReason,
    },
    /// SAS succeeded — key pinned at `pinned_key`.
    Verified {
        /// When the verification happened, epoch-millis UTC.
        at_ms: i64,
        /// Pinned Ed25519 public key bytes. Any future message whose
        /// author key differs triggers a transition to
        /// [`PeerTrust::DowngradedFromVerified`].
        pinned_key: [u8; 32],
    },
    /// Previously verified, but the peer's public key has rotated (or
    /// a later SAS mismatched). The unverified badge shows everywhere
    /// and the downgrade banner renders on the peer's letter until
    /// the user re-compares.
    DowngradedFromVerified {
        /// Previously pinned key.
        previous_key: [u8; 32],
        /// New key the peer is currently signing with.
        new_key: [u8; 32],
        /// When the rotation was detected, epoch-millis UTC.
        at_ms: i64,
    },
}

impl PeerTrust {
    /// Convenience: is this peer fully verified on this device?
    pub fn is_verified(&self) -> bool {
        matches!(self, PeerTrust::Verified { .. })
    }

    /// Convenience: should the unverified badge (amber) render?
    pub fn is_unverified(&self) -> bool {
        matches!(
            self,
            PeerTrust::Unverified { .. } | PeerTrust::DowngradedFromVerified { .. }
        )
    }
}

/// Why a peer is unverified. Drives copy choice on the downgrade banner
/// and onboarding hints.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum UnverifiedReason {
    /// The user has never run the compare-fingerprints dialog.
    NeverCompared,
    /// The last SAS attempt did not match.
    SasMismatch,
    /// The peer's public key rotated away from a previously pinned one.
    KeyRotation,
}

/// A snapshot preview for the `add a friend` dialog: the two sides'
/// six-word fingerprints already derived so the UI can render the grids
/// without re-hashing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComparePreview {
    /// The local user's side — `you`.
    pub you: [String; 6],
    /// The remote peer's side — `them`.
    pub them: [String; 6],
}

/// Local trust-store interface. `Send + Sync` so the UI can share it
/// across Leptos effects and `wasm_bindgen_futures::spawn_local`.
///
/// **State management:** the trait API is sync, which forces lock-based
/// impls. Trait elimination in favour of a `TrustActor` is tracked
/// in `docs/specs/2026-04-26-state-management-model-design.md`
/// § Follow-up work F1.
pub trait TrustStore: Send + Sync {
    /// Read the trust belief for a peer. Returns [`PeerTrust::Unknown`]
    /// for any peer the store has not seen.
    fn get(&self, peer_id: &str) -> PeerTrust;

    /// Overwrite the trust belief for a peer. Must be idempotent and
    /// durable (persistent within the store's platform — `localStorage`
    /// on wasm, in-memory on native tests).
    fn set(&self, peer_id: &str, trust: PeerTrust);

    /// Full snapshot of every recorded peer and its state. Callers use
    /// this to seed UI signals at boot. The order is implementation-
    /// defined.
    fn snapshot(&self) -> Vec<(String, PeerTrust)>;

    /// A monotonically-increasing version token that bumps on every
    /// `set` call. UIs subscribe to this to rebuild derived signals.
    fn version(&self) -> u64;
}

/// A convenience alias so we don't retype `Arc<dyn TrustStore>` everywhere.
pub type TrustStoreHandle = Arc<dyn TrustStore>;

/// Simple in-memory `TrustStore` used by tests and non-web consumers.
/// Persists nothing. Safe to clone.
#[derive(Debug, Default)]
pub struct InMemoryTrustStore {
    // state: lock-ok — `TrustStore` trait is sync; trait elimination
    // tracked in docs/specs/2026-04-26-state-management-model-design.md
    // § Follow-up work F1. `InMemoryState` already groups peers + version
    // under one guard, so cross-field atomicity is intact.
    inner: std::sync::Mutex<InMemoryState>,
}

#[derive(Debug, Default)]
struct InMemoryState {
    peers: std::collections::HashMap<String, PeerTrust>,
    version: u64,
}

impl InMemoryTrustStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl TrustStore for InMemoryTrustStore {
    fn get(&self, peer_id: &str) -> PeerTrust {
        self.inner
            .lock()
            .expect("InMemoryTrustStore mutex poisoned")
            .peers
            .get(peer_id)
            .cloned()
            .unwrap_or_default()
    }

    fn set(&self, peer_id: &str, trust: PeerTrust) {
        let mut guard = self
            .inner
            .lock()
            .expect("InMemoryTrustStore mutex poisoned");
        guard.peers.insert(peer_id.to_string(), trust);
        guard.version = guard.version.wrapping_add(1);
    }

    fn snapshot(&self) -> Vec<(String, PeerTrust)> {
        self.inner
            .lock()
            .expect("InMemoryTrustStore mutex poisoned")
            .peers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn version(&self) -> u64 {
        self.inner
            .lock()
            .expect("InMemoryTrustStore mutex poisoned")
            .version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_unknown() {
        assert_eq!(PeerTrust::default(), PeerTrust::Unknown);
    }

    #[test]
    fn is_verified_and_unverified_matchers() {
        assert!(PeerTrust::Verified {
            at_ms: 0,
            pinned_key: [0u8; 32]
        }
        .is_verified());
        assert!(PeerTrust::Unverified {
            reason: UnverifiedReason::SasMismatch
        }
        .is_unverified());
        assert!(PeerTrust::DowngradedFromVerified {
            previous_key: [1u8; 32],
            new_key: [2u8; 32],
            at_ms: 0,
        }
        .is_unverified());
        assert!(!PeerTrust::Unknown.is_verified());
        assert!(!PeerTrust::PendingVerify.is_verified());
    }

    #[test]
    fn in_memory_round_trip() {
        let store = InMemoryTrustStore::new();
        assert_eq!(store.get("alice"), PeerTrust::Unknown);
        assert_eq!(store.version(), 0);

        store.set(
            "alice",
            PeerTrust::Verified {
                at_ms: 42,
                pinned_key: [9u8; 32],
            },
        );
        assert!(store.get("alice").is_verified());
        assert_eq!(store.version(), 1);

        store.set(
            "bob",
            PeerTrust::Unverified {
                reason: UnverifiedReason::NeverCompared,
            },
        );
        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(store.version(), 2);
    }

    #[test]
    fn serde_round_trip_covers_every_variant() {
        let variants = vec![
            PeerTrust::Unknown,
            PeerTrust::PendingVerify,
            PeerTrust::Unverified {
                reason: UnverifiedReason::SasMismatch,
            },
            PeerTrust::Unverified {
                reason: UnverifiedReason::NeverCompared,
            },
            PeerTrust::Unverified {
                reason: UnverifiedReason::KeyRotation,
            },
            PeerTrust::Verified {
                at_ms: 12345,
                pinned_key: [0xAB; 32],
            },
            PeerTrust::DowngradedFromVerified {
                previous_key: [0x11; 32],
                new_key: [0x22; 32],
                at_ms: 777,
            },
        ];
        for v in variants {
            let bytes = willow_transport::pack(&v).expect("serialize");
            let back: PeerTrust = willow_transport::unpack(&bytes).expect("deserialize");
            assert_eq!(v, back);
        }
    }
}
