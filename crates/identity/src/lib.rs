//! # Willow Identity
//!
//! Cryptographic identity management for the Willow P2P network.
//!
//! Every participant in a Willow network has an [`Identity`] — an Ed25519
//! keypair that uniquely identifies them and lets them sign messages so that
//! other peers can verify authenticity.
//!
//! ## Key concepts
//!
//! - **[`Identity`]** — your secret keypair. Never leaves the local machine.
//! - **[`EndpointId`]** — a public identifier (= Ed25519 public key). Safe to
//!   share with anyone. Used as the peer address throughout the codebase.
//! - **[`UserProfile`]** — display name, avatar, status, etc. Attached to an
//!   `EndpointId`.
//! - **[`pack`] / [`unpack`]** — sign arbitrary data so that recipients can
//!   verify the sender.
//!
//! ## Examples
//!
//! ```
//! use willow_identity::{Identity, pack, unpack};
//!
//! let alice = Identity::generate();
//! let data = String::from("hello from alice");
//! let signed = pack(&data, &alice).unwrap();
//!
//! let (msg, endpoint_id) = unpack::<String>(&signed).unwrap();
//! assert_eq!(msg, "hello from alice");
//! assert_eq!(endpoint_id, alice.endpoint_id());
//! ```

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

// Re-export iroh identity types so downstream crates can use them
// without depending on iroh-base directly.
pub use iroh_base::{EndpointId, PublicKey, SecretKey, Signature};

// ───── Errors ────────────────────────────────────────────────────────────────

/// Errors produced by identity operations.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// Serialization or deserialization failed.
    #[error("serialization failed: {0}")]
    Serde(String),

    /// A cryptographic signature did not verify.
    #[error("invalid signature")]
    InvalidSignature,

    /// The private key could not produce a signature.
    #[error("signing failed: {0}")]
    SignError(String),

    /// A public key could not be decoded from its wire format.
    #[error("failed to decode public key: {0}")]
    PublicKeyDecode(String),

    /// An I/O or other error.
    #[error("{0}")]
    Other(String),
}

// ───── Identity ──────────────────────────────────────────────────────────────

/// A local cryptographic identity backed by an Ed25519 keypair.
///
/// This is the "secret" side of your presence on the network — it lets you
/// sign messages to prove they came from you.
///
/// `Identity` is cheap to clone (the secret key is 32 bytes, copied on clone)
/// and is `Send + Sync` so it can be shared across tokio tasks.
#[derive(Clone)]
pub struct Identity {
    secret_key: SecretKey,
}

impl Identity {
    /// Generate a fresh random Ed25519 identity.
    pub fn generate() -> Self {
        Self {
            secret_key: SecretKey::generate(&mut rand::rng()),
        }
    }

    /// Create an identity from raw Ed25519 secret key bytes (32 bytes).
    ///
    /// Returns `None` if the bytes are not exactly 32 bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes: [u8; 32] = bytes.try_into().ok()?;
        Some(Self {
            secret_key: SecretKey::from_bytes(&bytes),
        })
    }

    /// Export this identity as raw Ed25519 secret key bytes (32 bytes).
    pub fn to_bytes(&self) -> Vec<u8> {
        self.secret_key.to_bytes().to_vec()
    }

    /// Derive the public [`EndpointId`] for this identity.
    ///
    /// This is the peer's address on the network — a 32-byte Ed25519 public key.
    pub fn endpoint_id(&self) -> EndpointId {
        self.secret_key.public()
    }

    /// Access the underlying [`SecretKey`] (e.g. for configuring iroh endpoints).
    pub fn secret_key(&self) -> &SecretKey {
        &self.secret_key
    }

    /// Access the public key.
    pub fn public_key(&self) -> PublicKey {
        self.secret_key.public()
    }

    /// Sign arbitrary data with this identity's secret key.
    pub fn sign(&self, data: &[u8]) -> Signature {
        self.secret_key.sign(data)
    }

    /// Load an identity from a file, or generate and save a new one.
    ///
    /// The file stores the raw 32-byte Ed25519 secret key. Parent directories
    /// are created if they don't exist.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_or_generate(path: impl AsRef<std::path::Path>) -> Result<Self, IdentityError> {
        use std::fs;

        let path = path.as_ref();
        if let Ok(bytes) = fs::read(path) {
            Self::from_bytes(&bytes).ok_or_else(|| {
                IdentityError::Other(format!(
                    "invalid key file: expected 32 bytes, got {}",
                    bytes.len()
                ))
            })
        } else {
            let identity = Self::generate();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| IdentityError::Other(e.to_string()))?;
            }
            fs::write(path, identity.to_bytes())
                .map_err(|e| IdentityError::Other(e.to_string()))?;
            Ok(identity)
        }
    }
}

impl Default for Identity {
    fn default() -> Self {
        Self::generate()
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Identity")
            .field(&self.endpoint_id())
            .finish()
    }
}

// ───── Standalone verification ──────────────────────────────────────────────

/// Verify a signature against a public key without needing an [`Identity`].
pub fn verify(key: &PublicKey, data: &[u8], sig: &Signature) -> bool {
    key.verify(data, sig).is_ok()
}

// ───── User profile ─────────────────────────────────────────────────────────

/// A human-readable profile attached to an [`EndpointId`].
///
/// Profiles are gossiped across the network so that peers can show display
/// names and avatars instead of raw endpoint IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    /// The peer this profile belongs to.
    pub peer_id: EndpointId,
    /// Display name shown in the UI.
    pub display_name: String,
    /// Optional avatar (URL or content-addressed hash).
    pub avatar: Option<String>,
    /// Free-text status line (e.g. "Away", "In a meeting").
    pub status: Option<String>,
    /// Short bio.
    pub bio: Option<String>,
    /// When this profile was last updated.
    pub updated_at: DateTime<Utc>,
}

impl UserProfile {
    /// Create a minimal profile with just a display name.
    pub fn new(peer_id: EndpointId, display_name: impl Into<String>) -> Self {
        Self {
            peer_id,
            display_name: display_name.into(),
            avatar: None,
            status: None,
            bio: None,
            updated_at: Utc::now(),
        }
    }
}

// ───── Signed message envelope ──────────────────────────────────────────────

/// Internal wire format for a signed payload.
#[derive(Serialize, Deserialize)]
struct SignedMessage {
    /// The signer's Ed25519 public key (32 bytes).
    public_key: Vec<u8>,
    /// Ed25519 signature over `payload` (64 bytes).
    signature: Vec<u8>,
    /// The serialized inner data.
    payload: Vec<u8>,
}

impl SignedMessage {
    /// Verify the signature and return the signer's [`PublicKey`].
    fn verify(&self) -> Result<PublicKey, IdentityError> {
        let pk_bytes: [u8; 32] = self
            .public_key
            .as_slice()
            .try_into()
            .map_err(|_| IdentityError::PublicKeyDecode("expected 32 bytes".into()))?;
        let public_key = PublicKey::from_bytes(&pk_bytes)
            .map_err(|e| IdentityError::PublicKeyDecode(e.to_string()))?;

        let sig_bytes: [u8; 64] = self
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| IdentityError::InvalidSignature)?;
        let signature = Signature::from_bytes(&sig_bytes);

        if public_key.verify(&self.payload, &signature).is_ok() {
            Ok(public_key)
        } else {
            Err(IdentityError::InvalidSignature)
        }
    }
}

// ───── Public API ───────────────────────────────────────────────────────────

/// Sign and serialize `payload` using the given [`Identity`].
///
/// The returned bytes contain the serialized data, the Ed25519 signature, and
/// the signer's public key — everything a recipient needs to verify
/// authenticity via [`unpack`].
///
/// # Errors
///
/// Returns [`IdentityError::Serde`] if serialization fails.
pub fn pack<T: Serialize>(payload: &T, identity: &Identity) -> Result<Vec<u8>, IdentityError> {
    let payload_bytes =
        willow_transport::pack(payload).map_err(|e| IdentityError::Serde(e.to_string()))?;

    let signature = identity.sign(&payload_bytes);

    let message = SignedMessage {
        public_key: identity.public_key().as_bytes().to_vec(),
        signature: signature.to_bytes().to_vec(),
        payload: payload_bytes,
    };

    willow_transport::pack(&message).map_err(|e| IdentityError::Serde(e.to_string()))
}

/// Verify the signature on `data` and deserialize the inner payload.
///
/// Returns both the deserialized value and the [`EndpointId`] of the signer,
/// so the caller can check *who* sent the message.
///
/// # Errors
///
/// Returns an error if the bytes are malformed, the signature is invalid, or
/// the inner payload can't be deserialized into `T`.
pub fn unpack<T: DeserializeOwned>(data: &[u8]) -> Result<(T, EndpointId), IdentityError> {
    let message: SignedMessage =
        willow_transport::unpack(data).map_err(|e| IdentityError::Serde(e.to_string()))?;

    let public_key = message.verify()?;
    let payload: T = willow_transport::unpack(&message.payload)
        .map_err(|e| IdentityError::Serde(e.to_string()))?;

    Ok((payload, public_key))
}

// ───── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_identity_is_unique() {
        let a = Identity::generate();
        let b = Identity::generate();
        assert_ne!(a.endpoint_id(), b.endpoint_id());
    }

    #[test]
    fn endpoint_id_round_trip_serde() {
        let id = Identity::generate().endpoint_id();
        let bytes = willow_transport::pack(&id).unwrap();
        let decoded: EndpointId = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, id);
    }

    #[test]
    fn pack_and_unpack_verifies_signature() {
        let alice = Identity::generate();
        let payload = "hello from alice";

        let data = pack(&payload, &alice).unwrap();
        let (msg, endpoint) = unpack::<String>(&data).unwrap();

        assert_eq!(msg, payload);
        assert_eq!(endpoint, alice.endpoint_id());
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let alice = Identity::generate();
        let data = pack(&"original", &alice).unwrap();

        // Flip a byte near the end (inside the payload region).
        let mut tampered = data.clone();
        let len = tampered.len();
        tampered[len - 2] ^= 0xFF;

        // Should fail to deserialize or verify.
        let result = unpack::<String>(&tampered);
        assert!(result.is_err());
    }

    #[test]
    fn user_profile_new() {
        let peer = Identity::generate().endpoint_id();
        let profile = UserProfile::new(peer, "Alice");

        assert_eq!(profile.peer_id, peer);
        assert_eq!(profile.display_name, "Alice");
        assert!(profile.avatar.is_none());
        assert!(profile.status.is_none());
        assert!(profile.bio.is_none());
    }

    #[test]
    fn user_profile_serde_round_trip() {
        let peer = Identity::generate().endpoint_id();
        let mut profile = UserProfile::new(peer, "Bob");
        profile.status = Some("Online".into());
        profile.bio = Some("Just a test user".into());

        let bytes = willow_transport::pack(&profile).unwrap();
        let decoded: UserProfile = willow_transport::unpack(&bytes).unwrap();

        assert_eq!(decoded, profile);
    }

    #[test]
    fn identity_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Identity>();
        assert_send_sync::<EndpointId>();
    }

    #[test]
    fn endpoint_id_display() {
        let endpoint = Identity::generate().endpoint_id();
        let display = format!("{endpoint}");
        assert!(!display.is_empty());
        // EndpointId displays as 64-char hex string
        assert_eq!(display.len(), 64);
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn load_or_generate_persists_identity() {
        let dir = std::env::temp_dir().join(format!(
            "willow-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("identity.key");

        // First call: generates and saves.
        let id1 = Identity::load_or_generate(&path).unwrap();

        // Second call: loads the same identity.
        let id2 = Identity::load_or_generate(&path).unwrap();

        assert_eq!(id1.endpoint_id(), id2.endpoint_id());

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_bytes_round_trip() {
        let id = Identity::generate();
        let bytes = id.to_bytes();
        let restored = Identity::from_bytes(&bytes).expect("should restore");
        assert_eq!(restored.endpoint_id(), id.endpoint_id());
    }

    #[test]
    fn from_bytes_invalid_returns_none() {
        // Wrong length
        assert!(Identity::from_bytes(&[0u8; 10]).is_none());
        assert!(Identity::from_bytes(&[]).is_none());
    }

    #[test]
    fn to_bytes_length() {
        let id = Identity::generate();
        let bytes = id.to_bytes();
        assert_eq!(bytes.len(), 32); // Ed25519 secret key = 32 bytes
    }

    #[test]
    fn user_profile_all_fields() {
        let peer = Identity::generate().endpoint_id();
        let mut profile = UserProfile::new(peer, "Alice");
        profile.avatar = Some("https://example.com/avatar.png".into());
        profile.status = Some("Online".into());
        profile.bio = Some("Willow developer".into());

        let bytes = willow_transport::pack(&profile).unwrap();
        let decoded: UserProfile = willow_transport::unpack(&bytes).unwrap();

        assert_eq!(decoded.display_name, "Alice");
        assert_eq!(
            decoded.avatar.as_deref(),
            Some("https://example.com/avatar.png")
        );
        assert_eq!(decoded.status.as_deref(), Some("Online"));
        assert_eq!(decoded.bio.as_deref(), Some("Willow developer"));
        assert_eq!(decoded.peer_id, peer);
    }

    #[test]
    fn sign_and_verify_standalone() {
        let id = Identity::generate();
        let data = b"test data";
        let sig = id.sign(data);

        assert!(verify(&id.public_key(), data, &sig));
        assert!(!verify(&id.public_key(), b"wrong data", &sig));
    }

    #[test]
    fn endpoint_id_hash_map_key() {
        use std::collections::HashMap;
        let a = Identity::generate().endpoint_id();
        let b = Identity::generate().endpoint_id();

        let mut map = HashMap::new();
        map.insert(a, "alice");
        map.insert(b, "bob");

        assert_eq!(map.get(&a), Some(&"alice"));
        assert_eq!(map.get(&b), Some(&"bob"));
        assert_eq!(map.len(), 2);
    }
}
