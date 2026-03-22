//! The high-level network node that manages the libp2p swarm.
//!
//! [`NetworkNode`] owns the swarm and runs it on a background task.
//! Callers interact with it through a command-style API (subscribe, publish,
//! dial) and receive events via an mpsc channel.

use anyhow::{Context, Result};
use libp2p::{
    gossipsub, identify, kad, noise, swarm::SwarmEvent, yamux, Multiaddr, PeerId, Swarm,
    SwarmBuilder,
};
use tracing::{debug, info, warn};

use crate::{NetworkConfig, WillowBehaviour};
use willow_identity::Identity;

// ───── Events ────────────────────────────────────────────────────────────────

/// Events emitted by the network layer to the application.
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    Message {
        topic: String,
        data: Vec<u8>,
        source: Option<PeerId>,
    },
    PeerConnected(PeerId),
    PeerDisconnected(PeerId),
    PeerDiscovered {
        peer_id: PeerId,
        addrs: Vec<Multiaddr>,
    },
    Listening(Multiaddr),
}

// ───── Commands ──────────────────────────────────────────────────────────────

#[derive(Debug)]
enum Command {
    Subscribe(String),
    Unsubscribe(String),
    Publish { topic: String, data: Vec<u8> },
    Dial(Multiaddr),
}

// ───── NetworkNode (native) ──────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use libp2p::{mdns, tcp};
    use tokio::sync::mpsc;

    pub struct NetworkNode {
        command_tx: mpsc::UnboundedSender<Command>,
        local_peer_id: PeerId,
    }

    impl NetworkNode {
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

            for (peer, addr) in &config.bootstrap_peers {
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(peer, addr.clone());
            }
            if !config.bootstrap_peers.is_empty() {
                swarm.behaviour_mut().kademlia.bootstrap().ok();
            }

            let (command_tx, command_rx) = mpsc::unbounded_channel();
            let (event_tx, event_rx) = mpsc::unbounded_channel();

            tokio::spawn(run_swarm(swarm, command_rx, event_tx));

            Ok((
                Self {
                    command_tx,
                    local_peer_id,
                },
                event_rx,
            ))
        }

        pub fn peer_id(&self) -> PeerId {
            self.local_peer_id
        }

        pub fn subscribe(&self, topic: &str) -> Result<()> {
            self.command_tx
                .send(Command::Subscribe(topic.to_string()))
                .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
            Ok(())
        }

        pub fn unsubscribe(&self, topic: &str) -> Result<()> {
            self.command_tx
                .send(Command::Unsubscribe(topic.to_string()))
                .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
            Ok(())
        }

        pub fn publish(&self, topic: &str, data: Vec<u8>) -> Result<()> {
            self.command_tx
                .send(Command::Publish {
                    topic: topic.to_string(),
                    data,
                })
                .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
            Ok(())
        }

        pub fn dial(&self, addr: Multiaddr) -> Result<()> {
            self.command_tx
                .send(Command::Dial(addr))
                .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
            Ok(())
        }
    }

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

                let kademlia = kad::Behaviour::new(peer_id, kad::store::MemoryStore::new(peer_id));

                let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id)
                    .expect("valid mdns behaviour");

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

    async fn run_swarm(
        mut swarm: Swarm<WillowBehaviour>,
        mut commands: mpsc::UnboundedReceiver<Command>,
        events: mpsc::UnboundedSender<NetworkEvent>,
    ) {
        use libp2p::futures::StreamExt;

        loop {
            tokio::select! {
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

                event = swarm.select_next_some() => {
                    match event {
                        SwarmEvent::Behaviour(crate::behaviour::WillowBehaviourEvent::Gossipsub(
                            gossipsub::Event::Message { message, .. },
                        )) => {
                            let topic = message.topic.to_string();
                            let _ = events.send(NetworkEvent::Message {
                                topic,
                                data: message.data,
                                source: message.source,
                            });
                        }

                        SwarmEvent::Behaviour(crate::behaviour::WillowBehaviourEvent::Mdns(
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

                        SwarmEvent::Behaviour(crate::behaviour::WillowBehaviourEvent::Mdns(
                            mdns::Event::Expired(peers),
                        )) => {
                            for (peer_id, _addr) in peers {
                                debug!(%peer_id, "mDNS: peer expired");
                                swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                            }
                        }

                        SwarmEvent::Behaviour(crate::behaviour::WillowBehaviourEvent::Identify(
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
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::NetworkNode;

// ───── NetworkNode (WASM) ────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::*;
    use futures::channel::mpsc;
    use futures::{SinkExt, StreamExt};

    pub struct NetworkNode {
        command_tx: mpsc::UnboundedSender<Command>,
        local_peer_id: PeerId,
    }

    impl NetworkNode {
        /// Start the network node using WebSocket transport for the browser.
        ///
        /// WASM peers cannot listen for incoming connections — they must
        /// dial out to a relay or bootstrap node via WebSocket.
        pub async fn start(
            identity: Identity,
            config: NetworkConfig,
        ) -> Result<(Self, mpsc::UnboundedReceiver<NetworkEvent>)> {
            let keypair = identity.keypair().clone();
            let local_peer_id = PeerId::from(keypair.public());

            info!(%local_peer_id, "starting willow network node (wasm)");

            let mut swarm = build_swarm(keypair.clone(), &config)?;

            // WASM can't listen — dial bootstrap peers instead.
            for (peer, addr) in &config.bootstrap_peers {
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(peer, addr.clone());
                if let Err(e) = swarm.dial(addr.clone()) {
                    warn!(%e, "failed to dial bootstrap peer");
                }
            }
            if !config.bootstrap_peers.is_empty() {
                swarm.behaviour_mut().kademlia.bootstrap().ok();
            }

            let (command_tx, command_rx) = mpsc::unbounded();
            let (event_tx, event_rx) = mpsc::unbounded();

            wasm_bindgen_futures::spawn_local(run_swarm(swarm, command_rx, event_tx));

            Ok((
                Self {
                    command_tx,
                    local_peer_id,
                },
                event_rx,
            ))
        }

        pub fn peer_id(&self) -> PeerId {
            self.local_peer_id
        }

        pub fn subscribe(&self, topic: &str) -> Result<()> {
            self.command_tx
                .unbounded_send(Command::Subscribe(topic.to_string()))
                .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
            Ok(())
        }

        pub fn unsubscribe(&self, topic: &str) -> Result<()> {
            self.command_tx
                .unbounded_send(Command::Unsubscribe(topic.to_string()))
                .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
            Ok(())
        }

        pub fn publish(&self, topic: &str, data: Vec<u8>) -> Result<()> {
            self.command_tx
                .unbounded_send(Command::Publish {
                    topic: topic.to_string(),
                    data,
                })
                .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
            Ok(())
        }

        pub fn dial(&self, addr: Multiaddr) -> Result<()> {
            self.command_tx
                .unbounded_send(Command::Dial(addr))
                .map_err(|_| anyhow::anyhow!("swarm task has stopped"))?;
            Ok(())
        }
    }

    fn build_swarm(
        keypair: libp2p::identity::Keypair,
        config: &NetworkConfig,
    ) -> Result<Swarm<WillowBehaviour>> {
        use libp2p::core::muxing::StreamMuxerBox;
        use libp2p::core::upgrade::Version;
        use libp2p::core::Transport as _;

        let peer_id = PeerId::from(keypair.public());

        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_wasm_bindgen()
            .with_other_transport(|key| {
                let ws = libp2p::websocket_websys::Transport::default();
                ws.upgrade(Version::V1)
                    .authenticate(noise::Config::new(key).expect("noise config"))
                    .multiplex(yamux::Config::default())
                    .map(|(peer, muxer), _| (peer, StreamMuxerBox::new(muxer)))
                    .boxed()
            })?
            .with_relay_client(noise::Config::new, yamux::Config::default)?
            .with_behaviour(|key, relay_behaviour| {
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

                let kademlia = kad::Behaviour::new(peer_id, kad::store::MemoryStore::new(peer_id));

                let identify = identify::Behaviour::new(identify::Config::new(
                    "/willow/1.0.0".to_string(),
                    key.public(),
                ));

                Ok(WillowBehaviour {
                    gossipsub,
                    kademlia,
                    identify,
                    relay: relay_behaviour,
                })
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(config.idle_timeout))
            .build();

        Ok(swarm)
    }

    async fn run_swarm(
        mut swarm: Swarm<WillowBehaviour>,
        mut commands: mpsc::UnboundedReceiver<Command>,
        mut events: mpsc::UnboundedSender<NetworkEvent>,
    ) {
        loop {
            futures::select! {
                cmd = commands.next() => {
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

                event = swarm.select_next_some() => {
                    match event {
                        SwarmEvent::Behaviour(crate::behaviour::WillowBehaviourEvent::Gossipsub(
                            gossipsub::Event::Message { message, .. },
                        )) => {
                            let topic = message.topic.to_string();
                            let _ = events.send(NetworkEvent::Message {
                                topic,
                                data: message.data,
                                source: message.source,
                            }).await;
                        }

                        SwarmEvent::Behaviour(crate::behaviour::WillowBehaviourEvent::Identify(
                            identify::Event::Received { peer_id, info },
                        )) => {
                            debug!(%peer_id, protocol = %info.protocol_version, "identify: received");
                            for addr in info.listen_addrs {
                                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                            }
                        }

                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            info!(%peer_id, "connection established");
                            let _ = events.send(NetworkEvent::PeerConnected(peer_id)).await;
                        }

                        SwarmEvent::ConnectionClosed { peer_id, .. } => {
                            debug!(%peer_id, "connection closed");
                            let _ = events.send(NetworkEvent::PeerDisconnected(peer_id)).await;
                        }

                        SwarmEvent::NewListenAddr { address, .. } => {
                            info!(%address, "listening on");
                            let _ = events.send(NetworkEvent::Listening(address)).await;
                        }

                        _ => {}
                    }
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::NetworkNode;
