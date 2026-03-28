//! Willow Replay Node — fast bounded-memory state sync worker.

pub mod role;

use clap::Parser;

#[derive(Parser)]
#[command(name = "willow-replay", about = "Willow replay worker node")]
struct Cli {
    /// Path to the Ed25519 identity keypair file.
    #[arg(long, default_value = "/etc/willow/replay.key")]
    identity_path: String,

    /// Relay multiaddr to connect through.
    #[arg(long)]
    relay: Option<String>,

    /// Max events per server to buffer in memory.
    #[arg(long, default_value = "1000")]
    max_events_per_server: usize,

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

fn main() -> anyhow::Result<()> {
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

    tracing::info!(
        max_events = cli.max_events_per_server,
        sync_interval = cli.sync_interval,
        "starting replay node"
    );

    // Full runtime integration (network actor, etc.) will be wired
    // in Task 7. For now, the role and actors are independently testable.
    tracing::info!("replay node ready (runtime not yet wired)");
    Ok(())
}
