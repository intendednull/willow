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
//! - **[`PeerId`]** — a public identifier derived from the keypair. Safe to
//!   share with anyone.
//! - **[`UserProfile`]** — display name, avatar, status, etc. Attached to a
//!   `PeerId`.
//! - **[`SignedPayload`]** / [`pack`] / [`unpack`] — sign arbitrary data so
//!   that recipients can verify the sender.
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
//! let (msg, peer_id) = unpack::<String>(&signed).unwrap();
//! assert_eq!(msg, "hello from alice");
//! assert_eq!(peer_id, alice.peer_id());
//! ```

use std::sync::Arc;

use chrono::{DateTime, Utc};
use libp2p::identity::{Keypair, PublicKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

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

// ───── PeerId ────────────────────────────────────────────────────────────────

/// A globally unique, cryptographically derived peer identifier.
///
/// Wraps [`libp2p::PeerId`] in an `Arc` so it can be cheaply cloned and shared
/// across async tasks. Implements `Serialize` / `Deserialize` for wire
/// transport.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct PeerId(Arc<libp2p::PeerId>);

impl PeerId {
    /// Return a reference to the inner [`libp2p::PeerId`].
    pub fn inner(&self) -> &libp2p::PeerId {
        &self.0
    }

    /// Parse a base58-encoded PeerId string.
    pub fn parse(s: &str) -> Option<Self> {
        let inner: libp2p::PeerId = s.parse().ok()?;
        Some(Self(Arc::new(inner)))
    }
}

/// Extract Ed25519 public key bytes from a PeerId string.
///
/// Returns `None` if the string is not a valid PeerId or doesn't contain
/// an Ed25519 key.
pub fn ed25519_public_from_peer_id(peer_id_str: &str) -> Option<[u8; 32]> {
    let inner: libp2p::PeerId = peer_id_str.parse().ok()?;
    let digest = inner.as_ref().digest();
    // Ed25519 protobuf encoding: [0x08, 0x01, 0x12, 0x20, ...32 bytes...]
    if digest.len() >= 36
        && digest[0] == 0x08
        && digest[1] == 0x01
        && digest[2] == 0x12
        && digest[3] == 0x20
    {
        let mut key = [0u8; 32];
        key.copy_from_slice(&digest[4..36]);
        Some(key)
    } else {
        None
    }
}

impl std::fmt::Display for PeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<libp2p::PeerId> for PeerId {
    fn from(id: libp2p::PeerId) -> Self {
        Self(Arc::new(id))
    }
}

// ───── Identity ──────────────────────────────────────────────────────────────

/// A local cryptographic identity backed by an Ed25519 keypair.
///
/// This is the "secret" side of your presence on the network — it lets you
/// sign messages to prove they came from you.
///
/// `Identity` is cheap to clone (the keypair lives behind an [`Arc`]) and is
/// `Send + Sync` so it can be shared across tokio tasks.
#[derive(Clone)]
pub struct Identity(Arc<Keypair>);

impl Identity {
    /// Generate a fresh random Ed25519 identity.
    pub fn generate() -> Self {
        Self(Arc::new(Keypair::generate_ed25519()))
    }

    /// Create an identity from raw Ed25519 keypair bytes (64 bytes).
    ///
    /// Returns `None` if the bytes are invalid.
    pub fn from_ed25519_bytes(bytes: &[u8]) -> Option<Self> {
        let mut buf = bytes.to_vec();
        let ed_kp = libp2p::identity::ed25519::Keypair::try_from_bytes(&mut buf).ok()?;
        Some(Self(Arc::new(Keypair::from(ed_kp))))
    }

    /// Export this identity as raw Ed25519 keypair bytes (64 bytes).
    ///
    /// Returns `None` if the keypair is not Ed25519.
    pub fn to_ed25519_bytes(&self) -> Option<Vec<u8>> {
        let ed_kp = (*self.0).clone().try_into_ed25519().ok()?;
        Some(ed_kp.to_bytes().to_vec())
    }

    /// Load an identity from a file, or generate and save a new one.
    #[cfg(not(target_arch = "wasm32"))]
    ///
    /// The file stores the raw 64-byte Ed25519 keypair. Parent directories
    /// are created if they don't exist.
    pub fn load_or_generate(path: impl AsRef<std::path::Path>) -> Result<Self, IdentityError> {
        use std::fs;

        let path = path.as_ref();
        if let Ok(mut bytes) = fs::read(path) {
            let ed_kp = libp2p::identity::ed25519::Keypair::try_from_bytes(&mut bytes)
                .map_err(|e| IdentityError::Other(e.to_string()))?;
            Ok(Self(Arc::new(Keypair::from(ed_kp))))
        } else {
            let identity = Self::generate();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| IdentityError::Other(e.to_string()))?;
            }
            let kp: Keypair = (*identity.0).clone();
            let ed_kp = kp
                .try_into_ed25519()
                .map_err(|e| IdentityError::Other(e.to_string()))?;
            fs::write(path, ed_kp.to_bytes()).map_err(|e| IdentityError::Other(e.to_string()))?;
            Ok(identity)
        }
    }

    /// Derive the public [`PeerId`] for this identity.
    pub fn peer_id(&self) -> PeerId {
        PeerId::from(self.0.public().to_peer_id())
    }

    /// Access the underlying [`Keypair`] (e.g. for configuring libp2p).
    pub fn keypair(&self) -> &Keypair {
        &self.0
    }

    /// Access the public key.
    pub fn public_key(&self) -> PublicKey {
        self.0.public()
    }
}

impl Default for Identity {
    fn default() -> Self {
        Self::generate()
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Identity").field(&self.peer_id()).finish()
    }
}

// ───── User profile ──────────────────────────────────────────────────────────

/// A human-readable profile attached to a [`PeerId`].
///
/// Profiles are gossiped across the network so that peers can show display
/// names and avatars instead of raw peer IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfile {
    /// The peer this profile belongs to.
    pub peer_id: PeerId,
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
    pub fn new(peer_id: PeerId, display_name: impl Into<String>) -> Self {
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

// ───── Signed message envelope ───────────────────────────────────────────────

/// Internal wire format for a signed payload.
#[derive(Serialize, Deserialize)]
struct SignedMessage {
    /// The signer's public key in protobuf encoding.
    public_key: Vec<u8>,
    /// Ed25519 signature over `payload`.
    signature: Vec<u8>,
    /// The serialized inner data.
    payload: Vec<u8>,
}

impl SignedMessage {
    /// Verify the signature and return the signer's [`libp2p::PeerId`].
    fn verify(&self) -> Result<libp2p::PeerId, IdentityError> {
        let public_key = PublicKey::try_decode_protobuf(&self.public_key)
            .map_err(|e| IdentityError::PublicKeyDecode(e.to_string()))?;

        if public_key.verify(&self.payload, &self.signature) {
            Ok(public_key.to_peer_id())
        } else {
            Err(IdentityError::InvalidSignature)
        }
    }
}

// ───── Public API ────────────────────────────────────────────────────────────

/// Sign and serialize `payload` using the given [`Identity`].
///
/// The returned bytes contain the serialized data, the Ed25519 signature, and
/// the signer's public key — everything a recipient needs to verify
/// authenticity via [`unpack`].
///
/// # Errors
///
/// Returns [`IdentityError::Serde`] if serialization fails, or
/// [`IdentityError::SignError`] if the keypair cannot produce a signature.
pub fn pack<T: Serialize>(payload: &T, identity: &Identity) -> Result<Vec<u8>, IdentityError> {
    let payload_bytes =
        willow_transport::pack(payload).map_err(|e| IdentityError::Serde(e.to_string()))?;

    let signature = identity
        .0
        .sign(&payload_bytes)
        .map_err(|e| IdentityError::SignError(e.to_string()))?;

    let message = SignedMessage {
        public_key: identity.public_key().encode_protobuf(),
        signature,
        payload: payload_bytes,
    };

    willow_transport::pack(&message).map_err(|e| IdentityError::Serde(e.to_string()))
}

/// Verify the signature on `data` and deserialize the inner payload.
///
/// Returns both the deserialized value and the [`PeerId`] of the signer, so
/// the caller can check *who* sent the message.
///
/// # Errors
///
/// Returns an error if the bytes are malformed, the signature is invalid, or
/// the inner payload can't be deserialized into `T`.
pub fn unpack<T: DeserializeOwned>(data: &[u8]) -> Result<(T, PeerId), IdentityError> {
    let message: SignedMessage =
        willow_transport::unpack(data).map_err(|e| IdentityError::Serde(e.to_string()))?;

    let libp2p_peer = message.verify()?;
    let payload: T = willow_transport::unpack(&message.payload)
        .map_err(|e| IdentityError::Serde(e.to_string()))?;

    Ok((payload, PeerId::from(libp2p_peer)))
}

// ───── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_identity_is_unique() {
        let a = Identity::generate();
        let b = Identity::generate();
        assert_ne!(a.peer_id(), b.peer_id());
    }

    #[test]
    fn peer_id_round_trip_serde() {
        let id = Identity::generate().peer_id();
        let bytes = willow_transport::pack(&id).unwrap();
        let decoded: PeerId = willow_transport::unpack(&bytes).unwrap();
        assert_eq!(decoded, id);
    }

    #[test]
    fn pack_and_unpack_verifies_signature() {
        let alice = Identity::generate();
        let payload = "hello from alice";

        let data = pack(&payload, &alice).unwrap();
        let (msg, peer) = unpack::<String>(&data).unwrap();

        assert_eq!(msg, payload);
        assert_eq!(peer, alice.peer_id());
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
        let peer = Identity::generate().peer_id();
        let profile = UserProfile::new(peer.clone(), "Alice");

        assert_eq!(profile.peer_id, peer);
        assert_eq!(profile.display_name, "Alice");
        assert!(profile.avatar.is_none());
        assert!(profile.status.is_none());
        assert!(profile.bio.is_none());
    }

    #[test]
    fn user_profile_serde_round_trip() {
        let peer = Identity::generate().peer_id();
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
        assert_send_sync::<PeerId>();
    }

    #[test]
    fn peer_id_display() {
        let peer = Identity::generate().peer_id();
        let display = format!("{peer}");
        assert!(!display.is_empty());
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

        assert_eq!(id1.peer_id(), id2.peer_id());

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_ed25519_bytes_round_trip() {
        let id = Identity::generate();
        let bytes = id.to_ed25519_bytes().expect("should export bytes");
        let restored = Identity::from_ed25519_bytes(&bytes).expect("should restore");
        assert_eq!(restored.peer_id(), id.peer_id());
    }

    #[test]
    fn from_ed25519_bytes_invalid_returns_none() {
        assert!(Identity::from_ed25519_bytes(&[0u8; 10]).is_none());
        assert!(Identity::from_ed25519_bytes(&[]).is_none());
        assert!(Identity::from_ed25519_bytes(&[0xFF; 64]).is_none());
    }

    #[test]
    fn to_ed25519_bytes_length() {
        let id = Identity::generate();
        let bytes = id.to_ed25519_bytes().unwrap();
        assert_eq!(bytes.len(), 64); // Ed25519 keypair = 32 seed + 32 public
    }

    #[test]
    fn user_profile_all_fields() {
        let peer = Identity::generate().peer_id();
        let mut profile = UserProfile::new(peer.clone(), "Alice");
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
    fn ed25519_public_from_peer_id_round_trip() {
        let id = Identity::generate();
        let peer_str = id.peer_id().to_string();
        let pub_bytes = ed25519_public_from_peer_id(&peer_str).unwrap();

        // Compare with what we get from the keypair directly.
        let ed_kp = id.keypair().clone().try_into_ed25519().unwrap();
        let full = ed_kp.to_bytes();
        let mut expected = [0u8; 32];
        expected.copy_from_slice(&full[32..]);

        assert_eq!(pub_bytes, expected);
    }

    #[test]
    fn ed25519_public_from_peer_id_invalid() {
        assert!(ed25519_public_from_peer_id("not-a-peer-id").is_none());
        assert!(ed25519_public_from_peer_id("").is_none());
    }
}
