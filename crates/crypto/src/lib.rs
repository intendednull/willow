//! # Willow Crypto
//!
//! End-to-end encryption primitives for the Willow P2P chat network.
//!
//! ## Content Encryption
//!
//! Messages are encrypted at the [`Content`](willow_messaging::Content) level
//! using ChaCha20-Poly1305 (AEAD) with random nonces. Each channel has a
//! symmetric [`ChannelKey`] shared among its members.
//!
//! ## Key Distribution
//!
//! Channel keys are distributed via the invite system. When a server owner
//! creates an invite, the channel key is encrypted for the recipient using
//! ephemeral X25519 Diffie-Hellman + HKDF key derivation.

use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use x25519_dalek::{PublicKey as X25519Public, StaticSecret as X25519Secret};
use zeroize::{ZeroizeOnDrop, Zeroizing};

use willow_identity::Identity;

// Re-export for convenience.
pub use willow_messaging::{Content, SealedContent};

// ───── SAS (Short Authentication String) ────────────────────────────────────

pub mod sas;
pub mod sas_wordlist;

pub use sas::{
    peer_fingerprint, sas_words, SasError, PEER_FINGERPRINT_DS_TAG, SAS_DS_TAG, SAS_WORD_COUNT,
};
pub use sas_wordlist::{SAS_WORDLIST_HASH, SAS_WORDLIST_LEN, SAS_WORDS};

// ───── HKDF Domain Separators ──────────────────────────────────────────────
//
// Every HKDF derivation in this crate uses an explicit, versioned domain
// string as (part of) its `info` parameter. This ensures keys derived
// for distinct purposes (ratchet message keys, ratchet seed advance,
// channel-key-wrap) cannot collide, even if an implementation bug ever
// reused the same salt or IKM across contexts.
//
// The `v1` component is an in-band version marker. Any future change that
// alters the semantics of a derivation MUST bump the version (e.g. `v2`)
// so old and new keys are provably distinct.
//
// NOTE: the prefixes below are part of the wire protocol — changing them
// breaks decryption of any previously encrypted content. Willow has not
// shipped a stable release, so this is acceptable as an initial rollout.

/// Domain prefix for per-message key derivation inside the ratchet.
const HKDF_RATCHET_MSG_DOMAIN: &[u8] = b"willow-crypto/v1/ratchet/msg";

/// Domain string for the ratchet's seed-advance derivation.
const HKDF_RATCHET_ADVANCE_DOMAIN: &[u8] = b"willow-crypto/v1/ratchet/advance";

/// Domain string for the channel-key-wrap derivation (encrypt_channel_key_for
/// / decrypt_channel_key).
const HKDF_KEYWRAP_DOMAIN: &[u8] = b"willow-crypto/v1/keywrap/channel-key";

// ───── Errors ───────────────────────────────────────────────────────────────

/// Errors that can occur during cryptographic operations.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("encryption failed")]
    EncryptionFailed,

    #[error("decryption failed")]
    DecryptionFailed,

    #[error("key derivation failed")]
    KeyDerivationFailed,

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("ed25519 to x25519 conversion failed")]
    KeyConversionFailed,

    #[error("ratchet counter {claimed} out of range (max {max})")]
    RatchetCounterOutOfRange { claimed: u64, max: u64 },
}

// ───── Types ────────────────────────────────────────────────────────────────

/// A 256-bit symmetric key for encrypting a channel's messages.
///
/// The wrapped bytes are zeroized when the value is dropped so secret
/// material doesn't linger in freed memory.
#[derive(Clone, PartialEq, ZeroizeOnDrop)]
pub struct ChannelKey(#[zeroize] pub(crate) [u8; 32]);

impl std::fmt::Debug for ChannelKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChannelKey([REDACTED])")
    }
}

impl ChannelKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// A channel key encrypted for a specific recipient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedChannelKey {
    /// Sender's ephemeral X25519 public key (for the DH exchange).
    pub ephemeral_public: [u8; 32],
    /// The channel key bytes, encrypted with the derived shared secret.
    pub ciphertext: Vec<u8>,
    /// Nonce used for the key-wrapping encryption.
    pub nonce: [u8; 12],
}

// ───── Key Ratchet ──────────────────────────────────────────────────────────

/// A forward-secret key ratchet that derives unique per-message keys.
///
/// Each call to [`next_key()`](KeyRatchet::next_key) derives a new message
/// key via HKDF and increments the counter. Old keys cannot be recovered
/// from the current state, providing forward secrecy.
///
/// The ratchet is seeded from a [`ChannelKey`] and can be re-seeded on
/// key rotation (epoch change).
#[derive(Clone, ZeroizeOnDrop)]
pub struct KeyRatchet {
    seed: [u8; 32],
    #[zeroize(skip)]
    counter: u64,
    #[zeroize(skip)]
    epoch: u32,
}

impl KeyRatchet {
    /// Create a new ratchet from a channel key and epoch.
    pub fn new(key: &ChannelKey, epoch: u32) -> Self {
        Self {
            seed: key.0,
            counter: 1, // Start at 1; counter 0 means "no ratchet" (backwards compat).
            epoch,
        }
    }

    /// Derive the next message key and advance the ratchet.
    ///
    /// The returned key is unique to this (epoch, counter) pair. After
    /// calling this, the previous key cannot be derived again.
    pub fn next_key(&mut self) -> (ChannelKey, u32, u64) {
        let hk = Hkdf::<Sha256>::new(None, &self.seed);

        // Derive message key from seed + counter.
        // The HKDF `info` is explicitly prefixed with a versioned domain
        // separator so ratchet-derived keys cannot collide with keys
        // derived for any other use (channel-key-wrap, future contexts).
        // See `HKDF_RATCHET_MSG_DOMAIN`.
        let mut info = Vec::with_capacity(HKDF_RATCHET_MSG_DOMAIN.len() + 12);
        info.extend_from_slice(HKDF_RATCHET_MSG_DOMAIN);
        info.extend_from_slice(&self.counter.to_le_bytes());
        info.extend_from_slice(&self.epoch.to_le_bytes());

        let mut message_key = [0u8; 32];
        hk.expand(&info, &mut message_key)
            .expect("32 bytes is valid HKDF output length");

        // Ratchet forward: derive next seed from current seed + counter.
        // This ensures the old seed can't recover future keys. The advance
        // step uses a distinct versioned `info` string so the next-seed
        // derivation cannot collide with the message-key derivation.
        let mut next_seed = [0u8; 32];
        let hk_advance = Hkdf::<Sha256>::new(Some(&info), &self.seed);
        hk_advance
            .expand(HKDF_RATCHET_ADVANCE_DOMAIN, &mut next_seed)
            .expect("32 bytes is valid HKDF output length");
        self.seed = next_seed;

        let counter = self.counter;
        self.counter += 1;

        (ChannelKey(message_key), self.epoch, counter)
    }

    /// Current epoch.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    /// Current counter value (number of keys derived so far).
    pub fn counter(&self) -> u64 {
        self.counter
    }

    /// Re-seed the ratchet with a new channel key (on key rotation).
    pub fn reseed(&mut self, key: &ChannelKey, new_epoch: u32) {
        self.seed = key.0;
        self.counter = 1;
        self.epoch = new_epoch;
    }
}

impl std::fmt::Debug for KeyRatchet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyRatchet")
            .field("epoch", &self.epoch)
            .field("counter", &self.counter)
            .finish()
    }
}

/// Derive a specific message key for decryption given the epoch and counter.
///
/// This is used by the receiver who needs to derive the same key the sender
/// used. The receiver must have the channel key for the given epoch.
pub fn derive_message_key(channel_key: &ChannelKey, epoch: u32, counter: u64) -> ChannelKey {
    // Replay the ratchet from counter=1 to the target counter.
    let mut ratchet = KeyRatchet::new(channel_key, epoch);
    let mut key;
    loop {
        let (k, _, c) = ratchet.next_key();
        key = k;
        if c >= counter {
            break;
        }
    }
    key
}

// ───── Content Encryption ───────────────────────────────────────────────────

/// Generate a random 256-bit channel key.
pub fn generate_channel_key() -> ChannelKey {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    ChannelKey(bytes)
}

/// Encrypt a [`Content`] value using a channel's symmetric key.
///
/// For forward secrecy, pass the per-message key from a [`KeyRatchet`]
/// along with the epoch and counter. The `ratchet_counter` is stored in
/// the sealed content so the receiver can derive the same key.
pub fn seal_content(
    content: &Content,
    key: &ChannelKey,
    epoch: u32,
) -> Result<SealedContent, CryptoError> {
    seal_content_with_counter(content, key, epoch, 0)
}

/// Encrypt with explicit ratchet counter (used when forward secrecy is active).
pub fn seal_content_with_counter(
    content: &Content,
    key: &ChannelKey,
    epoch: u32,
    ratchet_counter: u64,
) -> Result<SealedContent, CryptoError> {
    let plaintext =
        willow_transport::pack(content).map_err(|e| CryptoError::Serialization(e.to_string()))?;

    let cipher = ChaCha20Poly1305::new(key.0.as_ref().into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| CryptoError::EncryptionFailed)?;

    Ok(SealedContent {
        ciphertext,
        nonce: nonce_bytes,
        key_epoch: epoch,
        ratchet_counter,
    })
}

/// Maximum lookahead above the receiver's current ratchet counter that
/// [`open_content_bounded`] will accept. This exists to bound the work
/// [`derive_message_key`] has to do before AEAD verification — without
/// it, an attacker-controlled `ratchet_counter` in a sealed packet
/// would force the receiver to perform 2 HKDF-Expand operations per
/// step up to the claimed value, and `u64::MAX` would take hundreds of
/// thousands of years on a single core.
///
/// The value is deliberately generous (1024 messages) so that a
/// receiver who misses a burst of messages can still catch up.
pub const MAX_RATCHET_LOOKAHEAD: u64 = 1024;

/// Decrypt a [`SealedContent`] back to a [`Content`] value, bounding
/// how far ahead of `current_counter` the sealed packet's claimed
/// `ratchet_counter` may be.
///
/// If `sealed.ratchet_counter > current_counter + MAX_RATCHET_LOOKAHEAD`,
/// returns [`CryptoError::RatchetCounterOutOfRange`] **before** doing
/// any HKDF work or AEAD verification. This prevents a CPU DoS where
/// an attacker ships a packet with a huge `ratchet_counter` and
/// freezes the receiver inside [`derive_message_key`].
///
/// Callers should pass the highest counter they have successfully
/// processed for this channel epoch. Start at 0 for a fresh channel
/// and advance monotonically.
///
/// If `ratchet_counter == 0`, the channel key is used directly
/// (backwards compat) and no bounds check applies.
pub fn open_content_bounded(
    sealed: &SealedContent,
    key: &ChannelKey,
    current_counter: u64,
) -> Result<Content, CryptoError> {
    if sealed.ratchet_counter > 0 {
        let max = current_counter.saturating_add(MAX_RATCHET_LOOKAHEAD);
        if sealed.ratchet_counter > max {
            return Err(CryptoError::RatchetCounterOutOfRange {
                claimed: sealed.ratchet_counter,
                max,
            });
        }
    }

    let decrypt_key = if sealed.ratchet_counter > 0 {
        derive_message_key(key, sealed.key_epoch, sealed.ratchet_counter)
    } else {
        key.clone()
    };
    let cipher = ChaCha20Poly1305::new(decrypt_key.0.as_ref().into());
    let nonce = Nonce::from_slice(&sealed.nonce);

    let plaintext = cipher
        .decrypt(nonce, sealed.ciphertext.as_ref())
        .map_err(|_| CryptoError::DecryptionFailed)?;

    willow_transport::unpack(&plaintext).map_err(|e| CryptoError::Serialization(e.to_string()))
}

/// Decrypt a [`SealedContent`] back to a [`Content`] value.
///
/// Thin wrapper over [`open_content_bounded`] with a `current_counter`
/// of 0 — accepts any `ratchet_counter` up to
/// [`MAX_RATCHET_LOOKAHEAD`]. Callers that track per-channel ratchet
/// state should prefer [`open_content_bounded`] directly so they can
/// advance the allowed window as messages are processed.
pub fn open_content(sealed: &SealedContent, key: &ChannelKey) -> Result<Content, CryptoError> {
    open_content_bounded(sealed, key, 0)
}

// ───── Key Exchange ─────────────────────────────────────────────────────────

/// Convert an Ed25519 identity to an X25519 static secret.
///
/// The conversion follows RFC 7748: SHA-512 hash the Ed25519 seed, take the
/// first 32 bytes with clamping. This matches the standard Ed25519-to-X25519 conversion.
pub fn identity_to_x25519(identity: &Identity) -> Result<X25519Secret, CryptoError> {
    use sha2::{Digest, Sha512};

    let seed = identity.to_bytes();

    // SHA-512(seed), take first 32 bytes = X25519 secret (clamping done by x25519-dalek).
    let hash = Sha512::digest(&seed);
    let mut x25519_bytes = [0u8; 32];
    x25519_bytes.copy_from_slice(&hash[..32]);

    Ok(X25519Secret::from(x25519_bytes))
}

/// Derive an X25519 public key from an Ed25519 public key.
///
/// Converts the Edwards-curve point to the Montgomery-curve point.
pub fn ed25519_public_to_x25519(
    ed25519_public_bytes: &[u8; 32],
) -> Result<X25519Public, CryptoError> {
    let ed_point = curve25519_dalek::edwards::CompressedEdwardsY(*ed25519_public_bytes);
    let edwards = ed_point
        .decompress()
        .ok_or(CryptoError::KeyConversionFailed)?;
    let montgomery = edwards.to_montgomery();
    Ok(X25519Public::from(montgomery.0))
}

/// Encrypt a [`ChannelKey`] for a specific recipient given their Ed25519
/// public key bytes.
pub fn encrypt_channel_key_for(
    channel_key: &ChannelKey,
    recipient_ed25519_public: &[u8; 32],
) -> Result<EncryptedChannelKey, CryptoError> {
    let recipient_x25519 = ed25519_public_to_x25519(recipient_ed25519_public)?;

    let ephemeral_secret = X25519Secret::random_from_rng(OsRng);
    let ephemeral_public = X25519Public::from(&ephemeral_secret);
    let shared_secret = ephemeral_secret.diffie_hellman(&recipient_x25519);

    // Derive wrapping key via HKDF-SHA256.
    // Wrapped in `Zeroizing` so the derived key is wiped on drop rather
    // than lingering on the stack after `cipher` is consumed.
    let hk = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut wrapping_key: Zeroizing<[u8; 32]> = Zeroizing::new([0u8; 32]);
    hk.expand(HKDF_KEYWRAP_DOMAIN, wrapping_key.as_mut())
        .map_err(|_| CryptoError::KeyDerivationFailed)?;

    let cipher = ChaCha20Poly1305::new(wrapping_key.as_ref().into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, channel_key.0.as_ref())
        .map_err(|_| CryptoError::EncryptionFailed)?;

    Ok(EncryptedChannelKey {
        ephemeral_public: ephemeral_public.to_bytes(),
        ciphertext,
        nonce: nonce_bytes,
    })
}

/// Decrypt a [`ChannelKey`] that was encrypted for our identity.
pub fn decrypt_channel_key(
    encrypted: &EncryptedChannelKey,
    our_identity: &Identity,
) -> Result<ChannelKey, CryptoError> {
    let our_x25519 = identity_to_x25519(our_identity)?;
    let sender_ephemeral = X25519Public::from(encrypted.ephemeral_public);
    let shared_secret = our_x25519.diffie_hellman(&sender_ephemeral);

    // Wrap the derived key in `Zeroizing` so it's wiped on drop rather
    // than lingering on the stack after `cipher` is consumed.
    let hk = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut wrapping_key: Zeroizing<[u8; 32]> = Zeroizing::new([0u8; 32]);
    hk.expand(HKDF_KEYWRAP_DOMAIN, wrapping_key.as_mut())
        .map_err(|_| CryptoError::KeyDerivationFailed)?;

    let cipher = ChaCha20Poly1305::new(wrapping_key.as_ref().into());
    let nonce = Nonce::from_slice(&encrypted.nonce);

    let plaintext = cipher
        .decrypt(nonce, encrypted.ciphertext.as_ref())
        .map_err(|_| CryptoError::DecryptionFailed)?;

    if plaintext.len() != 32 {
        return Err(CryptoError::DecryptionFailed);
    }
    let mut key_bytes = [0u8; 32];
    key_bytes.copy_from_slice(&plaintext);

    Ok(ChannelKey(key_bytes))
}

// ───── RatchetCache ─────────────────────────────────────────────────────────

/// A per-channel cache of derived ratchet keys that avoids O(counter) replay
/// cost on every decryption.
///
/// [`derive_message_key`] replays the ratchet from counter=1 on every call,
/// which costs 2 HKDF-Expand operations per step. For a receiver on a chatty
/// channel this repeated work accumulates quickly — 1M counter steps cost
/// roughly 1 second of CPU time.
///
/// `RatchetCache` remembers the last-derived keys and the ratchet state at the
/// highest cached counter per epoch. When asked for a key at counter `c` it
/// checks the cache first; on a miss it resumes from the highest cached entry
/// rather than rewinding all the way to counter=1.
///
/// The cache is bounded to `max_entries` entries. When the bound is exceeded
/// the lowest-counter entry is evicted (oldest messages are least likely to be
/// needed again).
///
/// This type is local to the receiver and has no wire-format impact.
///
/// # Example
///
/// ```
/// use willow_crypto::{generate_channel_key, RatchetCache};
///
/// let key = generate_channel_key();
/// let mut cache = RatchetCache::new(128);
///
/// // First call replays the ratchet; subsequent calls for the same
/// // (epoch, counter) return the cached key without HKDF work.
/// let k1 = cache.derive_or_cached(&key, 0, 5);
/// let k2 = cache.derive_or_cached(&key, 0, 5);
/// assert_eq!(k1.as_bytes(), k2.as_bytes());
/// ```
pub struct RatchetCache {
    /// Cached message keys keyed by `(epoch, counter)`.
    cache: std::collections::BTreeMap<(u32, u64), ChannelKey>,
    /// Saved ratchet state at the highest counter per epoch, so we can
    /// advance forward without rewinding.
    ///
    /// Value is `(counter_at_save, ratchet_ready_to_produce_counter+1)`.
    ratchet_states: std::collections::HashMap<u32, (u64, KeyRatchet)>,
    max_entries: usize,
}

impl std::fmt::Debug for RatchetCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RatchetCache")
            .field("entries", &self.cache.len())
            .field("max_entries", &self.max_entries)
            .finish()
    }
}

impl RatchetCache {
    /// Create a new cache with a maximum of `max_entries` cached keys.
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache: std::collections::BTreeMap::new(),
            ratchet_states: std::collections::HashMap::new(),
            max_entries,
        }
    }

    /// Return the message key for `(epoch, target_counter)`, deriving and
    /// caching it if not already present.
    ///
    /// Uses any previously saved ratchet state for `epoch` to resume from the
    /// highest already-computed counter rather than replaying from counter=1.
    ///
    /// `target_counter = 0` is the sentinel for "no ratchet" and returns the
    /// channel key directly without advancing the ratchet.
    pub fn derive_or_cached(
        &mut self,
        channel_key: &ChannelKey,
        epoch: u32,
        target_counter: u64,
    ) -> ChannelKey {
        // counter=0 is the "no ratchet" sentinel — return the channel key as-is.
        if target_counter == 0 {
            return channel_key.clone();
        }

        // Cache hit: return immediately without any HKDF work.
        if let Some(key) = self.cache.get(&(epoch, target_counter)) {
            return key.clone();
        }

        // Choose the best starting point: either a saved ratchet state for
        // this epoch (if its counter is below the target) or a fresh ratchet.
        let mut ratchet = match self.ratchet_states.get(&epoch) {
            Some((saved_counter, saved_ratchet)) if *saved_counter < target_counter => {
                saved_ratchet.clone()
            }
            _ => KeyRatchet::new(channel_key, epoch),
        };

        // Advance the ratchet until we hit target_counter, caching each key
        // along the way so future calls within this range are O(1).
        let mut result: Option<ChannelKey> = None;
        loop {
            let (key, _ep, counter) = ratchet.next_key();

            // Evict oldest entry when the cache is full.
            if self.cache.len() >= self.max_entries {
                if let Some(oldest) = self.cache.keys().next().copied() {
                    self.cache.remove(&oldest);
                }
            }
            self.cache.insert((epoch, counter), key.clone());

            if counter == target_counter {
                result = Some(key);
            }

            if counter >= target_counter {
                // Save ratchet state only at the final step — the ratchet is
                // now positioned to produce counter+1 on the next call.
                let should_save = match self.ratchet_states.get(&epoch) {
                    Some((prev, _)) => counter > *prev,
                    None => true,
                };
                if should_save {
                    self.ratchet_states.insert(epoch, (counter, ratchet));
                }
                break;
            }
        }

        // result is always Some here: the loop exits only when counter == target_counter,
        // at which point result was set. The unwrap_or_else is a safety net.
        result.unwrap_or_else(|| derive_message_key(channel_key, epoch, target_counter))
    }

    /// Invalidate all cached keys for `epoch` (e.g., on key rotation /
    /// re-seed). The next call for this epoch will replay from counter=1.
    pub fn evict_epoch(&mut self, epoch: u32) {
        self.cache.retain(|(e, _), _| *e != epoch);
        self.ratchet_states.remove(&epoch);
    }

    /// Drop every cached message key and saved ratchet state.
    ///
    /// Call this on sign-out, server-leave, or any other identity-bound
    /// teardown so derived [`ChannelKey`] material does not linger in
    /// process memory longer than necessary. Both [`ChannelKey`] and
    /// [`KeyRatchet`] implement [`zeroize::ZeroizeOnDrop`], so removing
    /// them from the underlying maps wipes their secret material before
    /// the allocation is freed.
    pub fn clear(&mut self) {
        self.cache.clear();
        self.ratchet_states.clear();
    }

    /// Number of entries currently in the cache.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns `true` if the cache contains no entries.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

// ───── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use willow_messaging::MessageId;

    fn recipient_public_bytes(identity: &Identity) -> [u8; 32] {
        *identity.public_key().as_bytes()
    }

    #[test]
    fn generate_channel_key_is_random() {
        let a = generate_channel_key();
        let b = generate_channel_key();
        assert_ne!(a.0, b.0);
    }

    #[test]
    fn seal_and_open_round_trip() {
        let key = generate_channel_key();
        let content = Content::Text {
            body: "hello encrypted world".into(),
        };
        let sealed = seal_content(&content, &key, 0).unwrap();
        let decrypted = open_content(&sealed, &key).unwrap();
        assert_eq!(decrypted, content);
    }

    #[test]
    fn open_with_wrong_key_fails() {
        let key_a = generate_channel_key();
        let key_b = generate_channel_key();
        let content = Content::Text {
            body: "secret".into(),
        };
        let sealed = seal_content(&content, &key_a, 0).unwrap();
        assert!(open_content(&sealed, &key_b).is_err());
    }

    #[test]
    fn open_with_tampered_ciphertext_fails() {
        let key = generate_channel_key();
        let content = Content::Text {
            body: "tamper me".into(),
        };
        let mut sealed = seal_content(&content, &key, 0).unwrap();
        if let Some(byte) = sealed.ciphertext.first_mut() {
            *byte ^= 0xFF;
        }
        assert!(open_content(&sealed, &key).is_err());
    }

    #[test]
    fn open_with_tampered_nonce_fails() {
        let key = generate_channel_key();
        let content = Content::Text {
            body: "nonce test".into(),
        };
        let mut sealed = seal_content(&content, &key, 0).unwrap();
        sealed.nonce[0] ^= 0xFF;
        assert!(open_content(&sealed, &key).is_err());
    }

    #[test]
    fn seal_produces_unique_nonces() {
        let key = generate_channel_key();
        let content = Content::Text {
            body: "same content".into(),
        };
        let sealed_a = seal_content(&content, &key, 0).unwrap();
        let sealed_b = seal_content(&content, &key, 0).unwrap();
        assert_ne!(sealed_a.nonce, sealed_b.nonce);
        assert_ne!(sealed_a.ciphertext, sealed_b.ciphertext);
    }

    #[test]
    fn key_epoch_is_preserved() {
        let key = generate_channel_key();
        let content = Content::Text {
            body: "epoch".into(),
        };
        let sealed = seal_content(&content, &key, 42).unwrap();
        assert_eq!(sealed.key_epoch, 42);
    }

    #[test]
    fn all_content_variants_encrypt() {
        let key = generate_channel_key();
        let target = MessageId::new();

        let variants = vec![
            Content::Text {
                body: "text".into(),
            },
            Content::File {
                hash: "abc123".into(),
                filename: "photo.jpg".into(),
                mime_type: "image/jpeg".into(),
                size_bytes: 1024,
            },
            Content::Reaction {
                target: target.clone(),
                emoji: "👍".into(),
            },
            Content::Reply {
                parent: target.clone(),
                body: "reply".into(),
            },
            Content::Edit {
                target: target.clone(),
                new_body: "edited".into(),
            },
            Content::Delete {
                target: target.clone(),
            },
            Content::System {
                description: "user joined".into(),
            },
        ];

        for content in variants {
            let sealed = seal_content(&content, &key, 0).unwrap();
            let decrypted = open_content(&sealed, &key).unwrap();
            assert_eq!(decrypted, content);
        }
    }

    #[test]
    fn sealed_content_serde_round_trip() {
        let key = generate_channel_key();
        let content = Content::Text {
            body: "serde test".into(),
        };
        let sealed = seal_content(&content, &key, 0).unwrap();
        let bytes = willow_transport::pack(&sealed).unwrap();
        let decoded: SealedContent = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, sealed);
    }

    #[test]
    fn encrypt_channel_key_round_trip() {
        let recipient = Identity::generate();
        let channel_key = generate_channel_key();
        let pub_bytes = recipient_public_bytes(&recipient);

        let encrypted = encrypt_channel_key_for(&channel_key, &pub_bytes).unwrap();
        let decrypted = decrypt_channel_key(&encrypted, &recipient).unwrap();
        assert_eq!(decrypted.0, channel_key.0);
    }

    #[test]
    fn decrypt_channel_key_wrong_identity_fails() {
        let recipient = Identity::generate();
        let intruder = Identity::generate();
        let channel_key = generate_channel_key();
        let pub_bytes = recipient_public_bytes(&recipient);

        let encrypted = encrypt_channel_key_for(&channel_key, &pub_bytes).unwrap();
        assert!(decrypt_channel_key(&encrypted, &intruder).is_err());
    }

    #[test]
    fn identity_to_x25519_is_deterministic() {
        let id = Identity::generate();
        let a = identity_to_x25519(&id).unwrap();
        let b = identity_to_x25519(&id).unwrap();
        let pub_a = X25519Public::from(&a);
        let pub_b = X25519Public::from(&b);
        assert_eq!(pub_a.as_bytes(), pub_b.as_bytes());
    }

    #[test]
    fn x25519_public_key_conversion_consistent() {
        let id = Identity::generate();
        let secret = identity_to_x25519(&id).unwrap();
        let pub_from_secret = X25519Public::from(&secret);

        let pub_bytes = recipient_public_bytes(&id);
        let pub_from_ed = ed25519_public_to_x25519(&pub_bytes).unwrap();
        assert_eq!(pub_from_secret.as_bytes(), pub_from_ed.as_bytes());
    }

    #[test]
    fn types_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ChannelKey>();
        assert_send_sync::<SealedContent>();
        assert_send_sync::<EncryptedChannelKey>();
        assert_send_sync::<KeyRatchet>();
    }

    // ── Key Ratchet Tests ───────────────────────────────────────────────

    #[test]
    fn ratchet_produces_unique_keys() {
        let key = generate_channel_key();
        let mut ratchet = KeyRatchet::new(&key, 0);

        let (k1, _, _) = ratchet.next_key();
        let (k2, _, _) = ratchet.next_key();
        let (k3, _, _) = ratchet.next_key();

        assert_ne!(k1.as_bytes(), k2.as_bytes());
        assert_ne!(k2.as_bytes(), k3.as_bytes());
        assert_ne!(k1.as_bytes(), k3.as_bytes());
    }

    #[test]
    fn ratchet_counter_increments() {
        let key = generate_channel_key();
        let mut ratchet = KeyRatchet::new(&key, 0);

        assert_eq!(ratchet.counter(), 1);
        ratchet.next_key();
        assert_eq!(ratchet.counter(), 2);
        ratchet.next_key();
        assert_eq!(ratchet.counter(), 3);
    }

    #[test]
    fn ratchet_epoch_preserved() {
        let key = generate_channel_key();
        let mut ratchet = KeyRatchet::new(&key, 42);

        let (_, epoch, _) = ratchet.next_key();
        assert_eq!(epoch, 42);
    }

    #[test]
    fn ratchet_reseed_resets_counter() {
        let key = generate_channel_key();
        let mut ratchet = KeyRatchet::new(&key, 0);
        ratchet.next_key();
        ratchet.next_key();
        assert_eq!(ratchet.counter(), 3);

        let new_key = generate_channel_key();
        ratchet.reseed(&new_key, 1);
        assert_eq!(ratchet.counter(), 1);
        assert_eq!(ratchet.epoch(), 1);
    }

    #[test]
    fn ratchet_reseed_produces_different_keys() {
        let key = generate_channel_key();
        let mut ratchet = KeyRatchet::new(&key, 0);
        let (k1, _, _) = ratchet.next_key();

        let new_key = generate_channel_key();
        ratchet.reseed(&new_key, 1);
        let (k2, _, _) = ratchet.next_key();

        assert_ne!(k1.as_bytes(), k2.as_bytes());
    }

    #[test]
    fn derive_message_key_matches_ratchet() {
        let key = generate_channel_key();
        let mut ratchet = KeyRatchet::new(&key, 0);

        // Advance to counter 4. Ratchet starts at 1.
        ratchet.next_key(); // counter 1
        ratchet.next_key(); // counter 2
        ratchet.next_key(); // counter 3
        let (k4, _, counter) = ratchet.next_key(); // counter 4

        assert_eq!(counter, 4);

        // derive_message_key should produce the same key at counter 4.
        let derived = derive_message_key(&key, 0, 4);
        assert_eq!(k4.as_bytes(), derived.as_bytes());
    }

    #[test]
    fn seal_and_open_with_ratchet_round_trip() {
        let key = generate_channel_key();
        let mut ratchet = KeyRatchet::new(&key, 0);

        let content = Content::Text {
            body: "forward secret".into(),
        };

        let (msg_key, epoch, counter) = ratchet.next_key();
        let sealed = seal_content_with_counter(&content, &msg_key, epoch, counter).unwrap();

        assert_eq!(sealed.ratchet_counter, 1);
        assert_eq!(sealed.key_epoch, 0);

        // Receiver derives the same key and decrypts.
        let decrypted = open_content(&sealed, &key).unwrap();
        assert_eq!(decrypted, content);
    }

    #[test]
    fn old_epoch_key_cannot_decrypt_new_epoch() {
        let old_key = generate_channel_key();
        let new_key = generate_channel_key();

        let content = Content::Text {
            body: "new epoch".into(),
        };

        // Encrypt with new key at epoch 1.
        let mut ratchet = KeyRatchet::new(&new_key, 1);
        let (msg_key, epoch, counter) = ratchet.next_key();
        let sealed = seal_content_with_counter(&content, &msg_key, epoch, counter).unwrap();

        // Old key cannot decrypt.
        assert!(open_content(&sealed, &old_key).is_err());

        // New key can decrypt.
        let decrypted = open_content(&sealed, &new_key).unwrap();
        assert_eq!(decrypted, content);
    }

    #[test]
    fn backwards_compat_no_ratchet() {
        // Messages with ratchet_counter=0 should still work without ratchet.
        let key = generate_channel_key();
        let content = Content::Text {
            body: "legacy message".into(),
        };

        let sealed = seal_content(&content, &key, 0).unwrap();
        assert_eq!(sealed.ratchet_counter, 0);

        let decrypted = open_content(&sealed, &key).unwrap();
        assert_eq!(decrypted, content);
    }

    #[test]
    fn channel_key_from_bytes_round_trip() {
        let key = generate_channel_key();
        let bytes = *key.as_bytes();
        let restored = ChannelKey::from_bytes(bytes);
        assert_eq!(key.as_bytes(), restored.as_bytes());
    }

    #[test]
    fn channel_key_debug_redacted() {
        let key = generate_channel_key();
        let debug = format!("{key:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains(&format!("{:02x}", key.as_bytes()[0])));
    }

    /// Compile-time guarantee that [`ChannelKey`] zeroizes its secret
    /// bytes on drop. See issue #127.
    #[test]
    fn channel_key_is_zeroize_on_drop() {
        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<ChannelKey>();
    }

    // ── Issue #110: ratchet counter DoS bound ──────────────────────

    /// Regression guard for issue #110: a sealed packet with a huge
    /// `ratchet_counter` must be rejected **before** `derive_message_key`
    /// is called. Without the bound, u64::MAX would take ~584 000 years
    /// of HKDF work on a single core.
    #[test]
    fn open_content_rejects_huge_ratchet_counter() {
        let key = generate_channel_key();
        let content = Content::Text { body: "x".into() };

        // Seal a legitimate packet at counter=1 so the ciphertext is valid
        // for that key, then tamper with the ratchet_counter field.
        let mut ratchet = KeyRatchet::new(&key, 0);
        let (msg_key, epoch, counter) = ratchet.next_key();
        let mut sealed = seal_content_with_counter(&content, &msg_key, epoch, counter).unwrap();
        sealed.ratchet_counter = u64::MAX;

        let start = std::time::Instant::now();
        let result = open_content_bounded(&sealed, &key, 1);
        let elapsed = start.elapsed();

        assert!(matches!(
            result,
            Err(CryptoError::RatchetCounterOutOfRange { .. })
        ));
        // Should return almost instantly — an unbounded run would not
        // complete in any realistic test timeout. 500ms gives CI plenty
        // of headroom over the ~microsecond real cost of the check.
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "bounds check should return instantly, took {elapsed:?}"
        );
    }

    /// Regression guard for issue #110: any counter more than
    /// `MAX_RATCHET_LOOKAHEAD` above the receiver's current counter
    /// must be rejected.
    #[test]
    fn open_content_bounded_rejects_above_lookahead() {
        let key = generate_channel_key();
        let content = Content::Text {
            body: "attack".into(),
        };
        let mut sealed = seal_content_with_counter(&content, &key, 0, 1).unwrap();
        // current_counter = 100, max = 100 + 1024 = 1124. Claim 1125.
        sealed.ratchet_counter = 100 + MAX_RATCHET_LOOKAHEAD + 1;
        let err = open_content_bounded(&sealed, &key, 100).unwrap_err();
        match err {
            CryptoError::RatchetCounterOutOfRange { claimed, max } => {
                assert_eq!(claimed, 100 + MAX_RATCHET_LOOKAHEAD + 1);
                assert_eq!(max, 100 + MAX_RATCHET_LOOKAHEAD);
            }
            other => panic!("expected RatchetCounterOutOfRange, got {other:?}"),
        }
    }

    /// Regression guard for issue #110: counters at or within the
    /// lookahead window must still decrypt successfully.
    #[test]
    fn open_content_bounded_accepts_within_lookahead() {
        let key = generate_channel_key();
        let content = Content::Text {
            body: "legitimate catch-up".into(),
        };

        // Sender is at counter=50.
        let mut ratchet = KeyRatchet::new(&key, 0);
        let (mut msg_key, mut epoch, mut counter) = ratchet.next_key();
        for _ in 0..49 {
            let (k, e, c) = ratchet.next_key();
            msg_key = k;
            epoch = e;
            counter = c;
        }
        assert_eq!(counter, 50);
        let sealed = seal_content_with_counter(&content, &msg_key, epoch, counter).unwrap();

        // Receiver is at counter=0; 50 is well within 1024 lookahead.
        let decrypted = open_content_bounded(&sealed, &key, 0).unwrap();
        assert_eq!(decrypted, content);
    }

    /// Regression guard for issue #110: the zero-arg `open_content`
    /// shim rejects counters above `MAX_RATCHET_LOOKAHEAD`, since it
    /// implicitly uses `current_counter = 0`.
    #[test]
    fn open_content_shim_rejects_counter_above_lookahead() {
        let key = generate_channel_key();
        let content = Content::Text { body: "x".into() };
        let mut sealed = seal_content_with_counter(&content, &key, 0, 1).unwrap();
        sealed.ratchet_counter = MAX_RATCHET_LOOKAHEAD + 1;
        assert!(matches!(
            open_content(&sealed, &key),
            Err(CryptoError::RatchetCounterOutOfRange { .. })
        ));
    }

    /// `ratchet_counter == 0` bypasses the ratchet derivation entirely
    /// (backwards compat), so the bounds check must not apply there.
    #[test]
    fn open_content_bounded_counter_zero_bypasses_bounds_check() {
        let key = generate_channel_key();
        let content = Content::Text {
            body: "legacy".into(),
        };
        let sealed = seal_content(&content, &key, 0).unwrap();
        assert_eq!(sealed.ratchet_counter, 0);

        // Even with current_counter = 0, ratchet_counter = 0 means "no
        // ratchet" and should decrypt fine.
        let decrypted = open_content_bounded(&sealed, &key, 0).unwrap();
        assert_eq!(decrypted, content);
    }

    // ── Issue #120: RatchetCache tests ─────────────────────────────

    /// `derive_or_cached` must return the same key as `derive_message_key`
    /// for a given (epoch, counter).
    #[test]
    fn ratchet_cache_matches_derive_message_key() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(128);

        for counter in 1u64..=10 {
            let cached = cache.derive_or_cached(&key, 0, counter);
            let direct = derive_message_key(&key, 0, counter);
            assert_eq!(
                cached.as_bytes(),
                direct.as_bytes(),
                "mismatch at counter {counter}"
            );
        }
    }

    /// A second call for the same (epoch, counter) returns the cached value
    /// without re-deriving it (cache hit).
    #[test]
    fn ratchet_cache_hit_returns_same_key() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(128);

        let first = cache.derive_or_cached(&key, 0, 5);
        let second = cache.derive_or_cached(&key, 0, 5);
        assert_eq!(first.as_bytes(), second.as_bytes());
    }

    /// After warming the cache to counter N, requesting counter N+1 should
    /// cost only one HKDF step, not N+1 steps.
    #[test]
    fn ratchet_cache_advances_incrementally() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(256);

        // Warm to counter 50.
        let _ = cache.derive_or_cached(&key, 0, 50);

        // All intermediate keys should now be cached.
        for c in 1u64..=50 {
            assert!(
                cache.cache.contains_key(&(0, c)),
                "counter {c} should be in cache"
            );
        }

        // Requesting counter 51 should produce the correct key.
        let k51_cached = cache.derive_or_cached(&key, 0, 51);
        let k51_direct = derive_message_key(&key, 0, 51);
        assert_eq!(k51_cached.as_bytes(), k51_direct.as_bytes());
    }

    /// Keys from different epochs must not collide.
    #[test]
    fn ratchet_cache_epoch_isolation() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(128);

        let k_epoch0 = cache.derive_or_cached(&key, 0, 1);
        let k_epoch1 = cache.derive_or_cached(&key, 1, 1);
        assert_ne!(
            k_epoch0.as_bytes(),
            k_epoch1.as_bytes(),
            "keys from different epochs must differ"
        );
    }

    /// `evict_epoch` removes all cached entries for that epoch and lets the
    /// next call start fresh.
    #[test]
    fn ratchet_cache_evict_epoch() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(128);

        // Populate epoch 0 and epoch 1.
        let _ = cache.derive_or_cached(&key, 0, 5);
        let _ = cache.derive_or_cached(&key, 1, 5);

        assert!(cache.cache.contains_key(&(0, 5)));
        assert!(cache.cache.contains_key(&(1, 5)));

        cache.evict_epoch(0);

        assert!(
            !cache.cache.contains_key(&(0, 5)),
            "epoch 0 entry should be evicted"
        );
        assert!(
            cache.cache.contains_key(&(1, 5)),
            "epoch 1 entry should survive"
        );

        // After eviction, derive_or_cached must still return the correct key.
        let fresh = cache.derive_or_cached(&key, 0, 5);
        let direct = derive_message_key(&key, 0, 5);
        assert_eq!(fresh.as_bytes(), direct.as_bytes());
    }

    /// `clear()` must wipe both the message-key cache and the saved
    /// per-epoch ratchet state. Issue #178: without explicit eviction,
    /// derived `ChannelKey` material lingered in `RatchetCache` past the
    /// point where the owning identity / server context was torn down.
    #[test]
    fn ratchet_cache_clear_drops_all_state() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(128);

        // Populate multiple epochs so both `cache` and `ratchet_states`
        // pick up entries.
        let _ = cache.derive_or_cached(&key, 0, 5);
        let _ = cache.derive_or_cached(&key, 1, 7);
        let _ = cache.derive_or_cached(&key, 2, 3);

        assert!(!cache.is_empty(), "cache should be populated before clear");
        assert!(!cache.ratchet_states.is_empty());

        cache.clear();

        assert!(cache.is_empty(), "clear() must empty the message-key cache");
        assert_eq!(cache.len(), 0);
        assert!(
            cache.ratchet_states.is_empty(),
            "clear() must also drop saved ratchet states"
        );

        // The cache must remain functional after clear(): derivations
        // still produce correct keys (matching `derive_message_key`),
        // proving we did not corrupt the cache, only emptied it.
        let post = cache.derive_or_cached(&key, 0, 5);
        let direct = derive_message_key(&key, 0, 5);
        assert_eq!(
            post.as_bytes(),
            direct.as_bytes(),
            "cache must remain usable after clear()"
        );
    }

    /// `clear()` on an already-empty cache is a no-op (idempotent).
    #[test]
    fn ratchet_cache_clear_is_idempotent() {
        let mut cache = RatchetCache::new(64);
        assert!(cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
        cache.clear();
        assert!(cache.is_empty());
    }

    /// The cache must not grow beyond `max_entries`.
    #[test]
    fn ratchet_cache_respects_max_entries() {
        let key = generate_channel_key();
        let max = 10usize;
        let mut cache = RatchetCache::new(max);

        // Derive 20 keys sequentially; the cache should stay at <= max.
        for c in 1u64..=20 {
            let _ = cache.derive_or_cached(&key, 0, c);
            assert!(
                cache.len() <= max,
                "cache size {} exceeded max {max} at counter {c}",
                cache.len()
            );
        }
    }

    /// `is_empty` and `len` report correct values.
    #[test]
    fn ratchet_cache_len_and_is_empty() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(128);

        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);

        let _ = cache.derive_or_cached(&key, 0, 3);
        assert!(!cache.is_empty());
        // deriving counter 3 from scratch advances through counters 1, 2, 3
        assert_eq!(cache.len(), 3);
    }

    /// `RatchetCache` must be `Send + Sync` for use in async contexts.
    #[test]
    fn ratchet_cache_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RatchetCache>();
    }

    /// Second call for the same counter is O(1): it should be at least
    /// 100x faster than the first call (warmup), validating the cache
    /// hit path as described in issue #120.
    #[test]
    fn cached_derive_is_fast_after_warmup() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(2048);

        // Warm the cache up to counter 1_000.
        let warmup_start = std::time::Instant::now();
        let _ = cache.derive_or_cached(&key, 0, 1_000);
        let warmup = warmup_start.elapsed();

        // Repeat derivation — should be essentially free (cache hit).
        let repeat_start = std::time::Instant::now();
        let _ = cache.derive_or_cached(&key, 0, 1_000);
        let repeat = repeat_start.elapsed();

        assert!(
            repeat < warmup / 100,
            "cache hit ({repeat:?}) should be much faster than warmup ({warmup:?})"
        );
    }

    /// `counter = 0` is the "no ratchet" sentinel — must return the channel
    /// key unchanged without advancing the ratchet.
    #[test]
    fn ratchet_cache_counter_zero_returns_channel_key() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(64);
        let result = cache.derive_or_cached(&key, 0, 0);
        assert_eq!(result, key, "counter=0 should return the channel key as-is");
        // Cache should remain empty — no ratchet work done.
        assert!(cache.is_empty());
    }

    /// Requesting a counter lower than the highest previously derived counter
    /// (backwards/out-of-order request) correctly falls back to a fresh
    /// ratchet replay and still returns the right key.
    #[test]
    fn ratchet_cache_backwards_request_returns_correct_key() {
        let key = generate_channel_key();
        let mut cache = RatchetCache::new(256);

        // Advance to counter 50.
        let k50 = cache.derive_or_cached(&key, 0, 50);
        let k50_direct = derive_message_key(&key, 0, 50);
        assert_eq!(k50, k50_direct);

        // Now request counter 10, which is below the saved ratchet state.
        let k10 = cache.derive_or_cached(&key, 0, 10);
        let k10_direct = derive_message_key(&key, 0, 10);
        assert_eq!(k10, k10_direct, "out-of-order request returned wrong key");
    }

    // ── New gap-filling tests ───────────────────────────────────────────────

    /// `EncryptedChannelKey` travels on the wire; verify it survives a
    /// `willow_transport::pack` / `unpack` round-trip and can still be
    /// decrypted after deserialization.
    #[test]
    fn encrypted_channel_key_serde_round_trip() {
        let recipient = Identity::generate();
        let channel_key = generate_channel_key();
        let pub_bytes = recipient_public_bytes(&recipient);

        let encrypted = encrypt_channel_key_for(&channel_key, &pub_bytes).unwrap();

        // Serialize → deserialize.
        let bytes = willow_transport::pack(&encrypted).unwrap();
        let deserialized: EncryptedChannelKey = willow_transport::unpack(&bytes).unwrap();

        // The deserialized value must still decrypt to the original key.
        let decrypted = decrypt_channel_key(&deserialized, &recipient).unwrap();
        assert_eq!(
            decrypted.as_bytes(),
            channel_key.as_bytes(),
            "key recovered from deserialized EncryptedChannelKey must match original"
        );
    }

    /// Flipping a byte in `ephemeral_public` changes the DH shared secret,
    /// so the derived wrapping key is wrong and AEAD decryption must fail.
    #[test]
    fn tampered_ephemeral_public_fails_decryption() {
        let recipient = Identity::generate();
        let channel_key = generate_channel_key();
        let pub_bytes = recipient_public_bytes(&recipient);

        let mut encrypted = encrypt_channel_key_for(&channel_key, &pub_bytes).unwrap();

        // Flip the first byte of the ephemeral public key.
        encrypted.ephemeral_public[0] ^= 0xFF;

        let result = decrypt_channel_key(&encrypted, &recipient);
        assert!(
            matches!(result, Err(CryptoError::DecryptionFailed)),
            "expected DecryptionFailed after ephemeral_public tamper, got {result:?}"
        );
    }

    /// Flipping a byte in the key-wrap `nonce` yields an AEAD failure.
    /// This guards the tampered-nonce path for
    /// `encrypt_channel_key_for` / `decrypt_channel_key`, parallel to the
    /// tampered-ciphertext and tampered-ephemeral-public tests above.
    #[test]
    fn tampered_nonce_fails_key_wrap_decryption() {
        let recipient = Identity::generate();
        let channel_key = generate_channel_key();
        let pub_bytes = recipient_public_bytes(&recipient);

        let mut encrypted = encrypt_channel_key_for(&channel_key, &pub_bytes).unwrap();

        // Flip one byte in the nonce portion of the wire payload.
        encrypted.nonce[0] ^= 0xFF;

        let result = decrypt_channel_key(&encrypted, &recipient);
        assert!(
            matches!(result, Err(CryptoError::DecryptionFailed)),
            "expected DecryptionFailed after nonce tamper, got {result:?}"
        );
    }

    /// `y = 2` (bytes `[2, 0, 0, …, 0]`) has no valid x-coordinate on the
    /// Ed25519 curve, so `ed25519_public_to_x25519` must return
    /// `KeyConversionFailed` rather than panic or succeed with a garbage key.
    #[test]
    fn encrypt_channel_key_for_invalid_public_key() {
        // [2, 0, …, 0] encodes y=2 in compressed Edwards form.  There is no
        // point on curve25519 with y=2, so decompression must fail.
        let mut invalid_pub = [0u8; 32];
        invalid_pub[0] = 2;

        let channel_key = generate_channel_key();
        let result = encrypt_channel_key_for(&channel_key, &invalid_pub);

        assert!(
            matches!(result, Err(CryptoError::KeyConversionFailed)),
            "expected KeyConversionFailed for invalid Ed25519 public key, got {result:?}"
        );
    }
}
