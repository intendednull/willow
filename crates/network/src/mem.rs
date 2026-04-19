//! # In-Memory Network Test Double
//!
//! [`MemNetwork`] and [`MemHub`] provide an in-process gossip simulation
//! for testing. No real connections, no network I/O — messages are delivered
//! via `tokio::sync::broadcast` channels.
//!
//! ## Usage
//!
//! ```ignore
//! let hub = MemHub::new();
//! let net_a = MemNetwork::new(&hub);
//! let net_b = MemNetwork::new(&hub);
//!
//! let topic = topic_id("test");
//! let (sender_a, _) = net_a.subscribe(topic, vec![]).await?;
//! let (_, mut events_b) = net_b.subscribe(topic, vec![]).await?;
//!
//! sender_a.broadcast(Bytes::from("hello")).await?;
//! // events_b.next() will yield the message
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use iroh_base::EndpointId;
use iroh_gossip::TopicId;
use tokio::sync::broadcast;
use willow_identity::Identity;

use crate::traits::*;

// ───── MemHub ───────────────────────────────────────────────────────────────

/// Internal message type sent through broadcast channels.
#[derive(Debug, Clone)]
struct HubMessage {
    sender: EndpointId,
    data: Bytes,
}

/// Subscriber tracking for neighbor events.
#[derive(Debug)]
struct TopicState {
    sender: broadcast::Sender<HubMessage>,
    /// Subscribers currently active on this topic.
    subscribers: Vec<EndpointId>,
    /// Channel for neighbor up/down events.
    neighbor_tx: broadcast::Sender<GossipEvent>,
}

/// Shared in-process gossip mesh for testing.
///
/// Each `MemHub` instance is independent — no cross-hub interference.
/// Create one per test to ensure isolation.
pub struct MemHub {
    topics: Mutex<HashMap<TopicId, TopicState>>,
}

impl MemHub {
    /// Create a new isolated hub.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            topics: Mutex::new(HashMap::new()),
        })
    }

    fn subscribe(
        &self,
        topic: TopicId,
        subscriber: EndpointId,
    ) -> (
        broadcast::Receiver<HubMessage>,
        broadcast::Receiver<GossipEvent>,
    ) {
        let mut topics = self.topics.lock().unwrap();
        let state = topics.entry(topic).or_insert_with(|| {
            let (sender, _) = broadcast::channel(256);
            let (neighbor_tx, _) = broadcast::channel(64);
            TopicState {
                sender,
                subscribers: Vec::new(),
                neighbor_tx,
            }
        });

        // Notify existing subscribers about the new neighbor.
        for &existing in &state.subscribers {
            if existing != subscriber {
                let _ = state.neighbor_tx.send(GossipEvent::NeighborUp(subscriber));
            }
        }

        let msg_rx = state.sender.subscribe();
        let neighbor_rx = state.neighbor_tx.subscribe();

        // Send NeighborUp for existing subscribers to the new peer.
        // We'll deliver these through the neighbor channel.
        let existing_peers: Vec<EndpointId> = state.subscribers.clone();
        state.subscribers.push(subscriber);

        // Send NeighborUp events for existing peers to the new subscriber
        // through the neighbor channel (they'll be interleaved with future events).
        for peer in existing_peers {
            let _ = state.neighbor_tx.send(GossipEvent::NeighborUp(peer));
        }

        (msg_rx, neighbor_rx)
    }

    fn unsubscribe(&self, topic: TopicId, subscriber: EndpointId) {
        let mut topics = self.topics.lock().unwrap();
        if let Some(state) = topics.get_mut(&topic) {
            state.subscribers.retain(|&id| id != subscriber);
            let _ = state
                .neighbor_tx
                .send(GossipEvent::NeighborDown(subscriber));
        }
    }

    fn get_sender(&self, topic: &TopicId) -> Option<broadcast::Sender<HubMessage>> {
        let topics = self.topics.lock().unwrap();
        topics.get(topic).map(|s| s.sender.clone())
    }

    fn get_subscribers(&self, topic: &TopicId) -> Vec<EndpointId> {
        let topics = self.topics.lock().unwrap();
        topics
            .get(topic)
            .map(|s| s.subscribers.clone())
            .unwrap_or_default()
    }
}

impl Default for MemHub {
    fn default() -> Self {
        Self {
            topics: Mutex::new(HashMap::new()),
        }
    }
}

// ───── MemBlobStore ─────────────────────────────────────────────────────────

/// In-memory blob store for tests.
pub struct MemBlobStore {
    store: Mutex<HashMap<crate::BlobHash, Bytes>>,
}

impl MemBlobStore {
    fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl BlobStore for MemBlobStore {
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

// ───── MemTopicHandle ───────────────────────────────────────────────────────

/// Topic handle backed by a broadcast channel.
#[derive(Clone)]
pub struct MemTopicHandle {
    id: EndpointId,
    topic: TopicId,
    hub: Arc<MemHub>,
}

#[async_trait]
impl TopicHandle for MemTopicHandle {
    async fn broadcast(&self, data: Bytes) -> Result<()> {
        if let Some(sender) = self.hub.get_sender(&self.topic) {
            let _ = sender.send(HubMessage {
                sender: self.id,
                data,
            });
        }
        Ok(())
    }

    /// Broadcast to all topic subscribers.
    ///
    /// # MemNetwork divergence from real iroh
    ///
    /// In production iroh, `broadcast_neighbors` delivers only to direct
    /// gossip-mesh neighbors (the immediate peers this node is connected to).
    /// `MemNetwork` has no concept of per-peer connections: the `HubMessage`
    /// is placed on the shared broadcast channel and received by every
    /// subscriber on the topic, so this method behaves identically to
    /// [`broadcast`](Self::broadcast).
    ///
    /// Tests that specifically rely on neighbor-scoped delivery will produce
    /// false-positive results here and should use the real `IrohNetwork` or
    /// a custom test double that models topology explicitly.
    async fn broadcast_neighbors(&self, data: Bytes) -> Result<()> {
        if let Some(sender) = self.hub.get_sender(&self.topic) {
            let _ = sender.send(HubMessage {
                sender: self.id,
                data,
            });
        }
        Ok(())
    }

    fn neighbors(&self) -> Vec<EndpointId> {
        self.hub
            .get_subscribers(&self.topic)
            .into_iter()
            .filter(|&id| id != self.id)
            .collect()
    }
}

// ───── MemTopicEvents ───────────────────────────────────────────────────────

/// Topic event stream backed by broadcast channels.
pub struct MemTopicEvents {
    id: EndpointId,
    msg_rx: broadcast::Receiver<HubMessage>,
    neighbor_rx: broadcast::Receiver<GossipEvent>,
    joined: bool,
}

#[async_trait]
impl TopicEvents for MemTopicEvents {
    async fn next(&mut self) -> Option<Result<GossipEvent>> {
        loop {
            tokio::select! {
                result = self.msg_rx.recv() => {
                    match result {
                        Ok(msg) if msg.sender != self.id => {
                            return Some(Ok(GossipEvent::Received(GossipMessage {
                                content: msg.data,
                                sender: msg.sender,
                            })));
                        }
                        Ok(_) => continue, // Skip self-messages
                        Err(broadcast::error::RecvError::Closed) => return None,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    }
                }
                result = self.neighbor_rx.recv() => {
                    match result {
                        Ok(event) => {
                            // Filter out events about ourselves
                            match &event {
                                GossipEvent::NeighborUp(id) | GossipEvent::NeighborDown(id) if *id == self.id => continue,
                                _ => return Some(Ok(event)),
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => return None,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    }
                }
            }
        }
    }

    async fn joined(&mut self) -> Result<()> {
        self.joined = true;
        Ok(())
    }
}

// ───── MemNetwork ───────────────────────────────────────────────────────────

/// In-memory network for tests. No real connections, no network I/O.
///
/// All delivery happens in-process via broadcast channels through a
/// shared [`MemHub`].
pub struct MemNetwork {
    id: EndpointId,
    identity: Identity,
    hub: Arc<MemHub>,
    blobs: MemBlobStore,
    subscriptions: Mutex<Vec<TopicId>>,
}

impl MemNetwork {
    /// Create a new test network connected to the given hub.
    pub fn new(hub: &Arc<MemHub>) -> Self {
        let identity = Identity::generate();
        Self {
            id: identity.endpoint_id(),
            identity,
            hub: Arc::clone(hub),
            blobs: MemBlobStore::new(),
            subscriptions: Mutex::new(Vec::new()),
        }
    }

    /// Create a new test network with a specific identity.
    pub fn with_identity(hub: &Arc<MemHub>, identity: Identity) -> Self {
        Self {
            id: identity.endpoint_id(),
            identity,
            hub: Arc::clone(hub),
            blobs: MemBlobStore::new(),
            subscriptions: Mutex::new(Vec::new()),
        }
    }

    /// Access the identity used by this network.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
}

impl Drop for MemNetwork {
    fn drop(&mut self) {
        let subs = self.subscriptions.lock().unwrap().clone();
        for topic in subs {
            self.hub.unsubscribe(topic, self.id);
        }
    }
}

#[async_trait]
impl Network for MemNetwork {
    type Topic = MemTopicHandle;
    type Events = MemTopicEvents;

    fn id(&self) -> EndpointId {
        self.id
    }

    async fn subscribe(
        &self,
        topic: TopicId,
        _bootstrap: Vec<EndpointId>,
    ) -> Result<(Self::Topic, Self::Events)> {
        let (msg_rx, neighbor_rx) = self.hub.subscribe(topic, self.id);
        self.subscriptions.lock().unwrap().push(topic);

        let handle = MemTopicHandle {
            id: self.id,
            topic,
            hub: Arc::clone(&self.hub),
        };

        let events = MemTopicEvents {
            id: self.id,
            msg_rx,
            neighbor_rx,
            joined: false,
        };

        Ok((handle, events))
    }

    async fn unsubscribe(&self, topic: TopicId) -> Result<()> {
        self.hub.unsubscribe(topic, self.id);
        self.subscriptions.lock().unwrap().retain(|&t| t != topic);
        Ok(())
    }

    fn blobs(&self) -> &dyn BlobStore {
        &self.blobs
    }

    async fn shutdown(&self) -> Result<()> {
        let subs = self.subscriptions.lock().unwrap().clone();
        for topic in subs {
            self.hub.unsubscribe(topic, self.id);
        }
        self.subscriptions.lock().unwrap().clear();
        Ok(())
    }
}

// ───── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topics::topic_id;

    /// Drain neighbor events and return the next Received event.
    async fn next_received(events: &mut MemTopicEvents) -> GossipMessage {
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.next())
                .await
                .expect("timed out waiting for event")
                .expect("stream ended")
                .expect("event error");
            if let GossipEvent::Received(msg) = event {
                return msg;
            }
        }
    }

    #[tokio::test]
    async fn broadcast_delivers_to_other_not_self() {
        let hub = MemHub::new();
        let net_a = MemNetwork::new(&hub);
        let net_b = MemNetwork::new(&hub);

        let topic = topic_id("test");
        let (sender_a, _events_a) = net_a.subscribe(topic, vec![]).await.unwrap();
        let (_sender_b, mut events_b) = net_b.subscribe(topic, vec![]).await.unwrap();

        sender_a.broadcast(Bytes::from("hello")).await.unwrap();

        let msg = next_received(&mut events_b).await;
        assert_eq!(msg.content.as_ref(), b"hello");
        assert_eq!(msg.sender, net_a.id());
    }

    #[tokio::test]
    async fn topic_isolation() {
        let hub = MemHub::new();
        let net_a = MemNetwork::new(&hub);
        let net_b = MemNetwork::new(&hub);

        let topic_x = topic_id("topic-x");
        let topic_y = topic_id("topic-y");

        let (sender_a, _) = net_a.subscribe(topic_x, vec![]).await.unwrap();
        let (_, mut events_b) = net_b.subscribe(topic_y, vec![]).await.unwrap();

        sender_a
            .broadcast(Bytes::from("wrong topic"))
            .await
            .unwrap();

        // events_b should not receive anything — timeout expected.
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), events_b.next()).await;

        assert!(result.is_err(), "should have timed out");
    }

    #[tokio::test]
    async fn neighbor_up_fires_when_peer_subscribes() {
        let hub = MemHub::new();
        let net_a = MemNetwork::new(&hub);
        let net_b = MemNetwork::new(&hub);

        let topic = topic_id("test");
        let (_, mut events_a) = net_a.subscribe(topic, vec![]).await.unwrap();

        // B subscribes — A should see NeighborUp
        let _ = net_b.subscribe(topic, vec![]).await.unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events_a.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        match event {
            GossipEvent::NeighborUp(id) => assert_eq!(id, net_b.id()),
            other => panic!("expected NeighborUp, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn neighbor_down_fires_on_unsubscribe() {
        let hub = MemHub::new();
        let net_a = MemNetwork::new(&hub);
        let net_b = MemNetwork::new(&hub);

        let topic = topic_id("test");
        let (_, mut events_a) = net_a.subscribe(topic, vec![]).await.unwrap();
        let _ = net_b.subscribe(topic, vec![]).await.unwrap();

        // Drain the NeighborUp event
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), events_a.next()).await;

        // B unsubscribes
        net_b.unsubscribe(topic).await.unwrap();

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events_a.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        match event {
            GossipEvent::NeighborDown(id) => assert_eq!(id, net_b.id()),
            other => panic!("expected NeighborDown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn blob_store_add_get_has_remove() {
        let hub = MemHub::new();
        let net = MemNetwork::new(&hub);
        let blobs = net.blobs();

        let data = Bytes::from("test blob data");
        let hash = blobs.add(data.clone()).await.unwrap();

        assert!(blobs.has(hash).await);
        assert_eq!(blobs.get(hash).await.unwrap(), Some(data));

        let removed = blobs.remove(hash).await.unwrap();
        assert!(removed);
        assert!(!blobs.has(hash).await);
        assert_eq!(blobs.get(hash).await.unwrap(), None);
    }

    #[tokio::test]
    async fn blob_store_size() {
        let hub = MemHub::new();
        let net = MemNetwork::new(&hub);
        let blobs = net.blobs();

        assert_eq!(blobs.store_size().await, Some(0));

        blobs.add(Bytes::from("hello")).await.unwrap(); // 5 bytes
        blobs.add(Bytes::from("world!")).await.unwrap(); // 6 bytes

        assert_eq!(blobs.store_size().await, Some(11));
    }

    #[tokio::test]
    async fn network_id_matches_identity() {
        let hub = MemHub::new();
        let net = MemNetwork::new(&hub);
        assert_eq!(net.id(), net.identity().endpoint_id());
    }

    #[tokio::test]
    async fn neighbors_list() {
        let hub = MemHub::new();
        let net_a = MemNetwork::new(&hub);
        let net_b = MemNetwork::new(&hub);

        let topic = topic_id("test");
        let (sender_a, _) = net_a.subscribe(topic, vec![]).await.unwrap();
        let _ = net_b.subscribe(topic, vec![]).await.unwrap();

        let neighbors = sender_a.neighbors();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0], net_b.id());
    }
}
