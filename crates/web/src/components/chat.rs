use leptos::prelude::*;
use willow_client::ChatMessage;

use super::MessageView;

/// Header bar showing the current channel name and connected peer count.
#[component]
pub fn ChannelHeader(
    channel: ReadSignal<String>,
    peer_count: ReadSignal<usize>,
    on_menu_click: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    view! {
        <div class="channel-header">
            <button class="mobile-nav-toggle" on:click=move |_| on_menu_click(())>
                "="
            </button>
            <span>"# " {move || channel.get()}</span>
            <span class="peer-count">
                {move || {
                    let n = peer_count.get();
                    if n == 1 { "1 peer".to_string() } else { format!("{n} peers") }
                }}
            </span>
        </div>
    }
}

/// Scrollable message list for the current channel.
/// Auto-scrolls to bottom when new messages arrive if the user
/// is already at (or near) the bottom.
#[component]
pub fn MessageList(messages: ReadSignal<Vec<ChatMessage>>) -> impl IntoView {
    let list_ref = NodeRef::<leptos::html::Div>::new();

    // When messages change, check if we should auto-scroll.
    Effect::new(move |prev_len: Option<usize>| {
        let msgs = messages.get();
        let len = msgs.len();

        if let Some(el) = list_ref.get() {
            let el: &web_sys::HtmlElement = &el;
            let scroll_top = el.scroll_top() as f64;
            let scroll_height = el.scroll_height() as f64;
            let client_height = el.client_height() as f64;

            // Auto-scroll if: this is the first render, new messages arrived,
            // OR the user was within 100px of the bottom.
            let was_at_bottom = (scroll_height - scroll_top - client_height) < 100.0;
            let is_new = prev_len.map(|p| len > p).unwrap_or(true);

            if was_at_bottom || is_new {
                el.set_scroll_top(el.scroll_height());
            }
        }

        len
    });

    view! {
        <div class="message-list" node_ref=list_ref>
            {move || {
                let msgs = messages.get();
                if msgs.is_empty() {
                    view! {
                        <div class="empty-state">
                            "No messages yet. Say hello!"
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <For
                            each=move || messages.get()
                            key=|m| m.id.clone()
                            let:msg
                        >
                            <MessageView message=msg />
                        </For>
                    }.into_any()
                }
            }}
        </div>
    }
}
