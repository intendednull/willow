//! Willow Replay Node — fast bounded-memory state sync worker.

pub mod role;

use std::path::PathBuf;

use clap::Parser;
use role::{ReplayConfig, ReplayRole};

/// Compute the platform-aware default path for the replay identity key.
///
/// Prefers the user's config dir (e.g. `$XDG_CONFIG_HOME/willow/replay.key`
/// on Linux, `~/Library/Application Support/willow/replay.key` on macOS),
/// falling back to `/etc/willow/replay.key` when no config dir is available
/// (matches the historical Linux deployment path used by the Docker images).
fn default_replay_key() -> PathBuf {
    dirs::config_dir()
        .map(|d| d.join("willow").join("replay.key"))
        .unwrap_or_else(|| PathBuf::from("/etc/willow/replay.key"))
}

#[derive(Parser)]
#[command(name = "willow-replay", about = "Willow replay worker node")]
struct Cli {
    /// Path to the Ed25519 identity keypair file.
    #[arg(long, default_value_t = default_replay_key().display().to_string())]
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
        ..Default::default()
    });

    let config = willow_worker::WorkerConfig {
        identity_path: cli.identity_path,
        relay_url: cli.relay_url,
        sync_interval_secs: cli.sync_interval,
        allocation: willow_worker::AllocationStrategy::Global,
    };

    willow_worker::runtime::run(Box::new(role), config, network).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn default_replay_key_targets_willow_subdir() {
        let p = default_replay_key();
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("replay.key"));
        let parent = p.parent().expect("default path has parent");
        assert_eq!(
            parent.file_name().and_then(|s| s.to_str()),
            Some("willow"),
            "expected willow/ as parent dir, got {parent:?}"
        );
        assert!(p.is_absolute(), "default path must be absolute, got {p:?}");
        // Sanity: path contains at least the `willow/replay.key` tail.
        assert!(
            p.ends_with(Path::new("willow/replay.key")),
            "expected path to end with willow/replay.key, got {p:?}"
        );
    }
}
