use leptos::prelude::*;
use willow_client::ChatMessage;

/// Chat input field. Sends a message on Enter (without Shift).
/// When `replying_to` is set, shows a reply preview bar above the input and
/// pressing Escape cancels the reply.
/// When `editing` is set, shows an "Editing message" bar above the input,
/// pre-fills the input with the original message body, and sends via the
/// edit callback instead of the normal send path.
#[component]
pub fn ChatInput(
    on_send: impl Fn(String) + Send + Clone + 'static,
    /// The message being replied to (if any).
    #[prop(optional, into)]
    replying_to: Option<ReadSignal<Option<ChatMessage>>>,
    /// Callback to cancel the current reply.
    #[prop(optional, into)]
    on_cancel_reply: Option<Callback<()>>,
    /// The message currently being edited (if any).
    #[prop(optional, into)]
    editing: Option<ReadSignal<Option<ChatMessage>>>,
    /// Callback fired when the user submits the edited message (message_id, new_body).
    #[prop(optional, into)]
    on_edit_send: Option<Callback<(String, String)>>,
    /// Callback to cancel the current edit.
    #[prop(optional, into)]
    on_cancel_edit: Option<Callback<()>>,
) -> impl IntoView {
    let (input_text, set_input_text) = signal(String::new());

    // When the `editing` signal becomes Some, pre-fill the input with the body.
    if let Some(editing_sig) = editing {
        let set_text = set_input_text;
        Effect::new(move |_| {
            if let Some(msg) = editing_sig.get() {
                set_text.set(msg.body.clone());
            }
        });
    }

    let cancel_reply_cb = on_cancel_reply;
    let cancel_edit_cb = on_cancel_edit;
    let edit_send_cb = on_edit_send;
    let editing_for_keydown = editing;

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            let text = input_text.get_untracked();
            if !text.trim().is_empty() {
                // Check if we are in edit mode.
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
                    on_send(text);
                }
                set_input_text.set(String::new());
            }
        } else if ev.key() == "Escape" {
            // Cancel edit takes priority over cancel reply.
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

    view! {
        <div class="input-area">
            // Editing bar -- shown when editing a message.
            {move || {
                editing.and_then(|sig| {
                    let msg = sig.get();
                    let cancel = on_cancel_edit;
                    msg.map(|m| {
                        let preview = if m.body.len() > 60 {
                            format!("{}...", &m.body[..60])
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
            // Reply bar -- shown when replying to a message (only if not editing).
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
                        let preview = if m.body.len() > 60 {
                            format!("{}...", &m.body[..60])
                        } else {
                            m.body.clone()
                        };
                        view! {
                            <div class="reply-bar">
                                <span class="reply-bar-text">
                                    {format!("Replying to {}: {}", m.author, preview)}
                                </span>
                                <button
                                    class="reply-bar-cancel"
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
            <input
                type="text"
                placeholder="Message #channel"
                prop:value=move || input_text.get()
                on:input=move |ev| {
                    set_input_text.set(event_target_value(&ev));
                }
                on:keydown=on_keydown
            />
        </div>
    }
}
