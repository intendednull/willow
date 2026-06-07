//! `<ResultsList>` — the grouped results listbox.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Grouping:
//!
//! - `all groves + letters` → group by grove id (header "grove: {id}")
//!   then letters cluster under a synthetic `letters` group at the
//!   bottom;
//! - `all letters` → group by letter id;
//! - `this channel` / `this letter` → single implicit group (header
//!   hidden).
//!
//! The banner `searching… · {n} matches so far` renders above the list
//! when [`SearchIndexBuildStatus::Indexing`] is active.

use std::collections::BTreeMap;

use leptos::prelude::*;
use willow_client::{SearchIndexBuildStatus, SearchResult, SearchScope};

use super::row::ResultRow;
use crate::state::AppState;

/// Compute the cumulative flat-index offset of every group in
/// `groups`. Returns a vector of the same length whose `i`th entry is
/// the count of rows that appear *before* group `i` in the rendered
/// listbox. Combined with the row's intra-group index, this yields the
/// row's global flat index — the unit `active_index` is expressed in.
fn group_offsets(groups: &[(String, Vec<SearchResult>)]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(groups.len());
    let mut running = 0usize;
    for (_, items) in groups {
        offsets.push(running);
        running += items.len();
    }
    offsets
}

/// Flatten grouped results into the order rows appear in the listbox.
/// `active_index` indexes into this vector, so keyboard navigation in
/// `<SearchInput>` and `aria-selected` rendering here agree on what
/// "row N" means.
pub(super) fn flat_ordered(rows: &[SearchResult], scope: &SearchScope) -> Vec<SearchResult> {
    group_results(rows, scope)
        .into_iter()
        .flat_map(|(_, items)| items)
        .collect()
}

/// Group results per the spec's scope-dependent rules.
pub(super) fn group_results(
    rows: &[SearchResult],
    scope: &SearchScope,
) -> Vec<(String, Vec<SearchResult>)> {
    match scope {
        SearchScope::ThisChannel(_) | SearchScope::ThisLetter(_) => {
            vec![(String::new(), rows.to_vec())]
        }
        SearchScope::AllLetters => {
            let mut m: BTreeMap<String, Vec<SearchResult>> = BTreeMap::new();
            for r in rows {
                let key = r.letter_id.clone().unwrap_or_else(|| "letter".into());
                m.entry(key).or_default().push(r.clone());
            }
            m.into_iter().collect()
        }
        SearchScope::AllGrovesAndLetters => {
            let mut m: BTreeMap<String, Vec<SearchResult>> = BTreeMap::new();
            for r in rows {
                let key = r
                    .grove_id
                    .clone()
                    .map(|g| format!("grove: {g}"))
                    .unwrap_or_else(|| "letters".into());
                m.entry(key).or_default().push(r.clone());
            }
            m.into_iter().collect()
        }
    }
}

/// Render the grouped listbox.
#[component]
pub fn ResultsList(
    /// Fired when a row is selected (click / Enter).
    #[prop(into)]
    on_select: Callback<SearchResult>,
) -> impl IntoView {
    let state = use_context::<AppState>().expect("AppState");

    let streaming_banner = move || match state.search.status.get() {
        SearchIndexBuildStatus::Indexing { done, total: _ } => Some(view! {
            <div class="search-streaming-banner" role="status" aria-live="polite">
                {format!("searching… · {done} matches so far")}
            </div>
        }),
        _ => None,
    };

    let groups =
        Memo::new(move |_| group_results(&state.search.results.get(), &state.search.scope.get()));
    let active_index = state.search.active_index;

    let sections = move || {
        let groups_now = groups.get();
        let offsets = group_offsets(&groups_now);
        groups_now
            .into_iter()
            .enumerate()
            .map(|(group_idx, (label, items))| {
                let header = if label.is_empty() {
                    None
                } else {
                    let count = items.len();
                    Some(view! {
                        <div class="search-group-header">
                            <em>{label.clone()}</em>
                            " "
                            <span class="search-group-count">{format!("({count})")}</span>
                        </div>
                    })
                };
                let base = offsets[group_idx];
                let rows: Vec<AnyView> = items
                    .into_iter()
                    .enumerate()
                    .map(|(intra, r)| {
                        // Flat (in-display-order) index of this row.
                        // The `active_index` signal is expressed in the
                        // same units so a row's `selected` derives from
                        // a single equality check.
                        let flat = base + intra;
                        view! {
                            <ResultRow
                                result=r
                                selected=Signal::derive(move || active_index.get() == flat)
                                on_select=on_select
                            />
                        }
                        .into_any()
                    })
                    .collect();
                view! {
                    <div class="search-group">
                        {header}
                        {rows}
                    </div>
                }
                .into_any()
            })
            .collect::<Vec<_>>()
    };

    view! {
        {streaming_banner}
        <div
            id="search-results-list"
            class="search-results"
            role="listbox"
            aria-label="search results"
        >
            {sections}
        </div>
    }
}
