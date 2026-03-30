//! Worker runtime — orchestrates all four actors.
//!
//! TODO: Phase 3 will rewrite this to use the Network trait from willow-network.

use tracing::warn;

use crate::config::WorkerConfig;
use crate::WorkerRole;

/// Run a worker node with the given role and configuration.
///
/// This is the main entry point called by each binary's `main()`.
/// It starts all four actors and blocks until shutdown (Ctrl+C).
///
/// **NOTE**: This is currently a stub. The old libp2p-based runtime has been
/// removed. Phase 3 of the iroh migration will rewrite this to use the
/// `Network` trait from `willow-network`.
pub async fn run(_role: Box<dyn WorkerRole>, _config: WorkerConfig) -> anyhow::Result<()> {
    warn!("worker runtime is not yet implemented for iroh — Phase 3 migration pending");
    // Wait for shutdown signal so the binary doesn't exit immediately.
    tokio::signal::ctrl_c().await?;
    Ok(())
}
