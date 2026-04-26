//! Pure data types for the event-sourced state machine.
//!
//! These types hold the shared state of a server without any UI framework,
//! networking, or crypto dependency. They are the building blocks of
//! [`ServerState`](crate::server::ServerState).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;

use crate::hash::EventHash;

/// Channel kind — text chat or voice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    /// A text chat channel (default).
    #[default]
    #[serde(alias = "text")]
    Text,
    /// A voice (and optionally video/screenshare) channel.
    #[serde(alias = "voice")]
    Voice,
}

/// A named conversation space inside a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// Unique ID (UUID string).
    pub id: String,
    /// Display name (e.g. "general").
    pub name: String,
    /// Hashes of pinned messages in this channel.
    #[serde(default)]
    pub pinned_messages: BTreeSet<EventHash>,
    /// Text or voice.
    #[serde(default)]
    pub kind: ChannelKind,
    /// `None` for permanent channels. When set, the channel is
    /// non-permanent and auto-archives after `idle_threshold_ms` of
    /// inactivity (see
    /// `docs/specs/2026-04-19-ui-design/ephemeral-channels.md`).
    #[serde(default)]
    pub ephemeral: Option<crate::ephemeral::EphemeralConfig>,
    /// Latest message HLC (physical millisecond, sourced from the
    /// event's `timestamp_hint_ms`). `None` until the first message
    /// lands. Tracked unconditionally — permanent channels carry it
    /// too so the materialize branch stays simple.
    #[serde(default)]
    pub last_activity_hlc: Option<u64>,
}

/// A named bundle of permissions that can be assigned to members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    /// Unique ID (UUID string).
    pub id: String,
    /// Human-readable name (e.g. "Moderator").
    pub name: String,
    /// The set of permission strings this role grants.
    pub permissions: BTreeSet<String>,
}

/// A peer's membership record within a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// The peer's endpoint ID.
    pub peer_id: EndpointId,
    /// Role IDs assigned to this member.
    pub roles: BTreeSet<String>,
    /// Optional display name override.
    pub display_name: Option<String>,
}

/// A single chat message with metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique message ID — the EventHash of the Message event that created it.
    pub id: EventHash,
    /// The channel this message belongs to.
    pub channel_id: String,
    /// Author's endpoint ID.
    pub author: EndpointId,
    /// Message body text.
    pub body: String,
    /// Wall-clock timestamp hint in milliseconds (display only).
    pub timestamp_ms: u64,
    /// Whether this message has been edited.
    pub edited: bool,
    /// Whether this message has been soft-deleted.
    pub deleted: bool,
    /// Reactions: emoji string -> set of reactor endpoint IDs.
    /// Stored as a `BTreeSet` so each peer can only react once with a
    /// given emoji to a given message.
    pub reactions: BTreeMap<String, BTreeSet<EndpointId>>,
    /// If this is a reply, the EventHash of the parent message.
    pub reply_to: Option<EventHash>,
}

/// A peer's display profile.
///
/// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
/// §Data dependencies. All new fields (pronouns, bio, tagline, crest_*,
/// pinned, elsewhere, since) are `#[serde(default)]` so events
/// serialized before these fields existed still deserialize cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// The peer's endpoint ID.
    pub peer_id: EndpointId,
    /// Display name. Empty string means "never set".
    pub display_name: String,
    /// Short pronouns pill (`she/her`, `they/them`, …). Cap
    /// [`PROFILE_CAP_PRONOUNS`] chars.
    #[serde(default)]
    pub pronouns: Option<String>,
    /// Free-form bio. Cap [`PROFILE_CAP_BIO`] chars.
    #[serde(default)]
    pub bio: Option<String>,
    /// Mono-small tagline rendered below the bio. Cap
    /// [`PROFILE_CAP_TAGLINE`] chars.
    #[serde(default)]
    pub tagline: Option<String>,
    /// Banner crest pattern. Defaults to [`CrestPattern::Leaf`] when unset.
    #[serde(default)]
    pub crest_pattern: Option<CrestPattern>,
    /// RGB hex including leading `#` (e.g. `#6b8e4e`). Exactly 7 chars.
    /// Values that don't match this shape are dropped on apply.
    #[serde(default)]
    pub crest_color: Option<String>,
    /// Pinned fragment — one quote / fragment the peer pins to their card.
    #[serde(default)]
    pub pinned: Option<PinnedFragment>,
    /// Non-identifying freeform "elsewhere" labels. Cap
    /// [`PROFILE_CAP_ELSEWHERE_LEN`] × [`PROFILE_CAP_ELSEWHERE_ENTRY`] chars.
    #[serde(default)]
    pub elsewhere: Vec<String>,
    /// Soft-time hint (`spring · yr 2`). Cap [`PROFILE_CAP_SINCE`] chars.
    #[serde(default)]
    pub since: Option<String>,
}

impl Profile {
    /// Construct an empty profile for a peer with all optional fields unset.
    ///
    /// [`EndpointId`] has no `Default` impl (it wraps a 32-byte public
    /// key), so `Profile` can't derive `Default` either. This constructor
    /// keeps the upsert path in `apply_event(UpdateProfile)` concise.
    pub fn new(peer_id: EndpointId) -> Self {
        Self {
            peer_id,
            display_name: String::new(),
            pronouns: None,
            bio: None,
            tagline: None,
            crest_pattern: None,
            crest_color: None,
            pinned: None,
            elsewhere: Vec::new(),
            since: None,
        }
    }
}

/// Procedural crest patterns for the profile card banner.
///
/// Deterministic SVG seeded by peer id; rendered in the UI layer
/// (`crates/web/src/profile/crest.rs`). Three patterns keep the visual
/// language small while still giving every peer a distinct banner.
///
/// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
/// §Crest banner.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrestPattern {
    /// 14 vertical frond strokes with seeded sway.
    Fronds,
    /// Six scattered circles + two concentric centre rings.
    Rings,
    /// Long ogee sweep with nine pendant leaves — spec default per
    /// `profile-card.md` §Missing / default.
    #[default]
    Leaf,
}

/// Shape of a pinned fragment: a literal quote or a freeform fragment.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinnedKind {
    /// Wrapped in curly quotation marks by the renderer.
    Quote,
    /// Rendered plain.
    Fragment,
}

/// A single pinned fragment on a profile card.
///
/// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md` §Field
/// inventory, row 11. v1 stores exactly one fragment per profile; the
/// shape is reserved for a future list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedFragment {
    /// Quote vs. fragment styling hint.
    pub kind: PinnedKind,
    /// Body text — cap [`PROFILE_CAP_PINNED_BODY`] chars enforced on
    /// `UpdateProfile` apply.
    pub body: String,
}

/// Profile-field delta payload carried by
/// [`crate::event::EventKind::UpdateProfile`].
///
/// Each outer `Option` is "unchanged when `None`", "overwrite when
/// `Some`". For nullable fields (`pronouns`, `bio`, `tagline`,
/// `crest_pattern`, `crest_color`, `pinned`, `since`) the inner
/// `Option` carries the "clear vs. set" distinction.
///
/// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
/// §Data dependencies.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileDelta {
    pub display_name: Option<String>,
    pub pronouns: Option<Option<String>>,
    pub bio: Option<Option<String>>,
    pub tagline: Option<Option<String>>,
    pub crest_pattern: Option<Option<CrestPattern>>,
    pub crest_color: Option<Option<String>>,
    pub pinned: Option<Option<PinnedFragment>>,
    pub elsewhere: Option<Vec<String>>,
    pub since: Option<Option<String>>,
}

/// Per-field caps enforced by `apply_event(UpdateProfile)`.
///
/// Values above the cap are silently truncated on apply rather than
/// rejecting the event — so a misbehaving client cannot DoS the DAG by
/// broadcasting over-long strings.
pub const PROFILE_CAP_PRONOUNS: usize = 32;
pub const PROFILE_CAP_BIO: usize = 240;
pub const PROFILE_CAP_TAGLINE: usize = 80;
pub const PROFILE_CAP_CREST_COLOR: usize = 7;
pub const PROFILE_CAP_PINNED_BODY: usize = 280;
pub const PROFILE_CAP_ELSEWHERE_ENTRY: usize = 48;
pub const PROFILE_CAP_ELSEWHERE_LEN: usize = 4;
pub const PROFILE_CAP_SINCE: usize = 32;

/// Per-identity mute state for one grove.
///
/// Stored on `ServerState::mute_state` keyed by `EndpointId`. Muting
/// silences the author's own notifications only — it is never
/// advertised to peers, so there is no authority check in
/// `apply_event` for `MuteChannel` / `MuteGrove`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MuteState {
    /// Explicitly-muted channel IDs. Membership in this set means
    /// "suppress notifications for this channel."
    pub channels: std::collections::HashSet<String>,
    /// True if the entire grove is muted (supersedes per-channel
    /// entries). A muted grove still emits unread counts so the
    /// badge layer can render the outlined muted pill.
    pub grove_muted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_peer() -> EndpointId {
        EndpointId::from_bytes(&[0u8; 32]).unwrap()
    }

    #[test]
    fn profile_new_has_empty_optional_fields() {
        let p = Profile::new(zero_peer());
        assert!(p.display_name.is_empty());
        assert!(p.pronouns.is_none());
        assert!(p.bio.is_none());
        assert!(p.tagline.is_none());
        assert!(p.crest_pattern.is_none());
        assert!(p.crest_color.is_none());
        assert!(p.pinned.is_none());
        assert!(p.elsewhere.is_empty());
        assert!(p.since.is_none());
    }

    #[test]
    fn crest_pattern_default_is_leaf() {
        assert_eq!(CrestPattern::default(), CrestPattern::Leaf);
    }

    #[test]
    fn profile_new_round_trips_through_bincode() {
        // Wire format is bincode; confirm the new optional-heavy shape
        // encodes + decodes cleanly.
        let before = Profile::new(zero_peer());
        let bytes = bincode::serialize(&before).unwrap();
        let after: Profile = bincode::deserialize(&bytes).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn profile_with_populated_fields_round_trips() {
        let p = Profile {
            peer_id: zero_peer(),
            display_name: "mira".into(),
            pronouns: Some("she/her".into()),
            bio: Some("gardener".into()),
            tagline: Some("tending the moss".into()),
            crest_pattern: Some(CrestPattern::Fronds),
            crest_color: Some("#6b8e4e".into()),
            pinned: Some(PinnedFragment {
                kind: PinnedKind::Quote,
                body: "quiet is a kind of music".into(),
            }),
            elsewhere: vec!["coast · west".into()],
            since: Some("spring · yr 2".into()),
        };
        let bytes = bincode::serialize(&p).unwrap();
        let p2: Profile = bincode::deserialize(&bytes).unwrap();
        assert_eq!(p, p2);
    }
}
