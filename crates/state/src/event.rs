//! Core event types for the per-author Merkle-DAG state machine.
//!
//! Every mutation to shared state is represented as a signed [`Event`]
//! containing an [`EventKind`]. Events are content-addressed — their
//! identity is their SHA-256 hash.

use serde::{Deserialize, Deserializer, Serialize};
use willow_identity::{EndpointId, Identity, Signature};

use crate::hash::EventHash;

// ───── Vector caps (anti-DoS) ──────────────────────────────────────────────
//
// These caps bound per-event memory growth so a single misbehaving peer
// holding a permission can't blow up every other peer's heap by emitting
// pathologically large vectors. See SEC-V-07 (#236).

/// Maximum number of cross-author causal hashes an event may carry in
/// `deps`. Legitimate events reference at most a handful of recent
/// other-author heads; 64 is comfortably above that ceiling and keeps
/// the per-event payload small.
pub const MAX_EVENT_DEPS: usize = 64;

/// Maximum byte length of a single encrypted-channel-key blob inside
/// `EventKind::RotateChannelKey.encrypted_keys`. One X25519-sealed
/// channel key fits well under 128 bytes (32-byte ciphertext + tag +
/// ephemeral pubkey = ~80 bytes); 128 leaves slack without giving a
/// hostile author room to bloat each entry.
pub const MAX_ENCRYPTED_KEY_BYTES: usize = 128;

/// Slack added to the current member count when capping
/// `RotateChannelKey.encrypted_keys.len()`. The legitimate ceiling is
/// "one entry per current member"; epsilon absorbs benign races between
/// membership changes and key-rotation construction.
pub const MAX_ENCRYPTED_KEYS_OVER_MEMBERS: usize = 4;

// ───── Permission ──────────────────────────────────────────────────────────

/// Permission types that can be granted directly by any admin.
///
/// Does NOT include admin status — that is managed exclusively through
/// [`ProposedAction`] and the vote path. This structural separation makes
/// it impossible for any peer to grant admin via a direct
/// [`EventKind::GrantPermission`] event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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
    /// Sentinel for permission names that an unknown / future client
    /// emitted. Reached only via the back-compat string-form deserialize
    /// path (e.g. an MCP tool boundary or a legacy JSON snapshot).
    /// `apply_event` treats this sentinel as a no-op so the event still
    /// joins the DAG — preserving signatures + hash linkage — without
    /// mutating any role's permission set.
    ///
    /// Hidden from generated docs; never emitted by Willow itself.
    #[doc(hidden)]
    #[serde(skip)]
    __UnknownLegacy,
}

impl Permission {
    /// Try to parse a permission name from its string form.
    ///
    /// Returns `None` for unknown names. Used by the agent MCP tool
    /// boundary, which rejects unknown permissions before they enter
    /// the DAG.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "SyncProvider" => Some(Self::SyncProvider),
            "ManageChannels" => Some(Self::ManageChannels),
            "ManageRoles" => Some(Self::ManageRoles),
            "SendMessages" => Some(Self::SendMessages),
            "CreateInvite" => Some(Self::CreateInvite),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Permission {
    /// Custom deserialize that accepts both the enum form (default for
    /// any format — bincode emits a u32 discriminant, JSON emits the
    /// variant name) and tolerates unknown variant names by mapping
    /// them to the [`Permission::__UnknownLegacy`] sentinel.
    ///
    /// This lets a peer running an older or rogue client that broadcast
    /// an unrecognised permission name still have its event join the
    /// DAG; `apply_event` then silently drops the unknown perm so the
    /// role's permission set is never polluted.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PermissionVisitor;

        impl<'de> serde::de::Visitor<'de> for PermissionVisitor {
            type Value = Permission;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a Permission variant")
            }

            // String form (JSON, MCP boundary, legacy snapshots).
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Permission, E> {
                Ok(Permission::from_name(v).unwrap_or_else(|| {
                    tracing::warn!(
                        permission = %v,
                        "unknown permission name; mapping to __UnknownLegacy",
                    );
                    Permission::__UnknownLegacy
                }))
            }

            // Owned-string variant — same handling.
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Permission, E> {
                self.visit_str(&v)
            }

            // Bincode encodes a unit-variant enum as the discriminant
            // index via `deserialize_enum`, which routes through this
            // method. Forward to the standard derived parser.
            fn visit_enum<A>(self, data: A) -> Result<Permission, A::Error>
            where
                A: serde::de::EnumAccess<'de>,
            {
                use serde::de::VariantAccess;
                #[derive(Deserialize)]
                enum Tag {
                    SyncProvider,
                    ManageChannels,
                    ManageRoles,
                    SendMessages,
                    CreateInvite,
                }
                let (tag, variant) = data.variant::<Tag>()?;
                variant.unit_variant()?;
                Ok(match tag {
                    Tag::SyncProvider => Permission::SyncProvider,
                    Tag::ManageChannels => Permission::ManageChannels,
                    Tag::ManageRoles => Permission::ManageRoles,
                    Tag::SendMessages => Permission::SendMessages,
                    Tag::CreateInvite => Permission::CreateInvite,
                })
            }
        }

        if deserializer.is_human_readable() {
            // JSON-style formats encode unit variants as strings; route
            // through the visitor so unknown names hit the sentinel.
            deserializer.deserialize_any(PermissionVisitor)
        } else {
            // Bincode-style formats encode unit variants as discriminant
            // indices; route through the enum visitor.
            deserializer.deserialize_enum(
                "Permission",
                &[
                    "SyncProvider",
                    "ManageChannels",
                    "ManageRoles",
                    "SendMessages",
                    "CreateInvite",
                ],
                PermissionVisitor,
            )
        }
    }
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

impl std::fmt::Display for ProposedAction {
    /// Render a structural, human-readable description of the action.
    ///
    /// Peer ids are rendered via [`EndpointId`]'s own `Display` (64-char
    /// hex). UI layers that want richer rendering (e.g. resolving a peer
    /// id to a display name) should consume the typed [`ProposedAction`]
    /// directly instead of substring-matching on this string.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProposedAction::GrantAdmin { peer_id } => {
                write!(f, "Grant admin to {peer_id}")
            }
            ProposedAction::RevokeAdmin { peer_id } => {
                write!(f, "Revoke admin from {peer_id}")
            }
            ProposedAction::KickMember { peer_id } => {
                write!(f, "Kick {peer_id}")
            }
            ProposedAction::SetVoteThreshold { threshold } => {
                write!(f, "Set vote threshold to {threshold}")
            }
        }
    }
}

impl std::fmt::Display for VoteThreshold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VoteThreshold::Majority => f.write_str("majority"),
            VoteThreshold::Unanimous => f.write_str("unanimous"),
            VoteThreshold::Count(n) => write!(f, "{n} admins"),
        }
    }
}

// ───── EventKind ───────────────────────────────────────────────────────────

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
        #[serde(default)]
        kind: crate::types::ChannelKind,
        /// `Some` when the channel is non-permanent (auto-archives
        /// after the configured idle threshold). Absence means
        /// permanent. See
        /// `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`.
        #[serde(default)]
        ephemeral: Option<crate::ephemeral::EphemeralConfig>,
    },
    /// Delete a channel by ID.
    DeleteChannel { channel_id: String },
    /// Rename a channel.
    RenameChannel {
        channel_id: String,
        new_name: String,
    },
    /// Revive an auto-archived ephemeral channel without posting a
    /// message. Author must be a member of the server (same gate as
    /// `Message`), but no `SendMessages` permission is required —
    /// a muted member can still un-archive a room they belong to.
    ChannelRevive { channel_id: String },
    /// Create a new role.
    CreateRole { name: String, role_id: String },
    /// Delete a role by ID.
    DeleteRole { role_id: String },
    /// Set or clear a permission on a role.
    SetPermission {
        role_id: String,
        permission: Permission,
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

    /// Overlay one or more profile fields in-place.
    ///
    /// Each *outer* `Option` on [`crate::types::ProfileDelta`] means
    /// "unchanged when `None`", "overwrite when `Some`". For nullable
    /// fields (`pronouns`, `bio`, `tagline`, `crest_pattern`,
    /// `crest_color`, `pinned`, `since`), the inner `Option`
    /// distinguishes "clear when `None`" from "set when `Some(value)`".
    ///
    /// The delta is [`Box`]ed because [`EventKind`] is stored inline in
    /// `WireMessage::Event` and clippy's `large_enum_variant` lint
    /// keeps the enum below the 200-byte threshold.
    ///
    /// Permission: self-authorship only (same contract as
    /// [`EventKind::SetProfile`]). No permission check is performed —
    /// the author signs for themselves.
    ///
    /// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
    /// §Data dependencies.
    UpdateProfile(Box<crate::types::ProfileDelta>),

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

    // -- Per-identity mute (not admin-gated) --
    /// Mute or unmute a channel for the *author only*.
    ///
    /// Mute is a per-identity notification gate — it never changes
    /// what other peers see. There is no permission check: any member
    /// can silence their own channel pills.
    MuteChannel { channel_id: String, muted: bool },
    /// Mute or unmute the entire grove for the author only.
    ///
    /// A muted grove silences toasts / push / sound across every
    /// channel in scope. Badges still increment so the user sees
    /// *something is here*.
    MuteGrove { muted: bool },
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
