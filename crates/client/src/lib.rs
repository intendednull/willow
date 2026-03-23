//! # Willow Client
//!
//! UI-agnostic client library for the Willow P2P chat network.
//! Use this crate to build bots, CLIs, TUIs, or alternative frontends.

pub mod base64;
pub mod emoji;
pub mod events;
pub mod files;
pub mod invite;
pub mod network;
pub mod ops;
pub mod state;
pub mod storage;
pub mod util;

// Re-export key types at crate root for convenience.
pub use events::ClientEvent;
pub use ops::{Op, StampedOp, SyncMessage};
pub use state::{
    ChannelKeyStore, ChatMessage, ChatState, OpLog, ProfileStore, ServerState, UnreadCounts,
};
