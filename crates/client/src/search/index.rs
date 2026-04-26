//! Inverted index for local search.
//!
//! Maps each token to the set of message postings that contain it.
//! Postings carry enough metadata to apply scope + operator filters at
//! execute time without touching the original message store.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Index: the
//! index is **never persisted**. On WASM it lives in memory for the
//! session; on native v1 it also lives in memory (SQLite FTS5 backend
//! is deferred — see plan §Architecture). Both targets inherit the
//! "encrypted-at-rest" property from not writing the index to disk at
//! all.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use willow_identity::EndpointId;

/// One message ready to be indexed.
///
/// All the metadata the executor needs to apply scope + operator
/// filters lives on the message itself — the index never looks the
/// message up elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexableMessage {
    /// Stable message id (UUID string) — used as the unique key.
    pub message_id: String,
    /// Channel id (for scope filtering).
    pub channel_id: String,
    /// Channel name (for `in:#channel` operator + result display).
    pub channel_name: String,
    /// Grove id (`None` for letter-only messages).
    pub grove_id: Option<String>,
    /// Letter id (`None` for grove-channel messages).
    pub letter_id: Option<String>,
    /// Author peer id.
    pub author_peer_id: EndpointId,
    /// Author handle, lowercased (`mira.forest.1`).
    pub author_handle: String,
    /// Author display name (preserves casing — `Mira`).
    pub author_display_name: String,
    /// Wall-clock timestamp in milliseconds since epoch.
    pub timestamp_ms: u64,
    /// Plain-text body, post-decrypt.
    pub body: String,
    /// `has:image` operator target.
    pub has_image: bool,
    /// `has:file` operator target (non-image attachment).
    pub has_file: bool,
    /// `has:link` operator target (body contains a URL).
    pub has_link: bool,
}

/// One row stored in the inverted index. Cheaply cloned into
/// `SearchResult`s at execute time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    pub message_id: String,
    pub channel_id: String,
    pub channel_name: String,
    pub grove_id: Option<String>,
    pub letter_id: Option<String>,
    pub author_peer_id: EndpointId,
    pub author_handle: String,
    pub author_display_name: String,
    pub timestamp_ms: u64,
    pub body: String,
    pub has_image: bool,
    pub has_file: bool,
    pub has_link: bool,
}

impl From<IndexableMessage> for Posting {
    fn from(m: IndexableMessage) -> Self {
        Self {
            message_id: m.message_id,
            channel_id: m.channel_id,
            channel_name: m.channel_name,
            grove_id: m.grove_id,
            letter_id: m.letter_id,
            author_peer_id: m.author_peer_id,
            author_handle: m.author_handle,
            author_display_name: m.author_display_name,
            timestamp_ms: m.timestamp_ms,
            body: m.body,
            has_image: m.has_image,
            has_file: m.has_file,
            has_link: m.has_link,
        }
    }
}

/// The inverted index itself. Not `Clone` — wrap it in an
/// `Arc<Mutex<_>>` in the handle layer.
///
/// Postings are stored as `Arc<Posting>` so a single message that
/// matches N tokens occupies one allocation plus N pointer-sized
/// references — not N deep clones of every field.
#[derive(Debug, Default)]
pub struct SearchIndex {
    /// token -> ordered list of postings (insertion order; executor
    /// re-sorts by timestamp-desc).
    pub(crate) postings: HashMap<String, Vec<Arc<Posting>>>,
    /// `message_id -> tokens` so [`Self::remove_message`] can unthread
    /// every posting list the message sits in without a full walk.
    pub(crate) by_msg: HashMap<String, Vec<String>>,
}

impl SearchIndex {
    /// Build an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one message. Idempotent — re-inserting the same
    /// `message_id` is a no-op so live-arrival + batch-rebuild paths
    /// don't double-count.
    pub fn insert(&mut self, m: IndexableMessage) {
        if self.by_msg.contains_key(&m.message_id) {
            return;
        }

        // Body tokens + synthetic tokens for author / channel so
        // `from:` / `in:` operator searches land even when the body
        // itself doesn't mention the author / channel.
        let body_tokens = super::tokenize::tokenize(&m.body);
        let mut token_set: HashSet<String> = body_tokens.into_iter().collect();
        token_set.insert(format!("@{}", m.author_handle.to_lowercase()));
        token_set.insert(m.author_handle.to_lowercase());
        if !m.author_display_name.is_empty() {
            token_set.insert(m.author_display_name.to_lowercase());
        }
        if !m.channel_name.is_empty() {
            token_set.insert(format!("#{}", m.channel_name.to_lowercase()));
            token_set.insert(m.channel_name.to_lowercase());
        }

        let id = m.message_id.clone();
        let posting = Arc::new(Posting::from(m));
        let tokens: Vec<String> = token_set.into_iter().collect();
        for t in &tokens {
            self.postings
                .entry(t.clone())
                .or_default()
                .push(Arc::clone(&posting));
        }
        self.by_msg.insert(id, tokens);
    }

    /// Remove one message by id. No-op if absent.
    pub fn remove_message(&mut self, id: &str) {
        let Some(tokens) = self.by_msg.remove(id) else {
            return;
        };
        for t in tokens {
            if let Some(list) = self.postings.get_mut(&t) {
                list.retain(|p| p.message_id != id);
            }
        }
    }

    /// Drop every message whose `channel_id` equals `cid`. Used when a
    /// channel is deleted or when per-grove-search is toggled off and
    /// the executor needs to forget everything inside it.
    pub fn remove_channel(&mut self, cid: &str) {
        let ids = self.collect_ids(|p| p.channel_id == cid);
        for id in ids {
            self.remove_message(&id);
        }
    }

    /// Drop every message whose `grove_id` equals `gid`.
    pub fn remove_grove(&mut self, gid: &str) {
        let ids = self.collect_ids(|p| p.grove_id.as_deref() == Some(gid));
        for id in ids {
            self.remove_message(&id);
        }
    }

    /// Drop every message older than `cutoff_ms`. Used by
    /// [`super::handle::SearchIndexHandle::set_horizon_days`] to keep
    /// the index bounded.
    pub fn evict_older_than(&mut self, cutoff_ms: u64) {
        let ids = self.collect_ids(|p| p.timestamp_ms < cutoff_ms);
        for id in ids {
            self.remove_message(&id);
        }
    }

    /// Number of distinct messages in the index.
    pub fn message_count(&self) -> usize {
        self.by_msg.len()
    }

    /// Read-only view of one token's postings (empty if absent). Used
    /// by [`super::execute::execute`].
    pub fn postings_for(&self, token: &str) -> Option<&[Arc<Posting>]> {
        self.postings.get(token).map(|v| v.as_slice())
    }

    /// Read-only iterator over every posting once (dedup by id).
    /// Used by [`super::execute::execute`] when the query has no tokens
    /// or phrases but still carries filters (e.g. `has:link` alone).
    pub fn all_postings(&self) -> Vec<&Arc<Posting>> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut out: Vec<&Arc<Posting>> = Vec::new();
        for list in self.postings.values() {
            for p in list {
                if seen.insert(p.message_id.as_str()) {
                    out.push(p);
                }
            }
        }
        out
    }

    fn collect_ids<F: Fn(&Posting) -> bool>(&self, pred: F) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for list in self.postings.values() {
            for p in list {
                if pred(p) && seen.insert(p.message_id.clone()) {
                    out.push(p.message_id.clone());
                }
            }
        }
        out
    }
}
