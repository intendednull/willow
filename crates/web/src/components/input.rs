use leptos::prelude::*;
use willow_client::ChatMessage;

/// Chat input field. Sends a message on Enter (without Shift).
/// When `replying_to` is set, shows a reply preview bar above the input and
/// pressing Escape cancels the reply.
#[component]
pub fn ChatInput(
    on_send: impl Fn(String) + Send + Clone + 'static,
    /// The message being replied to (if any).
    #[prop(optional, into)]
    replying_to: Option<ReadSignal<Option<ChatMessage>>>,
    /// Callback to cancel the current reply.
    #[prop(optional, into)]
    on_cancel_reply: Option<Callback<()>>,
) -> impl IntoView {
    let (input_text, set_input_text) = signal(String::new());

    let cancel_cb = on_cancel_reply.clone();
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" && !ev.shift_key() {
            ev.prevent_default();
            let text = input_text.get_untracked();
            if !text.trim().is_empty() {
                on_send(text);
                set_input_text.set(String::new());
            }
        } else if ev.key() == "Escape" {
            if let Some(ref cb) = cancel_cb {
                cb.run(());
            }
        }
    };

    view! {
        <div class="input-area">
            {move || {
                replying_to.and_then(|sig| {
                    let msg = sig.get();
                    let cancel = on_cancel_reply.clone();
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
