//! Exact strings from `profile-card.md` §Copy.
//!
//! All labels are lowercase per the foundation voice rule. Copy in
//! this module is load-bearing for byte-exact browser tests.

pub const MESSAGE: &str = "message";
pub const CALL: &str = "start call";
pub const WHISPER: &str = "whisper";
pub const COPY_FINGERPRINT: &str = "copy fingerprint";
pub const VERIFY: &str = "verify in person";
pub const BLOCK: &str = "block";
pub const EDIT_PROFILE: &str = "edit profile";
pub const SET_NICKNAME: &str = "set nickname";
pub const CHANGE_NICKNAME: &str = "change nickname";
pub const UNVERIFIED_TOOLTIP: &str = "unverified — compare fingerprints before you trust this peer";
pub const VERIFIED_TOOLTIP: &str = "verified peer";
pub const PENDING_TOOLTIP: &str = "compare in progress · resume →";
pub const SELF_CAPTION: &str = "this is you";
pub const QUEUED_PREFIX: &str = "queued ·";
pub const WHISPER_STATUS: &str = "whispering";
pub const FINGERPRINT_LABEL: &str = "fingerprint";
pub const SINCE_LABEL: &str = "in the grove since";
pub const SHARED_GROVES_LABEL: &str = "you share";
pub const KNOWN_AS_PREFIX: &str = "you call them";
pub const PINNED_LABEL: &str = "pinned fragment";
pub const ELSEWHERE_LABEL: &str = "elsewhere";
pub const EMPTY_PINNED: &str = "no pinned fragment";

/// Role labels referenced cross-spec. The profile card itself never
/// renders a role badge (spec §Role label), but other surfaces mirror
/// these strings.
pub const ROLE_STEWARD: &str = "steward";
pub const ROLE_MEMBER: &str = "member";
pub const ROLE_GUEST: &str = "guest";

/// Status-pill labels — user-visible copy for presence states. The
/// profile card's status pill uses these; never the internal
/// `online / idle / whisper / offline` codewords.
pub const STATUS_HERE: &str = "here";
pub const STATUS_AWAY: &str = "away";
pub const STATUS_WHISPERING: &str = "whispering";
pub const STATUS_GONE: &str = "gone";
