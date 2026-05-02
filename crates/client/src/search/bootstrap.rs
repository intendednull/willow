//! Bootstrap + incremental hooks for the local search index.
//!
//! Replaces the prior signal-driven full-rebuild path in
//! `crates/web/src/app.rs`. Per `local-search.md` §Build behaviour the
//! index is **incremental on arrival, lazy on historical scan** — every
//! send/receive/edit must NOT trigger a `Rebuild` (which destroys the
//! whole index, allocating fresh `Posting` lists for every token of
//! every message). Issue #354 covers the regression where the prior
//! `Effect` kept doing exactly that.
//!
//! Two paths live here:
//!
//! - [`hydrate_index`] — one-shot bootstrap. Walks every channel in the
//!   materialized `ServerState` and inserts each non-deleted message
//!   into the index. Idempotent (driven by `SearchIndex::insert`'s
//!   `by_msg` dedup).
//! - [`index_message`] / [`reindex_message`] — the incremental hooks
//!   the UI calls on `ClientEvent::MessageReceived` /
//!   `ClientEvent::MessageEdited`. Both look the channel name up from
//!   `ServerState::channels` so the `in:#name` operator stays correct.
//!
//! The web crate keeps the wiring (signal subscription + grove-id
//! resolution); this module owns only the data flow into the index.

use crate::search::{IndexableMessage, SearchIndexHandle};

/// Hydrate the index from every channel in the materialized state.
///
/// Called once on UI bootstrap. Subsequent updates flow through
/// [`index_message`] / [`reindex_message`] / `SearchIndexHandle::remove_message`.
///
/// `grove_id` is the active grove (server) id — stamped onto every
/// `IndexableMessage` so per-grove opt-out + `in:#grove` filtering work.
pub async fn hydrate_index<N: willow_network::Network>(
    client: &crate::ClientHandle<N>,
    search: &SearchIndexHandle,
    grove_id: Option<String>,
) {
    let channels = client.channels().await;
    for name in &channels {
        let msgs = client.messages(name).await;
        for m in msgs {
            // Skip deleted rows so the index never carries tombstones.
            if m.deleted {
                continue;
            }
            let im = IndexableMessage::from_display_message(&m, name, grove_id.clone(), None);
            search.insert(im);
        }
    }
}

/// Insert one message by id — incremental hook for
/// `ClientEvent::MessageReceived`.
///
/// Resolves channel name from `channel_id` via `state_snapshot` so the
/// `in:#name` operator continues to match. The DAG id is what
/// `MessageReceived.channel` carries (see `derive_client_events` in
/// `crates/client/src/mutations.rs`).
pub async fn index_message<N: willow_network::Network>(
    client: &crate::ClientHandle<N>,
    search: &SearchIndexHandle,
    channel_id: &str,
    message_id: &str,
    grove_id: Option<String>,
) {
    let snap = client.state_snapshot().await;
    let Some(channel_name) = snap.channels.get(channel_id).map(|c| c.name.clone()) else {
        // Channel id doesn't resolve — happens transiently during
        // sync if the message arrives before its CreateChannel event.
        // Bootstrap re-walks all channels on the next session start, so
        // dropping here is recoverable.
        return;
    };
    let Some(msg) = client
        .messages(&channel_name)
        .await
        .into_iter()
        .find(|m| m.id == message_id)
    else {
        return;
    };
    if msg.deleted {
        return;
    }
    let im = IndexableMessage::from_display_message(&msg, &channel_name, grove_id, None);
    search.insert(im);
}

/// Re-index one message — incremental hook for `ClientEvent::MessageEdited`.
///
/// `SearchIndex::insert` is idempotent (it short-circuits on
/// `message_id` to avoid double-counting from the legacy rebuild path).
/// Edits must therefore go through `remove_message` first; otherwise
/// the new body never lands.
pub async fn reindex_message<N: willow_network::Network>(
    client: &crate::ClientHandle<N>,
    search: &SearchIndexHandle,
    channel_id: &str,
    message_id: &str,
    grove_id: Option<String>,
) {
    search.remove_message(message_id);
    index_message(client, search, channel_id, message_id, grove_id).await;
}
