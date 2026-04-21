//! Local-search UI submodule — `docs/specs/2026-04-19-ui-design/local-search.md`.
//!
//! The Phase 2e work lands incrementally:
//!
//! - Task 9: [`input`], [`scope_chip`].
//! - Task 10: results list + row + recents.
//! - Task 11: surface mount + index hydration.
//! - Task 13: mobile pull-down reveal.

pub mod input;
pub mod recents;
pub mod results;
pub mod row;
pub mod scope_chip;
pub mod surface;

pub use input::SearchInput;
pub use recents::RecentsList;
pub use results::ResultsList;
pub use row::ResultRow;
pub use scope_chip::ScopeChip;
pub use surface::SearchSurface;
