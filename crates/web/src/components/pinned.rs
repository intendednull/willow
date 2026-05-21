//! `<PinnedPanel>` — right-rail / overlay slot showing pinned
//! messages for the current channel.
//!
//! Spec: `docs/specs/2026-04-19-ui-design/reactions-pins.md`
//! §Pinned panel contents. Entry layout: avatar (24 px) + display
//! name (body weight 500) + timestamp (`--ink-3` 11 px) + 2-line
//! body preview ellipsis + optional `pinned by {name} · {when}`
//! footer (`--ink-3` 10 px mono `when`). Per-entry actions on
//! hover (or always on mobile): `jump to` + `unpin` (permission-
//! gated on `ManageChannels`).
//!
//! Empty state copy `nothing pinned yet.` — italic, `--ink-3`,
//! 13 px (foundation tokens; no new hex). Header copy unchanged
//! ("Pinned Messages") since the spec doesn't override it.

use leptos::prelude::*;
use willow_client::DisplayMessage;

use super::message::extract_urls;
use crate::icons;

/// Render a message body with clickable URL links. Mirrors the row
/// renderer's link extraction so pinned previews keep the same
/// affordance as live messages.
fn render_body_with_links(body: &str) -> impl IntoView {
    let (segments, _images) = extract_urls(body);
    view! {
        <span>
            {segments.into_iter().map(|(text, is_url)| {
                if is_url {
                    let display = text.clone();
                    view! {
                        <a href=text target="_blank" rel="noopener noreferrer" class="message-link">{display}</a>
                    }.into_any()
                } else {
                    view! { <span>{text}</span> }.into_any()
                }
            }).collect::<Vec<_>>()}
        </span>
    }
}

/// Format a wall-clock millisecond timestamp into a compact `mm:ss`-ish
/// hint suitable for the entry's mono `when` slot. Falls back to the
/// message's display string for very recent entries.
///
/// Today this returns the relative-time string the rest of the row
/// renderer already uses; a follow-up can split out a true `mm:ss`
/// renderer once the spec calls for it.
fn pinned_timestamp(timestamp_ms: u64) -> String {
    super::message::format_relative_time(timestamp_ms)
}

/// Panel showing pinned messages for the current channel.
///
/// `messages` — pinned message list, newest-first per spec.
/// `can_unpin` — drives the per-entry unpin button's enabled state;
/// callers pass `Signal::derive(...)` reading `state.permissions.has_manage_channels`.
/// `on_jump` — fired when the user clicks a row's `jump to` button.
/// `on_unpin` — fired when the user clicks a row's `unpin` button.
/// Callers pass `Callback::new(...)` so the same parent state owns
/// both interactions.
/// `on_close` — fired when the user dismisses the panel.
#[component]
pub fn PinnedPanel(
    messages: ReadSignal<Vec<DisplayMessage>>,
    #[prop(optional, into)] can_unpin: Option<Signal<bool>>,
    on_jump: impl Fn(String) + Send + Clone + 'static,
    #[prop(optional)] on_unpin: Option<Callback<String>>,
    on_close: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    let can_unpin = can_unpin.unwrap_or_else(|| Signal::derive(|| false));

    view! {
        <aside class="pinned-panel" role="complementary" aria-label="pinned">
            <div class="pinned-header">
                <h3>"Pinned Messages"</h3>
                <button
                    class="btn btn-sm"
                    aria-label="close pinned panel"
                    on:click=move |_| on_close(())
                >
                    {icons::icon_x()}
                </button>
            </div>
            <div class="pinned-list">
                <For
                    each=move || messages.get()
                    key=|msg| msg.id.clone()
                    let:msg
                >
                    {
                        let msg_id = msg.id.clone();
                        let msg_id_for_unpin = msg.id.clone();
                        let author = msg.author_display_name.clone();
                        let body = msg.body.clone();
                        let when = pinned_timestamp(msg.timestamp_ms);
                        let on_jump = on_jump.clone();
                        let unpin_disabled = move || !can_unpin.get();
                        let unpin_aria_label = "unpin message".to_string();
                        // `pinned by {name} · {when}` footer — spec
                        // `docs/specs/2026-04-19-ui-design/reactions-pins.md`
                        // §Pinned panel contents, line 123. Rendered
                        // only when the projection populated
                        // `pinned_metadata`; absent metadata omits the
                        // entire <footer> per the
                        // `pinned-message-metadata-design` doc's
                        // omission contract.
                        let pinner_footer = msg.pinned_metadata.as_ref().map(|meta| {
                            let name = meta.pinner_display_name.clone();
                            let pin_when = pinned_timestamp(meta.pinned_at_ms);
                            view! {
                                <footer class="pinned-entry__footer">
                                    "pinned by " {name} " · "
                                    <span class="pinned-entry__footer-when">{pin_when}</span>
                                </footer>
                            }
                        });
                        view! {
                            <article class="pinned-entry">
                                <div class="pinned-entry__meta">
                                    <span class="pinned-entry__avatar" aria-hidden="true">
                                        {author.chars().next().map(|c| c.to_string()).unwrap_or_default()}
                                    </span>
                                    <span class="pinned-entry__author">{author}</span>
                                    <time class="pinned-entry__when">{when}</time>
                                </div>
                                <div class="pinned-entry__body">
                                    {render_body_with_links(&body)}
                                </div>
                                {pinner_footer}
                                <div class="pinned-entry__actions">
                                    <button
                                        class="pinned-entry__jump"
                                        aria-label="jump to pinned message"
                                        on:click=move |_| on_jump(msg_id.clone())
                                    >
                                        "jump to"
                                    </button>
                                    {move || on_unpin.map(|cb| {
                                        let id = msg_id_for_unpin.clone();
                                        let label = unpin_aria_label.clone();
                                        view! {
                                            <button
                                                class="pinned-entry__unpin"
                                                aria-label=label
                                                aria-disabled=move || if unpin_disabled() { "true" } else { "false" }
                                                disabled=unpin_disabled
                                                title=move || if unpin_disabled() { "only stewards can pin here" } else { "unpin" }
                                                on:click=move |_| {
                                                    if !unpin_disabled() {
                                                        cb.run(id.clone());
                                                    }
                                                }
                                            >
                                                "unpin"
                                            </button>
                                        }
                                    })}
                                </div>
                            </article>
                        }
                    }
                </For>
                {move || {
                    if messages.get().is_empty() {
                        Some(view! {
                            <div class="pinned-empty">
                                "nothing pinned yet."
                            </div>
                        })
                    } else {
                        None
                    }
                }}
            </div>
        </aside>
    }
}
