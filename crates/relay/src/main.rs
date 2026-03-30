//! # Willow Relay Server
//!
//! A lightweight, stateless relay node that bridges native (TCP) and browser
//! (WebSocket) peers. Deploy one on a VPS so browser clients can join.
//!
//! ## What it does
//!
//! - Listens on **TCP** (for native peers) and **WebSocket** (for browser peers)
//! - Runs the libp2p **relay** protocol so NAT'd peers can connect through it
//! - Participates in **GossipSub** to forward messages between peers
//! - Runs **Kademlia** for peer discovery
//! - Runs **Identify** for peer metadata exchange
//!
//! ## What it does NOT do
//!
//! - Store events (replay nodes handle state persistence)
//! - Respond to sync requests (replay nodes handle catch-up)
//! - Store message history (storage nodes handle archival)

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

    /// Display name for this relay node (visible in peer lists).
    #[arg(long, default_value = "Relay Node")]
    name: String,

    /// Print the peer ID (generating the identity if needed) and exit.
    #[arg(long)]
    print_peer_id: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Handle --print-peer-id before initializing tracing to get clean output.
    if args.print_peer_id {
        let keypair = load_or_generate_keypair(args.identity.as_deref())?;
        let peer_id = libp2p::PeerId::from(keypair.public());
        println!("{peer_id}");
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let keypair = load_or_generate_keypair(args.identity.as_deref())?;

    let mut relay = Relay::start(keypair).await?;

    relay.set_display_name(&args.name);
    info!(%relay.peer_id, name = %args.name, "starting willow relay");

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
        "relay listening (stateless — no event storage)"
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
