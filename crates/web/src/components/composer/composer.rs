//! `<Composer>` parent component — Phase 3a shell.
//!
//! T5 ships the minimum-viable shell: an autogrow textarea + a send
//! button that mirrors the existing `<ChatInput>` prop contract so
//! callsites only swap the component name. T6 lights up the full
//! keydown table (Enter / Shift+Enter / Ctrl|Cmd+Enter / Tab /
//! Esc-unwind / ArrowUp-edit-last) per
//! `docs/specs/2026-04-19-ui-design/composer.md`. Reply / edit bars
//! carry over the legacy markup unchanged here; T7 / T8 restyle them
//! per spec. Mention autocomplete, meta row, typing indicator, offline
//! tint, and per-channel-kind placeholder copy land in later tasks.
//!
//! The outer wrapper carries both `composer` (new) and `input-area`
//! (legacy). The legacy class keeps the existing CSS rules (focus
//! outline, padding) and the existing focus-back JS in `app.rs`
//! (`document.querySelector('.input-area input,.input-area textarea')`)
//! working until later tasks port them onto the `composer` namespace.
//!
//! Autogrow algorithm: the textarea's `style.height` is set to its
//! `scrollHeight` capped at 8 × line-height. The capping effect runs
//! whenever `input_text` changes — including when the submit handler
//! resets it to empty, which collapses the textarea back to its
//! `min-height: 1.45em` baseline.
//!
//! Mobile detection: the wrapper reads `data-shell` from the `<html>`
//! root (set by `mobile_shell` / test harness via
//! `mount_test_with_shell`) so a single `<Composer>` component can
//! serve both desktop (Enter sends) and mobile (Enter inserts newline,
//! Ctrl/Cmd+Enter still force-sends) keyboard conventions per spec
//! §Keyboard (mobile). No new global signals — the lookup is done
//! once per keydown.

use leptos::prelude::*;
use std::sync::OnceLock;
use wasm_bindgen::JsCast;
use willow_client::mentions::Suggestions;
use willow_client::views::MentionCandidate;
use willow_client::DisplayMessage;
use willow_identity::EndpointId;
use willow_state::types::ChannelKind;

use super::edit_bar::EditBar;
use super::mention_autocomplete::MentionAutocomplete;
use super::meta_row::MetaRow;
use super::placeholders::placeholder_for;
use super::reply_bar::ReplyBar;
use super::typing_indicator::TypingIndicator;
use crate::state::ConnectionState;

/// Maximum number of visible textarea lines before the textarea
/// switches from grow-to-fit to scroll. Matches the spec's
/// "grows by `scrollHeight` up to 8 lines then scrolls" rule.
const MAX_VISIBLE_LINES: f64 = 8.0;

/// Compose surface — autogrow textarea + send button.
///
/// Prop contract is the superset of `<ChatInput>`: `on_send`,
/// `replying_to`, `on_cancel_reply`, `editing`, `on_edit_send`,
/// `on_cancel_edit`, `on_typing`. Later phase 3a tasks add reactive
/// wiring (placeholder copy, mention autocomplete, typing indicator)
/// without changing this signature.
#[component]
pub fn Composer(
    /// Fires when the user submits a normal (non-edit) message.
    on_send: impl Fn(String) + Send + Clone + 'static,
    /// The message being replied to (if any).
    #[prop(optional, into)]
    replying_to: Option<ReadSignal<Option<DisplayMessage>>>,
    /// Callback to cancel the current reply.
    #[prop(optional, into)]
    on_cancel_reply: Option<Callback<()>>,
    /// The message currently being edited (if any).
    #[prop(optional, into)]
    editing: Option<ReadSignal<Option<DisplayMessage>>>,
    /// Callback fired when the user submits the edited message
    /// (`message_id`, `new_body`).
    #[prop(optional, into)]
    on_edit_send: Option<Callback<(String, String)>>,
    /// Callback to cancel the current edit.
    #[prop(optional, into)]
    on_cancel_edit: Option<Callback<()>>,
    /// Callback fired on each `input` event (drives the typing-ping
    /// throttle in the parent).
    #[prop(optional, into)]
    on_typing: Option<Callback<()>>,
    /// Fired when the user presses `ArrowUp` while the textarea is
    /// empty. The parent looks up `client.last_own_message(channel)`
    /// and writes the result into its `editing` signal — Composer
    /// stays unaware of the client handle and channel id (Option A in
    /// the T6 plan: parent owns the state, composer just emits).
    #[prop(optional, into)]
    on_arrow_up_edit: Option<Callback<()>>,
    /// Fired with the parent message id when the user clicks the body
    /// of the reply preview bar. The chat view wires this to a
    /// scroll-into-view + 180 ms `willow-row-flash` on the parent.
    /// Cancelling the reply uses `on_cancel_reply` instead.
    #[prop(optional, into)]
    on_jump_to_parent: Option<Callback<String>>,
    /// Live connection state. Drives the meta row offline form and the
    /// amber composer tint per spec §Offline state. When omitted (e.g.
    /// in a focused unit test) defaults to `Connected`.
    #[prop(optional, into)]
    connection: Option<ReadSignal<ConnectionState>>,
    /// Channel peer count for the mobile meta row's `sealed to {N}
    /// peers in grove` copy and the desktop placeholder's
    /// `encrypted to {N} peers` suffix. Defaults to `0` when omitted.
    #[prop(optional, into)]
    peer_count: Option<Signal<usize>>,
    /// Current channel's kind. Plumbed through to `placeholder_for`
    /// for forward compatibility (the spec's letter affordance has no
    /// dedicated kind in `willow-state` yet). Defaults to
    /// `ChannelKind::Text` when omitted.
    #[prop(optional, into)]
    channel_kind: Option<Signal<ChannelKind>>,
    /// Current channel name (without the `#`). Empty string means
    /// "no channel selected" — `placeholder_for` then returns the
    /// `choose a channel to start` form.
    #[prop(optional, into)]
    channel_name: Option<Signal<String>>,
    /// Recipient name for the letter / 1:1 DM placeholder form. `None`
    /// for v1 (Letter not shipped — see plan §Ambiguity decisions).
    /// Reserved as a prop so Phase 3b can wire it without re-shaping
    /// the API.
    #[prop(optional, into)]
    recipient_name: Option<Signal<Option<String>>>,
    /// Display names of peers currently typing in the active channel.
    /// Drives the `<TypingIndicator>` row above the composer per spec
    /// §Typing indicator. The local peer is already filtered out by
    /// `Client::typing_in`. Defaults to an empty vec so unit tests
    /// don't have to wire the polling loop.
    #[prop(optional, into)]
    typing_peers: Option<Signal<Vec<String>>>,
    /// Full list of `@`-mention candidates for the current channel,
    /// resolved by the parent via
    /// [`willow_client::Client::mention_candidates`]. The composer
    /// applies [`willow_client::mentions::Suggestions::filter`]
    /// against the live `@`-query before rendering. Defaults to an
    /// empty vec so the popover stays inert in tests that don't need
    /// it. Spec: `composer.md` §Mention autocomplete.
    #[prop(optional, into)]
    mention_candidates: Option<Signal<Vec<MentionCandidate>>>,
    /// `true` when the local peer holds `Permission::ManageChannels`
    /// in the active server. When set, a synthetic `@channel` row is
    /// prepended to the candidate list so the popover can offer
    /// "everyone in this channel" alongside the per-peer rows. Spec
    /// line 104-105: "Special row `@channel` (mentions all members)
    /// visible only with `ManageChannels`." Defaults to `false`.
    #[prop(optional, into)]
    allow_channel_mention: Option<Signal<bool>>,
) -> impl IntoView {
    let (input_text, set_input_text) = signal(String::new());
    let textarea_ref = NodeRef::<leptos::html::Textarea>::new();

    // Resolve optional context props to concrete signals so the meta
    // row + offline tint + placeholder copy don't have to branch on
    // `Option<…>` at every read. Tests that mount `<Composer>` with
    // just `on_send` get sensible online defaults.
    let (default_connection, _) = signal(ConnectionState::Connected);
    let connection_sig: ReadSignal<ConnectionState> = connection.unwrap_or(default_connection);
    let peer_count_sig: Signal<usize> = peer_count.unwrap_or_else(|| Signal::derive(|| 0));
    let channel_kind_sig: Signal<ChannelKind> =
        channel_kind.unwrap_or_else(|| Signal::derive(|| ChannelKind::Text));
    let channel_name_sig: Signal<String> =
        channel_name.unwrap_or_else(|| Signal::derive(String::new));
    let recipient_name_sig: Signal<Option<String>> =
        recipient_name.unwrap_or_else(|| Signal::derive(|| None));
    let typing_peers_sig: Signal<Vec<String>> =
        typing_peers.unwrap_or_else(|| Signal::derive(Vec::new));
    let mention_candidates_sig: Signal<Vec<MentionCandidate>> =
        mention_candidates.unwrap_or_else(|| Signal::derive(Vec::new));
    let allow_channel_mention_sig: Signal<bool> =
        allow_channel_mention.unwrap_or_else(|| Signal::derive(|| false));
    // `data-shell` is set once at mount by `mobile_shell` / the
    // wasm-pack test harness; deriving a Signal still works because the
    // closure runs each time a downstream reader subscribes.
    let is_mobile_sig: Signal<bool> = Signal::derive(is_mobile_shell);

    // ── Mention autocomplete state (T13) ─────────────────────────────
    //
    // Owned at the composer level because the popover, the textarea
    // input handler, and the keydown handler all need to read or
    // mutate it. Kept on local `RwSignal`s — no global state needed.
    //
    // - `autocomplete_open`: `true` while the user is mid-`@`-token.
    // - `mention_query`: lowercase prefix typed after the `@`.
    // - `mention_at_pos`: byte offset of the active `@` in the
    //   textarea. Used by the splice path so the original `@` is
    //   consumed when a candidate is inserted.
    // - `mention_selected`: highlighted row index inside the popover.
    // - `mention_anchor`: `(top, left)` pixel anchor for the popover,
    //   recomputed on each open. v1 anchors to the textarea's top-
    //   left rather than the actual `@` glyph (TODO above the popover
    //   component documents the trade-off).
    let autocomplete_open = RwSignal::new(false);
    let mention_query = RwSignal::new(String::new());
    let mention_at_pos = RwSignal::new(0usize);
    let mention_selected = RwSignal::new(0usize);
    let mention_anchor = RwSignal::new((0i32, 0i32));

    let filtered_candidates: Memo<Vec<MentionCandidate>> = Memo::new(move |_| {
        if !autocomplete_open.get() {
            return Vec::new();
        }
        let q = mention_query.get();
        let mut all = mention_candidates_sig.get();
        // Prepend the synthetic `@channel` row when the local peer is
        // allowed to address everyone (T14 — spec line 104-105). The
        // ranker treats it as a normal handle-prefix candidate, so an
        // empty query lists it first (alphabetical "channel" sorts
        // before most peer handles) and a query like `c` filters via
        // the same prefix path the per-peer rows use.
        if allow_channel_mention_sig.get() {
            let mut next = Vec::with_capacity(all.len() + 1);
            next.push(super::mention_autocomplete::channel_mention_candidate(
                synthetic_channel_peer_id(),
            ));
            next.append(&mut all);
            all = next;
        }
        Suggestions::filter(&q, &all)
    });
    let filtered_signal: Signal<Vec<MentionCandidate>> =
        Signal::derive(move || filtered_candidates.get());

    // Reset the highlighted index whenever the filtered list shrinks
    // below the current selection (e.g. the user typed another letter
    // and the matching set narrowed). Keeps Enter / Tab from
    // selecting a row that no longer exists.
    Effect::new(move |_| {
        let n = filtered_candidates.get().len();
        if n == 0 {
            mention_selected.set(0);
            return;
        }
        if mention_selected.get_untracked() >= n {
            mention_selected.set(0);
        }
    });

    // Placeholder copy is recomputed by `Memo` so it tracks every
    // input — channel name, peer count, kind, recipient, connection.
    // Spec source-of-truth: `composer.md` §Composer placeholders.
    let placeholder = Memo::new(move |_| {
        let kind = channel_kind_sig.get();
        let name = channel_name_sig.get();
        let recipient = recipient_name_sig.get();
        let count = peer_count_sig.get();
        let conn = connection_sig.get();
        placeholder_for(kind, &name, recipient.as_deref(), count, conn)
    });

    // Offline tint flips the wrapper class when connection drops out.
    // Spec §Offline state: background softens to
    // `color-mix(in oklab, var(--amber) 10%, var(--bg-2))`. Reconnecting
    // keeps the tint on for the same reason the meta row does — the
    // user's send still routes to the queue and can't reach peers.
    let wrapper_class = move || {
        let mut classes = String::from("composer input-area");
        if matches!(
            connection_sig.get(),
            ConnectionState::Offline | ConnectionState::Reconnecting
        ) {
            classes.push_str(" composer--offline");
        }
        classes
    };

    // When `editing` becomes `Some`, pre-fill the textarea with the
    // message body. Mirrors the legacy `<ChatInput>` behaviour so the
    // edit affordance keeps working through the swap.
    if let Some(editing_sig) = editing {
        let set_text = set_input_text;
        Effect::new(move |_| {
            if let Some(msg) = editing_sig.get() {
                set_text.set(msg.body.clone());
            }
        });
    }

    // Autogrow: every time `input_text` changes — including the reset
    // to empty after submit — re-measure `scrollHeight` and clamp to
    // `MAX_VISIBLE_LINES * line-height`. Resetting the inline height
    // before reading `scrollHeight` is required for shrink-back to
    // work; otherwise `scrollHeight` stays at the last grown size.
    Effect::new(move |_| {
        let _ = input_text.get();
        if let Some(el) = textarea_ref.get() {
            // Use the inherent `web_sys` method — Leptos' `.style()`
            // takes a style argument, the DOM `.style` is a property.
            let dom: &web_sys::HtmlTextAreaElement = &el;
            let style = web_sys::HtmlElement::style(dom);
            // Reset to `auto` so `scrollHeight` reflects the natural
            // content size, not the previously-grown height.
            let _ = style.set_property("height", "auto");
            let line_height = parse_line_height_px(dom).unwrap_or(21.0);
            let max_h = line_height * MAX_VISIBLE_LINES;
            let scroll_h = dom.scroll_height() as f64;
            let target = scroll_h.min(max_h);
            let _ = style.set_property("height", &format!("{target}px"));
        }
    });

    // Full keydown table per `composer.md` §Keyboard:
    //
    //   Ctrl|Cmd + Enter        → force-send (both shells).
    //   Enter (no modifiers)    → send (both shells).
    //   Shift + Enter           → newline (default — no preventDefault).
    //   Escape                  → unwind: edit → reply → blur.
    //   Tab inside textarea     → insert two spaces (no focus move).
    //   ArrowUp on empty input  → fire `on_arrow_up_edit` (parent decides).
    //   `@` and other keys      → fall through (mention autocomplete + IME
    //                             land in T13).
    let cancel_reply_cb = on_cancel_reply;
    let cancel_edit_cb = on_cancel_edit;
    let edit_send_cb = on_edit_send;
    let editing_for_keydown = editing;
    let replying_for_keydown = replying_to;
    let arrow_up_cb = on_arrow_up_edit;
    let on_send_clone = on_send.clone();

    let submit = move || {
        let text = input_text.get_untracked();
        if text.trim().is_empty() {
            return;
        }
        let is_editing = editing_for_keydown
            .map(|sig| sig.get_untracked().is_some())
            .unwrap_or(false);
        if is_editing {
            if let Some(sig) = editing_for_keydown {
                if let Some(msg) = sig.get_untracked() {
                    if let Some(ref cb) = edit_send_cb {
                        cb.run((msg.id.clone(), text));
                    }
                }
            }
        } else {
            on_send_clone(text);
        }
        set_input_text.set(String::new());
    };

    // Splice the selected mention into the textarea, replacing the
    // span from the active `@` through the caret with `@{handle} `.
    // Closes the popover and resets the highlighted index. Captured
    // by the popover's `on_select` callback and by the keydown
    // handler's Enter / Tab paths.
    let insert_selected_mention = move |candidate: &MentionCandidate| {
        let Some(el) = textarea_ref.get_untracked() else {
            return;
        };
        let dom: &web_sys::HtmlTextAreaElement = &el;
        let value = dom.value();
        let at_byte = mention_at_pos.get_untracked();
        // Caret is wherever the user last left it. We collapse the
        // range to a single point — autocomplete fires on input
        // events, which always leave the selection collapsed.
        let caret_chars = dom.selection_end().ok().flatten().unwrap_or(0) as usize;
        let caret_byte = char_index_to_byte(&value, caret_chars).unwrap_or(value.len());
        let at_byte = at_byte.min(value.len());
        let end = caret_byte.max(at_byte);
        let replacement = format!("@{} ", candidate.handle);
        let mut new_value = String::with_capacity(value.len() + replacement.len());
        new_value.push_str(&value[..at_byte]);
        new_value.push_str(&replacement);
        new_value.push_str(&value[end..]);
        let new_caret_chars = byte_index_to_char(&new_value, at_byte + replacement.len())
            .unwrap_or_else(|| new_value.chars().count()) as u32;
        dom.set_value(&new_value);
        set_input_text.set(new_value);
        let _ = dom.set_selection_range(new_caret_chars, new_caret_chars);
        autocomplete_open.set(false);
        mention_query.set(String::new());
        mention_selected.set(0);
    };

    let on_select_mention = Callback::new(move |c: MentionCandidate| {
        insert_selected_mention(&c);
    });

    let submit_for_key = submit.clone();
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        let key = ev.key();

        // ── Autocomplete keyboard handling ──────────────────────────
        //
        // When the popover is open the navigation / commit / dismiss
        // keys belong to the autocomplete, not the composer. Enter
        // and Tab insert the selected handle (and *do not* send the
        // message); Escape dismisses without changing the textarea;
        // Arrow keys move the highlight with wrap. Spec lines 102–103.
        if autocomplete_open.get_untracked() {
            let n = filtered_candidates.get_untracked().len();
            match key.as_str() {
                "ArrowDown" if n > 0 => {
                    ev.prevent_default();
                    let next = (mention_selected.get_untracked() + 1) % n;
                    mention_selected.set(next);
                    return;
                }
                "ArrowUp" if n > 0 => {
                    ev.prevent_default();
                    let cur = mention_selected.get_untracked();
                    let prev = if cur == 0 { n - 1 } else { cur - 1 };
                    mention_selected.set(prev);
                    return;
                }
                "Enter" | "Tab" if n > 0 => {
                    ev.prevent_default();
                    ev.stop_propagation();
                    let idx = mention_selected.get_untracked().min(n - 1);
                    let candidate = filtered_candidates.get_untracked()[idx].clone();
                    insert_selected_mention(&candidate);
                    return;
                }
                "Escape" => {
                    ev.prevent_default();
                    autocomplete_open.set(false);
                    mention_query.set(String::new());
                    mention_selected.set(0);
                    return;
                }
                _ => {}
            }
        }

        let force_send = (ev.ctrl_key() || ev.meta_key()) && key == "Enter";

        if force_send {
            // Ctrl/Cmd+Enter: force-send regardless of shell. Spec
            // §Keyboard (desktop): "for users who prefer Enter as
            // newline" — also the only modal way to submit on mobile
            // when the textarea is multi-line.
            ev.prevent_default();
            submit_for_key();
            return;
        }

        if key == "Enter" {
            if ev.shift_key() {
                // Spec §Keyboard: Shift+Enter inserts a newline. Let
                // the browser do the default insert.
                return;
            }
            // Plain Enter sends on both shells.
            ev.prevent_default();
            submit_for_key();
            return;
        }

        if key == "Escape" {
            // Spec §Keyboard: "unwinds in order: cancel edit → cancel
            // reply → blur." Each Esc press performs the next step.
            let editing_active = editing_for_keydown
                .map(|sig| sig.get_untracked().is_some())
                .unwrap_or(false);
            if editing_active {
                ev.prevent_default();
                if let Some(ref cb) = cancel_edit_cb {
                    cb.run(());
                }
                set_input_text.set(String::new());
                return;
            }
            let replying_active = replying_for_keydown
                .map(|sig| sig.get_untracked().is_some())
                .unwrap_or(false);
            if replying_active {
                ev.prevent_default();
                if let Some(ref cb) = cancel_reply_cb {
                    cb.run(());
                }
                return;
            }
            // Nothing to unwind — blur the textarea so the next Esc
            // exits the surface entirely. Pulled from the event
            // target rather than the captured ref so we still blur
            // when the keydown bubbles from a child element.
            if let Some(target) = ev.target() {
                if let Ok(el) = target.dyn_into::<web_sys::HtmlElement>() {
                    let _ = el.blur();
                }
            }
            return;
        }

        if key == "Tab" {
            // Spec §Keyboard: "Tab inside textarea inserts two spaces
            // (no focus move)." Splice at the current selection so
            // the user's caret stays anchored after the inserted
            // pair.
            if let Some(target) = ev.target() {
                if let Ok(el) = target.dyn_into::<web_sys::HtmlTextAreaElement>() {
                    ev.prevent_default();
                    insert_at_caret(&el, "  ");
                    set_input_text.set(el.value());
                }
            }
            return;
        }

        if key == "ArrowUp" {
            // Spec §Keyboard: "ArrowUp when textarea empty → enters
            // edit mode on most recent own message." Only fires when
            // the textarea is empty *and* the caret is at position 0
            // — otherwise the user is navigating within a multi-line
            // draft and the default browser behaviour wins.
            if !input_text.get_untracked().is_empty() {
                return;
            }
            if let Some(ref cb) = arrow_up_cb {
                ev.prevent_default();
                cb.run(());
            }
        }
    };

    let submit_for_click = submit.clone();
    let on_click_send = move |_ev: web_sys::MouseEvent| submit_for_click();

    // Send-button label flips to `save` while editing.
    let send_label = move || {
        let is_editing = editing.map(|sig| sig.get().is_some()).unwrap_or(false);
        if is_editing {
            "save"
        } else {
            "send"
        }
    };

    // Aria-label mirrors the visible label so screen-reader users hear
    // the same state sighted users see — `send` normally, `save` while
    // editing. The spec's accessibility table (line 235-241) lists `send`
    // for the default state; the flip during edit mode is an intentional
    // extension so AT users aren't told the button still says "send"
    // when it visibly reads "save". Recorded in plan T15 as the chosen
    // resolution to the spec's silence on the edit-mode label.
    let send_aria_label = move || {
        let is_editing = editing.map(|sig| sig.get().is_some()).unwrap_or(false);
        if is_editing {
            "save"
        } else {
            "send"
        }
    };

    let send_disabled = move || input_text.get().trim().is_empty();

    // Attach + emoji buttons: stub click handlers per plan §Ambiguity
    // decisions point 5. The actual file dialog (`files-inline.md`,
    // Phase 3b) and emoji picker (`reactions-pins.md`, Phase 3c) drop
    // in here without re-shaping props — the buttons render now with
    // the spec-mandated aria-labels so screen readers can already
    // discover them, and the click handlers are intentional no-ops.
    let on_click_attach = |_ev: web_sys::MouseEvent| {
        // TODO(files-inline.md): open file dialog.
    };
    let on_click_emoji = |_ev: web_sys::MouseEvent| {
        // TODO(reactions-pins.md): open emoji picker popover.
    };

    view! {
        // `input-area` retained alongside `composer` for backward
        // compatibility with the existing CSS + focus-back JS;
        // T9–T15 will port those onto the `composer` namespace.
        // `composer--offline` is appended reactively when the device
        // or relay has dropped out — drives the spec's amber tint per
        // §Offline state.
        <div class=wrapper_class>
            // Typing indicator — sits at the very top of the composer
            // wrapper per spec §Typing indicator. The component
            // collapses itself when `typing_peers` is empty so the
            // row doesn't claim layout while no one is typing.
            <TypingIndicator peers=typing_peers_sig />
            // Edit bar — full spec layout per `composer.md` §Edit mode.
            // The send-button label flip to `save` is owned below; this
            // bar only renders the hint + cancel affordance.
            {move || {
                let sig = editing?;
                let cancel = on_cancel_edit.unwrap_or_else(|| Callback::new(|_| ()));
                Some(view! { <EditBar editing=sig on_cancel=cancel /> })
            }}
            // Reply bar — full spec layout per `composer.md` §Reply
            // preview. Suppressed while edit mode is active so the
            // composer never shows two stacked context bars.
            {move || {
                let is_editing = editing
                    .map(|sig| sig.get().is_some())
                    .unwrap_or(false);
                if is_editing {
                    return None;
                }
                let sig = replying_to?;
                // Fall back to a no-op so the bar still renders when
                // the parent didn't supply a cancel handler. The
                // composer's keydown handler also drives cancellation
                // via `Escape`, which goes through `on_cancel_reply`
                // independently.
                let cancel = on_cancel_reply.unwrap_or_else(|| Callback::new(|_| ()));
                // `on_jump_to_parent` is optional on `<ReplyBar>` —
                // splitting the two render paths keeps the typed-builder
                // happy without forcing every callsite to pass a stub.
                Some(match on_jump_to_parent {
                    Some(jump) => view! {
                        <ReplyBar
                            replying_to=sig
                            on_cancel=cancel
                            on_jump_to_parent=jump
                        />
                    }
                    .into_any(),
                    None => view! {
                        <ReplyBar
                            replying_to=sig
                            on_cancel=cancel
                        />
                    }
                    .into_any(),
                })
            }}
            <div class="composer__row">
                // Attach button (spec §Desktop compose surface, upper
                // row). Clicks are stubs in v1 — Phase 3b
                // (`files-inline.md`) wires the file dialog. The
                // `aria-label="attach file"` matches the spec's
                // accessibility table.
                <button
                    type="button"
                    class="composer__attach"
                    aria-label="attach file"
                    on:click=on_click_attach
                >
                    <span aria-hidden="true">{crate::icons::icon_plus()}</span>
                </button>
                <textarea
                    class="composer__textarea"
                    node_ref=textarea_ref
                    rows="1"
                    placeholder=move || placeholder.get()
                    prop:value=move || input_text.get()
                    on:input=move |ev| {
                        let value = event_target_value(&ev);
                        set_input_text.set(value.clone());
                        if let Some(ref cb) = on_typing {
                            cb.run(());
                        }
                        // Recompute mention-autocomplete state.
                        // Anchor relative to the textarea's bounding
                        // rect so the popover follows the composer if
                        // the page scrolls. v1 anchors to the
                        // textarea's top-left (TODO: mirror-based @
                        // glyph anchor).
                        let target = ev.target().and_then(|t| {
                            t.dyn_into::<web_sys::HtmlTextAreaElement>().ok()
                        });
                        let caret_chars = target
                            .as_ref()
                            .and_then(|el| el.selection_end().ok().flatten())
                            .map(|n| n as usize)
                            .unwrap_or_else(|| value.chars().count());
                        match find_at_word_boundary(&value, caret_chars) {
                            Some((at_byte, query)) => {
                                autocomplete_open.set(true);
                                mention_at_pos.set(at_byte);
                                mention_query.set(query);
                                if let Some(el) = target.as_ref() {
                                    let rect = el.get_bounding_client_rect();
                                    // -8px offset above the composer per
                                    // spec line 97. We bias `top` by
                                    // -200 (popover max-height) so the
                                    // popover lays out above the
                                    // textarea regardless of viewport
                                    // overflow handling.
                                    let top = (rect.top() - 8.0 - 200.0) as i32;
                                    let left = rect.left() as i32;
                                    mention_anchor.set((top, left));
                                }
                            }
                            None => {
                                autocomplete_open.set(false);
                                mention_query.set(String::new());
                            }
                        }
                    }
                    on:keydown=on_keydown
                />
                // Emoji button (spec §Desktop compose surface, upper
                // row). Clicks are stubs in v1 — Phase 3c
                // (`reactions-pins.md`) wires the emoji popover. The
                // `aria-label="open emoji picker"` matches the spec's
                // accessibility table.
                <button
                    type="button"
                    class="composer__emoji"
                    aria-label="open emoji picker"
                    on:click=on_click_emoji
                >
                    <span aria-hidden="true">{crate::icons::icon_smile()}</span>
                </button>
                <button
                    type="button"
                    class="composer__send"
                    aria-label=send_aria_label
                    prop:disabled=send_disabled
                    on:click=on_click_send
                >
                    {send_label}
                </button>
            </div>
            <MentionAutocomplete
                candidates=filtered_signal
                selected=mention_selected
                on_select=on_select_mention
                anchor=Signal::derive(move || mention_anchor.get())
            />
            <MetaRow
                connection=connection_sig
                peer_count=peer_count_sig
                is_mobile=is_mobile_sig
            />
        </div>
    }
}

/// Read the textarea's computed `line-height` in CSS pixels. Falls
/// back to `None` if the value can't be parsed (e.g. `normal`); the
/// caller substitutes a sensible default.
fn parse_line_height_px(el: &web_sys::HtmlTextAreaElement) -> Option<f64> {
    let window = web_sys::window()?;
    let style = window.get_computed_style(el).ok()??;
    let raw = style.get_property_value("line-height").ok()?;
    let trimmed = raw.trim_end_matches("px");
    trimmed.parse::<f64>().ok()
}

/// Returns true when the document root is rendering under the mobile
/// shell. Reads `<html data-shell="mobile">`, set by
/// `mobile_shell` at runtime and by the wasm-pack `mount_test_with_shell`
/// helper in tests. Returns false (i.e. desktop) when the attribute is
/// absent so SSR / unit harnesses default to the desktop send path.
fn is_mobile_shell() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Some(doc) = window.document() else {
        return false;
    };
    let Some(root) = doc.document_element() else {
        return false;
    };
    matches!(root.get_attribute("data-shell").as_deref(), Some("mobile"))
}

/// Stable placeholder `EndpointId` for the synthetic `@channel`
/// candidate row. `EndpointId` is an Ed25519 public key so we can't
/// fabricate one from zero bytes; we generate a fresh identity once
/// and cache it for the lifetime of the process. The composer never
/// uses this id for routing — selection writes the handle into the
/// textarea, not the peer id — so the placeholder is purely a slot
/// in the `MentionCandidate` struct.
fn synthetic_channel_peer_id() -> EndpointId {
    static SENTINEL: OnceLock<EndpointId> = OnceLock::new();
    *SENTINEL.get_or_init(|| willow_identity::Identity::generate().endpoint_id())
}

/// Find the active `@`-mention token in `value` at the given caret
/// position, if any. Returns `(at_byte_offset, lowercase_query)` when
/// the caret sits after an `@` that is itself either at the start of
/// the buffer or preceded by whitespace, with no whitespace between
/// the `@` and the caret.
///
/// Spec `composer.md` §Mention autocomplete: "triggered on `@` at a
/// word boundary." Word boundary in v1 = start of buffer or any
/// whitespace character (space / tab / newline). Punctuation like
/// `foo@bar` does NOT trigger autocomplete because the `@` follows a
/// non-whitespace, non-start glyph.
///
/// Returns `None` when:
/// - There is no `@` between the caret and the most recent
///   whitespace.
/// - The `@` is preceded by a non-whitespace character (so it's an
///   email-like fragment, not a mention).
/// - Whitespace appears between the `@` and the caret (the boundary
///   has been broken).
pub(crate) fn find_at_word_boundary(value: &str, caret_chars: usize) -> Option<(usize, String)> {
    // Truncate the value at the caret's char position. We work on
    // chars (not bytes) because the caller talks to us in selection-
    // index units, then convert the discovered `@` position back to
    // a byte offset for splice math.
    let prefix: String = value.chars().take(caret_chars).collect();
    let mut at_char_idx: Option<usize> = None;
    for (idx, ch) in prefix.char_indices().rev() {
        if ch == '@' {
            // Boundary check: previous char must be whitespace, or
            // this must be the start of the buffer.
            let preceded_ok = idx == 0
                || prefix[..idx]
                    .chars()
                    .last()
                    .map(|c| c.is_whitespace())
                    .unwrap_or(true);
            if preceded_ok {
                at_char_idx = Some(idx);
            }
            break;
        }
        if ch.is_whitespace() {
            // Hit whitespace before any `@` — boundary is broken.
            return None;
        }
    }
    let at_byte = at_char_idx?;
    let after_at = &prefix[at_byte + 1..];
    Some((at_byte, after_at.to_lowercase()))
}

/// Convert a UTF-16 / char-count selection index (as returned by
/// `selection_end`) to a byte offset into `value`. We treat the
/// JS-side index as a character count, which is exact for the BMP
/// range but loses precision for astral codepoints — composer
/// content is overwhelmingly BMP so this is the simplest approach.
///
/// Returns `None` when the index is out of bounds.
pub(crate) fn char_index_to_byte(value: &str, char_idx: usize) -> Option<usize> {
    if char_idx == 0 {
        return Some(0);
    }
    let mut count = 0usize;
    for (byte_idx, _) in value.char_indices() {
        if count == char_idx {
            return Some(byte_idx);
        }
        count += 1;
    }
    if count == char_idx {
        Some(value.len())
    } else {
        None
    }
}

/// Inverse of [`char_index_to_byte`]: convert a byte offset to a
/// char-count index suitable for `set_selection_range`. Returns
/// `None` when the byte index is past `value.len()`.
pub(crate) fn byte_index_to_char(value: &str, byte_idx: usize) -> Option<usize> {
    if byte_idx > value.len() {
        return None;
    }
    Some(value[..byte_idx].chars().count())
}

/// Insert `text` at the textarea's current caret position, replacing
/// any active selection, and advance the caret by `text.len()`. Used
/// for the Tab → 2-spaces affordance and (later) mention insertion.
/// Caret math uses `selection_start` / `selection_end`; falls back to
/// `len()` when the browser can't report a valid range (rare — happens
/// when the textarea is detached from layout).
fn insert_at_caret(el: &web_sys::HtmlTextAreaElement, text: &str) {
    let value = el.value();
    let len = value.chars().count() as u32;
    let start = el.selection_start().ok().flatten().unwrap_or(len);
    let end = el.selection_end().ok().flatten().unwrap_or(start);
    let s = start as usize;
    let e = end.max(start) as usize;
    let s = s.min(value.len());
    let e = e.min(value.len());
    let mut new_value = String::with_capacity(value.len() + text.len());
    new_value.push_str(&value[..s]);
    new_value.push_str(text);
    new_value.push_str(&value[e..]);
    el.set_value(&new_value);
    let new_caret = (s + text.len()) as u32;
    let _ = el.set_selection_range(new_caret, new_caret);
}
