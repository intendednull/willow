//! # Willow Network
//!
//! The P2P networking layer for Willow, built on [libp2p] with a [tokio]
//! runtime.
//!
//! ## Architecture
//!
//! The network is composed of several libp2p protocols working together:
//!
//! - **[GossipSub]** — Pub/sub messaging. Each channel maps to a gossipsub
//!   topic, so messages are flooded only to interested peers.
//! - **[Kademlia]** — Distributed hash table for peer discovery and content
//!   routing beyond the local network.
//! - **[mDNS]** — Automatic discovery of peers on the same LAN.
//! - **[Identify]** — Exchange of peer metadata on connection.
//! - **[Relay]** — NAT traversal via relay nodes when direct connections fail.
//!
//! ## Usage
//!
//! ```no_run
//! use willow_network::{NetworkConfig, NetworkNode, NetworkEvent};
//! use willow_identity::Identity;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let identity = Identity::generate();
//! let config = NetworkConfig::default();
//! let (mut node, mut events) = NetworkNode::start(identity, config).await?;
//!
//! // Subscribe to a channel topic.
//! node.subscribe("my-server/general")?;
//!
//! // Publish a message.
//! node.publish("my-server/general", b"hello everyone".to_vec())?;
//!
//! // Process incoming events.
//! while let Some(event) = events.recv().await {
//!     match event {
//!         NetworkEvent::Message { topic, data, source } => {
//!             println!("got message on {topic} from {source:?}");
//!         }
//!         NetworkEvent::PeerConnected(peer) => {
//!             println!("peer connected: {peer}");
//!         }
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [GossipSub]: libp2p::gossipsub
//! [Kademlia]: libp2p::kad
//! [mDNS]: libp2p::mdns
//! [Identify]: libp2p::identify
//! [Relay]: libp2p::relay

pub mod behaviour;
pub mod config;
pub mod node;

pub use behaviour::WillowBehaviour;
pub use config::NetworkConfig;
pub use node::{NetworkEvent, NetworkNode};
