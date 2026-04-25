//! `<ScopeChip>` — the four-way scope selector above the results list.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Scope ladder:
//! pill button + popover with four options (`this letter`,
//! `this channel`, `all letters`, `all groves + letters`). Unreachable
//! scopes (no focused letter / channel) render disabled with
//! `title="open a {letter|channel} first"`. Keyboard: Enter opens,
//! arrow keys move, Esc closes.

use leptos::prelude::*;
use willow_client::SearchScope;

use crate::icons;
use crate::state::{AppState, AppWriteSignals};

/// Return the lowercase label for a scope, per spec §Copy.
fn label_for(s: &SearchScope) -> &'static str {
    match s {
        SearchScope::ThisLetter(_) => "this letter",
        SearchScope::ThisChannel(_) => "this channel",
        SearchScope::AllLetters => "all letters",
        SearchScope::AllGrovesAndLetters => "all groves + letters",
    }
}

/// Compare scope kinds ignoring the id payload (so the popover can
/// show which *kind* is selected even if the selected id differs).
fn kind_matches(a: &SearchScope, b: &SearchScope) -> bool {
    matches!(
        (a, b),
        (SearchScope::ThisLetter(_), SearchScope::ThisLetter(_))
            | (SearchScope::ThisChannel(_), SearchScope::ThisChannel(_))
            | (SearchScope::AllLetters, SearchScope::AllLetters)
            | (
                SearchScope::AllGrovesAndLetters,
                SearchScope::AllGrovesAndLetters
            )
    )
}

/// Popover that lists the four scope options.
#[component]
pub fn ScopeChip(
    /// Id of the currently-focused channel. `None` greys the
    /// `this channel` popover row.
    #[prop(into)]
    focused_channel: Signal<Option<String>>,
    /// Id of the currently-focused letter. `None` greys the
    /// `this letter` popover row.
    #[prop(optional, into)]
    focused_letter: Option<Signal<Option<String>>>,
) -> impl IntoView {
    let state = use_context::<AppState>().expect("AppState");
    let write = use_context::<AppWriteSignals>().expect("AppWriteSignals");
    let (open, set_open) = signal(false);

    let disabled_channel = Signal::derive(move || focused_channel.get().is_none());
    let disabled_letter =
        Signal::derive(move || focused_letter.map(|s| s.get().is_none()).unwrap_or(true));

    view! {
        <div class="scope-chip-wrap">
            <button
                class="scope-chip"
                aria-haspopup="listbox"
                aria-expanded=move || open.get().to_string()
                on:click=move |_| set_open.update(|v| *v = !*v)
            >
                <span class="scope-chip-label">
                    {move || label_for(&state.search.scope.get())}
                </span>
                <span class="scope-chip-chevron" aria-hidden="true">
                    {icons::icon_chevron_down()}
                </span>
            </button>
            {move || open.get().then(|| view! {
                <div class="scope-chip-popover" role="listbox">
                    <button
                        class="scope-chip-popover-option"
                        role="option"
                        aria-selected=move || {
                            matches!(state.search.scope.get(), SearchScope::ThisLetter(_)).to_string()
                        }
                        disabled=disabled_letter
                        title=move || if disabled_letter.get() { "open a letter first" } else { "" }
                        on:click=move |_| {
                            if let Some(sig) = focused_letter {
                                if let Some(id) = sig.get() {
                                    write.search.set_scope.set(SearchScope::ThisLetter(id));
                                    set_open.set(false);
                                }
                            }
                        }
                    >
                        "this letter"
                    </button>
                    <button
                        class="scope-chip-popover-option"
                        role="option"
                        aria-selected=move || {
                            matches!(state.search.scope.get(), SearchScope::ThisChannel(_)).to_string()
                        }
                        disabled=disabled_channel
                        title=move || if disabled_channel.get() { "open a channel first" } else { "" }
                        on:click=move |_| {
                            if let Some(id) = focused_channel.get() {
                                write.search.set_scope.set(SearchScope::ThisChannel(id));
                                set_open.set(false);
                            }
                        }
                    >
                        "this channel"
                    </button>
                    <button
                        class="scope-chip-popover-option"
                        role="option"
                        aria-selected=move || kind_matches(&state.search.scope.get(), &SearchScope::AllLetters).to_string()
                        on:click=move |_| {
                            write.search.set_scope.set(SearchScope::AllLetters);
                            set_open.set(false);
                        }
                    >
                        "all letters"
                    </button>
                    <button
                        class="scope-chip-popover-option"
                        role="option"
                        aria-selected=move || kind_matches(&state.search.scope.get(), &SearchScope::AllGrovesAndLetters).to_string()
                        on:click=move |_| {
                            write.search.set_scope.set(SearchScope::AllGrovesAndLetters);
                            set_open.set(false);
                        }
                    >
                        "all groves + letters"
                    </button>
                </div>
            })}
        </div>
    }
}
