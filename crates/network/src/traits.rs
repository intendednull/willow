//! # Network Traits
//!
//! Iroh-shaped trait abstractions for gossip, blobs, and network lifecycle.
//! Production code uses [`IrohNetwork`](crate::iroh::IrohNetwork), tests use
//! [`MemNetwork`](crate::mem::MemNetwork).

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use iroh_base::EndpointId;
use iroh_gossip::TopicId;
use serde::{Deserialize, Serialize};

/// A 32-byte content-addressed blob hash (BLAKE3).
///
/// Wrapper around a raw hash so the crate compiles on WASM where
/// `iroh_blobs` is not available. On native, convert to/from
/// `BlobHash` via the named methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlobHash(pub [u8; 32]);

impl BlobHash {
    /// Compute the BLAKE3 hash of data.
    pub fn new(data: &[u8]) -> Self {
        Self(*blake3::hash(data).as_bytes())
    }

    /// Create from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

// ───── Gossip types ─────────────────────────────────────────────────────────

/// An incoming gossip message.
#[derive(Debug, Clone)]
pub struct GossipMessage {
    /// The raw message bytes.
    pub content: Bytes,
    /// The peer that sent this message.
    pub sender: EndpointId,
}

/// Events received from a gossip topic subscription.
#[derive(Debug, Clone)]
pub enum GossipEvent {
    /// A message was received from a peer.
    Received(GossipMessage),
    /// A new peer joined the topic.
    NeighborUp(EndpointId),
    /// A peer left the topic.
    NeighborDown(EndpointId),
}

// ───── TopicHandle ──────────────────────────────────────────────────────────

/// A handle to a single gossip topic subscription.
///
/// Mirrors `iroh_gossip::GossipSender` but as a trait for testability.
#[async_trait]
pub trait TopicHandle: Send + Sync + Clone {
    /// Broadcast data to all peers subscribed to this topic.
    async fn broadcast(&self, data: Bytes) -> Result<()>;

    /// Broadcast data only to direct neighbors (not forwarded further).
    async fn broadcast_neighbors(&self, data: Bytes) -> Result<()>;

    /// Return the current set of neighbor peers for this topic.
    fn neighbors(&self) -> Vec<EndpointId>;
}

// ───── TopicEvents ──────────────────────────────────────────────────────────

/// A stream of incoming gossip events for a topic.
///
/// Mirrors `iroh_gossip::GossipReceiver` but as a trait for testability.
#[async_trait]
pub trait TopicEvents: Send {
    /// Wait for the next event. Returns `None` when the subscription ends.
    async fn next(&mut self) -> Option<Result<GossipEvent>>;

    /// Wait until this subscription has joined the topic mesh.
    async fn joined(&mut self) -> Result<()>;
}

// ───── BlobStore ────────────────────────────────────────────────────────────

/// Content-addressed blob storage.
///
/// Mirrors `iroh_blobs` operations but as a trait for testability.
#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Add data to the store, returning its content hash.
    async fn add(&self, data: Bytes) -> Result<BlobHash>;

    /// Retrieve data by hash. Returns `None` if not found.
    async fn get(&self, hash: BlobHash) -> Result<Option<Bytes>>;

    /// Check whether a blob exists in the store.
    async fn has(&self, hash: BlobHash) -> bool;

    /// Remove a blob from the store. Returns `true` if it existed.
    async fn remove(&self, hash: BlobHash) -> Result<bool>;

    /// Current store size in bytes. Returns `None` if unsupported.
    async fn store_size(&self) -> Option<u64>;
}

// ───── Connection events ────────────────────────────────────────────────────

/// Network connectivity events.
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// Connected to a relay server.
    RelayConnected,
    /// Disconnected from the relay server.
    RelayDisconnected,
    /// A direct QUIC connection was established to a peer.
    DirectConnected(EndpointId),
    /// A direct QUIC connection to a peer was lost.
    DirectDisconnected(EndpointId),
}

/// A stream of connection events.
pub type ConnectionEventStream =
    std::pin::Pin<Box<dyn futures_lite::Stream<Item = ConnectionEvent> + Send>>;

// ───── Network ──────────────────────────────────────────────────────────────

/// Top-level network handle. Assembled once, passed to client/workers.
///
/// Production: [`IrohNetwork`](crate::iroh::IrohNetwork).
/// Tests: [`MemNetwork`](crate::mem::MemNetwork).
#[async_trait]
pub trait Network: Send + Sync + 'static {
    /// The topic handle type returned by [`subscribe`](Network::subscribe).
    type Topic: TopicHandle;
    /// The topic events type returned by [`subscribe`](Network::subscribe).
    type Events: TopicEvents;

    /// This node's public identity.
    fn id(&self) -> EndpointId;

    /// Subscribe to a gossip topic with optional bootstrap peers.
    async fn subscribe(
        &self,
        topic: TopicId,
        bootstrap: Vec<EndpointId>,
    ) -> Result<(Self::Topic, Self::Events)>;

    /// Unsubscribe from a topic, leaving the gossip mesh.
    async fn unsubscribe(&self, topic: TopicId) -> Result<()>;

    /// Access the blob store.
    fn blobs(&self) -> &dyn BlobStore;

    /// Stream of connectivity events (relay up/down, peer connects).
    async fn connection_events(&self) -> ConnectionEventStream;

    /// Gracefully shut down the network.
    async fn shutdown(&self) -> Result<()>;
}
