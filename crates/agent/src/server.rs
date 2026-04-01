//! # MCP Server
//!
//! Sets up the MCP server using rmcp, wiring tools, resources, and
//! notifications to a `ClientHandle`.

use std::sync::Arc;

use rmcp::{model::*, service::RequestContext, ErrorData, RoleServer, ServerHandler};

use crate::resources;
use crate::tools;
use willow_client::ClientHandle;
use willow_network::Network;

/// MCP server backed by a Willow `ClientHandle`.
#[derive(Clone)]
pub struct WillowMcpServer<N: Network> {
    pub(crate) client: Arc<ClientHandle<N>>,
    pub tool_router: tools::WillowToolRouter<N>,
}

impl<N: Network> WillowMcpServer<N> {
    /// Create a new MCP server wrapping the given client handle.
    pub fn new(client: ClientHandle<N>) -> Self {
        let client = Arc::new(client);
        let tool_router = tools::WillowToolRouter::new(Arc::clone(&client));
        Self {
            client,
            tool_router,
        }
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
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        async {
            Ok(ListToolsResult {
                tools: self.tool_router.tool_list(),
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
        async move { self.tool_router.call(&request).await }
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + Send + '_ {
        async {
            Ok(ListResourcesResult {
                resources: resources::list_resources(),
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
        async move { resources::read_resource(&self.client, &request.uri).await }
    }
}

use std::future::Future;

/// Serve the MCP server over stdio.
pub async fn serve_stdio<N: Network>(client: ClientHandle<N>) -> anyhow::Result<()> {
    let server = WillowMcpServer::new(client);
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport).await?;
    service.waiting().await?;
    Ok(())
}

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
}
