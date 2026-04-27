//! # Willow Agent
//!
//! MCP server binary that exposes `ClientHandle` as tools, resources, and
//! notifications to AI agents, bots, and scripts. The agent binary is a
//! first-class Willow peer with its own Ed25519 identity.

use clap::{Parser, ValueEnum};
use willow_agent::scopes::TokenScope;
use willow_agent::{auth, server};
use willow_client::{ClientConfig, ClientHandle};
use willow_identity::Identity;
use willow_network::iroh::{Config as IrohConfig, IrohNetwork, RelayUrl};

#[derive(Parser)]
#[command(name = "willow-agent", about = "Willow MCP agent peer")]
struct Cli {
    /// Iroh relay URL for NAT traversal.
    #[arg(long)]
    relay: Option<String>,

    /// Display name for the agent peer.
    #[arg(long, default_value = "Agent")]
    name: String,

    /// Server ID to switch to on startup.
    #[arg(long)]
    server: Option<String>,

    /// Invite code to accept on startup.
    #[arg(long)]
    invite: Option<String>,

    /// MCP transport: stdio | http [default: stdio].
    #[arg(long, default_value = "stdio")]
    transport: String,

    /// Bind address for SSE/HTTP transports.
    #[arg(long, default_value = "127.0.0.1:9100")]
    bind: String,

    /// Bearer token (auto-generated if omitted).
    #[arg(long)]
    token: Option<String>,

    /// Write bearer token to this file (0600 permissions).
    #[arg(long)]
    token_file: Option<String>,

    /// Path to Ed25519 identity file.
    #[arg(long)]
    identity: Option<String>,

    /// Whether to persist state to disk.
    #[arg(long)]
    persist: bool,

    /// Log level filter.
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Token scope for HTTP transport (least-privilege default).
    #[arg(long, value_enum, default_value_t = ScopeArg::Messaging)]
    scope: ScopeArg,

    /// Generate a new identity and exit.
    #[arg(long)]
    generate_identity: bool,

    /// Print the peer ID for the identity file and exit.
    #[arg(long)]
    print_peer_id: bool,
}

/// CLI-friendly enum for `--scope`. Maps to `TokenScope`.
///
/// Per-scope tool list:
/// - `messaging` (default): `send_message`, `send_reply`, `edit_message`,
///   `delete_message`, `react`, `pin_message`, `unpin_message`, `send_typing`.
///   All resources readable.
/// - `read`: no tools. All resources readable.
/// - `full`: every tool reachable from the bearer token, all resources.
/// - `admin`: every tool, all resources. Semantically distinct from `full`
///   so audit log consumers can tell intent apart.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ScopeArg {
    /// Messaging tools only (least-privilege default).
    Messaging,
    /// Read-only: no tools, all resources.
    Read,
    /// Full access: all tools, all resources.
    Full,
    /// Admin access: all tools, all resources (audit-distinct from `full`).
    Admin,
}

impl From<ScopeArg> for TokenScope {
    fn from(s: ScopeArg) -> Self {
        match s {
            ScopeArg::Messaging => TokenScope::Messaging,
            ScopeArg::Read => TokenScope::ReadOnly,
            ScopeArg::Full => TokenScope::Full,
            ScopeArg::Admin => TokenScope::Admin,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cli.log_level.clone().into()),
        )
        .with_writer(std::io::stderr)
        .init();

    // Identity management.
    let identity_path = cli.identity.clone().unwrap_or_else(default_identity_path);

    if cli.generate_identity {
        // Use the identity helper so the file is written atomically with
        // mode 0o600 (refusing to leave a world-readable secret on disk).
        let _identity = Identity::load_or_generate(&identity_path)?;
        tracing::info!("identity generated at {}", identity_path);
        return Ok(());
    }

    let identity = load_or_generate_identity(&identity_path)?;

    if cli.print_peer_id {
        println!("{}", identity.endpoint_id());
        return Ok(());
    }

    // Build client.
    let config = ClientConfig {
        relay_addr: cli.relay.clone(),
        display_name: Some(cli.name.clone()),
        persistence: cli.persist,
        bootstrap_peers: vec![],
    };

    let (mut client, _event_loop) = ClientHandle::<IrohNetwork>::new(config);

    // Connect to network if relay specified.
    if let Some(ref relay_url) = cli.relay {
        let relay: RelayUrl = relay_url
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid relay URL {relay_url:?}: {e}"))?;
        let iroh_config = IrohConfig {
            secret_key: identity.secret_key().clone(),
            relay_url: Some(relay),
            bootstrap_peers: vec![],
            mdns: false,
        };
        let network = IrohNetwork::new(iroh_config).await?;
        client.connect(network).await;
        tracing::info!(peer_id = %client.peer_id(), "connected to network");
    }

    // Accept invite if provided.
    if let Some(ref invite) = cli.invite {
        client.accept_invite(invite).await?;
        tracing::info!("accepted invite");
    }

    // Switch server if specified.
    if let Some(ref server_id) = cli.server {
        client.switch_server(server_id).await;
        tracing::info!(server = server_id, "switched to server");
    }

    // Set display name.
    client.set_display_name(&cli.name).await;

    // Generate or use provided bearer token.
    let token = auth::resolve_token(&cli.token, cli.token_file.as_deref())?;

    // Start MCP server.
    match cli.transport.as_str() {
        "stdio" => {
            tracing::info!("starting MCP server on stdio");
            server::serve_stdio(client).await?;
        }
        "http" => {
            tracing::info!("starting MCP HTTP server on {}", cli.bind);
            server::serve_http(client, &cli.bind, cli.scope.into(), token).await?;
        }
        other => {
            anyhow::bail!("unsupported transport: {other} (supported: 'stdio', 'http')");
        }
    }

    Ok(())
}

/// Default identity path: ~/.willow/agent-identity
fn default_identity_path() -> String {
    dirs::home_dir()
        .map(|h| h.join(".willow").join("agent-identity"))
        .unwrap_or_else(|| std::path::PathBuf::from(".willow/agent-identity"))
        .to_string_lossy()
        .into_owned()
}

/// Load identity from file, or generate a new one if the file doesn't exist.
///
/// Delegates to [`Identity::load_or_generate`] so that:
///
/// - newly generated key files are written atomically with mode `0o600`,
/// - pre-existing key files with insecure permissions (group/world readable)
///   are refused rather than silently used.
fn load_or_generate_identity(path: &str) -> anyhow::Result<Identity> {
    let p = std::path::Path::new(path);
    let existed = p.exists();
    let identity = Identity::load_or_generate(p)?;
    if !existed {
        tracing::info!("generated new identity at {}", path);
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    //! CLI parsing tests for `--scope`. Verifies that:
    //!
    //! - omitting `--scope` yields the least-privilege `Messaging` default
    //!   (closes the SEC-A-09 footgun where `serve_http` previously got
    //!   `TokenScope::default()` == `Full`),
    //! - every documented value (`messaging`, `read`, `full`, `admin`) parses,
    //! - each `ScopeArg` maps to the expected `TokenScope` variant.
    //!
    //! Regression test for https://github.com/intendednull/willow/issues/311.
    use super::*;
    use clap::Parser;
    use willow_agent::scopes::TokenScope;
    // Required arg for clap to accept a successful parse.
    const MIN_ARGS: &[&str] = &["willow-agent"];

    fn parse(extra: &[&str]) -> Cli {
        let mut argv: Vec<&str> = MIN_ARGS.to_vec();
        argv.extend_from_slice(extra);
        Cli::parse_from(argv)
    }

    #[test]
    fn default_scope_is_messaging() {
        // SEC-A-09: when operator does not pass --scope, HTTP transport must
        // not silently fall back to full admin authority.
        let cli = parse(&[]);
        assert_eq!(cli.scope, ScopeArg::Messaging);
    }

    #[test]
    fn scope_messaging_parses() {
        let cli = parse(&["--scope", "messaging"]);
        assert_eq!(cli.scope, ScopeArg::Messaging);
    }

    #[test]
    fn scope_read_parses() {
        let cli = parse(&["--scope", "read"]);
        assert_eq!(cli.scope, ScopeArg::Read);
    }

    #[test]
    fn scope_full_parses() {
        let cli = parse(&["--scope", "full"]);
        assert_eq!(cli.scope, ScopeArg::Full);
    }

    #[test]
    fn scope_admin_parses() {
        let cli = parse(&["--scope", "admin"]);
        assert_eq!(cli.scope, ScopeArg::Admin);
    }

    #[test]
    fn unknown_scope_rejected() {
        let mut argv: Vec<&str> = MIN_ARGS.to_vec();
        argv.extend_from_slice(&["--scope", "root"]);
        assert!(Cli::try_parse_from(argv).is_err());
    }

    #[test]
    fn scope_arg_maps_to_token_scope() {
        // ScopeArg → TokenScope wiring. If this drifts, the wrong scope
        // ends up gating tool calls in serve_http.
        assert!(matches!(
            TokenScope::from(ScopeArg::Messaging),
            TokenScope::Messaging
        ));
        assert!(matches!(
            TokenScope::from(ScopeArg::Read),
            TokenScope::ReadOnly
        ));
        assert!(matches!(TokenScope::from(ScopeArg::Full), TokenScope::Full));
        assert!(matches!(
            TokenScope::from(ScopeArg::Admin),
            TokenScope::Admin
        ));
    }
}
