//! Willow Replay Node — fast bounded-memory state sync worker.

pub mod role;

use clap::Parser;
use role::{ReplayConfig, ReplayRole};

#[derive(Parser)]
#[command(name = "willow-replay", about = "Willow replay worker node")]
struct Cli {
    /// Path to the Ed25519 identity keypair file.
    #[arg(long, default_value = "/etc/willow/replay.key")]
    identity_path: String,

    /// Iroh relay URL to connect through.
    #[arg(long)]
    relay_url: Option<String>,

    /// Max events per author chain to buffer in memory.
    #[arg(long, default_value = "1000")]
    max_events_per_author: usize,

    /// Active sync interval in seconds.
    #[arg(long, default_value = "30")]
    sync_interval: u64,

    /// Generate a new identity and exit.
    #[arg(long)]
    generate_identity: bool,

    /// Print the peer ID for the identity file and exit.
    #[arg(long)]
    print_peer_id: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    if cli.generate_identity {
        willow_worker::identity::generate_identity(&cli.identity_path)?;
        tracing::info!("identity generated at {}", cli.identity_path);
        return Ok(());
    }

    if cli.print_peer_id {
        return willow_worker::identity::print_peer_id(&cli.identity_path);
    }

    let identity = willow_worker::identity::load_or_generate(&cli.identity_path)?;

    let relay_url = cli
        .relay_url
        .as_deref()
        .map(|url| {
            url.parse::<willow_network::iroh::RelayUrl>()
                .map_err(|e| anyhow::anyhow!("invalid relay URL '{url}': {e}"))
        })
        .transpose()?;

    tracing::info!(
        max_events = cli.max_events_per_author,
        sync_interval = cli.sync_interval,
        relay_url = ?relay_url,
        "starting replay node"
    );

    let iroh_config = willow_network::iroh::Config {
        secret_key: identity.secret_key().clone(),
        relay_url,
        bootstrap_peers: vec![],
        mdns: false,
    };
    let network = willow_network::iroh::IrohNetwork::new(iroh_config).await?;

    let role = ReplayRole::new(ReplayConfig {
        max_events_per_author: cli.max_events_per_author,
    });

    let config = willow_worker::WorkerConfig {
        identity_path: cli.identity_path,
        relay_url: cli.relay_url,
        sync_interval_secs: cli.sync_interval,
        allocation: willow_worker::AllocationStrategy::Global,
    };

    willow_worker::runtime::run(Box::new(role), config, network).await
}
