use leptos::prelude::*;
use wasm_bindgen::JsCast;
use willow_client::{DisplayMessage, QueueNote};

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

/// Read the first `Touch` from a `TouchEvent`, tolerating synthetic
/// `Event`s (without a `touches` list) dispatched by the browser-test
/// harness. Returns `None` when no touches are present or when the
/// event is not a real `TouchEvent`.
///
/// Without this guard, `ev.touches().get(0)` panics with a JS
/// `TypeError` when the harness dispatches a plain `Event` — see
/// `crates/web/tests/browser.rs` `open_sheet_via_long_press`.
fn first_touch(ev: &web_sys::TouchEvent) -> Option<web_sys::Touch> {
    use wasm_bindgen::JsCast;
    let target: &wasm_bindgen::JsValue = ev.as_ref();
    let touches = js_sys::Reflect::get(target, &wasm_bindgen::JsValue::from_str("touches")).ok()?;
    if touches.is_undefined() || touches.is_null() {
        return None;
    }
    let list: web_sys::TouchList = touches.dyn_into().ok()?;
    list.get(0)
}

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
    /// Callback fired when the user swipes right on the row to open the
    /// thread. If omitted, the gesture still captures but the release is
    /// a no-op (thread pane is owned by `thread-pane.md`, not yet wired).
    #[prop(optional, into)]
    on_open_thread: Option<Callback<DisplayMessage>>,
) -> impl IntoView {
    let author_color = super::peer_color(&message.author_peer_id);
    // Phase 2a Task 14 — spec §Copy / Deleted placeholder + empty-body
    // fallback: deleted rows render the fixed `this message was
    // withdrawn` string inside `.body.body--deleted` (italic `--ink-3`);
    // rows whose body is whitespace-only (migration edge case) but not
    // deleted render `empty message` inside `.body.body--empty`. All
    // other rows use the plain `.body` class so the normal segment
    // pipeline runs.
    let body_is_empty = message.body.trim().is_empty();
    let body_class = if message.deleted {
        "body body--deleted"
    } else if body_is_empty {
        "body body--empty"
    } else {
        "body"
    };
    let timestamp = format_relative_time(message.timestamp_ms);
    // Phase 2a Task 6: `message.pinned` gates the row marker + badge
    // and also feeds the run-break predicate in `MessageList` (see
    // `chat.rs`). Phase 2a Task 7 extends the same treatment to
    // `queue_note`: non-None variants drive the inline hint + badge
    // + `.message--pending` opacity class (see below).
    let is_pinned = message.pinned;
    let queue_note = message.queue_note;
    let is_pending = queue_note == QueueNote::Pending;
    let has_queue_note = queue_note != QueueNote::None;
    // Phase 2a Task 8: reserve the whisper surface. `message.whisper`
    // is gated always-false in the projection today (see
    // `client/src/views.rs` TODO(#562)); once that phase lands the
    // projection will flip it and the class + badge below light up
    // automatically.
    let is_whisper = message.whisper;

    let reply_preview = message.reply_preview.clone();
    let reply_to_id = message.reply_to.clone();
    let show_edited = message.edited && !message.deleted;
    let author = message.author_display_name.clone();
    let body = message.body.clone();
    // `<ReactionStrip>` (phase 3c.3) sorts + counts internally from
    // the projection-resolved `HashMap<emoji, reactor display names>`.
    // We only need the truthy `has_reactions` gate here so the strip
    // doesn't render an empty `.reactions-strip` div on every row.
    let has_reactions = !message.reactions.is_empty();

    // Phase 2a Task 4: derive self-mention highlight from the
    // projection-populated `mentions` field. The existing `is_mention`
    // prop encodes "this is a reply targeting me" (reply-preview match)
    // and is kept for backwards compatibility; the new class
    // `message--mention` is the spec-named row state for *body-level*
    // self-mentions per message-row.md §Self-mention row highlight.
    use leptos::context::use_context;
    let local_peer_from_ctx: Option<willow_identity::EndpointId> =
        use_context::<crate::state::AppState>()
            .map(|a| a.network.peer_id.get_untracked())
            .and_then(|s| s.parse().ok());
    let is_self_mention = local_peer_from_ctx
        .as_ref()
        .map(|lp| willow_client::mentions::mentions_me(&message, lp))
        .unwrap_or(false);

    let base_msg_class = match (show_header, is_mention, is_self_mention) {
        (true, _, true) => "message message--mention",
        (true, true, false) => "message mentioned",
        (true, false, false) => "message",
        (false, _, true) => "message grouped message--mention",
        (false, true, false) => "message grouped mentioned",
        (false, false, false) => "message grouped",
    };
    // Append `message--pinned` when the projection flagged this row
    // pinned. Pinned rows always break a run (see `chat.rs`), so a
    // pinned row always lands in a first-of-run branch above.
    //
    // Phase 2a Task 7: additionally append `message--pending` when
    // `queue_note == Pending`. CSS drops that variant to
    // `opacity: 0.7` with an 180 ms fade-out (see §Queue notes). The
    // run-break predicate in `chat.rs` ensures rows with a queue_note
    // always render as a first-of-run branch above, matching the
    // spec's badge-in-author-row contract.
    let mut suffix = String::new();
    if is_pinned {
        suffix.push_str(" message--pinned");
    }
    if is_pending {
        suffix.push_str(" message--pending");
    }
    if is_whisper {
        suffix.push_str(" message--whisper");
    }
    let msg_class = if suffix.is_empty() {
        std::borrow::Cow::Borrowed(base_msg_class)
    } else {
        std::borrow::Cow::Owned(format!("{base_msg_class}{suffix}"))
    };
    let msg_dom_id = format!("msg-{}", message.id);

    // Signal controlling the dropdown menu visibility.
    let (show_dropdown, set_show_dropdown) = signal(false);
    let (show_react_row, set_show_react_row) = signal(false);
    // Phase 3c.2: emoji picker open-state. The hover toolbar's smile
    // button (and the dropdown's More-reactions row) flips this; the
    // popover mounts below the row when open.
    let (emoji_picker_open, set_emoji_picker_open) = signal(false);

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
    // Phase 2a Task 12: hover-toolbar thread + quick-reaction targets.
    // `on_open_thread` is the same Callback plumbed through Task 11 for the
    // swipe-right gesture; the `start thread` toolbar button reuses it so
    // desktop users get the same affordance without swipes.
    let msg_for_thread = message.clone();
    let msg_for_quick_react = message.clone();

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

    // Four-phase data-state lifecycle on the inner .mobile-action-sheet
    // div. Driving property: transform — style.css declares
    // `.mobile-action-sheet { transform: translateY(100%); transition:
    // transform 0.3s cubic-bezier(...) }` and `.mobile-action-sheet.open
    // { transform: translateY(0) }`.
    //
    // The lifecycle is mirrored from `show_sheet`. While the user is
    // dragging the sheet down, the inline style sets `transition: none`
    // (line ~1274 below); under that condition transitionend doesn't
    // fire, so the lifecycle simply doesn't advance during drag — the
    // sheet ends up in either Open or Closing depending on whether the
    // drag triggered dismissal.
    //
    // The existing `mobile-action-sheet open` class binding is kept so
    // the `.mobile-action-sheet.open` CSS selectors continue to match.
    //
    // See docs/specs/2026-04-27-event-based-waits-design.md
    // §`data-state` attribute pattern.
    let sheet_ref: NodeRef<leptos::html::Div> = NodeRef::new();
    let sheet_lifecycle = RwSignal::new(if show_sheet.get_untracked() {
        crate::components::lifecycle::LifecycleState::Open
    } else {
        crate::components::lifecycle::LifecycleState::Closed
    });

    Effect::new(move |prev: Option<bool>| {
        use crate::components::lifecycle::{advance, is_zero_duration, LifecycleState};
        let now_open = show_sheet.get();
        if prev.is_none() || prev == Some(now_open) {
            sheet_lifecycle.set(if now_open {
                LifecycleState::Open
            } else {
                LifecycleState::Closed
            });
            return now_open;
        }
        sheet_lifecycle.set(if now_open {
            LifecycleState::Opening
        } else {
            LifecycleState::Closing
        });
        if let Some(el) = sheet_ref.get_untracked() {
            if is_zero_duration(el.as_ref()) {
                sheet_lifecycle.set(advance(sheet_lifecycle.get_untracked()));
            }
        }
        now_open
    });

    let on_sheet_transition_end = move |ev: web_sys::TransitionEvent| {
        use crate::components::lifecycle::advance;
        if ev.property_name() == "transform" {
            sheet_lifecycle.update(|s| *s = advance(*s));
        }
    };
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
        // Phase 2a Task 13 / spec §Long-press action sheet:
        // dismiss on `drag >= 80 px` OR release-velocity > 200 px/s.
        // Transition is already disabled during drag via the inline
        // `transition: none` on the sheet's style binding; tapping the
        // overlay dismisses from the overlay's own click handler below.
        let velocity = if elapsed > 0.0 {
            distance / elapsed * 1000.0
        } else {
            0.0
        };
        if drag >= 80.0 || velocity > 200.0 {
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

    // Phase 2a Task 11: swipe-left quote-reply + swipe-right open-thread.
    // Contract (spec §Swipe gestures):
    // * `dx > 60` && `dx.abs() > 1.2 * dy.abs()` → open thread.
    // * `dx < -60` && `dx.abs() > 1.2 * dy.abs()` → reply (populates
    //    composer `replying_to` via the existing `on_click` callback).
    // * Below threshold → snap back over 200ms (transition on
    //   `.message`, disabled while `.message.is-dragging`). Reduced
    //   motion collapses to an instant state change via the CSS rule.
    // The 1.2× horizontal-dominance gate ensures vertical list-scroll
    // wins before the row captures the gesture.
    let drag_x = RwSignal::new(0.0f64);
    let is_dragging = RwSignal::new(false);
    let swipe_touch_start =
        send_wrapper::SendWrapper::new(std::rc::Rc::new(std::cell::Cell::new((0.0f64, 0.0f64))));
    let swipe_captured =
        send_wrapper::SendWrapper::new(std::rc::Rc::new(std::cell::Cell::new(false)));
    let swipe_start_for_start = swipe_touch_start.clone();
    let swipe_cap_for_start = swipe_captured.clone();
    let swipe_start_for_move = swipe_touch_start.clone();
    let swipe_cap_for_move = swipe_captured.clone();
    let swipe_cap_for_end = swipe_captured.clone();
    let msg_for_swipe_reply = message.clone();
    let msg_for_swipe_thread = message.clone();
    let swipe_reply_cb = on_click;
    let swipe_thread_cb = on_open_thread;

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
        // Record swipe start position (shared closure state below).
        // Guarded via `first_touch` because synthetic `Event`s dispatched
        // by the browser test harness lack a `touches` list — a plain
        // `ev.touches().get(0)` would hit a JS `TypeError` trying to
        // read `.get` on `undefined`.
        if let Some(t) = first_touch(&ev) {
            swipe_start_for_start.set((t.client_x() as f64, t.client_y() as f64));
            swipe_cap_for_start.set(false);
            drag_x.set(0.0);
            is_dragging.set(true);
        }
        set_long_press_active.set(true);
        // Start 500ms timer via web_sys. Use `once_into_js` so the
        // closure transfers ownership to JS — once the timer fires (or
        // `clear_timeout_with_handle` discards it on the cancel paths
        // below), the JS GC reclaims it. `Closure::once(...).forget()`
        // would leak per touchstart on mobile (issue #193).
        if let Some(window) = web_sys::window() {
            let open_sheet = set_show_sheet_open.clone();
            let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                set_long_press_active.set(false);
                open_sheet();
                // Haptic feedback. Headless test browsers lack
                // `navigator.vibrate`, so feature-detect first.
                if let Some(w) = web_sys::window() {
                    let nav = w.navigator();
                    if js_sys::Reflect::has(nav.as_ref(), &"vibrate".into()).unwrap_or(false) {
                        let _ = nav.vibrate_with_duration(25);
                    }
                }
            });
            if let Ok(id) = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 500)
            {
                lp_start.set(id);
            }
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
        // Finalise swipe gesture. Only act on release if we actually
        // captured the gesture (i.e. horizontal motion dominated and
        // crossed the 8px idle band) during touchmove.
        if swipe_cap_for_end.get() {
            let dx = drag_x.get_untracked();
            if dx > 60.0 {
                if let Some(cb) = swipe_thread_cb {
                    cb.run(msg_for_swipe_thread.clone());
                }
            } else if dx < -60.0 {
                if let Some(cb) = swipe_reply_cb {
                    cb.run(msg_for_swipe_reply.clone());
                }
            }
        }
        drag_x.set(0.0);
        is_dragging.set(false);
        swipe_cap_for_end.set(false);
    };

    let on_msg_touchmove = move |ev: web_sys::TouchEvent| {
        // Cancel long-press on any movement.
        let id = lp_move.get();
        if id != 0 {
            if let Some(w) = web_sys::window() {
                w.clear_timeout_with_handle(id);
            }
            lp_move.set(0);
        }
        set_long_press_active.set(false);
        // Track swipe gesture. Capture only when horizontal motion
        // exceeds vertical by ≥1.2× AND is at least 8px (idle band) —
        // until then, defer so vertical scroll can win. Guarded via
        // `first_touch` for the synthetic-Event case (see touchstart).
        if let Some(t) = first_touch(&ev) {
            let (sx, sy) = swipe_start_for_move.get();
            let dx = t.client_x() as f64 - sx;
            let dy = t.client_y() as f64 - sy;
            if !swipe_cap_for_move.get() && dx.abs() > 1.2 * dy.abs() && dx.abs() > 8.0 {
                swipe_cap_for_move.set(true);
            }
            if swipe_cap_for_move.get() {
                // Prevent native scroll/overscroll while we drive the
                // row transform. Safe because we only reach here after
                // the horizontal-dominance gate has passed.
                ev.prevent_default();
                drag_x.set(dx);
            }
        }
    };

    let base_class = msg_class.to_string();
    // Phase 2a Task 15 — spec §Accessibility / ARIA labels.
    // The row announces as one unit to screen readers:
    //   `role="article"` (implicit on `<article>`) + `aria-label="message
    //   from {display_name} at {timestamp}"`, where `{timestamp}` is the
    //   canonical `HH:MM` 24-hour stamp produced by
    //   `willow_client::util::format_timestamp`. We reuse the same
    //   formatter the meta-row uses for the collapsed run-hover stamp so
    //   the ARIA string never drifts from the visible time.
    // `tabindex="-1"` keeps the row programmatically focusable (arrow-key
    // navigation driven by the parent list) while leaving Tab focus on
    // the list container itself (single tab stop — see `chat.rs`).
    let row_aria_label = format!(
        "message from {} at {}",
        message.author_display_name,
        willow_client::util::format_timestamp(message.timestamp_ms)
    );
    view! {
        <article
            class=move || {
                // Compose base class + long-press-active + is-dragging.
                // `is-dragging` disables the 200ms snap-back transition
                // while the user's finger is driving the translate;
                // release path re-enables the transition so `drag_x`
                // returning to 0.0 animates naturally.
                let mut out = base_class.clone();
                if long_press_active.get() {
                    out.push_str(" long-press-active");
                }
                if is_dragging.get() {
                    out.push_str(" is-dragging");
                }
                out
            }
            style=move || {
                // Only emit a transform when there's actual horizontal
                // displacement, so idle rows get no inline style at all
                // (keeps the DOM diff clean and avoids clobbering other
                // transform-based effects).
                let dx = drag_x.get();
                if dx != 0.0 {
                    format!("transform: translateX({dx}px);")
                } else {
                    String::new()
                }
            }
            id=msg_dom_id
            role="article"
            aria-label=row_aria_label
            tabindex="-1"
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
                // Phase 2a Task 15 — spec §Accessibility / ARIA labels.
                // The author name is the profile-card entry point: render as
                // a real `<button>` with `aria-label="{name} — open profile"`
                // so screen readers announce it as an interactive affordance.
                // The click still opens the profile popover once
                // `profile-card.md` lands; today it's a visual-only button
                // (no click handler), matching the rest of the profile
                // affordances in this phase. `.author-btn` strips the UA
                // default button chrome so the visual is unchanged.
                let author_for_aria = author.clone();
                let author_aria = format!("{author_for_aria} — open profile");
                let author_pid_for_click = author_pid.clone();
                let on_author_click = move |ev: web_sys::MouseEvent| {
                    // Spec §Event-bus API: every avatar surface dispatches
                    // `open_profile(user_id, anchor)` — the anchor is the
                    // clicked button so the desktop popover can position
                    // itself against it.
                    use wasm_bindgen::JsCast as _;
                    let target = ev
                        .current_target()
                        .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok());
                    crate::profile::open_profile(&author_pid_for_click, target);
                };
                view! {
                    <div class="meta">
                        <button
                            class="author author-btn"
                            type="button"
                            aria-label=author_aria
                            style=format!("color: {author_color}")
                            on:click=on_author_click
                        >{author}</button>
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
                        {is_pinned.then(|| view! {
                            <span class="pinned-badge" aria-label="pinned">
                                {icons::icon_pin()}
                                " pinned"
                            </span>
                        })}
                        {has_queue_note.then(|| view! {
                            <span class="queued-badge" aria-label="queued">
                                {icons::icon_hourglass()}
                                " queued"
                            </span>
                        })}
                        {is_whisper.then(|| view! {
                            <span class="whisper-badge" aria-label="whisper">
                                {icons::icon_ear()}
                                " whisper"
                            </span>
                        })}
                    </div>
                }.into_any()
            } else {
                // Collapsed (grouped) row: expose an HH:MM stamp inside the
                // empty avatar column. CSS reveals it on `.message.grouped:hover`.
                // Pre-formatted 24-hour HH:MM stamp. Headered rows already
                // carry the timestamp in `.meta` so we only compute this for
                // grouped (run) rows.
                let run_hover_ts = willow_client::util::format_timestamp(message.timestamp_ms);
                view! {
                    <span class="run-hover-ts" aria-hidden="true">{run_hover_ts}</span>
                }.into_any()
            }}
            {if message.deleted {
                // Phase 2a Task 14 — spec §Copy / Deleted placeholder:
                // a withdrawn message renders a fixed italic stub in
                // `--ink-3`. Byte-exact copy comes from the spec's
                // "deleted placeholder" bullet.
                view! { <div class=body_class>"this message was withdrawn"</div> }.into_any()
            } else if let Some(attachment) = message.attachment.clone() {
                // Phase 3b T7 — typed `EventKind::FileMessage` rendering.
                // Replaces the legacy `[file:NAME:base64]` body-scrape
                // path (still active in the next branch for back-compat)
                // with the proper attachment surface picked by spec.
                use crate::components::attachment::{
                    pick, AttachmentFileCard, AttachmentImage, AttachmentKind,
                    AttachmentVoiceNote,
                };
                let kind = pick(&attachment.mime_type, attachment.size_bytes);
                let caption = if body_is_empty {
                    None
                } else {
                    Some(message.body.clone())
                };
                let inner = match kind {
                    AttachmentKind::Image => view! {
                        <AttachmentImage
                            hash=attachment.hash.clone()
                            filename=attachment.filename.clone()
                            size_bytes=attachment.size_bytes
                            mime_type=attachment.mime_type.clone()
                        />
                    }
                    .into_any(),
                    AttachmentKind::FileCard => view! {
                        <AttachmentFileCard
                            hash=attachment.hash.clone()
                            filename=attachment.filename.clone()
                            size_bytes=attachment.size_bytes
                        />
                    }
                    .into_any(),
                    AttachmentKind::VoiceNote => view! {
                        <AttachmentVoiceNote filename=attachment.filename.clone() />
                    }
                    .into_any(),
                };
                view! {
                    <div class="message-embeds">
                        {inner}
                        {caption.map(|c| view! { <div class=body_class>{c}</div> })}
                    </div>
                }
                .into_any()
            } else if body_is_empty {
                // Phase 2a Task 14 — spec §Edge cases: empty /
                // whitespace-only bodies (migration edge case) render
                // `empty message` instead of an invisible row. Same
                // italic `--ink-3` treatment as the deleted path.
                view! { <div class=body_class>"empty message"</div> }.into_any()
            } else if let Some((filename, data)) = file_info.clone() {
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
                // Segment pipeline: mentions → urls.
                //
                // Phase 2a Task 4: build `peers` from the app-state
                // members registry so `@handle` resolves in the row.
                // The display-name → handle derivation mirrors
                // `views::compute_messages_view` (see there for the
                // profile-card plan TODO —
                // `docs/plans/2026-04-21-ui-phase-2c-profile-card.md`).
                use leptos::context::use_context;
                let app_state = use_context::<crate::state::AppState>();
                let local_peer_str = app_state
                    .as_ref()
                    .map(|a| a.network.peer_id.get_untracked())
                    .unwrap_or_default();
                let local_peer: Option<willow_identity::EndpointId> =
                    local_peer_str.parse().ok();
                // TODO(plan: docs/plans/2026-04-21-ui-phase-2c-profile-card.md):
                // use real handles when profile data is plumbed. For
                // now handle ≈ display-name lowercased with spaces →
                // dots, matching the client-side projection.
                let peers_vec: Vec<willow_client::mentions::PeerRef> = app_state
                    .as_ref()
                    .map(|a| {
                        a.network
                            .peers
                            .get_untracked()
                            .into_iter()
                            .filter_map(|(pid_str, display, _online)| {
                                pid_str.parse::<willow_identity::EndpointId>().ok().map(
                                    |peer_id| {
                                        let handle = display.to_lowercase().replace(' ', ".");
                                        willow_client::mentions::PeerRef {
                                            peer_id,
                                            handle,
                                            display_name: display,
                                        }
                                    },
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mention_segments = if let Some(ref lp) = local_peer {
                    willow_client::mentions::parse_mentions(&body, &peers_vec, lp)
                } else {
                    // No local peer available (test / pre-init); fall
                    // through with a single text segment so the URL
                    // stage still runs over the full body.
                    vec![willow_client::mentions::Segment::Text(body.clone())]
                };
                // Pre-compute embeddable image URLs. Only `Text`
                // runs can carry URLs — mention pills, inline code
                // pills, and fenced blocks must never contribute to
                // auto-embed, so each mention-text is first passed
                // through `parse_code_segments` and only its `Text`
                // children feed `extract_urls`.
                let mut images: Vec<String> = Vec::new();
                for seg in &mention_segments {
                    if let willow_client::mentions::Segment::Text(t) = seg {
                        for code_seg in super::parse_code_segments(t) {
                            if let super::CodeSegment::Text(plain) = code_seg {
                                let (_, urls) = extract_urls(&plain);
                                images.extend(urls);
                            }
                        }
                    }
                }
                let has_images = !images.is_empty();
                view! {
                    <div class=body_class>
                        {mention_segments.into_iter().map(|seg| {
                            match seg {
                                willow_client::mentions::Segment::Mention { label, full_label, is_self, .. } => {
                                    view! {
                                        <super::MentionPill label=label full_label=full_label is_self=is_self />
                                    }.into_any()
                                }
                                willow_client::mentions::Segment::Text(t) => {
                                    // Phase 2a Task 5: run the code
                                    // pass *inside* each mention-text
                                    // run so `@user` pills stay out
                                    // of code spans and code pills
                                    // stay out of the URL autolink
                                    // stage (URL handling only fires
                                    // on the remaining plain text).
                                    let code_segments = super::parse_code_segments(&t);
                                    view! {
                                        {code_segments.into_iter().map(|cs| {
                                            match cs {
                                                super::CodeSegment::Inline(code_text) => {
                                                    view! { <super::InlineCodePill text=code_text /> }.into_any()
                                                }
                                                super::CodeSegment::Fenced { lang, body } => {
                                                    let lang_str = lang.unwrap_or_default();
                                                    view! { <super::FencedCodeBlock body=body lang=lang_str /> }.into_any()
                                                }
                                                super::CodeSegment::Text(plain) => {
                                                    let (url_segments, _) = extract_urls(&plain);
                                                    view! {
                                                        {url_segments.into_iter().map(|(text, is_url)| {
                                                            if is_url {
                                                                let display = text.clone();
                                                                view! {
                                                                    <a href=text target="_blank" rel="noopener noreferrer" class="message-link">{display}</a>
                                                                }.into_any()
                                                            } else {
                                                                view! { <span>{text}</span> }.into_any()
                                                            }
                                                        }).collect::<Vec<_>>()}
                                                    }.into_any()
                                                }
                                            }
                                        }).collect::<Vec<_>>()}
                                    }.into_any()
                                }
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
                                            <img class="embed-image" src=url_clone alt="embedded image" loading="lazy" referrerpolicy="no-referrer" />
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
            // Phase 2b Task 10: inline queue-note hint below the body.
            // Replaces the Phase 2a static strings with the shared
            // <InlineQueueNote> component so copy + ARIA live in one
            // place. `QueueNote::None` intentionally renders nothing
            // (zero layout contribution).
            {
                let msg_id = message.id.clone();
                // Peer-or-grove placeholder — spec ships with the peer
                // display name for direct messages; grove fan-out uses
                // the grove name. The projection currently only
                // surfaces the author, so we fall back to the author
                // display name in all variants. Replace with proper
                // recipient resolution when `letters-dms.md` lands.
                let peer_or_grove = message.author_display_name.clone();
                match queue_note {
                    QueueNote::Pending => view! {
                        <crate::components::InlineQueueNote
                            state=Signal::derive(|| crate::components::InlineState::Queued)
                            peer_or_grove=Signal::derive(move || peer_or_grove.clone())
                            message_id=Signal::derive(move || msg_id.clone())
                        />
                    }.into_any(),
                    QueueNote::LateArrival => view! {
                        <crate::components::InlineQueueNote
                            state=Signal::derive(|| crate::components::InlineState::InboundHeld)
                            peer_or_grove=Signal::derive(move || peer_or_grove.clone())
                            message_id=Signal::derive(move || msg_id.clone())
                        />
                    }.into_any(),
                    QueueNote::None => view! { <span class="queue-note-empty"/> }.into_any(),
                }
            }
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

                // Phase 2a Task 12: the desktop hover toolbar sits above the
                // row (top: -14px, right: 8px) and fades in on `.message:hover`
                // / `.message:focus-within`. The `more-horizontal` trigger here
                // owns the dropdown — clicking it still toggles `show_dropdown`
                // so the existing dropdown contents (Reply / Pin / React / Edit
                // / Delete / Download) stay intact. Quick-reaction slots render
                // placeholder emoji until `reactions-pins.md` lands a recency-
                // based quick-reactions list; each click routes through
                // `on_react` immediately. The whisper button is a layout
                // placeholder awaiting `whisper-mode.md` (`WhisperStart`) —
                // click is a no-op. Mobile viewports hide the toolbar via a
                // `@media (max-width: 720px)` CSS rule; the long-press action
                // sheet remains the mobile entry.
                let react_cb_for_quick = on_react;
                Some(view! {
                    <div class="message-actions">
                        <div class="message-hover-toolbar" role="toolbar" aria-label="message actions">
                            {if has_react {
                                ["\u{1F44D}", "\u{1F389}", "\u{2764}\u{FE0F}", "\u{1F642}", "\u{1F440}"]
                                    .into_iter()
                                    .map(|emoji| {
                                        let e_for_click = emoji.to_string();
                                        let e_for_label = emoji.to_string();
                                        let e_for_render = emoji.to_string();
                                        let cb = react_cb_for_quick;
                                        let msg = msg_for_quick_react.clone();
                                        Some(view! {
                                            <button
                                                class="toolbar-btn toolbar-btn--quick-react"
                                                type="button"
                                                aria-label=format!("react with {e_for_label}")
                                                on:click=move |ev| {
                                                    ev.stop_propagation();
                                                    if let Some(ref cb) = cb {
                                                        cb.run((msg.clone(), e_for_click.clone()));
                                                    }
                                                }
                                            >
                                                {e_for_render}
                                            </button>
                                        })
                                    })
                                    .collect::<Vec<_>>()
                            } else {
                                Vec::new()
                            }}
                            {has_react.then(|| view! {
                                <span class="toolbar-divider" aria-hidden="true"></span>
                            })}
                            {has_react.then(|| {
                                view! {
                                    <button
                                        class="toolbar-btn"
                                        type="button"
                                        aria-label="more reactions"
                                        on:click=move |ev| {
                                            ev.stop_propagation();
                                            // Phase 3c.2: open the emoji picker
                                            // popover instead of the legacy in-
                                            // dropdown react row. The picker
                                            // mounts below the row; on_select
                                            // routes through `on_react`.
                                            set_emoji_picker_open.update(|v| *v = !*v);
                                        }
                                    >
                                        {icons::icon_smile()}
                                    </button>
                                }
                            })}
                            {on_open_thread.map(|cb| {
                                let msg = msg_for_thread.clone();
                                view! {
                                    <button
                                        class="toolbar-btn"
                                        type="button"
                                        aria-label="start thread"
                                        on:click=move |ev| {
                                            ev.stop_propagation();
                                            cb.run(msg.clone());
                                        }
                                    >
                                        {icons::icon_thread()}
                                    </button>
                                }
                            })}
                            <button
                                class="toolbar-btn"
                                type="button"
                                aria-label="whisper reply"
                                // TODO(#562): permission-gated; no-op until
                                // `WhisperStart` EventKind lands and the local
                                // peer has permission to send a whisper reply
                                // to this row's author.
                                on:click=move |ev| { ev.stop_propagation(); }
                            >
                                {icons::icon_ear()}
                            </button>
                            <button
                                class="toolbar-btn action-trigger"
                                type="button"
                                aria-label="more actions"
                                on:click=move |ev| {
                                    ev.stop_propagation();
                                    set_show_dropdown.update(|v| *v = !*v);
                                    set_show_react_row.set(false);
                                }
                            >
                                {icons::icon_more_horizontal()}
                            </button>
                        </div>
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
                        // Phase 3c.2 emoji picker. Mounts conditionally
                        // inside `.message-actions` so its absolute
                        // positioning lands relative to the row. The
                        // smile button in the hover toolbar (above)
                        // flips `emoji_picker_open`; on_select routes
                        // through `on_react`. Recent shelf is empty in
                        // v1 — the static categories cover the picker
                        // contract until a follow-up plumbs
                        // `client.recent_reactions(channel)` through
                        // the row.
                        {
                            let react_cb_for_picker = on_react;
                            let msg_for_picker = message.clone();
                            let on_select = Callback::new(move |glyph: String| {
                                if let Some(cb) = react_cb_for_picker {
                                    cb.run((msg_for_picker.clone(), glyph));
                                }
                                set_emoji_picker_open.set(false);
                            });
                            let on_close = Callback::new(move |()| {
                                set_emoji_picker_open.set(false);
                            });
                            let recent = crate::reaction_recency::use_recent_reactions();
                            view! {
                                <Show when=move || emoji_picker_open.get()>
                                    <div class="message-emoji-picker-anchor">
                                        <crate::components::emoji_picker::EmojiPicker
                                            recent=recent
                                            on_select=on_select
                                            on_close=on_close
                                        />
                                    </div>
                                </Show>
                            }
                        }
                    </div>
                })
            } else {
                None
            }}
            // Phase 2a Task 13 / spec §Long-press action sheet:
            // the mobile bottom sheet renders in this exact order with
            // all-lowercase copy — quick-emoji row → `reply` →
            // `reply in thread` → `add reaction` → `pin`/`unpin` →
            // `copy text` → `edit` → `delete` (`--err` foreground via
            // `.sheet-item--delete`) → trailing `cancel`. Items that
            // depend on a permission-gated callback (`reply`, `pin`,
            // `edit`, `delete`, `add reaction`) are rendered only when
            // that callback is supplied; `copy text` and `cancel` are
            // always shown. `reply in thread` falls back to a no-op
            // when `on_open_thread` is unwired (thread pane belongs to
            // `thread-pane.md`, not this phase). `add reaction` is a
            // stand-in that re-opens the quick-emoji row until the
            // full picker lands in `reactions-pins.md`.
            //
            // Swipe-down ≥ 80 px OR release velocity > 200 px/s
            // dismisses; the overlay tap also dismisses. See
            // `on_sheet_touchend` for the arithmetic.
            {if show_actions {
                let reply_cb2 = on_click;
                let reply_msg2 = message.clone();
                let thread_cb2 = on_open_thread;
                let thread_msg2 = message.clone();
                let pin_cb2 = on_pin;
                let pin_msg2 = message.clone();
                let pin_label2 = pin_label.clone();
                let edit_cb2 = on_edit;
                let edit_msg2 = message.clone();
                let react_cb2 = on_react;
                let react_msg2 = message.clone();
                let body_for_copy = message.body.clone();

                let close_sheet = set_show_sheet_close;

                Some(view! {
                    <div
                        class=move || if show_sheet.get() { "mobile-action-sheet-overlay open" } else { "mobile-action-sheet-overlay" }
                        on:click=move |_| close_sheet()
                    ></div>
                    <div
                        class=move || if show_sheet.get() { "mobile-action-sheet open" } else { "mobile-action-sheet" }
                        node_ref=sheet_ref
                        data-state=move || sheet_lifecycle.get().as_str()
                        on:transitionend=on_sheet_transition_end
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
                        // Quick-emoji row — six hit targets from recency.
                        // TODO(#564): swap `REACTION_EMOJI` for the
                        // channel-scoped recency list once that spec
                        // lands. Rendered first so the sheet opens
                        // with the common case one tap away.
                        {if has_react {
                            let cb = react_cb2;
                            let msg = react_msg2.clone();
                            let close = close_sheet;
                            Some(view! {
                                <div class="sheet-emoji-row">
                                    {REACTION_EMOJI.iter().take(6).map(|emoji| {
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
                        {if has_reply {
                            let cb = reply_cb2;
                            let msg = reply_msg2.clone();
                            let close = close_sheet;
                            Some(view! {
                                <button class="sheet-item" on:click=move |ev| {
                                    ev.stop_propagation();
                                    if let Some(ref cb) = cb { cb.run(msg.clone()); }
                                    close();
                                }>"reply"</button>
                            })
                        } else { None }}
                        // `reply in thread` — always rendered per
                        // spec (thread-pane is a standalone feature,
                        // not permission-gated on the row). When
                        // `on_open_thread` is unwired the tap is a
                        // no-op (see `thread-pane.md`).
                        {
                            let cb_opt = thread_cb2;
                            let msg = thread_msg2.clone();
                            let close = close_sheet;
                            view! {
                                <button class="sheet-item" on:click=move |ev| {
                                    ev.stop_propagation();
                                    if let Some(cb) = cb_opt { cb.run(msg.clone()); }
                                    close();
                                }>"reply in thread"</button>
                            }
                        }
                        // `add reaction` — opens the full picker when
                        // `reactions-pins.md` lands. Today the
                        // quick-emoji row already sits at the top of
                        // the sheet, so this item re-focuses the sheet
                        // without dismissing; once the picker lands it
                        // will route there instead.
                        {has_react.then(|| view! {
                            <button class="sheet-item" on:click=move |ev| {
                                // TODO(#564): route to the full emoji
                                // picker here. For now the quick-emoji
                                // row above is the only path, so we
                                // keep the sheet open.
                                ev.stop_propagation();
                            }>"add reaction"</button>
                        })}
                        {if has_pin {
                            let cb = pin_cb2;
                            let msg = pin_msg2.clone();
                            // `pin_label` is either `Pin` or `Unpin`
                            // — lowercase it to match spec copy.
                            let label = pin_label2.to_lowercase();
                            let close = close_sheet;
                            Some(view! {
                                <button class="sheet-item" on:click=move |ev| {
                                    ev.stop_propagation();
                                    if let Some(ref cb) = cb { cb.run(msg.clone()); }
                                    close();
                                }>{label}</button>
                            })
                        } else { None }}
                        // `copy text` is a free action — available on
                        // every row regardless of permissions. Uses
                        // the shared `copy_to_clipboard` helper (same
                        // clipboard-API + textarea fallback as invite
                        // codes + fenced-code copy).
                        {
                            let close = close_sheet;
                            view! {
                                <button class="sheet-item" on:click=move |ev| {
                                    ev.stop_propagation();
                                    crate::util::copy_to_clipboard(&body_for_copy);
                                    close();
                                }>"copy text"</button>
                            }
                        }
                        {if has_edit {
                            let cb = edit_cb2;
                            let msg = edit_msg2.clone();
                            let close = close_sheet;
                            Some(view! {
                                <button class="sheet-item" on:click=move |ev| {
                                    ev.stop_propagation();
                                    if let Some(ref cb) = cb { cb.run(msg.clone()); }
                                    close();
                                }>"edit"</button>
                            })
                        } else { None }}
                        {if has_delete {
                            let close = close_sheet;
                            Some(view! {
                                <button class="sheet-item sheet-danger sheet-item--delete" on:click=move |ev| {
                                    ev.stop_propagation();
                                    close();
                                    set_show_del_confirm.set(true);
                                }>"delete"</button>
                            })
                        } else { None }}
                        <button class="sheet-item sheet-cancel" on:click=move |_| close_sheet()>"cancel"</button>
                    </div>
                })
            } else { None }}
            {if has_reactions {
                let react_cb = on_react_for_reactions;
                let msg_for_strip = message.clone();
                let raw_reactions = message.reactions.clone();
                // Local display name from AppState — drives the
                // `.reaction-pill--reacted` highlight on pills the
                // viewer has clicked. Falls back to `None` (no
                // highlight) when the AppState context is absent
                // (e.g. unit-test mounts without the full shell).
                let local_name: String = use_context::<crate::state::AppState>()
                    .map(|app| app.server.display_name.get_untracked())
                    .unwrap_or_default();
                let on_react_curried = Callback::new(move |emoji: String| {
                    if let Some(ref cb) = react_cb {
                        cb.run((msg_for_strip.clone(), emoji));
                    }
                });
                let on_open_picker = Callback::new(move |()| {
                    set_emoji_picker_open.set(true);
                });
                Some(view! {
                    <crate::components::reactions::ReactionStrip
                        reactions=raw_reactions
                        local_display_name=local_name
                        on_react=on_react_curried
                        on_open_picker=on_open_picker
                    />
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
                        title="withdraw message?"
                        message=Signal::derive(|| "this removes it from every peer's view. it was already read by some.".to_string())
                        confirm_text="withdraw"
                        cancel_text="keep"
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
        </article>
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
