//! Scope-aware query executor.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Scope ladder +
//! §Results presentation: executes a [`SearchQuery`] over a
//! [`SearchIndex`] under a [`SearchScope`], returning hits in
//! timestamp-desc order with matched byte-ranges ready for the
//! highlight renderer.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::index::{Posting, SearchIndex};
use super::query::{QueryFilters, SearchQuery};

/// Scope of a search invocation. Matches `local-search.md` §Scope ladder.
///
/// `Serialize` / `Deserialize` so the web UI can persist the user's
/// chosen scope across reloads via `localStorage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id")]
pub enum SearchScope {
    /// Only this letter's messages.
    ThisLetter(String),
    /// Only this channel's messages (channel id, not name — ids are stable).
    ThisChannel(String),
    /// Every peer + group letter on this device.
    AllLetters,
    /// Every grove channel plus every letter (widest).
    AllGrovesAndLetters,
}

/// One hit emitted by [`execute`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub message_id: String,
    pub channel_id: String,
    pub channel_name: String,
    pub grove_id: Option<String>,
    pub letter_id: Option<String>,
    pub author_display_name: String,
    pub author_handle: String,
    pub timestamp_ms: u64,
    pub body: String,
    /// Byte ranges of each matched span inside `body`. Populated when
    /// the highlight module lands (Task 5) — empty until then.
    pub matched_ranges: Vec<(usize, usize)>,
}

/// Execute `query` over `index` under `scope`.
///
/// Returns hits in timestamp-desc order. Dedup is by `message_id`: a
/// message that matches via multiple tokens renders once.
pub fn execute(index: &SearchIndex, query: &SearchQuery, scope: &SearchScope) -> Vec<SearchResult> {
    let candidates = candidate_postings(index, query);

    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<SearchResult> = candidates
        .into_iter()
        .filter(|p| scope_admits(p, scope))
        .filter(|p| filters_admit(p, &query.filters))
        .filter(|p| body_admits(p, query))
        .filter(|p| seen.insert(p.message_id.clone()))
        .map(|p| SearchResult {
            message_id: p.message_id.clone(),
            channel_id: p.channel_id.clone(),
            channel_name: p.channel_name.clone(),
            grove_id: p.grove_id.clone(),
            letter_id: p.letter_id.clone(),
            author_display_name: p.author_display_name.clone(),
            author_handle: p.author_handle.clone(),
            timestamp_ms: p.timestamp_ms,
            body: p.body.clone(),
            matched_ranges: super::highlight::match_ranges(&p.body, query),
        })
        .collect();

    out.sort_by_key(|p| std::cmp::Reverse(p.timestamp_ms));
    out
}

/// Candidate set: posting-lists for every token + first-word of each
/// phrase. Falls back to every posting when the query has no tokens
/// (pure operator-only queries like `has:link`).
fn candidate_postings(index: &SearchIndex, query: &SearchQuery) -> Vec<Posting> {
    let mut lookup_tokens: Vec<String> = query.tokens.clone();
    for ph in &query.phrases {
        if let Some(first) = ph.split_whitespace().next() {
            lookup_tokens.push(first.to_string());
        }
    }

    let mut candidates: Vec<Posting> = Vec::new();
    for t in &lookup_tokens {
        if let Some(slice) = index.postings_for(t) {
            candidates.extend(slice.iter().cloned());
        }
    }

    if candidates.is_empty() {
        candidates = index.all_postings().into_iter().cloned().collect();
    }

    candidates
}

fn scope_admits(p: &Posting, scope: &SearchScope) -> bool {
    match scope {
        SearchScope::ThisLetter(id) => p.letter_id.as_deref() == Some(id.as_str()),
        SearchScope::ThisChannel(id) => p.channel_id == *id,
        SearchScope::AllLetters => p.letter_id.is_some(),
        SearchScope::AllGrovesAndLetters => true,
    }
}

fn filters_admit(p: &Posting, f: &QueryFilters) -> bool {
    if let Some(h) = &f.from_author {
        let lc = h.to_lowercase();
        if p.author_handle.to_lowercase() != lc && p.author_display_name.to_lowercase() != lc {
            return false;
        }
    }
    if let Some(c) = &f.in_channel {
        if p.channel_name.to_lowercase() != c.to_lowercase() {
            return false;
        }
    }
    if let Some(d) = f.since {
        if p.timestamp_ms < local_midnight_ms(d) {
            return false;
        }
    }
    if let Some(d) = f.before {
        if p.timestamp_ms >= local_midnight_ms(d) {
            return false;
        }
    }
    if f.has_image && !p.has_image {
        return false;
    }
    if f.has_file && !p.has_file {
        return false;
    }
    if f.has_link && !p.has_link {
        return false;
    }
    true
}

/// Convert a [`NaiveDate`] into a millisecond-epoch cutoff using local
/// time. Per spec §Query language: `since:` / `before:` are "local
/// timezone" boundaries.
///
/// Uses `chrono::Local` which compiles on both native and wasm32.
/// On wasm the browser's timezone drives the offset.
fn local_midnight_ms(d: NaiveDate) -> u64 {
    use chrono::TimeZone;
    let naive = d.and_hms_opt(0, 0, 0).expect("00:00:00 is always valid");
    let ts = chrono::Local
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.timestamp_millis())
        .unwrap_or_else(|| {
            // Fallback for DST-ambiguous local times: use UTC
            // midnight — deliberately coarse. Spec says "local
            // timezone" so this branch is rare and the UI will flag
            // a warning via the parser in a future pass if needed.
            chrono::Utc.from_utc_datetime(&naive).timestamp_millis()
        });
    ts.max(0) as u64
}

/// Every token in `query.tokens` must appear; every phrase must appear
/// as a contiguous substring. Case-insensitive throughout.
///
/// Tokens match across body + author + channel so `hello from:@mira`
/// finds messages where "mira" is the author even if the body doesn't
/// say "mira" — per spec §Query language.
fn body_admits(p: &Posting, q: &SearchQuery) -> bool {
    let body_lc = p.body.to_lowercase();
    let display_lc = p.author_display_name.to_lowercase();
    let handle_lc = p.author_handle.to_lowercase();
    let channel_lc = p.channel_name.to_lowercase();
    for t in &q.tokens {
        if !body_lc.contains(t)
            && !display_lc.contains(t)
            && !handle_lc.contains(t)
            && !channel_lc.contains(t)
        {
            return false;
        }
    }
    for ph in &q.phrases {
        if !body_lc.contains(ph) {
            return false;
        }
    }
    true
}
