//! # Willow Agent
//!
//! MCP server binary that exposes `ClientHandle` as tools, resources, and
//! notifications to AI agents, bots, and scripts. The agent binary is a
//! first-class Willow peer with its own Ed25519 identity.

use clap::Parser;
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

    /// Generate a new identity and exit.
    #[arg(long)]
    generate_identity: bool,

    /// Print the peer ID for the identity file and exit.
    #[arg(long)]
    print_peer_id: bool,
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
        let identity = Identity::generate();
        std::fs::create_dir_all(
            std::path::Path::new(&identity_path)
                .parent()
                .unwrap_or(std::path::Path::new(".")),
        )?;
        std::fs::write(&identity_path, identity.to_bytes())?;
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
            server::serve_http(client, &cli.bind, Default::default(), token).await?;
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
fn load_or_generate_identity(path: &str) -> anyhow::Result<Identity> {
    let p = std::path::Path::new(path);
    if p.exists() {
        let bytes = std::fs::read(p)?;
        Identity::from_bytes(&bytes).ok_or_else(|| anyhow::anyhow!("invalid identity file"))
    } else {
        let identity = Identity::generate();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, identity.to_bytes())?;
        tracing::info!("generated new identity at {}", path);
        Ok(identity)
    }
}
