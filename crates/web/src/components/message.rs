use leptos::prelude::*;
use wasm_bindgen::JsCast;
use willow_client::DisplayMessage;

use super::file_share::{parse_inline_file, FileCard};
use crate::components::ConfirmDialog;
use crate::icons;

/// Image file extensions for URL and upload embedding.
/// SAFETY: SVG is included but must ONLY be rendered via `<img>` tags
/// (which sandbox scripts). Never use `<object>`, `<embed>`, or innerHTML
/// for SVG rendering as that would allow XSS.
const IMAGE_EXTENSIONS: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp", ".ico",
];

/// Check if a URL points to an image based on extension.
fn is_image_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    let path = lower.split('?').next().unwrap_or(&lower);
    IMAGE_EXTENSIONS.iter().any(|ext| path.ends_with(ext))
}

/// Check if a filename is an image.
fn is_image_file(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    IMAGE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// Get MIME type for an image filename.
fn mime_for_image(filename: &str) -> &'static str {
    let lower = filename.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".bmp") {
        "image/bmp"
    } else if lower.ends_with(".ico") {
        "image/x-icon"
    } else {
        "image/jpeg"
    }
}

/// Trigger a browser download for binary data.
fn download_blob(data: &[u8], filename: &str) {
    use wasm_bindgen::JsCast;
    let array = js_sys::Uint8Array::from(data);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(&parts) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(body) = document.body() else {
        return;
    };
    let Ok(a) = document.create_element("a") else {
        return;
    };
    a.set_attribute("href", &url).ok();
    a.set_attribute("download", filename).ok();
    a.set_attribute("style", "display:none").ok();
    body.append_child(&a).ok();
    if let Ok(html_a) = a.clone().dyn_into::<web_sys::HtmlElement>() {
        html_a.click();
    }
    body.remove_child(&a).ok();
    web_sys::Url::revoke_object_url(&url).ok();
}

/// Extract URLs from text. Returns (segments, image_urls).
pub(crate) fn extract_urls(text: &str) -> (Vec<(String, bool)>, Vec<String>) {
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
    message: DisplayMessage,
    /// Whether to display the author + timestamp header.
    /// Set to `false` for grouped (consecutive same-author) messages.
    #[prop(default = true)]
    show_header: bool,
    /// Whether this message was sent by the local user.
    #[prop(default = false)]
    is_own: bool,
    /// Optional callback fired when the user clicks Reply in the dropdown.
    #[prop(optional, into)]
    on_click: Option<Callback<DisplayMessage>>,
    /// Callback fired when the user wants to edit this message.
    #[prop(optional, into)]
    on_edit: Option<Callback<DisplayMessage>>,
    /// Callback fired when the user wants to delete this message.
    #[prop(optional, into)]
    on_delete: Option<Callback<DisplayMessage>>,
    /// Callback fired when the user picks an emoji reaction (message, emoji).
    #[prop(optional, into)]
    on_react: Option<Callback<(DisplayMessage, String)>>,
    /// Callback fired when the user pins/unpins this message.
    #[prop(optional, into)]
    on_pin: Option<Callback<DisplayMessage>>,
    /// Label for the pin button ("Pin" or "Unpin").
    #[prop(default = "Pin".to_string(), into)]
    pin_label: String,
    /// Whether this message is a reply to the local user (highlights it).
    #[prop(default = false)]
    is_mention: bool,
    /// Shared signal tracking which message ID has the action sheet open.
    /// Lives in `MessageList` so it survives message-list re-renders.
    #[prop(optional, into)]
    active_sheet_msg: Option<RwSignal<Option<String>>>,
) -> impl IntoView {
    let author_color = super::peer_color(&message.author_peer_id.to_string());
    let body_class = if message.deleted {
        "body deleted"
    } else {
        "body"
    };
    let timestamp = format_relative_time(message.timestamp_ms);

    let reply_preview = message.reply_preview.clone();
    let reply_to_id = message.reply_to.clone();
    let show_edited = message.edited && !message.deleted;
    let author = message.author_display_name.clone();
    let body = message.body.clone();
    let mut reactions: Vec<(String, usize)> = message
        .reactions
        .iter()
        .map(|(emoji, authors)| (emoji.clone(), authors.len()))
        .collect();
    reactions.sort_by(|a, b| a.0.cmp(&b.0));
    let has_reactions = !reactions.is_empty();

    let msg_class = match (show_header, is_mention) {
        (true, true) => "message mentioned",
        (true, false) => "message",
        (false, true) => "message grouped mentioned",
        (false, false) => "message grouped",
    };
    let msg_dom_id = format!("msg-{}", message.id);

    // Signal controlling the dropdown menu visibility.
    let (show_dropdown, set_show_dropdown) = signal(false);
    let (show_react_row, set_show_react_row) = signal(false);

    // Delete confirmation state.
    let (show_del_confirm, set_show_del_confirm) = signal(false);

    // Determine whether to show any action buttons at all.
    let has_reply = on_click.is_some();
    let has_react = on_react.is_some();
    let has_pin = on_pin.is_some();
    let has_edit = on_edit.is_some() && is_own && !message.deleted;
    let has_delete = on_delete.is_some() && is_own && !message.deleted;
    let show_actions = has_reply || has_react || has_pin || has_edit || has_delete;

    // Check if this is a file message (for the download action).
    let file_info = parse_inline_file(&body);
    let is_file_message = file_info.is_some();
    let file_data_for_download = file_info.clone();

    // Clones for closures.
    let msg_for_reply = message.clone();
    let msg_for_edit = message.clone();
    let msg_for_delete = message.clone();
    let msg_for_pin = message.clone();

    // Clone on_react for use in the reactions display.
    let on_react_for_reactions = on_react;

    // Long-press to show mobile action sheet.
    // The open/close state is shared with the parent `MessageList` via
    // `active_sheet_msg` so it survives message-list re-renders caused by
    // sync events.  When the parent signal is not provided we fall back to
    // a local signal (standalone usage).
    let msg_id_for_sheet = message.id.clone();
    let fallback = RwSignal::new(Option::<String>::None);
    let sheet_signal = active_sheet_msg.unwrap_or(fallback);
    let show_sheet = {
        let id = msg_id_for_sheet.clone();
        Memo::new(move |_| sheet_signal.get().as_deref() == Some(id.as_str()))
    };
    let set_show_sheet_open = {
        let id = msg_id_for_sheet.clone();
        move || sheet_signal.set(Some(id.clone()))
    };
    let set_show_sheet_close = move || sheet_signal.set(None);
    let (long_press_active, set_long_press_active) = signal(false);
    // Swipe-down-to-dismiss state for the action sheet.
    let (sheet_drag_y, set_sheet_drag_y) = signal(0.0f64);
    let (sheet_dragging, set_sheet_dragging) = signal(false);
    let sheet_touch_start_y =
        send_wrapper::SendWrapper::new(std::rc::Rc::new(std::cell::Cell::new(0.0f64)));
    let sheet_touch_last_y =
        send_wrapper::SendWrapper::new(std::rc::Rc::new(std::cell::Cell::new(0.0f64)));
    let sheet_touch_time =
        send_wrapper::SendWrapper::new(std::rc::Rc::new(std::cell::Cell::new(0.0f64)));
    let st_start_for_start = sheet_touch_start_y.clone();
    let st_last_for_start = sheet_touch_last_y.clone();
    let st_time_for_start = sheet_touch_time.clone();
    let st_start_for_move = sheet_touch_start_y.clone();
    let st_last_for_move = sheet_touch_last_y.clone();
    let st_start_for_end = sheet_touch_start_y.clone();
    let st_last_for_end = sheet_touch_last_y.clone();
    let st_time_for_end = sheet_touch_time.clone();

    let on_sheet_touchstart = move |ev: web_sys::TouchEvent| {
        if let Some(touch) = ev.touches().get(0) {
            let y = touch.client_y() as f64;
            st_start_for_start.set(y);
            st_last_for_start.set(y);
            st_time_for_start.set(js_sys::Date::now());
            set_sheet_dragging.set(true);
        }
    };

    let on_sheet_touchmove = move |ev: web_sys::TouchEvent| {
        if !sheet_dragging.get_untracked() {
            return;
        }
        if let Some(touch) = ev.touches().get(0) {
            let y = touch.client_y() as f64;
            let delta = y - st_start_for_move.get();
            // Only allow dragging downward (positive delta).
            set_sheet_drag_y.set(delta.max(0.0));
            st_last_for_move.set(y);
        }
    };

    let on_sheet_touchend = move |_: web_sys::TouchEvent| {
        if !sheet_dragging.get_untracked() {
            return;
        }
        set_sheet_dragging.set(false);
        let drag = sheet_drag_y.get_untracked();
        let elapsed = js_sys::Date::now() - st_time_for_end.get();
        let distance = st_last_for_end.get() - st_start_for_end.get();
        // Dismiss if dragged past 80px OR fast swipe (>200px/s downward).
        let velocity = if elapsed > 0.0 {
            distance / elapsed * 1000.0
        } else {
            0.0
        };
        if drag > 80.0 || velocity > 200.0 {
            set_show_sheet_close();
        }
        set_sheet_drag_y.set(0.0);
    };

    // Use Rc<Cell> so all closures share the SAME timer ID cell.
    let long_press_timer =
        send_wrapper::SendWrapper::new(std::rc::Rc::new(std::cell::Cell::new(0i32)));
    let lp_start = long_press_timer.clone();
    let lp_end = long_press_timer.clone();
    let lp_move = long_press_timer.clone();

    let on_msg_touchstart = move |ev: web_sys::TouchEvent| {
        // Skip if touching the action sheet or overlay.
        if let Some(target) = ev.target() {
            let el: web_sys::Element = target.unchecked_into();
            if el.closest(".mobile-action-sheet").ok().flatten().is_some()
                || el
                    .closest(".mobile-action-sheet-overlay")
                    .ok()
                    .flatten()
                    .is_some()
            {
                return;
            }
        }
        set_long_press_active.set(true);
        // Start 500ms timer via web_sys.
        if let Some(window) = web_sys::window() {
            let open_sheet = set_show_sheet_open.clone();
            let cb = wasm_bindgen::closure::Closure::once(move || {
                set_long_press_active.set(false);
                open_sheet();
                // Haptic feedback.
                if let Some(w) = web_sys::window() {
                    let nav = w.navigator();
                    let _ = nav.vibrate_with_duration(25);
                }
            });
            if let Ok(id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                500,
            ) {
                lp_start.set(id);
            }
            cb.forget();
        }
    };

    let on_msg_touchend = move |_: web_sys::TouchEvent| {
        let id = lp_end.get();
        if id != 0 {
            if let Some(w) = web_sys::window() {
                w.clear_timeout_with_handle(id);
            }
            lp_end.set(0);
        }
        set_long_press_active.set(false);
    };

    let on_msg_touchmove = move |_: web_sys::TouchEvent| {
        let id = lp_move.get();
        if id != 0 {
            if let Some(w) = web_sys::window() {
                w.clear_timeout_with_handle(id);
            }
            lp_move.set(0);
        }
        set_long_press_active.set(false);
    };

    let base_class = msg_class.to_string();
    view! {
        <div
            class=move || {
                if long_press_active.get() {
                    format!("{base_class} long-press-active")
                } else {
                    base_class.clone()
                }
            }
            id=msg_dom_id
            on:touchstart=on_msg_touchstart
            on:touchend=on_msg_touchend
            on:touchmove=on_msg_touchmove
        >
            {reply_preview.map(|preview| {
                let jump_id = reply_to_id.clone();
                view! {
                    <div
                        class={if jump_id.is_some() { "reply-preview reply-clickable" } else { "reply-preview" }}
                        on:click=move |ev| {
                            ev.stop_propagation();
                            if let Some(ref id) = jump_id {
                                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                                    if let Some(el) = doc.get_element_by_id(&format!("msg-{id}")) {
                                        let opts = web_sys::ScrollIntoViewOptions::new();
                                        opts.set_behavior(web_sys::ScrollBehavior::Smooth);
                                        opts.set_block(web_sys::ScrollLogicalPosition::Center);
                                        el.scroll_into_view_with_scroll_into_view_options(&opts);
                                    }
                                }
                            }
                        }
                    >
                        {format!("> {preview}")}
                    </div>
                }
            })}
            {if show_header {
                let author_pid = message.author_peer_id.to_string();
                let author_pid_for_presence = author_pid.clone();
                let presence_state = Signal::derive(move || {
                    use leptos::context::use_context;
                    use_context::<crate::state::AppState>()
                        .and_then(|a| a.presence.per_peer.get().get(&author_pid_for_presence).copied())
                        .unwrap_or(willow_client::presence::PresenceState::Here)
                });
                Some(view! {
                    <div class="meta">
                        <span class="author" style=format!("color: {author_color}")>{author}</span>
                        <super::TrustBadge
                            peer_id=author_pid.clone()
                            size=super::TrustBadgeSize::Disk12
                        />
                        <span class="author-presence" title=move || presence_state.get().label()>
                            <super::StatusDot
                                state=presence_state
                                size=super::StatusDotSize::Author
                                border=super::StatusDotBorder::Bg0
                                ambient=false
                            />
                        </span>
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
            {if let Some((filename, data)) = file_info.clone() {
                if is_image_file(&filename) {
                    // Render uploaded images inline as embeds.
                    let mime = mime_for_image(&filename);
                    let b64 = willow_client::base64::encode(&data);
                    let src = format!("data:{mime};base64,{b64}");
                    let alt = filename.clone();
                    view! {
                        <div class="message-embeds">
                            <img class="embed-image" src=src alt=alt loading="lazy" />
                        </div>
                    }.into_any()
                } else {
                    view! { <FileCard filename=filename data=data /> }.into_any()
                }
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
            // Action bar -- single dropdown triggered by "..." button.
            {if show_actions {
                let edit_cb = on_edit;
                let edit_msg = msg_for_edit.clone();
                let react_cb = on_react;
                let pin_cb = on_pin;
                let pin_msg = msg_for_pin.clone();
                let pin_label_text = pin_label.clone();
                let reply_cb = on_click;
                let reply_msg = msg_for_reply.clone();

                let msg_for_react = message.clone();

                Some(view! {
                    <div class="message-actions">
                        <button class="action-trigger" on:click=move |ev| {
                            ev.stop_propagation();
                            set_show_dropdown.update(|v| *v = !*v);
                            set_show_react_row.set(false);
                        }>{icons::icon_more_horizontal()}</button>
                        {move || {
                            if show_dropdown.get() {
                                let reply_view = if has_reply {
                                    let cb = reply_cb;
                                    let msg = reply_msg.clone();
                                    Some(view! {
                                        <button class="dropdown-item" on:click=move |ev| {
                                            ev.stop_propagation();
                                            if let Some(ref cb) = cb {
                                                cb.run(msg.clone());
                                            }
                                            set_show_dropdown.set(false);
                                        }>"Reply"</button>
                                    })
                                } else {
                                    None
                                };

                                let pin_view = if has_pin {
                                    let cb = pin_cb;
                                    let msg = pin_msg.clone();
                                    let label = pin_label_text.clone();
                                    Some(view! {
                                        <button class="dropdown-item" on:click=move |ev| {
                                            ev.stop_propagation();
                                            if let Some(ref cb) = cb {
                                                cb.run(msg.clone());
                                            }
                                            set_show_dropdown.set(false);
                                        }>{label}</button>
                                    })
                                } else {
                                    None
                                };

                                let react_view = if has_react {
                                    let react_cb_for_row = react_cb;
                                    let msg_for_emoji = msg_for_react.clone();
                                    Some(view! {
                                        <button class="dropdown-item" on:click=move |ev| {
                                            ev.stop_propagation();
                                            set_show_react_row.update(|v| *v = !*v);
                                        }>"React"</button>
                                        {move || {
                                            if show_react_row.get() {
                                                let cb = react_cb_for_row;
                                                let msg_inner = msg_for_emoji.clone();
                                                Some(view! {
                                                    <div class="dropdown-emoji-row">
                                                        {REACTION_EMOJI.iter().map(|emoji| {
                                                            let emoji_str = emoji.to_string();
                                                            let emoji_val = emoji_str.clone();
                                                            let msg_clone = msg_inner.clone();
                                                            let cb_clone = cb;
                                                            view! {
                                                                <button on:click=move |ev| {
                                                                    ev.stop_propagation();
                                                                    if let Some(ref cb) = cb_clone {
                                                                        cb.run((msg_clone.clone(), emoji_val.clone()));
                                                                    }
                                                                    set_show_dropdown.set(false);
                                                                    set_show_react_row.set(false);
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
                                    }.into_any())
                                } else {
                                    None
                                };

                                let edit_view = if has_edit {
                                    let cb = edit_cb;
                                    let msg = edit_msg.clone();
                                    Some(view! {
                                        <button class="dropdown-item" on:click=move |ev| {
                                            ev.stop_propagation();
                                            if let Some(ref cb) = cb {
                                                cb.run(msg.clone());
                                            }
                                            set_show_dropdown.set(false);
                                        }>"Edit"</button>
                                    })
                                } else {
                                    None
                                };

                                let delete_view = if has_delete {
                                    Some(view! {
                                        <button class="dropdown-item dropdown-danger" on:click=move |ev| {
                                            ev.stop_propagation();
                                            set_show_dropdown.set(false);
                                            set_show_del_confirm.set(true);
                                        }>"Delete"</button>
                                    })
                                } else {
                                    None
                                };

                                let download_view = if is_file_message {
                                    file_data_for_download.clone().map(|(filename, data)| {
                                        view! {
                                            <button class="dropdown-item" on:click=move |ev| {
                                                ev.stop_propagation();
                                                download_blob(&data, &filename);
                                                set_show_dropdown.set(false);
                                            }>"Download"</button>
                                        }
                                    })
                                } else {
                                    None
                                };

                                Some(view! {
                                    <div class="message-dropdown">
                                        {reply_view}
                                        {pin_view}
                                        {react_view}
                                        {edit_view}
                                        {delete_view}
                                        {download_view}
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
            // Mobile bottom action sheet (shown via long-press, hidden by default).
            {if show_actions {
                let reply_cb2 = on_click;
                let reply_msg2 = message.clone();
                let pin_cb2 = on_pin;
                let pin_msg2 = message.clone();
                let pin_label2 = pin_label.clone();
                let edit_cb2 = on_edit;
                let edit_msg2 = message.clone();
                let react_cb2 = on_react;
                let react_msg2 = message.clone();

                let close_sheet = set_show_sheet_close;

                Some(view! {
                    <div
                        class=move || if show_sheet.get() { "mobile-action-sheet-overlay open" } else { "mobile-action-sheet-overlay" }
                        on:click=move |_| close_sheet()
                    ></div>
                    <div
                        class=move || if show_sheet.get() { "mobile-action-sheet open" } else { "mobile-action-sheet" }
                        style=move || {
                            let dy = sheet_drag_y.get();
                            if dy > 0.0 {
                                // While dragging, disable transition and apply transform.
                                format!("transform: translateY({dy}px); transition: none;")
                            } else {
                                String::new()
                            }
                        }
                        on:touchstart=on_sheet_touchstart
                        on:touchmove=on_sheet_touchmove
                        on:touchend=on_sheet_touchend
                    >
                        {if has_reply {
                            let cb = reply_cb2;
                            let msg = reply_msg2.clone();
                            let close = close_sheet;
                            Some(view! {
                                <button class="sheet-item" on:click=move |ev| {
                                    ev.stop_propagation();
                                    if let Some(ref cb) = cb { cb.run(msg.clone()); }
                                    close();
                                }>"Reply"</button>
                            })
                        } else { None }}
                        {if has_pin {
                            let cb = pin_cb2;
                            let msg = pin_msg2.clone();
                            let label = pin_label2.clone();
                            let close = close_sheet;
                            Some(view! {
                                <button class="sheet-item" on:click=move |ev| {
                                    ev.stop_propagation();
                                    if let Some(ref cb) = cb { cb.run(msg.clone()); }
                                    close();
                                }>{label}</button>
                            })
                        } else { None }}
                        {if has_react {
                            let cb = react_cb2;
                            let msg = react_msg2.clone();
                            let close = close_sheet;
                            Some(view! {
                                <div class="sheet-emoji-row">
                                    {REACTION_EMOJI.iter().map(|emoji| {
                                        let e = emoji.to_string();
                                        let ev = e.clone();
                                        let m = msg.clone();
                                        let c = cb;
                                        let cl = close;
                                        view! {
                                            <button on:click=move |_| {
                                                if let Some(ref c) = c { c.run((m.clone(), ev.clone())); }
                                                cl();
                                            }>{e}</button>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            })
                        } else { None }}
                        {if has_edit {
                            let cb = edit_cb2;
                            let msg = edit_msg2.clone();
                            let close = close_sheet;
                            Some(view! {
                                <button class="sheet-item" on:click=move |ev| {
                                    ev.stop_propagation();
                                    if let Some(ref cb) = cb { cb.run(msg.clone()); }
                                    close();
                                }>"Edit"</button>
                            })
                        } else { None }}
                        {if has_delete {
                            let close = close_sheet;
                            Some(view! {
                                <button class="sheet-item sheet-danger" on:click=move |ev| {
                                    ev.stop_propagation();
                                    close();
                                    set_show_del_confirm.set(true);
                                }>"Delete"</button>
                            })
                        } else { None }}
                        <button class="sheet-item sheet-cancel" on:click=move |_| close_sheet()>"Cancel"</button>
                    </div>
                })
            } else { None }}
            {if has_reactions {
                let react_cb = on_react_for_reactions;
                Some(view! {
                    <div class="reactions">
                        {reactions.into_iter().map(|(emoji, count)| {
                            let emoji_for_click = emoji.clone();
                            let msg_clone = message.clone();
                            let cb_clone = react_cb;
                            view! {
                                <button class="reaction" on:click=move |ev| {
                                    ev.stop_propagation();
                                    if let Some(ref cb) = cb_clone {
                                        cb.run((msg_clone.clone(), emoji_for_click.clone()));
                                    }
                                }>
                                    {emoji} " " {count.to_string()}
                                </button>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                })
            } else {
                None
            }}
            {if has_delete {
                let del_cb = on_delete;
                let del_msg = msg_for_delete.clone();
                Some(view! {
                    <ConfirmDialog
                        visible=show_del_confirm
                        title="Delete Message"
                        message=Signal::derive(|| "Are you sure you want to delete this message?".to_string())
                        confirm_text="Delete"
                        danger=true
                        on_confirm=Callback::new(move |_| {
                            if let Some(ref cb) = del_cb {
                                cb.run(del_msg.clone());
                            }
                            set_show_del_confirm.set(false);
                        })
                        on_cancel=Callback::new(move |_| {
                            set_show_del_confirm.set(false);
                        })
                    />
                })
            } else {
                None
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_image_url_recognizes_common_formats() {
        assert!(is_image_url("https://example.com/photo.png"));
        assert!(is_image_url("https://example.com/photo.JPG"));
        assert!(is_image_url("https://example.com/anim.gif"));
        assert!(is_image_url("https://example.com/photo.webp"));
        assert!(!is_image_url("https://example.com/doc.pdf"));
        assert!(!is_image_url("https://example.com/page"));
    }

    #[test]
    fn is_image_url_handles_query_params() {
        assert!(is_image_url("https://example.com/photo.png?w=200"));
        assert!(!is_image_url("https://example.com/api?file=photo.png"));
        // The second one should be false because the path doesn't end in .png
    }

    #[test]
    fn is_image_file_works() {
        assert!(is_image_file("photo.png"));
        assert!(is_image_file("animation.GIF"));
        assert!(is_image_file("icon.svg"));
        assert!(!is_image_file("document.pdf"));
        assert!(!is_image_file("archive.zip"));
    }

    #[test]
    fn extract_urls_no_urls() {
        let (segments, images) = extract_urls("hello world");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].0, "hello world");
        assert!(!segments[0].1);
        assert!(images.is_empty());
    }

    #[test]
    fn extract_urls_single_url() {
        let (segments, images) = extract_urls("check https://example.com please");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].0, "check ");
        assert!(!segments[0].1);
        assert_eq!(segments[1].0, "https://example.com");
        assert!(segments[1].1);
        assert_eq!(segments[2].0, " please");
        assert!(!segments[2].1);
        assert!(images.is_empty());
    }

    #[test]
    fn extract_urls_image_url() {
        let (segments, images) = extract_urls("look https://example.com/cat.gif");
        assert_eq!(segments.len(), 2);
        assert!(segments[1].1); // is URL
        assert_eq!(images.len(), 1);
        assert_eq!(images[0], "https://example.com/cat.gif");
    }

    #[test]
    fn extract_urls_multiple_urls() {
        let (segments, images) = extract_urls("a https://one.com b https://two.com/pic.png c");
        // Should have 5 segments: text, url, text, url, text
        assert_eq!(segments.len(), 5);
        assert!(segments[1].1);
        assert!(segments[3].1);
        assert_eq!(images.len(), 1); // only the .png one
    }

    #[test]
    fn extract_urls_url_only() {
        let (segments, images) = extract_urls("https://example.com");
        assert_eq!(segments.len(), 1);
        assert!(segments[0].1);
        assert!(images.is_empty());
    }

    #[test]
    fn extract_urls_https_not_doubled() {
        // https:// should not be matched twice (once as http:// prefix)
        let (segments, _) = extract_urls("https://example.com");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].0, "https://example.com");
    }

    #[test]
    fn mime_for_image_returns_correct_types() {
        assert_eq!(mime_for_image("photo.png"), "image/png");
        assert_eq!(mime_for_image("photo.jpg"), "image/jpeg");
        assert_eq!(mime_for_image("photo.jpeg"), "image/jpeg");
        assert_eq!(mime_for_image("anim.gif"), "image/gif");
        assert_eq!(mime_for_image("photo.webp"), "image/webp");
        assert_eq!(mime_for_image("icon.svg"), "image/svg+xml");
        assert_eq!(mime_for_image("icon.bmp"), "image/bmp");
        assert_eq!(mime_for_image("icon.ico"), "image/x-icon");
        // Unknown extension defaults to jpeg
        assert_eq!(mime_for_image("photo.xyz"), "image/jpeg");
    }
}
