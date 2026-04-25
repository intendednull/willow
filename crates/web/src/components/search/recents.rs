//! `<RecentsList>` — suggestion chips under the empty search input.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Privacy /
//! §Empty states: up to 8 chips, each with a leading search icon and
//! clipped query text. Individual long-press / right-click `forget`;
//! global `clear all recents` below.
//!
//! Rendered only when `SearchIndexConfig::remember_recents` is true —
//! the caller suppresses the component when the toggle is off.

use leptos::prelude::*;
use willow_client::RecentQuery;

use crate::icons;
use crate::state::AppState;

#[component]
pub fn RecentsList(
    /// Fired when a chip is clicked — caller fills the input with the
    /// stored text.
    #[prop(into)]
    on_pick: Callback<String>,
    /// Fired when a chip's right-click / forget action fires.
    #[prop(into)]
    on_forget: Callback<String>,
    /// Fired by the `clear all recents` action.
    #[prop(into)]
    on_clear_all: Callback<()>,
) -> impl IntoView {
    let state = use_context::<AppState>().expect("AppState");

    view! {
        <div class="search-recents" role="list" aria-label="recent searches">
            <For
                each=move || state.search.recents.get()
                key=|r: &RecentQuery| r.text.clone()
                let:r
            >
                {
                    let text_click = r.text.clone();
                    let text_forget = r.text.clone();
                    let text_label = r.text.clone();
                    view! {
                        <button
                            class="search-recent-chip"
                            role="listitem"
                            on:click=move |_| on_pick.run(text_click.clone())
                            on:contextmenu=move |ev| {
                                // Right-click / long-press → forget.
                                ev.prevent_default();
                                on_forget.run(text_forget.clone());
                            }
                            title="right-click to forget"
                        >
                            <span class="icon">{icons::icon_search()}</span>
                            <span>{text_label}</span>
                        </button>
                    }
                }
            </For>
            <button
                class="search-recent-clear"
                on:click=move |_| on_clear_all.run(())
            >
                "clear all recents"
            </button>
        </div>
    }
}
