use leptos::prelude::*;
use willow_client::ChatMessage;

use super::file_share::{parse_inline_file, FileCard};

/// Image file extensions for URL embedding.
const IMAGE_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp", ".ico",
];

/// Check if a URL points to an image based on extension.
fn is_image_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    // Strip query params for extension check.
    let path = lower.split('?').next().unwrap_or(&lower);
    IMAGE_EXTENSIONS.iter().any(|ext| path.ends_with(ext))
}

/// Extract URLs from text. Returns (segments, image_urls).
fn extract_urls(text: &str) -> (Vec<(String, bool)>, Vec<String>) {
    let mut segments = Vec::new();
    let mut images = Vec::new();
    let mut last_end = 0;

    // Collect all URL start positions, sorted by position.
    let mut url_starts: Vec<usize> = text
        .match_indices("https://")
        .chain(text.match_indices("http://"))
        .map(|(i, _)| i)
        .collect();
    url_starts.sort_unstable();
    url_starts.dedup();

    for &url_start in &url_starts {
        if url_start < last_end {
            continue; // skip overlapping matches
        }

        let rest = &text[url_start..];
        let url_end = rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == ')' || c == ']')
            .map(|i| url_start + i)
            .unwrap_or(text.len());
        let url = &text[url_start..url_end];

        if url_start > last_end {
            segments.push((text[last_end..url_start].to_string(), false));
        }
        segments.push((url.to_string(), true));

        if is_image_url(url) {
            images.push(url.to_string());
        }

        last_end = url_end;
    }

    if last_end < text.len() {
        segments.push((text[last_end..].to_string(), false));
    }

    if segments.is_empty() {
        segments.push((text.to_string(), false));
    }

    (segments, images)
}

/// Common emoji used in the reaction picker.
pub const REACTION_EMOJI: &[&str] = &[
    "\u{1F44D}",        // thumbs up
    "\u{2764}\u{FE0F}", // heart
    "\u{1F602}",        // joy
    "\u{1F389}",        // party
    "\u{1F62E}",        // surprised
    "\u{1F440}",        // eyes
    "\u{1F525}",        // fire
    "\u{2705}",         // check
];

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

/// A single message bubble with author, timestamp, body, reactions, and
/// optional action buttons (edit/delete/react) for own messages.
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
    /// Whether this message was sent by the local user.
    #[prop(default = false)]
    is_own: bool,
    /// Optional callback fired when the user clicks this message (used for replies).
    #[prop(optional, into)]
    on_click: Option<Callback<ChatMessage>>,
    /// Callback fired when the user wants to edit this message.
    #[prop(optional, into)]
    on_edit: Option<Callback<ChatMessage>>,
    /// Callback fired when the user wants to delete this message.
    #[prop(optional, into)]
    on_delete: Option<Callback<ChatMessage>>,
    /// Callback fired when the user picks an emoji reaction (message, emoji).
    #[prop(optional, into)]
    on_react: Option<Callback<(ChatMessage, String)>>,
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

    // Signal controlling the reaction picker popup visibility.
    let (show_picker, set_show_picker) = signal(false);

    // Determine whether to show any action buttons at all.
    let has_react = on_react.is_some();
    let has_edit = on_edit.is_some() && is_own && !message.deleted;
    let has_delete = on_delete.is_some() && is_own && !message.deleted;
    let show_actions = has_react || has_edit || has_delete;

    // Clones for closures.
    let msg_for_edit = message.clone();
    let msg_for_delete = message.clone();

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
            {if let Some((filename, data)) = parse_inline_file(&body) {
                view! { <FileCard filename=filename data=data /> }.into_any()
            } else {
                let (segments, images) = extract_urls(&body);
                let has_images = !images.is_empty();
                view! {
                    <div class=body_class>
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
                    </div>
                    {if has_images {
                        Some(view! {
                            <div class="message-embeds">
                                {images.into_iter().map(|url| {
                                    let url_clone = url.clone();
                                    view! {
                                        <a href=url.clone() target="_blank" rel="noopener noreferrer" class="embed-link">
                                            <img class="embed-image" src=url_clone alt="embedded image" loading="lazy" />
                                        </a>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        })
                    } else {
                        None
                    }}
                }.into_any()
            }}
            // Action bar -- shown on hover via CSS.
            {if show_actions {
                let edit_cb = on_edit;
                let edit_msg = msg_for_edit.clone();
                let delete_cb = on_delete;
                let delete_msg = msg_for_delete.clone();
                let react_cb = on_react;

                Some(view! {
                    <div class="message-actions">
                        {if has_react {
                            let react_cb_inner = react_cb;
                            Some(view! {
                                <div class="reaction-trigger">
                                    <button
                                        class="react-action"
                                        on:click=move |ev| {
                                            ev.stop_propagation();
                                            set_show_picker.update(|v| *v = !*v);
                                        }
                                    >
                                        "+"
                                    </button>
                                    {move || {
                                        if show_picker.get() {
                                            let cb = react_cb_inner;
                                            Some(view! {
                                                <div class="reaction-picker">
                                                    {REACTION_EMOJI.iter().map(|emoji| {
                                                        let emoji_str = emoji.to_string();
                                                        let emoji_val = emoji_str.clone();
                                                        let msg_clone = message.clone();
                                                        let cb_clone = cb;
                                                        view! {
                                                            <button on:click=move |ev| {
                                                                ev.stop_propagation();
                                                                if let Some(ref cb) = cb_clone {
                                                                    cb.run((msg_clone.clone(), emoji_val.clone()));
                                                                }
                                                                set_show_picker.set(false);
                                                            }>
                                                                {emoji_str}
                                                            </button>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            })
                                        } else {
                                            None
                                        }
                                    }}
                                </div>
                            })
                        } else {
                            None
                        }}
                        {if has_edit {
                            Some(view! {
                                <button
                                    class="edit-action"
                                    on:click=move |ev| {
                                        ev.stop_propagation();
                                        if let Some(ref cb) = edit_cb {
                                            cb.run(edit_msg.clone());
                                        }
                                    }
                                >
                                    "Edit"
                                </button>
                            })
                        } else {
                            None
                        }}
                        {if has_delete {
                            Some(view! {
                                <button
                                    class="delete-action"
                                    on:click=move |ev| {
                                        ev.stop_propagation();
                                        if let Some(ref cb) = delete_cb {
                                            cb.run(delete_msg.clone());
                                        }
                                    }
                                >
                                    "Delete"
                                </button>
                            })
                        } else {
                            None
                        }}
                    </div>
                })
            } else {
                None
            }}
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
