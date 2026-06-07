//! Stable JSON wire shape for `ClientEvent`.
//!
//! `to_wire(event)` returns `Some(WireEvent)` for variants exposed to
//! e2e tests, and `None` for internal-only variants. The `WireEvent`
//! shape is `{kind: <PascalCase>, ...camelCase fields}` per the spec.

use serde::Serialize;
use willow_client::events::ClientEvent;

/// JSON-stable representation of a `ClientEvent` for the test surface.
#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum WireEvent {
    SyncCompleted {
        #[serde(rename = "opsApplied")]
        ops_applied: u32,
    },
    MessageReceived {
        channel: String,
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "isLocal")]
        is_local: bool,
    },
    PeerConnected {
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    PeerDisconnected {
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    ChannelCreated {
        name: String,
    },
    ChannelDeleted {
        name: String,
    },
    PeerTrusted {
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    PeerUntrusted {
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    ProfileUpdated {
        #[serde(rename = "peerId")]
        peer_id: String,
        #[serde(rename = "displayName")]
        display_name: String,
    },
    RoleCreated {
        #[serde(rename = "roleId")]
        role_id: String,
        name: String,
    },
    /// History-sync EOSE boundary marker (history-sync-eose spec, plan PR 5).
    /// Surfaced to e2e so `e2e/history-sync.spec.ts` can assert the
    /// `HistorySyncComplete` marker fires for a joining peer. `topic` is the
    /// lowercase-hex of the marker's 32-byte `topic_id`; `provider` is the
    /// verified envelope signer; `stillPending` counts trusted providers that
    /// have not yet completed for the same topic. Mirrors
    /// `e2e/test-hooks.ts`'s `HistorySynced` variant.
    HistorySynced {
        topic: String,
        provider: String,
        #[serde(rename = "stillPending")]
        still_pending: u32,
    },
}

/// Convert a `ClientEvent` to its wire shape, or `None` if the variant
/// is internal-only and should not be surfaced to e2e tests.
pub fn to_wire(event: &ClientEvent) -> Option<WireEvent> {
    match event {
        ClientEvent::SyncCompleted { ops_applied } => Some(WireEvent::SyncCompleted {
            ops_applied: *ops_applied as u32,
        }),
        ClientEvent::MessageReceived {
            channel,
            message_id,
            is_local,
        } => Some(WireEvent::MessageReceived {
            channel: channel.clone(),
            message_id: message_id.clone(),
            is_local: *is_local,
        }),
        ClientEvent::PeerConnected(id) => Some(WireEvent::PeerConnected {
            peer_id: id.to_string(),
        }),
        ClientEvent::PeerDisconnected(id) => Some(WireEvent::PeerDisconnected {
            peer_id: id.to_string(),
        }),
        ClientEvent::ChannelCreated(name) => Some(WireEvent::ChannelCreated { name: name.clone() }),
        ClientEvent::ChannelDeleted(name) => Some(WireEvent::ChannelDeleted { name: name.clone() }),
        ClientEvent::PeerTrusted(id) => Some(WireEvent::PeerTrusted {
            peer_id: id.to_string(),
        }),
        ClientEvent::PeerUntrusted(id) => Some(WireEvent::PeerUntrusted {
            peer_id: id.to_string(),
        }),
        ClientEvent::ProfileUpdated {
            peer_id,
            display_name,
        } => Some(WireEvent::ProfileUpdated {
            peer_id: peer_id.to_string(),
            display_name: display_name.clone(),
        }),
        ClientEvent::RoleCreated { name, role_id } => Some(WireEvent::RoleCreated {
            role_id: role_id.clone(),
            name: name.clone(),
        }),
        ClientEvent::HistorySynced {
            topic,
            provider,
            still_pending,
        } => Some(WireEvent::HistorySynced {
            topic: topic.clone(),
            provider: provider.to_string(),
            still_pending: *still_pending as u32,
        }),
        // Internal-only variants are filtered out.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::SecretKey;

    /// Stable test fixture: deterministic `EndpointId` derived from a
    /// 32-byte all-ones secret key.
    fn endpoint_a() -> willow_identity::EndpointId {
        // SecretKey::from_bytes takes a `&[u8; 32]` value (not a Result).
        // .public() returns the corresponding Ed25519 public key = EndpointId.
        SecretKey::from_bytes(&[1u8; 32]).public()
    }

    #[test]
    fn sync_completed_serializes_to_stable_shape() {
        let ev = ClientEvent::SyncCompleted { ops_applied: 5 };
        let wire = to_wire(&ev).expect("SyncCompleted must convert");
        let json = serde_json::to_string(&wire).unwrap();
        assert_eq!(json, r#"{"kind":"SyncCompleted","opsApplied":5}"#);
    }

    #[test]
    fn message_received() {
        let ev = ClientEvent::MessageReceived {
            channel: "general".into(),
            message_id: "m1".into(),
            is_local: false,
        };
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"MessageReceived","channel":"general","messageId":"m1","isLocal":false}"#,
        );
    }

    #[test]
    fn peer_connected() {
        let ev = ClientEvent::PeerConnected(endpoint_a());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.starts_with(r#"{"kind":"PeerConnected","peerId":"#));
    }

    #[test]
    fn peer_disconnected() {
        let ev = ClientEvent::PeerDisconnected(endpoint_a());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.starts_with(r#"{"kind":"PeerDisconnected","peerId":"#));
    }

    #[test]
    fn channel_created() {
        let ev = ClientEvent::ChannelCreated("general".into());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert_eq!(json, r#"{"kind":"ChannelCreated","name":"general"}"#);
    }

    #[test]
    fn channel_deleted() {
        let ev = ClientEvent::ChannelDeleted("general".into());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert_eq!(json, r#"{"kind":"ChannelDeleted","name":"general"}"#);
    }

    #[test]
    fn peer_trusted() {
        let ev = ClientEvent::PeerTrusted(endpoint_a());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.starts_with(r#"{"kind":"PeerTrusted","peerId":"#));
    }

    #[test]
    fn peer_untrusted() {
        let ev = ClientEvent::PeerUntrusted(endpoint_a());
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.starts_with(r#"{"kind":"PeerUntrusted","peerId":"#));
    }

    #[test]
    fn profile_updated() {
        let ev = ClientEvent::ProfileUpdated {
            peer_id: endpoint_a(),
            display_name: "alice".into(),
        };
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert!(json.contains(r#""kind":"ProfileUpdated""#));
        assert!(json.contains(r#""displayName":"alice""#));
    }

    #[test]
    fn role_created() {
        let ev = ClientEvent::RoleCreated {
            name: "moderator".into(),
            role_id: "r1".into(),
        };
        let json = serde_json::to_string(&to_wire(&ev).unwrap()).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"RoleCreated","roleId":"r1","name":"moderator"}"#,
        );
    }

    #[test]
    fn history_synced_serializes_to_stable_shape() {
        // The e2e surface (`e2e/history-sync.spec.ts`, via `e2e/test-hooks.ts`)
        // waits for `{ kind: 'HistorySynced', topic, provider, stillPending }`.
        // Without this mapping `to_wire` returns `None` and the event is
        // silently dropped, so the e2e EOSE assertion (the PR 5 deliverable)
        // can never fire. Pin the exact wire shape.
        let ev = ClientEvent::HistorySynced {
            topic: "ab".repeat(32),
            provider: endpoint_a(),
            still_pending: 2,
        };
        let wire = to_wire(&ev).expect("HistorySynced must convert to the e2e wire shape");
        let json = serde_json::to_string(&wire).unwrap();
        assert!(json.contains(r#""kind":"HistorySynced""#));
        assert!(json.contains(&format!(r#""topic":"{}""#, "ab".repeat(32))));
        assert!(json.contains(r#""stillPending":2"#));
        // provider is the signer's EndpointId string.
        assert!(json.contains(r#""provider":"#));
    }

    /// Task 3.4: ensure internal-only variants are filtered.
    #[test]
    fn internal_variants_are_filtered() {
        // Pick any internal-only variant. RelayStatusChanged is a good choice
        // (simple tuple variant with single field).
        let ev = ClientEvent::RelayStatusChanged(willow_client::queue::RelayStatus::Reachable);
        assert!(
            to_wire(&ev).is_none(),
            "internal-only variants must not leak to the wire"
        );
    }
}
