//! # Willow App
//!
//! A Bevy-powered desktop client for the Willow P2P chat network.
//!
//! ## Architecture
//!
//! The app uses Bevy's ECS (Entity Component System) to manage all UI state.
//! Network I/O runs on a separate tokio runtime and communicates with the Bevy
//! world through message channels.
//!
//! ### Plugin structure
//!
//! - [`NetworkPlugin`](network_bridge::NetworkPlugin) — bridges the tokio
//!   networking layer to Bevy messages.
//! - [`UiPlugin`](ui::UiPlugin) — top-level layout: sidebar, channel list,
//!   message area, input, and chat rendering.

pub mod clipboard;
pub mod network_bridge;
pub mod notify;
pub mod text_edit;
pub mod theme;
pub mod ui;

// Re-export from willow-client for backward compatibility.
pub use willow_client::base64;
pub use willow_client::emoji;
pub use willow_client::files as file_manager;
pub use willow_client::invite;
pub use willow_client::ops as server_sync;
pub use willow_client::storage;

#[cfg(test)]
mod tests;
