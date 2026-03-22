//! Composite libp2p network behaviour for Willow.
//!
//! [`WillowBehaviour`] combines GossipSub, Kademlia, Identify, and Relay Client
//! into a single [`NetworkBehaviour`]. On native targets, mDNS is also included
//! for LAN peer discovery.

use libp2p::{gossipsub, identify, kad, relay, swarm::NetworkBehaviour};

/// The composite behaviour that powers a Willow peer.
///
/// | Field       | Protocol   | Purpose                                |
/// |-------------|------------|----------------------------------------|
/// | `gossipsub` | GossipSub  | Pub/sub message flooding per topic     |
/// | `kademlia`  | Kademlia   | DHT for peer/content discovery         |
/// | `mdns`      | mDNS       | LAN peer discovery (native only)       |
/// | `identify`  | Identify   | Peer metadata exchange on connect      |
/// | `relay`     | Relay      | NAT traversal via relay nodes          |
#[derive(NetworkBehaviour)]
pub struct WillowBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    #[cfg(not(target_arch = "wasm32"))]
    pub mdns: libp2p::mdns::tokio::Behaviour,
    pub identify: identify::Behaviour,
    pub relay: relay::client::Behaviour,
}
