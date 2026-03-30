//! # Stub Network
//!
//! A minimal no-op [`Network`] implementation for platforms where the full
//! iroh networking stack isn't available (e.g. WASM until iroh's WebTransport
//! support is ready).

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use iroh_base::EndpointId;
use willow_identity::Identity;

use crate::traits::BlobHash;
use crate::traits::*;
use iroh_gossip::TopicId;

/// A no-op network that doesn't send or receive anything.
///
/// Used as a placeholder on WASM until iroh's WASM transport is integrated.
pub struct StubNetwork {
    id: EndpointId,
}

impl StubNetwork {
    /// Create a new stub network with a random identity.
    pub fn new() -> Self {
        Self {
            id: Identity::generate().endpoint_id(),
        }
    }
}

impl Default for StubNetwork {
    fn default() -> Self {
        Self::new()
    }
}

/// A no-op topic handle.
#[derive(Clone)]
pub struct StubTopicHandle;

#[async_trait]
impl TopicHandle for StubTopicHandle {
    async fn broadcast(&self, _data: Bytes) -> Result<()> {
        Ok(())
    }
    async fn broadcast_neighbors(&self, _data: Bytes) -> Result<()> {
        Ok(())
    }
    fn neighbors(&self) -> Vec<EndpointId> {
        vec![]
    }
}

/// A no-op topic events stream that never yields.
pub struct StubTopicEvents;

#[async_trait]
impl TopicEvents for StubTopicEvents {
    async fn next(&mut self) -> Option<Result<GossipEvent>> {
        // Never yields — pending forever.
        futures_lite::future::pending().await
    }
    async fn joined(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A no-op blob store.
struct StubBlobStore;

#[async_trait]
impl BlobStore for StubBlobStore {
    async fn add(&self, data: Bytes) -> Result<crate::BlobHash> {
        Ok(crate::BlobHash::new(&data))
    }
    async fn get(&self, _hash: crate::BlobHash) -> Result<Option<Bytes>> {
        Ok(None)
    }
    async fn has(&self, _hash: crate::BlobHash) -> bool {
        false
    }
    async fn remove(&self, _hash: crate::BlobHash) -> Result<bool> {
        Ok(false)
    }
    async fn store_size(&self) -> Option<u64> {
        Some(0)
    }
}

#[async_trait]
impl Network for StubNetwork {
    type Topic = StubTopicHandle;
    type Events = StubTopicEvents;

    fn id(&self) -> EndpointId {
        self.id
    }

    async fn subscribe(
        &self,
        _topic: TopicId,
        _bootstrap: Vec<EndpointId>,
    ) -> Result<(Self::Topic, Self::Events)> {
        Ok((StubTopicHandle, StubTopicEvents))
    }

    async fn unsubscribe(&self, _topic: TopicId) -> Result<()> {
        Ok(())
    }

    fn blobs(&self) -> &dyn BlobStore {
        &StubBlobStore
    }

    async fn connection_events(&self) -> ConnectionEventStream {
        Box::pin(futures_lite::stream::pending())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
