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

use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use libp2p::{
    futures::StreamExt, gossipsub, identify, kad, noise, relay, swarm::SwarmEvent, tcp, yamux,
    Multiaddr, PeerId, SwarmBuilder,
};
use tracing::{debug, info, warn};

mod event_store;
use event_store::RelayEventStore;

/// Wire message format — mirrors willow_client::WireMessage but defined
/// locally to avoid pulling in the full client dependency.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum WireMessage {
    Event(willow_state::Event),
    SyncRequest {
        state_hash: willow_state::StateHash,
        topic: Option<String>,
    },
    SyncBatch {
        events: Vec<willow_state::Event>,
    },
}

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
    let local_peer_id = PeerId::from(keypair.public());

    info!(%local_peer_id, "starting willow relay");

    // Build the swarm with TCP + WebSocket transports.
    let mut swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_websocket(noise::Config::new, yamux::Config::default)
        .await?
        .with_behaviour(|key| {
            // GossipSub
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(1))
                .validation_mode(gossipsub::ValidationMode::Strict)
                .message_id_fn(|msg: &gossipsub::Message| {
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    msg.data.hash(&mut hasher);
                    msg.topic.hash(&mut hasher);
                    gossipsub::MessageId::from(hasher.finish().to_string())
                })
                .build()
                .expect("valid gossipsub config");

            let gossipsub = gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(key.clone()),
                gossipsub_config,
            )
            .expect("valid gossipsub behaviour");

            // Kademlia
            let kademlia =
                kad::Behaviour::new(local_peer_id, kad::store::MemoryStore::new(local_peer_id));

            // Identify
            let identify = identify::Behaviour::new(identify::Config::new(
                "/willow/1.0.0".to_string(),
                key.public(),
            ));

            // Relay server (not client — this node IS the relay)
            let relay = relay::Behaviour::new(local_peer_id, relay::Config::default());

            Ok(RelayBehaviour {
                gossipsub,
                kademlia,
                identify,
                relay,
            })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(300)))
        .build();

    // Listen on TCP.
    let tcp_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", args.tcp_port)
        .parse()
        .context("invalid TCP address")?;
    swarm.listen_on(tcp_addr)?;

    // Listen on WebSocket (TCP + /ws upgrade).
    let ws_addr: Multiaddr = format!("/ip4/0.0.0.0/tcp/{}/ws", args.ws_port)
        .parse()
        .context("invalid WebSocket address")?;
    swarm.listen_on(ws_addr)?;

    info!(
        tcp_port = args.tcp_port,
        ws_port = args.ws_port,
        "relay listening"
    );

    // Open the event store for history persistence.
    let data_dir = args
        .data_dir
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("willow-relay")
        });
    std::fs::create_dir_all(&data_dir).ok();
    let mut event_store =
        RelayEventStore::open(&data_dir.join("events.db")).expect("failed to open event store");
    info!(path = ?data_dir.join("events.db"), events = event_store.count(), "event store ready");

    // Run the swarm event loop.
    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!(%address, "listening on");
                println!("Relay PeerId: {local_peer_id}");
                println!("Listening on: {address}");
            }

            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                info!(%peer_id, ?endpoint, "peer connected");
            }

            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                debug!(%peer_id, ?cause, "peer disconnected");
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
            })) => {
                debug!(%peer_id, protocol = %info.protocol_version, "identify received");
                for addr in info.listen_addrs {
                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                }
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Gossipsub(
                gossipsub::Event::Subscribed { peer_id, topic },
            )) => {
                info!(%peer_id, %topic, "peer subscribed");
                // Auto-subscribe to any topic a peer subscribes to,
                // so the relay can forward messages.
                let topic = gossipsub::IdentTopic::new(topic.to_string());
                if let Err(e) = swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                    warn!(%e, "failed to subscribe");
                }
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                message,
                ..
            })) => {
                let topic_str = message.topic.to_string();
                let data = &message.data;

                // Try to parse and store events as they pass through.
                // The relay is just another peer that remembers everything.
                if let Ok((envelope_bytes, _signer)) =
                    willow_identity::unpack::<Vec<u8>>(data)
                {
                    if let Ok((wire_msg, willow_transport::MessageType::Channel)) =
                        willow_transport::unpack_envelope::<WireMessage>(&envelope_bytes)
                    {
                        match wire_msg {
                            WireMessage::Event(ref event) => {
                                event_store.store_event(&topic_str, event, data);
                                debug!(
                                    id = %event.id,
                                    author = %event.author,
                                    topic = %topic_str,
                                    "stored event"
                                );
                            }
                            WireMessage::SyncBatch { ref events } => {
                                for event in events {
                                    event_store.store_event(&topic_str, event, &[]);
                                }
                                debug!(count = events.len(), "cached sync batch");
                            }
                            _ => {}
                        }
                    }
                }

                debug!(
                    topic = %topic_str,
                    source = ?message.source,
                    bytes = data.len(),
                    stored = event_store.count(),
                    "relaying message"
                );
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(event)) => {
                debug!(?event, "relay event");
            }

            _ => {}
        }
    }
}

/// Composite behaviour for the relay server.
#[derive(libp2p::swarm::NetworkBehaviour)]
struct RelayBehaviour {
    gossipsub: gossipsub::Behaviour,
    kademlia: kad::Behaviour<kad::store::MemoryStore>,
    identify: identify::Behaviour,
    relay: relay::Behaviour,
}

/// Load an Ed25519 keypair from disk, or generate a fresh one.
fn load_or_generate_keypair(path: Option<&std::path::Path>) -> Result<libp2p::identity::Keypair> {
    use libp2p::identity::{ed25519, Keypair};

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
