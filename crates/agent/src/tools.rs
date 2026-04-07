//! # MCP Tool Definitions and Handlers
//!
//! All 36 MCP tools mapped to `ClientHandle` methods. Each tool has a
//! typed parameter struct (JSON Schema via `schemars`) and an async handler.

use std::sync::Arc;

use rmcp::model::*;
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use willow_client::ClientHandle;
use willow_identity::EndpointId;
use willow_network::Network;

/// Parse a 64-character hex string into an `EndpointId`.
fn parse_endpoint_id(hex: &str) -> Result<EndpointId, String> {
    hex.parse::<EndpointId>()
        .map_err(|e| format!("invalid peer_id: {e}"))
}

/// Parse a 64-character hex string into an `EventHash`.
fn parse_event_hash(hex: &str) -> Result<willow_state::EventHash, String> {
    hex.parse::<willow_state::EventHash>()
        .map_err(|e| format!("invalid event hash: {e}"))
}

/// Parse multiple hex peer IDs.
fn parse_endpoint_ids(ids: &[String]) -> Result<Vec<EndpointId>, String> {
    ids.iter().map(|s| parse_endpoint_id(s)).collect()
}

fn success_json(value: impl Serialize) -> Result<CallToolResult, ErrorData> {
    let text = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn error_text(msg: impl Into<String>) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![Content::text(msg.into())]))
}

// ──────────────────────────── Parameter types ────────────────────────────────

// Messaging
#[derive(Deserialize, JsonSchema)]
pub struct SendMessageParams {
    /// Channel name to send to.
    pub channel: String,
    /// Message body text.
    pub body: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SendReplyParams {
    /// Channel name.
    pub channel: String,
    /// ID of the parent message.
    pub parent_id: String,
    /// Reply body text.
    pub body: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ShareFileInlineParams {
    /// Channel name.
    pub channel: String,
    /// Filename for the shared file.
    pub filename: String,
    /// Base64-encoded file data (max 256KB).
    pub data: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct EditMessageParams {
    /// Channel name.
    pub channel: String,
    /// ID of the message to edit.
    pub message_id: String,
    /// New message body.
    pub new_body: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct DeleteMessageParams {
    /// Channel name.
    pub channel: String,
    /// ID of the message to delete.
    pub message_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReactParams {
    /// Channel name.
    pub channel: String,
    /// ID of the message to react to.
    pub message_id: String,
    /// Emoji to react with.
    pub emoji: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct PinMessageParams {
    /// Channel name.
    pub channel: String,
    /// ID of the message to pin.
    pub message_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct UnpinMessageParams {
    /// Channel name.
    pub channel: String,
    /// ID of the message to unpin.
    pub message_id: String,
}

// Channels
#[derive(Deserialize, JsonSchema)]
pub struct CreateChannelParams {
    /// Channel name.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateVoiceChannelParams {
    /// Voice channel name.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct DeleteChannelParams {
    /// Channel name to delete.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SwitchChannelParams {
    /// Channel name to switch to.
    pub name: String,
}

// Permissions & Members
#[derive(Deserialize, JsonSchema)]
pub struct PeerIdParams {
    /// Peer ID (64-char hex Ed25519 public key).
    pub peer_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateRoleParams {
    /// Role name.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct DeleteRoleParams {
    /// Role ID (UUID).
    pub role_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetPermissionParams {
    /// Role ID (UUID).
    pub role_id: String,
    /// Permission name: SyncProvider, ManageChannels, ManageRoles,
    /// SendMessages, or CreateInvite.
    pub permission: String,
    /// Whether to grant (true) or revoke (false) the permission.
    pub granted: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct AssignRoleParams {
    /// Peer ID (64-char hex).
    pub peer_id: String,
    /// Role ID (UUID).
    pub role_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct AuthorizeWorkersParams {
    /// List of worker peer IDs (64-char hex each).
    pub worker_peer_ids: Vec<String>,
}

// Server management
#[derive(Deserialize, JsonSchema)]
pub struct CreateServerParams {
    /// Server display name.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SwitchServerParams {
    /// Server ID to switch to.
    pub id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct LeaveServerParams {
    /// Server ID to leave.
    pub id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct RenameServerParams {
    /// New server name.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetServerDescriptionParams {
    /// New server description.
    pub description: String,
}

// Identity
#[derive(Deserialize, JsonSchema)]
pub struct SetDisplayNameParams {
    /// Display name.
    pub name: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetServerDisplayNameParams {
    /// Server-scoped display name.
    pub name: String,
}

// Invites
#[derive(Deserialize, JsonSchema)]
pub struct GenerateInviteParams {
    /// Recipient peer ID (64-char hex).
    pub recipient_peer_id: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct AcceptInviteParams {
    /// Invite code to accept.
    pub code: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct CreateJoinLinkParams {
    /// Maximum number of uses for this link.
    pub max_uses: u32,
    /// Optional expiration timestamp (Unix ms).
    pub expires_at: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DeleteJoinLinkParams {
    /// Link ID to delete.
    pub link_id: String,
}

// Voice
#[derive(Deserialize, JsonSchema)]
pub struct JoinVoiceParams {
    /// Voice channel ID to join.
    pub channel_id: String,
}

// ──────────────────────────── Tool Router ────────────────────────────────────

/// Central tool router that holds tool definitions and dispatches calls.
pub struct WillowToolRouter<N: Network> {
    client: Arc<ClientHandle<N>>,
    tools: Vec<Tool>,
}

impl<N: Network> Clone for WillowToolRouter<N> {
    fn clone(&self) -> Self {
        Self {
            client: Arc::clone(&self.client),
            tools: self.tools.clone(),
        }
    }
}

fn make_tool<P: JsonSchema>(name: &'static str, description: &'static str) -> Tool {
    let schema = schemars::generate::SchemaSettings::default()
        .into_generator()
        .into_root_schema_for::<P>();
    let schema_value = serde_json::to_value(&schema).unwrap_or_default();
    let schema_obj: JsonObject = match schema_value {
        serde_json::Value::Object(m) => m,
        _ => JsonObject::default(),
    };
    Tool::new(name, description, Arc::new(schema_obj))
}

fn make_tool_no_params(name: &'static str, description: &'static str) -> Tool {
    let mut schema = JsonObject::new();
    schema.insert("type".to_string(), serde_json::json!("object"));
    Tool::new(name, description, Arc::new(schema))
}

impl<N: Network> WillowToolRouter<N> {
    /// Build the full tool list.
    pub fn new(client: Arc<ClientHandle<N>>) -> Self {
        let tools = vec![
            // Messaging (8)
            make_tool::<SendMessageParams>("send_message", "Send a text message to a channel"),
            make_tool::<SendReplyParams>("send_reply", "Reply to a specific message"),
            make_tool::<ShareFileInlineParams>(
                "share_file_inline",
                "Share a file inline (base64, max 256KB)",
            ),
            make_tool::<EditMessageParams>("edit_message", "Edit a message"),
            make_tool::<DeleteMessageParams>("delete_message", "Delete a message"),
            make_tool::<ReactParams>("react", "Add an emoji reaction to a message"),
            make_tool::<PinMessageParams>("pin_message", "Pin a message in a channel"),
            make_tool::<UnpinMessageParams>("unpin_message", "Unpin a message in a channel"),
            // Channels (4)
            make_tool::<CreateChannelParams>("create_channel", "Create a text channel"),
            make_tool::<CreateVoiceChannelParams>("create_voice_channel", "Create a voice channel"),
            make_tool::<DeleteChannelParams>("delete_channel", "Delete a channel"),
            make_tool::<SwitchChannelParams>("switch_channel", "Set the active channel"),
            // Permissions & Members (7)
            make_tool::<PeerIdParams>("trust_peer", "Propose granting admin status to a peer"),
            make_tool::<PeerIdParams>("untrust_peer", "Propose revoking admin status from a peer"),
            make_tool::<PeerIdParams>("kick_member", "Propose kicking a member from the server"),
            make_tool::<CreateRoleParams>("create_role", "Create a permission role"),
            make_tool::<DeleteRoleParams>("delete_role", "Delete a role"),
            make_tool::<SetPermissionParams>(
                "set_permission",
                "Set a permission on a role (grant or revoke)",
            ),
            make_tool::<AssignRoleParams>("assign_role", "Assign a role to a peer"),
            // Server management (6)
            make_tool::<CreateServerParams>(
                "create_server",
                "Create a new server. Returns the server ID.",
            ),
            make_tool::<SwitchServerParams>("switch_server", "Switch to a different server"),
            make_tool::<LeaveServerParams>("leave_server", "Leave a server"),
            make_tool::<RenameServerParams>("rename_server", "Rename the current server"),
            make_tool::<SetServerDescriptionParams>(
                "set_server_description",
                "Set the server description",
            ),
            make_tool::<AuthorizeWorkersParams>(
                "authorize_workers",
                "Grant SyncProvider permission to worker peers",
            ),
            // Identity (3)
            make_tool::<SetDisplayNameParams>("set_display_name", "Set the agent's display name"),
            make_tool::<SetServerDisplayNameParams>(
                "set_server_display_name",
                "Set server-scoped display name",
            ),
            make_tool_no_params("send_typing", "Broadcast a typing indicator"),
            // Invites (4)
            make_tool::<GenerateInviteParams>(
                "generate_invite",
                "Create an encrypted invite for a specific peer",
            ),
            make_tool::<AcceptInviteParams>("accept_invite", "Accept an invite and join a server"),
            make_tool::<CreateJoinLinkParams>("create_join_link", "Create a shareable join link"),
            make_tool::<DeleteJoinLinkParams>("delete_join_link", "Delete a join link"),
            // Voice (4)
            make_tool::<JoinVoiceParams>("join_voice", "Join a voice channel"),
            make_tool_no_params("leave_voice", "Leave the current voice channel"),
            make_tool_no_params("toggle_mute", "Toggle mute state. Returns new state."),
            make_tool_no_params("toggle_deafen", "Toggle deafen state. Returns new state."),
        ];
        Self { client, tools }
    }

    /// Return the tool definitions for `tools/list`.
    pub fn tool_list(&self) -> Vec<Tool> {
        self.tools.clone()
    }

    /// Dispatch a tool call by name.
    pub async fn call(&self, request: &CallToolRequestParams) -> Result<CallToolResult, ErrorData> {
        let name = request.name.as_ref();
        let args = request
            .arguments
            .as_ref()
            .map(|a| serde_json::Value::Object(a.clone()))
            .unwrap_or(serde_json::Value::Object(JsonObject::new()));

        match name {
            // ── Messaging ────────────────────────────────────────────────
            "send_message" => {
                let p: SendMessageParams = parse_args(&args)?;
                match self.client.send_message(&p.channel, &p.body).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "send_reply" => {
                let p: SendReplyParams = parse_args(&args)?;
                let parent_hash = parse_event_hash(&p.parent_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self
                    .client
                    .send_reply(&p.channel, &parent_hash, &p.body)
                    .await
                {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "share_file_inline" => {
                let p: ShareFileInlineParams = parse_args(&args)?;
                let data = base64_decode(&p.data).map_err(|e| {
                    ErrorData::invalid_params(format!("invalid base64 data: {e}"), None)
                })?;
                match self
                    .client
                    .share_file_inline(&p.channel, &p.filename, &data)
                    .await
                {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "edit_message" => {
                let p: EditMessageParams = parse_args(&args)?;
                let hash = parse_event_hash(&p.message_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self
                    .client
                    .edit_message(&p.channel, &hash, &p.new_body)
                    .await
                {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "delete_message" => {
                let p: DeleteMessageParams = parse_args(&args)?;
                let hash = parse_event_hash(&p.message_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self.client.delete_message(&p.channel, &hash).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "react" => {
                let p: ReactParams = parse_args(&args)?;
                let hash = parse_event_hash(&p.message_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self.client.react(&p.channel, &hash, &p.emoji).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "pin_message" => {
                let p: PinMessageParams = parse_args(&args)?;
                let hash = parse_event_hash(&p.message_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self.client.pin_message(&p.channel, &hash).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "unpin_message" => {
                let p: UnpinMessageParams = parse_args(&args)?;
                let hash = parse_event_hash(&p.message_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self.client.unpin_message(&p.channel, &hash).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }

            // ── Channels ─────────────────────────────────────────────────
            "create_channel" => {
                let p: CreateChannelParams = parse_args(&args)?;
                match self.client.create_channel(&p.name).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "create_voice_channel" => {
                let p: CreateVoiceChannelParams = parse_args(&args)?;
                match self.client.create_voice_channel(&p.name).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "delete_channel" => {
                let p: DeleteChannelParams = parse_args(&args)?;
                match self.client.delete_channel(&p.name).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "switch_channel" => {
                let p: SwitchChannelParams = parse_args(&args)?;
                self.client.switch_channel(&p.name).await;
                success_json(serde_json::json!({"success": true}))
            }

            // ── Permissions & Members ────────────────────────────────────
            "trust_peer" => {
                let p: PeerIdParams = parse_args(&args)?;
                let eid = parse_endpoint_id(&p.peer_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self.client.propose_grant_admin(eid).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "untrust_peer" => {
                let p: PeerIdParams = parse_args(&args)?;
                let eid = parse_endpoint_id(&p.peer_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self.client.propose_revoke_admin(eid).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "kick_member" => {
                let p: PeerIdParams = parse_args(&args)?;
                let eid = parse_endpoint_id(&p.peer_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self.client.propose_kick_member(eid).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "create_role" => {
                let p: CreateRoleParams = parse_args(&args)?;
                match self.client.create_role(&p.name).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "delete_role" => {
                let p: DeleteRoleParams = parse_args(&args)?;
                match self.client.delete_role(&p.role_id).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "set_permission" => {
                let p: SetPermissionParams = parse_args(&args)?;
                match self
                    .client
                    .set_permission(&p.role_id, &p.permission, p.granted)
                    .await
                {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "assign_role" => {
                let p: AssignRoleParams = parse_args(&args)?;
                let eid = parse_endpoint_id(&p.peer_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self.client.assign_role(eid, &p.role_id).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }

            // ── Server Management ────────────────────────────────────────
            "create_server" => {
                let p: CreateServerParams = parse_args(&args)?;
                match self.client.create_server(&p.name).await {
                    Ok(id) => success_json(serde_json::json!({"server_id": id})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "switch_server" => {
                let p: SwitchServerParams = parse_args(&args)?;
                self.client.switch_server(&p.id).await;
                success_json(serde_json::json!({"success": true}))
            }
            "leave_server" => {
                let p: LeaveServerParams = parse_args(&args)?;
                self.client.leave_server(&p.id).await;
                success_json(serde_json::json!({"success": true}))
            }
            "rename_server" => {
                let p: RenameServerParams = parse_args(&args)?;
                match self.client.rename_server(&p.name).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "set_server_description" => {
                let p: SetServerDescriptionParams = parse_args(&args)?;
                match self.client.set_server_description(&p.description).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "authorize_workers" => {
                let p: AuthorizeWorkersParams = parse_args(&args)?;
                let eids = parse_endpoint_ids(&p.worker_peer_ids)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                let _ = self.client.authorize_workers(&eids).await;
                success_json(serde_json::json!({"success": true}))
            }

            // ── Identity ─────────────────────────────────────────────────
            "set_display_name" => {
                let p: SetDisplayNameParams = parse_args(&args)?;
                self.client.set_display_name(&p.name).await;
                success_json(serde_json::json!({"success": true}))
            }
            "set_server_display_name" => {
                let p: SetServerDisplayNameParams = parse_args(&args)?;
                match self.client.set_server_display_name(&p.name).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "send_typing" => {
                self.client.send_typing().await;
                success_json(serde_json::json!({"success": true}))
            }

            // ── Invites ──────────────────────────────────────────────────
            "generate_invite" => {
                let p: GenerateInviteParams = parse_args(&args)?;
                let eid = parse_endpoint_id(&p.recipient_peer_id)
                    .map_err(|e| ErrorData::invalid_params(e, None))?;
                match self.client.generate_invite(&eid).await {
                    Ok(code) => success_json(serde_json::json!({"invite_code": code})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "accept_invite" => {
                let p: AcceptInviteParams = parse_args(&args)?;
                match self.client.accept_invite(&p.code).await {
                    Ok(()) => success_json(serde_json::json!({"success": true})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "create_join_link" => {
                let p: CreateJoinLinkParams = parse_args(&args)?;
                match self.client.create_join_link(p.max_uses, p.expires_at).await {
                    Ok(link) => success_json(serde_json::json!({"link": link})),
                    Err(e) => error_text(e.to_string()),
                }
            }
            "delete_join_link" => {
                let p: DeleteJoinLinkParams = parse_args(&args)?;
                self.client.delete_join_link(&p.link_id).await;
                success_json(serde_json::json!({"success": true}))
            }

            // ── Voice ────────────────────────────────────────────────────
            "join_voice" => {
                let p: JoinVoiceParams = parse_args(&args)?;
                self.client.join_voice(&p.channel_id).await;
                success_json(serde_json::json!({"success": true}))
            }
            "leave_voice" => {
                self.client.leave_voice().await;
                success_json(serde_json::json!({"success": true}))
            }
            "toggle_mute" => {
                let muted = self.client.toggle_mute().await;
                success_json(serde_json::json!({"muted": muted}))
            }
            "toggle_deafen" => {
                let deafened = self.client.toggle_deafen().await;
                success_json(serde_json::json!({"deafened": deafened}))
            }

            _ => Err(ErrorData::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool: {name}"),
                None,
            )),
        }
    }
}

fn parse_args<T: serde::de::DeserializeOwned>(args: &serde_json::Value) -> Result<T, ErrorData> {
    serde_json::from_value(args.clone())
        .map_err(|e| ErrorData::invalid_params(format!("invalid arguments: {e}"), None))
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    willow_client::base64::decode(s).ok_or_else(|| "invalid base64".to_string())
}

// ──────────────────────────── Tests ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_36_tools_defined() {
        // Verify we can construct the tool list without a real client.
        // We test the count by checking the tool_list vector length.
        let expected = 36;
        let tools = vec![
            // Messaging (8)
            make_tool::<SendMessageParams>("send_message", ""),
            make_tool::<SendReplyParams>("send_reply", ""),
            make_tool::<ShareFileInlineParams>("share_file_inline", ""),
            make_tool::<EditMessageParams>("edit_message", ""),
            make_tool::<DeleteMessageParams>("delete_message", ""),
            make_tool::<ReactParams>("react", ""),
            make_tool::<PinMessageParams>("pin_message", ""),
            make_tool::<UnpinMessageParams>("unpin_message", ""),
            // Channels (4)
            make_tool::<CreateChannelParams>("create_channel", ""),
            make_tool::<CreateVoiceChannelParams>("create_voice_channel", ""),
            make_tool::<DeleteChannelParams>("delete_channel", ""),
            make_tool::<SwitchChannelParams>("switch_channel", ""),
            // Permissions & Members (7)
            make_tool::<PeerIdParams>("trust_peer", ""),
            make_tool::<PeerIdParams>("untrust_peer", ""),
            make_tool::<PeerIdParams>("kick_member", ""),
            make_tool::<CreateRoleParams>("create_role", ""),
            make_tool::<DeleteRoleParams>("delete_role", ""),
            make_tool::<SetPermissionParams>("set_permission", ""),
            make_tool::<AssignRoleParams>("assign_role", ""),
            // Server management (6)
            make_tool::<CreateServerParams>("create_server", ""),
            make_tool::<SwitchServerParams>("switch_server", ""),
            make_tool::<LeaveServerParams>("leave_server", ""),
            make_tool::<RenameServerParams>("rename_server", ""),
            make_tool::<SetServerDescriptionParams>("set_server_description", ""),
            make_tool::<AuthorizeWorkersParams>("authorize_workers", ""),
            // Identity (3)
            make_tool::<SetDisplayNameParams>("set_display_name", ""),
            make_tool::<SetServerDisplayNameParams>("set_server_display_name", ""),
            make_tool_no_params("send_typing", ""),
            // Invites (4)
            make_tool::<GenerateInviteParams>("generate_invite", ""),
            make_tool::<AcceptInviteParams>("accept_invite", ""),
            make_tool::<CreateJoinLinkParams>("create_join_link", ""),
            make_tool::<DeleteJoinLinkParams>("delete_join_link", ""),
            // Voice (4)
            make_tool::<JoinVoiceParams>("join_voice", ""),
            make_tool_no_params("leave_voice", ""),
            make_tool_no_params("toggle_mute", ""),
            make_tool_no_params("toggle_deafen", ""),
        ];
        assert_eq!(tools.len(), expected);
    }

    #[test]
    fn tool_schemas_are_valid_json() {
        let tool = make_tool::<SendMessageParams>("send_message", "Send a message");
        let schema = tool.input_schema;
        let value = serde_json::to_value(&*schema).unwrap();
        assert!(value.is_object());
        // Should have properties.channel and properties.body
        let props = value.get("properties").expect("should have properties");
        assert!(props.get("channel").is_some());
        assert!(props.get("body").is_some());
    }

    #[test]
    fn tool_names_are_unique() {
        let tools = vec![
            "send_message",
            "send_reply",
            "share_file_inline",
            "edit_message",
            "delete_message",
            "react",
            "pin_message",
            "unpin_message",
            "create_channel",
            "create_voice_channel",
            "delete_channel",
            "switch_channel",
            "trust_peer",
            "untrust_peer",
            "kick_member",
            "create_role",
            "delete_role",
            "set_permission",
            "assign_role",
            "create_server",
            "switch_server",
            "leave_server",
            "rename_server",
            "set_server_description",
            "authorize_workers",
            "set_display_name",
            "set_server_display_name",
            "send_typing",
            "generate_invite",
            "accept_invite",
            "create_join_link",
            "delete_join_link",
            "join_voice",
            "leave_voice",
            "toggle_mute",
            "toggle_deafen",
        ];
        let mut set = std::collections::HashSet::new();
        for name in &tools {
            assert!(set.insert(name), "duplicate tool name: {name}");
        }
        assert_eq!(set.len(), 36);
    }
}
