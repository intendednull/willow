//! Local search primitives — docs/specs/2026-04-19-ui-design/local-search.md.
//!
//! On-device, encrypted-at-rest search index that lets the UI query the
//! local corpus without ever talking to the relay. The module is dual-
//! target (native + wasm32) and consumes already-decrypted
//! [`DisplayMessage`][crate::state::DisplayMessage]s — no new crypto,
//! no new wire types, no new `EventKind`.
//!
//! Sub-modules land incrementally as the plan's tasks are ticked off.
//! Today (Task 1): [`query`] only.

pub mod query;
pub mod tokenize;

#[cfg(test)]
mod tests;

pub use query::{parse_query, QueryFilters, QueryWarning, SearchQuery};
pub use tokenize::{token_positions, tokenize};
