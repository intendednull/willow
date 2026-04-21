//! # Message-row sub-components
//!
//! Rendering primitives specific to the chat message row. Each sub-module
//! owns one slice of `docs/specs/2026-04-19-ui-design/message-row.md`:
//!
//! * `day_separator` — day-bucket labels between local-date boundaries.
//! * `mention` — `@handle` pill rendering (peer / self variants).
//! * `code` — inline backtick pills + fenced triple-backtick blocks.
//!
//! Future sub-modules land here per the Phase 2a plan (jump-to-latest,
//! etc.).

pub mod code;
pub mod day_separator;
pub mod jump_latest;
pub mod mention;

pub use code::{parse_code_segments, CodeSegment, FencedCodeBlock, InlineCodePill};
pub use day_separator::{day_bucket, DayBucket, DaySeparator};
pub use jump_latest::JumpToLatestPill;
pub use mention::MentionPill;
