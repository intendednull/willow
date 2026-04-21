//! Index build-status signal.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Build
//! behaviour and §Status signal: the UI's `indexing… (local only)`
//! placeholder and the `searching… · {n} matches so far` streaming
//! banner both read this single enum. Exposed read-only to UI
//! consumers; the handle owns the writer.

/// One of four build states the index can be in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SearchIndexBuildStatus {
    /// Idle — index is current, no work in flight.
    #[default]
    Idle,
    /// Preparing to rebuild (cleared old index, not yet scanning).
    Building,
    /// Scanning historical messages. `done` of `total`.
    Indexing {
        /// Messages indexed so far.
        done: u32,
        /// Total messages the current scan aims to cover.
        total: u32,
    },
    /// Rebuild failed — surfaces the spec's `couldn't rebuild the
    /// index. open tweaks to retry.` meta.
    Error(String),
}
