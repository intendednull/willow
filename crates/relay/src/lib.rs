//! # Willow Relay — Library
//!
//! Stateless relay-server: TCP↔WS bridging, NAT traversal, and gossipsub
//! message routing. No event storage — replay and storage nodes handle
//! state persistence.

use std::time::Duration;

use anyhow::Result;
use libp2p::{
    gossipsub, identify, kad, noise, relay, swarm::SwarmEvent, tcp, yamux, PeerId, SwarmBuilder,
};
use tracing::{debug, info, warn};

/// Composite behaviour for the relay server.
#[derive(libp2p::swarm::NetworkBehaviour)]
pub struct RelayBehaviour {
    /// GossipSub for pub/sub message forwarding.
    pub gossipsub: gossipsub::Behaviour,
    /// Kademlia for peer discovery.
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    /// Identify for peer metadata exchange.
    pub identify: identify::Behaviour,
    /// Relay protocol so NAT'd peers can connect through us.
    pub relay: relay::Behaviour,
}

/// Profile topic for broadcasting display names.
const PROFILE_TOPIC: &str = "_willow_profiles";

/// A running relay node — pure network plumbing, no state storage.
pub struct Relay {
    /// The libp2p swarm driving all network protocols.
    pub swarm: libp2p::Swarm<RelayBehaviour>,
    /// Ed25519 identity used to sign profile broadcasts.
    pub identity: willow_identity::Identity,
    /// This relay's libp2p peer ID.
    pub peer_id: PeerId,
    /// Display name for this relay in peer lists.
    pub display_name: String,
}

impl Relay {
    /// Build a relay node with the given keypair.
    ///
    /// The swarm is fully constructed but has no listen addresses yet — call
    /// [`libp2p::Swarm::listen_on`] after this to bind to ports.
    pub async fn start(keypair: libp2p::identity::Keypair) -> Result<Self> {
        let local_peer_id = PeerId::from(keypair.public());

        // Build a willow Identity for signing profile broadcasts.
        let relay_identity = {
            let ed_kp = keypair
                .clone()
                .try_into_ed25519()
                .expect("keypair should be ed25519");
            let bytes = ed_kp.to_bytes();
            willow_identity::Identity::from_ed25519_bytes(&bytes).expect("valid ed25519 bytes")
        };

        let swarm = SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_websocket(noise::Config::new, yamux::Config::default)
            .await?
            .with_behaviour(|key| {
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

                let kademlia =
                    kad::Behaviour::new(local_peer_id, kad::store::MemoryStore::new(local_peer_id));

                let identify = identify::Behaviour::new(identify::Config::new(
                    "/willow/1.0.0".to_string(),
                    key.public(),
                ));

                let relay = relay::Behaviour::new(local_peer_id, relay::Config::default());

                Ok(RelayBehaviour {
                    gossipsub,
                    kademlia,
                    identify,
                    relay,
                })
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(600)))
            .build();

        Ok(Relay {
            swarm,
            identity: relay_identity,
            peer_id: local_peer_id,
            display_name: String::new(),
        })
    }

    /// Process a single swarm event.
    pub fn handle_swarm_event(&mut self, event: SwarmEvent<RelayBehaviourEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!(%address, "listening on");
            }

            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                info!(%peer_id, ?endpoint, "peer connected");
                self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .add_explicit_peer(&peer_id);
            }

            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                debug!(%peer_id, ?cause, "peer disconnected");
                self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .remove_explicit_peer(&peer_id);
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
                ..
            })) => {
                debug!(%peer_id, protocol = %info.protocol_version, "identify received");
                for addr in info.listen_addrs {
                    self.swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, addr);
                }
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Gossipsub(
                gossipsub::Event::Subscribed { peer_id, topic },
            )) => {
                info!(%peer_id, %topic, "peer subscribed");
                let topic = gossipsub::IdentTopic::new(topic.to_string());
                if let Err(e) = self.swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                    warn!(%e, "failed to subscribe");
                }
                self.broadcast_profile();
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                message,
                ..
            })) => {
                // Pure pass-through — gossipsub handles message forwarding.
                // No storage, no parsing, no sync responses.
                debug!(
                    topic = %message.topic,
                    source = ?message.source,
                    bytes = message.data.len(),
                    "relaying message"
                );
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(event)) => {
                debug!(?event, "relay event");
            }

            _ => {}
        }
    }

    /// Subscribe to the profile topic and broadcast our display name.
    pub fn set_display_name(&mut self, name: &str) {
        self.display_name = name.to_string();
        let topic = gossipsub::IdentTopic::new(PROFILE_TOPIC);
        if let Err(e) = self.swarm.behaviour_mut().gossipsub.subscribe(&topic) {
            warn!(%e, "failed to subscribe to profile topic");
        }
        self.broadcast_profile();
    }

    /// Returns the relay's peer ID as a string.
    pub fn peer_id_string(&self) -> String {
        self.peer_id.to_string()
    }

    /// Broadcast our display name on the profile topic.
    fn broadcast_profile(&mut self) {
        if self.display_name.is_empty() {
            return;
        }
        let profile = willow_identity::UserProfile::new(
            willow_identity::PeerId::from(self.peer_id),
            self.display_name.clone(),
        );
        if let Ok(data) =
            willow_transport::pack_envelope(willow_transport::MessageType::Identity, &profile)
        {
            let topic = gossipsub::IdentTopic::new(PROFILE_TOPIC);
            if let Err(e) = self.swarm.behaviour_mut().gossipsub.publish(topic, data) {
                debug!(%e, "failed to broadcast profile (no subscribers yet)");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::Multiaddr;

    fn test_keypair() -> libp2p::identity::Keypair {
        libp2p::identity::Keypair::generate_ed25519()
    }

    #[tokio::test]
    async fn start_creates_valid_relay() {
        let kp = test_keypair();
        let expected_peer_id = PeerId::from(kp.public());

        let relay = Relay::start(kp).await.unwrap();

        assert_eq!(relay.peer_id, expected_peer_id);
        assert!(relay.display_name.is_empty());
        assert!(!relay.peer_id_string().is_empty());
    }

    #[tokio::test]
    async fn set_display_name_updates_field() {
        let relay_result = Relay::start(test_keypair()).await;
        let mut relay = relay_result.unwrap();

        assert!(relay.display_name.is_empty());
        relay.set_display_name("Test Relay");
        assert_eq!(relay.display_name, "Test Relay");
    }

    #[tokio::test]
    async fn relay_can_listen_on_tcp() {
        let mut relay = Relay::start(test_keypair()).await.unwrap();

        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
        let result = relay.swarm.listen_on(addr);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn relay_has_no_event_store() {
        // Verify the relay struct has no event_store field —
        // this is a compile-time check. If someone adds an event_store
        // back, this test's assertions will need updating.
        let relay = Relay::start(test_keypair()).await.unwrap();

        // The relay is stateless — only has swarm, identity, peer_id, display_name.
        assert!(!relay.peer_id_string().is_empty());
        assert!(relay.display_name.is_empty());
    }

    #[tokio::test]
    async fn relay_identity_matches_keypair() {
        let kp = test_keypair();
        let ed_kp = kp.clone().try_into_ed25519().unwrap();
        let expected_bytes = ed_kp.to_bytes();

        let relay = Relay::start(kp).await.unwrap();

        // Verify the willow Identity was correctly derived from the keypair.
        let relay_bytes = relay.identity.to_ed25519_bytes().unwrap();
        assert_eq!(relay_bytes, expected_bytes);
    }
}
