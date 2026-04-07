//! Core event types for the per-author Merkle-DAG state machine.
//!
//! Every mutation to shared state is represented as a signed [`Event`]
//! containing an [`EventKind`]. Events are content-addressed — their
//! identity is their SHA-256 hash.

use serde::{Deserialize, Serialize};
use willow_identity::{EndpointId, Identity, Signature};

use crate::hash::EventHash;

// ───── Permission ──────────────────────────────────────────────────────────

/// Permission types that can be granted directly by any admin.
///
/// Does NOT include admin status — that is managed exclusively through
/// [`ProposedAction`] and the vote path. This structural separation makes
/// it impossible for any peer to grant admin via a direct
/// [`EventKind::GrantPermission`] event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Can sync/provide full history to other peers.
    SyncProvider,
    /// Can manage channels (create, delete, rename).
    ManageChannels,
    /// Can manage roles and non-admin permissions.
    ManageRoles,
    /// Can send messages, edit, delete, react. Required for
    /// Message, EditMessage, DeleteMessage, and Reaction events.
    SendMessages,
    /// Can create invites.
    CreateInvite,
}

// ───── Governance types ────────────────────────────────────────────────────

/// Actions that require admin vote to take effect.
///
/// This enum defines EXACTLY which actions must go through the vote path.
/// These actions cannot be triggered any other way — the data model makes
/// direct execution structurally impossible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposedAction {
    /// Grant admin status to a peer.
    GrantAdmin { peer_id: EndpointId },
    /// Revoke admin status from a peer.
    RevokeAdmin { peer_id: EndpointId },
    /// Remove a member from the server.
    KickMember { peer_id: EndpointId },
    /// Change the vote threshold for admin actions.
    SetVoteThreshold { threshold: VoteThreshold },
}

/// Vote threshold for admin governance actions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoteThreshold {
    /// More than half of admins must approve (default).
    #[default]
    Majority,
    /// All admins must approve.
    Unanimous,
    /// A specific count of admins must approve (capped at admin count).
    Count(u32),
}

// ───── EventKind ───────────────────────────────────────────────────────────

/// Default channel kind for deserialization backward compatibility.
fn default_create_channel_kind() -> String {
    "text".to_string()
}

/// All possible state mutations — 22 variants.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum EventKind {
    // -- Server lifecycle --
    /// Genesis event: creates the server. Must be the first event in the DAG.
    CreateServer { name: String },

    // -- Governance (vote-based, auto-apply on threshold) --
    /// Propose a privileged action for admin vote.
    Propose { action: ProposedAction },
    /// Vote on a proposal. The `proposal` field is the EventHash of the
    /// Propose event being voted on — structurally binding the vote to a
    /// specific proposal.
    Vote { proposal: EventHash, accept: bool },

    // -- Permissions (direct, by any admin) --
    /// Grant a non-admin permission to a peer.
    GrantPermission {
        peer_id: EndpointId,
        permission: Permission,
    },
    /// Revoke a non-admin permission from a peer.
    RevokePermission {
        peer_id: EndpointId,
        permission: Permission,
    },

    // -- Server structure --
    /// Create a new channel.
    CreateChannel {
        name: String,
        channel_id: String,
        #[serde(default = "default_create_channel_kind")]
        kind: String,
    },
    /// Delete a channel by ID.
    DeleteChannel { channel_id: String },
    /// Rename a channel.
    RenameChannel {
        channel_id: String,
        new_name: String,
    },
    /// Create a new role.
    CreateRole { name: String, role_id: String },
    /// Delete a role by ID.
    DeleteRole { role_id: String },
    /// Set or clear a permission on a role.
    SetPermission {
        role_id: String,
        permission: String,
        granted: bool,
    },
    /// Assign a role to a member.
    AssignRole {
        peer_id: EndpointId,
        role_id: String,
    },

    // -- Chat --
    /// Send a chat message.
    Message {
        channel_id: String,
        body: String,
        reply_to: Option<EventHash>,
    },
    /// Edit a previously sent message.
    EditMessage {
        message_id: EventHash,
        new_body: String,
    },
    /// Soft-delete a message (preserves history).
    DeleteMessage { message_id: EventHash },
    /// Add a reaction to a message.
    Reaction {
        message_id: EventHash,
        emoji: String,
    },

    // -- Identity --
    /// Set or update the author's display name.
    SetProfile { display_name: String },

    // -- Encryption --
    /// Rotate a channel's encryption key.
    RotateChannelKey {
        channel_id: String,
        encrypted_keys: Vec<(EndpointId, Vec<u8>)>,
    },

    // -- Pinning --
    /// Pin a message in a channel.
    PinMessage {
        channel_id: String,
        message_id: EventHash,
    },
    /// Unpin a message in a channel.
    UnpinMessage {
        channel_id: String,
        message_id: EventHash,
    },

    // -- Server metadata (any admin) --
    /// Rename the server.
    RenameServer { new_name: String },
    /// Set the server description.
    SetServerDescription { description: String },
}

// ───── Event ───────────────────────────────────────────────────────────────

/// A single state mutation, content-addressed and author-signed.
///
/// The `hash` field is the SHA-256 of the signable content (all fields
/// except `hash` and `sig`). The `sig` field is the Ed25519 signature
/// over that same content.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    /// Content hash — SHA-256 of the signable fields. This IS the event's
    /// identity.
    pub hash: EventHash,
    /// Author's public key (Ed25519).
    pub author: EndpointId,
    /// Monotonically increasing sequence number within this author's chain.
    /// Starts at 1.
    pub seq: u64,
    /// Hash of this author's previous event (`EventHash::ZERO` for seq=1).
    pub prev: EventHash,
    /// Hashes of events from OTHER authors that this event has "seen."
    /// Advisory, not exhaustive — soft-accepted even if deps are unknown.
    pub deps: Vec<EventHash>,
    /// The state mutation to apply.
    pub kind: EventKind,
    /// Ed25519 signature over the signable content.
    pub sig: Signature,
    /// Wall-clock timestamp hint (ms). Display only — never used for ordering.
    pub timestamp_hint_ms: u64,
}

/// The signable content of an event — everything except `hash` and `sig`.
#[derive(Serialize)]
struct SignableContent<'a> {
    author: &'a EndpointId,
    seq: u64,
    prev: &'a EventHash,
    deps: &'a [EventHash],
    kind: &'a EventKind,
    timestamp_hint_ms: u64,
}

impl Event {
    /// Create a new signed event.
    ///
    /// Computes the content hash and signs with the identity's private key.
    pub fn new(
        identity: &Identity,
        seq: u64,
        prev: EventHash,
        deps: Vec<EventHash>,
        kind: EventKind,
        timestamp_hint_ms: u64,
    ) -> Self {
        let author = identity.endpoint_id();
        let signable = SignableContent {
            author: &author,
            seq,
            prev: &prev,
            deps: &deps,
            kind: &kind,
            timestamp_hint_ms,
        };
        let bytes = bincode::serialize(&signable).expect("event serialization should not fail");
        let hash = EventHash::from_bytes(&bytes);
        let sig = identity.sign(&bytes);

        Self {
            hash,
            author,
            seq,
            prev,
            deps,
            kind,
            sig,
            timestamp_hint_ms,
        }
    }

    /// Verify the event's signature against its content.
    pub fn verify(&self) -> bool {
        let signable = SignableContent {
            author: &self.author,
            seq: self.seq,
            prev: &self.prev,
            deps: &self.deps,
            kind: &self.kind,
            timestamp_hint_ms: self.timestamp_hint_ms,
        };
        let bytes = bincode::serialize(&signable).expect("event serialization should not fail");

        // Verify hash matches content.
        if self.hash != EventHash::from_bytes(&bytes) {
            return false;
        }

        // Verify signature.
        willow_identity::verify(&self.author, &bytes, &self.sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(identity: &Identity, kind: EventKind) -> Event {
        Event::new(identity, 1, EventHash::ZERO, vec![], kind, 1000)
    }

    fn test_kind() -> EventKind {
        EventKind::CreateServer {
            name: "test".into(),
        }
    }

    #[test]
    fn event_hash_is_deterministic() {
        let id = Identity::generate();
        let e1 = make_event(&id, test_kind());
        let e2 = make_event(&id, test_kind());
        assert_eq!(e1.hash, e2.hash);
    }

    #[test]
    fn event_hash_changes_with_any_field() {
        let id = Identity::generate();
        let base = make_event(&id, test_kind());

        // Different seq.
        let different_seq = Event::new(&id, 2, base.hash, vec![], test_kind(), 1000);
        assert_ne!(base.hash, different_seq.hash);

        // Different kind.
        let different_kind = Event::new(
            &id,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::SetProfile {
                display_name: "alice".into(),
            },
            1000,
        );
        assert_ne!(base.hash, different_kind.hash);

        // Different timestamp.
        let different_ts = Event::new(&id, 1, EventHash::ZERO, vec![], test_kind(), 9999);
        assert_ne!(base.hash, different_ts.hash);

        // Different author.
        let other = Identity::generate();
        let different_author = make_event(&other, test_kind());
        assert_ne!(base.hash, different_author.hash);

        // Different deps.
        let different_deps = Event::new(
            &id,
            1,
            EventHash::ZERO,
            vec![EventHash::from_bytes(b"dep")],
            test_kind(),
            1000,
        );
        assert_ne!(base.hash, different_deps.hash);
    }

    #[test]
    fn event_signature_verifies() {
        let id = Identity::generate();
        let event = make_event(&id, test_kind());
        assert!(event.verify());
    }

    #[test]
    fn event_signature_rejects_tampered() {
        let id = Identity::generate();
        let mut event = make_event(&id, test_kind());
        // Tamper with the kind after signing.
        event.kind = EventKind::SetProfile {
            display_name: "hacked".into(),
        };
        assert!(!event.verify());
    }

    #[test]
    fn event_signature_rejects_wrong_key() {
        let id_a = Identity::generate();
        let id_b = Identity::generate();
        let mut event = make_event(&id_a, test_kind());
        // Replace author with a different key (but keep the original sig).
        event.author = id_b.endpoint_id();
        assert!(!event.verify());
    }
}
