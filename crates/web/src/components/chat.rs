use leptos::prelude::*;
use willow_client::ChatMessage;

use super::MessageView;

/// Header bar showing the current channel name and connected peer count.
#[component]
pub fn ChannelHeader(channel: ReadSignal<String>, peer_count: ReadSignal<usize>) -> impl IntoView {
    view! {
        <div class="channel-header">
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
#[component]
pub fn MessageList(messages: ReadSignal<Vec<ChatMessage>>) -> impl IntoView {
    view! {
        <div class="message-list">
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
