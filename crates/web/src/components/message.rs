use leptos::prelude::*;
use willow_client::ChatMessage;

/// Format a timestamp as a relative time string for recent messages.
///
/// - Less than 60 seconds ago: "just now"
/// - Less than 1 hour ago: "Xm ago"
/// - Less than 24 hours ago: "Xh ago"
/// - Older: "HH:MM"
pub fn format_relative_time(timestamp_ms: u64) -> String {
    if timestamp_ms == 0 {
        return String::new();
    }
    let now_ms = js_sys::Date::now() as u64;
    if timestamp_ms > now_ms {
        // Future timestamp -- fall back to HH:MM.
        return willow_client::util::format_timestamp(timestamp_ms);
    }
    let diff_secs = (now_ms - timestamp_ms) / 1000;
    if diff_secs < 60 {
        "just now".to_string()
    } else if diff_secs < 3600 {
        format!("{}m ago", diff_secs / 60)
    } else if diff_secs < 86400 {
        format!("{}h ago", diff_secs / 3600)
    } else {
        willow_client::util::format_timestamp(timestamp_ms)
    }
}

/// A single message bubble with author, timestamp, body, and reactions.
///
/// When `show_header` is `false` the author/timestamp meta row is hidden,
/// which is used for consecutive messages from the same author (grouping).
#[component]
pub fn MessageView(
    message: ChatMessage,
    /// Whether to display the author + timestamp header.
    /// Set to `false` for grouped (consecutive same-author) messages.
    #[prop(default = true)]
    show_header: bool,
    /// Optional callback fired when the user clicks this message (used for replies).
    #[prop(optional, into)]
    on_click: Option<Callback<ChatMessage>>,
) -> impl IntoView {
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
    let timestamp = format_relative_time(message.timestamp_ms);

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

    let msg_class = if show_header {
        "message"
    } else {
        "message grouped"
    };

    let msg_for_click = message.clone();

    view! {
        <div
            class=msg_class
            on:click=move |_| {
                if let Some(ref cb) = on_click {
                    cb.run(msg_for_click.clone());
                }
            }
        >
            {reply_preview.map(|preview| {
                view! {
                    <div class="reply-preview">{format!("> {preview}")}</div>
                }
            })}
            {if show_header {
                Some(view! {
                    <div class="meta">
                        <span class=author_class>{author}</span>
                        <span class="timestamp">{timestamp}</span>
                        {if show_edited {
                            Some(view! { <span class="edited">"(edited)"</span> })
                        } else {
                            None
                        }}
                    </div>
                })
            } else {
                None
            }}
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
