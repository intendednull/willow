//! # MCP Server
//!
//! Sets up the MCP server using rmcp, wiring tools, resources, and
//! notifications to a `ClientHandle`.

use std::sync::Arc;

use rmcp::{
    model::*,
    service::{Peer, RequestContext},
    ErrorData, RoleServer, ServerHandler,
};

use crate::resources;
use crate::scopes::TokenScope;
use crate::tools;
use willow_client::ClientHandle;
use willow_network::Network;

/// MCP server backed by a Willow `ClientHandle`.
#[derive(Clone)]
pub struct WillowMcpServer<N: Network> {
    pub(crate) client: Arc<ClientHandle<N>>,
    pub tool_router: tools::WillowToolRouter<N>,
    pub scope: TokenScope,
    /// Ensures the notification bridge is started at most once per server instance.
    notification_started: Arc<tokio::sync::OnceCell<()>>,
}

impl<N: Network> WillowMcpServer<N> {
    /// Create a new MCP server wrapping the given client handle.
    pub fn new(client: ClientHandle<N>) -> Self {
        let client = Arc::new(client);
        let tool_router = tools::WillowToolRouter::new(Arc::clone(&client));
        Self {
            client,
            tool_router,
            scope: TokenScope::default(),
            notification_started: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    /// Create a new MCP server with a specific token scope.
    pub fn with_scope(client: ClientHandle<N>, scope: TokenScope) -> Self {
        let client = Arc::new(client);
        let tool_router = tools::WillowToolRouter::new(Arc::clone(&client));
        Self {
            client,
            tool_router,
            scope,
            notification_started: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    /// Start the notification bridge if not already running. Subscribes to
    /// `Broker<ClientEvent>` and forwards each event as a custom MCP notification.
    pub(crate) fn ensure_notification_bridge(&self, peer: Peer<RoleServer>) {
        let client = Arc::clone(&self.client);
        let started = Arc::clone(&self.notification_started);
        tokio::spawn(async move {
            // Only start once per server instance.
            let already = started
                .get_or_init(|| async {
                    let mut events = client.subscribe_events().await;
                    let p = peer;
                    tokio::spawn(async move {
                        while let Some(event) = events.recv().await {
                            let json = crate::notifications::event_to_json(&event);
                            let notif = CustomNotification::new("willow/event", Some(json));
                            if p.send_notification(notif.into()).await.is_err() {
                                // Transport closed — stop forwarding.
                                break;
                            }
                        }
                    });
                })
                .await;
            let _ = already;
        });
    }
}

#[allow(clippy::manual_async_fn)]
impl<N: Network> ServerHandler for WillowMcpServer<N> {
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            "willow-agent",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "Willow P2P chat agent. Use tools to send messages, manage channels, \
             and administer servers. Read resources for current state.",
        )
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        // Start the notification bridge on the first request (list_tools is
        // always the first method called by the client during initialization).
        self.ensure_notification_bridge(context.peer);
        async {
            let tools = self
                .tool_router
                .tool_list()
                .into_iter()
                .filter(|t| self.scope.allows_tool(t.name.as_ref()))
                .collect();
            Ok(ListToolsResult {
                tools,
                next_cursor: None,
                meta: None,
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResult, ErrorData>> + Send + '_ {
        async move {
            let name = request.name.as_ref();
            if !self.scope.allows_tool(name) {
                return Err(ErrorData::new(
                    ErrorCode::INVALID_REQUEST,
                    format!("tool '{name}' not allowed by token scope"),
                    None,
                ));
            }
            self.tool_router.call(&request).await
        }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + Send + '_ {
        async {
            let resources = resources::list_resources()
                .into_iter()
                .filter(|r| self.scope.allows_resource(&r.raw.uri))
                .collect();
            Ok(ListResourcesResult {
                resources,
                next_cursor: None,
                meta: None,
            })
        }
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResult, ErrorData>> + Send + '_ {
        async move {
            if !self.scope.allows_resource(&request.uri) {
                return Err(ErrorData::new(
                    ErrorCode::INVALID_REQUEST,
                    format!("resource '{}' not allowed by token scope", request.uri),
                    None,
                ));
            }
            resources::read_resource(&self.client, &request.uri).await
        }
    }
}

use std::future::Future;

/// Serve the MCP server over stdio.
pub async fn serve_stdio<N: Network>(client: ClientHandle<N>) -> anyhow::Result<()> {
    let server = WillowMcpServer::new(client);
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport).await?;
    // The notification bridge is also started in list_tools (first handler
    // called during init), but we start it here too as a safety net.
    service
        .service()
        .ensure_notification_bridge(service.peer().clone());
    service.waiting().await?;
    Ok(())
}

/// Serve the MCP server over Streamable HTTP (SSE/JSON) with bearer token auth.
pub async fn serve_http<N: Network + 'static>(
    client: ClientHandle<N>,
    bind: &str,
    scope: TokenScope,
    token: String,
) -> anyhow::Result<()> {
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };

    let config = StreamableHttpServerConfig::default();
    let session_manager = Arc::new(LocalSessionManager::default());

    let service = StreamableHttpService::new(
        move || Ok(WillowMcpServer::with_scope(client.clone(), scope.clone())),
        session_manager,
        config,
    );

    let app = axum::Router::new()
        .route_service("/mcp", service)
        .layer(axum::middleware::from_fn(move |req, next| {
            let expected = token.clone();
            bearer_auth_middleware(req, next, expected)
        }));

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("MCP HTTP server listening on {bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Axum middleware that validates `Authorization: Bearer <token>` on every request.
async fn bearer_auth_middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
    expected_token: String,
) -> axum::response::Response {
    use axum::http::StatusCode;

    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(value) if value.starts_with("Bearer ") => {
            let provided = &value["Bearer ".len()..];
            if tokens_eq_ct(provided.as_bytes(), expected_token.as_bytes()) {
                next.run(req).await
            } else {
                (StatusCode::FORBIDDEN, "invalid bearer token").into_response()
            }
        }
        _ => (StatusCode::UNAUTHORIZED, "missing bearer token").into_response(),
    }
}

/// Constant-time equality check for bearer tokens.
///
/// Length is compared first to avoid leaking the expected length via timing,
/// then `subtle::ConstantTimeEq` performs a branch-free byte-wise comparison.
/// This prevents timing side-channel attacks where an attacker measures
/// response latency to recover the token byte by byte (issues #301, #304).
fn tokens_eq_ct(provided: &[u8], expected: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if provided.len() != expected.len() {
        return false;
    }
    provided.ct_eq(expected).into()
}

use axum::response::IntoResponse;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_info_is_correct() {
        // We can't easily construct a WillowMcpServer without a full client,
        // but we can test the static parts.
        let info = InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            "willow-agent",
            env!("CARGO_PKG_VERSION"),
        ));
        assert_eq!(info.server_info.name, "willow-agent");
    }

    #[test]
    fn tokens_eq_ct_accepts_correct_token() {
        let token = "s3cret-bearer-token";
        assert!(tokens_eq_ct(token.as_bytes(), token.as_bytes()));
    }

    #[test]
    fn tokens_eq_ct_rejects_wrong_token_same_length() {
        let expected = "s3cret-bearer-token";
        let provided = "X3cret-bearer-token";
        assert_eq!(provided.len(), expected.len());
        assert!(!tokens_eq_ct(provided.as_bytes(), expected.as_bytes()));
    }

    #[test]
    fn tokens_eq_ct_rejects_shorter_token() {
        let expected = "s3cret-bearer-token";
        let provided = "s3cret-bearer";
        assert!(!tokens_eq_ct(provided.as_bytes(), expected.as_bytes()));
    }

    #[test]
    fn tokens_eq_ct_rejects_longer_token() {
        let expected = "s3cret-bearer-token";
        let provided = "s3cret-bearer-token-extra";
        assert!(!tokens_eq_ct(provided.as_bytes(), expected.as_bytes()));
    }

    #[test]
    fn tokens_eq_ct_rejects_empty_against_nonempty() {
        let expected = "s3cret-bearer-token";
        assert!(!tokens_eq_ct(b"", expected.as_bytes()));
    }
}
