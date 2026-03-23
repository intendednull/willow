//! State hashing for divergence detection.
//!
//! [`StateHash`] is a 32-byte SHA-256 digest computed from the canonical
//! serialization of a [`ServerState`](crate::server::ServerState). Two peers
//! with the same event history will always produce the same hash, enabling
//! efficient divergence detection.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A 32-byte SHA-256 hash of the server state.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StateHash(pub [u8; 32]);

impl Default for StateHash {
    fn default() -> Self {
        Self::ZERO
    }
}

impl StateHash {
    /// The zero hash, used as the parent hash for the genesis event.
    pub const ZERO: StateHash = StateHash([0u8; 32]);

    /// Compute a SHA-256 hash from raw bytes.
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        StateHash(hash)
    }
}

impl fmt::Display for StateHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_hash_is_all_zeros() {
        assert_eq!(StateHash::ZERO.0, [0u8; 32]);
    }

    #[test]
    fn display_is_hex() {
        let hash = StateHash([0xab; 32]);
        let hex = format!("{hash}");
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn same_input_same_hash() {
        let a = StateHash::from_bytes(b"hello");
        let b = StateHash::from_bytes(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn different_input_different_hash() {
        let a = StateHash::from_bytes(b"hello");
        let b = StateHash::from_bytes(b"world");
        assert_ne!(a, b);
    }
}
