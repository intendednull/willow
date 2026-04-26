//! `<Composer>` parent component — Phase 3a shell.
//!
//! T5 ships the minimum-viable shell: an autogrow textarea + a send
//! button that mirrors the existing `<ChatInput>` prop contract so
//! callsites only swap the component name. Reply / edit bars carry
//! over the legacy markup unchanged here; T7 / T8 restyle them per
//! `docs/specs/2026-04-19-ui-design/composer.md`. Full keybinding
//! semantics, autocomplete, meta row, typing indicator, offline tint,
//! and per-channel-kind placeholder copy land in T6 onward.
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

use leptos::prelude::*;
use willow_client::DisplayMessage;

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
) -> impl IntoView {
    let (input_text, set_input_text) = signal(String::new());
    let textarea_ref = NodeRef::<leptos::html::Textarea>::new();

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

    // Submit-on-Enter / Escape unwind. Full kbd table (Shift+Enter
    // newline, Ctrl/Cmd+Enter force-send, Tab→2 spaces, ArrowUp edit)
    // lands in T6.
    let cancel_reply_cb = on_cancel_reply;
    let cancel_edit_cb = on_cancel_edit;
    let edit_send_cb = on_edit_send;
    let editing_for_keydown = editing;
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

    let submit_for_key = submit.clone();
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            submit_for_key();
        } else if ev.key() == "Escape" {
            // Edit cancel takes priority over reply cancel — matches
            // the legacy `<ChatInput>` semantics and `composer.md`
            // §Keyboard "unwinds in order: cancel edit → cancel reply".
            let is_editing = editing_for_keydown
                .map(|sig| sig.get_untracked().is_some())
                .unwrap_or(false);
            if is_editing {
                if let Some(ref cb) = cancel_edit_cb {
                    cb.run(());
                }
                set_input_text.set(String::new());
            } else if let Some(ref cb) = cancel_reply_cb {
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

    let send_disabled = move || input_text.get().trim().is_empty();

    view! {
        // `input-area` retained alongside `composer` for backward
        // compatibility with the existing CSS + focus-back JS;
        // T9–T15 will port those onto the `composer` namespace.
        <div class="composer input-area">
            // Edit bar — visual port from legacy `<ChatInput>`. T8
            // restyles per spec.
            {move || {
                editing.and_then(|sig| {
                    let msg = sig.get();
                    let cancel = on_cancel_edit;
                    msg.map(|m| {
                        let preview = if m.body.chars().count() > 60 {
                            format!("{}...", m.body.chars().take(60).collect::<String>())
                        } else {
                            m.body.clone()
                        };
                        view! {
                            <div class="edit-bar">
                                <span class="edit-bar-text">
                                    {format!("Editing: {}", preview)}
                                </span>
                                <button
                                    class="edit-bar-cancel"
                                    aria-label="cancel edit"
                                    on:click=move |_| {
                                        if let Some(ref cb) = cancel {
                                            cb.run(());
                                        }
                                    }
                                >
                                    "x"
                                </button>
                            </div>
                        }
                    })
                })
            }}
            // Reply bar — visual port from legacy `<ChatInput>`. T7
            // restyles per spec.
            {move || {
                let is_editing = editing
                    .map(|sig| sig.get().is_some())
                    .unwrap_or(false);
                if is_editing {
                    return None;
                }
                replying_to.and_then(|sig| {
                    let msg = sig.get();
                    let cancel = on_cancel_reply;
                    msg.map(|m| {
                        let preview = if m.body.chars().count() > 60 {
                            format!("{}...", m.body.chars().take(60).collect::<String>())
                        } else {
                            m.body.clone()
                        };
                        view! {
                            <div class="reply-bar">
                                <span class="reply-bar-text">
                                    {format!("Replying to {}: {}", m.author_display_name, preview)}
                                </span>
                                <button
                                    class="reply-bar-cancel"
                                    aria-label="cancel reply"
                                    on:click=move |_| {
                                        if let Some(ref cb) = cancel {
                                            cb.run(());
                                        }
                                    }
                                >
                                    "x"
                                </button>
                            </div>
                        }
                    })
                })
            }}
            <div class="composer__row">
                <textarea
                    class="composer__textarea"
                    node_ref=textarea_ref
                    rows="1"
                    placeholder="message #channel"
                    prop:value=move || input_text.get()
                    on:input=move |ev| {
                        set_input_text.set(event_target_value(&ev));
                        if let Some(ref cb) = on_typing {
                            cb.run(());
                        }
                    }
                    on:keydown=on_keydown
                />
                <button
                    class="composer__send"
                    aria-label="send"
                    prop:disabled=send_disabled
                    on:click=on_click_send
                >
                    {send_label}
                </button>
            </div>
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
