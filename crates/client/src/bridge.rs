//! # Event-Op Bridge
//!
//! Bidirectional conversion between the new [`willow_state::Event`] system and
//! the legacy [`Op`](crate::ops::Op) wire format. This keeps the network
//! protocol stable while migrating internal state management to the
//! event-sourced model.

use willow_state::{Event, EventKind};

#[allow(deprecated)]
use crate::ops::Op;

/// Convert a [`willow_state::Event`] to the wire-format [`Op`] for network
/// transmission.
///
/// Returns `None` for event kinds that have no `Op` equivalent (e.g.
/// `RenameChannel`), since those ops don't exist in the legacy protocol.
#[allow(deprecated)]
pub fn event_to_op(event: &Event) -> Option<Op> {
    match &event.kind {
        EventKind::CreateChannel { name, channel_id } => Some(Op::CreateChannel {
            name: name.clone(),
            channel_id: channel_id.clone(),
        }),
        EventKind::DeleteChannel { channel_id } => {
            // The legacy Op::DeleteChannel uses a channel name, but we only
            // have the channel_id. The caller must resolve the name before
            // calling this, or we fall back to the channel_id as the name.
            Some(Op::DeleteChannel {
                name: channel_id.clone(),
            })
        }
        EventKind::CreateRole { name, role_id } => Some(Op::CreateRole {
            name: name.clone(),
            role_id: role_id.clone(),
        }),
        EventKind::DeleteRole { role_id } => Some(Op::DeleteRole {
            role_id: role_id.clone(),
        }),
        EventKind::SetPermission {
            role_id,
            permission,
            granted,
        } => Some(Op::SetPermission {
            role_id: role_id.clone(),
            permission: permission.clone(),
            granted: *granted,
        }),
        EventKind::AssignRole { peer_id, role_id } => Some(Op::AssignRole {
            peer_id: peer_id.clone(),
            role_id: role_id.clone(),
        }),
        EventKind::GrantPermission { peer_id, .. } => Some(Op::TrustPeer {
            peer_id: peer_id.clone(),
        }),
        EventKind::RevokePermission { peer_id, .. } => Some(Op::UntrustPeer {
            peer_id: peer_id.clone(),
        }),
        EventKind::KickMember { peer_id } => Some(Op::KickMember {
            peer_id: peer_id.clone(),
            rotated_keys: vec![],
        }),
        EventKind::SetProfile { .. } => {
            // Profile is broadcast via a separate mechanism, not an Op.
            None
        }
        EventKind::Message { .. }
        | EventKind::EditMessage { .. }
        | EventKind::DeleteMessage { .. }
        | EventKind::Reaction { .. } => {
            // Chat messages use the ChatMessage Op variant with serialized
            // content. The caller constructs these directly, so we don't
            // convert them here.
            None
        }
        EventKind::PinMessage { .. } | EventKind::UnpinMessage { .. } => {
            // No legacy Op equivalent — pin/unpin is event-sourced only.
            None
        }
        EventKind::RenameChannel { .. }
        | EventKind::RotateChannelKey { .. }
        | EventKind::RenameServer { .. }
        | EventKind::SetServerDescription { .. }
        | EventKind::StateVerification { .. } => {
            // No legacy Op equivalent.
            None
        }
    }
}

/// Convert a received [`Op`] into a [`willow_state::Event`] for state
/// application.
///
/// `author` is the peer ID of the op's author, `timestamp_ms` is the
/// wall-clock timestamp, and `op_id` is the unique operation ID.
/// `parent_hash` is the current state hash to use as the event's parent.
///
/// Returns `None` for op kinds that don't map to an event (e.g.
/// `ChatMessage`, which is handled separately).
#[allow(deprecated)]
pub fn op_to_event(
    op: &Op,
    author: &str,
    timestamp_ms: u64,
    op_id: &str,
    parent_hash: willow_state::StateHash,
) -> Option<Event> {
    let kind = match op {
        Op::CreateChannel { name, channel_id } => EventKind::CreateChannel {
            name: name.clone(),
            channel_id: channel_id.clone(),
        },
        Op::DeleteChannel { name } => {
            // Legacy Op uses channel name; we need a channel_id. Use the name
            // as the channel_id since the caller can resolve it.
            EventKind::DeleteChannel {
                channel_id: name.clone(),
            }
        }
        Op::CreateRole { name, role_id } => EventKind::CreateRole {
            name: name.clone(),
            role_id: role_id.clone(),
        },
        Op::DeleteRole { role_id } => EventKind::DeleteRole {
            role_id: role_id.clone(),
        },
        Op::SetPermission {
            role_id,
            permission,
            granted,
        } => EventKind::SetPermission {
            role_id: role_id.clone(),
            permission: permission.clone(),
            granted: *granted,
        },
        Op::AssignRole { peer_id, role_id } => EventKind::AssignRole {
            peer_id: peer_id.clone(),
            role_id: role_id.clone(),
        },
        Op::KickMember { peer_id, .. } => EventKind::KickMember {
            peer_id: peer_id.clone(),
        },
        Op::TrustPeer { peer_id } => EventKind::GrantPermission {
            peer_id: peer_id.clone(),
            permission: willow_state::Permission::Administrator,
        },
        Op::UntrustPeer { peer_id } => EventKind::RevokePermission {
            peer_id: peer_id.clone(),
            permission: willow_state::Permission::Administrator,
        },
        Op::ChatMessage { .. } => {
            // Chat messages are handled separately via process_chat_message.
            return None;
        }
    };

    Some(Event {
        id: op_id.to_string(),
        parent_hash,
        author: author.to_string(),
        timestamp_ms,
        kind,
    })
}

/// Convert a received [`Op::ChatMessage`] into a [`willow_state::Event`]
/// with a `Message` kind.
///
/// This is separate from `op_to_event` because chat messages require
/// resolving the topic to a channel_id and extracting the message body from
/// the serialized content data.
pub fn chat_op_to_event(
    channel_id: &str,
    body: &str,
    author: &str,
    timestamp_ms: u64,
    op_id: &str,
    parent_hash: willow_state::StateHash,
) -> Event {
    Event {
        id: op_id.to_string(),
        parent_hash,
        author: author.to_string(),
        timestamp_ms,
        kind: EventKind::Message {
            channel_id: channel_id.to_string(),
            body: body.to_string(),
        },
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use willow_state::StateHash;

    #[test]
    fn create_channel_round_trip() {
        let event = Event {
            id: "e1".to_string(),
            parent_hash: StateHash::ZERO,
            author: "peer-1".to_string(),
            timestamp_ms: 1000,
            kind: EventKind::CreateChannel {
                name: "general".to_string(),
                channel_id: "ch-1".to_string(),
            },
        };

        let op = event_to_op(&event).unwrap();
        assert!(matches!(
            op,
            Op::CreateChannel { ref name, ref channel_id }
            if name == "general" && channel_id == "ch-1"
        ));

        let event2 = op_to_event(&op, "peer-1", 1000, "e1", StateHash::ZERO).unwrap();
        assert_eq!(event2.author, "peer-1");
        assert!(matches!(
            event2.kind,
            EventKind::CreateChannel { ref name, ref channel_id }
            if name == "general" && channel_id == "ch-1"
        ));
    }

    #[test]
    fn delete_channel_converts() {
        let op = Op::DeleteChannel {
            name: "random".to_string(),
        };
        let event = op_to_event(&op, "peer-1", 2000, "e2", StateHash::ZERO).unwrap();
        assert!(matches!(
            event.kind,
            EventKind::DeleteChannel { ref channel_id } if channel_id == "random"
        ));
    }

    #[test]
    fn trust_peer_maps_to_grant_permission() {
        let op = Op::TrustPeer {
            peer_id: "alice".to_string(),
        };
        let event = op_to_event(&op, "owner", 3000, "e3", StateHash::ZERO).unwrap();
        assert!(matches!(
            event.kind,
            EventKind::GrantPermission {
                ref peer_id,
                ref permission
            } if peer_id == "alice"
                && *permission == willow_state::Permission::Administrator
        ));
    }

    #[test]
    fn untrust_peer_maps_to_revoke_permission() {
        let op = Op::UntrustPeer {
            peer_id: "bob".to_string(),
        };
        let event = op_to_event(&op, "owner", 4000, "e4", StateHash::ZERO).unwrap();
        assert!(matches!(
            event.kind,
            EventKind::RevokePermission {
                ref peer_id,
                ref permission
            } if peer_id == "bob"
                && *permission == willow_state::Permission::Administrator
        ));
    }

    #[test]
    fn kick_member_converts() {
        let op = Op::KickMember {
            peer_id: "eve".to_string(),
            rotated_keys: vec![],
        };
        let event = op_to_event(&op, "owner", 5000, "e5", StateHash::ZERO).unwrap();
        assert!(matches!(
            event.kind,
            EventKind::KickMember { ref peer_id } if peer_id == "eve"
        ));
    }

    #[test]
    fn chat_message_returns_none() {
        let op = Op::ChatMessage {
            topic: "test".to_string(),
            content_data: vec![1, 2, 3],
        };
        assert!(op_to_event(&op, "peer", 6000, "e6", StateHash::ZERO).is_none());
    }

    #[test]
    fn grant_permission_maps_to_trust_op() {
        let event = Event {
            id: "e7".to_string(),
            parent_hash: StateHash::ZERO,
            author: "owner".to_string(),
            timestamp_ms: 7000,
            kind: EventKind::GrantPermission {
                peer_id: "alice".to_string(),
                permission: willow_state::Permission::Administrator,
            },
        };
        let op = event_to_op(&event).unwrap();
        assert!(matches!(op, Op::TrustPeer { ref peer_id } if peer_id == "alice"));
    }

    #[test]
    fn set_profile_returns_none() {
        let event = Event {
            id: "e8".to_string(),
            parent_hash: StateHash::ZERO,
            author: "peer".to_string(),
            timestamp_ms: 8000,
            kind: EventKind::SetProfile {
                display_name: "Alice".to_string(),
            },
        };
        assert!(event_to_op(&event).is_none());
    }

    #[test]
    fn create_role_round_trip() {
        let op = Op::CreateRole {
            name: "Moderator".to_string(),
            role_id: "r1".to_string(),
        };
        let event = op_to_event(&op, "owner", 9000, "e9", StateHash::ZERO).unwrap();
        assert!(matches!(
            event.kind,
            EventKind::CreateRole { ref name, ref role_id }
            if name == "Moderator" && role_id == "r1"
        ));

        let op2 = event_to_op(&event).unwrap();
        assert!(matches!(
            op2,
            Op::CreateRole { ref name, ref role_id }
            if name == "Moderator" && role_id == "r1"
        ));
    }

    #[test]
    fn chat_op_to_event_creates_message() {
        let event = chat_op_to_event(
            "ch-1",
            "hello world",
            "peer-1",
            10000,
            "e10",
            StateHash::ZERO,
        );
        assert_eq!(event.id, "e10");
        assert_eq!(event.author, "peer-1");
        assert!(matches!(
            event.kind,
            EventKind::Message { ref channel_id, ref body }
            if channel_id == "ch-1" && body == "hello world"
        ));
    }
}
