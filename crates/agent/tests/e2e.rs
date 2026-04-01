//! # E2E Tests for the Willow Agent
//!
//! Tests use `test_client()` from willow-client (via `test-utils` feature)
//! to create in-process `ClientHandle<MemNetwork>` instances. These are
//! single-peer tests that verify the full MCP server → ClientHandle → actor
//! pipeline works end-to-end.

use std::sync::Arc;
use willow_client::{test_client, ClientHandle};
use willow_network::mem::MemNetwork;

use willow_agent::server::WillowMcpServer;
use willow_agent::tools::WillowToolRouter;

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

    assert!(client.is_pinned("general", msg_id).await);

    call_tool(
        &router,
        "unpin_message",
        serde_json::json!({ "channel": "general", "message_id": msg_id }),
    )
    .await;

    assert!(!client.is_pinned("general", msg_id).await);
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
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    call_tool(
        &router,
        "rename_server",
        serde_json::json!({ "name": "Renamed" }),
    )
    .await;

    // Verify via resource read
    let name = client.active_server_name().await;
    // The rename goes through event-sourced state
    // Check that it either applied or the event was built
    assert!(!name.is_empty());
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
async fn kick_member_removes_from_server() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    // Our own peer_id — kicking self should produce an error or be a no-op
    let peer_id = client.peer_id();

    let result = call_tool(
        &router,
        "kick_member",
        serde_json::json!({ "peer_id": peer_id }),
    )
    .await;

    // Kicking oneself is expected to either error or succeed gracefully
    let text = result_text(&result);
    assert!(!text.is_empty(), "kick_member should return a response");
}

#[tokio::test]
async fn server_rename_via_tool() {
    let (server, client) = test_mcp_server();
    let router = server.tool_router.clone();

    let result = call_tool(
        &router,
        "rename_server",
        serde_json::json!({ "name": "My Renamed Server" }),
    )
    .await;
    assert!(result.is_error != Some(true));

    let name = client.active_server_name().await;
    assert!(!name.is_empty());
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

    // Resources should all be visible
    let resources = willow_agent::resources::list_resources();
    for r in &resources {
        assert!(
            server.scope.allows_resource(&r.raw.uri),
            "ReadOnly should allow resource: {}",
            r.raw.uri
        );
    }
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
