//! Content-addressed event hashing.
//!
//! [`EventHash`] is a 32-byte SHA-256 digest used as the identity of an
//! event and for all DAG links (prev, deps). Two events with the same
//! content always produce the same hash.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A 32-byte SHA-256 hash used as an event's identity and for DAG links.
///
/// `Ord` is lexicographic byte comparison — used by `BTreeSet` in
/// topological sort for deterministic tiebreaking of concurrent events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventHash(pub [u8; 32]);

impl EventHash {
    /// The zero hash — used as `prev` for an author's first event.
    pub const ZERO: EventHash = EventHash([0u8; 32]);

    /// Compute a SHA-256 hash from raw bytes.
    pub fn from_bytes(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        EventHash(hash)
    }
}

impl Default for EventHash {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Ord for EventHash {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for EventHash {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for EventHash {
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
        assert_eq!(EventHash::ZERO.0, [0u8; 32]);
    }

    #[test]
    fn same_input_same_hash() {
        let a = EventHash::from_bytes(b"hello");
        let b = EventHash::from_bytes(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn different_input_different_hash() {
        let a = EventHash::from_bytes(b"hello");
        let b = EventHash::from_bytes(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn display_is_hex() {
        let hash = EventHash([0xab; 32]);
        let hex = format!("{hash}");
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(hex.starts_with("ab"));
    }

    #[test]
    fn ord_is_lexicographic() {
        let mut a = EventHash([0u8; 32]);
        let mut b = EventHash([0u8; 32]);
        a.0[0] = 1;
        b.0[0] = 2;
        assert!(a < b);

        // Same first byte, differ on second.
        a.0[0] = 1;
        b.0[0] = 1;
        a.0[1] = 0;
        b.0[1] = 1;
        assert!(a < b);
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(EventHash::default(), EventHash::ZERO);
    }
}
