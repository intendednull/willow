//! # Willow App
//!
//! A Bevy-powered desktop client for the Willow P2P chat network.
//!
//! ## Architecture
//!
//! The app uses Bevy's ECS (Entity Component System) to manage all UI state.
//! Network I/O runs on a separate tokio runtime and communicates with the Bevy
//! world through event channels.
//!
//! ### Plugin structure
//!
//! - [`NetworkPlugin`] — bridges the tokio networking layer to Bevy events.
//! - [`ChatPlugin`] — manages message state, input, and chat rendering.
//! - [`UiPlugin`] — top-level layout: sidebar, channel list, message area.

mod network_bridge;
mod ui;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Willow".to_string(),
                resolution: (1280.0, 720.0).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(network_bridge::NetworkPlugin)
        .add_plugins(ui::UiPlugin)
        .run();
}
