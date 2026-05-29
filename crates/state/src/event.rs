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

/// Maximum byte length of an [`EventKind::FileMessage`] filename. POSIX
/// `NAME_MAX` aligned with `willow_messaging::MAX_FILENAME_BYTES`.
pub const MAX_ATTACHMENT_FILENAME_BYTES: usize = 255;

/// Maximum byte length of an [`EventKind::FileMessage`] MIME type.
/// Aligned with `willow_messaging::MAX_MIME_BYTES`.
pub const MAX_ATTACHMENT_MIME_BYTES: usize = 255;

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

/// Human-readable permission labels for UI surfaces.
///
/// Centralising the strings here keeps UX wording in one place and
/// avoids leaking the `Debug`-derived variant identifiers (e.g.
/// `SyncProvider`) into role lists, settings panes, or MCP resources.
/// `Debug` remains available for logs and developer-facing output.
impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::SyncProvider => "Sync provider",
            Self::ManageChannels => "Manage channels",
            Self::ManageRoles => "Manage roles",
            Self::SendMessages => "Send messages",
            Self::CreateInvite => "Create invite",
            // Sentinel from a future/unknown client. Should never reach
            // a UI render path in practice — `apply_event` drops these
            // before they enter any role's permission set — but Display
            // must be total, so emit a clearly-flagged placeholder.
            Self::__UnknownLegacy => "Unknown",
        };
        f.write_str(label)
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
    /// Send a chat message that carries a file attachment.
    ///
    /// Distinct variant rather than an extra optional field on
    /// [`EventKind::Message`] so the wire format stays positional /
    /// bincode-friendly. The materialize branch builds a [`ChatMessage`]
    /// with `attachment: Some(_)`; the existing `EventKind::Message`
    /// path keeps producing `attachment: None`.
    ///
    /// `body` carries an optional caption (often empty). `width` /
    /// `height` are `Some(_)` for image attachments whose dimensions
    /// were extracted at send time and `None` otherwise. Bounded to
    /// [`crate::types::FileAttachment::MAX_DIMENSION_PX`] in the
    /// materialize branch to keep the layout-bombing surface in sync
    /// with `willow_messaging::Content::File`.
    ///
    /// Spec: `docs/specs/2026-04-19-ui-design/files-inline.md`.
    /// Plan: `docs/plans/2026-05-08-ui-phase-3b-files-inline.md`.
    FileMessage {
        channel_id: String,
        /// Content-addressed blob hash; receivers fetch via the
        /// [`willow_network::BlobStore`] trait.
        hash: String,
        /// Original filename, capped at [`MAX_ATTACHMENT_FILENAME_BYTES`].
        filename: String,
        /// MIME type (`image/png`, `application/pdf`, …), capped at
        /// [`MAX_ATTACHMENT_MIME_BYTES`].
        mime_type: String,
        /// Sender-declared file size. Attacker-declared — the renderer
        /// MUST treat this as a hint, not a trust value.
        size_bytes: u64,
        /// Image width in pixels, when known. `None` for non-images.
        width: Option<u32>,
        /// Image height in pixels, when known. `None` for non-images.
        height: Option<u32>,
        /// Optional caption / comment displayed alongside the attachment.
        body: String,
        /// If this is a reply, the parent message hash.
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

impl EventKind {
    /// The channel id this event carries, if any.
    ///
    /// Returns `Some` for every channel-bearing kind — chat *and* channel
    /// lifecycle / key rotation / per-channel mute. This is the general
    /// accessor; the heads-based sync filter uses the narrower
    /// [`EventKind::chat_channel_id`] instead, because the spec requires
    /// structural channel events (`CreateChannel`, `RotateChannelKey`, …) to
    /// reconcile regardless of a channel filter.
    pub fn channel_id(&self) -> Option<&str> {
        match self {
            EventKind::Message { channel_id, .. }
            | EventKind::FileMessage { channel_id, .. }
            | EventKind::CreateChannel { channel_id, .. }
            | EventKind::DeleteChannel { channel_id, .. }
            | EventKind::RenameChannel { channel_id, .. }
            | EventKind::ChannelRevive { channel_id, .. }
            | EventKind::RotateChannelKey { channel_id, .. }
            | EventKind::PinMessage { channel_id, .. }
            | EventKind::UnpinMessage { channel_id, .. }
            | EventKind::MuteChannel { channel_id, .. } => Some(channel_id),
            _ => None,
        }
    }

    /// The channel id of a **chat-shaped** event, if this is one.
    ///
    /// Returns `Some` only for kinds that represent conversation content within
    /// a channel — `Message`, `FileMessage`, `PinMessage`, `UnpinMessage` — and
    /// `None` for everything else, *including* channel-structural kinds
    /// (`CreateChannel`, `DeleteChannel`, `RenameChannel`, `ChannelRevive`,
    /// `RotateChannelKey`) and the per-identity `MuteChannel`.
    ///
    /// This is the predicate the heads-based sync `channels` filter keys off:
    /// per `docs/specs/2026-04-24-negentropy-sync.md` § Filter semantics, the
    /// channel filter narrows chat-shaped kinds only, while structural events
    /// always reconcile so server structure (and key epochs) stay complete.
    pub fn chat_channel_id(&self) -> Option<&str> {
        match self {
            EventKind::Message { channel_id, .. }
            | EventKind::FileMessage { channel_id, .. }
            | EventKind::PinMessage { channel_id, .. }
            | EventKind::UnpinMessage { channel_id, .. } => Some(channel_id),
            _ => None,
        }
    }

    /// Stable single-byte discriminant for this variant, used by the
    /// heads-based sync filter's `event_kinds` whitelist.
    ///
    /// bincode is **not** self-describing: it encodes an enum's variant index
    /// as a fixed-width little-endian `u32` prefix, independent of any serde
    /// attribute (which only affects self-describing formats such as JSON).
    /// This returns that index's low byte. The value is stable across releases
    /// because `EventKind` variants are **append-only** (see "Adding a new
    /// EventKind" in `CLAUDE.md`), so an existing variant's index never
    /// changes. The encoding is pinned by
    /// `discriminant_matches_bincode_variant_index_low_byte` in the sync tests.
    pub fn discriminant(&self) -> u8 {
        // The first byte of the bincode encoding is the low byte of the
        // variant index. Serialization of a bare variant cannot fail.
        bincode::serialize(self)
            .ok()
            .and_then(|b| b.first().copied())
            .unwrap_or(0)
    }
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
        // Defense-in-depth: bincode of owned Vec/String/integers shouldn't
        // fail in practice, but `kind` is attacker-controlled. A malformed
        // String produced via `unsafe` could in theory fail to serialize.
        // Reject the event instead of panicking on the hot verify path.
        let bytes = match bincode::serialize(&signable) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(error = %e, "event verify: bincode serialize failed; rejecting");
                return false;
            }
        };

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

    #[test]
    fn verify_returns_false_on_garbage_event() {
        // Defense-in-depth: verify() should never panic on adversarial
        // input, even if the hash and signature are obviously bogus.
        // The bincode-failure branch in verify() is unreachable from safe
        // Rust on the current types (owned Vec/String/integers), so this
        // test exercises the adjacent hash/sig mismatch path to confirm
        // the function returns gracefully instead of panicking.
        let id = Identity::generate();
        let mut event = make_event(&id, test_kind());
        event.hash = EventHash::from_bytes(b"not-the-real-hash");
        event.sig = Signature::from_bytes(&[0u8; 64]);
        assert!(!event.verify());
    }

    #[test]
    fn permission_display_strings() {
        // UI surfaces (role lists, settings, MCP resources) render
        // permissions via Display. Locking these strings here keeps the
        // wording stable and surfaces wording changes as test diffs.
        assert_eq!(Permission::SyncProvider.to_string(), "Sync provider");
        assert_eq!(Permission::ManageChannels.to_string(), "Manage channels");
        assert_eq!(Permission::ManageRoles.to_string(), "Manage roles");
        assert_eq!(Permission::SendMessages.to_string(), "Send messages");
        assert_eq!(Permission::CreateInvite.to_string(), "Create invite");
        assert_eq!(Permission::__UnknownLegacy.to_string(), "Unknown");
    }
}
