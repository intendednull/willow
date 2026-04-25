//! `<SearchInput>` — the sticky top-of-surface query field.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Entry points:
//!
//! - wrapped in `<form role="search" aria-label="local search">` so
//!   assistive tech can land on the search landmark;
//! - `Esc` with a non-empty query clears it; `Esc` with an empty query
//!   closes the surface (spec §Desktop — Escape contract);
//! - `aria-controls="search-results-list"` + `aria-autocomplete="list"`
//!   + a placeholder that mirrors the active scope.

use leptos::prelude::*;
use willow_client::SearchScope;

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
    /// Fired with the current query text when the user presses Enter
    /// (used for recents push).
    #[prop(into)]
    on_submit: Callback<String>,
) -> impl IntoView {
    let state = use_context::<AppState>().expect("AppState");
    let write = use_context::<AppWriteSignals>().expect("AppWriteSignals");

    let placeholder = move || placeholder_for(&state.search.scope.get());

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
            ev.prevent_default();
            on_submit.run(state.search.query.get_untracked());
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
                prop:value=move || state.search.query.get()
                on:input=move |ev| write.search.set_query.set(event_target_value(&ev))
                on:keydown=on_keydown
            />
        </form>
    }
}
