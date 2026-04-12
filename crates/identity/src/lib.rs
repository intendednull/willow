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
