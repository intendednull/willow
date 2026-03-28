//! Willow Storage Node — archival disk-backed history worker.

mod role;
mod store;

use clap::Parser;

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
        db_path = %cli.db_path,
        sync_interval = cli.sync_interval,
        "starting storage node"
    );

    // Full runtime integration will be wired in Task 7.
    tracing::info!("storage node ready (runtime not yet wired)");
    Ok(())
}
