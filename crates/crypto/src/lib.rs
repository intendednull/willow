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

use willow_identity::Identity;

// Re-export for convenience.
pub use willow_messaging::{Content, SealedContent};

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
}

// ───── Types ────────────────────────────────────────────────────────────────

/// A 256-bit symmetric key for encrypting a channel's messages.
#[derive(Clone)]
pub struct ChannelKey(pub(crate) [u8; 32]);

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

// ───── Content Encryption ───────────────────────────────────────────────────

/// Generate a random 256-bit channel key.
pub fn generate_channel_key() -> ChannelKey {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    ChannelKey(bytes)
}

/// Encrypt a [`Content`] value using a channel's symmetric key.
pub fn seal_content(
    content: &Content,
    key: &ChannelKey,
    epoch: u32,
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
    })
}

/// Decrypt a [`SealedContent`] back to a [`Content`] value.
pub fn open_content(sealed: &SealedContent, key: &ChannelKey) -> Result<Content, CryptoError> {
    let cipher = ChaCha20Poly1305::new(key.0.as_ref().into());
    let nonce = Nonce::from_slice(&sealed.nonce);

    let plaintext = cipher
        .decrypt(nonce, sealed.ciphertext.as_ref())
        .map_err(|_| CryptoError::DecryptionFailed)?;

    willow_transport::unpack(&plaintext).map_err(|e| CryptoError::Serialization(e.to_string()))
}

// ───── Key Exchange ─────────────────────────────────────────────────────────

/// Convert an Ed25519 identity to an X25519 static secret.
///
/// The conversion follows RFC 7748: SHA-512 hash the Ed25519 seed, take the
/// first 32 bytes with clamping. This matches what libp2p's Noise does
/// internally for the same keypair.
pub fn identity_to_x25519(identity: &Identity) -> Result<X25519Secret, CryptoError> {
    use sha2::{Digest, Sha512};

    let ed_kp = identity
        .keypair()
        .clone()
        .try_into_ed25519()
        .map_err(|_| CryptoError::KeyConversionFailed)?;
    let seed = &ed_kp.to_bytes()[..32]; // first 32 bytes = Ed25519 seed

    // SHA-512(seed), take first 32 bytes = X25519 secret (clamping done by x25519-dalek).
    let hash = Sha512::digest(seed);
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
    let hk = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut wrapping_key = [0u8; 32];
    hk.expand(b"willow-channel-key-wrap", &mut wrapping_key)
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

    let hk = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
    let mut wrapping_key = [0u8; 32];
    hk.expand(b"willow-channel-key-wrap", &mut wrapping_key)
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

// ───── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use willow_messaging::MessageId;

    fn recipient_public_bytes(identity: &Identity) -> [u8; 32] {
        let ed_kp = identity.keypair().clone().try_into_ed25519().unwrap();
        let full = ed_kp.to_bytes();
        // Public key is last 32 bytes of the 64-byte keypair.
        let mut pub_bytes = [0u8; 32];
        pub_bytes.copy_from_slice(&full[32..]);
        pub_bytes
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
    }
}
