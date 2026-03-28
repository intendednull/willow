//! Willow Storage Node — archival disk-backed history worker.

pub mod role;
pub mod store;

use clap::Parser;
use role::StorageRole;
use store::StorageEventStore;

#[derive(Parser)]
#[command(name = "willow-storage", about = "Willow storage worker node")]
struct Cli {
    /// Path to the Ed25519 identity keypair file.
    #[arg(long, default_value = "/etc/willow/storage.key")]
    identity_path: String,

    /// Relay multiaddr to connect through.
    #[arg(long)]
    relay: Option<String>,

    /// Path to SQLite database.
    #[arg(long, default_value = "/var/lib/willow/storage.db")]
    db_path: String,

    /// Active sync interval in seconds.
    #[arg(long, default_value = "60")]
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
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
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

    let relay = cli
        .relay
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--relay is required"))?;

    tracing::info!(
        db_path = %cli.db_path,
        sync_interval = cli.sync_interval,
        %relay,
        "starting storage node"
    );

    let store = StorageEventStore::open(&cli.db_path)?;
    let role = StorageRole::new(store);

    let config = willow_worker::WorkerConfig {
        identity_path: cli.identity_path,
        relay_addr: relay.to_string(),
        sync_interval_secs: cli.sync_interval,
        allocation: willow_worker::AllocationStrategy::Global,
    };

    willow_worker::runtime::run(Box::new(role), config).await
}
