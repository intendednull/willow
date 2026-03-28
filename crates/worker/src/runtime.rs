//! Worker runtime — orchestrates all four actors.

use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tracing::info;
use willow_network::{NetworkConfig, NetworkNode};

use crate::actors::{heartbeat, network, state, sync, NetworkOutMsg, StateMsg};
use crate::config::WorkerConfig;
use crate::WorkerRole;

/// Run a worker node with the given role and configuration.
///
/// This is the main entry point called by each binary's `main()`.
/// It starts all four actors and blocks until shutdown (Ctrl+C).
pub async fn run(role: Box<dyn WorkerRole>, config: WorkerConfig) -> anyhow::Result<()> {
    // Load identity.
    let identity = crate::identity::load_or_generate(&config.identity_path)?;
    let peer_id = identity.peer_id().to_string();
    info!(%peer_id, "worker identity loaded");

    // Build network config.
    let net_config = NetworkConfig::default().with_relay(&config.relay_addr)?;

    // Start the network node.
    let (node, events) = NetworkNode::start(identity, net_config).await?;
    info!("network node started");

    // Create channels.
    let (state_tx, state_rx) = mpsc::channel::<StateMsg>(256);
    let (network_tx, network_rx) = mpsc::channel::<NetworkOutMsg>(256);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn actors.
    let state_handle = tokio::spawn(state::run(role, state_rx));

    let network_handle = tokio::spawn(network::run(
        node,
        events,
        state_tx.clone(),
        network_rx,
        peer_id.clone(),
    ));

    let heartbeat_handle = tokio::spawn(heartbeat::run(
        peer_id.clone(),
        Duration::from_secs(10),
        state_tx.clone(),
        network_tx.clone(),
        shutdown_rx.clone(),
    ));

    let sync_handle = tokio::spawn(sync::run(
        peer_id,
        Duration::from_secs(config.sync_interval_secs),
        state_tx.clone(),
        network_tx,
        shutdown_rx,
    ));

    // Wait for shutdown signal (Ctrl+C).
    tokio::signal::ctrl_c().await?;
    info!("shutdown signal received");

    // Signal all actors to stop.
    let _ = shutdown_tx.send(true);
    let _ = state_tx.send(StateMsg::Shutdown).await;

    // Wait for actors to finish.
    let _ = tokio::join!(state_handle, network_handle, heartbeat_handle, sync_handle);

    info!("worker shut down cleanly");
    Ok(())
}
