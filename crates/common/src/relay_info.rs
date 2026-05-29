//! Signed relay capability document (`WillowRelayInfo`).
//!
//! A Willow relay exposes a NIP-11-style capability sidecar at
//! `GET /.well-known/willow`: a single signed JSON object describing the
//! relay's protocol versions, limits, auth/payment requirements, operator
//! metadata, and feature tags. Clients fetch it *before* connecting so they can
//! pick the right wire version and surface gating up front, without a
//! failed-connection round-trip.
//!
//! This module is the single source of truth for the document **types**, its
//! **RFC 8785 (JCS) canonicalization**, its **Ed25519 signing/verification**,
//! and the **strong ETag** derived from the canonical bytes. Both the relay
//! producer and the client consumer import this exact code, so a relay's
//! signature is verifiable by construction — reimplementing canonicalization on
//! either side would silently break signature verification.
//!
//! Two canonical forms exist and must stay in lockstep (see the spec's
//! "Signing" section):
//! - **`CANON_SIGNED`** — canonical JSON with the `signature` field *removed*.
//!   These are the bytes the signature covers ([`canonical_json`] with
//!   `include_signature = false`).
//! - **`CANON_ETAG`** — canonical JSON with the `signature` field *included*.
//!   These are the bytes the strong ETag hashes ([`canonical_json`] with
//!   `include_signature = true`).
//!
//! See `docs/specs/2026-04-24-relay-capability-doc.md`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use willow_identity::{Identity, PublicKey, Signature};

/// Capability document served at `GET /.well-known/willow`.
///
/// All fields are optional except `protocol_versions`, `pubkey`, and
/// `signature` (the latter two are required because v1 mandates signing).
/// Clients MUST ignore unknown top-level fields, which `serde` does by default,
/// so the schema stays forward-compatible.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WillowRelayInfo {
    /// Operator-chosen display name (≤ 60 UTF-8 bytes by convention).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Plain-text description, no markup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Operator contact, e.g. `mailto:` / `https:` / `matrix:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    /// Hex Ed25519 operator DM key (a hint; trust comes from the DAG).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin_pubkey: Option<String>,
    /// REQUIRED. Hex Ed25519 public half of the relay's own key — the verifier
    /// for [`WillowRelayInfo::signature`]. Mandatory in v1 because signing is
    /// mandatory.
    pub pubkey: String,
    /// Project name; operators MAY omit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software: Option<String>,
    /// Coarse semver (e.g. `"0.3.x"`); operators MAY omit. Never a git SHA.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Terms-of-service URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terms_of_service: Option<String>,
    /// Privacy-policy URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy_policy: Option<String>,
    /// Square icon URL (≥ 64×64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// REQUIRED. Wire-protocol versions the relay accepts, sorted highest-first,
    /// no duplicates. Mirrors `willow_transport::PROTOCOL_VERSION`.
    pub protocol_versions: Vec<u16>,
    /// Short string feature tags (see [`features`] for the canonical set).
    #[serde(default)]
    pub supported_features: Vec<String>,
    /// REQUIRED. Detached Ed25519 signature over the `CANON_SIGNED` bytes
    /// (canonical JSON with this field removed). Lowercase hex.
    pub signature: String,
    /// Advertised limits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limitation: Option<Limitation>,
    /// Retention policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<Retention>,
    /// Required iff `payment_required`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payments_url: Option<String>,
    /// Required iff `invite_required`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invites_url: Option<String>,
    /// `"ok"` | `"degraded"` | `"read_only"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Human-readable status detail (plain text).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
}

/// Advertised relay limits. All fields optional.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Limitation {
    /// Mirrors `willow_transport::MAX_DESER_SIZE`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_message_bytes: Option<u32>,
    /// Max topic-string length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_topic_len: Option<u16>,
    /// Max simultaneously announced topics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_topics: Option<u32>,
    /// Max concurrent connections.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<u32>,
    /// Max blob bytes; `0` = blob pinning off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_blob_bytes: Option<u64>,
    /// Whether an invite is required to use the relay.
    #[serde(default)]
    pub invite_required: bool,
    /// Whether payment is required to use the relay.
    #[serde(default)]
    pub payment_required: bool,
    /// Oldest accepted HLC, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hlc_lower_limit: Option<u64>,
    /// Reject handshakes from clients older than this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_client_version: Option<u16>,
}

/// Retention policy advertised by the relay's worker class.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Retention {
    /// `"replay"` (in-memory, per-author cap) or `"storage"` (SQLite).
    pub mode: String,
    /// Per-author event cap; `null` = unbounded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_events_per_author: Option<u32>,
    /// Max event age in seconds; `null` = keep everything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_age_seconds: Option<u64>,
    /// Whether the relay escrows sealed channel keys. Willow default: `false`.
    #[serde(default)]
    pub channel_key_escrow: bool,
}

/// Canonical v1 `supported_features` tag strings (pinned decision #3).
///
/// Producer (relay) and consumer (client) agree on these by construction. Tags
/// for sibling specs are advertised only once their implementing PR lands.
pub mod features {
    /// Gossip transport (always supported).
    pub const GOSSIP: &str = "gossip";
    /// Historical event replay.
    pub const HISTORY: &str = "history";
    /// Blob storage / pinning.
    pub const BLOBS: &str = "blobs";
    /// Voice signalling relay.
    pub const VOICE_SIGNAL: &str = "voice-signal";
    /// Invite-gated access.
    pub const INVITE_GATE: &str = "invite-gate";
    /// Payment-gated access.
    pub const PAYMENT_GATE: &str = "payment-gate";
    /// End-of-stored-events sentinel (advertised once the EOSE PR lands).
    pub const HISTORY_EOSE: &str = "history-eose";
    /// Per-author seq-vector delta sync (advertised once the heads-sync PR
    /// lands). Note: this is **not** `"negentropy"` — RBSR was abandoned for
    /// per-author seq cursors, so advertising `"negentropy"` would
    /// misrepresent the wire protocol.
    pub const SEQ_VECTOR_SYNC: &str = "seq-vector-sync";
}

/// Errors produced while canonicalizing, signing, or verifying a capability
/// document.
#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    /// A required field (`pubkey` or `signature`) was empty.
    #[error("missing required field: {0}")]
    MissingField(&'static str),
    /// JSON canonicalization failed.
    #[error("canonicalization failed: {0}")]
    Canon(String),
    /// The detached signature did not verify against `pubkey`.
    #[error("signature verification failed")]
    BadSignature,
}

/// RFC 8785 (JCS) canonical bytes of the document.
///
/// When `include_signature` is `false`, the `signature` field is removed first,
/// producing the `CANON_SIGNED` bytes the signature covers. When `true`, the
/// signature is kept, producing the `CANON_ETAG` bytes the strong ETag hashes.
///
/// Canonicalization is delegated to `serde_jcs`, which implements RFC 8785
/// (sorted keys, minimal escaping, shortest-form numbers). Hand-rolling the
/// number/unicode escaping is the documented source of cross-implementation
/// canonicalization bugs, so the crate is preferred.
///
/// # Errors
///
/// Returns [`CapabilityError::Canon`] if the document cannot be represented as
/// canonical JSON (impossible for this struct's field types, but surfaced
/// rather than panicking).
pub fn canonical_json(
    info: &WillowRelayInfo,
    include_signature: bool,
) -> Result<Vec<u8>, CapabilityError> {
    let mut value =
        serde_json::to_value(info).map_err(|e| CapabilityError::Canon(e.to_string()))?;
    if !include_signature {
        if let Some(obj) = value.as_object_mut() {
            obj.remove("signature");
        }
    }
    serde_jcs::to_vec(&value).map_err(|e| CapabilityError::Canon(e.to_string()))
}

/// Strong ETag = lowercase hex SHA-256 over the `CANON_ETAG` bytes (canonical
/// JSON with `signature` included).
///
/// Canonical JSON gives byte-equality semantics, so the ETag is *strong* and
/// enables cross-relay caching keyed by content hash.
#[must_use]
pub fn capability_etag(canonical_etag_bytes: &[u8]) -> String {
    let digest = Sha256::digest(canonical_etag_bytes);
    hex::encode(digest)
}

/// Sign the document in place.
///
/// Canonicalizes the document with the `signature` field removed
/// (`CANON_SIGNED`), signs those bytes with the relay's Ed25519 key, and stores
/// the lowercase-hex signature in [`WillowRelayInfo::signature`]. The caller is
/// responsible for having set `pubkey` to the hex of the same identity's public
/// key.
///
/// # Errors
///
/// Returns [`CapabilityError::Canon`] if canonicalization fails.
pub fn sign_capability_doc(
    info: &mut WillowRelayInfo,
    identity: &Identity,
) -> Result<(), CapabilityError> {
    info.signature.clear();
    let bytes = canonical_json(info, false)?;
    info.signature = hex::encode(identity.sign(&bytes).to_bytes());
    Ok(())
}

/// Verify the detached signature against the document's own `pubkey` field.
///
/// A document whose signature does not verify MUST be treated by callers as if
/// the endpoint returned `404`, and MUST NOT be cached.
///
/// # Errors
///
/// Returns [`CapabilityError::MissingField`] if `pubkey` or `signature` is
/// empty, or [`CapabilityError::Canon`] if canonicalization fails. A malformed
/// `pubkey`/`signature`, or a signature that does not verify, returns
/// `Ok(false)` rather than an error so callers have a single "treat as 404"
/// path.
pub fn verify_capability_doc(info: &WillowRelayInfo) -> Result<bool, CapabilityError> {
    if info.pubkey.is_empty() {
        return Err(CapabilityError::MissingField("pubkey"));
    }
    if info.signature.is_empty() {
        return Err(CapabilityError::MissingField("signature"));
    }
    let bytes = canonical_json(info, false)?;

    let Ok(sig_bytes) = hex::decode(&info.signature) else {
        return Ok(false);
    };
    let Ok(sig_arr) = <[u8; 64]>::try_from(sig_bytes.as_slice()) else {
        return Ok(false);
    };
    let signature = Signature::from_bytes(&sig_arr);

    let Ok(pk_bytes) = hex::decode(&info.pubkey) else {
        return Ok(false);
    };
    let Ok(pk_arr) = <[u8; 32]>::try_from(pk_bytes.as_slice()) else {
        return Ok(false);
    };
    let Ok(public_key) = PublicKey::from_bytes(&pk_arr) else {
        return Ok(false);
    };

    Ok(willow_identity::verify(&public_key, &bytes, &signature))
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    fn sample() -> WillowRelayInfo {
        WillowRelayInfo {
            name: Some("Test Relay".into()),
            description: None,
            contact: None,
            admin_pubkey: None,
            pubkey: "ab".repeat(32),
            software: None,
            version: None,
            terms_of_service: None,
            privacy_policy: None,
            icon: None,
            protocol_versions: vec![2],
            supported_features: vec![features::GOSSIP.into(), features::HISTORY.into()],
            signature: String::new(),
            limitation: None,
            retention: None,
            payments_url: None,
            invites_url: None,
            status: Some("ok".into()),
            status_detail: None,
        }
    }

    /// Fully-populated, representative document for byte-budget probes. Hex
    /// pubkey/signature are fixed-length, so the canonical byte length does not
    /// depend on the random key — the size is deterministic across runs.
    fn representative(id: &Identity) -> WillowRelayInfo {
        let mut info = WillowRelayInfo {
            name: Some("Willow Community Relay".into()),
            description: Some("A public relay for the Willow network.".into()),
            contact: Some("mailto:ops@relay.example.com".into()),
            admin_pubkey: Some("ab".repeat(32)),
            pubkey: hex::encode(id.public_key().as_bytes()),
            software: Some("willow-relay".into()),
            version: Some("0.1.x".into()),
            terms_of_service: Some("https://relay.example.com/tos".into()),
            privacy_policy: Some("https://relay.example.com/privacy".into()),
            icon: Some("https://relay.example.com/icon.png".into()),
            protocol_versions: vec![2, 1],
            supported_features: vec![
                features::GOSSIP.into(),
                features::HISTORY.into(),
                features::BLOBS.into(),
                features::VOICE_SIGNAL.into(),
                features::INVITE_GATE.into(),
                features::PAYMENT_GATE.into(),
            ],
            signature: String::new(),
            limitation: Some(Limitation {
                max_message_bytes: Some(65_536),
                max_topic_len: Some(256),
                max_topics: Some(10_000),
                max_connections: Some(1_000),
                max_blob_bytes: Some(0),
                invite_required: false,
                payment_required: false,
                hlc_lower_limit: None,
                min_client_version: Some(2),
            }),
            retention: Some(Retention {
                mode: "storage".into(),
                max_events_per_author: Some(1_000),
                max_age_seconds: None,
                channel_key_escrow: false,
            }),
            payments_url: None,
            invites_url: None,
            status: Some("ok".into()),
            status_detail: None,
        };
        sign_capability_doc(&mut info, id).unwrap();
        info
    }

    // ── Task 1.2: struct definitions + serde round-trip ──────────────────────

    #[test]
    fn full_round_trip() {
        let info = sample();
        let json = serde_json::to_string(&info).unwrap();
        let back: WillowRelayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }

    #[test]
    fn minimal_doc_parses_with_none_fields() {
        let json = r#"{"protocol_versions":[1],"pubkey":"aa","signature":"bb"}"#;
        let info: WillowRelayInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.protocol_versions, vec![1]);
        assert!(info.name.is_none() && info.limitation.is_none());
        assert!(info.supported_features.is_empty());
    }

    #[test]
    fn unknown_top_level_field_is_ignored() {
        let json = r#"{"protocol_versions":[2],"pubkey":"aa","signature":"bb","future_field":42}"#;
        let info: WillowRelayInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.protocol_versions, vec![2]);
    }

    #[test]
    fn nested_limitation_retention_round_trip() {
        let mut info = sample();
        info.limitation = Some(Limitation {
            max_message_bytes: Some(65_536),
            max_topic_len: Some(256),
            max_topics: Some(10_000),
            max_connections: Some(1_000),
            max_blob_bytes: Some(0),
            invite_required: true,
            payment_required: false,
            hlc_lower_limit: Some(1_700_000_000_000),
            min_client_version: Some(2),
        });
        info.retention = Some(Retention {
            mode: "storage".into(),
            max_events_per_author: Some(1_000),
            max_age_seconds: None,
            channel_key_escrow: false,
        });
        let json = serde_json::to_string(&info).unwrap();
        let back: WillowRelayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, back);
    }

    // ── Task 1.3: JCS canonicalization (two forms) + ETag ────────────────────

    #[test]
    fn canonical_is_field_order_independent() {
        let a: WillowRelayInfo =
            serde_json::from_str(r#"{"pubkey":"aa","protocol_versions":[2],"signature":"sig"}"#)
                .unwrap();
        let b: WillowRelayInfo =
            serde_json::from_str(r#"{"signature":"sig","protocol_versions":[2],"pubkey":"aa"}"#)
                .unwrap();
        assert_eq!(
            canonical_json(&a, true).unwrap(),
            canonical_json(&b, true).unwrap()
        );
    }

    #[test]
    fn canonical_keys_are_sorted_and_compact() {
        let info: WillowRelayInfo =
            serde_json::from_str(r#"{"pubkey":"aa","protocol_versions":[2],"signature":"sig"}"#)
                .unwrap();
        let bytes = canonical_json(&info, true).unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        // No whitespace, keys lexicographically sorted. `supported_features` has
        // `#[serde(default)]` but no `skip_serializing_if`, so an empty vec is
        // still emitted ([]) — this pins that the canonical form is stable and
        // explicit about the field's presence: protocol_versions < pubkey <
        // signature < supported_features.
        assert_eq!(
            s,
            r#"{"protocol_versions":[2],"pubkey":"aa","signature":"sig","supported_features":[]}"#
        );
    }

    #[test]
    fn canonical_signed_excludes_signature() {
        let mut info = sample();
        info.signature = "DEADBEEF".into();
        let signed_form = canonical_json(&info, false).unwrap();
        assert!(!String::from_utf8_lossy(&signed_form).contains("DEADBEEF"));
        assert!(
            String::from_utf8_lossy(&canonical_json(&info, true).unwrap()).contains("DEADBEEF")
        );
    }

    #[test]
    fn etag_is_deterministic_sha256_hex() {
        let bytes = canonical_json(&sample(), true).unwrap();
        let e1 = capability_etag(&bytes);
        assert_eq!(e1, capability_etag(&bytes));
        assert_eq!(e1.len(), 64); // SHA-256 hex
        assert!(e1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── Task 1.4: sign + verify (Ed25519 over CANON_SIGNED) ──────────────────

    #[test]
    fn sign_then_verify_roundtrip() {
        let id = Identity::generate();
        let mut info = sample();
        info.pubkey = hex::encode(id.public_key().as_bytes());
        sign_capability_doc(&mut info, &id).unwrap();
        assert_eq!(info.signature.len(), 128); // 64-byte Ed25519 sig as hex
        assert!(verify_capability_doc(&info).unwrap());
    }

    #[test]
    fn tampering_breaks_verification() {
        let id = Identity::generate();
        let mut info = sample();
        info.pubkey = hex::encode(id.public_key().as_bytes());
        sign_capability_doc(&mut info, &id).unwrap();
        info.status = Some("read_only".into()); // attacker flips a field
        assert!(!verify_capability_doc(&info).unwrap());
    }

    #[test]
    fn verify_rejects_wrong_pubkey() {
        let signer = Identity::generate();
        let other = Identity::generate();
        let mut info = sample();
        info.pubkey = hex::encode(signer.public_key().as_bytes());
        sign_capability_doc(&mut info, &signer).unwrap();
        // Swap in a different relay's pubkey: signature no longer matches.
        info.pubkey = hex::encode(other.public_key().as_bytes());
        assert!(!verify_capability_doc(&info).unwrap());
    }

    #[test]
    fn verify_missing_pubkey_is_error() {
        let mut info = sample();
        info.pubkey = String::new();
        info.signature = "bb".into();
        assert!(matches!(
            verify_capability_doc(&info),
            Err(CapabilityError::MissingField("pubkey"))
        ));
    }

    #[test]
    fn verify_missing_signature_is_error() {
        let info = sample(); // signature is empty
        assert!(matches!(
            verify_capability_doc(&info),
            Err(CapabilityError::MissingField("signature"))
        ));
    }

    #[test]
    fn verify_malformed_signature_is_false_not_error() {
        let id = Identity::generate();
        let mut info = sample();
        info.pubkey = hex::encode(id.public_key().as_bytes());
        info.signature = "not hex !!".into();
        // Malformed encoding must funnel into the single "treat as 404" path.
        assert!(!verify_capability_doc(&info).unwrap());
    }

    #[test]
    fn verify_short_signature_is_false_not_error() {
        let id = Identity::generate();
        let mut info = sample();
        info.pubkey = hex::encode(id.public_key().as_bytes());
        info.signature = "aa".into(); // valid hex, wrong length
        assert!(!verify_capability_doc(&info).unwrap());
    }

    #[test]
    fn unknown_feature_tags_survive_round_trip_and_verify() {
        let id = Identity::generate();
        let mut info = sample();
        info.pubkey = hex::encode(id.public_key().as_bytes());
        info.supported_features
            .push("some-future-unknown-tag".into());
        sign_capability_doc(&mut info, &id).unwrap();

        let json = serde_json::to_string(&info).unwrap();
        let decoded: WillowRelayInfo = serde_json::from_str(&json).unwrap();
        assert!(decoded
            .supported_features
            .iter()
            .any(|f| f == "some-future-unknown-tag"));
        // Forward-compat: unknown tags must not break verification.
        assert!(verify_capability_doc(&decoded).unwrap());
    }

    #[test]
    fn signature_is_stable_across_field_reordering_on_the_wire() {
        let id = Identity::generate();
        let mut info = sample();
        info.pubkey = hex::encode(id.public_key().as_bytes());
        sign_capability_doc(&mut info, &id).unwrap();

        // Re-parse from a hand-reordered JSON object: the signature must still
        // verify because canonicalization is order-free.
        let reordered = format!(
            r#"{{"signature":"{}","protocol_versions":[2],"pubkey":"{}","name":"Test Relay","status":"ok","supported_features":["gossip","history"]}}"#,
            info.signature, info.pubkey
        );
        let decoded: WillowRelayInfo = serde_json::from_str(&reordered).unwrap();
        assert!(verify_capability_doc(&decoded).unwrap());
    }

    // ── Byte-budget probe (cross-spec pinned decision #4: 64 KiB pkarr) ───────

    #[test]
    fn budget_probe_well_under_64kib() {
        // The canonical (CANON_ETAG) bytes are the on-wire form the pkarr packet
        // embeds. Assert generously under the 64 KiB budget and under a tight
        // 1 KiB ceiling for a representative document; the exact pinned value is
        // checked in `budget_probe_pinned_canonical_size`.
        let id = Identity::generate();
        let info = representative(&id);
        let canonical = canonical_json(&info, true).unwrap();
        assert!(
            canonical.len() < 2 * 1024,
            "representative canonical doc must be < 2 KiB, got {} bytes",
            canonical.len()
        );
        assert!(
            canonical.len() < 64 * 1024,
            "must fit pkarr 64 KiB budget, got {} bytes",
            canonical.len()
        );
        // The non-canonical serde wire form is also bounded.
        assert!(serde_json::to_vec(&info).unwrap().len() < 64 * 1024);
    }

    #[test]
    fn budget_probe_pinned_canonical_size() {
        let id = Identity::generate();
        let info = representative(&id);
        let canonical = canonical_json(&info, true).unwrap();
        // PINNED measured value — update only on a deliberate schema change.
        assert_eq!(
            canonical.len(),
            PINNED_CANONICAL_LEN,
            "representative canonical doc size drifted from the pinned value"
        );
    }

    /// Pinned canonical byte length of [`representative`].
    ///
    /// MEASURED at 1037 bytes (full operator metadata, 6 feature tags, signed) —
    /// the on-wire CANON_ETAG size the pkarr packet carries, ~1.6% of the 64 KiB
    /// pkarr budget (pinned decision #4). Deterministic across runs: hex
    /// pubkey/signature are fixed-length and serde_jcs output is canonical.
    /// Update only on a deliberate schema change.
    const PINNED_CANONICAL_LEN: usize = 1037;
}
