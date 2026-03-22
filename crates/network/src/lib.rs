//! # Willow Network
//!
//! The P2P networking layer for Willow, built on [libp2p].
//!
//! ## Platform Support
//!
//! - **Native** (Linux/macOS/Windows): TCP + Noise, mDNS LAN discovery, tokio
//! - **WASM** (browser): WebSocket + Noise, relay-based discovery

pub mod behaviour;
pub mod config;
pub mod file_transfer;
pub mod node;

pub use behaviour::WillowBehaviour;
pub use config::NetworkConfig;
pub use node::{NetworkEvent, NetworkNode};
