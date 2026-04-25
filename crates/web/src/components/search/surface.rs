//! `<SearchSurface>` — the full-screen takeover that hosts the input,
//! scope chip, results, and privacy footer.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Layout:
//!
//! 1. Sticky `<SearchInput>`;
//! 2. `<ScopeChip>` + streaming-banner slot inside `<ResultsList>`;
//! 3. Results (or recents, when query is empty);
//! 4. Privacy footer (always visible) — `search runs on this device
//!    only. queries never leave your device.`

use leptos::prelude::*;
use willow_client::search::parse_query;
use willow_client::{RecentQuery, SearchIndexHandle, SearchResult};

use super::input::SearchInput;
use super::recents::RecentsList;
use super::results::ResultsList;
use super::scope_chip::ScopeChip;
use crate::state::{AppState, AppWriteSignals};

/// Full-screen search takeover.
#[component]
pub fn SearchSurface(
    /// The app-wide index handle. Cloned into the debounced query
    /// effect and into the recent-queries mutations.
    index: SearchIndexHandle,
    /// Fired when a result row is activated — caller navigates to the
    /// message's native container.
    #[prop(into)]
    on_select_result: Callback<SearchResult>,
) -> impl IntoView {
    let state = use_context::<AppState>().expect("AppState");
    let write = use_context::<AppWriteSignals>().expect("AppWriteSignals");

    // Debounced query driver: 120 ms after the last keystroke, parse
    // the query and run against the index under the current scope.
    //
    // Reads `state.search.query` + `state.search.scope` so both
    // re-run the search. `set_debouncing` / `set_results` are the
    // only writes — everything else flows through the handle.
    let idx = index.clone();
    Effect::new(move |_| {
        let raw = state.search.query.get();
        let scope = state.search.scope.get();
        let idx = idx.clone();

        if raw.is_empty() {
            // Empty query: no scan, no results, no spinner.
            write.search.set_debouncing.set(false);
            write.search.set_results.set(Vec::new());
            return;
        }

        write.search.set_debouncing.set(true);
        let handle_res = set_timeout_with_handle(
            move || {
                let q = parse_query(&raw);
                let results = idx.query(&q, &scope);
                write.search.set_results.set(results);
                write.search.set_debouncing.set(false);
            },
            std::time::Duration::from_millis(120),
        );
        // Cancel on next run so an in-flight debounce doesn't stomp a
        // fresher query. `on_cleanup` fires before the effect re-runs.
        if let Ok(h) = handle_res {
            on_cleanup(move || h.clear());
        }
    });

    let on_submit = {
        let idx = index.clone();
        Callback::new(move |q: String| {
            if !q.is_empty() {
                idx.push_recent(RecentQuery {
                    text: q,
                    timestamp_ms: js_sys::Date::now() as u64,
                });
                write.search.set_recents.set(idx.recents());
            }
        })
    };

    let on_pick_recent = {
        Callback::new(move |text: String| {
            write.search.set_query.set(text);
        })
    };

    let on_forget_recent = {
        let idx = index.clone();
        Callback::new(move |text: String| {
            idx.forget_recent(&text);
            write.search.set_recents.set(idx.recents());
        })
    };

    let on_clear_all_recents = {
        let idx = index.clone();
        Callback::new(move |()| {
            idx.clear_all_recents();
            write.search.set_recents.set(idx.recents());
        })
    };

    // Focused-channel signal derives from the active channel; focused-
    // letter stays `None` until `letters-dms.md` ships.
    let focused_channel = Signal::derive(move || {
        let ch = state.chat.current_channel.get();
        if ch.is_empty() {
            None
        } else {
            Some(ch)
        }
    });

    view! {
        <div class="search-surface">
            <SearchInput on_submit=on_submit />
            <ScopeChip focused_channel=focused_channel />
            {move || {
                let q = state.search.query.get();
                if q.is_empty() {
                    // Recents / empty-state branch. When `remember_recents`
                    // is off, the recents vec will already be empty.
                    view! {
                        <RecentsList
                            on_pick=on_pick_recent
                            on_forget=on_forget_recent
                            on_clear_all=on_clear_all_recents
                        />
                        <div class="search-empty-never">
                            "type to search — queries stay on this device."
                        </div>
                    }
                    .into_any()
                } else {
                    view! {
                        <ResultsList on_select=on_select_result />
                    }
                    .into_any()
                }
            }}
            <div class="search-privacy-footer">
                "search runs on this device only. queries never leave your device."
            </div>
        </div>
    }
}
