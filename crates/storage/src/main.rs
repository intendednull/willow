//! Willow Storage Node — archival disk-backed history worker.

pub mod role;
pub mod store;

use std::path::PathBuf;

use clap::Parser;
use role::StorageRole;
use store::StorageEventStore;

/// Compute the platform-aware default path for the storage identity key.
///
/// Prefers the user's config dir (e.g. `$XDG_CONFIG_HOME/willow/storage.key`
/// on Linux), falling back to `/etc/willow/storage.key` when no config dir
/// is available (matches the historical Linux deployment path).
fn default_storage_key() -> PathBuf {
    dirs::config_dir()
        .map(|d| d.join("willow").join("storage.key"))
        .unwrap_or_else(|| PathBuf::from("/etc/willow/storage.key"))
}

/// Compute the platform-aware default path for the storage SQLite database.
///
/// Prefers the user's data dir (e.g. `$XDG_DATA_HOME/willow/storage.db` on
/// Linux), falling back to `/var/lib/willow/storage.db` when no data dir is
/// available (matches the historical Linux deployment path).
fn default_storage_db() -> PathBuf {
    dirs::data_dir()
        .map(|d| d.join("willow").join("storage.db"))
        .unwrap_or_else(|| PathBuf::from("/var/lib/willow/storage.db"))
}

#[derive(Parser)]
#[command(name = "willow-storage", about = "Willow storage worker node")]
struct Cli {
    /// Path to the Ed25519 identity keypair file.
    #[arg(long, default_value_t = default_storage_key().display().to_string())]
    identity_path: String,

    /// Iroh relay URL to connect through.
    #[arg(long)]
    relay_url: Option<String>,

    /// Path to SQLite database.
    #[arg(long, default_value_t = default_storage_db().display().to_string())]
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

    let relay_url = cli.relay_url.as_deref().map(|url| {
        url.parse::<willow_network::iroh::RelayUrl>()
            .expect("invalid relay URL")
    });

    tracing::info!(
        db_path = %cli.db_path,
        sync_interval = cli.sync_interval,
        relay_url = ?relay_url,
        "starting storage node"
    );

    let iroh_config = willow_network::iroh::Config {
        secret_key: identity.secret_key().clone(),
        relay_url,
        bootstrap_peers: vec![],
        mdns: false,
    };
    let network = willow_network::iroh::IrohNetwork::new(iroh_config).await?;

    let store = StorageEventStore::open(&cli.db_path)?;
    let role = StorageRole::new(store);

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
    fn default_storage_key_targets_willow_subdir() {
        let p = default_storage_key();
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("storage.key"));
        let parent = p.parent().expect("default path has parent");
        assert_eq!(
            parent.file_name().and_then(|s| s.to_str()),
            Some("willow"),
            "expected willow/ as parent dir, got {parent:?}"
        );
        assert!(p.is_absolute(), "default path must be absolute, got {p:?}");
        assert!(
            p.ends_with(Path::new("willow/storage.key")),
            "expected path to end with willow/storage.key, got {p:?}"
        );
    }

    #[test]
    fn default_storage_db_targets_willow_subdir() {
        let p = default_storage_db();
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("storage.db"));
        let parent = p.parent().expect("default path has parent");
        assert_eq!(
            parent.file_name().and_then(|s| s.to_str()),
            Some("willow"),
            "expected willow/ as parent dir, got {parent:?}"
        );
        assert!(p.is_absolute(), "default path must be absolute, got {p:?}");
        assert!(
            p.ends_with(Path::new("willow/storage.db")),
            "expected path to end with willow/storage.db, got {p:?}"
        );
    }
}
