use leptos::prelude::*;
use willow_client::DisplayMessage;

use super::message::extract_urls;
use crate::icons;

/// Render a message body with clickable URL links.
fn render_body_with_links(body: &str) -> impl IntoView {
    let (segments, _images) = extract_urls(body);
    view! {
        <span>
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
        </span>
    }
}

/// Panel showing pinned messages for the current channel.
/// Each pinned message shows its content (with clickable URLs),
/// author, and a "Jump" button.
#[component]
pub fn PinnedPanel(
    messages: ReadSignal<Vec<DisplayMessage>>,
    on_jump: impl Fn(String) + Send + Clone + 'static,
    on_close: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView {
    view! {
        <aside class="pinned-panel" role="complementary" aria-label="pinned">
            <div class="pinned-header">
                <h3>"Pinned Messages"</h3>
                <button class="btn btn-sm" on:click=move |_| on_close(())>{icons::icon_x()}</button>
            </div>
            <div class="pinned-list">
                <For
                    each=move || messages.get()
                    key=|msg| msg.id.clone()
                    let:msg
                >
                    {
                        let msg_id = msg.id.clone();
                        let author = msg.author_display_name.clone();
                        let body = msg.body.clone();
                        let on_jump = on_jump.clone();
                        view! {
                            <div class="pinned-item">
                                <div class="pinned-meta">
                                    <span class="pinned-author">{author}</span>
                                </div>
                                <div class="pinned-body">
                                    {render_body_with_links(&body)}
                                </div>
                                <button
                                    class="btn btn-sm pinned-jump"
                                    on:click=move |_| on_jump(msg_id.clone())
                                >
                                    "Jump"
                                </button>
                            </div>
                        }
                    }
                </For>
                {move || {
                    if messages.get().is_empty() {
                        Some(view! { <div class="empty-state">"No pinned messages"</div> })
                    } else {
                        None
                    }
                }}
            </div>
        </aside>
    }
}
