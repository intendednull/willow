//! Composite libp2p network behaviour for Willow.
//!
//! [`WillowBehaviour`] combines GossipSub, Kademlia, Identify, Relay Client,
//! and a file chunk request-response protocol into a single [`NetworkBehaviour`].
//! On native targets, mDNS is also included for LAN peer discovery.

use libp2p::{gossipsub, identify, kad, relay, request_response, swarm::NetworkBehaviour};

use crate::file_transfer::{ChunkRequest, ChunkResponse};

/// The composite behaviour that powers a Willow peer.
#[derive(NetworkBehaviour)]
pub struct WillowBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    #[cfg(not(target_arch = "wasm32"))]
    pub mdns: libp2p::mdns::tokio::Behaviour,
    pub identify: identify::Behaviour,
    pub relay: relay::client::Behaviour,
    pub chunk_transfer: request_response::cbor::Behaviour<ChunkRequest, ChunkResponse>,
}
