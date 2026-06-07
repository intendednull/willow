//! `<EmojiPicker>` — 320 × 360 popover with search + categories +
//! recent shelf.
//!
//! T3 of the phase-3c plan ships the visual surface with full
//! arrow-key navigation + Enter / Escape semantics. Click a glyph or
//! press Enter on a highlighted glyph fires `on_select(glyph)`;
//! Escape fires `on_close()`. The component is render-only — callers
//! own the open/closed state.

use leptos::ev::KeyboardEvent;
use leptos::prelude::*;

use super::categories::{search, EMOJI_CATEGORIES};

/// Popover-style emoji picker per spec §Emoji picker.
///
/// `recent` is the spec's "recent" category — typically threaded from
/// `ClientHandle::recent_reactions(channel)`. `on_select` fires with
/// the selected glyph; `on_close` fires on Escape or click-away. The
/// component does not own its visibility — wrap with `<Show when=...>`
/// at the call site.
#[component]
pub fn EmojiPicker(
    /// Recent emojis (typically per-channel from the client). Shown
    /// at the top of the grid above the static categories.
    recent: Signal<Vec<String>>,
    /// Glyph-pick callback.
    on_select: Callback<String>,
    /// Escape / dismiss callback.
    on_close: Callback<()>,
) -> impl IntoView {
    let (query, set_query) = signal(String::new());
    let (highlight, set_highlight) = signal(0_usize);

    let glyphs = Memo::new(move |_| {
        let recent = recent.get();
        let q = query.get();
        // Clone so the returned `Vec<&str>` doesn't outlive `recent`
        // when the memo evaluates again.
        let glyph_refs = search(&q, &recent);
        glyph_refs
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    });

    let glyphs_for_keydown = glyphs;
    let on_select_for_keydown = on_select;
    let on_close_for_keydown = on_close;
    let on_keydown = move |ev: KeyboardEvent| {
        let key = ev.key();
        let count = glyphs_for_keydown.with(|g| g.len());
        match key.as_str() {
            "ArrowRight" => {
                ev.prevent_default();
                set_highlight.update(|h| {
                    if count == 0 {
                        *h = 0;
                    } else {
                        *h = (*h + 1) % count;
                    }
                });
            }
            "ArrowLeft" => {
                ev.prevent_default();
                set_highlight.update(|h| {
                    if count == 0 {
                        *h = 0;
                    } else if *h == 0 {
                        *h = count - 1;
                    } else {
                        *h -= 1;
                    }
                });
            }
            "Enter" | "Tab" => {
                ev.prevent_default();
                let glyph = glyphs_for_keydown.with(|g| g.get(highlight.get_untracked()).cloned());
                if let Some(g) = glyph {
                    on_select_for_keydown.run(g);
                }
            }
            "Escape" => {
                ev.prevent_default();
                on_close_for_keydown.run(());
            }
            _ => {}
        }
    };

    view! {
        <div
            class="emoji-picker"
            role="dialog"
            aria-label="emoji picker"
            on:keydown=on_keydown
        >
            <input
                class="emoji-picker__search"
                type="text"
                placeholder="search emoji"
                aria-label="search emoji"
                on:input=move |ev| {
                    let v = event_target_value(&ev);
                    set_query.set(v);
                    set_highlight.set(0);
                }
            />
            <div class="emoji-picker__grid">
                {move || {
                    glyphs.get().into_iter().enumerate().map(|(i, glyph)| {
                        let glyph_for_click = glyph.clone();
                        let glyph_for_aria = glyph.clone();
                        let glyph_for_label = glyph.clone();
                        let select = on_select;
                        let class = move || if highlight.get() == i {
                            "emoji-picker__cell emoji-picker__cell--selected"
                        } else {
                            "emoji-picker__cell"
                        };
                        view! {
                            <button
                                class=class
                                type="button"
                                aria-label=glyph_for_aria
                                on:click=move |_| select.run(glyph_for_click.clone())
                            >
                                {glyph_for_label}
                            </button>
                        }
                    }).collect::<Vec<_>>()
                }}
            </div>
            <div class="emoji-picker__category-strip">
                {EMOJI_CATEGORIES.iter().map(|(label, _)| view! {
                    <span class="emoji-picker__category-label">{*label}</span>
                }).collect::<Vec<_>>()}
            </div>
        </div>
    }
}
