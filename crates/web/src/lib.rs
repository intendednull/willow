//! willow-web — Leptos web UI for the Willow P2P chat network.
//!
//! This crate is primarily a binary (`src/main.rs`) but also exposes a
//! library surface so integration tests in `tests/browser.rs` can
//! mount components directly. Only items reachable from tests need to
//! be `pub`.

#![allow(dead_code)]

pub mod app;
pub mod audio;
pub mod components;
pub mod event_processing;
pub mod handlers;
pub mod icons;
pub mod keybindings;
pub mod palette_recents;
pub mod state;
pub mod state_bridge;
pub mod trust_store;
pub mod util;
pub mod voice;
