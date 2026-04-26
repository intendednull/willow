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

/// Group results per the spec's scope-dependent rules.
fn group_results(rows: &[SearchResult], scope: &SearchScope) -> Vec<(String, Vec<SearchResult>)> {
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

    let sections = move || {
        groups
            .get()
            .into_iter()
            .map(|(label, items)| {
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
                let rows: Vec<AnyView> = items
                    .into_iter()
                    .map(|r| {
                        view! {
                            <ResultRow
                                result=r
                                selected=Signal::derive(|| false)
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
