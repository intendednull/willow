//! The high-level network node that manages the libp2p swarm.
//!
//! [`NetworkNode`] owns the swarm and runs it on a background tokio task.
//! Callers interact with it through a command-style API (subscribe, publish,
//! dial) and receive events via a [`tokio::sync::mpsc`] channel.

use anyhow::{Context, Result};
use libp2p::{
    gossipsub, identify, kad, mdns, noise,
    swarm::SwarmEvent,
    tcp, yamux, Multiaddr, PeerId, Swarm, SwarmBuilder,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{NetworkConfig, WillowBehaviour};
use willow_identity::Identity;

// ───── Events ────────────────────────────────────────────────────────────────

/// Events emitted by the network layer to the application.
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    /// A message was received on a subscribed topic.
    Message {
        /// The topic (channel) the message was published to.
        topic: String,
        /// Raw message bytes.
        data: Vec<u8>,
        /// The peer that published the message (if known).
        source: Option<PeerId>,
    },

    /// A new peer connected to us.
    PeerConnected(PeerId),

    /// A peer disconnected.
    PeerDisconnected(PeerId),

    /// A new peer was discovered on the local network via mDNS.
    PeerDiscovered {
        peer_id: PeerId,
        addrs: Vec<Multiaddr>,
    },

    /// We started listening on an address.
    Listening(Multiaddr),
}

// ───── Commands ──────────────────────────────────────────────────────────────

/// Internal commands sent from the `NetworkNode` handle to the swarm task.
#[derive(Debug)]
enum Command {
    Subscribe(String),
    Unsubscribe(String),
    Publish { topic: String, data: Vec<u8> },
    Dial(Multiaddr),
}

// ───── NetworkNode ───────────────────────────────────────────────────────────

/// Handle to a running Willow network node.
///
/// This is the main entry point for the networking layer. Call
/// [`NetworkNode::start`] to launch the libp2p swarm on a background task,
/// then use the returned handle to subscribe to topics, publish messages, and
/// connect to peers.
///
/// Events from the network are delivered through the
/// [`mpsc::Receiver<NetworkEvent>`] returned alongside the handle.
pub struct NetworkNode {
    command_tx: mpsc::UnboundedSender<Command>,
    local_peer_id: PeerId,
}

impl NetworkNode {
    /// Start the network node and return a handle + event stream.
    ///
    /// This spawns a background tokio task that drives the libp2p swarm.
    ///
    /// # Arguments
    ///
    /// - `identity` — the local peer's cryptographic identity.
    /// - `config` — network configuration (listen address, bootstrap peers, etc.).
    ///
    /// # Returns
    ///
    /// A tuple of `(handle, event_receiver)`. Use the handle to send commands
    /// and the receiver to consume network events.
    pub async fn start(
        identity: Identity,
        config: NetworkConfig,
    ) -> Result<(Self, mpsc::UnboundedReceiver<NetworkEvent>)> {
        let keypair = identity.keypair().clone();
        let local_peer_id = PeerId::from(keypair.public());

        info!(%local_peer_id, "starting willow network node");

        let mut swarm = build_swarm(keypair.clone(), &config)?;

        swarm
            .listen_on(config.listen_addr.clone())
            .context("failed to listen")?;

        // Bootstrap Kademlia if we have known peers.
        for (peer, addr) in &config.bootstrap_peers {
            swarm.behaviour_mut().kademlia.add_address(peer, addr.clone());
        }
        if !config.bootstrap_peers.is_empty() {
            swarm
                .behaviour_mut()
                .kademlia
                .bootstrap()
                .ok();
        }

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        tokio::spawn(run_swarm(swarm, command_rx, event_tx));

        let node = Self {
            command_tx,
            local_peer_id,
        };

        Ok((node, event_rx))
    }

    /// The local peer ID of this node.
    pub fn peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// Subscribe to a gossipsub topic (typically a channel identifier).
    pub fn subscribe(&self, topic: &str) -> Result<()> {
        self.command_tx
            .send(Command::Subscribe(topic.to_string()))
            .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
        Ok(())
    }

    /// Unsubscribe from a gossipsub topic.
    pub fn unsubscribe(&self, topic: &str) -> Result<()> {
        self.command_tx
            .send(Command::Unsubscribe(topic.to_string()))
            .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
        Ok(())
    }

    /// Publish a message to a gossipsub topic.
    pub fn publish(&self, topic: &str, data: Vec<u8>) -> Result<()> {
        self.command_tx
            .send(Command::Publish {
                topic: topic.to_string(),
                data,
            })
            .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
        Ok(())
    }

    /// Dial a remote peer by multiaddress.
    pub fn dial(&self, addr: Multiaddr) -> Result<()> {
        self.command_tx
            .send(Command::Dial(addr))
            .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
        Ok(())
    }
}

// ───── Swarm construction ────────────────────────────────────────────────────

/// Build the libp2p swarm with all configured protocols.
fn build_swarm(
    keypair: libp2p::identity::Keypair,
    config: &NetworkConfig,
) -> Result<Swarm<WillowBehaviour>> {
    let peer_id = PeerId::from(keypair.public());

    let swarm = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_relay_client(noise::Config::new, yamux::Config::default)?
        .with_behaviour(|key, relay_behaviour| {
            // GossipSub — use content-based message IDs to deduplicate.
            let gossipsub_config = gossipsub::ConfigBuilder::default()
                .heartbeat_interval(config.gossipsub_heartbeat)
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
            let kademlia = kad::Behaviour::new(peer_id, kad::store::MemoryStore::new(peer_id));

            // mDNS
            let mdns = mdns::tokio::Behaviour::new(
                mdns::Config::default(),
                peer_id,
            )
            .expect("valid mdns behaviour");

            // Identify
            let identify = identify::Behaviour::new(identify::Config::new(
                "/willow/1.0.0".to_string(),
                key.public(),
            ));

            Ok(WillowBehaviour {
                gossipsub,
                kademlia,
                mdns,
                identify,
                relay: relay_behaviour,
            })
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(config.idle_timeout))
        .build();

    Ok(swarm)
}

// ───── Swarm event loop ──────────────────────────────────────────────────────

/// The main event loop that drives the swarm on a background task.
async fn run_swarm(
    mut swarm: Swarm<WillowBehaviour>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<NetworkEvent>,
) {
    loop {
        tokio::select! {
            // Process commands from the handle.
            cmd = commands.recv() => {
                match cmd {
                    Some(Command::Subscribe(topic)) => {
                        let topic = gossipsub::IdentTopic::new(&topic);
                        if let Err(e) = swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                            warn!(%e, "failed to subscribe");
                        }
                    }
                    Some(Command::Unsubscribe(topic)) => {
                        let topic = gossipsub::IdentTopic::new(&topic);
                        if let Err(e) = swarm.behaviour_mut().gossipsub.unsubscribe(&topic) {
                            warn!(%e, "failed to unsubscribe");
                        }
                    }
                    Some(Command::Publish { topic, data }) => {
                        let topic = gossipsub::IdentTopic::new(&topic);
                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic, data) {
                            warn!(%e, "failed to publish");
                        }
                    }
                    Some(Command::Dial(addr)) => {
                        if let Err(e) = swarm.dial(addr) {
                            warn!(%e, "failed to dial");
                        }
                    }
                    None => {
                        info!("command channel closed, shutting down swarm");
                        return;
                    }
                }
            }

            // Process swarm events.
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::Behaviour(behaviour::WillowBehaviourEvent::Gossipsub(
                        gossipsub::Event::Message { message, .. },
                    )) => {
                        let topic = message.topic.to_string();
                        let _ = events.send(NetworkEvent::Message {
                            topic,
                            data: message.data,
                            source: message.source,
                        });
                    }

                    SwarmEvent::Behaviour(behaviour::WillowBehaviourEvent::Mdns(
                        mdns::Event::Discovered(peers),
                    )) => {
                        for (peer_id, addr) in peers {
                            debug!(%peer_id, %addr, "mDNS: discovered peer");
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                            let _ = events.send(NetworkEvent::PeerDiscovered {
                                peer_id,
                                addrs: vec![addr],
                            });
                        }
                    }

                    SwarmEvent::Behaviour(behaviour::WillowBehaviourEvent::Mdns(
                        mdns::Event::Expired(peers),
                    )) => {
                        for (peer_id, _addr) in peers {
                            debug!(%peer_id, "mDNS: peer expired");
                            swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                        }
                    }

                    SwarmEvent::Behaviour(behaviour::WillowBehaviourEvent::Identify(
                        identify::Event::Received { peer_id, info },
                    )) => {
                        debug!(%peer_id, protocol = %info.protocol_version, "identify: received");
                        for addr in info.listen_addrs {
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                        }
                    }

                    SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                        info!(%peer_id, "connection established");
                        let _ = events.send(NetworkEvent::PeerConnected(peer_id));
                    }

                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        debug!(%peer_id, "connection closed");
                        let _ = events.send(NetworkEvent::PeerDisconnected(peer_id));
                    }

                    SwarmEvent::NewListenAddr { address, .. } => {
                        info!(%address, "listening on");
                        let _ = events.send(NetworkEvent::Listening(address));
                    }

                    _ => {}
                }
            }
        }
    }
}

// We import behaviour module to reference the generated event enum.
use crate::behaviour;

use libp2p::futures::StreamExt;
