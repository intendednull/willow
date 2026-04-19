//! # Willow Relay Server
//!
//! Wraps an iroh-relay server for NAT traversal and runs a bootstrap
//! gossip node that subscribes to system topics so new peers can join
//! the gossip mesh. Dynamically subscribes to channel topics announced
//! by peers via [`TopicAnnounce`](willow_common::WireMessage::TopicAnnounce).

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iroh_base::RelayUrl;
use iroh_relay::server::{AccessConfig, RelayConfig, Server, ServerConfig};
use tokio::sync::Semaphore;
use tracing::info;
use willow_identity::Identity;
use willow_network::Network;
use willow_relay::{
    run_proxy_listener, topic_announce_listener, MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS,
};

#[derive(Parser)]
#[command(name = "willow-relay", about = "Willow iroh relay + bootstrap node")]
struct Args {
    /// Port for the iroh relay HTTP server.
    #[arg(long, default_value = "3340")]
    relay_port: u16,

    /// Path to persist the bootstrap node's identity key.
    #[arg(long)]
    identity: Option<std::path::PathBuf>,

    /// Print the bootstrap node's EndpointId and exit.
    #[arg(long)]
    print_id: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Load or generate bootstrap identity.
    let identity = if let Some(ref path) = args.identity {
        Identity::load_or_generate(path)?
    } else {
        Identity::generate()
    };

    if args.print_id {
        println!("{}", identity.endpoint_id());
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    info!(id = %identity.endpoint_id().fmt_short(), "bootstrap node identity");

    // ── Public listener (HTTP dispatch: /bootstrap-id + proxy) ───────────
    // Bind the public-facing TCP port FIRST so `relay_port = 0` can pick
    // an ephemeral port that we can then advertise. The iroh-relay is
    // bound to a loopback ephemeral port below, and the proxy below
    // forwards non-bootstrap traffic to it.
    let public_bind: SocketAddr = (Ipv4Addr::UNSPECIFIED, args.relay_port).into();
    let public_listener = tokio::net::TcpListener::bind(public_bind)
        .await
        .context("failed to bind public relay HTTP port")?;
    let public_port = public_listener.local_addr()?.port();
    info!(port = public_port, "public relay HTTP listener ready");

    // ── iroh-relay server (loopback only, plain HTTP, no TLS) ────────────
    // The iroh-relay is only reachable through the proxy above; binding
    // to 127.0.0.1 prevents direct access from the network.
    let relay_bind: SocketAddr = (Ipv4Addr::LOCALHOST, 0).into();

    let mut relay_server = Server::spawn(ServerConfig::<(), ()> {
        relay: Some(RelayConfig::<(), ()> {
            http_bind_addr: relay_bind,
            tls: None,
            limits: Default::default(),
            key_cache_capacity: Some(1024),
            access: AccessConfig::Everyone,
        }),
        quic: None,
        metrics_addr: None,
    })
    .await
    .context("failed to start iroh-relay server")?;

    let upstream_relay_addr = relay_server
        .http_addr()
        .context("relay server has no HTTP address")?;
    // Advertise the PUBLIC port to bootstrap node and clients, not the
    // loopback upstream port.
    let advertised_addr: SocketAddr = (Ipv4Addr::LOCALHOST, public_port).into();
    let relay_url: RelayUrl = format!("http://{advertised_addr}")
        .parse()
        .context("failed to parse relay URL")?;
    info!(
        %relay_url,
        upstream = %upstream_relay_addr,
        "iroh-relay server running"
    );

    // ── Bootstrap gossip node ────────────────────────────────────────────
    let config = willow_network::iroh::Config {
        secret_key: identity.secret_key().clone(),
        relay_url: Some(relay_url),
        bootstrap_peers: vec![],
        mdns: true,
    };
    let network = willow_network::iroh::IrohNetwork::new(config)
        .await
        .context("failed to start bootstrap network")?;

    // Subscribe to system topics so new peers can bootstrap into the mesh.
    let server_ops_topic = willow_network::topic_id("_willow_server_ops");
    let workers_topic = willow_network::topic_id("_willow_workers");
    let profiles_topic = willow_network::topic_id("_willow_profiles");

    // Keep the server_ops events stream to listen for TopicAnnounce messages.
    let (_ops_sender, ops_events) = network.subscribe(server_ops_topic, vec![]).await?;
    let _ = network.subscribe(workers_topic, vec![]).await?;
    let _ = network.subscribe(profiles_topic, vec![]).await?;

    info!("bootstrap node subscribed to system topics");

    // ── Public dispatch loop ────────────────────────────────────────────
    // The public listener multiplexes two concerns on ONE port:
    //   * `GET /bootstrap-id` → serves the bootstrap node's endpoint ID
    //   * everything else     → proxied to the loopback iroh-relay
    //
    // The accept loop applies per-connection I/O timeouts and is gated
    // by a semaphore so a connection-flood DoS cannot exhaust file
    // descriptors. See `willow_relay::run_proxy_listener`.
    let bootstrap_id = Arc::new(identity.endpoint_id().to_string());
    let proxy_semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_BOOTSTRAP_CONNECTIONS));
    tokio::spawn(run_proxy_listener(
        public_listener,
        upstream_relay_addr,
        Arc::clone(&bootstrap_id),
        proxy_semaphore,
    ));
    info!(
        port = public_port,
        "serving /bootstrap-id + iroh-relay proxy"
    );

    // Spawn a task that listens for TopicAnnounce messages on _willow_server_ops
    // and dynamically subscribes to announced channel topics.
    tokio::spawn(topic_announce_listener(ops_events, network));

    info!("relay running — press Ctrl+C to stop");

    // Wait for shutdown signal or relay task failure.
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("received Ctrl+C, shutting down");
        }
        result = relay_server.task_handle() => {
            match result {
                Ok(Ok(())) => info!("relay server exited cleanly"),
                Ok(Err(e)) => tracing::error!(%e, "relay server error"),
                Err(e) => tracing::error!(%e, "relay server task panicked"),
            }
        }
    }

    // Network is moved into the listener task, so we just shut down the relay.
    relay_server.shutdown().await.ok();
    info!("shut down complete");
    Ok(())
}
