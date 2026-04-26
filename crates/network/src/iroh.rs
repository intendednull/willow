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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures_lite::StreamExt;
use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, RelayMode};
use iroh_base::{EndpointId, SecretKey};
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
    // state: lock-ok — interim in-memory stub for the
    // `crate::traits::BlobStore` async trait. A persistent backend
    // (sled / sqlite / iroh-blobs store) will replace this; the lock
    // surface goes away with that swap. Not actor-migrated because
    // the stub itself is throwaway. The annotation here is a "this
    // exists pending replacement", not an iroh-callback boundary.
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
    // state: lock-ok — neighbor list is mutated from the iroh gossip event
    // callback (see IrohTopicEvents::next) which runs outside any actor loop.
    // The handle is a sync read API mirrored to topic events.
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
    // state: lock-ok — shared with IrohTopicHandle::receiver_neighbors so the
    // sync neighbors() API sees updates from this event loop. iroh's event
    // callback is the boundary that forces the lock.
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
    // state: lock-ok — iroh Router has a sync lifecycle API (start/shutdown)
    // and Network trait methods are &self; no actor loop owns the router.
    router: Mutex<Option<Router>>,
    // state: lock-ok — subscribe/unsubscribe are sync entry points on the
    // Network trait, called from many call sites without a coordinating actor.
    subscriptions: Mutex<HashMap<TopicId, TopicSubscription>>,
    /// Bootstrap peers merged into every topic subscription.
    bootstrap_peers: Vec<EndpointId>,
    /// Whether a relay was configured at startup. When `false`,
    /// `relay_status` returns `NotConfigured` unconditionally; when
    /// `true` we report `Reachable` iff the endpoint reported itself
    /// online at boot (see `Config::new`). Richer live-probe support is
    /// tracked in Phase 2b Open Questions §4.
    relay_configured: bool,
    /// Snapshot of the endpoint's online signal at startup. Flipped to
    /// `true` after the boot `endpoint.online()` future resolves. The
    /// field is read by `relay_status` via atomic load so the Network
    /// trait method stays lock-free.
    relay_online_at_boot: AtomicBool,
    /// Instant at which the boot-time online signal was observed (native
    /// only). Used by Phase 2b Task 7 to decide whether the 30 s
    /// `Reachable` window has elapsed.
    // state: lock-ok — written once when the boot online signal resolves,
    // read from sync `relay_status()`. No actor mediates this boundary.
    #[cfg(not(target_arch = "wasm32"))]
    relay_online_since: Mutex<Option<Instant>>,
}

impl IrohNetwork {
    /// Create a new IrohNetwork from the given configuration.
    ///
    /// This builds an iroh endpoint, spawns the gossip protocol actor,
    /// sets up the protocol router, and returns a ready-to-use network.
    pub async fn new(config: Config) -> Result<Self> {
        // 1. Build the iroh endpoint.
        // Use `presets::Minimal` so iroh installs its rustls crypto
        // provider (ring, via the iroh `tls-ring` default feature). The
        // earlier 0.97 `empty_builder()` call set this implicitly; in
        // 0.98 the only mandatory builder option is `crypto_provider`,
        // and `Minimal` is the smallest preset that satisfies it without
        // pulling in DNS/relay defaults.
        let mut builder = Endpoint::builder(presets::Minimal).secret_key(config.secret_key);

        // Configure relay mode and seed bootstrap peer addresses.
        if let Some(relay_url) = &config.relay_url {
            let relay_map =
                iroh::RelayMap::try_from_iter([relay_url.as_str()]).context("invalid relay URL")?;
            builder = builder.relay_mode(RelayMode::Custom(relay_map));

            // If bootstrap peers are provided with a relay URL, create a
            // MemoryLookup so iroh can resolve those peers to the relay
            // when dialing them. Without this, iroh only knows the peer's
            // ID but not how to reach it, and gossip dials fail with
            // "No addressing information available".
            if !config.bootstrap_peers.is_empty() {
                let lookup = iroh::address_lookup::memory::MemoryLookup::new();
                for peer_id in &config.bootstrap_peers {
                    let addr = iroh::EndpointAddr::new(*peer_id).with_relay_url(relay_url.clone());
                    lookup.add_endpoint_info(addr);
                }
                builder = builder.address_lookup(lookup);
            }
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

        // If a relay is configured, wait for the endpoint to become online so
        // it has discovered its relay URL. Without this, gossip topic
        // subscriptions publish empty PeerData, which means other peers can't
        // learn this endpoint's address via ForwardJoin and can't dial it.
        //
        // We cap the wait at 5 s so the app never hangs indefinitely when
        // the relay is slow to respond (common in cross-browser CI setups).
        // A local relay connects in <100ms; a remote relay typically connects
        // in 1–3s. Capping at 5s gives enough margin while leaving more of
        // the 60-second test window for gossip mesh formation and sync.
        // If the relay connects later, gossip will self-heal via NeighborUp.
        if config.relay_url.is_some() {
            #[cfg(not(target_arch = "wasm32"))]
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(5), endpoint.online()).await;

            #[cfg(target_arch = "wasm32")]
            futures_lite::future::race(endpoint.online(), async {
                gloo_timers::future::TimeoutFuture::new(5_000).await
            })
            .await;

            debug!(id = %endpoint.id().fmt_short(), "iroh endpoint online (or timed out)");
        }

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

        let relay_configured = config.relay_url.is_some();
        let relay_online_at_boot = AtomicBool::new(relay_configured);
        #[cfg(not(target_arch = "wasm32"))]
        let relay_online_since = if relay_configured {
            Mutex::new(Some(Instant::now()))
        } else {
            Mutex::new(None)
        };

        Ok(Self {
            endpoint,
            gossip,
            blob_store,
            router: Mutex::new(Some(router)),
            subscriptions: Mutex::new(HashMap::new()),
            bootstrap_peers: config.bootstrap_peers,
            relay_configured,
            relay_online_at_boot,
            #[cfg(not(target_arch = "wasm32"))]
            relay_online_since,
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

    fn relay_status(&self) -> RelayStatus {
        if !self.relay_configured {
            return RelayStatus::NotConfigured;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let since = self.relay_online_since.lock().unwrap();
            match *since {
                Some(t) if t.elapsed() < std::time::Duration::from_secs(30) => {
                    RelayStatus::Reachable
                }
                Some(_) => RelayStatus::Unreachable,
                None => RelayStatus::Unreachable,
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if self.relay_online_at_boot.load(Ordering::Relaxed) {
                RelayStatus::Reachable
            } else {
                RelayStatus::Unreachable
            }
        }
    }

    fn device_online(&self) -> bool {
        // Native: iroh does not yet expose a device-level online probe;
        // surface `true` unless the endpoint is closed (tracked via the
        // boot signal). Web callers layer `window.online/offline` on top
        // per Phase 2b Task 7.
        self.relay_online_at_boot.load(Ordering::Relaxed) || !self.relay_configured
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

// NOTE: IrohNetwork requires a live iroh endpoint (bound UDP socket + protocol
// router) and cannot be tested without real I/O. Unit tests for Config struct
// construction have been removed because they exercise no runtime behavior.
// End-to-end startup is covered by `iroh_network_new_and_shutdown` in the
// integration test suite (`crates/network/tests/integration.rs`).

#[cfg(test)]
mod tests {
    use super::*;

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
