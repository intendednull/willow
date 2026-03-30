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
use iroh::protocol::Router;
use iroh::{Endpoint, RelayMode};
use iroh_base::{EndpointId, SecretKey};
use futures_lite::StreamExt;
use iroh_gossip::api::{GossipReceiver, GossipSender};
use iroh_gossip::TopicId;
use tracing::{debug, warn};

pub use iroh_base::RelayUrl;

use crate::traits::*;

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
    store: Mutex<HashMap<iroh_blobs::Hash, Bytes>>,
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
    async fn add(&self, data: Bytes) -> Result<iroh_blobs::Hash> {
        let hash = iroh_blobs::Hash::new(&data);
        self.store.lock().unwrap().insert(hash, data);
        Ok(hash)
    }

    async fn get(&self, hash: iroh_blobs::Hash) -> Result<Option<Bytes>> {
        Ok(self.store.lock().unwrap().get(&hash).cloned())
    }

    async fn has(&self, hash: iroh_blobs::Hash) -> bool {
        self.store.lock().unwrap().contains_key(&hash)
    }

    async fn remove(&self, hash: iroh_blobs::Hash) -> Result<bool> {
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
}

impl IrohNetwork {
    /// Create a new IrohNetwork from the given configuration.
    ///
    /// This builds an iroh endpoint, spawns the gossip protocol actor,
    /// sets up the protocol router, and returns a ready-to-use network.
    pub async fn new(config: Config) -> Result<Self> {
        // 1. Build the iroh endpoint.
        let mut builder = Endpoint::empty_builder()
            .secret_key(config.secret_key);

        // Configure relay mode.
        if let Some(relay_url) = &config.relay_url {
            let relay_map = iroh::RelayMap::try_from_iter([relay_url.as_str()])
                .context("invalid relay URL")?;
            builder = builder.relay_mode(RelayMode::Custom(relay_map));
        } else {
            builder = builder.relay_mode(RelayMode::Disabled);
        }

        // Enable mDNS discovery if requested.
        if config.mdns {
            builder = builder
                .address_lookup(iroh::address_lookup::MdnsAddressLookup::builder());
        }

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
        bootstrap: Vec<EndpointId>,
    ) -> Result<(Self::Topic, Self::Events)> {
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
}
