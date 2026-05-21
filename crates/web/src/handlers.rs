//! Handler constructors for UI actions.
//!
//! Each function returns a closure for use as a Leptos event handler.
//! Async client methods are wrapped in `spawn_local` since Leptos
//! event handlers are synchronous.
//!
//! ## Error reporting (issue #350)
//!
//! Async client mutations return `anyhow::Result<()>`. Previously each
//! handler discarded the error with `let _ = ...`, so a typed message
//! could vanish from the input with no feedback. Every site now routes
//! failures through [`warn_and_toast`], which logs at `WARN` and pushes
//! an `err` toast onto the ambient [`ToastStack`] so the user sees
//! something happened.
//!
//! The toast stack is captured *before* `spawn_local` (in the
//! synchronous handler closure) — `wasm_bindgen_futures::spawn_local`
//! does not preserve a reactive owner, so `use_context::<ToastStack>()`
//! must be resolved on the calling task. Mirrors the pattern PR #411
//! introduced in `sync_queue_view.rs`.

use std::fmt::Debug;

use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::components::{Toast, ToastStack};
use crate::state::{AppState, AppWriteSignals};

/// Parse a hex string into an `EventHash`, logging on failure.
fn parse_event_hash(hex: &str) -> Option<willow_client::willow_state::EventHash> {
    hex.parse().ok()
}

/// Log a UI handler failure at WARN and surface it to the user via a
/// toast.
///
/// `action` is a short human-readable verb-phrase (`"send message"`,
/// `"edit message"`, …). It appears in the tracing field and in the
/// toast copy so the user knows which action failed.
///
/// `toasts` is the optional toast stack captured at handler-call time
/// (before any `spawn_local`). It is `None` in headless test harnesses
/// or very early during boot — the `tracing::warn!` call still fires
/// in that case so the failure is never fully silent.
///
/// The convenience overload [`warn_and_toast`] resolves the stack from
/// context for callers running on a reactive owner; prefer that on
/// synchronous code paths.
pub fn warn_and_toast_with(action: &'static str, e: &dyn Debug, toasts: Option<&ToastStack>) {
    tracing::warn!(error = ?e, action, "ui handler failed");
    if let Some(stack) = toasts {
        stack.push(
            Toast::err(format!("Couldn't {action}. Try again."))
                .dedup(format!("handler-error:{action}"))
                .build(),
        );
    }
}

/// Reactive-owner shortcut for [`warn_and_toast_with`]: looks up the
/// ambient [`ToastStack`] via `use_context` and forwards.
///
/// Only safe to call on a reactive owner — for example, a Leptos
/// event-handler closure or a synchronous component body. Inside an
/// `async` block spawned via `wasm_bindgen_futures::spawn_local`, no
/// owner is preserved, so the toast stack must be captured on the
/// outer synchronous frame and passed via [`warn_and_toast_with`].
pub fn warn_and_toast(action: &'static str, e: &dyn Debug) {
    warn_and_toast_with(action, e, use_context::<ToastStack>().as_ref());
}

/// Create a handler for sending messages (including replies).
pub fn make_send_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn(String) + Clone + 'static {
    move |body: String| {
        let ch = state.chat.current_channel.get_untracked();
        let h = handle.clone();
        let replying = state.chat.replying_to.get_untracked();
        // Capture the toast stack on the outer (reactive-owner) frame
        // — `spawn_local` strips the owner so `use_context` inside the
        // async block would return None.
        let toasts = use_context::<ToastStack>();
        // Clear reply state immediately so the UI updates.
        write.chat.set_replying_to.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(reply_msg) = replying {
                if let Some(hash) = parse_event_hash(&reply_msg.id) {
                    if let Err(e) = h.send_reply(&ch, &hash, &body).await {
                        warn_and_toast_with("send reply", &e, toasts.as_ref());
                    }
                }
            } else if let Err(e) = h.send_message(&ch, &body).await {
                warn_and_toast_with("send message", &e, toasts.as_ref());
            }
        });
    }
}

/// Create a handler for editing messages.
pub fn make_edit_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn((String, String)) + Clone + 'static {
    move |(message_id, new_body): (String, String)| {
        let ch = state.chat.current_channel.get_untracked();
        let h = handle.clone();
        let toasts = use_context::<ToastStack>();
        write.chat.set_editing.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(hash) = parse_event_hash(&message_id) {
                if let Err(e) = h.edit_message(&ch, &hash, &new_body).await {
                    warn_and_toast_with("edit message", &e, toasts.as_ref());
                }
            }
        });
    }
}

/// Create a handler for deleting messages.
pub fn make_delete_handler(
    handle: WebClientHandle,
    state: AppState,
    _write: AppWriteSignals,
) -> impl Fn(willow_client::DisplayMessage) + Clone + 'static {
    move |msg: willow_client::DisplayMessage| {
        let ch = state.chat.current_channel.get_untracked();
        let h = handle.clone();
        let toasts = use_context::<ToastStack>();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(hash) = parse_event_hash(&msg.id) {
                if let Err(e) = h.delete_message(&ch, &hash).await {
                    warn_and_toast_with("delete message", &e, toasts.as_ref());
                }
            }
        });
    }
}

/// Create a handler for adding/toggling reactions.
pub fn make_react_handler(
    handle: WebClientHandle,
    state: AppState,
    _write: AppWriteSignals,
) -> impl Fn((willow_client::DisplayMessage, String)) + Clone + 'static {
    move |(msg, emoji): (willow_client::DisplayMessage, String)| {
        let ch = state.chat.current_channel.get_untracked();
        let h = handle.clone();
        let toasts = use_context::<ToastStack>();
        // Capture the tick context up front so the spawned future
        // doesn't need a context lookup. On success we bump it so
        // the recency `LocalResource` re-fires and the freshly-
        // clicked emoji floats to the top of the picker without
        // waiting for a channel switch.
        let tick = use_context::<crate::reaction_recency::RecencyRefreshTick>();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(hash) = parse_event_hash(&msg.id) {
                match h.react(&ch, &hash, &emoji).await {
                    Ok(()) => {
                        if let Some(crate::reaction_recency::RecencyRefreshTick(t)) = tick {
                            t.update(|n| *n = n.wrapping_add(1));
                        }
                    }
                    Err(e) => warn_and_toast_with("add reaction", &e, toasts.as_ref()),
                }
            }
        });
    }
}

/// Create a handler for switching channels.
pub fn make_channel_click_handler(
    handle: WebClientHandle,
    _state: AppState,
    write: AppWriteSignals,
) -> impl Fn(String) + Clone + 'static {
    move |name: String| {
        write.chat.set_current_channel.set(name.clone());
        write.ui.set_show_sidebar.set(false);
        write.ui.set_show_pinned.set(false);
        write.ui.set_show_call_page.set(false);
        let h = handle.clone();
        let n = name.clone();
        wasm_bindgen_futures::spawn_local(async move {
            h.switch_channel(&n).await;
        });
        // Clear unread for this channel.
        write.server.set_unread.update(|m| {
            m.remove(&name);
        });
    }
}

/// Create a handler for switching servers.
pub fn make_server_click_handler(
    handle: WebClientHandle,
    write: AppWriteSignals,
) -> impl Fn(String) + Clone + 'static {
    move |id: String| {
        let h = handle.clone();
        let w = write;
        let sid = id.clone();
        w.ui.set_show_settings.set(false);
        w.ui.set_show_add_server.set(false);
        wasm_bindgen_futures::spawn_local(async move {
            h.switch_server(&sid).await;
            // Derived signals will auto-update channels, messages, etc.
        });
    }
}

/// Create a handler for pinning/unpinning messages.
pub fn make_pin_handler(
    handle: WebClientHandle,
    state: AppState,
    _write: AppWriteSignals,
) -> impl Fn(willow_client::DisplayMessage) + Clone + 'static {
    move |msg: willow_client::DisplayMessage| {
        let ch = state.chat.current_channel.get_untracked();
        let h = handle.clone();
        let mid = msg.id.clone();
        let toasts = use_context::<ToastStack>();
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(hash) = parse_event_hash(&mid) {
                if h.is_pinned(&ch, &hash).await {
                    if let Err(e) = h.unpin_message(&ch, &hash).await {
                        warn_and_toast_with("unpin message", &e, toasts.as_ref());
                    }
                } else if let Err(e) = h.pin_message(&ch, &hash).await {
                    warn_and_toast_with("pin message", &e, toasts.as_ref());
                }
            }
        });
    }
}
