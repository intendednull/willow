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

pub mod file_manager;
pub mod network_bridge;
pub mod storage;
pub mod ui;

#[cfg(test)]
mod tests;
