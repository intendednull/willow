use leptos::prelude::*;
use willow_client::ChatMessage;

/// A single message bubble with author, timestamp, body, and reactions.
#[component]
pub fn MessageView(message: ChatMessage) -> impl IntoView {
    let author_class = if message.is_local {
        "author local"
    } else {
        "author remote"
    };
    let body_class = if message.deleted {
        "body deleted"
    } else {
        "body"
    };
    let timestamp = willow_client::util::format_timestamp(message.timestamp_ms);

    let reply_preview = message.reply_preview.clone();
    let show_edited = message.edited && !message.deleted;
    let author = message.author.clone();
    let body = message.body.clone();
    let reactions: Vec<(String, usize)> = message
        .reactions
        .iter()
        .map(|(emoji, authors)| (emoji.clone(), authors.len()))
        .collect();
    let has_reactions = !reactions.is_empty();

    view! {
        <div class="message">
            {reply_preview.map(|preview| {
                view! {
                    <div class="reply-preview">{format!("> {preview}")}</div>
                }
            })}
            <div class="meta">
                <span class=author_class>{author}</span>
                <span class="timestamp">{timestamp}</span>
                {if show_edited {
                    Some(view! { <span class="edited">"(edited)"</span> })
                } else {
                    None
                }}
            </div>
            <div class=body_class>{body}</div>
            {if has_reactions {
                Some(view! {
                    <div class="reactions">
                        {reactions.into_iter().map(|(emoji, count)| {
                            view! {
                                <span class="reaction">
                                    {emoji} " " {count.to_string()}
                                </span>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                })
            } else {
                None
            }}
        </div>
    }
}
