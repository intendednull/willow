//! # Secure Invite Codes
//!
//! Invite codes use per-recipient encryption. Channel keys are encrypted
//! using ephemeral X25519 Diffie-Hellman for the specific recipient's
//! Ed25519 public key. Even if the invite code is intercepted, only the
//! intended recipient can decrypt the channel keys.
//!
//! ## Flow
//!
//! 1. Recipient shares their PeerId (derived from Ed25519 public key).
//! 2. Server admin enters the recipient's PeerId and generates an invite.
//! 3. Each channel key is encrypted via [`willow_crypto::encrypt_channel_key_for`]
//!    using the recipient's Ed25519 public key converted to X25519.
//! 4. The invite is serialized + base64-encoded for sharing.
//! 5. Recipient pastes the code, decrypts with their private key, and joins.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use willow_crypto::ChannelKey;

/// The data embedded in a secure invite code.
///
/// **Security note:** The `genesis_author` and `sync_providers` fields are
/// *suggestions* from the invite creator, NOT guaranteed truths. A
/// malicious actor could forge an invite with fake trusted users.
/// The joining peer should verify state from multiple sources and use
/// the event log (GrantPermission events from admins) as the
/// canonical source of trust.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvitePayload {
    /// Server name (for display to the recipient).
    pub server_name: String,
    /// Server ID (for constructing gossipsub topics).
    pub server_id: String,
    /// EndpointId of the genesis event author (first admin).
    /// This is a *hint* — verify by checking event history.
    pub genesis_author: willow_identity::EndpointId,
    /// Suggested peers that can provide full history (SyncProvider permission).
    /// These are *hints* — the joining peer should verify from multiple sources.
    #[serde(default)]
    pub sync_providers: Vec<willow_identity::EndpointId>,
    /// Per-channel encrypted keys. Each channel key is encrypted for the
    /// specific recipient -- only they can decrypt with their Ed25519 key.
    pub channels: Vec<EncryptedChannel>,
}

/// A channel entry in the invite with its key encrypted for the recipient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedChannel {
    /// Human-readable channel name.
    pub name: String,
    /// Gossipsub topic for this channel.
    pub topic: String,
    /// Channel key encrypted for the recipient via X25519 DH + ChaCha20.
    pub encrypted_key: willow_crypto::EncryptedChannelKey,
}

/// Maximum number of channels permitted in a single invite.
///
/// Caps how many gossip topics a malicious invite can subscribe a joining
/// peer to. Real servers ship a handful to a few dozen channels; 1000 is
/// well above any realistic legitimate use.
pub const MAX_INVITE_CHANNELS: usize = 1000;

/// Maximum permitted length, in bytes, of a channel topic string.
///
/// Chosen to bound subscription state — gossip topics over a few hundred
/// bytes have no legitimate use. Mirrors the relay's per-topic cap
/// (`MAX_TOPIC_LEN`) so an invite cannot ship a topic the relay would
/// reject anyway.
pub const MAX_TOPIC_LEN: usize = 256;

/// Validate that a channel `topic` belongs to `server_id` and embeds
/// the declared channel `name`.
///
/// Topics are constructed (in `generate_invite` and across the codebase)
/// as `"{server_id}/{channel_name}"`. A malicious invite that ships a
/// topic with a different prefix could subscribe a joining peer to an
/// arbitrary gossip topic. We reject any invite whose topics do not
/// match this format end-to-end:
///
/// - `topic` must start with `"{server_id}/"`,
/// - the suffix after that prefix must equal the declared `name`,
/// - and the topic must be no longer than [`MAX_TOPIC_LEN`].
fn topic_matches_server(server_id: &str, topic: &str, name: &str) -> bool {
    if topic.len() > MAX_TOPIC_LEN {
        return false;
    }
    let expected_prefix = format!("{server_id}/");
    let Some(suffix) = topic.strip_prefix(&expected_prefix) else {
        return false;
    };
    suffix == name
}

/// Validate an [`InvitePayload`] before its topics are trusted.
///
/// Returns `false` if the channel count exceeds [`MAX_INVITE_CHANNELS`]
/// or if any channel's `(topic, name)` pair fails [`topic_matches_server`].
/// On any failure the *entire* invite is rejected — we never silently
/// drop a single channel and accept the rest, because a partial accept
/// still subscribes the joining peer to whatever malicious topics
/// remained.
fn validate_invite_payload(payload: &InvitePayload) -> bool {
    if payload.channels.len() > MAX_INVITE_CHANNELS {
        return false;
    }
    payload
        .channels
        .iter()
        .all(|ch| topic_matches_server(&payload.server_id, &ch.topic, &ch.name))
}

/// Generate a secure invite code encrypted for a specific recipient.
///
/// Takes the data it needs directly rather than a Server object:
/// - `server_name` / `server_id` / `genesis_author` for the invite payload
/// - `keys`: topic → channel key
/// - `topic_map`: topic → channel name
/// - `recipient_ed25519_public`: 32-byte Ed25519 public key
///
/// Returns `None` if encryption fails.
pub fn generate_invite(
    server_name: &str,
    server_id: &str,
    genesis_author: willow_identity::EndpointId,
    keys: &HashMap<String, ChannelKey>,
    topic_map: &HashMap<String, String>,
    recipient_ed25519_public: &[u8; 32],
) -> Option<String> {
    let mut channels = Vec::new();

    for (topic, name) in topic_map {
        if let Some(key) = keys.get(topic) {
            let encrypted_key =
                willow_crypto::encrypt_channel_key_for(key, recipient_ed25519_public).ok()?;
            channels.push(EncryptedChannel {
                name: name.clone(),
                topic: topic.clone(),
                encrypted_key,
            });
        }
    }

    let payload = InvitePayload {
        server_name: server_name.to_string(),
        server_id: server_id.to_string(),
        genesis_author,
        sync_providers: Vec::new(), // populated by caller if known
        channels,
    };

    let bytes = willow_transport::pack(&payload).ok()?;
    Some(crate::base64::encode(&bytes))
}

/// Parse an invite code and decrypt the channel keys using our identity.
///
/// Returns the server info and decrypted channel keys, or `None` if the
/// code is invalid, fails topic validation (see [`validate_invite_payload`]),
/// or we're not the intended recipient.
pub fn accept_invite(
    code: &str,
    our_identity: &willow_identity::Identity,
) -> Option<AcceptedInvite> {
    let bytes = crate::base64::decode(code.trim())?;
    let payload: InvitePayload = willow_transport::unpack(&bytes).ok()?;

    // Reject the entire invite on any topic-confusion failure: a malicious
    // invite could otherwise subscribe a joining peer to arbitrary gossip
    // topics, or ship an unbounded channel list to exhaust resources.
    if !validate_invite_payload(&payload) {
        return None;
    }

    let mut channel_keys = HashMap::new();
    for ch in &payload.channels {
        let key = willow_crypto::decrypt_channel_key(&ch.encrypted_key, our_identity).ok()?;
        channel_keys.insert(ch.topic.clone(), (ch.name.clone(), key));
    }

    Some(AcceptedInvite {
        server_name: payload.server_name,
        server_id: payload.server_id,
        genesis_author: payload.genesis_author,
        sync_providers: payload.sync_providers,
        channel_keys,
    })
}

/// Result of successfully accepting an invite.
///
/// The `genesis_author` and `sync_providers` are *hints* from the invite creator.
/// They should be verified against the event log from multiple sources.
pub struct AcceptedInvite {
    pub server_name: String,
    pub server_id: String,
    /// Suggested genesis author EndpointId (verify via event history).
    pub genesis_author: willow_identity::EndpointId,
    /// Suggested sync providers (verify via event history).
    pub sync_providers: Vec<willow_identity::EndpointId>,
    /// topic -> (channel_name, decrypted key)
    pub channel_keys: HashMap<String, (String, ChannelKey)>,
}

/// Extract the 32-byte Ed25519 public key from an EndpointId.
///
/// Since `EndpointId` IS the Ed25519 public key, this just returns its bytes.
pub fn endpoint_id_to_ed25519_public(endpoint_id: &willow_identity::EndpointId) -> [u8; 32] {
    *endpoint_id.as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    /// Helper: create a server_id, a channel key, and the corresponding
    /// keys + topic_map for a single-channel server.
    fn test_server_with_channels(
        _server_name: &str,
        channel_names: &[&str],
    ) -> (
        String,                      // server_id
        HashMap<String, ChannelKey>, // keys
        HashMap<String, String>,     // topic_map: topic → name
    ) {
        let server_id = format!("test-server-{}", uuid::Uuid::new_v4());
        let mut keys = HashMap::new();
        let mut topic_map = HashMap::new();

        for name in channel_names {
            let topic = format!("{}/{}", server_id, name);
            let key = willow_crypto::generate_channel_key();
            keys.insert(topic.clone(), key);
            topic_map.insert(topic, name.to_string());
        }

        (server_id, keys, topic_map)
    }

    /// Helper to extract Ed25519 public key bytes from an Identity.
    fn recipient_public_bytes(identity: &Identity) -> [u8; 32] {
        *identity.endpoint_id().as_bytes()
    }

    #[test]
    fn secure_invite_round_trip() {
        let owner = Identity::generate();
        let recipient = Identity::generate();

        let (server_id, keys, topic_map) = test_server_with_channels("Secure Server", &["general"]);
        let topic = format!("{}/general", server_id);
        let recipient_pub = recipient_public_bytes(&recipient);

        let code = generate_invite(
            "Secure Server",
            &server_id,
            owner.endpoint_id(),
            &keys,
            &topic_map,
            &recipient_pub,
        )
        .unwrap();

        let accepted = accept_invite(&code, &recipient).unwrap();

        assert_eq!(accepted.server_name, "Secure Server");
        assert_eq!(accepted.channel_keys.len(), 1);

        let (name, decrypted_key) = &accepted.channel_keys[&topic];
        assert_eq!(name, "general");
        assert_eq!(decrypted_key.as_bytes(), keys[&topic].as_bytes());
    }

    #[test]
    fn wrong_recipient_cannot_decrypt() {
        let owner = Identity::generate();
        let intended = Identity::generate();
        let intruder = Identity::generate();

        let (server_id, keys, topic_map) = test_server_with_channels("Secure", &["secret"]);

        let intended_pub = recipient_public_bytes(&intended);
        let code = generate_invite(
            "Secure",
            &server_id,
            owner.endpoint_id(),
            &keys,
            &topic_map,
            &intended_pub,
        )
        .unwrap();

        // Intruder cannot decrypt the invite.
        assert!(accept_invite(&code, &intruder).is_none());

        // Intended recipient can.
        assert!(accept_invite(&code, &intended).is_some());
    }

    #[test]
    fn invalid_code_returns_none() {
        let id = Identity::generate();
        assert!(accept_invite("not-valid-base64!!!", &id).is_none());
        assert!(accept_invite("", &id).is_none());
    }

    #[test]
    fn endpoint_id_to_public_key_round_trip() {
        let id = Identity::generate();
        let endpoint_id = id.endpoint_id();

        let pub_bytes = endpoint_id_to_ed25519_public(&endpoint_id);
        let expected = recipient_public_bytes(&id);

        assert_eq!(pub_bytes, expected);
    }

    #[test]
    fn multiple_channels_encrypted() {
        let owner = Identity::generate();
        let recipient = Identity::generate();

        let (server_id, keys, topic_map) =
            test_server_with_channels("Multi", &["general", "random", "voice"]);

        let recipient_pub = recipient_public_bytes(&recipient);
        let code = generate_invite(
            "Multi",
            &server_id,
            owner.endpoint_id(),
            &keys,
            &topic_map,
            &recipient_pub,
        )
        .unwrap();
        let accepted = accept_invite(&code, &recipient).unwrap();

        assert_eq!(accepted.channel_keys.len(), 3);
    }

    /// Helper: build an invite payload, hand-rewrite it, and re-encode
    /// so we can produce a code that is cryptographically valid for
    /// `recipient` but carries malicious topic data. This is the same
    /// shape an attacker would forge to mount a topic-confusion attack.
    fn forged_code<F: FnOnce(&mut InvitePayload)>(
        owner: &Identity,
        recipient: &Identity,
        server_name: &str,
        channel_names: &[&str],
        mutate: F,
    ) -> String {
        let (server_id, keys, topic_map) = test_server_with_channels(server_name, channel_names);
        let recipient_pub = recipient_public_bytes(recipient);
        let code = generate_invite(
            server_name,
            &server_id,
            owner.endpoint_id(),
            &keys,
            &topic_map,
            &recipient_pub,
        )
        .unwrap();
        let raw = crate::base64::decode(&code).unwrap();
        let mut payload: InvitePayload = willow_transport::unpack(&raw).unwrap();
        mutate(&mut payload);
        let bytes = willow_transport::pack(&payload).unwrap();
        crate::base64::encode(&bytes)
    }

    #[test]
    fn happy_path_passes_validation() {
        // Sanity check: a freshly generated invite passes the new
        // validator end-to-end. Guards against the validator being
        // accidentally too strict and rejecting honest traffic.
        let owner = Identity::generate();
        let recipient = Identity::generate();
        let (server_id, keys, topic_map) =
            test_server_with_channels("Happy", &["general", "random"]);
        let recipient_pub = recipient_public_bytes(&recipient);

        let code = generate_invite(
            "Happy",
            &server_id,
            owner.endpoint_id(),
            &keys,
            &topic_map,
            &recipient_pub,
        )
        .unwrap();

        let accepted = accept_invite(&code, &recipient).expect("valid invite must be accepted");
        assert_eq!(accepted.channel_keys.len(), 2);
    }

    #[test]
    fn mismatched_server_id_prefix_is_rejected() {
        // A topic whose server_id prefix does not match the payload's
        // server_id would subscribe the joining peer to gossip on an
        // attacker-chosen server. The whole invite must be rejected.
        let owner = Identity::generate();
        let recipient = Identity::generate();
        let code = forged_code(&owner, &recipient, "Evil", &["general"], |payload| {
            // Repoint every channel's topic at a different server.
            for ch in &mut payload.channels {
                ch.topic = format!("attacker-server/{}", ch.name);
            }
        });
        assert!(
            accept_invite(&code, &recipient).is_none(),
            "topic with mismatched server_id prefix must be rejected"
        );
    }

    #[test]
    fn over_long_topic_is_rejected() {
        // A topic longer than MAX_TOPIC_LEN has no legitimate use and
        // could be used to bloat subscription state on the joiner.
        let owner = Identity::generate();
        let recipient = Identity::generate();
        let code = forged_code(&owner, &recipient, "Long", &["general"], |payload| {
            // Build a topic that keeps the right server_id prefix and
            // declared name, but pads the name out past the cap so it
            // still "matches" the suffix yet trips the length check.
            let big_name = "x".repeat(MAX_TOPIC_LEN);
            for ch in &mut payload.channels {
                ch.name = big_name.clone();
                ch.topic = format!("{}/{}", payload.server_id, big_name);
            }
        });
        assert!(
            accept_invite(&code, &recipient).is_none(),
            "over-length topic must be rejected"
        );
    }

    #[test]
    fn channel_count_over_cap_is_rejected() {
        // An invite that ships more than MAX_INVITE_CHANNELS channels
        // could be used to fan a joining peer out across an unbounded
        // number of gossip subscriptions.
        let owner = Identity::generate();
        let recipient = Identity::generate();
        let code = forged_code(&owner, &recipient, "Big", &["general"], |payload| {
            // Replicate the single legitimate channel until the list
            // exceeds MAX_INVITE_CHANNELS. Topics still validate
            // individually, so this proves the count cap fires.
            let template = payload.channels[0].clone();
            payload.channels.clear();
            for _ in 0..(MAX_INVITE_CHANNELS + 1) {
                payload.channels.push(template.clone());
            }
        });
        assert!(
            accept_invite(&code, &recipient).is_none(),
            "channel count above MAX_INVITE_CHANNELS must be rejected"
        );
    }

    #[test]
    fn channel_name_mismatch_with_topic_suffix_is_rejected() {
        // A topic whose suffix does not match the declared `name` would
        // let an attacker route the joiner to a different channel under
        // an innocent-looking display name.
        let owner = Identity::generate();
        let recipient = Identity::generate();
        let code = forged_code(&owner, &recipient, "Spoof", &["general"], |payload| {
            // Keep `name = "general"` but point `topic` at a different
            // channel under the same server.
            for ch in &mut payload.channels {
                ch.topic = format!("{}/secret-admin", payload.server_id);
            }
        });
        assert!(
            accept_invite(&code, &recipient).is_none(),
            "topic suffix mismatching declared name must be rejected"
        );
    }

    #[test]
    fn channel_count_at_cap_is_accepted() {
        // Boundary check: the cap is "more than" MAX_INVITE_CHANNELS,
        // so exactly MAX_INVITE_CHANNELS channels must still pass.
        let owner = Identity::generate();
        let recipient = Identity::generate();
        let code = forged_code(&owner, &recipient, "Edge", &["general"], |payload| {
            let template = payload.channels[0].clone();
            payload.channels.clear();
            for _ in 0..MAX_INVITE_CHANNELS {
                payload.channels.push(template.clone());
            }
        });
        assert!(
            accept_invite(&code, &recipient).is_some(),
            "channel count at exactly MAX_INVITE_CHANNELS must still be accepted"
        );
    }

    #[test]
    fn generate_invite_via_endpoint_id_produces_valid_invite() {
        let owner = Identity::generate();
        let joiner = Identity::generate();

        let (server_id, keys, topic_map) = test_server_with_channels("Join Test", &["general"]);

        let pub_key = endpoint_id_to_ed25519_public(&joiner.endpoint_id());
        let code = generate_invite(
            "Join Test",
            &server_id,
            owner.endpoint_id(),
            &keys,
            &topic_map,
            &pub_key,
        );
        assert!(code.is_some(), "generate_invite should produce a value");

        let accepted = accept_invite(&code.unwrap(), &joiner).unwrap();
        assert_eq!(accepted.server_name, "Join Test");
        assert_eq!(accepted.channel_keys.len(), 1);
    }
}
