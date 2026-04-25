//! Per-device search settings and the recent-queries ring buffer.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Privacy:
//! recents, per-grove toggles, horizon, and scope live **only on this
//! device** — they never ride the event stream. This module owns the
//! in-memory shape; `crate::storage` owns the per-target persistence
//! (native files / browser localStorage).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Per-device search configuration.
///
/// Shipped defaults: `enabled=true`, `horizon_days=90`, `remember_recents=true`,
/// `per_grove_enabled` empty (every grove participates until explicitly
/// opted out).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIndexConfig {
    /// Master enable. `false` short-circuits the executor and hides
    /// the results surface. Default `true`.
    pub enabled: bool,
    /// Days of history to retain in the index. Valid values per spec:
    /// `30`, `90`, `365`, `u32::MAX` (= `all history`). Default `90`.
    pub horizon_days: u32,
    /// Whether to save recent queries locally. Default `true`.
    pub remember_recents: bool,
    /// Per-grove index opt-out. `false` = grove skipped at insert and
    /// evicted on config save; missing / `true` = grove participates.
    pub per_grove_enabled: HashMap<String, bool>,
}

impl Default for SearchIndexConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            horizon_days: 90,
            remember_recents: true,
            per_grove_enabled: HashMap::new(),
        }
    }
}

/// Ring-buffer cap for recents. Per spec §Privacy the UI caps at 8.
pub const MAX_RECENTS: usize = 8;

/// One recent query. The raw text is preserved so the chip renders the
/// user's original casing; `timestamp_ms` drives the optional "latest-
/// first" ordering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentQuery {
    /// Raw query text (preserves user casing; not lowercased).
    pub text: String,
    /// Wall-clock of the push, in ms.
    pub timestamp_ms: u64,
}

/// Push a new recent to the front; dedup by text; cap at [`MAX_RECENTS`].
pub fn push_recent(list: &mut Vec<RecentQuery>, r: RecentQuery) {
    list.retain(|e| e.text != r.text);
    list.insert(0, r);
    if list.len() > MAX_RECENTS {
        list.truncate(MAX_RECENTS);
    }
}

/// Remove a single entry by its text. No-op if absent.
pub fn forget_recent(list: &mut Vec<RecentQuery>, text: &str) {
    list.retain(|e| e.text != text);
}

/// Drop every recent. Paired with the spec's `clear all recents` UI
/// affordance.
pub fn clear_all_recents(list: &mut Vec<RecentQuery>) {
    list.clear();
}
