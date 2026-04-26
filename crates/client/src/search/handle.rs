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
//! UI and the client wire use. It wraps an [`Addr`] to a
//! [`SearchActor`] (see [`super::actor`]). The actor owns the
//! inverted index, config, recents, and build status as one unit;
//! cross-field updates are atomic by mailbox serialization. See
//! `docs/specs/2026-04-26-state-management-model-design.md` § State Management.

use willow_actor::{Addr, SystemHandle};

use super::actor::{
    ClearAllRecents, ForgetRecent, GetConfig, GetRecents, GetStatus, Insert, MessageCount,
    PushRecent, Query, Rebuild, RemoveChannel, RemoveGrove, RemoveMessage, SearchActor, SetConfig,
};
use super::config::{RecentQuery, SearchIndexConfig};
use super::execute::{SearchResult, SearchScope};
use super::index::IndexableMessage;
use super::query::SearchQuery;
use super::status::SearchIndexBuildStatus;

/// Clonable, `Send + Sync` entry-point to the local search index.
#[derive(Clone)]
pub struct SearchIndexHandle {
    addr: Addr<SearchActor>,
}

impl SearchIndexHandle {
    /// Build a handle, loading config + recents from persistent
    /// storage (via `crate::storage`). First-run callers get defaults.
    pub fn new(system: &SystemHandle) -> Self {
        let config = crate::storage::load_search_config().unwrap_or_default();
        let recents = crate::storage::load_search_recents();
        let addr = system.spawn(SearchActor::new(config, recents, true));
        Self { addr }
    }

    /// Build a handle without touching `crate::storage`. Used by tests
    /// to avoid polluting the shared native data dir. Config + recents
    /// writes stay in memory for the lifetime of the handle.
    pub fn new_in_memory(system: &SystemHandle) -> Self {
        let addr = system.spawn(SearchActor::new(
            SearchIndexConfig::default(),
            Vec::new(),
            false,
        ));
        Self { addr }
    }

    /// Insert one live message. Guarded on `enabled` + per-grove opt-out.
    ///
    /// Fire-and-forget via `do_send`; ordering with subsequent reads is
    /// preserved by the actor's FIFO mailbox on the same `Addr`.
    ///
    /// Backpressure: under sustained burst (e.g. 10k+ live messages
    /// arriving while a long rebuild is mid-flight) `do_send` may drop
    /// the message when the actor mailbox is full. This is recoverable —
    /// the rebuild Effect in `crates/web/src/app.rs` re-runs on every
    /// `messages_sig` change and reseeds the index from scratch, so any
    /// dropped insert is picked up on the next rebuild trigger. Callers
    /// don't need to retry.
    pub fn insert(&self, m: IndexableMessage) {
        self.addr.do_send(Insert(m)).ok();
    }

    /// Drop the current index and rebuild from `msgs`. Returns once the
    /// rebuild is complete and `status` has settled back to `Idle`.
    pub async fn rebuild(&self, msgs: Vec<IndexableMessage>) {
        self.addr.ask(Rebuild(msgs)).await.ok();
    }

    /// Run a query against the index under `scope`.
    ///
    /// Returns hits in timestamp-desc order. Caller is expected to
    /// pre-parse with [`super::parse_query`].
    pub async fn query(&self, q: &SearchQuery, scope: &SearchScope) -> Vec<SearchResult> {
        self.addr
            .ask(Query {
                q: q.clone(),
                scope: scope.clone(),
            })
            .await
            .unwrap_or_default()
    }

    /// Remove one message by id. Called from the incremental-update
    /// path when a message is deleted.
    pub fn remove_message(&self, id: &str) {
        self.addr.do_send(RemoveMessage(id.to_string())).ok();
    }

    /// Remove everything in a channel — channel deletion / per-grove
    /// opt-out pipes here.
    pub fn remove_channel(&self, cid: &str) {
        self.addr.do_send(RemoveChannel(cid.to_string())).ok();
    }

    /// Remove everything in a grove — per-grove opt-out pipes here
    /// when the toggle flips off.
    pub fn remove_grove(&self, gid: &str) {
        self.addr.do_send(RemoveGrove(gid.to_string())).ok();
    }

    /// Current config snapshot.
    pub async fn config(&self) -> SearchIndexConfig {
        self.addr.ask(GetConfig).await.unwrap_or_default()
    }

    /// Replace the config and persist. Called from settings-tweaks.md.
    pub fn set_config(&self, c: SearchIndexConfig) {
        self.addr.do_send(SetConfig(c)).ok();
    }

    /// Current recents snapshot.
    pub async fn recents(&self) -> Vec<RecentQuery> {
        self.addr.ask(GetRecents).await.unwrap_or_default()
    }

    /// Push a new recent; guarded on `config.remember_recents`.
    pub fn push_recent(&self, r: RecentQuery) {
        self.addr.do_send(PushRecent(r)).ok();
    }

    /// Forget one recent by its text.
    pub fn forget_recent(&self, text: &str) {
        self.addr.do_send(ForgetRecent(text.to_string())).ok();
    }

    /// Clear all recents.
    pub fn clear_all_recents(&self) {
        self.addr.do_send(ClearAllRecents).ok();
    }

    /// Current build status.
    pub async fn status(&self) -> SearchIndexBuildStatus {
        self.addr
            .ask(GetStatus)
            .await
            .unwrap_or(SearchIndexBuildStatus::Idle)
    }

    /// How many messages are indexed right now.
    pub async fn message_count(&self) -> usize {
        self.addr.ask(MessageCount).await.unwrap_or(0)
    }
}
