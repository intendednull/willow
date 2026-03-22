//! Network configuration.

use std::time::Duration;

use libp2p::Multiaddr;

/// Configuration for a [`NetworkNode`](crate::NetworkNode).
///
/// Provides sane defaults for a friend-group deployment. Override fields as
/// needed for larger or more specialised networks.
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    /// Address to listen on. Defaults to all interfaces, OS-assigned port.
    pub listen_addr: Multiaddr,

    /// Bootstrap peers to connect to on startup (Kademlia).
    ///
    /// For a friend group this might be one VPS node that is always online. An
    /// empty list means "discover peers via mDNS only".
    pub bootstrap_peers: Vec<(libp2p::PeerId, Multiaddr)>,

    /// How long idle connections stay open before being closed.
    pub idle_timeout: Duration,

    /// GossipSub heartbeat interval.
    pub gossipsub_heartbeat: Duration,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_addr: "/ip4/0.0.0.0/tcp/0".parse().expect("valid multiaddr"),
            bootstrap_peers: Vec::new(),
            idle_timeout: Duration::from_secs(120),
            gossipsub_heartbeat: Duration::from_secs(1),
        }
    }
}
