//! Composite libp2p network behaviour for Willow.
//!
//! [`WillowBehaviour`] combines GossipSub, Kademlia, mDNS, Identify, and
//! Relay Client into a single [`NetworkBehaviour`] that drives the swarm.

use libp2p::{gossipsub, identify, kad, mdns, relay, swarm::NetworkBehaviour};

/// The composite behaviour that powers a Willow peer.
///
/// Each sub-behaviour handles a different concern:
///
/// | Field       | Protocol   | Purpose                                |
/// |-------------|------------|----------------------------------------|
/// | `gossipsub` | GossipSub  | Pub/sub message flooding per topic     |
/// | `kademlia`  | Kademlia   | DHT for peer/content discovery         |
/// | `mdns`      | mDNS       | LAN peer discovery                     |
/// | `identify`  | Identify   | Peer metadata exchange on connect      |
/// | `relay`     | Relay      | NAT traversal via relay nodes          |
#[derive(NetworkBehaviour)]
pub struct WillowBehaviour {
    /// Pub/sub messaging — one topic per channel.
    pub gossipsub: gossipsub::Behaviour,
    /// Distributed hash table for discovery.
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    /// Local network peer discovery.
    pub mdns: mdns::tokio::Behaviour,
    /// Peer identification and metadata exchange.
    pub identify: identify::Behaviour,
    /// Relay client for NAT traversal.
    pub relay: relay::client::Behaviour,
}
