//! Profile-card wiring: event bus, controller, nickname store, copy.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
//! §Event-bus API + §Private nickname + §Copy.

pub mod bus;
pub mod controller;
pub mod copy;
pub mod crest;
pub mod nickname_store;

pub use bus::{close_profile, open_profile, PROFILE_CLOSE_EVENT, PROFILE_OPEN_EVENT};
pub use controller::{use_profile_controller, ProfileState};
pub use crest::{crest_defaults, CrestBanner};
pub use nickname_store::WebNicknameStore;
