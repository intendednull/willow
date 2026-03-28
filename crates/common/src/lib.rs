//! Shared types for the Willow P2P system.
//!
//! Contains wire protocol types used by both `willow-client` and
//! `willow-worker`, including `WireMessage` (the gossipsub wire format)
//! and worker node types. WASM-compatible.

pub mod wire;
pub mod worker_types;

pub use wire::*;
pub use worker_types::*;
