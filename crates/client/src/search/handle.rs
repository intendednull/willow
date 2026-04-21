//! Top-level search handle.
//!
//! **PRIVACY CONTRACT.** Per
//! `docs/specs/2026-04-19-ui-design/local-search.md` §Privacy: no query
//! string, match count, scope selection, or recents entry is ever
//! emitted to any network path or log. All `tracing::*` macros in this
//! module are forbidden. If you need to debug, add a local assertion —
//! never a log line that could leak search state.
//!
//! [`SearchIndexHandle`] is the clonable, thread-safe entry-point the
//! UI and the client wire use. It wraps the non-`Clone` [`SearchIndex`]
//! in an `Arc<Mutex<_>>` and exposes the verbs the UI needs: `insert`
//! live messages, `rebuild` from a batch, `query` against a scope, plus
//! config + recents + status accessors.

use std::sync::Arc;

use parking_lot::Mutex;

use super::config::{RecentQuery, SearchIndexConfig};
use super::execute::{execute, SearchResult, SearchScope};
use super::index::{IndexableMessage, SearchIndex};
use super::query::SearchQuery;
use super::status::SearchIndexBuildStatus;

/// Clonable, `Send + Sync` entry-point to the local search index.
#[derive(Clone)]
pub struct SearchIndexHandle {
    index: Arc<Mutex<SearchIndex>>,
    config: Arc<Mutex<SearchIndexConfig>>,
    recents: Arc<Mutex<Vec<RecentQuery>>>,
    status: Arc<Mutex<SearchIndexBuildStatus>>,
    /// `true` in production (config + recents writes hit
    /// `crate::storage`); `false` under test (`new_in_memory()`) to
    /// keep unit tests from stomping the shared data directory.
    persist: bool,
}

impl SearchIndexHandle {
    /// Build a handle, loading config + recents from persistent
    /// storage (via `crate::storage`). First-run callers get defaults.
    pub fn new() -> Self {
        let config = crate::storage::load_search_config().unwrap_or_default();
        let recents = crate::storage::load_search_recents();
        Self {
            index: Arc::new(Mutex::new(SearchIndex::new())),
            config: Arc::new(Mutex::new(config)),
            recents: Arc::new(Mutex::new(recents)),
            status: Arc::new(Mutex::new(SearchIndexBuildStatus::Idle)),
            persist: true,
        }
    }

    /// Build a handle without touching `crate::storage`. Used by tests
    /// to avoid polluting the shared native data dir. Config + recents
    /// writes stay in memory for the lifetime of the handle.
    pub fn new_in_memory() -> Self {
        Self {
            index: Arc::new(Mutex::new(SearchIndex::new())),
            config: Arc::new(Mutex::new(SearchIndexConfig::default())),
            recents: Arc::new(Mutex::new(Vec::new())),
            status: Arc::new(Mutex::new(SearchIndexBuildStatus::Idle)),
            persist: false,
        }
    }

    /// Insert one live message. Guarded on `enabled` + per-grove opt-out.
    pub fn insert(&self, m: IndexableMessage) {
        {
            let cfg = self.config.lock();
            if !cfg.enabled {
                return;
            }
            if let Some(gid) = &m.grove_id {
                if cfg.per_grove_enabled.get(gid).copied() == Some(false) {
                    return;
                }
            }
        }
        self.index.lock().insert(m);
    }

    /// Drop the current index and rebuild from `msgs`. Updates the
    /// status signal so the UI can drive the streaming banner.
    pub fn rebuild(&self, msgs: Vec<IndexableMessage>) {
        let total = msgs.len() as u32;
        *self.status.lock() = SearchIndexBuildStatus::Building;
        let mut index = self.index.lock();
        *index = SearchIndex::new();
        for (i, m) in msgs.into_iter().enumerate() {
            let skip = {
                let cfg = self.config.lock();
                if !cfg.enabled {
                    true
                } else if let Some(gid) = &m.grove_id {
                    cfg.per_grove_enabled.get(gid).copied() == Some(false)
                } else {
                    false
                }
            };
            if skip {
                continue;
            }
            index.insert(m);
            *self.status.lock() = SearchIndexBuildStatus::Indexing {
                done: (i + 1) as u32,
                total,
            };
        }
        drop(index);
        *self.status.lock() = SearchIndexBuildStatus::Idle;
    }

    /// Run a query against the index under `scope`.
    ///
    /// Returns hits in timestamp-desc order. Caller is expected to
    /// pre-parse with [`super::parse_query`].
    pub fn query(&self, q: &SearchQuery, scope: &SearchScope) -> Vec<SearchResult> {
        let index = self.index.lock();
        execute(&index, q, scope)
    }

    /// Remove one message by id. Called from the incremental-update
    /// path when a message is deleted.
    pub fn remove_message(&self, id: &str) {
        self.index.lock().remove_message(id);
    }

    /// Remove everything in a channel — channel deletion / per-grove
    /// opt-out pipes here.
    pub fn remove_channel(&self, cid: &str) {
        self.index.lock().remove_channel(cid);
    }

    /// Remove everything in a grove — per-grove opt-out pipes here
    /// when the toggle flips off.
    pub fn remove_grove(&self, gid: &str) {
        self.index.lock().remove_grove(gid);
    }

    /// Current config snapshot.
    pub fn config(&self) -> SearchIndexConfig {
        self.config.lock().clone()
    }

    /// Replace the config and persist. Called from settings-tweaks.md.
    pub fn set_config(&self, c: SearchIndexConfig) {
        *self.config.lock() = c.clone();
        if self.persist {
            crate::storage::save_search_config(&c);
        }
    }

    /// Current recents snapshot.
    pub fn recents(&self) -> Vec<RecentQuery> {
        self.recents.lock().clone()
    }

    /// Push a new recent; guarded on `config.remember_recents`.
    pub fn push_recent(&self, r: RecentQuery) {
        if !self.config.lock().remember_recents {
            return;
        }
        let mut list = self.recents.lock();
        super::config::push_recent(&mut list, r);
        if self.persist {
            crate::storage::save_search_recents(&list);
        }
    }

    /// Forget one recent by its text.
    pub fn forget_recent(&self, text: &str) {
        let mut list = self.recents.lock();
        super::config::forget_recent(&mut list, text);
        if self.persist {
            crate::storage::save_search_recents(&list);
        }
    }

    /// Clear all recents.
    pub fn clear_all_recents(&self) {
        let mut list = self.recents.lock();
        super::config::clear_all_recents(&mut list);
        if self.persist {
            crate::storage::save_search_recents(&list);
        }
    }

    /// Current build status.
    pub fn status(&self) -> SearchIndexBuildStatus {
        self.status.lock().clone()
    }

    /// How many messages are indexed right now.
    pub fn message_count(&self) -> usize {
        self.index.lock().message_count()
    }
}

impl Default for SearchIndexHandle {
    fn default() -> Self {
        Self::new()
    }
}
