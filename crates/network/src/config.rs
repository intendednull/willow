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
    /// Ignored on WASM (browser peers cannot listen).
    pub listen_addr: Multiaddr,

    /// Bootstrap peers to connect to on startup (Kademlia).
    ///
    /// For a friend group this might be one VPS node that is always online. An
    /// empty list means "discover peers via mDNS only" (native) or "no
    /// connectivity" (WASM).
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
            idle_timeout: Duration::from_secs(600),
            gossipsub_heartbeat: Duration::from_secs(1),
        }
    }
}

impl NetworkConfig {
    /// Parse a relay multiaddr string and add it as a bootstrap peer.
    ///
    /// The address must end with `/p2p/<peer_id>`, e.g.:
    /// ```text
    /// /ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooWAbCdEf...
    /// ```
    ///
    /// Returns `Err` if the address is malformed or missing the `/p2p` suffix.
    pub fn with_relay(mut self, addr_str: &str) -> anyhow::Result<Self> {
        let addr: Multiaddr = addr_str
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid relay address: {e}"))?;

        // Extract the PeerId from the /p2p/ suffix.
        let peer_id = addr
            .iter()
            .find_map(|p| {
                if let libp2p::multiaddr::Protocol::P2p(peer_id) = p {
                    Some(peer_id)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                anyhow::anyhow!("relay address must end with /p2p/<peer_id>, got: {addr_str}")
            })?;

        // Strip the /p2p suffix from the address for dialing.
        let dial_addr: Multiaddr = addr
            .iter()
            .filter(|p| !matches!(p, libp2p::multiaddr::Protocol::P2p(_)))
            .collect();

        self.bootstrap_peers.push((peer_id, dial_addr));
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_relay_parses_tcp_address() {
        let config = NetworkConfig::default()
            .with_relay(
                "/ip4/127.0.0.1/tcp/9090/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN",
            )
            .unwrap();

        assert_eq!(config.bootstrap_peers.len(), 1);
        let (peer_id, addr) = &config.bootstrap_peers[0];
        assert_eq!(
            peer_id.to_string(),
            "12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN"
        );
        assert_eq!(addr.to_string(), "/ip4/127.0.0.1/tcp/9090");
    }

    #[test]
    fn with_relay_parses_ws_address() {
        let config = NetworkConfig::default()
            .with_relay(
                "/ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN",
            )
            .unwrap();

        assert_eq!(config.bootstrap_peers.len(), 1);
        let (_peer_id, addr) = &config.bootstrap_peers[0];
        assert_eq!(addr.to_string(), "/ip4/1.2.3.4/tcp/9091/ws");
    }

    #[test]
    fn with_relay_rejects_missing_peer_id() {
        let result = NetworkConfig::default().with_relay("/ip4/127.0.0.1/tcp/9090");
        assert!(result.is_err());
    }

    #[test]
    fn with_relay_rejects_invalid_addr() {
        let result = NetworkConfig::default().with_relay("not-a-multiaddr");
        assert!(result.is_err());
    }

    #[test]
    fn default_config_values() {
        let config = NetworkConfig::default();
        assert_eq!(config.listen_addr.to_string(), "/ip4/0.0.0.0/tcp/0");
        assert!(config.bootstrap_peers.is_empty());
        assert_eq!(config.idle_timeout, Duration::from_secs(600));
        assert_eq!(config.gossipsub_heartbeat, Duration::from_secs(1));
    }

    #[test]
    fn with_relay_chained() {
        let config = NetworkConfig::default()
            .with_relay(
                "/ip4/1.1.1.1/tcp/9090/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN",
            )
            .unwrap()
            .with_relay(
                "/ip4/2.2.2.2/tcp/9091/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN",
            )
            .unwrap();

        assert_eq!(config.bootstrap_peers.len(), 2);
        // Other defaults preserved.
        assert_eq!(config.idle_timeout, Duration::from_secs(600));
    }
}
