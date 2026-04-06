//! # MCP Resource Definitions and Handlers
//!
//! All 14 MCP resources mapped to `ClientHandle` accessors and `StateRef` views.

use std::sync::Arc;

use rmcp::model::*;
use rmcp::ErrorData;
use serde::Serialize;
use willow_client::ClientHandle;
use willow_network::Network;

/// Build the static list of resource definitions for `resources/list`.
pub fn list_resources() -> Vec<Resource> {
    let defs = [
        (
            "willow://identity",
            "Identity",
            "Local peer identity (peer_id, display_name)",
        ),
        (
            "willow://connection",
            "Connection",
            "Connection status (connected, peer_count, typing_peers)",
        ),
        ("willow://servers", "Servers", "List of servers (id, name)"),
        (
            "willow://server/current",
            "Current Server",
            "Active server details",
        ),
        (
            "willow://server/channels",
            "Channels",
            "Channels in the active server",
        ),
        (
            "willow://server/members",
            "Members",
            "Members of the active server",
        ),
        (
            "willow://server/roles",
            "Roles",
            "Roles in the active server",
        ),
        (
            "willow://server/unread",
            "Unread Counts",
            "Unread message counts per channel",
        ),
        (
            "willow://server/join-links",
            "Join Links",
            "Active join links",
        ),
        (
            "willow://channel/{name}/messages",
            "Channel Messages",
            "Messages in a channel (use channel name in URI)",
        ),
        (
            "willow://channel/{name}/pins",
            "Pinned Messages",
            "Pinned messages in a channel",
        ),
        (
            "willow://channel/{name}/typing",
            "Typing Indicators",
            "Who is typing in a channel",
        ),
        (
            "willow://voice/status",
            "Voice Status",
            "Current voice state (active channel, muted, deafened)",
        ),
        (
            "willow://voice/{channel}/participants",
            "Voice Participants",
            "Participants in a voice channel",
        ),
    ];

    defs.iter()
        .map(|(uri, name, desc)| Annotated {
            raw: RawResource {
                uri: uri.to_string(),
                name: name.to_string(),
                title: None,
                description: Some(desc.to_string()),
                mime_type: Some("application/json".to_string()),
                size: None,
                icons: None,
                meta: None,
            },
            annotations: None,
        })
        .collect()
}

/// Read a resource by URI. Returns JSON-encoded state snapshots.
pub async fn read_resource<N: Network>(
    client: &Arc<ClientHandle<N>>,
    uri: &str,
) -> Result<ReadResourceResult, ErrorData> {
    let json = match uri {
        "willow://identity" => {
            let peer_id = client.peer_id();
            let display_name = client.display_name().await;
            to_json(&IdentityResource {
                peer_id,
                display_name,
            })
        }

        "willow://connection" => {
            let connected = client.is_connected().await;
            let peers = client.peers().await;
            let typing = client.typing_peers().await;
            let typing_entries: Vec<TypingPeerEntry> = typing
                .into_iter()
                .map(|(peer_id, channel)| TypingPeerEntry { peer_id, channel })
                .collect();
            to_json(&ConnectionResource {
                connected,
                peer_count: peers.len(),
                typing_peers: typing_entries,
            })
        }

        "willow://servers" => {
            let servers = client.server_list().await;
            let entries: Vec<ServerListEntry> = servers
                .into_iter()
                .map(|(id, name)| ServerListEntry { id, name })
                .collect();
            to_json(&entries)
        }

        "willow://server/current" => {
            let id = client.active_server_id().await;
            let name = client.active_server_name().await;
            let admins: Vec<String> = client.admins().await.iter().map(|a| a.to_string()).collect();
            let description = client.server_description().await;
            let display_name = client.display_name().await;
            to_json(&CurrentServerResource {
                id,
                name,
                admins,
                description,
                display_name,
            })
        }

        "willow://server/channels" => {
            let channels = client.channel_kinds().await;
            let entries: Vec<ChannelEntry> = channels
                .into_iter()
                .map(|(name, kind)| ChannelEntry { name, kind })
                .collect();
            to_json(&entries)
        }

        "willow://server/members" => {
            let members = client.server_members().await;
            let entries: Vec<MemberEntry> = members
                .into_iter()
                .map(|(peer_id, display_name, is_online)| MemberEntry {
                    peer_id: peer_id.to_string(),
                    display_name,
                    is_online,
                })
                .collect();
            to_json(&entries)
        }

        "willow://server/roles" => {
            let roles = client.roles_data().await;
            let entries: Vec<RoleEntry> = roles
                .into_iter()
                .map(|(id, name, permissions)| RoleEntry {
                    id,
                    name,
                    permissions,
                })
                .collect();
            to_json(&entries)
        }

        "willow://server/unread" => {
            let counts = client.unread_counts().await;
            to_json(&counts)
        }

        "willow://server/join-links" => {
            let links = client.join_links().await;
            let entries: Vec<JoinLinkEntry> = links
                .into_iter()
                .map(|l| JoinLinkEntry {
                    id: l.link_id,
                    max_uses: l.max_uses,
                    uses: l.used,
                    active: l.active,
                    expires_at: l.expires_at,
                })
                .collect();
            to_json(&entries)
        }

        "willow://voice/status" => {
            let active_channel = client.active_voice_channel().await;
            let muted = client.is_voice_muted().await;
            let deafened = client.is_voice_deafened().await;
            to_json(&VoiceStatusResource {
                active_channel,
                muted,
                deafened,
            })
        }

        _ if uri.starts_with("willow://channel/") && uri.ends_with("/messages") => {
            let channel = extract_channel_name(uri, "/messages");
            let messages = client.messages(&channel).await;
            let entries: Vec<MessageEntry> = messages.into_iter().map(MessageEntry::from).collect();
            to_json(&entries)
        }

        _ if uri.starts_with("willow://channel/") && uri.ends_with("/pins") => {
            let channel = extract_channel_name(uri, "/pins");
            let messages = client.pinned_messages(&channel).await;
            let entries: Vec<MessageEntry> = messages.into_iter().map(MessageEntry::from).collect();
            to_json(&entries)
        }

        _ if uri.starts_with("willow://channel/") && uri.ends_with("/typing") => {
            let channel = extract_channel_name(uri, "/typing");
            let typing = client.typing_in(&channel).await;
            to_json(&typing)
        }

        _ if uri.starts_with("willow://voice/") && uri.ends_with("/participants") => {
            let channel = uri
                .strip_prefix("willow://voice/")
                .and_then(|s| s.strip_suffix("/participants"))
                .unwrap_or("");
            let participants = client.voice_participants(channel).await;
            let entries: Vec<String> = participants.into_iter().map(|p| p.to_string()).collect();
            to_json(&entries)
        }

        _ => {
            return Err(ErrorData::resource_not_found(
                format!("unknown resource: {uri}"),
                None,
            ));
        }
    };

    Ok(ReadResourceResult::new(vec![ResourceContents::text(
        json, uri,
    )
    .with_mime_type("application/json")]))
}

fn extract_channel_name(uri: &str, suffix: &str) -> String {
    uri.strip_prefix("willow://channel/")
        .and_then(|s| s.strip_suffix(suffix))
        .unwrap_or("general")
        .to_string()
}

fn to_json(value: &impl Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

// ─────────────────────── Resource response types ─────────────────────────────

#[derive(Serialize)]
struct IdentityResource {
    peer_id: String,
    display_name: String,
}

#[derive(Serialize)]
struct ConnectionResource {
    connected: bool,
    peer_count: usize,
    typing_peers: Vec<TypingPeerEntry>,
}

#[derive(Serialize)]
struct TypingPeerEntry {
    peer_id: String,
    channel: String,
}

#[derive(Serialize)]
struct ServerListEntry {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct CurrentServerResource {
    id: Option<String>,
    name: String,
    admins: Vec<String>,
    description: String,
    display_name: String,
}

#[derive(Serialize)]
struct ChannelEntry {
    name: String,
    kind: String,
}

#[derive(Serialize)]
struct MemberEntry {
    peer_id: String,
    display_name: String,
    is_online: bool,
}

#[derive(Serialize)]
struct RoleEntry {
    id: String,
    name: String,
    permissions: Vec<String>,
}

#[derive(Serialize)]
struct JoinLinkEntry {
    id: String,
    max_uses: u32,
    uses: u32,
    active: bool,
    expires_at: Option<u64>,
}

#[derive(Serialize)]
struct VoiceStatusResource {
    active_channel: Option<String>,
    muted: bool,
    deafened: bool,
}

#[derive(Serialize)]
struct MessageEntry {
    id: String,
    author_peer_id: String,
    author_display_name: String,
    body: String,
    timestamp_ms: u64,
    edited: bool,
    reply_to: Option<String>,
    reactions: std::collections::HashMap<String, Vec<String>>,
}

impl From<willow_client::DisplayMessage> for MessageEntry {
    fn from(m: willow_client::DisplayMessage) -> Self {
        Self {
            id: m.id,
            author_peer_id: m.author_peer_id.to_string(),
            author_display_name: m.author_display_name,
            body: m.body,
            timestamp_ms: m.timestamp_ms,
            edited: m.edited,
            reply_to: m.reply_to,
            reactions: m.reactions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_14_resources_defined() {
        let resources = list_resources();
        assert_eq!(resources.len(), 14);
    }

    #[test]
    fn resource_uris_are_unique() {
        let resources = list_resources();
        let mut uris: Vec<&str> = resources.iter().map(|r| r.raw.uri.as_str()).collect();
        let before = uris.len();
        uris.sort();
        uris.dedup();
        assert_eq!(uris.len(), before, "duplicate resource URIs found");
    }

    #[test]
    fn resource_uris_start_with_willow() {
        for r in list_resources() {
            assert!(
                r.raw.uri.starts_with("willow://"),
                "URI should start with willow:// but got: {}",
                r.raw.uri
            );
        }
    }

    #[test]
    fn extract_channel_name_works() {
        assert_eq!(
            extract_channel_name("willow://channel/general/messages", "/messages"),
            "general"
        );
        assert_eq!(
            extract_channel_name("willow://channel/dev-ops/pins", "/pins"),
            "dev-ops"
        );
    }
}
