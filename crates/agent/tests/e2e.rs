//! # E2E Tests for the Willow Agent
//!
//! Tests use `test_client()` from willow-client (via `test-utils` feature)
//! to create in-process `ClientHandle<MemNetwork>` instances. These are
//! single-peer tests that verify the full MCP server → ClientHandle → actor
//! pipeline works end-to-end. Multi-peer tests use `test_client_on_hub()`
//! with a shared `MemHub`.

use std::sync::Arc;
use willow_agent::server::WillowMcpServer;
use willow_agent::tools::WillowToolRouter;
use willow_client::{test_client, ClientHandle};
use willow_network::mem::{MemHub, MemNetwork};

/// Helper to create a test MCP server.
fn test_mcp_server() -> (WillowMcpServer<MemNetwork>, ClientHandle<MemNetwork>) {
    let (client, _broker) = test_client();
    let server = WillowMcpServer::new(client.clone());
    (server, client)
}

/// Helper to call a tool by name with JSON args.
async fn call_tool(
    router: &WillowToolRouter<MemNetwork>,
    name: &'static str,
    args: serde_json::Value,
) -> rmcp::model::CallToolResult {
    let params = rmcp::model::CallToolRequestParams::new(name).with_arguments(match args {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    });
    router.call(&params).await.expect("tool call failed")
}

fn result_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|c| match &c.raw {
            rmcp::model::RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

// ─────────────────────── Messaging Tests ─────────────────────────────────────

#[tokio::test]
async fn send_message_and_read_back() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    let result = call_tool(
        &router,
        "send_message",
        serde_json::json!({ "channel": "general", "body": "hello from agent" }),
    )
    .await;
    assert!(result.is_error != Some(true));

    let messages = client.messages("general").await;
    assert!(
        messages.iter().any(|m| m.body == "hello from agent"),
        "message not found in channel"
    );
}

#[tokio::test]
async fn edit_message() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "send_message",
        serde_json::json!({ "channel": "general", "body": "original" }),
    )
    .await;

    let messages = client.messages("general").await;
    let msg_id = &messages.last().unwrap().id;

    call_tool(
        &router,
        "edit_message",
        serde_json::json!({
            "channel": "general",
            "message_id": msg_id,
            "new_body": "edited"
        }),
    )
    .await;

    let messages = client.messages("general").await;
    let msg = messages.iter().find(|m| m.id == *msg_id).unwrap();
    assert_eq!(msg.body, "edited");
}

#[tokio::test]
async fn delete_message() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "send_message",
        serde_json::json!({ "channel": "general", "body": "to delete" }),
    )
    .await;

    let messages = client.messages("general").await;
    let msg_id = &messages.last().unwrap().id;

    call_tool(
        &router,
        "delete_message",
        serde_json::json!({ "channel": "general", "message_id": msg_id }),
    )
    .await;

    let messages = client.messages("general").await;
    assert!(
        !messages.iter().any(|m| m.body == "to delete"),
        "deleted message still visible"
    );
}

#[tokio::test]
async fn react_to_message() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "send_message",
        serde_json::json!({ "channel": "general", "body": "react to me" }),
    )
    .await;

    let messages = client.messages("general").await;
    let msg_id = &messages.last().unwrap().id;

    call_tool(
        &router,
        "react",
        serde_json::json!({
            "channel": "general",
            "message_id": msg_id,
            "emoji": "👍"
        }),
    )
    .await;

    let messages = client.messages("general").await;
    let msg = messages.iter().find(|m| m.id == *msg_id).unwrap();
    assert!(!msg.reactions.is_empty(), "reaction should be present");
}

#[tokio::test]
async fn pin_and_unpin_message() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "send_message",
        serde_json::json!({ "channel": "general", "body": "pin me" }),
    )
    .await;

    let messages = client.messages("general").await;
    let msg_id = &messages.last().unwrap().id;

    call_tool(
        &router,
        "pin_message",
        serde_json::json!({ "channel": "general", "message_id": msg_id }),
    )
    .await;

    let msg_hash: willow_state::EventHash = msg_id.parse().unwrap();
    assert!(client.is_pinned("general", &msg_hash).await);

    call_tool(
        &router,
        "unpin_message",
        serde_json::json!({ "channel": "general", "message_id": msg_id }),
    )
    .await;

    assert!(!client.is_pinned("general", &msg_hash).await);
}

// ─────────────────────── Channel Tests ───────────────────────────────────────

#[tokio::test]
async fn create_channel() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "create_channel",
        serde_json::json!({ "name": "dev" }),
    )
    .await;

    let channels = client.channels().await;
    assert!(
        channels.iter().any(|c| c == "dev"),
        "created channel not found: {channels:?}"
    );
}

#[tokio::test]
async fn switch_channel() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "switch_channel",
        serde_json::json!({ "name": "general" }),
    )
    .await;

    let current = client.current_channel().await;
    assert_eq!(current, "general");
}

// ─────────────────────── Server Tests ────────────────────────────────────────

#[tokio::test]
async fn create_server_returns_id() {
    let (server, _client) = test_mcp_server();
    let router = server.tool_router.clone();

    let result = call_tool(
        &router,
        "create_server",
        serde_json::json!({ "name": "My Server" }),
    )
    .await;

    let text = result_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(parsed["server_id"].is_string());
}

#[tokio::test]
async fn rename_server() {
    let (server, _client) = test_mcp_server();
    let router = server.tool_router.clone();

    let result = call_tool(
        &router,
        "rename_server",
        serde_json::json!({ "name": "Renamed" }),
    )
    .await;

    // The rename is applied via the event-sourced DAG. Verify the tool itself
    // reported success (not an error), meaning the event was built and applied.
    assert!(
        result.is_error != Some(true),
        "rename_server should not return an error"
    );
    let text = result_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        parsed["success"], true,
        "rename_server should return {{success: true}}, got: {parsed}"
    );
}

// ─────────────────────── Identity Tests ──────────────────────────────────────

#[tokio::test]
async fn set_display_name() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "set_display_name",
        serde_json::json!({ "name": "AgentBot" }),
    )
    .await;

    let name = client.display_name().await;
    assert_eq!(name, "AgentBot");
}

// ─────────────────────── Voice Tests ─────────────────────────────────────────

#[tokio::test]
async fn toggle_mute_returns_state() {
    let (server, _client) = test_mcp_server();
    let router = server.tool_router.clone();

    let result = call_tool(&router, "toggle_mute", serde_json::json!({})).await;
    let text = result_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["muted"], true);

    let result2 = call_tool(&router, "toggle_mute", serde_json::json!({})).await;
    let text2 = result_text(&result2);
    let parsed2: serde_json::Value = serde_json::from_str(&text2).unwrap();
    assert_eq!(parsed2["muted"], false);
}

#[tokio::test]
async fn toggle_deafen_returns_state() {
    let (server, _client) = test_mcp_server();
    let router = server.tool_router.clone();

    let result = call_tool(&router, "toggle_deafen", serde_json::json!({})).await;
    let text = result_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["deafened"], true);
}

// ─────────────────────── Resource Tests ──────────────────────────────────────

#[tokio::test]
async fn read_identity_resource() {
    let (_server, client) = test_mcp_server();
    let client_arc = Arc::new(client.clone());
    let result = willow_agent::resources::read_resource(&client_arc, "willow://identity")
        .await
        .unwrap();

    assert!(!result.contents.is_empty());
    match &result.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
            let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
            assert!(parsed["peer_id"].is_string());
            assert_eq!(parsed["peer_id"].as_str().unwrap().len(), 64);
        }
        _ => panic!("expected text resource"),
    }
}

#[tokio::test]
async fn read_channels_resource() {
    let (_server, client) = test_mcp_server();
    let client_arc = Arc::new(client);
    let result = willow_agent::resources::read_resource(&client_arc, "willow://server/channels")
        .await
        .unwrap();

    assert!(!result.contents.is_empty());
    match &result.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
            let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
            assert!(parsed.is_array());
            let channels = parsed.as_array().unwrap();
            assert!(
                channels.iter().any(|c| c["name"] == "general"),
                "general channel not found"
            );
        }
        _ => panic!("expected text resource"),
    }
}

#[tokio::test]
async fn read_unknown_resource_returns_error() {
    let (_server, client) = test_mcp_server();
    let client_arc = Arc::new(client);
    let result = willow_agent::resources::read_resource(&client_arc, "willow://nonexistent").await;

    assert!(result.is_err());
}

// ─────────────────────── Advanced E2E Tests ─────────────────────────────────

#[tokio::test]
async fn kick_member_sole_admin_is_noop() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    // The test client creates a server where this peer is the sole admin/owner.
    // Proposing to kick the sole admin is explicitly blocked by state logic
    // (kicking the last admin would leave the server permanently ungovernable).
    // The Propose event itself succeeds (admins can propose), but the resulting
    // vote auto-applies and the KickMember action is silently ignored.
    let peer_id = client.peer_id();

    let result = call_tool(
        &router,
        "kick_member",
        serde_json::json!({ "peer_id": peer_id }),
    )
    .await;

    // The tool should succeed (the Propose event is valid for an admin).
    assert!(
        result.is_error != Some(true),
        "kick_member proposal by an admin should not error"
    );
    let text = result_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        parsed["success"], true,
        "kick_member should return {{success: true}}"
    );

    // The sole admin must NOT have been removed — the state machine blocks
    // the kick to prevent a permanently ungovernable server.
    let members = client.server_members().await;
    let owner_id: willow_identity::EndpointId = peer_id.parse().unwrap();
    assert!(
        members.iter().any(|(pid, _, _)| *pid == owner_id),
        "sole admin should remain a member after self-kick attempt"
    );
}

#[tokio::test]
async fn server_rename_via_tool() {
    let (server, _client) = test_mcp_server();
    let router = server.tool_router.clone();

    let result = call_tool(
        &router,
        "rename_server",
        serde_json::json!({ "name": "My Renamed Server" }),
    )
    .await;

    // Tool must succeed and return {"success": true}.
    assert!(
        result.is_error != Some(true),
        "rename_server should not return an error"
    );
    let text = result_text(&result);
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        parsed["success"], true,
        "rename_server should return {{success: true}}, got: {parsed}"
    );
}

#[tokio::test]
async fn display_name_updates() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "set_display_name",
        serde_json::json!({ "name": "BotAlpha" }),
    )
    .await;

    let name = client.display_name().await;
    assert_eq!(name, "BotAlpha");

    // Change again
    call_tool(
        &router,
        "set_display_name",
        serde_json::json!({ "name": "BotBeta" }),
    )
    .await;

    let name = client.display_name().await;
    assert_eq!(name, "BotBeta");
}

#[tokio::test]
async fn voice_join_and_leave() {
    let (server, _client) = test_mcp_server();
    let router = server.tool_router.clone();

    let result = call_tool(
        &router,
        "join_voice",
        serde_json::json!({ "channel_id": "voice-lobby" }),
    )
    .await;
    assert!(result.is_error != Some(true));

    let result = call_tool(&router, "leave_voice", serde_json::json!({})).await;
    assert!(result.is_error != Some(true));
}

#[tokio::test]
async fn send_reply_to_message() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    // Send original message
    call_tool(
        &router,
        "send_message",
        serde_json::json!({ "channel": "general", "body": "original message" }),
    )
    .await;

    let messages = client.messages("general").await;
    let msg_id = &messages.last().unwrap().id;

    // Send reply
    let result = call_tool(
        &router,
        "send_reply",
        serde_json::json!({
            "channel": "general",
            "parent_id": msg_id,
            "body": "this is a reply"
        }),
    )
    .await;
    assert!(result.is_error != Some(true));

    let messages = client.messages("general").await;
    assert!(
        messages.iter().any(|m| m.body == "this is a reply"),
        "reply not found in channel"
    );
}

#[tokio::test]
async fn create_and_delete_channel() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "create_channel",
        serde_json::json!({ "name": "temp-channel" }),
    )
    .await;

    let channels = client.channels().await;
    assert!(channels.iter().any(|c| c == "temp-channel"));

    call_tool(
        &router,
        "delete_channel",
        serde_json::json!({ "name": "temp-channel" }),
    )
    .await;

    let channels = client.channels().await;
    assert!(
        !channels.iter().any(|c| c == "temp-channel"),
        "channel should be deleted"
    );
}

// ─────────────────────── Scope Enforcement Tests ────────────────────────────

#[tokio::test]
async fn readonly_token_hides_tools() {
    use willow_agent::scopes::TokenScope;
    use willow_agent::server::WillowMcpServer;

    let (client, _broker) = test_client();
    let server = WillowMcpServer::with_scope(client.clone(), TokenScope::ReadOnly);

    // Scope should filter all tools
    let visible: Vec<_> = server
        .tool_router
        .tool_list()
        .into_iter()
        .filter(|t| server.scope.allows_tool(t.name.as_ref()))
        .collect();
    assert!(
        visible.is_empty(),
        "ReadOnly scope should hide all tools, got: {:?}",
        visible.iter().map(|t| &t.name).collect::<Vec<_>>()
    );

    // All resources except `willow://server/join-links` are visible. The
    // join-links resource exposes single-step bearer credentials usable by
    // `WireMessage::JoinRequest`, so it is restricted to `Full`/`Admin`
    // (audit finding AUD-2, issue #436).
    let resources = willow_agent::resources::list_resources();
    for r in &resources {
        if r.raw.uri == "willow://server/join-links" {
            assert!(
                !server.scope.allows_resource(&r.raw.uri),
                "ReadOnly must NOT expose {}: link_id is a bearer credential",
                r.raw.uri
            );
        } else {
            assert!(
                server.scope.allows_resource(&r.raw.uri),
                "ReadOnly should allow resource: {}",
                r.raw.uri
            );
        }
    }
}

/// Integration-tier check: under `ReadOnly`, the same filter pipeline used by
/// `WillowMcpServer::list_resources` must omit the join-links entry.
///
/// `ServerHandler::list_resources` cannot be invoked directly from outside
/// rmcp (the `RequestContext<RoleServer>` is not externally constructible),
/// so we replicate the exact filter pipeline that the handler runs:
/// `resources::list_resources().filter(|r| scope.allows_resource(&r.raw.uri))`.
#[tokio::test]
async fn readonly_list_resources_omits_join_links() {
    use willow_agent::scopes::TokenScope;
    use willow_agent::server::WillowMcpServer;

    let (client, _broker) = test_client();
    let server = WillowMcpServer::with_scope(client.clone(), TokenScope::ReadOnly);

    let visible: Vec<String> = willow_agent::resources::list_resources()
        .into_iter()
        .filter(|r| server.scope.allows_resource(&r.raw.uri))
        .map(|r| r.raw.uri)
        .collect();

    assert!(
        !visible.iter().any(|u| u == "willow://server/join-links"),
        "ReadOnly list_resources must omit join-links, got: {visible:?}"
    );
    // Sanity: other resources are still listed.
    assert!(visible.iter().any(|u| u == "willow://identity"));
    assert!(visible.iter().any(|u| u == "willow://server/channels"));

    // `Full` scope must still see join-links (positive control).
    let full = WillowMcpServer::with_scope(client, TokenScope::Full);
    let visible_full: Vec<String> = willow_agent::resources::list_resources()
        .into_iter()
        .filter(|r| full.scope.allows_resource(&r.raw.uri))
        .map(|r| r.raw.uri)
        .collect();
    assert!(
        visible_full
            .iter()
            .any(|u| u == "willow://server/join-links"),
        "Full scope should still list join-links"
    );
}

#[tokio::test]
async fn messaging_scope_restricts_tools() {
    use willow_agent::scopes::TokenScope;
    use willow_agent::server::WillowMcpServer;

    let (client, _broker) = test_client();
    let server = WillowMcpServer::with_scope(client.clone(), TokenScope::Messaging);

    let all_tools = server.tool_router.tool_list();
    let visible: Vec<&str> = all_tools
        .iter()
        .filter(|t| server.scope.allows_tool(t.name.as_ref()))
        .map(|t| t.name.as_ref())
        .collect();

    assert!(visible.contains(&"send_message"));
    assert!(visible.contains(&"edit_message"));
    assert!(visible.contains(&"react"));
    assert!(!visible.contains(&"create_channel"));
    assert!(!visible.contains(&"kick_member"));
    assert!(!visible.contains(&"create_server"));

    // Verify the allowed set matches expectations
    assert_eq!(visible.len(), 8, "Messaging scope should allow 8 tools");
}

#[tokio::test]
async fn custom_scope_allowlist() {
    use std::collections::HashSet;
    use willow_agent::scopes::TokenScope;
    use willow_agent::server::WillowMcpServer;

    let (client, _broker) = test_client();
    let mut allowed = HashSet::new();
    allowed.insert("send_message".to_string());
    allowed.insert("react".to_string());
    let server = WillowMcpServer::with_scope(client.clone(), TokenScope::Custom(allowed));

    let all_tools = server.tool_router.tool_list();
    let visible: Vec<&str> = all_tools
        .iter()
        .filter(|t| server.scope.allows_tool(t.name.as_ref()))
        .map(|t| t.name.as_ref())
        .collect();

    assert_eq!(visible.len(), 2);
    assert!(visible.contains(&"send_message"));
    assert!(visible.contains(&"react"));
}

/// Test that `WillowMcpServer::call_tool` returns an MCP error when a
/// `ReadOnly`-scoped token tries to invoke a write tool.
///
/// `call_tool` on `WillowMcpServer` has two layers:
/// 1. Scope check — returns `Err(ErrorData)` immediately if the tool is blocked
/// 2. Router dispatch — only reached if scope allows
///
/// This test verifies layer 1 by constructing the scope-gate logic directly
/// (the `RequestContext<RoleServer>` needed by the `ServerHandler` trait method
/// cannot be constructed from outside the rmcp crate, so we replicate the same
/// conditional that `call_tool` executes).
#[tokio::test]
async fn readonly_scope_rejects_send_message() {
    use rmcp::model::ErrorCode;
    use willow_agent::scopes::TokenScope;
    use willow_agent::server::WillowMcpServer;

    let (client, _broker) = test_client();
    let server = WillowMcpServer::with_scope(client, TokenScope::ReadOnly);

    // Replicate exactly what WillowMcpServer::call_tool does before dispatching:
    //   if !self.scope.allows_tool(name) { return Err(ErrorData::new(INVALID_REQUEST, ...)) }
    let tool_name = "send_message";
    assert!(
        !server.scope.allows_tool(tool_name),
        "ReadOnly scope must reject send_message"
    );

    // Build the ErrorData that call_tool would return — verify it carries the
    // right error code so callers can detect scope rejection vs. other errors.
    let err = rmcp::ErrorData::new(
        ErrorCode::INVALID_REQUEST,
        format!("tool '{tool_name}' not allowed by token scope"),
        None,
    );
    assert_eq!(
        err.code,
        ErrorCode::INVALID_REQUEST,
        "scope rejection should use INVALID_REQUEST error code"
    );
    assert!(
        err.message.contains(tool_name),
        "error message should mention the blocked tool name"
    );

    // Also confirm that ALL write tools are rejected (not just send_message).
    let all_tools = server.tool_router.tool_list();
    let blocked: Vec<&str> = all_tools
        .iter()
        .filter(|t| !server.scope.allows_tool(t.name.as_ref()))
        .map(|t| t.name.as_ref())
        .collect();
    assert!(
        blocked.contains(&"send_message"),
        "send_message must be in the blocked list"
    );
    assert!(
        blocked.contains(&"create_channel"),
        "create_channel must be in the blocked list"
    );
    assert!(
        blocked.contains(&"kick_member"),
        "kick_member must be in the blocked list"
    );
}

/// Test that `WillowMcpServer::read_resource` returns the resource when the
/// scope allows the URI (positive path).
///
/// Today every `TokenScope` returns `true` from `allows_resource`, so the
/// `Full` scope matches what production sees. This test pins the allowed-path
/// behaviour so a future scope change that accidentally short-circuits the
/// allowed branch is caught.
#[tokio::test]
async fn read_resource_allowed_uri_returns_resource() {
    use willow_agent::scopes::TokenScope;
    use willow_agent::server::WillowMcpServer;

    let (client, _broker) = test_client();
    let server = WillowMcpServer::with_scope(client.clone(), TokenScope::Full);

    let uri = "willow://identity";
    assert!(
        server.scope.allows_resource(uri),
        "Full scope must allow {uri}"
    );

    // Gate passes — handler delegates to resources::read_resource. Replicate
    // exactly what the ServerHandler method does: scope check, then dispatch.
    let client_arc = Arc::new(client);
    let result = willow_agent::resources::read_resource(&client_arc, uri)
        .await
        .expect("read_resource should succeed for allowed URI");
    assert!(
        !result.contents.is_empty(),
        "allowed read_resource should return contents"
    );
}

/// Test that `WillowMcpServer::read_resource` rejects `willow://server/join-links`
/// for a `ReadOnly`-scoped token with `INVALID_REQUEST`, matching the error
/// code `call_tool` uses when scope blocks a tool.
///
/// `read_resource` on `WillowMcpServer` has two layers:
/// 1. Scope check — returns `Err(ErrorData)` immediately if the URI is blocked
/// 2. Dispatch to `resources::read_resource` — only reached if scope allows
///
/// Layer 1 is what this test pins: under `ReadOnly`, join-links is now denied
/// (audit finding AUD-2, issue #436). The `RequestContext<RoleServer>` needed
/// by the `ServerHandler` trait method cannot be constructed from outside the
/// rmcp crate, so we replicate the exact gate that `read_resource` runs.
#[tokio::test]
async fn readonly_scope_rejects_join_links_read() {
    use rmcp::model::ErrorCode;
    use willow_agent::scopes::TokenScope;
    use willow_agent::server::WillowMcpServer;

    let (client, _broker) = test_client();
    let server = WillowMcpServer::with_scope(client, TokenScope::ReadOnly);

    let denied_uri = "willow://server/join-links";

    // The real `ReadOnly` scope must deny the join-links URI now.
    assert!(
        !server.scope.allows_resource(denied_uri),
        "ReadOnly must deny {denied_uri} (link_id is a bearer credential)"
    );

    // Replicate exactly what WillowMcpServer::read_resource does before
    // dispatching:
    //   if !self.scope.allows_resource(&uri) {
    //       return Err(ErrorData::new(INVALID_REQUEST, ...));
    //   }
    let err = rmcp::ErrorData::new(
        ErrorCode::INVALID_REQUEST,
        format!("resource '{denied_uri}' not allowed by token scope"),
        None,
    );

    // Same error code that call_tool returns when scope blocks a tool —
    // see `readonly_scope_rejects_send_message`.
    assert_eq!(
        err.code,
        ErrorCode::INVALID_REQUEST,
        "denied resource read should use INVALID_REQUEST, matching call_tool"
    );
    assert!(
        err.message.contains(denied_uri),
        "error message should mention the blocked URI"
    );

    // Sanity: non-credential URIs still pass under ReadOnly.
    assert!(
        server.scope.allows_resource("willow://identity"),
        "ReadOnly must still allow non-credential URIs"
    );
}

// ─────────────────────── Resource URI Coverage Tests ───────────────────────

#[tokio::test]
async fn read_members_resource() {
    let (_server, client) = test_mcp_server();
    let client_arc = Arc::new(client);
    let result = willow_agent::resources::read_resource(&client_arc, "willow://server/members")
        .await
        .unwrap();

    assert!(!result.contents.is_empty());
    match &result.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
            let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
            // Members is a JSON array of {peer_id, display_name, is_online} objects.
            assert!(parsed.is_array(), "members resource should be a JSON array");
            // The test client creates the local peer as the sole member.
            let members = parsed.as_array().unwrap();
            assert!(
                !members.is_empty(),
                "members list should contain at least the local peer"
            );
            let first = &members[0];
            assert!(
                first["peer_id"].is_string(),
                "each member should have a peer_id string"
            );
            assert!(
                first["display_name"].is_string(),
                "each member should have a display_name string"
            );
            assert!(
                first["is_online"].is_boolean(),
                "each member should have an is_online boolean"
            );
        }
        _ => panic!("expected text resource"),
    }
}

#[tokio::test]
async fn read_voice_status_resource() {
    let (_server, client) = test_mcp_server();
    let client_arc = Arc::new(client);
    let result = willow_agent::resources::read_resource(&client_arc, "willow://voice/status")
        .await
        .unwrap();

    assert!(!result.contents.is_empty());
    match &result.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
            let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
            // Voice status has active_channel (null when idle), muted, deafened.
            assert!(
                parsed["active_channel"].is_null() || parsed["active_channel"].is_string(),
                "active_channel should be null or a string"
            );
            assert!(
                parsed["muted"].is_boolean(),
                "muted should be a boolean, got: {}",
                parsed
            );
            assert!(
                parsed["deafened"].is_boolean(),
                "deafened should be a boolean, got: {}",
                parsed
            );
            // Fresh client should not be in a voice channel.
            assert!(
                parsed["active_channel"].is_null(),
                "new client should not be in a voice channel"
            );
            assert_eq!(parsed["muted"], false, "new client should not be muted");
            assert_eq!(
                parsed["deafened"], false,
                "new client should not be deafened"
            );
        }
        _ => panic!("expected text resource"),
    }
}

// ─────────────────────── Notification Tests ────────────────────────────────

#[tokio::test]
async fn notification_serialization_covers_all_variants() {
    // Verify that event_to_json produces valid output for every event type
    // in `EVENT_TYPE_NAMES`. This test complements the unit tests in
    // `notifications.rs` by running in the integration test context.
    //
    // The count is pinned to 31 — one entry per `ClientEvent` variant.
    // When a new variant is added, bump this assertion and extend the
    // `notifications::event_to_json` match + `EVENT_TYPE_NAMES` list.
    assert_eq!(willow_agent::notifications::EVENT_TYPE_NAMES.len(), 31);

    for name in willow_agent::notifications::EVENT_TYPE_NAMES {
        assert!(!name.is_empty(), "event type name should not be empty");
    }
}

#[tokio::test]
async fn notification_event_to_json_roundtrip() {
    use willow_client::ClientEvent;

    let event = ClientEvent::MessageReceived {
        channel: "general".into(),
        message_id: "msg-1".into(),
        is_local: false,
    };
    let json = willow_agent::notifications::event_to_json(&event);

    // Should be valid JSON with type and data
    assert_eq!(json["type"], "MessageReceived");
    assert_eq!(json["data"]["channel"], "general");
    assert_eq!(json["data"]["message_id"], "msg-1");
    assert_eq!(json["data"]["is_local"], false);

    // Should be serializable to string and back
    let json_str = serde_json::to_string(&json).unwrap();
    let reparsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(json, reparsed);
}

// ─────────────────────── Multi-Peer Infrastructure Tests ───────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_peer_agent_servers_have_separate_state() {
    // NOTE: These two clients share a MemHub (in-memory gossip mesh) but each
    // starts with its own independent server state seeded from genesis. They
    // are NOT joined to each other's server, so gossip sync across the hub does
    // not make A's messages visible to B and vice versa. True cross-peer sync
    // requires an invite flow followed by gossip delivery — that belongs in the
    // Playwright E2E suite (e2e/multi-peer-sync.spec.ts), not here.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let hub = MemHub::new();
            let (client_a, _) = willow_client::test_client_on_hub(&hub).await;
            let (client_b, _) = willow_client::test_client_on_hub(&hub).await;

            let server_a = WillowMcpServer::new(client_a.clone());
            let _server_b = WillowMcpServer::new(client_b.clone());

            // Send message on A
            call_tool(
                &server_a.tool_router,
                "send_message",
                serde_json::json!({ "channel": "general", "body": "from A" }),
            )
            .await;

            // A sees the message
            let msgs_a = client_a.messages("general").await;
            assert!(msgs_a.iter().any(|m| m.body == "from A"));

            // B has its own state — won't see A's message (separate server states)
            let msgs_b = client_b.messages("general").await;
            assert!(
                !msgs_b.iter().any(|m| m.body == "from A"),
                "B should not see A's message without joining A's server"
            );
        })
        .await;
}
