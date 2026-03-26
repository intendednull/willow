use leptos::prelude::*;
use willow_client::DisplayMessage;

use super::MessageView;
use crate::icons;

/// Header bar showing the current channel name and connected peer count.
#[component]
pub fn ChannelHeader(
    channel: ReadSignal<String>,
    peer_count: ReadSignal<usize>,
    on_menu_click: impl Fn(()) + Send + Clone + 'static,
    on_members_click: impl Fn(()) + Send + Clone + 'static,
    #[prop(optional, into)] on_pinned_click: Option<Callback<()>>,
) -> impl IntoView {
    view! {
        <div class="channel-header">
            <button class="mobile-nav-toggle" on:click=move |_| on_menu_click(())>
                {icons::icon_menu()}
            </button>
            <span>{icons::icon_hash()} " " {move || channel.get()}</span>
            <span class="channel-header-right">
                {on_pinned_click.map(|cb| view! {
                    <button class="pinned-toggle" title="Pinned Messages" on:click=move |_| cb.run(())>
                        {icons::icon_pin()}
                    </button>
                })}
                <button class="mobile-members-toggle" on:click=move |_| on_members_click(())>
                    {icons::icon_users()} " " {move || peer_count.get().to_string()}
                </button>
            </span>
        </div>
    }
}

/// Scrollable message list for the current channel.
/// Auto-scrolls to bottom when new messages arrive if the user
/// is already at (or near) the bottom. Shows a floating
/// "scroll to bottom" pill when the user has scrolled up.
#[component]
pub fn MessageList(
    messages: ReadSignal<Vec<DisplayMessage>>,
    /// Whether the app is still in its initial loading state.
    #[prop(optional, into)]
    loading: Signal<bool>,
    /// The local user's display name (used to determine "own" messages).
    #[prop(optional, into)]
    local_display_name: Option<Signal<String>>,
    /// Callback fired when the user clicks a message (to start a reply).
    #[prop(optional, into)]
    on_message_click: Option<Callback<DisplayMessage>>,
    /// Callback fired when the user wants to edit a message.
    #[prop(optional, into)]
    on_edit: Option<Callback<DisplayMessage>>,
    /// Callback fired when the user wants to delete a message.
    #[prop(optional, into)]
    on_delete: Option<Callback<DisplayMessage>>,
    /// Callback fired when the user picks an emoji reaction.
    #[prop(optional, into)]
    on_react: Option<Callback<(DisplayMessage, String)>>,
    /// Callback fired when the user pins/unpins a message.
    #[prop(optional, into)]
    on_pin: Option<Callback<DisplayMessage>>,
    /// Signal mapping message IDs to pin labels ("Pin" or "Unpin").
    #[prop(optional, into)]
    pin_labels: Option<Signal<std::collections::HashMap<String, String>>>,
) -> impl IntoView {
    let list_ref = NodeRef::<leptos::html::Div>::new();
    let (show_scroll_btn, set_show_scroll_btn) = signal(false);

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
            // OR the user was within 200px of the bottom.
            let was_at_bottom = (scroll_height - scroll_top - client_height) < 200.0;
            let is_new = prev_len.map(|p| len > p).unwrap_or(true);

            if was_at_bottom || is_new {
                // Defer scroll to next microtask so DOM has updated (fixes mobile).
                let el_clone = el.clone();
                set_timeout(
                    move || el_clone.set_scroll_top(el_clone.scroll_height()),
                    std::time::Duration::ZERO,
                );
            }

            // Update scroll-to-bottom button visibility.
            let distance = scroll_height - scroll_top - client_height;
            set_show_scroll_btn.set(distance > 200.0);
        }

        len
    });

    let scroll_to_bottom = move |_| {
        if let Some(el) = list_ref.get() {
            let el: &web_sys::HtmlElement = &el;
            el.set_scroll_top(el.scroll_height());
            set_show_scroll_btn.set(false);
        }
    };

    let on_scroll = move |_| {
        if let Some(el) = list_ref.get() {
            let el: &web_sys::HtmlElement = &el;
            let scroll_top = el.scroll_top() as f64;
            let scroll_height = el.scroll_height() as f64;
            let client_height = el.client_height() as f64;
            let distance = scroll_height - scroll_top - client_height;
            set_show_scroll_btn.set(distance > 200.0);
        }
    };

    let on_msg_click = on_message_click;

    view! {
        <div class="message-list-container">
            <div class="message-list" node_ref=list_ref on:scroll=on_scroll>
                {move || {
                    let msgs = messages.get();
                    let is_loading = loading.get();
                    if is_loading && msgs.is_empty() {
                        view! {
                            <div class="loading-spinner" role="status">
                                <div class="spinner"></div>
                                <span>"Connecting..."</span>
                            </div>
                        }.into_any()
                    } else if msgs.is_empty() {
                        view! {
                            <div class="empty-state">
                                "No messages yet. Say hello!"
                            </div>
                        }.into_any()
                    } else {
                        // Build grouped message views: consecutive messages from
                        // the same author collapse the header.
                        let on_click = on_msg_click;
                        let on_ed = on_edit;
                        let on_del = on_delete;
                        let on_re = on_react;
                        let on_pn = on_pin;
                        let pn_labels = pin_labels;
                        let local_name = local_display_name
                            .map(|sig| sig.get())
                            .unwrap_or_default();
                        let label_map = pn_labels.map(|s| s.get()).unwrap_or_default();
                        let views: Vec<_> = msgs.iter().enumerate().map(|(i, msg)| {
                            let show_header = if i == 0 {
                                true
                            } else {
                                let prev = &msgs[i - 1];
                                prev.author_display_name != msg.author_display_name
                                    || msg.timestamp_ms.saturating_sub(prev.timestamp_ms)
                                        > 300_000
                            };
                            let m = msg.clone();
                            let is_own = msg.is_local;
                            // Check if this is a reply targeting the local user.
                            let is_mention = !is_own
                                && msg
                                    .reply_preview
                                    .as_ref()
                                    .map(|p| p.starts_with(&format!("{local_name}:")))
                                    .unwrap_or(false);
                            let pin_label = label_map
                                .get(&msg.id)
                                .cloned()
                                .unwrap_or_else(|| "Pin".to_string());
                            let mut builder = view! {
                                <MessageView
                                    message=m
                                    show_header=show_header
                                    is_own=is_own
                                    is_mention=is_mention
                                />
                            };
                            // We need to build with all props. Re-create to pass them.
                            let m2 = msg.clone();
                            if on_click.is_some() || on_ed.is_some() || on_del.is_some() || on_re.is_some() || on_pn.is_some() {
                                let click_cb = on_click;
                                let ed_cb = on_ed;
                                let del_cb = on_del;
                                let re_cb = on_re;
                                let pn_cb = on_pn;
                                // Unfortunately we need to re-create the view
                                // to pass all optional props. Using nested if/else
                                // creates mismatched types, so use into_any().
                                builder = view! {
                                    <MessageView
                                        message=m2
                                        show_header=show_header
                                        is_own=is_own
                                        is_mention=is_mention
                                        on_click=click_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        on_edit=ed_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        on_delete=del_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        on_react=re_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        on_pin=pn_cb.unwrap_or_else(|| Callback::new(|_| {}))
                                        pin_label=pin_label
                                    />
                                };
                            }
                            builder.into_any()
                        }).collect();
                        view! { <div>{views}</div> }.into_any()
                    }
                }}
            </div>
            {move || {
                if show_scroll_btn.get() {
                    Some(view! {
                        <button class="scroll-to-bottom" on:click=scroll_to_bottom>
                            "New messages"
                        </button>
                    })
                } else {
                    None
                }
            }}
        </div>
    }
}
