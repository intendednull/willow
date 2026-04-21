//! # Mention parsing
//!
//! Splits a message body into plain-text and mention segments so the UI
//! can render coloured pills for `@handle` tokens. Resolution order
//! (per `docs/specs/2026-04-19-ui-design/message-row.md` §Mentions):
//!
//! 1. exact handle match (e.g. `mira.forest.1`)
//! 2. first segment of a handle (e.g. `mira` → `mira.forest.1`)
//! 3. display-name match, case-insensitive (e.g. `@mira` → display name `Mira`)
//! 4. literal `@you` → the local peer
//!
//! Tokens that don't resolve fall back to plain text so downstream
//! pipeline stages (code, URL autolink) can still see them.
//!
//! The parser is pure text — no WASM-specific dependencies — so it
//! lives in `willow-client` and can be called both from the web UI
//! and from future `DisplayMessage` projection (see Phase 2a Task 4).

use regex::Regex;
use std::sync::OnceLock;
use willow_identity::EndpointId;

/// One chunk of a parsed message body.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    /// Literal text (including unresolved `@handle` tokens).
    Text(String),
    /// A resolved `@mention`.
    Mention {
        /// What the pill renders (the original captured handle text,
        /// truncated to 28 chars + `…` if longer than 32; overridden
        /// to `"you"` for self-mentions per spec §Mentions).
        label: String,
        /// Full, pre-truncation, pre-self-override handle as captured
        /// from the message body. Used for the `title` attribute on
        /// the rendered pill so hovering a truncated or re-labelled
        /// mention still reveals the full handle.
        full_label: String,
        /// `None` for `@you` when the local peer has no peer id, or
        /// unresolved mentions (not emitted — these become `Text`).
        /// Always `Some(_)` in practice when we emit `Mention`.
        peer_id: Option<EndpointId>,
        /// Whether the mention refers to the local peer.
        is_self: bool,
    },
}

/// A peer known in the current channel, used to resolve `@handle` tokens.
#[derive(Debug, Clone)]
pub struct PeerRef {
    /// Stable peer identity.
    pub peer_id: EndpointId,
    /// Handle like `mira.forest.1` (stored lowercase).
    pub handle: String,
    /// Display name like `Mira` (preserved casing).
    pub display_name: String,
}

/// Max characters to render in a mention pill's label before truncating.
const MAX_LABEL_LEN: usize = 32;
/// When truncating, keep this many characters and append `…`.
const TRUNCATE_KEEP: usize = 28;

/// Cached mention regex: `@handle` where handle starts with a letter
/// and may contain letters, digits, `.`, `_`, `-`. The `(?i)` inline
/// flag is case-insensitive, so `@Mira` matches too; the regex itself
/// keeps a lowercase-only character class to stay aligned with the
/// spec while tolerating user casing.
fn mention_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)@([a-z][a-z0-9._\-]*)").expect("mention regex is valid"))
}

/// Truncate a mention label for display. Pre-truncation string is used
/// for peer-handle matching; only the rendered label is clipped.
fn truncate_label(label: &str) -> String {
    // Count chars, not bytes — handles multi-byte input gracefully.
    if label.chars().count() <= MAX_LABEL_LEN {
        return label.to_string();
    }
    let kept: String = label.chars().take(TRUNCATE_KEEP).collect();
    format!("{kept}…")
}

/// Parse `body` into text + mention segments.
///
/// `peers` is the list of peers in the current channel. `local_peer`
/// identifies the viewer so `@you` resolves and self-mentions can be
/// flagged. Unresolved mentions are left as `Text` segments so other
/// pipeline stages (inline-code, URL autolink) can still inspect them.
pub fn parse_mentions(body: &str, peers: &[PeerRef], local_peer: &EndpointId) -> Vec<Segment> {
    let re = mention_regex();
    let mut segments: Vec<Segment> = Vec::new();
    let mut cursor = 0usize;

    for caps in re.captures_iter(body) {
        let whole = caps.get(0).expect("group 0 always exists");
        let handle_cap = caps.get(1).expect("group 1 is the handle");
        let full_token_start = whole.start();
        let full_token_end = whole.end();
        let raw_handle = handle_cap.as_str();

        // Try to resolve the captured handle against peers / local alias.
        let resolved = resolve_mention(raw_handle, peers, local_peer);

        if let Some((peer_id, is_self)) = resolved {
            // Flush any plain text before this mention.
            if full_token_start > cursor {
                segments.push(Segment::Text(body[cursor..full_token_start].to_string()));
            }
            // Spec §Mentions: self-mentions always render as `@you`,
            // regardless of whether the captured handle matched via
            // exact handle, first segment, or display-name resolution.
            // `full_label` preserves the original capture for the
            // `title` attribute so the user still sees what was typed.
            let full_label = raw_handle.to_string();
            let display_base = if is_self { "you" } else { raw_handle };
            let label = truncate_label(display_base);
            segments.push(Segment::Mention {
                label,
                full_label,
                peer_id: Some(peer_id),
                is_self,
            });
            cursor = full_token_end;
        }
        // Unresolved captures: leave bytes in the text stream; the next
        // iteration (or the final flush) will emit them as Text.
    }

    if cursor < body.len() {
        segments.push(Segment::Text(body[cursor..].to_string()));
    }

    // Empty body → single empty text segment so downstream code can
    // iterate without special-casing.
    if segments.is_empty() {
        segments.push(Segment::Text(String::new()));
    }

    segments
}

/// Resolve a single captured handle to `(peer_id, is_self)`. Follows
/// the order documented on [`parse_mentions`]. Returns `None` when the
/// token doesn't match any known peer, display name, or `@you` alias.
fn resolve_mention(
    raw_handle: &str,
    peers: &[PeerRef],
    local_peer: &EndpointId,
) -> Option<(EndpointId, bool)> {
    let lower = raw_handle.to_lowercase();

    // 4. `@you` → local peer.
    if lower == "you" {
        return Some((*local_peer, true));
    }

    // 1. Exact handle match.
    if let Some(peer) = peers.iter().find(|p| p.handle == lower) {
        let is_self = &peer.peer_id == local_peer;
        return Some((peer.peer_id, is_self));
    }

    // 2. First segment of handle.
    if let Some(peer) = peers.iter().find(|p| {
        p.handle
            .split('.')
            .next()
            .map(|seg| seg == lower)
            .unwrap_or(false)
    }) {
        let is_self = &peer.peer_id == local_peer;
        return Some((peer.peer_id, is_self));
    }

    // 3. Display-name match (case-insensitive).
    if let Some(peer) = peers
        .iter()
        .find(|p| p.display_name.to_lowercase() == lower)
    {
        let is_self = &peer.peer_id == local_peer;
        return Some((peer.peer_id, is_self));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    fn peer(handle: &str, display: &str) -> PeerRef {
        PeerRef {
            peer_id: Identity::generate().endpoint_id(),
            handle: handle.to_string(),
            display_name: display.to_string(),
        }
    }

    #[test]
    fn exact_handle_match() {
        let mira = peer("mira.forest.1", "Mira");
        let local = Identity::generate().endpoint_id();
        let segs = parse_mentions("@mira.forest.1", std::slice::from_ref(&mira), &local);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            Segment::Mention {
                peer_id, is_self, ..
            } => {
                assert_eq!(peer_id.as_ref(), Some(&mira.peer_id));
                assert!(!is_self);
            }
            _ => panic!("expected mention segment, got {:?}", segs[0]),
        }
    }

    #[test]
    fn first_segment_match() {
        let mira = peer("mira.forest.1", "Mira");
        let local = Identity::generate().endpoint_id();
        let segs = parse_mentions("@mira", std::slice::from_ref(&mira), &local);
        assert_eq!(segs.len(), 1);
        assert!(matches!(segs[0], Segment::Mention { is_self: false, .. }));
    }

    #[test]
    fn display_name_match() {
        // Handle doesn't match `mira`, but display name does.
        let mira = peer("miraculous.forest.1", "Mira");
        let local = Identity::generate().endpoint_id();
        let segs = parse_mentions("@mira", std::slice::from_ref(&mira), &local);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            Segment::Mention { peer_id, .. } => {
                assert_eq!(peer_id.as_ref(), Some(&mira.peer_id));
            }
            _ => panic!("expected mention segment, got {:?}", segs[0]),
        }
    }

    #[test]
    fn you_resolves_to_local() {
        let local = Identity::generate().endpoint_id();
        let segs = parse_mentions("ping @you", &[], &local);
        // "ping " + mention
        assert_eq!(segs.len(), 2);
        match &segs[1] {
            Segment::Mention {
                peer_id,
                is_self,
                label,
                full_label,
            } => {
                assert_eq!(peer_id.as_ref(), Some(&local));
                assert!(is_self);
                assert_eq!(label, "you");
                assert_eq!(full_label, "you");
            }
            _ => panic!("expected mention segment, got {:?}", segs[1]),
        }
    }

    #[test]
    fn self_mention_label_is_you() {
        // Spec §Mentions: a handle that resolves to the local peer
        // must render as `@you`, regardless of how it was typed.
        let mira = peer("mira.forest.1", "Mira");
        let local = mira.peer_id;
        let segs = parse_mentions("@mira.forest.1", std::slice::from_ref(&mira), &local);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            Segment::Mention {
                label,
                full_label,
                is_self,
                peer_id,
            } => {
                assert!(is_self, "handle that matches local peer must be is_self");
                assert_eq!(label, "you", "self-mention label must be `you`");
                assert_eq!(
                    full_label, "mira.forest.1",
                    "full_label must preserve original capture"
                );
                assert_eq!(peer_id.as_ref(), Some(&local));
            }
            other => panic!("expected mention segment, got {other:?}"),
        }
    }

    #[test]
    fn self_mention_via_first_segment_label_is_you() {
        // Same rule applies when the self-mention resolves through the
        // first-handle-segment or display-name path, not just exact.
        let mira = peer("mira.forest.1", "Mira");
        let local = mira.peer_id;
        let segs = parse_mentions("@mira", std::slice::from_ref(&mira), &local);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            Segment::Mention {
                label,
                full_label,
                is_self,
                ..
            } => {
                assert!(is_self);
                assert_eq!(label, "you");
                assert_eq!(full_label, "mira");
            }
            other => panic!("expected mention segment, got {other:?}"),
        }
    }

    #[test]
    fn unresolved_stays_text() {
        let mira = peer("mira.forest.1", "Mira");
        let local = Identity::generate().endpoint_id();
        let segs = parse_mentions("@ghostpeer", std::slice::from_ref(&mira), &local);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            Segment::Text(t) => assert_eq!(t, "@ghostpeer"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn long_handle_truncates_label() {
        // 40-char single-segment handle. Matching is on the full handle,
        // but the pill label must clip to 28 chars + `…`.
        let long = "a".repeat(40);
        let peer_ref = peer(&long, "Long");
        let local = Identity::generate().endpoint_id();
        let body = format!("@{long}");
        let segs = parse_mentions(&body, std::slice::from_ref(&peer_ref), &local);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            Segment::Mention {
                label,
                full_label,
                peer_id,
                ..
            } => {
                // Kept 28 chars + one `…`.
                let mut expected: String = "a".repeat(TRUNCATE_KEEP);
                expected.push('…');
                assert_eq!(label, &expected, "label must truncate to 28 chars + …");
                assert_eq!(
                    full_label, &long,
                    "full_label must preserve the untruncated handle"
                );
                assert_eq!(peer_id.as_ref(), Some(&peer_ref.peer_id));
            }
            other => panic!("expected Mention, got {other:?}"),
        }
    }

    #[test]
    fn mention_inside_plain_text() {
        let mira = peer("mira.forest.1", "Mira");
        let local = Identity::generate().endpoint_id();
        let segs = parse_mentions("hello @mira bye", std::slice::from_ref(&mira), &local);
        assert_eq!(segs.len(), 3);
        assert!(matches!(&segs[0], Segment::Text(t) if t == "hello "));
        assert!(matches!(&segs[1], Segment::Mention { .. }));
        assert!(matches!(&segs[2], Segment::Text(t) if t == " bye"));
    }

    #[test]
    fn regex_starts_with_alpha() {
        // `123abc` starts with a digit → regex won't capture → literal stays.
        let mira = peer("123abc", "Numeric");
        let local = Identity::generate().endpoint_id();
        let segs = parse_mentions("@123abc", std::slice::from_ref(&mira), &local);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            Segment::Text(t) => assert_eq!(t, "@123abc"),
            other => panic!("expected Text, got {other:?}"),
        }
    }
}
