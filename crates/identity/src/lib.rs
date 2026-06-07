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
use zeroize::{Zeroize, ZeroizeOnDrop};

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

    /// An on-disk key file has loose permissions (readable by group or
    /// other). Refuse to load it rather than silently use a leaked key.
    #[error(
        "refusing to load key file {path} with insecure permissions {mode:#o} \
         (must not be readable or writable by group/other; expected mode 0o600)"
    )]
    InsecurePermissions {
        /// Filesystem path of the offending key file.
        path: String,
        /// The Unix mode bits of the file.
        mode: u32,
    },

    /// A signed [`UserProfile`] claimed a `peer_id` that did not match
    /// the key that actually signed the envelope. Indicates an attempted
    /// display-name spoof.
    #[error("profile peer_id mismatch: signer {signer} does not match claimed peer_id {claimed}")]
    PeerMismatch {
        /// The peer ID the profile claimed to belong to.
        claimed: EndpointId,
        /// The peer ID that actually signed the envelope.
        signer: EndpointId,
    },

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
///
/// The wrapped [`SecretKey`] is itself `ZeroizeOnDrop` (provided by
/// `iroh-base`), so the secret material is wiped from memory when the
/// last clone is dropped. We derive [`ZeroizeOnDrop`] on `Identity`
/// itself with `#[zeroize(skip)]` so the type-level guarantee is
/// visible to downstream crates and will fail to compile if the inner
/// representation ever changes to a non-zeroizing type.
#[derive(Clone, ZeroizeOnDrop)]
pub struct Identity {
    #[zeroize(skip)]
    secret_key: SecretKey,
}

impl Identity {
    /// Generate a fresh random Ed25519 identity.
    pub fn generate() -> Self {
        Self {
            secret_key: SecretKey::generate(),
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
    ///
    /// **Security:** the returned `Vec<u8>` contains live secret key
    /// material. The buffer is *not* automatically zeroized — callers
    /// are responsible for wiping it as soon as they're done with it
    /// (e.g. by passing it through [`zeroize::Zeroize::zeroize`] on a
    /// mutable binding before drop, or by holding it in a
    /// `zeroize::Zeroizing<Vec<u8>>` wrapper).
    ///
    /// Prefer [`Identity::with_secret_bytes`] when possible: it
    /// confines the exposed bytes to the closure's scope and zeroizes
    /// them automatically.
    #[must_use = "the returned Vec<u8> contains secret key material; \
                  zeroize it after use, or prefer Identity::with_secret_bytes"]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.secret_key.to_bytes().to_vec()
    }

    /// Run a closure with temporary access to this identity's raw 32-byte
    /// Ed25519 secret key, then zeroize the buffer before returning.
    ///
    /// This is the preferred way to read the secret bytes — the buffer
    /// never escapes the closure and is wiped from memory automatically,
    /// so callers can't accidentally leave secret material lying around.
    pub fn with_secret_bytes<R>(&self, f: impl FnOnce(&[u8; 32]) -> R) -> R {
        let mut bytes = self.secret_key.to_bytes();
        let result = f(&bytes);
        bytes.zeroize();
        result
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
    /// The file stores the raw 32-byte Ed25519 secret key. Parent
    /// directories are created if they don't exist.
    ///
    /// On Unix the key file is created with mode `0o600` (owner-only
    /// read/write) via an atomic temp-file + rename so a crash during
    /// write can't leave a half-written key on disk. When loading an
    /// existing file, the mode is checked: if any group/other bits are
    /// set, [`IdentityError::InsecurePermissions`] is returned rather
    /// than silently using a leaked key.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load_or_generate(path: impl AsRef<std::path::Path>) -> Result<Self, IdentityError> {
        use std::fs;

        let path = path.as_ref();
        match fs::metadata(path) {
            Ok(metadata) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = metadata.permissions().mode();
                    // Reject any group/other access (the bottom 6 bits).
                    if mode & 0o077 != 0 {
                        return Err(IdentityError::InsecurePermissions {
                            path: path.display().to_string(),
                            mode,
                        });
                    }
                }
                // `metadata` is consulted only for the perm check; the
                // unused binding under non-unix targets is fine.
                #[cfg(not(unix))]
                let _ = metadata;

                let bytes = fs::read(path).map_err(|e| IdentityError::Other(e.to_string()))?;
                Self::from_bytes(&bytes).ok_or_else(|| {
                    IdentityError::Other(format!(
                        "invalid key file: expected 32 bytes, got {}",
                        bytes.len()
                    ))
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let identity = Self::generate();
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|e| IdentityError::Other(e.to_string()))?;
                }
                Self::atomic_write_key(path, &identity)?;
                Ok(identity)
            }
            Err(e) => Err(IdentityError::Other(e.to_string())),
        }
    }

    /// Atomically write `identity`'s secret bytes to `path`.
    ///
    /// Writes to a uniquely-named sibling temp file (created with
    /// `O_CREAT | O_EXCL` so a concurrent startup race cannot clobber
    /// it), `fsync`s on Unix, sets mode `0o600`, and `rename`s into
    /// place. The rename is atomic on POSIX filesystems, so a crash at
    /// any point either leaves the previous key intact or installs the
    /// new one — never a truncated half-write.
    #[cfg(not(target_arch = "wasm32"))]
    fn atomic_write_key(path: &std::path::Path, identity: &Identity) -> Result<(), IdentityError> {
        use std::fs::{self, OpenOptions};
        use std::io::Write;

        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let file_name = path
            .file_name()
            .ok_or_else(|| IdentityError::Other("key path has no file name".into()))?;

        // Pick a sibling temp path. Including the PID + a nanosecond
        // counter makes accidental collisions astronomically unlikely
        // and the `create_new(true)` open below catches the rest.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut tmp_name = std::ffi::OsString::from(".");
        tmp_name.push(file_name);
        tmp_name.push(format!(".tmp.{}.{}", std::process::id(), nanos));
        let tmp_path = parent.join(tmp_name);

        let mut open_opts = OpenOptions::new();
        open_opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open_opts.mode(0o600);
        }
        let mut file = open_opts
            .open(&tmp_path)
            .map_err(|e| IdentityError::Other(format!("open temp key file: {e}")))?;

        // Write the secret bytes through `with_secret_bytes` so the
        // intermediate buffer is zeroized as soon as the write returns.
        let write_result =
            identity.with_secret_bytes(|bytes| -> std::io::Result<()> { file.write_all(bytes) });
        if let Err(e) = write_result {
            fs::remove_file(&tmp_path).ok();
            return Err(IdentityError::Other(format!("write temp key file: {e}")));
        }
        if let Err(e) = file.sync_all() {
            fs::remove_file(&tmp_path).ok();
            return Err(IdentityError::Other(format!("fsync temp key file: {e}")));
        }
        // Drop the file handle before rename — Windows requires it,
        // POSIX doesn't care.
        drop(file);

        // On Unix, belt-and-braces: enforce mode 0o600 even if the
        // umask was unusual when we opened the file.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600)) {
                fs::remove_file(&tmp_path).ok();
                return Err(IdentityError::Other(format!("chmod temp key file: {e}")));
            }
        }

        if let Err(e) = fs::rename(&tmp_path, path) {
            fs::remove_file(&tmp_path).ok();
            return Err(IdentityError::Other(format!("rename key file: {e}")));
        }
        Ok(())
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
///
/// This delegates to [`PublicKey::verify`] from `iroh-base`, which internally
/// uses `ed25519_dalek::VerifyingKey::verify_strict` (RFC 8032 strict mode).
/// Strict verification rejects non-canonical signature encodings — in particular,
/// signatures whose `S` component is not reduced modulo the curve order — which
/// closes off Ed25519 signature malleability vectors. Defense in depth: any
/// future change that bypasses `iroh-base`'s wrapper must continue to use the
/// strict primitive.
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

// ───── Signed profile helpers ───────────────────────────────────────────────

/// Sign a [`UserProfile`] for broadcast.
///
/// The profile is wrapped in an Ed25519-signed envelope via [`pack`]. Peers
/// that receive the bytes must use [`unpack_profile`] to verify both the
/// signature and that the signer's `EndpointId` matches the `peer_id` field
/// inside the profile — otherwise a malicious peer could broadcast a profile
/// claiming any `peer_id` and spoof that peer's display name.
///
/// # Errors
///
/// Returns [`IdentityError::Serde`] if serialization fails.
pub fn pack_profile(profile: &UserProfile, identity: &Identity) -> Result<Vec<u8>, IdentityError> {
    pack(profile, identity)
}

/// Verify a signed [`UserProfile`] envelope and confirm the signer matches
/// the claimed `peer_id`.
///
/// Callers should always use this (rather than [`unpack`] directly) for
/// profile broadcasts so spoofed `peer_id`s are rejected.
///
/// # Errors
///
/// - [`IdentityError::Serde`] if the bytes are malformed.
/// - [`IdentityError::InvalidSignature`] if the signature doesn't verify.
/// - [`IdentityError::PeerMismatch`] if the verified signer does not match
///   `profile.peer_id`.
pub fn unpack_profile(data: &[u8]) -> Result<UserProfile, IdentityError> {
    let (profile, signer) = unpack::<UserProfile>(data)?;
    if signer != profile.peer_id {
        return Err(IdentityError::PeerMismatch {
            claimed: profile.peer_id,
            signer,
        });
    }
    Ok(profile)
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");

        // First call: generates and saves.
        let id1 = Identity::load_or_generate(&path).unwrap();

        // Second call: loads the same identity.
        let id2 = Identity::load_or_generate(&path).unwrap();

        assert_eq!(id1.endpoint_id(), id2.endpoint_id());
    }

    /// Issue #126: a freshly generated key file must be created with
    /// mode `0o600` so it isn't readable by other users on the system.
    #[test]
    #[cfg(unix)]
    fn load_or_generate_creates_file_with_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");

        let _id = Identity::load_or_generate(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        // Strip the file-type bits — we only care about the perm bits.
        assert_eq!(
            mode & 0o777,
            0o600,
            "expected key file mode 0o600, got {:#o}",
            mode & 0o777
        );
    }

    /// Issue #126: loading a pre-existing key file with loose
    /// permissions (anything readable by group or other) must fail
    /// loudly with [`IdentityError::InsecurePermissions`] rather than
    /// silently use the leaked key.
    #[test]
    #[cfg(unix)]
    fn load_existing_with_loose_perms_returns_insecure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");

        // Plant a key file with mode 0o644 (world-readable).
        let id = Identity::generate();
        let bytes = id.to_bytes();
        std::fs::write(&path, &bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        // Wipe the temporary buffer; the file on disk is what matters.
        let mut bytes = bytes;
        zeroize::Zeroize::zeroize(&mut bytes);

        let err = Identity::load_or_generate(&path).unwrap_err();
        match err {
            IdentityError::InsecurePermissions { path: p, mode } => {
                assert_eq!(p, path.display().to_string());
                assert_eq!(mode & 0o777, 0o644);
            }
            other => panic!("expected InsecurePermissions, got {other:?}"),
        }
    }

    /// Issue #126: a key written by `load_or_generate` must round-trip
    /// cleanly through a second call (the perm check on the just-written
    /// 0o600 file must succeed).
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn load_existing_with_secure_perms_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.key");

        let id1 = Identity::load_or_generate(&path).unwrap();
        let id2 = Identity::load_or_generate(&path).unwrap();
        assert_eq!(id1.endpoint_id(), id2.endpoint_id());
    }

    /// Issue #127: type-level guarantee that [`Identity`] (and the
    /// [`willow_crypto::ChannelKey`] referenced from the issue) zeroize
    /// their secret material on drop. This will fail to compile if
    /// `Identity` ever loses its `ZeroizeOnDrop` derive or grows a new
    /// secret-bearing field that doesn't zeroize.
    #[test]
    fn identity_is_zeroize_on_drop() {
        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<Identity>();
    }

    /// Issue #127: `with_secret_bytes` exposes the raw 32-byte secret
    /// to a closure and zeroizes the buffer afterwards.
    #[test]
    fn with_secret_bytes_exposes_full_secret() {
        let id = Identity::generate();
        // Both of these hold raw secret bytes; wrap them in `Zeroizing`
        // so the test itself doesn't leak unzeroized copies on the heap.
        let from_to_bytes = zeroize::Zeroizing::new(id.to_bytes());
        let from_callback = zeroize::Zeroizing::new(id.with_secret_bytes(|bytes| bytes.to_vec()));
        assert_eq!(*from_to_bytes, *from_callback);
        assert_eq!(from_callback.len(), 32);
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

    /// Regression: `verify` must use strict Ed25519 verification (RFC 8032),
    /// rejecting signatures whose `S` component is non-canonical (S ≥ ℓ).
    ///
    /// Setting the top bit of byte 63 of `S` makes `S` larger than the curve
    /// order ℓ ≈ 2^252 + …, so non-strict verification might accept it while
    /// strict verification must reject it. This pins the call-site invariant:
    /// the underlying primitive is `verify_strict`, providing defense in depth
    /// against signature malleability.
    #[test]
    fn verify_rejects_non_canonical_s_component() {
        let id = Identity::generate();
        let data = b"test data";
        let sig = id.sign(data);

        // Original signature is valid.
        assert!(verify(&id.public_key(), data, &sig));

        // Mutate the S scalar's high bit. S occupies bytes 32..64 of the
        // signature; its most-significant byte is at index 63. Flipping the top
        // bit pushes S above the curve order, producing a non-canonical encoding
        // that strict verification rejects.
        let mut bytes = sig.to_bytes();
        bytes[63] ^= 0x80;
        let mutated = Signature::from_bytes(&bytes);

        assert!(
            !verify(&id.public_key(), data, &mutated),
            "verify must reject non-canonical S (signature malleability vector)"
        );
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

    /// A message signed by Alice unpacks with Alice's EndpointId, not Bob's.
    ///
    /// This confirms that signing attribution is correct — two different
    /// identities cannot be confused by the verify/recover path.
    #[test]
    fn signature_from_alice_does_not_verify_as_bob() {
        let alice = Identity::generate();
        let bob = Identity::generate();

        // Ensure they are genuinely different identities.
        assert_ne!(alice.endpoint_id(), bob.endpoint_id());

        let signed = pack(&String::from("hello from alice"), &alice).unwrap();
        let (msg, recovered_id) = unpack::<String>(&signed).unwrap();

        assert_eq!(msg, "hello from alice");
        // The recovered signer must be Alice, not Bob.
        assert_eq!(recovered_id, alice.endpoint_id());
        assert_ne!(recovered_id, bob.endpoint_id());
    }

    /// Passing an empty byte slice to `unpack` returns an error, not a panic.
    #[test]
    fn unpack_empty_bytes_returns_none() {
        let result = unpack::<String>(&[]);
        assert!(
            result.is_err(),
            "unpack of empty bytes should return an error"
        );
    }

    /// Issue #145: a profile signed by its own peer must round-trip
    /// through [`pack_profile`] / [`unpack_profile`] and return the
    /// original profile.
    #[test]
    fn pack_and_unpack_profile_accepts_self_signed() {
        let alice = Identity::generate();
        let profile = UserProfile::new(alice.endpoint_id(), "Alice");

        let data = pack_profile(&profile, &alice).unwrap();
        let decoded = unpack_profile(&data).unwrap();

        assert_eq!(decoded, profile);
        assert_eq!(decoded.peer_id, alice.endpoint_id());
        assert_eq!(decoded.display_name, "Alice");
    }

    /// Issue #145: a profile signed by a different key than the one it
    /// claims to represent must be rejected with
    /// [`IdentityError::PeerMismatch`]. Without this check, any peer
    /// could broadcast a profile claiming another peer's `peer_id` and
    /// spoof their display name.
    #[test]
    fn unpack_profile_rejects_spoofed_peer_id() {
        let alice = Identity::generate();
        let mallory = Identity::generate();
        assert_ne!(alice.endpoint_id(), mallory.endpoint_id());

        // Mallory builds a profile claiming to be Alice and signs it
        // with his own key.
        let spoof = UserProfile::new(alice.endpoint_id(), "Not Alice");
        let data = pack_profile(&spoof, &mallory).unwrap();

        let err = unpack_profile(&data).unwrap_err();
        match err {
            IdentityError::PeerMismatch { claimed, signer } => {
                assert_eq!(claimed, alice.endpoint_id());
                assert_eq!(signer, mallory.endpoint_id());
            }
            other => panic!("expected PeerMismatch, got {other:?}"),
        }
    }

    /// Issue #145: tampered bytes fail signature verification before we
    /// ever get to the peer_id check.
    #[test]
    fn unpack_profile_rejects_tampered_bytes() {
        let alice = Identity::generate();
        let profile = UserProfile::new(alice.endpoint_id(), "Alice");
        let mut data = pack_profile(&profile, &alice).unwrap();
        let len = data.len();
        data[len - 2] ^= 0xFF;

        let result = unpack_profile(&data);
        assert!(result.is_err(), "tampered profile must not verify");
    }

    /// Issue #424: an `EndpointId` parsed from a string that is not valid hex
    /// (and not a valid z32-encoded key) must fail with a parse error rather
    /// than silently producing a bogus key. `EndpointId` aliases iroh's
    /// `PublicKey`, whose `FromStr` impl rejects garbage input.
    #[test]
    fn endpoint_id_rejects_malformed_hex() {
        use std::str::FromStr;

        // Not hex, not z32, just a clearly-wrong string.
        let result = EndpointId::from_str("not-a-valid-key!!!");
        assert!(
            result.is_err(),
            "malformed string must not parse to an EndpointId, got: {result:?}"
        );

        // 64 chars but contains non-hex characters — should still fail.
        let bad_hex = "z".repeat(64);
        let result = EndpointId::from_str(&bad_hex);
        assert!(
            result.is_err(),
            "non-hex 64-char string must not parse, got: {result:?}"
        );

        // Wrong length hex (too short).
        let result = EndpointId::from_str("deadbeef");
        assert!(
            result.is_err(),
            "short hex must not parse to an EndpointId, got: {result:?}"
        );
    }

    /// Issue #424: feeding random / garbage bytes to `unpack_profile`
    /// (which deserializes via `willow_transport::unpack`, i.e. bincode)
    /// must surface as `IdentityError::Serde`, not a panic and not a
    /// silent default-constructed profile.
    #[test]
    fn profile_decode_rejects_invalid_cbor() {
        // Random-looking bytes that don't form a valid bincode-encoded
        // `SignedMessage`. (The crate uses bincode under the hood, but
        // the issue still names the test for historical reasons.)
        let garbage: [u8; 16] = [
            0xFF, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0,
            0x42, 0x42,
        ];

        let err = unpack_profile(&garbage).unwrap_err();
        match err {
            IdentityError::Serde(_) => {}
            other => panic!("expected IdentityError::Serde for garbage bytes, got {other:?}"),
        }

        // Empty input must also be rejected as a serialization error
        // (bincode can't parse an empty buffer into a `SignedMessage`).
        let err = unpack_profile(&[]).unwrap_err();
        match err {
            IdentityError::Serde(_) => {}
            other => panic!("expected IdentityError::Serde for empty bytes, got {other:?}"),
        }
    }

    /// Issue #424: malformed serialized envelope bytes — truncated mid-record
    /// or otherwise non-decodable — must be rejected by `unpack` with
    /// `IdentityError::Serde`. Exercises the outer-envelope deserialization
    /// path, distinct from a tampered-but-well-formed payload (which is
    /// covered by `tampered_payload_fails_verification`).
    #[test]
    fn unpack_rejects_truncated_envelope_bytes() {
        let alice = Identity::generate();
        let signed = pack(&String::from("hello"), &alice).unwrap();

        // Truncate to roughly the first quarter of the envelope so that
        // bincode can't reconstruct the `SignedMessage` framing
        // (length prefixes / Vec contents will run off the end).
        let truncated = &signed[..signed.len() / 4];
        let err = unpack::<String>(truncated).unwrap_err();
        match err {
            IdentityError::Serde(_) => {}
            other => panic!("expected IdentityError::Serde for truncated bytes, got {other:?}"),
        }

        // A `SignedMessage` whose `public_key` field is the wrong length
        // must reach the verify path and fail with `PublicKeyDecode`,
        // proving the signature path's malformed-bytes rejection works
        // for well-framed but cryptographically-invalid inputs.
        let bogus = SignedMessage {
            public_key: vec![0u8; 7], // not 32 bytes
            signature: vec![0u8; 64],
            payload: willow_transport::pack(&String::from("anything")).unwrap(),
        };
        let bytes = willow_transport::pack(&bogus).unwrap();
        let err = unpack::<String>(&bytes).unwrap_err();
        match err {
            IdentityError::PublicKeyDecode(_) => {}
            other => panic!("expected IdentityError::PublicKeyDecode for short pk, got {other:?}"),
        }
    }

    /// A key file with fewer than 32 bytes causes `load_or_generate` to return
    /// an error rather than panic.
    #[test]
    #[cfg(unix)]
    fn load_truncated_key_file_returns_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("truncated.key");

        // Write 16 bytes (half the expected 32-byte secret key) with secure perms.
        std::fs::write(&path, [0u8; 16]).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let result = Identity::load_or_generate(&path);
        assert!(
            result.is_err(),
            "truncated key file should produce an error, got: {result:?}"
        );
    }
}
