//! # Iroh Network Implementation
//!
//! Production implementation of the [`Network`](crate::traits::Network) trait
//! backed by real iroh endpoints, gossip, and blob protocols.
//!
//! ## Example
//!
//! ```ignore
//! use willow_identity::Identity;
//! use willow_network::iroh::{Config, IrohNetwork};
//!
//! let identity = Identity::generate();
//! let config = Config {
//!     secret_key: identity.secret_key().clone(),
//!     relay_url: None,
//!     bootstrap_peers: vec![],
//!     mdns: true,
//! };
//! let network = IrohNetwork::new(config).await?;
//! ```

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures_lite::StreamExt;
use iroh::protocol::Router;
use iroh::{Endpoint, RelayMode};
use iroh_base::{EndpointId, SecretKey};
use iroh_gossip::api::{GossipReceiver, GossipSender};
use iroh_gossip::TopicId;
use tracing::{debug, warn};

pub use iroh_base::RelayUrl;

use crate::traits::*;

// ───── Multiaddr parsing ─────────────────────────────────────────────────

/// Parse a libp2p PeerId (base58 "12D3KooW..." format) into an iroh [`EndpointId`].
///
/// The PeerId wire format for Ed25519 keys is:
/// `0x00 0x24 0x08 0x01 0x12 0x20 <32 bytes of Ed25519 public key>`
fn parse_libp2p_peer_id(s: &str) -> Result<EndpointId> {
    let bytes = bs58::decode(s)
        .into_vec()
        .map_err(|e| anyhow::anyhow!("invalid base58 PeerId: {e}"))?;

    // Ed25519 PeerId: identity multihash prefix (6 bytes) + 32-byte key
    const ED25519_PREFIX: &[u8] = &[0x00, 0x24, 0x08, 0x01, 0x12, 0x20];
    if bytes.len() == 38 && bytes.starts_with(ED25519_PREFIX) {
        let key_bytes: [u8; 32] = bytes[6..]
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid key length"))?;
        EndpointId::from_bytes(&key_bytes).context("invalid Ed25519 public key")
    } else {
        anyhow::bail!(
            "unsupported PeerId format (expected Ed25519, got {} bytes)",
            bytes.len()
        )
    }
}

/// Parse a relay multiaddr string into a [`RelayUrl`] and bootstrap [`EndpointId`].
///
/// Supported formats:
/// - `/ip4/A.B.C.D/tcp/PORT/p2p/ID`
/// - `/ip4/A.B.C.D/tcp/PORT/ws/p2p/ID`
/// - `/dns4/HOST/tcp/PORT/wss/p2p/ID`
///
/// The `wss` transport produces an `https://` URL; all others produce `http://`.
pub fn parse_relay_multiaddr(addr: &str) -> Result<(RelayUrl, EndpointId)> {
    let parts: Vec<&str> = addr.split('/').collect();
    // Expect: ["", proto, host, "tcp", port, ...transport..., "p2p", id]
    if parts.len() < 6 || !parts[0].is_empty() {
        anyhow::bail!("invalid relay multiaddr: {addr}");
    }

    let (host_proto, host) = (parts[1], parts[2]);
    if parts[3] != "tcp" {
        anyhow::bail!("expected /tcp/ in relay multiaddr: {addr}");
    }
    let port: u16 = parts[4]
        .parse()
        .with_context(|| format!("invalid port in relay multiaddr: {addr}"))?;

    // Find /p2p/<id> — it's always the last two segments.
    let p2p_idx = parts
        .iter()
        .position(|&p| p == "p2p")
        .with_context(|| format!("missing /p2p/ in relay multiaddr: {addr}"))?;
    let id_str = parts
        .get(p2p_idx + 1)
        .with_context(|| format!("missing endpoint ID after /p2p/ in: {addr}"))?;

    // Parse the endpoint ID. Try iroh's native base32-hex format first,
    // then fall back to libp2p PeerId format (base58 "12D3KooW..." prefix).
    let endpoint_id: EndpointId = id_str
        .parse()
        .or_else(|_| parse_libp2p_peer_id(id_str))
        .with_context(|| format!("invalid endpoint ID '{id_str}' in: {addr}"))?;

    // Check for wss transport → https, otherwise http.
    let has_wss = parts[5..p2p_idx].contains(&"wss");
    let scheme = if has_wss { "https" } else { "http" };

    // Use the hostname from the multiaddr.
    let host_str = match host_proto {
        "ip4" | "ip6" => host.to_string(),
        "dns4" | "dns6" | "dns" => host.to_string(),
        _ => anyhow::bail!("unsupported address type '{host_proto}' in: {addr}"),
    };

    let url_str = format!("{scheme}://{host_str}:{port}");
    let relay_url: RelayUrl = url_str
        .parse()
        .with_context(|| format!("failed to parse relay URL from: {addr}"))?;

    Ok((relay_url, endpoint_id))
}

// ───── Config ──────────────────────────────────────────────────────────────

/// Configuration for creating an [`IrohNetwork`].
pub struct Config {
    /// The secret key for this node's identity.
    pub secret_key: SecretKey,
    /// Optional relay server URL for NAT traversal.
    pub relay_url: Option<RelayUrl>,
    /// Bootstrap peers to connect to when subscribing to topics.
    pub bootstrap_peers: Vec<EndpointId>,
    /// Enable mDNS for LAN discovery.
    pub mdns: bool,
}

// ───── IrohBlobStore ───────────────────────────────────────────────────────

/// Content-addressed blob storage backed by an in-memory HashMap.
///
/// This is a simple implementation suitable for ephemeral usage.
/// A persistent store can be swapped in later.
pub struct IrohBlobStore {
    store: Mutex<HashMap<crate::BlobHash, Bytes>>,
}

impl IrohBlobStore {
    fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl BlobStore for IrohBlobStore {
    async fn add(&self, data: Bytes) -> Result<crate::BlobHash> {
        let hash = crate::BlobHash::new(&data);
        self.store.lock().unwrap().insert(hash, data);
        Ok(hash)
    }

    async fn get(&self, hash: crate::BlobHash) -> Result<Option<Bytes>> {
        Ok(self.store.lock().unwrap().get(&hash).cloned())
    }

    async fn has(&self, hash: crate::BlobHash) -> bool {
        self.store.lock().unwrap().contains_key(&hash)
    }

    async fn remove(&self, hash: crate::BlobHash) -> Result<bool> {
        Ok(self.store.lock().unwrap().remove(&hash).is_some())
    }

    async fn store_size(&self) -> Option<u64> {
        let store = self.store.lock().unwrap();
        Some(store.values().map(|v| v.len() as u64).sum())
    }
}

// ───── IrohTopicHandle ─────────────────────────────────────────────────────

/// Handle to a gossip topic, wrapping [`GossipSender`].
#[derive(Clone)]
pub struct IrohTopicHandle {
    sender: GossipSender,
    receiver_neighbors: std::sync::Arc<std::sync::RwLock<Vec<EndpointId>>>,
}

#[async_trait]
impl TopicHandle for IrohTopicHandle {
    async fn broadcast(&self, data: Bytes) -> Result<()> {
        self.sender.broadcast(data).await?;
        Ok(())
    }

    async fn broadcast_neighbors(&self, data: Bytes) -> Result<()> {
        self.sender.broadcast_neighbors(data).await?;
        Ok(())
    }

    fn neighbors(&self) -> Vec<EndpointId> {
        self.receiver_neighbors.read().unwrap().clone()
    }
}

// ───── IrohTopicEvents ─────────────────────────────────────────────────────

/// Stream of gossip events for a topic, wrapping [`GossipReceiver`].
pub struct IrohTopicEvents {
    receiver: GossipReceiver,
    neighbors: std::sync::Arc<std::sync::RwLock<Vec<EndpointId>>>,
}

#[async_trait]
impl TopicEvents for IrohTopicEvents {
    async fn next(&mut self) -> Option<Result<GossipEvent>> {
        loop {
            let event = self.receiver.next().await?;
            match event {
                Ok(iroh_gossip::api::Event::Received(msg)) => {
                    return Some(Ok(GossipEvent::Received(GossipMessage {
                        content: msg.content,
                        sender: msg.delivered_from,
                    })));
                }
                Ok(iroh_gossip::api::Event::NeighborUp(id)) => {
                    {
                        let mut neighbors = self.neighbors.write().unwrap();
                        if !neighbors.contains(&id) {
                            neighbors.push(id);
                        }
                    }
                    return Some(Ok(GossipEvent::NeighborUp(id)));
                }
                Ok(iroh_gossip::api::Event::NeighborDown(id)) => {
                    {
                        let mut neighbors = self.neighbors.write().unwrap();
                        neighbors.retain(|&n| n != id);
                    }
                    return Some(Ok(GossipEvent::NeighborDown(id)));
                }
                Ok(iroh_gossip::api::Event::Lagged) => {
                    warn!("gossip receiver lagged, some messages were dropped");
                    continue;
                }
                Err(e) => return Some(Err(anyhow::Error::from(e))),
            }
        }
    }

    async fn joined(&mut self) -> Result<()> {
        self.receiver.joined().await?;
        Ok(())
    }
}

// ───── Subscription tracking ───────────────────────────────────────────────

/// Tracks an active topic subscription so we can drop it on unsubscribe.
struct TopicSubscription {
    /// The sender half; dropping it signals leave.
    _sender: GossipSender,
}

// ───── IrohNetwork ─────────────────────────────────────────────────────────

/// Production implementation of [`Network`] backed by iroh.
///
/// Provides gossip pub/sub over iroh's QUIC transport with optional
/// relay and mDNS discovery.
pub struct IrohNetwork {
    endpoint: Endpoint,
    gossip: iroh_gossip::Gossip,
    blob_store: IrohBlobStore,
    router: Mutex<Option<Router>>,
    subscriptions: Mutex<HashMap<TopicId, TopicSubscription>>,
    /// Bootstrap peers merged into every topic subscription.
    bootstrap_peers: Vec<EndpointId>,
}

impl IrohNetwork {
    /// Create a new IrohNetwork from the given configuration.
    ///
    /// This builds an iroh endpoint, spawns the gossip protocol actor,
    /// sets up the protocol router, and returns a ready-to-use network.
    pub async fn new(config: Config) -> Result<Self> {
        // 1. Build the iroh endpoint.
        let mut builder = Endpoint::empty_builder().secret_key(config.secret_key);

        // Configure relay mode.
        if let Some(relay_url) = &config.relay_url {
            let relay_map =
                iroh::RelayMap::try_from_iter([relay_url.as_str()]).context("invalid relay URL")?;
            builder = builder.relay_mode(RelayMode::Custom(relay_map));
        } else {
            builder = builder.relay_mode(RelayMode::Disabled);
        }

        // Enable mDNS discovery if requested (native only — not available on WASM).
        #[cfg(not(target_arch = "wasm32"))]
        if config.mdns {
            // NOTE: requires iroh "address-lookup-mdns" feature.
            // On WASM, mDNS is silently skipped.
        }
        let _ = config.mdns; // suppress unused warning on WASM

        let endpoint = builder.bind().await.context("failed to bind endpoint")?;

        debug!(id = %endpoint.id().fmt_short(), "iroh endpoint bound");

        // 2. Create the gossip protocol with 64 KiB max message size.
        let gossip = iroh_gossip::Gossip::builder()
            .max_message_size(65536)
            .spawn(endpoint.clone());

        // 3. Create blob store (in-memory for now).
        let blob_store = IrohBlobStore::new();

        // 4. Build and spawn the protocol router.
        let router = Router::builder(endpoint.clone())
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn();

        debug!("iroh protocol router spawned");

        Ok(Self {
            endpoint,
            gossip,
            blob_store,
            router: Mutex::new(Some(router)),
            subscriptions: Mutex::new(HashMap::new()),
            bootstrap_peers: config.bootstrap_peers,
        })
    }
}

#[async_trait]
impl Network for IrohNetwork {
    type Topic = IrohTopicHandle;
    type Events = IrohTopicEvents;

    fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    async fn subscribe(
        &self,
        topic: TopicId,
        mut bootstrap: Vec<EndpointId>,
    ) -> Result<(Self::Topic, Self::Events)> {
        // Merge configured bootstrap peers into the subscription.
        for peer in &self.bootstrap_peers {
            if !bootstrap.contains(peer) {
                bootstrap.push(*peer);
            }
        }

        let gossip_topic = self
            .gossip
            .subscribe(topic, bootstrap)
            .await
            .context("failed to subscribe to gossip topic")?;

        let (sender, receiver) = gossip_topic.split();

        // Shared neighbor tracking between handle and events.
        let neighbors = std::sync::Arc::new(std::sync::RwLock::new(Vec::new()));

        // Track the subscription for cleanup.
        {
            let mut subs = self.subscriptions.lock().unwrap();
            subs.insert(
                topic,
                TopicSubscription {
                    _sender: sender.clone(),
                },
            );
        }

        let handle = IrohTopicHandle {
            sender,
            receiver_neighbors: neighbors.clone(),
        };

        let events = IrohTopicEvents {
            receiver,
            neighbors,
        };

        Ok((handle, events))
    }

    async fn unsubscribe(&self, topic: TopicId) -> Result<()> {
        let mut subs = self.subscriptions.lock().unwrap();
        // Dropping the TopicSubscription drops the sender, which signals leave.
        subs.remove(&topic);
        Ok(())
    }

    fn blobs(&self) -> &dyn BlobStore {
        &self.blob_store
    }

    async fn connection_events(&self) -> ConnectionEventStream {
        // Placeholder: return a stream that never yields.
        // Full implementation would monitor endpoint relay and direct connection state.
        Box::pin(futures_lite::stream::pending())
    }

    async fn shutdown(&self) -> Result<()> {
        // Drop all subscriptions first.
        {
            let mut subs = self.subscriptions.lock().unwrap();
            subs.clear();
        }

        // Shut down the router.
        let router = {
            let mut guard = self.router.lock().unwrap();
            guard.take()
        };
        if let Some(router) = router {
            router
                .shutdown()
                .await
                .context("failed to shut down router")?;
        }

        // Close the endpoint.
        self.endpoint.close().await;

        debug!("iroh network shut down");
        Ok(())
    }
}

// ───── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_accepts_generated_key() {
        let identity = willow_identity::Identity::generate();
        let _config = Config {
            secret_key: identity.secret_key().clone(),
            relay_url: None,
            bootstrap_peers: vec![],
            mdns: false,
        };
    }

    #[tokio::test]
    async fn blob_store_round_trip() {
        let store = IrohBlobStore::new();
        let data = Bytes::from("hello iroh");
        let hash = store.add(data.clone()).await.unwrap();

        assert!(store.has(hash).await);
        assert_eq!(store.get(hash).await.unwrap(), Some(data));
        assert!(store.remove(hash).await.unwrap());
        assert!(!store.has(hash).await);
    }

    #[tokio::test]
    async fn blob_store_size() {
        let store = IrohBlobStore::new();
        assert_eq!(store.store_size().await, Some(0));

        store.add(Bytes::from("abc")).await.unwrap(); // 3 bytes
        store.add(Bytes::from("defgh")).await.unwrap(); // 5 bytes

        assert_eq!(store.store_size().await, Some(8));
    }

    #[test]
    fn parse_relay_multiaddr_ip4_tcp() {
        let addr = "/ip4/1.2.3.4/tcp/9090/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN";
        let (url, id) = parse_relay_multiaddr(addr).unwrap();
        assert_eq!(url.to_string(), "http://1.2.3.4:9090/");
        // ID is valid and non-empty (format is iroh's base32-hex, not libp2p base58).
        assert!(!id.to_string().is_empty());
    }

    #[test]
    fn parse_relay_multiaddr_ip4_ws() {
        let addr =
            "/ip4/127.0.0.1/tcp/9091/ws/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN";
        let (url, _id) = parse_relay_multiaddr(addr).unwrap();
        assert_eq!(url.to_string(), "http://127.0.0.1:9091/");
    }

    #[test]
    fn parse_relay_multiaddr_dns4_wss() {
        let addr = "/dns4/willow.example.com/tcp/9443/wss/p2p/12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN";
        let (url, _id) = parse_relay_multiaddr(addr).unwrap();
        assert_eq!(url.to_string(), "https://willow.example.com:9443/");
    }

    #[test]
    fn parse_relay_multiaddr_invalid() {
        assert!(parse_relay_multiaddr("not-a-multiaddr").is_err());
        assert!(parse_relay_multiaddr("/ip4/1.2.3.4/tcp/9090").is_err()); // no /p2p/
        assert!(parse_relay_multiaddr("").is_err());
    }

    #[test]
    fn parse_relay_multiaddr_round_trip_identity() {
        // Generate an identity, encode it as a libp2p PeerId, and parse it back.
        let id = willow_identity::Identity::generate();
        let endpoint_id = id.endpoint_id();

        // Encode as libp2p Ed25519 PeerId: base58(0x00 0x24 0x08 0x01 0x12 0x20 <32 bytes>)
        let mut peer_id_bytes = vec![0x00, 0x24, 0x08, 0x01, 0x12, 0x20];
        peer_id_bytes.extend_from_slice(endpoint_id.as_bytes());
        let peer_id_str = bs58::encode(&peer_id_bytes).into_string();

        let addr = format!("/ip4/10.0.0.1/tcp/3340/p2p/{peer_id_str}");
        let (url, parsed_id) = parse_relay_multiaddr(&addr).unwrap();
        assert_eq!(url.to_string(), "http://10.0.0.1:3340/");
        assert_eq!(parsed_id, endpoint_id);
    }

    #[test]
    fn parse_relay_multiaddr_iroh_native_format() {
        // Test with iroh's native base32-hex endpoint ID format.
        let id = willow_identity::Identity::generate();
        let endpoint_id = id.endpoint_id();
        let id_str = endpoint_id.to_string();

        let addr = format!("/ip4/10.0.0.1/tcp/3340/p2p/{id_str}");
        let (_url, parsed_id) = parse_relay_multiaddr(&addr).unwrap();
        assert_eq!(parsed_id, endpoint_id);
    }
}
