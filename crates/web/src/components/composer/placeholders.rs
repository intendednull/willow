//! Composer placeholder copy per channel kind / connection state.
//!
//! Pure function with no DOM or signal dependencies — single source of
//! truth for the strings listed in
//! `docs/specs/2026-04-19-ui-design/composer.md` §Composer placeholders.
//!
//! The spec enumerates four placeholder forms:
//!
//! | Context | Copy |
//! |---|---|
//! | Channel | `message #{channel} — encrypted to {N} peers` |
//! | Letter (1:1 DM) | `message {name}` |
//! | Offline | `offline — messages queue until reconnect` |
//! | No channel selected | `choose a channel to start` |
//!
//! Letter form is selected when `recipient_name` is `Some(_)`. The
//! [`ChannelKind`] parameter is plumbed through for forward
//! compatibility (the spec's letter affordance does not yet have a
//! dedicated kind in `willow-state`); when `recipient_name` is `None`
//! the channel form is used regardless of kind.
//!
//! `Offline` connection state takes precedence over every other case
//! except "no channel selected" — there is nothing to queue against.

use willow_state::types::ChannelKind;

use crate::state::ConnectionState;

/// Resolve the textarea placeholder string for the given composer
/// context.
///
/// `connection == ConnectionState::Offline` overrides the kind-specific
/// copy except when no channel is selected. "No channel selected" is
/// detected when `channel_name` is empty **and** `recipient_name` is
/// `None`.
pub fn placeholder_for(
    _kind: ChannelKind,
    channel_name: &str,
    recipient_name: Option<&str>,
    peer_count: usize,
    connection: ConnectionState,
) -> String {
    // No channel and no recipient — nothing to compose into.
    if channel_name.is_empty() && recipient_name.is_none() {
        return "choose a channel to start".to_string();
    }

    if connection == ConnectionState::Offline {
        return "offline — messages queue until reconnect".to_string();
    }

    if let Some(name) = recipient_name {
        return format!("message {name}");
    }

    format!("message #{channel_name} — encrypted to {peer_count} peers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_for_text_channel() {
        let out = placeholder_for(
            ChannelKind::Text,
            "general",
            None,
            3,
            ConnectionState::Connected,
        );
        assert_eq!(out, "message #general — encrypted to 3 peers");
    }

    #[test]
    fn placeholder_for_letter_uses_recipient_name() {
        let out = placeholder_for(
            ChannelKind::Text,
            "",
            Some("mira"),
            1,
            ConnectionState::Connected,
        );
        assert_eq!(out, "message mira");
    }

    #[test]
    fn placeholder_for_offline_overrides_kind() {
        let out = placeholder_for(
            ChannelKind::Text,
            "general",
            None,
            3,
            ConnectionState::Offline,
        );
        assert_eq!(out, "offline — messages queue until reconnect");
    }

    #[test]
    fn placeholder_for_no_channel_selected() {
        let out = placeholder_for(ChannelKind::Text, "", None, 0, ConnectionState::Connected);
        assert_eq!(out, "choose a channel to start");
    }
}
