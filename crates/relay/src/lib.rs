//! # Willow Relay — Library
//!
//! Reusable relay-server logic: swarm construction, event storage, and message
//! handling. The binary in `main.rs` is a thin CLI wrapper around this library.

pub mod event_store;

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use libp2p::{
    gossipsub, identify, kad, noise, relay, swarm::SwarmEvent, tcp, yamux, PeerId, SwarmBuilder,
};
use tracing::{debug, info, warn};

/// Wire message format — mirrors the client's `WireMessage` but defined
/// locally to avoid pulling in the full client dependency.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum WireMessage {
    /// A state event to be applied.
    Event(willow_state::Event),
    /// Request missing events from peers / relay.
    SyncRequest {
        /// Hash of the requester's current state.
        state_hash: willow_state::StateHash,
        /// Optional topic to scope the request.
        topic: Option<String>,
    },
    /// A batch of events sent in response to a `SyncRequest`.
    SyncBatch {
        /// The events being synced.
        events: Vec<willow_state::Event>,
    },
}

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

/// A running relay node with its swarm, event store, and signing identity.
pub struct Relay {
    /// The libp2p swarm driving all network protocols.
    pub swarm: libp2p::Swarm<RelayBehaviour>,
    /// Persistent store for events passing through the relay.
    pub event_store: event_store::RelayEventStore,
    /// Ed25519 identity used to sign sync responses.
    pub identity: willow_identity::Identity,
    /// This relay's libp2p peer ID.
    pub peer_id: PeerId,
    /// Display name for this relay in peer lists.
    pub display_name: String,
}

impl Relay {
    /// Build a relay node with the given keypair and database path.
    ///
    /// The swarm is fully constructed but has no listen addresses yet — call
    /// [`libp2p::Swarm::listen_on`] after this to bind to ports.
    pub async fn start(keypair: libp2p::identity::Keypair, db_path: &Path) -> Result<Self> {
        let local_peer_id = PeerId::from(keypair.public());

        // Build a willow Identity for signing sync responses.
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
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(300)))
            .build();

        std::fs::create_dir_all(db_path.parent().unwrap_or(Path::new("."))).ok();
        let event_store =
            event_store::RelayEventStore::open(db_path).context("failed to open event store")?;

        Ok(Relay {
            swarm,
            event_store,
            identity: relay_identity,
            peer_id: local_peer_id,
            display_name: String::new(),
        })
    }

    /// Process a single swarm event. Returns `true` if a message was stored.
    pub fn handle_swarm_event(&mut self, event: SwarmEvent<RelayBehaviourEvent>) -> bool {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!(%address, "listening on");
                false
            }

            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                info!(%peer_id, ?endpoint, "peer connected");
                false
            }

            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                debug!(%peer_id, ?cause, "peer disconnected");
                false
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
                false
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Gossipsub(
                gossipsub::Event::Subscribed { peer_id, topic },
            )) => {
                info!(%peer_id, %topic, "peer subscribed");
                let topic = gossipsub::IdentTopic::new(topic.to_string());
                if let Err(e) = self.swarm.behaviour_mut().gossipsub.subscribe(&topic) {
                    warn!(%e, "failed to subscribe");
                }
                // Broadcast our display name so the new peer sees us.
                self.broadcast_profile();
                false
            }

            SwarmEvent::Behaviour(RelayBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                message,
                ..
            })) => self.handle_gossipsub_message(&message),

            SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(event)) => {
                debug!(?event, "relay event");
                false
            }

            _ => false,
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

    /// Handle a single GossipSub message: parse, store events, respond to sync
    /// requests. Returns `true` if an event was stored.
    fn handle_gossipsub_message(&mut self, message: &gossipsub::Message) -> bool {
        let topic_str = message.topic.to_string();
        let data = &message.data;
        let mut stored = false;

        if let Ok((envelope_bytes, _signer)) = willow_identity::unpack::<Vec<u8>>(data) {
            if let Ok((wire_msg, willow_transport::MessageType::Channel)) =
                willow_transport::unpack_envelope::<WireMessage>(&envelope_bytes)
            {
                match wire_msg {
                    WireMessage::Event(ref event) => {
                        self.event_store.store_event(&topic_str, event, data);
                        debug!(
                            id = %event.id,
                            author = %event.author,
                            topic = %topic_str,
                            "stored event"
                        );
                        stored = true;
                    }
                    WireMessage::SyncRequest { ref topic, .. } => {
                        let events = if let Some(ref t) = topic {
                            self.event_store.events_for_topic_since(t, 0)
                        } else {
                            self.event_store.all_events_since(0)
                        };

                        if !events.is_empty() {
                            let batch = WireMessage::SyncBatch {
                                events: events.clone(),
                            };
                            if let Ok(envelope) = willow_transport::pack_envelope(
                                willow_transport::MessageType::Channel,
                                &batch,
                            ) {
                                if let Ok(signed) = willow_identity::pack(&envelope, &self.identity)
                                {
                                    let reply_topic =
                                        topic.as_deref().unwrap_or("_willow_server_ops");
                                    let gt = gossipsub::IdentTopic::new(reply_topic);
                                    let _ =
                                        self.swarm.behaviour_mut().gossipsub.publish(gt, signed);
                                    info!(
                                        count = events.len(),
                                        topic = %reply_topic,
                                        "relay sent sync response"
                                    );
                                }
                            }
                        } else {
                            debug!("sync request but no events stored");
                        }
                    }
                    WireMessage::SyncBatch { ref events } => {
                        for event in events {
                            self.event_store.store_event(&topic_str, event, &[]);
                        }
                        debug!(count = events.len(), "cached sync batch");
                    }
                }
            }
        }

        debug!(
            topic = %topic_str,
            source = ?message.source,
            bytes = data.len(),
            stored = self.event_store.count(),
            "relaying message"
        );

        stored
    }
}
