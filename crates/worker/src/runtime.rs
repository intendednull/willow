//! Worker runtime — orchestrates all four actors using the Network trait.

use std::time::Duration;

use tokio::sync::mpsc;
use tracing::info;
use willow_network::Network;

use crate::actors::{heartbeat, network, state, sync, StateMsg};
use crate::config::WorkerConfig;
use crate::WorkerRole;

/// Run a worker node with the given role, configuration, and network.
///
/// This is the main entry point called by each binary's `main()`.
/// It subscribes to system gossip topics, spawns all four actors
/// (state, network, heartbeat, sync), and blocks until Ctrl+C.
pub async fn run<N: Network>(
    role: Box<dyn WorkerRole>,
    config: WorkerConfig,
    network: N,
) -> anyhow::Result<()> {
    let identity = crate::identity::load_or_generate(&config.identity_path)?;
    let peer_id = identity.endpoint_id();
    info!(%peer_id, "worker identity loaded");

    // Subscribe to system topics.
    let workers_topic_id = willow_network::topic_id(crate::types::WORKERS_TOPIC);
    let (workers_sender, workers_events) = network.subscribe(workers_topic_id, vec![]).await?;

    let _ops_topic_id = willow_network::topic_id(crate::types::SERVER_OPS_TOPIC);
    let (_ops_sender, _ops_events) = network.subscribe(_ops_topic_id, vec![]).await?;

    // Create channels.
    let (state_tx, state_rx) = mpsc::channel::<StateMsg>(256);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Spawn actors.
    let state_handle = tokio::spawn(state::run(role, state_rx));

    let network_handle = tokio::spawn(network::run(workers_events, state_tx.clone(), peer_id));

    let heartbeat_handle = tokio::spawn(heartbeat::run(
        peer_id,
        Duration::from_secs(10),
        state_tx.clone(),
        workers_sender.clone(),
        shutdown_rx.clone(),
    ));

    let sync_handle = tokio::spawn(sync::run(
        peer_id,
        Duration::from_secs(config.sync_interval_secs),
        state_tx.clone(),
        workers_sender,
        shutdown_rx,
    ));

    // Wait for shutdown.
    tokio::signal::ctrl_c().await?;
    info!("shutdown signal received");
    let _ = shutdown_tx.send(true);
    let _ = state_tx.send(StateMsg::Shutdown).await;
    let _ = tokio::join!(state_handle, network_handle, heartbeat_handle, sync_handle);

    network.shutdown().await?;
    info!("worker shut down cleanly");
    Ok(())
}
