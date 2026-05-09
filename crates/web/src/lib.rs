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
pub mod notifications;
pub mod palette_recents;
pub mod profile;
pub mod reaction_recency;
pub mod service_worker_bridge;
pub mod state;
pub mod state_bridge;
#[cfg(feature = "test-hooks")]
pub mod test_hooks;
pub mod trust_store;
pub mod upload_state;
pub mod util;
pub mod voice;
pub mod voice_note_player;
