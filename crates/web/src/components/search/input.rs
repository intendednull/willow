//! `<SearchInput>` — the sticky top-of-surface query field.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Entry points:
//!
//! - wrapped in `<form role="search" aria-label="local search">` so
//!   assistive tech can land on the search landmark;
//! - `Esc` with a non-empty query clears it; `Esc` with an empty query
//!   closes the surface (spec §Desktop — Escape contract);
//! - `aria-controls="search-results-list"` + `aria-autocomplete="list"`
//!   + a placeholder that mirrors the active scope;
//! - `ArrowUp` / `ArrowDown` / `Home` / `End` move
//!   [`SearchUiState::active_index`](crate::state::SearchUiState::active_index)
//!   so keyboard users can see which result row is the activation
//!   target. The active row is also announced via
//!   `aria-activedescendant`, which points at the row's
//!   `id="search-row-{message_id}"` per WAI-ARIA listbox guidance.
//! - `Enter` activates the highlighted row (same path mouse click takes
//!   on the row) when `active_index` points at a real result; otherwise
//!   it falls back to the recents-push `on_submit` so the empty-query
//!   affordance still works. Wired this way per issue #406 — keyboard
//!   and pointer converge on a single activation path.

use leptos::prelude::*;
use willow_client::{SearchResult, SearchScope};

use crate::state::{AppState, AppWriteSignals};

/// Placeholder copy for each scope. Lowercase, per foundation.md.
fn placeholder_for(scope: &SearchScope) -> &'static str {
    match scope {
        SearchScope::ThisLetter(_) => "search this letter",
        SearchScope::ThisChannel(_) => "search this channel",
        SearchScope::AllLetters => "search all letters",
        SearchScope::AllGrovesAndLetters => "search groves + letters",
    }
}

/// Sticky search input at the top of the surface.
#[component]
pub fn SearchInput(
    /// Fired with the current query text when Enter is pressed and there
    /// is no highlighted row to activate (no results, or `active_index`
    /// out of range). Used for the empty-query "push to recents" path.
    #[prop(into)]
    on_submit: Callback<String>,
    /// Fired with the highlighted result row when Enter is pressed and
    /// `active_index` points at a real row. Same path a mouse click on
    /// the row takes — keyboard and pointer must converge here per
    /// issue #406.
    #[prop(into)]
    on_activate: Callback<SearchResult>,
) -> impl IntoView {
    let state = use_context::<AppState>().expect("AppState");
    let write = use_context::<AppWriteSignals>().expect("AppWriteSignals");

    let placeholder = move || placeholder_for(&state.search.scope.get());

    // Length of the *flat, in-display-order* result list. Computed
    // here (not in the keydown handler) so the bound stays current
    // when results are streamed in.
    let result_count = move || {
        super::results::flat_ordered(
            &state.search.results.get_untracked(),
            &state.search.scope.get_untracked(),
        )
        .len()
    };

    // Active row's DOM id, or `None` when there are no results. Used
    // for `aria-activedescendant` — per WAI-ARIA, the attribute must
    // be omitted (not blank) when no option is active.
    let active_descendant = Memo::new(move |_| {
        let flat =
            super::results::flat_ordered(&state.search.results.get(), &state.search.scope.get());
        let i = state.search.active_index.get();
        flat.get(i).map(|r| format!("search-row-{}", r.message_id))
    });

    let on_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "Escape" => {
            ev.prevent_default();
            if !state.search.query.get_untracked().is_empty() {
                write.search.set_query.set(String::new());
            } else {
                write.search.set_open.set(false);
            }
        }
        "Enter" => {
            // Always swallow Enter — the surrounding `<form role="search">`
            // would otherwise navigate / submit and tear down the surface.
            ev.prevent_default();
            // If a result row is highlighted (active_index in range of
            // the flat, in-display-order results), activate it via the
            // same callback a click on the row fires. Otherwise (no
            // results, or stale out-of-bounds index) fall back to the
            // recents-push submit path so the empty-query affordance
            // still works. Keeping both paths behind one Enter handler
            // is what closes #406.
            let flat = super::results::flat_ordered(
                &state.search.results.get_untracked(),
                &state.search.scope.get_untracked(),
            );
            let i = state.search.active_index.get_untracked();
            match flat.get(i) {
                Some(row) => on_activate.run(row.clone()),
                None => on_submit.run(state.search.query.get_untracked()),
            }
        }
        "ArrowDown" => {
            let n = result_count();
            if n == 0 {
                return;
            }
            ev.prevent_default();
            let cur = state.search.active_index.get_untracked();
            // Wrap to top at the tail. Wrapping is the listbox-pattern
            // default and matches how command palettes elsewhere in
            // the app behave.
            let next = if cur + 1 >= n { 0 } else { cur + 1 };
            write.search.set_active_index.set(next);
        }
        "ArrowUp" => {
            let n = result_count();
            if n == 0 {
                return;
            }
            ev.prevent_default();
            let cur = state.search.active_index.get_untracked();
            let next = if cur == 0 { n - 1 } else { cur - 1 };
            write.search.set_active_index.set(next);
        }
        "Home" => {
            if result_count() == 0 {
                return;
            }
            ev.prevent_default();
            write.search.set_active_index.set(0);
        }
        "End" => {
            let n = result_count();
            if n == 0 {
                return;
            }
            ev.prevent_default();
            write.search.set_active_index.set(n - 1);
        }
        _ => {}
    };

    view! {
        <form
            role="search"
            aria-label="local search"
            class="search-form"
            on:submit=move |ev| ev.prevent_default()
        >
            <input
                class=move || {
                    if state.search.debouncing.get() {
                        "search-input is-debouncing"
                    } else {
                        "search-input"
                    }
                }
                type="text"
                placeholder=placeholder
                aria-label="local search input"
                aria-autocomplete="list"
                aria-controls="search-results-list"
                aria-activedescendant=move || active_descendant.get()
                prop:value=move || state.search.query.get()
                on:input=move |ev| write.search.set_query.set(event_target_value(&ev))
                on:keydown=on_keydown
            />
        </form>
    }
}
