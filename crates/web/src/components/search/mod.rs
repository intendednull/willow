//! Local-search UI submodule — `docs/specs/2026-04-19-ui-design/local-search.md`.
//!
//! The Phase 2e work lands incrementally:
//!
//! - Task 9: [`input`], [`scope_chip`].
//! - Task 10: results list + row + recents.
//! - Task 11: surface mount + index hydration.
//! - Task 13: mobile pull-down reveal.

pub mod input;
pub mod scope_chip;

pub use input::SearchInput;
pub use scope_chip::ScopeChip;
