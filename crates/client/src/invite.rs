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

/// Generate a secure invite code encrypted for a specific recipient.
///
/// The `recipient_ed25519_public` is the 32-byte Ed25519 public key of
/// the intended recipient (extracted from their PeerId).
///
/// Returns `None` if encryption fails.
pub fn generate_invite(
    server: &willow_channel::Server,
    keys: &HashMap<String, ChannelKey>,
    topic_map: &HashMap<String, (String, willow_channel::ChannelId)>,
    recipient_ed25519_public: &[u8; 32],
) -> Option<String> {
    let mut channels = Vec::new();

    for (topic, (name, _ch_id)) in topic_map {
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
        server_name: server.name().to_string(),
        server_id: server.id().to_string(),
        genesis_author: server.creator,
        sync_providers: Vec::new(), // populated by caller if known
        channels,
    };

    let bytes = willow_transport::pack(&payload).ok()?;
    Some(crate::base64::encode(&bytes))
}

/// Parse an invite code and decrypt the channel keys using our identity.
///
/// Returns the server info and decrypted channel keys, or `None` if the
/// code is invalid or we're not the intended recipient.
pub fn accept_invite(
    code: &str,
    our_identity: &willow_identity::Identity,
) -> Option<AcceptedInvite> {
    let bytes = crate::base64::decode(code.trim())?;
    let payload: InvitePayload = willow_transport::unpack(&bytes).ok()?;

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

    #[test]
    fn secure_invite_round_trip() {
        use willow_channel::ChannelKind;

        let owner = Identity::generate();
        let recipient = Identity::generate();

        // Owner creates a server with a channel.
        let mut server = willow_channel::Server::new("Secure Server", owner.endpoint_id());
        let ch_id = server.create_channel("general", ChannelKind::Text).unwrap();

        let mut keys = HashMap::new();
        let mut topic_map = HashMap::new();
        let topic = format!("{}/general", server.id());

        if let Some(key) = server.channel_key(&ch_id) {
            keys.insert(topic.clone(), key.clone());
        }
        topic_map.insert(topic.clone(), ("general".into(), ch_id));

        // Get recipient's Ed25519 public key bytes.
        let recipient_pub = recipient_public_bytes(&recipient);

        // Generate invite encrypted for the recipient.
        let code = generate_invite(&server, &keys, &topic_map, &recipient_pub).unwrap();

        // Recipient accepts the invite.
        let accepted = accept_invite(&code, &recipient).unwrap();

        assert_eq!(accepted.server_name, "Secure Server");
        assert_eq!(accepted.channel_keys.len(), 1);

        // Verify the decrypted key matches the original.
        let (name, decrypted_key) = &accepted.channel_keys[&topic];
        assert_eq!(name, "general");
        assert_eq!(decrypted_key.as_bytes(), keys[&topic].as_bytes());
    }

    #[test]
    fn wrong_recipient_cannot_decrypt() {
        use willow_channel::ChannelKind;

        let owner = Identity::generate();
        let intended = Identity::generate();
        let intruder = Identity::generate();

        let mut server = willow_channel::Server::new("Secure", owner.endpoint_id());
        let ch_id = server.create_channel("secret", ChannelKind::Text).unwrap();

        let mut keys = HashMap::new();
        let mut topic_map = HashMap::new();
        let topic = format!("{}/secret", server.id());

        if let Some(key) = server.channel_key(&ch_id) {
            keys.insert(topic.clone(), key.clone());
        }
        topic_map.insert(topic, ("secret".into(), ch_id));

        let intended_pub = recipient_public_bytes(&intended);
        let code = generate_invite(&server, &keys, &topic_map, &intended_pub).unwrap();

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
        use willow_channel::ChannelKind;

        let owner = Identity::generate();
        let recipient = Identity::generate();

        let mut server = willow_channel::Server::new("Multi", owner.endpoint_id());
        let ch1 = server.create_channel("general", ChannelKind::Text).unwrap();
        let ch2 = server.create_channel("random", ChannelKind::Text).unwrap();
        let ch3 = server.create_channel("voice", ChannelKind::Voice).unwrap();

        let mut keys = HashMap::new();
        let mut topic_map = HashMap::new();

        for (ch_id, name) in [(ch1, "general"), (ch2, "random"), (ch3, "voice")] {
            let topic = format!("{}/{name}", server.id());
            if let Some(key) = server.channel_key(&ch_id) {
                keys.insert(topic.clone(), key.clone());
            }
            topic_map.insert(topic, (name.into(), ch_id));
        }

        let recipient_pub = recipient_public_bytes(&recipient);
        let code = generate_invite(&server, &keys, &topic_map, &recipient_pub).unwrap();
        let accepted = accept_invite(&code, &recipient).unwrap();

        assert_eq!(accepted.channel_keys.len(), 3);
    }

    /// Helper to extract Ed25519 public key bytes from an Identity.
    fn recipient_public_bytes(identity: &Identity) -> [u8; 32] {
        *identity.endpoint_id().as_bytes()
    }

    #[test]
    fn generate_invite_via_endpoint_id_produces_valid_invite() {
        use willow_channel::ChannelKind;

        let owner = Identity::generate();
        let joiner = Identity::generate();

        let mut server = willow_channel::Server::new("Join Test", owner.endpoint_id());
        let ch_id = server.create_channel("general", ChannelKind::Text).unwrap();

        let mut keys = HashMap::new();
        let mut topic_map = HashMap::new();
        let topic = format!("{}/general", server.id());

        if let Some(key) = server.channel_key(&ch_id) {
            keys.insert(topic.clone(), key.clone());
        }
        topic_map.insert(topic.clone(), ("general".into(), ch_id));

        // Use endpoint_id_to_ed25519_public — same path as JoinRequest handler.
        let pub_key = endpoint_id_to_ed25519_public(&joiner.endpoint_id());
        let code = generate_invite(&server, &keys, &topic_map, &pub_key);
        assert!(code.is_some(), "generate_invite should produce a value");

        // Joiner can accept and decrypt the invite.
        let accepted = accept_invite(&code.unwrap(), &joiner).unwrap();
        assert_eq!(accepted.server_name, "Join Test");
        assert_eq!(accepted.channel_keys.len(), 1);
    }
}
