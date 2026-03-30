//! # Willow Network
//!
//! The P2P networking layer for Willow, built on [iroh].
//!
//! ## Architecture
//!
//! - **[`traits`]** — iroh-shaped trait abstractions (`Network`, `TopicHandle`,
//!   `TopicEvents`, `BlobStore`) that can be swapped for test doubles.
//! - **[`topics`]** — deterministic topic ID generation using BLAKE3.
//! - **[`iroh`]** — production implementation backed by real iroh endpoints.
//! - **[`mem`]** — in-memory test double (behind `test-utils` feature).
//!
//! ## Platform Support
//!
//! Iroh handles native/WASM transport differences internally. The same code
//! compiles for both targets.

pub mod iroh;
pub mod topics;
pub mod traits;

#[cfg(any(test, feature = "test-utils"))]
pub mod mem;

// Re-export key types for convenience.
pub use topics::{
    channel_topic, topic_id, voice_topic, PROFILES_TOPIC, SERVER_OPS_TOPIC, WORKERS_TOPIC,
};
pub use traits::{
    BlobHash, BlobStore, ConnectionEvent, ConnectionEventStream, GossipEvent, GossipMessage,
    Network, TopicEvents, TopicHandle,
};
