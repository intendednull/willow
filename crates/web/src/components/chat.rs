use leptos::prelude::*;
use wasm_bindgen::JsCast;
use willow_client::DisplayMessage;

use super::message_row::{day_bucket, DaySeparator, JumpToLatestPill};
use super::MessageView;
use crate::icons;

/// Scrollable message list for the current channel.
///
/// Auto-scroll to bottom only fires when the user sits within 120 px
/// of the bottom (per `docs/specs/2026-04-19-ui-design/message-row.md`
/// §Scroll anchoring). When the user has scrolled up further, a
/// `jump to latest` pill floats at the bottom-right with a ` · {N} new`
/// suffix counting messages that have arrived while they were away.
/// Clicking the pill smooth-scrolls back to the newest row and clears
/// the count.
#[component]
pub fn MessageList(
    messages: ReadSignal<Vec<DisplayMessage>>,
    /// Whether the app is still in its initial loading state.
    #[prop(optional, into)]
    loading: Signal<bool>,
    /// The local user's display name (used to determine "own" messages).
    #[prop(optional, into)]
    local_display_name: Option<Signal<String>>,
    /// Callback fired when the user clicks a message (to start a reply).
    #[prop(optional, into)]
    on_message_click: Option<Callback<DisplayMessage>>,
    /// Callback fired when the user wants to edit a message.
    #[prop(optional, into)]
    on_edit: Option<Callback<DisplayMessage>>,
    /// Callback fired when the user wants to delete a message.
    #[prop(optional, into)]
    on_delete: Option<Callback<DisplayMessage>>,
    /// Callback fired when the user picks an emoji reaction.
    #[prop(optional, into)]
    on_react: Option<Callback<(DisplayMessage, String)>>,
    /// Callback fired when the user pins/unpins a message.
    #[prop(optional, into)]
    on_pin: Option<Callback<DisplayMessage>>,
    /// Signal mapping message IDs to pin labels ("Pin" or "Unpin").
    #[prop(optional, into)]
    pin_labels: Option<Signal<std::collections::HashMap<String, String>>>,
    /// Callback fired when the user presses Escape with the list focused.
    /// Parents should focus the composer textarea in response. When
    /// unwired the Escape key is a no-op.
    #[prop(optional, into)]
    on_focus_composer: Option<Callback<()>>,
) -> impl IntoView {
    let list_ref = NodeRef::<leptos::html::Div>::new();
    // `show_pill` drives pill mount/unmount. True when the user is
    // more than 120 px from the bottom of the list (per spec).
    let (show_pill, set_show_pill) = signal(false);
    // `new_count` counts messages arrived since the user last sat
    // within the 120 px bottom band. The pill renders ` · {N} new`
    // when this is greater than zero; clicking the pill or scrolling
    // back into the band resets it to zero.
    let new_count: RwSignal<u32> = RwSignal::new(0);
    // Previous message-vector length, carried across ticks without
    // participating in reactivity — used to compute the delta of
    // newly-arrived messages each render.
    let prev_msg_len = StoredValue::new(0usize);
    // Tracks which message ID has the mobile action sheet open.
    // Lives here (outside the reactive closure) so it survives
    // message-list re-renders caused by sync events.
    let active_sheet_msg = RwSignal::new(Option::<String>::None);

    // `has_been_populated` flips to true the first time `messages` is
    // non-empty and stays true forever. Combined with the live emptiness
    // check below, it lets us distinguish the never-had-messages empty
    // state from the all-deleted / all-cleared empty state without
    // tracking a sliding `prev_len` (which would race with the render
    // memo). See `docs/specs/2026-04-19-ui-design/message-row.md`
    // §Empty / loading states.
    let has_been_populated = RwSignal::new(false);
    Effect::new(move |_| {
        if !messages.get().is_empty() {
            has_been_populated.set(true);
        }
    });

    // When messages change, check if we should auto-scroll and/or
    // bump `new_count`. Contract (spec §Scroll anchoring):
    //
    // * Auto-scroll fires only when the user is within 120 px of
    //   the bottom (the "at-bottom band").
    // * If the user is *outside* the band and new messages arrive,
    //   bump `new_count` by the delta so the pill can surface how
    //   many arrived while they were away.
    // * First render auto-scrolls unconditionally (no "was away"
    //   history yet to preserve).
    Effect::new(move |is_first: Option<bool>| {
        let msgs = messages.get();
        let len = msgs.len();
        let prev = prev_msg_len.get_value();
        let delta = len.saturating_sub(prev);
        let first_render = is_first.is_none();

        if let Some(el) = list_ref.get() {
            let el: &web_sys::HtmlElement = &el;
            let scroll_top = el.scroll_top() as f64;
            let scroll_height = el.scroll_height() as f64;
            let client_height = el.client_height() as f64;

            // Distance from the bottom *before* this tick's DOM
            // changes visually resolve. We use this to decide
            // whether the user was "at bottom" immediately prior
            // to the new content arriving.
            let distance = scroll_height - scroll_top - client_height;
            let near_bottom = distance < 120.0;

            if first_render || near_bottom {
                // Defer scroll to next microtask so DOM has
                // updated (also fixes a mobile Safari quirk).
                let el_clone = el.clone();
                set_timeout(
                    move || el_clone.set_scroll_top(el_clone.scroll_height()),
                    std::time::Duration::ZERO,
                );
            } else if delta > 0 {
                // User is scrolled up AND new messages arrived →
                // accumulate unread delta onto the pill counter.
                new_count.update(|n| *n = n.saturating_add(delta as u32));
            }

            // Pill visibility gate matches the auto-scroll gate
            // exactly: outside the 120 px band → pill visible.
            set_show_pill.set(!near_bottom);
            if near_bottom {
                new_count.set(0);
            }
        }

        prev_msg_len.set_value(len);
        false
    });

    // Smooth-scroll-to-bottom handler, bound to the pill click.
    // Per spec §Scroll anchoring: pill click runs
    // `scrollIntoView({ behavior: 'smooth' })` on the last row, then
    // clears the count. We target the list's final child element so
    // the browser's own smooth-scroll animation carries the viewport
    // to the newest message. When the list is empty (no last child),
    // fall back to an instant `set_scroll_top` jump — still satisfies
    // the "hide pill + clear count" half of the contract.
    let jump_to_latest = move || {
        if let Some(el) = list_ref.get() {
            let el: &web_sys::HtmlElement = &el;
            if let Some(last) = el.last_element_child() {
                let opts = web_sys::ScrollIntoViewOptions::new();
                opts.set_behavior(web_sys::ScrollBehavior::Smooth);
                opts.set_block(web_sys::ScrollLogicalPosition::End);
                last.scroll_into_view_with_scroll_into_view_options(&opts);
            } else {
                el.set_scroll_top(el.scroll_height());
            }
            set_show_pill.set(false);
            new_count.set(0);
        }
    };
    let jump_cb = Callback::new(move |()| jump_to_latest());

    let on_scroll = move |_| {
        if let Some(el) = list_ref.get() {
            let el: &web_sys::HtmlElement = &el;
            let scroll_top = el.scroll_top() as f64;
            let scroll_height = el.scroll_height() as f64;
            let client_height = el.client_height() as f64;
            let distance = scroll_height - scroll_top - client_height;
            let near_bottom = distance < 120.0;
            set_show_pill.set(!near_bottom);
            if near_bottom {
                new_count.set(0);
            }
        }
    };

    let on_msg_click = on_message_click;

    // Phase 2a Task 15 — spec §Accessibility / Keyboard path.
    //
    // The list container is a single Tab stop (`tabindex="0"`); arrow
    // keys move a logical `focused_idx` pointer across rows. The pointer
    // is clamped to `[0, messages_len)` on each render so newly-arrived
    // messages don't dangle the index past the end. Per-row shortcuts
    // (`R` reply, `E` edit, `Delete`, `C` copy, `+`/`:` react, `P` pin,
    // `T` reply in thread, `Enter` open overflow) route to the existing
    // MessageList callbacks by indexing into the current message vec —
    // no per-row handler churn. `Escape` fires `on_focus_composer` so
    // the parent can return focus to the composer textarea.
    let focused_idx: RwSignal<usize> = RwSignal::new(0);

    // Helper: focus the `<article>` for the message at `idx`. We look
    // the row up by `id="msg-{id}"` (already emitted by `MessageView`)
    // because the message-list DOM layout is flat and the id-based
    // query is stable across re-renders.
    let focus_row_by_idx = move |idx: usize| {
        let msgs = messages.get_untracked();
        if let Some(msg) = msgs.get(idx) {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                let sel = format!("#msg-{}", msg.id);
                if let Ok(Some(el)) = doc.query_selector(&sel) {
                    if let Ok(html_el) = el.dyn_into::<web_sys::HtmlElement>() {
                        let _ = html_el.focus();
                    }
                }
            }
        }
    };

    // Capture the inbound callbacks for the keyboard handler so we can
    // dispatch without re-borrowing MessageView's prop surface. Each
    // handler operates on the message at `focused_idx.get_untracked()`
    // at dispatch-time so keyboard actions follow the visible cursor.
    let kb_on_click = on_message_click;
    let kb_on_edit = on_edit;
    let kb_on_delete = on_delete;
    let kb_on_react = on_react;
    let kb_on_pin = on_pin;
    let kb_on_focus_composer = on_focus_composer;
    // Phase 3c.3 — gate the `P` keybinding on `ManageChannels` per
    // spec `reactions-pins.md` §Permission + action. Falls back to
    // `false` when AppState context is absent (unit-test mounts
    // without the full shell), so the keystroke is a silent no-op
    // rather than firing an unauthorised pin event.
    let kb_can_pin: leptos::prelude::Signal<bool> = use_context::<crate::state::AppState>()
        .map(|s| s.server.local_can_manage_channels.into())
        .unwrap_or_else(|| leptos::prelude::Signal::derive(|| false));

    let handle_list_keydown = move |ev: web_sys::KeyboardEvent| {
        let msgs = messages.get_untracked();
        let messages_len = msgs.len();
        if messages_len == 0 {
            return;
        }
        // Re-clamp defensively in case messages shrank since the last
        // render (delete, channel swap, etc.).
        let mut idx = focused_idx.get_untracked().min(messages_len - 1);
        let key = ev.key();
        match key.as_str() {
            "ArrowUp" => {
                ev.prevent_default();
                idx = idx.saturating_sub(1);
                focused_idx.set(idx);
                focus_row_by_idx(idx);
            }
            "ArrowDown" => {
                ev.prevent_default();
                if idx + 1 < messages_len {
                    idx += 1;
                }
                focused_idx.set(idx);
                focus_row_by_idx(idx);
            }
            "Home" => {
                ev.prevent_default();
                focused_idx.set(0);
                focus_row_by_idx(0);
            }
            "End" => {
                ev.prevent_default();
                let last = messages_len - 1;
                focused_idx.set(last);
                focus_row_by_idx(last);
            }
            "Escape" => {
                ev.prevent_default();
                if let Some(cb) = kb_on_focus_composer {
                    cb.run(());
                }
            }
            "Enter" => {
                // Per spec: Enter opens the overflow menu on the focused
                // row. The overflow menu is attached to the row's
                // `.action-trigger` button; click it programmatically.
                ev.prevent_default();
                if let Some(msg) = msgs.get(idx) {
                    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                        let sel = format!("#msg-{} .action-trigger", msg.id);
                        if let Ok(Some(el)) = doc.query_selector(&sel) {
                            if let Ok(html_el) = el.dyn_into::<web_sys::HtmlElement>() {
                                html_el.click();
                            }
                        }
                    }
                }
            }
            "r" | "R" => {
                ev.prevent_default();
                if let (Some(msg), Some(cb)) = (msgs.get(idx), kb_on_click) {
                    cb.run(msg.clone());
                }
            }
            "t" | "T" => {
                // Reply in thread — no-op placeholder until the thread
                // pane ships (`thread-pane.md`). Consuming the keystroke
                // prevents a literal `t` from leaking to the composer
                // when the list is focused.
                ev.prevent_default();
            }
            "p" | "P" => {
                ev.prevent_default();
                // Phase 3c.3: silent no-op when the local peer lacks
                // `ManageChannels` per spec `reactions-pins.md`
                // §Permission + action. The visible affordances
                // (overflow menu, action sheet, pinned-panel unpin)
                // grey out + show the `only stewards can pin here`
                // tooltip; the keybinding's silent fallback matches
                // the spec's "no surprises" intent.
                if !kb_can_pin.get_untracked() {
                    return;
                }
                if let (Some(msg), Some(cb)) = (msgs.get(idx), kb_on_pin) {
                    cb.run(msg.clone());
                }
            }
            "e" | "E" => {
                ev.prevent_default();
                if let (Some(msg), Some(cb)) = (msgs.get(idx), kb_on_edit) {
                    // Parent gates edit on `is_own`; we mirror that
                    // gate here so an `E` keystroke on a non-own row
                    // is a silent no-op rather than a permission error.
                    if msg.is_local {
                        cb.run(msg.clone());
                    }
                }
            }
            "Delete" | "Backspace" => {
                ev.prevent_default();
                if let (Some(msg), Some(cb)) = (msgs.get(idx), kb_on_delete) {
                    if msg.is_local {
                        cb.run(msg.clone());
                    }
                }
            }
            "c" | "C" => {
                ev.prevent_default();
                if let Some(msg) = msgs.get(idx) {
                    crate::util::copy_to_clipboard(&msg.body);
                }
            }
            "+" | ":" => {
                // Spec: `+` or `:` opens the reaction picker. Until
                // `reactions-pins.md` lands the full picker, fire the
                // first quick-reaction (thumbs up) through the existing
                // `on_react` callback so the keystroke has a visible
                // effect rather than silently doing nothing.
                ev.prevent_default();
                if let (Some(msg), Some(cb)) = (msgs.get(idx), kb_on_react) {
                    cb.run((msg.clone(), "\u{1F44D}".to_string()));
                }
            }
            _ => {}
        }
    };

    // Surface downgrade banners for any downgraded peer visible in this
    // chat view. The peer-letter / DM view is the eventual home for
    // this banner per spec; until that surface exists we render the
    // first downgraded peer at the top of the message list so the
    // user never loses sight of a key-rotation warning.
    let downgraded_target = {
        use willow_client::trust::PeerTrust;
        let app_state = use_context::<crate::state::AppState>();
        leptos::prelude::Memo::new(move |_| {
            app_state.and_then(|s| {
                s.trust.trust_map.get().iter().find_map(|(pid, trust)| {
                    if matches!(trust, PeerTrust::DowngradedFromVerified { .. }) {
                        Some(pid.clone())
                    } else {
                        None
                    }
                })
            })
        })
    };

    view! {
        <div class="message-list-container">
            {move || {
                downgraded_target.get().map(|pid| {
                    view! {
                        <crate::components::DowngradeBanner
                            peer_id=Signal::derive(move || pid.clone())
                        />
                    }
                })
            }}
            // Phase 2a Task 15 — spec §Accessibility.
            // * `role="log"` + `aria-live="polite"`: incoming messages
            //   announce to screen readers while the list is focused.
            //   Not focused → no announcement (notifications in 1f
            //   handle the OS-level cue per spec).
            // * `aria-label="channel messages"`: names the log region.
            // * `tabindex="0"`: single tab-stop — Tab from the composer
            //   moves focus here; arrow keys then navigate rows.
            // * `on:keydown=handle_list_keydown`: wires the spec's
            //   keyboard path (ArrowUp/Down, Enter, Escape, R/T/P/E/
            //   Delete/C/+/:).
            <div
                class="message-list"
                node_ref=list_ref
                role="log"
                aria-live="polite"
                aria-label="channel messages"
                tabindex="0"
                on:scroll=on_scroll
                on:keydown=handle_list_keydown
            >
                {move || {
                    let msgs = messages.get();
                    let is_loading = loading.get();
                    if is_loading && msgs.is_empty() {
                        // Loading skeleton: five rows, 32 px circle +
                        // two shimmer bars (name + body). Reduced motion
                        // is handled by `foundation.css` which disables
                        // the shimmer animation globally; the bars
                        // remain visible as static `--bg-2` rectangles.
                        // Contract: `docs/specs/2026-04-19-ui-design/\
                        // message-row.md` §Loading.
                        let rows: Vec<_> = (0..5).map(|_| view! {
                            <div class="chat-skeleton-row">
                                <div class="chat-skeleton__avatar"></div>
                                <div class="chat-skeleton__bars">
                                    <div class="chat-skeleton__bar chat-skeleton__bar--name"></div>
                                    <div class="chat-skeleton__bar chat-skeleton__bar--body"></div>
                                </div>
                            </div>
                        }.into_any()).collect();
                        view! {
                            <div class="chat-skeleton" aria-hidden="true">
                                {rows}
                            </div>
                        }.into_any()
                    } else if msgs.is_empty() {
                        // Split empty into "never-had" vs "cleared" via
                        // the `has_been_populated` latch — see field
                        // comment above. Spec §Empty channel (no
                        // messages ever) vs §Empty after deletions.
                        if has_been_populated.get() {
                            view! {
                                <div class="chat-cleared" role="status">
                                    <h2 class="chat-cleared__headline">
                                        "cleared — nothing here yet."
                                    </h2>
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div class="chat-empty" role="status">
                                    <div class="chat-empty__art">
                                        {icons::icon_leaf()}
                                    </div>
                                    <h2 class="chat-empty__headline">
                                        "this channel is quiet. say hi?"
                                    </h2>
                                    <p class="chat-empty__subtext">
                                        "messages here are sealed to everyone in the grove."
                                    </p>
                                </div>
                            }.into_any()
                        }
                    } else {
                        // Build grouped message views: consecutive messages from
                        // the same author collapse the header.
                        let on_click = on_msg_click;
                        let on_ed = on_edit;
                        let on_del = on_delete;
                        let on_re = on_react;
                        let on_pn = on_pin;
                        let pn_labels = pin_labels;
                        let local_name = local_display_name
                            .map(|sig| sig.get())
                            .unwrap_or_default();
                        let label_map = pn_labels.map(|s| s.get()).unwrap_or_default();
                        // Track the previous message's local-date bucket so we
                        // can inject a `<DaySeparator>` at every local-date
                        // boundary (and before the first message). `PartialEq`
                        // on `DayBucket` drives the comparison — a bucket
                        // difference means "emit a separator before this row".
                        let mut prev_bucket: Option<super::message_row::DayBucket> = None;
                        let views: Vec<_> = msgs.iter().enumerate().flat_map(|(i, msg)| {
                            let show_header = if i == 0 {
                                true
                            } else {
                                let prev = &msgs[i - 1];
                                // Per message-row.md §Author-run grouping: break a run
                                // on author change, >5min gap, or when either the
                                // previous *or* current message carries a run-break
                                // cue (whisper / pinned / queueNote). Task 8 fully
                                // wires all three rules — `whisper` is gated
                                // always-false in the projection today (see
                                // `views::compute_messages_view` TODO) but the
                                // predicate is ready for when `whisper-mode.md`
                                // flips the gate.
                                prev.author_display_name != msg.author_display_name
                                    || msg.timestamp_ms.saturating_sub(prev.timestamp_ms)
                                        > 300_000
                                    || prev.pinned
                                    || msg.pinned
                                    || prev.queue_note != willow_client::QueueNote::None
                                    || msg.queue_note != willow_client::QueueNote::None
                                    || prev.whisper
                                    || msg.whisper
                            };
                            let curr_bucket = day_bucket(msg.timestamp_ms);
                            let emit_sep = match &prev_bucket {
                                None => true,
                                Some(p) => p != &curr_bucket,
                            };
                            prev_bucket = Some(curr_bucket.clone());
                            let sep_view = if emit_sep {
                                Some(view! {
                                    <DaySeparator bucket=curr_bucket.clone() />
                                }.into_any())
                            } else {
                                None
                            };
                            let m = msg.clone();
                            let is_own = msg.is_local;
                            // Check if this is a reply targeting the local user.
                            let is_mention = !is_own
                                && msg
                                    .reply_preview
                                    .as_ref()
                                    .map(|p| p.starts_with(&format!("{local_name}:")))
                                    .unwrap_or(false);
                            let pin_label = label_map
                                .get(&msg.id)
                                .cloned()
                                .unwrap_or_else(|| "Pin".to_string());
                            let mut builder = view! {
                                <MessageView
                                    message=m
                                    show_header=show_header
                                    is_own=is_own
                                    is_mention=is_mention
                                    active_sheet_msg=active_sheet_msg
                                />
                            };
                            // We need to build with all props. Re-create to pass them.
                            let m2 = msg.clone();
                            if on_click.is_some() || on_ed.is_some() || on_del.is_some() || on_re.is_some() || on_pn.is_some() {
                                let click_cb = on_click;
                                let ed_cb = on_ed;
                                let del_cb = on_del;
                                let re_cb = on_re;
                                let pn_cb = on_pn;
                                builder = view! {
                                    <MessageView
                                        message=m2
                                        show_header=show_header
                                        is_own=is_own
                                        is_mention=is_mention
                                        on_click=click_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        on_edit=ed_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        on_delete=del_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        on_react=re_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        on_pin=pn_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        pin_label=pin_label
                                        active_sheet_msg=active_sheet_msg
                                    />
                                };
                            }
                            let mut out = Vec::with_capacity(2);
                            if let Some(sep) = sep_view {
                                out.push(sep);
                            }
                            out.push(builder.into_any());
                            out
                        }).collect();
                        view! { <div>{views}</div> }.into_any()
                    }
                }}
            </div>
            {move || {
                // Mount the jump-to-latest pill only when the user
                // is outside the 120 px bottom band. The pill reads
                // `new_count` internally and renders ` · {N} new`
                // when positive.
                if show_pill.get() {
                    Some(view! {
                        <JumpToLatestPill
                            new_count=Signal::derive(move || new_count.get())
                            on_click=jump_cb
                        />
                    })
                } else {
                    None
                }
            }}
        </div>
    }
}
