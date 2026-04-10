//! # Willow Relay Server
//!
//! Wraps an iroh-relay server for NAT traversal and runs a bootstrap
//! gossip node that subscribes to system topics so new peers can join
//! the gossip mesh. Dynamically subscribes to channel topics announced
//! by peers via [`TopicAnnounce`](willow_common::WireMessage::TopicAnnounce).

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use iroh_base::RelayUrl;
use iroh_relay::server::{AccessConfig, RelayConfig, Server, ServerConfig};
use tracing::info;
use willow_identity::Identity;
use willow_network::traits::{GossipEvent, TopicEvents};
use willow_network::Network;

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

    // ── iroh-relay server (plain HTTP, no TLS) ───────────────────────────
    let relay_bind: SocketAddr = (Ipv4Addr::UNSPECIFIED, args.relay_port).into();

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

    let relay_addr = relay_server
        .http_addr()
        .context("relay server has no HTTP address")?;
    // The bootstrap node must connect to the relay via a loopback address
    // (127.0.0.1), not the bind address (0.0.0.0). 0.0.0.0 is a valid bind
    // address but not a valid destination to connect to.
    let connect_addr: SocketAddr = if relay_addr.ip().is_unspecified() {
        (std::net::Ipv4Addr::LOCALHOST, relay_addr.port()).into()
    } else {
        relay_addr
    };
    let relay_url: RelayUrl = format!("http://{connect_addr}")
        .parse()
        .context("failed to parse relay URL")?;
    info!(%relay_url, "iroh-relay server running");

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

    // ── Bootstrap ID HTTP endpoint ──────────────────────────────────────
    // Serve the bootstrap node's endpoint ID so web clients can fetch it.
    let bootstrap_id = Arc::new(identity.endpoint_id().to_string());
    let id_for_handler = Arc::clone(&bootstrap_id);
    let bootstrap_listener =
        tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, args.relay_port + 1))
            .await
            .context("failed to bind bootstrap-id HTTP port")?;
    let bootstrap_port = bootstrap_listener.local_addr()?.port();
    info!(port = bootstrap_port, "bootstrap-id endpoint listening");
    tokio::spawn(async move {
        loop {
            if let Ok((stream, _)) = bootstrap_listener.accept().await {
                let id = Arc::clone(&id_for_handler);
                tokio::spawn(async move {
                    let (mut reader, mut writer) = stream.into_split();
                    // Read the request (we don't care about its contents).
                    let mut buf = [0u8; 1024];
                    let _ = tokio::io::AsyncReadExt::read(&mut reader, &mut buf).await;
                    let body = id.as_str();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ =
                        tokio::io::AsyncWriteExt::write_all(&mut writer, response.as_bytes()).await;
                });
            }
        }
    });

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

/// Listen for [`TopicAnnounce`] messages on `_willow_server_ops` and
/// dynamically subscribe to announced channel topics.
async fn topic_announce_listener(
    mut events: <willow_network::iroh::IrohNetwork as Network>::Events,
    network: willow_network::iroh::IrohNetwork,
) {
    let mut subscribed: HashSet<String> = HashSet::new();

    while let Some(Ok(event)) = events.next().await {
        if let GossipEvent::Received(msg) = event {
            if let Some((willow_common::WireMessage::TopicAnnounce { topics }, _)) =
                willow_common::unpack_wire(&msg.content)
            {
                for topic_str in topics {
                    if subscribed.insert(topic_str.clone()) {
                        let topic_id = willow_network::topic_id(&topic_str);
                        match network.subscribe(topic_id, vec![]).await {
                            Ok(_) => {
                                info!(topic = %topic_str, "subscribed to announced channel topic");
                            }
                            Err(e) => {
                                tracing::warn!(
                                    topic = %topic_str, %e,
                                    "failed to subscribe to announced topic"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
