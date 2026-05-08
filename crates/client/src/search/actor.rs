//! `SearchActor` — owns the inverted index, config, recents, and build
//! status as one unit of state on a single mailbox.
//!
//! Replaces the prior `Arc<Mutex<_>>` quartet in [`super::handle`].
//! Cross-field updates (e.g. "rebuild done + status reset") are atomic
//! by construction because handlers run to completion before the next
//! message processes. See `docs/specs/2026-04-26-state-management-model-design.md`.
//!
//! The actor is bespoke (not `StateActor<S>`) for two reasons:
//!
//! 1. [`super::index::SearchIndex`] is intentionally not `Clone` — the
//!    inverted index is large and cloning would be wasteful.
//!    `StateActor::Mutate` requires `S: Clone` for copy-on-write, so
//!    a generic state actor doesn't fit.
//! 2. Most messages (`Insert`, `Rebuild`, `Query`, `RemoveMessage`,
//!    etc.) carry domain-specific intent that maps poorly to a generic
//!    "mutate via closure" surface; bespoke handlers read better.

use std::future::Future;

use willow_actor::{Actor, Context, Handler, Message};

use super::config::{self, RecentQuery, SearchIndexConfig};
use super::execute::{execute, SearchResult, SearchScope};
use super::index::{IndexableMessage, SearchIndex};
use super::query::SearchQuery;
use super::status::SearchIndexBuildStatus;

// ───── Actor ─────────────────────────────────────────────────────────────

/// Owns search state on a single mailbox.
pub struct SearchActor {
    index: SearchIndex,
    config: SearchIndexConfig,
    recents: Vec<RecentQuery>,
    status: SearchIndexBuildStatus,
    /// `true` when config + recents writes hit `crate::storage`.
    persist: bool,
}

impl SearchActor {
    pub fn new(config: SearchIndexConfig, recents: Vec<RecentQuery>, persist: bool) -> Self {
        Self {
            index: SearchIndex::new(),
            config,
            recents,
            status: SearchIndexBuildStatus::Idle,
            persist,
        }
    }

    fn message_allowed(&self, m: &IndexableMessage) -> bool {
        if !self.config.enabled {
            return false;
        }
        if let Some(gid) = &m.grove_id {
            if self.config.per_grove_enabled.get(gid).copied() == Some(false) {
                return false;
            }
        }
        true
    }
}

impl Actor for SearchActor {}

// ───── Messages ──────────────────────────────────────────────────────────

/// Insert one live message. No-op if config disables it.
pub struct Insert(pub IndexableMessage);
impl Message for Insert {
    type Result = ();
}

impl Handler<Insert> for SearchActor {
    fn handle(&mut self, msg: Insert, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        if self.message_allowed(&msg.0) {
            self.index.insert(msg.0);
        }
        async {}
    }
}

/// Drop the current index and rebuild from `msgs`. The status field
/// transitions Idle → Building → Idle. The `Indexing { done, total }`
/// variant is reserved for a future chunked rebuild that yields between
/// batches; this handler runs to completion in one mailbox turn so
/// per-step progress is unobservable and intentionally not emitted.
///
/// Config is captured as a snapshot at handler entry: a `SetConfig`
/// queued during a long rebuild waits behind it in the mailbox and
/// only takes effect after rebuild completes (atomicity by mailbox
/// serialization).
pub struct Rebuild(pub Vec<IndexableMessage>);
impl Message for Rebuild {
    type Result = ();
}

impl Handler<Rebuild> for SearchActor {
    fn handle(
        &mut self,
        msg: Rebuild,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.status = SearchIndexBuildStatus::Building;
        self.index = SearchIndex::new();
        for m in msg.0 {
            if !self.message_allowed(&m) {
                continue;
            }
            self.index.insert(m);
        }
        self.status = SearchIndexBuildStatus::Idle;
        async {}
    }
}

/// Run a query against the index under `scope`.
pub struct Query {
    pub q: SearchQuery,
    pub scope: SearchScope,
}
impl Message for Query {
    type Result = Vec<SearchResult>;
}

impl Handler<Query> for SearchActor {
    fn handle(
        &mut self,
        msg: Query,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = Vec<SearchResult>> + Send {
        let results = execute(&self.index, &msg.q, &msg.scope);
        async move { results }
    }
}

/// Remove one message by id.
pub struct RemoveMessage(pub String);
impl Message for RemoveMessage {
    type Result = ();
}

impl Handler<RemoveMessage> for SearchActor {
    fn handle(
        &mut self,
        msg: RemoveMessage,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.index.remove_message(&msg.0);
        async {}
    }
}

/// Remove every message in a channel.
pub struct RemoveChannel(pub String);
impl Message for RemoveChannel {
    type Result = ();
}

impl Handler<RemoveChannel> for SearchActor {
    fn handle(
        &mut self,
        msg: RemoveChannel,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.index.remove_channel(&msg.0);
        async {}
    }
}

/// Remove every message in a grove.
pub struct RemoveGrove(pub String);
impl Message for RemoveGrove {
    type Result = ();
}

impl Handler<RemoveGrove> for SearchActor {
    fn handle(
        &mut self,
        msg: RemoveGrove,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.index.remove_grove(&msg.0);
        async {}
    }
}

/// Read the current config snapshot.
pub struct GetConfig;
impl Message for GetConfig {
    type Result = SearchIndexConfig;
}

impl Handler<GetConfig> for SearchActor {
    fn handle(
        &mut self,
        _msg: GetConfig,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = SearchIndexConfig> + Send {
        let cfg = self.config.clone();
        async move { cfg }
    }
}

/// Replace the config (and persist if enabled).
pub struct SetConfig(pub SearchIndexConfig);
impl Message for SetConfig {
    type Result = ();
}

impl Handler<SetConfig> for SearchActor {
    fn handle(
        &mut self,
        msg: SetConfig,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        self.config = msg.0.clone();
        if self.persist {
            crate::storage::save_search_config(&msg.0);
        }
        async {}
    }
}

/// Read the current recents snapshot.
pub struct GetRecents;
impl Message for GetRecents {
    type Result = Vec<RecentQuery>;
}

impl Handler<GetRecents> for SearchActor {
    fn handle(
        &mut self,
        _msg: GetRecents,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = Vec<RecentQuery>> + Send {
        let recents = self.recents.clone();
        async move { recents }
    }
}

/// Push a new recent (guarded on `config.remember_recents`).
pub struct PushRecent(pub RecentQuery);
impl Message for PushRecent {
    type Result = ();
}

impl Handler<PushRecent> for SearchActor {
    fn handle(
        &mut self,
        msg: PushRecent,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        if self.config.remember_recents {
            config::push_recent(&mut self.recents, msg.0);
            if self.persist {
                crate::storage::save_search_recents(&self.recents);
            }
        }
        async {}
    }
}

/// Forget one recent by its text.
pub struct ForgetRecent(pub String);
impl Message for ForgetRecent {
    type Result = ();
}

impl Handler<ForgetRecent> for SearchActor {
    fn handle(
        &mut self,
        msg: ForgetRecent,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        config::forget_recent(&mut self.recents, &msg.0);
        if self.persist {
            crate::storage::save_search_recents(&self.recents);
        }
        async {}
    }
}

/// Clear every recent.
pub struct ClearAllRecents;
impl Message for ClearAllRecents {
    type Result = ();
}

impl Handler<ClearAllRecents> for SearchActor {
    fn handle(
        &mut self,
        _msg: ClearAllRecents,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = ()> + Send {
        config::clear_all_recents(&mut self.recents);
        if self.persist {
            crate::storage::save_search_recents(&self.recents);
        }
        async {}
    }
}

/// Read the current build status.
pub struct GetStatus;
impl Message for GetStatus {
    type Result = SearchIndexBuildStatus;
}

impl Handler<GetStatus> for SearchActor {
    fn handle(
        &mut self,
        _msg: GetStatus,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = SearchIndexBuildStatus> + Send {
        let status = self.status.clone();
        async move { status }
    }
}

/// Read how many messages are indexed right now.
pub struct MessageCount;
impl Message for MessageCount {
    type Result = usize;
}

impl Handler<MessageCount> for SearchActor {
    fn handle(
        &mut self,
        _msg: MessageCount,
        _ctx: &mut Context<Self>,
    ) -> impl Future<Output = usize> + Send {
        let n = self.index.message_count();
        async move { n }
    }
}
