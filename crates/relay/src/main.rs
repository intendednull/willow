//! # Willow Relay Server
//!
//! A lightweight relay node that bridges native (TCP) and browser (WebSocket)
//! peers. Deploy one on a VPS so browser clients can join the network.
//!
//! ## What it does
//!
//! - Listens on **TCP** (for native peers) and **WebSocket** (for browser peers)
//! - Runs the libp2p **relay** protocol so NAT'd peers can connect through it
//! - Participates in **GossipSub** to forward messages between peers
//! - Runs **Kademlia** for peer discovery
//! - Runs **Identify** for peer metadata exchange
//!
//! ## Usage
//!
//! ```bash
//! # Listen on default ports (TCP 9090, WebSocket 9091)
//! willow-relay
//!
//! # Custom ports
//! willow-relay --tcp-port 4001 --ws-port 4002
//!
//! # Persist identity across restarts
//! willow-relay --identity relay.key
//! ```

use anyhow::{Context, Result};
use clap::Parser;
use libp2p::{futures::StreamExt, identity::Keypair, Multiaddr};
use tracing::info;

use willow_relay::Relay;

#[derive(Parser)]
#[command(name = "willow-relay", about = "Willow P2P relay server")]
struct Args {
    /// TCP listen port for native peers.
    #[arg(long, default_value = "9090")]
    tcp_port: u16,

    /// WebSocket listen port for browser peers.
    #[arg(long, default_value = "9091")]
    ws_port: u16,

    /// Path to persist the relay's Ed25519 identity key.
    /// If not set, a new identity is generated each run.
    #[arg(long)]
    identity: Option<std::path::PathBuf>,

    /// Directory for the event store database.
    /// Defaults to ~/.local/share/willow-relay/
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();

    let keypair = load_or_generate_keypair(args.identity.as_deref())?;

    let data_dir = args.data_dir.unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("willow-relay")
    });
    let db_path = data_dir.join("events.db");

    let mut relay = Relay::start(keypair, &db_path).await?;

    info!(%relay.peer_id, "starting willow relay");

    // Listen on TCP.
    let tcp_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", args.tcp_port)
        .parse()
        .context("invalid TCP address")?;
    relay.swarm.listen_on(tcp_addr)?;

    // Listen on WebSocket (TCP + /ws upgrade).
    let ws_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}/ws", args.ws_port)
        .parse()
        .context("invalid WebSocket address")?;
    relay.swarm.listen_on(ws_addr)?;

    info!(
        tcp_port = args.tcp_port,
        ws_port = args.ws_port,
        events = relay.event_store.count(),
        "relay listening"
    );

    // Run the swarm event loop.
    loop {
        let event = relay.swarm.select_next_some().await;
        relay.handle_swarm_event(event);
    }
}

/// Load an Ed25519 keypair from disk, or generate a fresh one.
fn load_or_generate_keypair(path: Option<&std::path::Path>) -> Result<Keypair> {
    use libp2p::identity::ed25519;

    if let Some(path) = path {
        if let Ok(mut bytes) = std::fs::read(path) {
            let ed_kp =
                ed25519::Keypair::try_from_bytes(&mut bytes).context("invalid identity file")?;
            info!(?path, "loaded identity from disk");
            return Ok(Keypair::from(ed_kp));
        }

        // Generate and save.
        let kp = Keypair::generate_ed25519();
        let ed_kp = kp
            .clone()
            .try_into_ed25519()
            .context("keypair conversion")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, ed_kp.to_bytes())?;
        info!(?path, "generated and saved new identity");
        Ok(kp)
    } else {
        let kp = Keypair::generate_ed25519();
        info!("generated ephemeral identity (no --identity flag)");
        Ok(kp)
    }
}
