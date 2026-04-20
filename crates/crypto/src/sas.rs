//! Short Authentication String (SAS) derivation — 6-word fingerprint grid.
//!
//! See `docs/specs/2026-04-19-ui-design/trust-verification.md` §Data
//! dependencies and the implementation plan
//! `docs/plans/2026-04-20-ui-phase-1d-trust-verification.md` Task 1.
//!
//! ## Property
//!
//! Given a shared session key and the two endpoint identities (in any
//! order), [`sas_words`] produces the same six words. Both peers compare
//! the same six words read aloud. If an active MITM sits between them,
//! the session keys they each derived will differ and at least one
//! window of the output will disagree.
//!
//! ## Construction
//!
//! Let `a`, `b` be the two endpoint IDs (32-byte Ed25519 public keys).
//! We canonicalise ordering by sorting `(a, b)` lexicographically so the
//! output is symmetric. The fingerprint is:
//!
//! ```text
//! hash = BLAKE3(DS_TAG || canon_a || canon_b || session_key)
//! ```
//!
//! where `DS_TAG = b"willow-sas-v1"`. We then take 6 × 11-bit windows
//! from the first 66 bits of `hash` (big-endian) and look up each
//! 11-bit index in [`SAS_WORDS`](crate::sas_wordlist::SAS_WORDS). Because
//! the wordlist is exactly 2048 entries, each 11-bit window maps one-to-
//! one to a word without modular arithmetic.
//!
//! BLAKE3 is used because it is already a workspace dependency, WASM-
//! safe, and runs at roughly line rate on every target platform. The
//! domain-separation tag prevents a collision with any other BLAKE3-
//! hashed payload in the protocol.
//!
//! ## Safety
//!
//! `sas_words` is pure and panic-free on valid inputs. Empty session
//! keys, mismatched endpoint bytes, and short/long keys are all
//! accepted: the output is deterministic over the exact bytes provided.
//! Callers are responsible for passing the real per-DM session key
//! (see ambiguity decision in the plan: bootstrap seed is
//! `blake3(local_pub || remote_pub || DS_TAG)` until the real exchange
//! is wired up).
//!
//! No secrets are revealed by the fingerprint: the 66-bit output is a
//! commitment to the session key, not a key itself. An attacker who
//! learns the fingerprint cannot invert BLAKE3 or recover the key.

use willow_identity::EndpointId;

use crate::sas_wordlist::{SAS_WORDLIST_LEN, SAS_WORDS};

/// Domain-separation tag. Any change to this constant breaks cross-
/// version SAS compatibility; bump the suffix if the derivation changes.
pub const SAS_DS_TAG: &[u8] = b"willow-sas-v1";

/// How many words make up a SAS fingerprint. Fixed at 6 per spec.
pub const SAS_WORD_COUNT: usize = 6;

/// SAS-specific errors. None are currently emitted — `sas_words` is
/// total on accepted inputs — but the error type reserves space for
/// future validation hooks (e.g. wordlist integrity checks) without a
/// breaking API change.
#[derive(Debug, thiserror::Error)]
pub enum SasError {
    /// The derived 66-bit window index exceeded the wordlist length.
    /// Should be impossible when the wordlist is exactly 2048 entries.
    #[error("word index {0} out of range (wordlist has {SAS_WORDLIST_LEN} entries)")]
    IndexOutOfRange(usize),
}

/// Derive the six-word SAS fingerprint for a pair of endpoints and a
/// shared session key.
///
/// Symmetric in `(a, b)`: `sas_words(k, a, b)` == `sas_words(k, b, a)`.
/// Deterministic and pure — safe to call from any thread and from WASM.
pub fn sas_words(session_key: &[u8], a: &EndpointId, b: &EndpointId) -> [String; SAS_WORD_COUNT] {
    let a_bytes = *a.as_bytes();
    let b_bytes = *b.as_bytes();

    // Canonicalise ordering lex-smaller-first for symmetry.
    let (first, second) = if a_bytes <= b_bytes {
        (a_bytes, b_bytes)
    } else {
        (b_bytes, a_bytes)
    };

    let mut hasher = blake3::Hasher::new();
    hasher.update(SAS_DS_TAG);
    hasher.update(&first);
    hasher.update(&second);
    hasher.update(session_key);
    let hash = hasher.finalize();

    // Extract six 11-bit windows from the first 66 bits (9 bytes + 2
    // bits of the 10th byte). Big-endian: byte 0 is the most-significant.
    let bytes = hash.as_bytes();
    let mut out: [String; SAS_WORD_COUNT] = Default::default();
    for (i, slot) in out.iter_mut().enumerate() {
        let idx = extract_11bit_window(bytes, i);
        // The wordlist is exactly 2048 entries = 2^11, so `idx` is always
        // a valid index. Defensive bounds check kept to make an unsafe
        // indexing panic impossible even under accidental list mutation.
        let word = SAS_WORDS
            .get(idx)
            .copied()
            .unwrap_or_else(|| panic!("SAS index {idx} out of range"));
        *slot = word.to_string();
    }
    out
}

/// Extract the `window_index`-th 11-bit window (big-endian) from the
/// hash bytes. `window_index` must be < 6; we read from bit offset
/// `window_index * 11` for 11 bits.
#[inline]
fn extract_11bit_window(bytes: &[u8], window_index: usize) -> usize {
    debug_assert!(window_index < SAS_WORD_COUNT);
    let bit_offset = window_index * 11;
    let byte_offset = bit_offset / 8;
    let bit_in_byte = bit_offset % 8;

    // Read three bytes big-endian to cover all alignment cases. The
    // hash is 32 bytes so byte_offset + 2 always fits.
    let b0 = bytes[byte_offset] as u32;
    let b1 = bytes[byte_offset + 1] as u32;
    let b2 = bytes[byte_offset + 2] as u32;
    let combined = (b0 << 16) | (b1 << 8) | b2;

    // We want 11 bits starting at `bit_in_byte` from the top of `combined`.
    // Shift right to drop the tail bits, then mask to 11 bits.
    let shift = 24 - bit_in_byte - 11;
    ((combined >> shift) & 0x7FF) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    fn peer_pair() -> (EndpointId, EndpointId) {
        let a = Identity::generate().endpoint_id();
        let b = Identity::generate().endpoint_id();
        (a, b)
    }

    /// Same key + same pair must always produce the same words.
    #[test]
    fn deterministic_for_same_inputs() {
        let (a, b) = peer_pair();
        let key = [0x11u8; 32];
        let w1 = sas_words(&key, &a, &b);
        let w2 = sas_words(&key, &a, &b);
        assert_eq!(w1, w2);
    }

    /// Swapping `(a, b)` must not change the output.
    #[test]
    fn symmetric_in_endpoint_pair() {
        let (a, b) = peer_pair();
        let key = [0x42u8; 32];
        let forward = sas_words(&key, &a, &b);
        let reversed = sas_words(&key, &b, &a);
        assert_eq!(forward, reversed);
    }

    /// Changing the session key must change the fingerprint. (With
    /// overwhelming probability — collisions at 66 bits are ~2^-66.)
    #[test]
    fn different_keys_yield_different_words() {
        let (a, b) = peer_pair();
        let k1 = [0x01u8; 32];
        let k2 = [0x02u8; 32];
        assert_ne!(sas_words(&k1, &a, &b), sas_words(&k2, &a, &b));
    }

    /// Swapping in a different peer must change the fingerprint (same
    /// probability caveat as above).
    #[test]
    fn different_peers_yield_different_words() {
        let a = Identity::generate().endpoint_id();
        let b = Identity::generate().endpoint_id();
        let c = Identity::generate().endpoint_id();
        let key = [0x77u8; 32];
        assert_ne!(sas_words(&key, &a, &b), sas_words(&key, &a, &c));
    }

    /// Every output word must live in the 2048-word canonical list.
    #[test]
    fn output_words_are_in_canonical_wordlist() {
        let (a, b) = peer_pair();
        let key = [0xAAu8; 32];
        let words = sas_words(&key, &a, &b);
        for w in &words {
            assert!(
                SAS_WORDS.contains(&w.as_str()),
                "{w} missing from SAS_WORDS"
            );
        }
    }

    /// The wordlist must be exactly 2048 entries so 11-bit windows
    /// address a word one-to-one.
    #[test]
    fn wordlist_size_matches_11_bit_windows() {
        assert_eq!(SAS_WORDS.len(), 1 << 11);
        assert_eq!(SAS_WORDLIST_LEN, 2048);
    }

    /// Every SAS word must be 3-8 ASCII-lowercase chars (checked again
    /// here so the sas module owns the contract from the consumer side).
    #[test]
    fn all_words_are_ascii_lowercase_3_to_8_chars() {
        for w in SAS_WORDS.iter() {
            assert!(
                (3..=8).contains(&w.len()),
                "word length out of 3..=8: {w} ({} chars)",
                w.len()
            );
            assert!(
                w.chars().all(|c| c.is_ascii_lowercase()),
                "non-ascii-lowercase word: {w}"
            );
        }
    }

    /// Regression / stable-vector test. Fixed endpoint identities + fixed
    /// session key must always produce the exact six words below. Any
    /// change to the derivation (DS tag, hash, window layout, wordlist
    /// order) breaks this assertion and signals a protocol incompatibility.
    ///
    /// The vector was captured deterministically from this file's
    /// implementation; see the plan Task 1 note. Identities are derived
    /// from fixed 32-byte seeds so the public keys are valid and stable.
    #[test]
    fn stable_vector_regression() {
        let a_seed = [
            0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20,
        ];
        let b_seed = [
            0x20u8, 0x1f, 0x1e, 0x1d, 0x1c, 0x1b, 0x1a, 0x19, 0x18, 0x17, 0x16, 0x15, 0x14, 0x13,
            0x12, 0x11, 0x10, 0x0f, 0x0e, 0x0d, 0x0c, 0x0b, 0x0a, 0x09, 0x08, 0x07, 0x06, 0x05,
            0x04, 0x03, 0x02, 0x01,
        ];
        let a = Identity::from_bytes(&a_seed)
            .expect("valid seed")
            .endpoint_id();
        let b = Identity::from_bytes(&b_seed)
            .expect("valid seed")
            .endpoint_id();
        let session_key = [0x5Au8; 32];

        let got = sas_words(&session_key, &a, &b);

        let expected: [&str; SAS_WORD_COUNT] = STABLE_VECTOR;
        assert_eq!(got, expected, "stable SAS vector drift");
    }

    /// Stable-vector golden words. Captured on first green test run.
    /// Regenerate only when the derivation intentionally changes; bump
    /// [`SAS_DS_TAG`] in the same commit so old clients recompute.
    const STABLE_VECTOR: [&str; SAS_WORD_COUNT] = [
        "forcible", "parent", "vinifera", "unarmed", "utilize", "fraud",
    ];
}
