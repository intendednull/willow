//! # ClientEvent → MCP Notifications
//!
//! Bridges `ClientEvent` from `Broker<ClientEvent>` into MCP
//! server-sent notifications. Each event variant is serialized to JSON
//! with a `type` field matching the spec notification table.

use serde::Serialize;
use willow_client::ClientEvent;

/// Serialize a `ClientEvent` into a JSON value for MCP notification params.
pub fn event_to_json(event: &ClientEvent) -> serde_json::Value {
    match event {
        ClientEvent::MessageReceived {
            channel,
            message_id,
            is_local,
        } => to_value(&NotificationPayload {
            r#type: "MessageReceived",
            data: serde_json::json!({
                "channel": channel,
                "message_id": message_id,
                "is_local": is_local,
            }),
        }),
        ClientEvent::MessageEdited {
            channel,
            message_id,
            new_body,
        } => to_value(&NotificationPayload {
            r#type: "MessageEdited",
            data: serde_json::json!({
                "channel": channel,
                "message_id": message_id,
                "new_body": new_body,
            }),
        }),
        ClientEvent::MessageDeleted {
            channel,
            message_id,
        } => to_value(&NotificationPayload {
            r#type: "MessageDeleted",
            data: serde_json::json!({
                "channel": channel,
                "message_id": message_id,
            }),
        }),
        ClientEvent::ReactionAdded {
            channel,
            message_id,
            emoji,
            author,
        } => to_value(&NotificationPayload {
            r#type: "ReactionAdded",
            data: serde_json::json!({
                "channel": channel,
                "message_id": message_id,
                "emoji": emoji,
                "author": author.to_string(),
            }),
        }),
        ClientEvent::PeerConnected(peer) => to_value(&NotificationPayload {
            r#type: "PeerConnected",
            data: serde_json::json!({ "peer_id": peer.to_string() }),
        }),
        ClientEvent::PeerDisconnected(peer) => to_value(&NotificationPayload {
            r#type: "PeerDisconnected",
            data: serde_json::json!({ "peer_id": peer.to_string() }),
        }),
        ClientEvent::ChannelCreated(name) => to_value(&NotificationPayload {
            r#type: "ChannelCreated",
            data: serde_json::json!({ "name": name }),
        }),
        ClientEvent::ChannelDeleted(name) => to_value(&NotificationPayload {
            r#type: "ChannelDeleted",
            data: serde_json::json!({ "name": name }),
        }),
        ClientEvent::PeerTrusted(peer) => to_value(&NotificationPayload {
            r#type: "PeerTrusted",
            data: serde_json::json!({ "peer_id": peer.to_string() }),
        }),
        ClientEvent::PeerUntrusted(peer) => to_value(&NotificationPayload {
            r#type: "PeerUntrusted",
            data: serde_json::json!({ "peer_id": peer.to_string() }),
        }),
        ClientEvent::ProfileUpdated {
            peer_id,
            display_name,
        } => to_value(&NotificationPayload {
            r#type: "ProfileUpdated",
            data: serde_json::json!({
                "peer_id": peer_id.to_string(),
                "display_name": display_name,
            }),
        }),
        ClientEvent::FileAnnounced {
            channel,
            filename,
            size,
            from,
        } => to_value(&NotificationPayload {
            r#type: "FileAnnounced",
            data: serde_json::json!({
                "channel": channel,
                "filename": filename,
                "size": size,
                "from": from,
            }),
        }),
        ClientEvent::Listening(address) => to_value(&NotificationPayload {
            r#type: "Listening",
            data: serde_json::json!({ "address": address }),
        }),
        ClientEvent::SyncCompleted { ops_applied } => to_value(&NotificationPayload {
            r#type: "SyncCompleted",
            data: serde_json::json!({ "ops_applied": ops_applied }),
        }),
        ClientEvent::RoleCreated { name, role_id } => to_value(&NotificationPayload {
            r#type: "RoleCreated",
            data: serde_json::json!({
                "name": name,
                "role_id": role_id,
            }),
        }),
        ClientEvent::RoleDeleted { role_id } => to_value(&NotificationPayload {
            r#type: "RoleDeleted",
            data: serde_json::json!({ "role_id": role_id }),
        }),
        ClientEvent::ProposalCreated {
            proposal_hash,
            action_description,
        } => to_value(&NotificationPayload {
            r#type: "ProposalCreated",
            data: serde_json::json!({
                "proposal_hash": proposal_hash,
                "action_description": action_description,
            }),
        }),
        ClientEvent::VoteCast {
            proposal_hash,
            accept,
            voter,
        } => to_value(&NotificationPayload {
            r#type: "VoteCast",
            data: serde_json::json!({
                "proposal_hash": proposal_hash,
                "accept": accept,
                "voter": voter.to_string(),
            }),
        }),
        ClientEvent::ServerRenamed { new_name } => to_value(&NotificationPayload {
            r#type: "ServerRenamed",
            data: serde_json::json!({ "new_name": new_name }),
        }),
        ClientEvent::ServerDescriptionChanged { description } => to_value(&NotificationPayload {
            r#type: "ServerDescriptionChanged",
            data: serde_json::json!({ "description": description }),
        }),
        ClientEvent::MessagePinned {
            channel,
            message_id,
        } => to_value(&NotificationPayload {
            r#type: "MessagePinned",
            data: serde_json::json!({
                "channel": channel,
                "message_id": message_id,
            }),
        }),
        ClientEvent::MessageUnpinned {
            channel,
            message_id,
        } => to_value(&NotificationPayload {
            r#type: "MessageUnpinned",
            data: serde_json::json!({
                "channel": channel,
                "message_id": message_id,
            }),
        }),
        ClientEvent::VoiceJoined {
            channel_id,
            peer_id,
        } => to_value(&NotificationPayload {
            r#type: "VoiceJoined",
            data: serde_json::json!({
                "channel_id": channel_id,
                "peer_id": peer_id.to_string(),
            }),
        }),
        ClientEvent::VoiceLeft {
            channel_id,
            peer_id,
        } => to_value(&NotificationPayload {
            r#type: "VoiceLeft",
            data: serde_json::json!({
                "channel_id": channel_id,
                "peer_id": peer_id.to_string(),
            }),
        }),
        ClientEvent::VoiceSignal {
            channel_id,
            from_peer,
            signal,
        } => to_value(&NotificationPayload {
            r#type: "VoiceSignal",
            data: serde_json::json!({
                "channel_id": channel_id,
                "from_peer": from_peer.to_string(),
                "signal": signal,
            }),
        }),
        ClientEvent::JoinLinkResponse { invite_data } => to_value(&NotificationPayload {
            r#type: "JoinLinkResponse",
            data: serde_json::json!({ "invite_data": invite_data }),
        }),
        ClientEvent::JoinLinkDenied { reason } => to_value(&NotificationPayload {
            r#type: "JoinLinkDenied",
            data: serde_json::json!({ "reason": reason }),
        }),
        ClientEvent::MuteChanged { scope, muted } => to_value(&NotificationPayload {
            r#type: "MuteChanged",
            data: serde_json::json!({
                "scope": match scope {
                    willow_client::events::MuteScope::Grove => "grove".to_string(),
                    willow_client::events::MuteScope::Channel(id) => format!("channel:{id}"),
                },
                "muted": muted,
            }),
        }),
        ClientEvent::QueueChanged(view) => to_value(&NotificationPayload {
            r#type: "QueueChanged",
            data: serde_json::json!({
                "depth": view.depth,
                "peer_count": view.peer_count,
                "device_online": view.device_online,
            }),
        }),
        ClientEvent::RelayStatusChanged(status) => to_value(&NotificationPayload {
            r#type: "RelayStatusChanged",
            data: serde_json::json!({
                "status": match status {
                    willow_client::RelayStatus::Reachable => "reachable",
                    willow_client::RelayStatus::Unreachable => "unreachable",
                    willow_client::RelayStatus::NotConfigured => "not_configured",
                },
            }),
        }),
        ClientEvent::DeviceOnlineChanged(online) => to_value(&NotificationPayload {
            r#type: "DeviceOnlineChanged",
            data: serde_json::json!({ "online": online }),
        }),
    }
}

/// All 31 event type names for validation.
pub const EVENT_TYPE_NAMES: &[&str] = &[
    "MessageReceived",
    "MessageEdited",
    "MessageDeleted",
    "ReactionAdded",
    "PeerConnected",
    "PeerDisconnected",
    "ChannelCreated",
    "ChannelDeleted",
    "PeerTrusted",
    "PeerUntrusted",
    "ProfileUpdated",
    "FileAnnounced",
    "Listening",
    "SyncCompleted",
    "RoleCreated",
    "RoleDeleted",
    "ProposalCreated",
    "VoteCast",
    "ServerRenamed",
    "ServerDescriptionChanged",
    "MessagePinned",
    "MessageUnpinned",
    "VoiceJoined",
    "VoiceLeft",
    "VoiceSignal",
    "JoinLinkResponse",
    "JoinLinkDenied",
    "MuteChanged",
    // Phase 2b sync-queue variants.
    "QueueChanged",
    "RelayStatusChanged",
    "DeviceOnlineChanged",
];

#[derive(Serialize)]
struct NotificationPayload {
    r#type: &'static str,
    data: serde_json::Value,
}

fn to_value(payload: &NotificationPayload) -> serde_json::Value {
    serde_json::to_value(payload).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    #[test]
    fn all_31_event_types_listed() {
        assert_eq!(EVENT_TYPE_NAMES.len(), 31);
    }

    #[test]
    fn event_type_names_are_unique() {
        let mut set = std::collections::HashSet::new();
        for name in EVENT_TYPE_NAMES {
            assert!(set.insert(name), "duplicate event type: {name}");
        }
    }

    #[test]
    fn message_received_serializes_correctly() {
        let event = ClientEvent::MessageReceived {
            channel: "general".to_string(),
            message_id: "msg-1".to_string(),
            is_local: false,
        };
        let json = event_to_json(&event);
        assert_eq!(json["type"], "MessageReceived");
        assert_eq!(json["data"]["channel"], "general");
        assert_eq!(json["data"]["message_id"], "msg-1");
        assert_eq!(json["data"]["is_local"], false);
    }

    #[test]
    fn peer_connected_serializes_correctly() {
        let id = Identity::generate().endpoint_id();
        let event = ClientEvent::PeerConnected(id);
        let json = event_to_json(&event);
        assert_eq!(json["type"], "PeerConnected");
        assert_eq!(json["data"]["peer_id"], id.to_string());
    }

    #[test]
    fn all_variants_produce_valid_json() {
        let id = Identity::generate().endpoint_id();
        let events = vec![
            ClientEvent::MessageReceived {
                channel: "ch".into(),
                message_id: "m".into(),
                is_local: true,
            },
            ClientEvent::MessageEdited {
                channel: "ch".into(),
                message_id: "m".into(),
                new_body: "new".into(),
            },
            ClientEvent::MessageDeleted {
                channel: "ch".into(),
                message_id: "m".into(),
            },
            ClientEvent::ReactionAdded {
                channel: "ch".into(),
                message_id: "m".into(),
                emoji: "👍".into(),
                author: id,
            },
            ClientEvent::PeerConnected(id),
            ClientEvent::PeerDisconnected(id),
            ClientEvent::ChannelCreated("dev".into()),
            ClientEvent::ChannelDeleted("dev".into()),
            ClientEvent::PeerTrusted(id),
            ClientEvent::PeerUntrusted(id),
            ClientEvent::ProfileUpdated {
                peer_id: id,
                display_name: "Alice".into(),
            },
            ClientEvent::FileAnnounced {
                channel: "ch".into(),
                filename: "f.txt".into(),
                size: 100,
                from: "Alice".into(),
            },
            ClientEvent::Listening("topic".into()),
            ClientEvent::SyncCompleted { ops_applied: 5 },
            ClientEvent::RoleCreated {
                name: "mod".into(),
                role_id: "r1".into(),
            },
            ClientEvent::RoleDeleted {
                role_id: "r1".into(),
            },
            ClientEvent::ProposalCreated {
                proposal_hash: "abc123".into(),
                action_description: "grant admin".into(),
            },
            ClientEvent::VoteCast {
                proposal_hash: "abc123".into(),
                accept: true,
                voter: id,
            },
            ClientEvent::ServerRenamed {
                new_name: "New".into(),
            },
            ClientEvent::ServerDescriptionChanged {
                description: "desc".into(),
            },
            ClientEvent::MessagePinned {
                channel: "ch".into(),
                message_id: "m".into(),
            },
            ClientEvent::MessageUnpinned {
                channel: "ch".into(),
                message_id: "m".into(),
            },
            ClientEvent::VoiceJoined {
                channel_id: "vc".into(),
                peer_id: id,
            },
            ClientEvent::VoiceLeft {
                channel_id: "vc".into(),
                peer_id: id,
            },
            ClientEvent::VoiceSignal {
                channel_id: "vc".into(),
                from_peer: id,
                signal: willow_client::VoiceSignalPayload::Offer("sdp-offer".into()),
            },
            ClientEvent::JoinLinkResponse {
                invite_data: "data".into(),
            },
            ClientEvent::JoinLinkDenied {
                reason: "no".into(),
            },
            ClientEvent::MuteChanged {
                scope: willow_client::events::MuteScope::Grove,
                muted: true,
            },
            ClientEvent::QueueChanged(willow_client::views::QueueView::default()),
            ClientEvent::RelayStatusChanged(willow_client::RelayStatus::Reachable),
            ClientEvent::DeviceOnlineChanged(true),
        ];
        // One entry per `ClientEvent` variant — mirrors `EVENT_TYPE_NAMES`.
        assert_eq!(
            events.len(),
            EVENT_TYPE_NAMES.len(),
            "should test every ClientEvent variant"
        );
        for event in &events {
            let json = event_to_json(event);
            assert!(json.is_object(), "expected object for {event:?}");
            assert!(json["type"].is_string(), "missing type for {event:?}");
        }
    }

    #[test]
    fn voice_signal_includes_payload() {
        let id = Identity::generate().endpoint_id();

        // Test Offer variant
        let event = ClientEvent::VoiceSignal {
            channel_id: "vc".into(),
            from_peer: id,
            signal: willow_client::VoiceSignalPayload::Offer("sdp-data".into()),
        };
        let json = event_to_json(&event);
        assert_eq!(json["type"], "VoiceSignal");
        assert!(
            json["data"]["signal"].is_object(),
            "signal should be present"
        );
        assert_eq!(json["data"]["signal"]["Offer"], "sdp-data");

        // Test IceCandidate variant
        let event = ClientEvent::VoiceSignal {
            channel_id: "vc".into(),
            from_peer: id,
            signal: willow_client::VoiceSignalPayload::IceCandidate("candidate-data".into()),
        };
        let json = event_to_json(&event);
        assert_eq!(json["data"]["signal"]["IceCandidate"], "candidate-data");
    }
}
