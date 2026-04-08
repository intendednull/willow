//! Worker runtime — orchestrates all four actors using `willow-actor`.

use std::time::Duration;

use tracing::info;
use willow_actor::System;
use willow_network::Network;

use crate::actors::{
    heartbeat::HeartbeatActor, network::NetworkActor, state::StateActor, sync::SyncActor,
};
use crate::config::WorkerConfig;
use crate::WorkerRole;

/// Run a worker node with the given role, configuration, and network.
///
/// This is the main entry point called by each binary's `main()`.
/// It subscribes to system gossip topics, spawns all four actors
/// via the actor system, and blocks until Ctrl+C.
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

    let ops_topic_id = willow_network::topic_id(crate::types::SERVER_OPS_TOPIC);
    let (_ops_sender, ops_events) = network.subscribe(ops_topic_id, vec![]).await?;

    // Create actor system and spawn actors.
    // The ready signal ensures NetworkActor waits for StateActor to
    // complete initialization before draining gossip events.
    let system = System::new();
    let (ready_tx, ready_rx) = tokio::sync::watch::channel(false);

    let state_addr = system.spawn(StateActor {
        role,
        ready: Some(ready_tx),
    });

    let _network = system.spawn(
        NetworkActor::new(
            workers_events,
            state_addr.clone(),
            peer_id,
            workers_sender.clone(),
        )
        .with_ops_events(ops_events)
        .with_ready_signal(ready_rx),
    );

    let _heartbeat = system.spawn(HeartbeatActor::new(
        peer_id,
        Duration::from_secs(10),
        state_addr.clone(),
        workers_sender.clone(),
    ));

    let _sync = system.spawn(SyncActor::new(
        peer_id,
        Duration::from_secs(config.sync_interval_secs),
        state_addr,
        workers_sender,
    ));

    // Wait for shutdown.
    tokio::signal::ctrl_c().await?;
    info!("shutdown signal received");
    system.shutdown().await;

    network.shutdown().await?;
    info!("worker shut down cleanly");
    Ok(())
}
