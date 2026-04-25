//! `<ResultRow>` — one search hit in the results listbox.
//!
//! Per `docs/specs/2026-04-19-ui-design/local-search.md` §Row anatomy:
//!
//! 1. Context line (Fraunces italic container name · author · soft ts)
//! 2. Body excerpt (3 lines, centred on first match, `<mark>` around
//!    each matched range)
//! 3. Right-column `ArrowUpRight` on desktop (hidden on mobile).
//!
//! The row is a `<button role="option">` sized ≥ 44 × 44 on mobile so
//! touch targets meet spec.

use leptos::prelude::*;
use willow_client::{search::build_excerpt, SearchResult};

use crate::icons;
use willow_client::util::format_timestamp;

/// Render the excerpt as a sequence of `<span>` / `<mark>` children.
/// Ranges point into `text` (local, not body-global). Falls back to a
/// single span when there are no ranges.
fn render_excerpt(text: &str, ranges: &[(usize, usize)]) -> Vec<AnyView> {
    if ranges.is_empty() {
        return vec![view! { <span>{text.to_string()}</span> }.into_any()];
    }
    let mut out: Vec<AnyView> = Vec::with_capacity(ranges.len() * 2 + 1);
    let mut cursor = 0usize;
    for &(a, b) in ranges {
        if a > cursor {
            let slice = text[cursor..a].to_string();
            out.push(view! { <span>{slice}</span> }.into_any());
        }
        if a < b && b <= text.len() {
            let slice = text[a..b].to_string();
            out.push(view! { <mark aria-label="match">{slice}</mark> }.into_any());
            cursor = b;
        }
    }
    if cursor < text.len() {
        out.push(view! { <span>{text[cursor..].to_string()}</span> }.into_any());
    }
    out
}

/// One result row.
#[component]
pub fn ResultRow(
    result: SearchResult,
    /// Whether this row is the keyboard-focused selection.
    #[prop(into)]
    selected: Signal<bool>,
    /// Fired when the row is clicked / activated with Enter.
    #[prop(into)]
    on_select: Callback<SearchResult>,
) -> impl IntoView {
    let id_attr = format!("search-row-{}", result.message_id);
    let excerpt = build_excerpt(&result.body, &result.matched_ranges, 60);
    let ts = format_timestamp(result.timestamp_ms);
    let spans = render_excerpt(&excerpt.text, &excerpt.ranges);

    let result_for_click = result.clone();

    view! {
        <button
            id=id_attr
            class=move || {
                if selected.get() {
                    "search-result-row is-selected"
                } else {
                    "search-result-row"
                }
            }
            role="option"
            aria-selected=move || selected.get().to_string()
            on:click=move |_| on_select.run(result_for_click.clone())
        >
            <div class="search-result-context">
                <em class="search-result-container">{result.channel_name.clone()}</em>
                " "
                <span class="search-result-author">{result.author_display_name.clone()}</span>
                " · "
                <span class="search-result-ts">{ts}</span>
            </div>
            <div class="search-result-excerpt">{spans}</div>
            <span class="search-result-arrow" aria-hidden="true">
                {icons::icon_arrow_up_right()}
            </span>
        </button>
    }
}
