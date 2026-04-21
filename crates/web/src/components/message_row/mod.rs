//! # Message-row sub-components
//!
//! Rendering primitives specific to the chat message row. Each sub-module
//! owns one slice of `docs/specs/2026-04-19-ui-design/message-row.md`:
//!
//! * `day_separator` — day-bucket labels between local-date boundaries.
//!
//! Future sub-modules land here per the Phase 2a plan (mentions, code,
//! jump-to-latest, etc.).

pub mod day_separator;

pub use day_separator::{day_bucket, DayBucket, DaySeparator};
