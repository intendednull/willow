//! Shared types for the Willow worker node system.
//!
//! This crate is lightweight (no tokio, no networking) so both
//! `willow-client` and `willow-worker` can depend on it, including
//! on WASM targets.

pub mod worker_types;

pub use worker_types::*;
