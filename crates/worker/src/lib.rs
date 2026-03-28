//! Shared worker node library.
//!
//! Provides the [`WorkerRole`] trait, actor-based runtime, and common
//! peer lifecycle (identity, networking, heartbeat, sync) for all
//! worker node binaries.

pub mod actors;
pub mod config;
pub mod identity;
pub mod types;

pub use config::WorkerConfig;
pub use types::*;
