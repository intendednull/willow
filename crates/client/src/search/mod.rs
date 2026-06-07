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

pub mod actor;
pub mod bootstrap;
pub mod config;
pub mod execute;
pub mod handle;
pub mod highlight;
pub mod index;
pub mod query;
pub mod status;
pub mod tokenize;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

pub use bootstrap::{hydrate_index, index_message, reindex_message};
pub use config::{
    clear_all_recents, forget_recent, push_recent, RecentQuery, SearchIndexConfig, MAX_RECENTS,
};
pub use execute::{execute, SearchResult, SearchScope};
pub use handle::SearchIndexHandle;
pub use highlight::{build_excerpt, match_ranges, Excerpt};
pub use index::{IndexableMessage, Posting, SearchIndex};
pub use query::{parse_query, QueryFilters, QueryWarning, SearchQuery};
pub use status::SearchIndexBuildStatus;
pub use tokenize::{token_positions, tokenize};
