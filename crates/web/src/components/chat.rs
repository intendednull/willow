use leptos::prelude::*;
use willow_client::DisplayMessage;

use super::message_row::{day_bucket, DaySeparator};
use super::MessageView;

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
    // Tracks which message ID has the mobile action sheet open.
    // Lives here (outside the reactive closure) so it survives
    // message-list re-renders caused by sync events.
    let active_sheet_msg = RwSignal::new(Option::<String>::None);

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

    // Surface downgrade banners for any downgraded peer visible in this
    // chat view. The peer-letter / DM view is the eventual home for
    // this banner per spec; until that surface exists we render the
    // first downgraded peer at the top of the message list so the
    // user never loses sight of a key-rotation warning.
    let downgraded_target = {
        use willow_client::trust::PeerTrust;
        let app_state = use_context::<crate::state::AppState>();
        leptos::prelude::Memo::new(move |_| {
            app_state.and_then(|s| {
                s.trust.trust_map.get().iter().find_map(|(pid, trust)| {
                    if matches!(trust, PeerTrust::DowngradedFromVerified { .. }) {
                        Some(pid.clone())
                    } else {
                        None
                    }
                })
            })
        })
    };

    view! {
        <div class="message-list-container">
            {move || {
                downgraded_target.get().map(|pid| {
                    view! {
                        <crate::components::DowngradeBanner
                            peer_id=Signal::derive(move || pid.clone())
                        />
                    }
                })
            }}
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
                        // Track the previous message's local-date bucket so we
                        // can inject a `<DaySeparator>` at every local-date
                        // boundary (and before the first message). `PartialEq`
                        // on `DayBucket` drives the comparison — a bucket
                        // difference means "emit a separator before this row".
                        let mut prev_bucket: Option<super::message_row::DayBucket> = None;
                        let views: Vec<_> = msgs.iter().enumerate().flat_map(|(i, msg)| {
                            let show_header = if i == 0 {
                                true
                            } else {
                                let prev = &msgs[i - 1];
                                // Per message-row.md §Author-run grouping: break a run
                                // on author change, >5min gap, or when either the
                                // previous *or* current message carries a run-break
                                // cue (whisper / pinned / queueNote). `pinned` and
                                // `queue_note` are wired today; whisper joins in
                                // Task 8 once whisper-mode.md lands.
                                prev.author_display_name != msg.author_display_name
                                    || msg.timestamp_ms.saturating_sub(prev.timestamp_ms)
                                        > 300_000
                                    || prev.pinned
                                    || msg.pinned
                                    || prev.queue_note != willow_client::QueueNote::None
                                    || msg.queue_note != willow_client::QueueNote::None
                            };
                            let curr_bucket = day_bucket(msg.timestamp_ms);
                            let emit_sep = match &prev_bucket {
                                None => true,
                                Some(p) => p != &curr_bucket,
                            };
                            prev_bucket = Some(curr_bucket.clone());
                            let sep_view = if emit_sep {
                                Some(view! {
                                    <DaySeparator bucket=curr_bucket.clone() />
                                }.into_any())
                            } else {
                                None
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
                                    active_sheet_msg=active_sheet_msg
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
                                        active_sheet_msg=active_sheet_msg
                                    />
                                };
                            }
                            let mut out = Vec::with_capacity(2);
                            if let Some(sep) = sep_view {
                                out.push(sep);
                            }
                            out.push(builder.into_any());
                            out
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
