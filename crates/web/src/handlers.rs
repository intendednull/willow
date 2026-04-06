//! Handler constructors for UI actions.
//!
//! Each function returns a closure for use as a Leptos event handler.
//! Async client methods are wrapped in `spawn_local` since Leptos
//! event handlers are synchronous.

use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::state::{AppState, AppWriteSignals};

/// Parse a hex string into an `EventHash`, logging on failure.
fn parse_event_hash(hex: &str) -> Option<willow_client::willow_state::EventHash> {
    hex.parse().ok()
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
        // Clear reply state immediately so the UI updates.
        write.chat.set_replying_to.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(reply_msg) = replying {
                if let Some(hash) = parse_event_hash(&reply_msg.id) {
                    let _ = h.send_reply(&ch, &hash, &body).await;
                }
            } else {
                let _ = h.send_message(&ch, &body).await;
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
        write.chat.set_editing.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(hash) = parse_event_hash(&message_id) {
                let _ = h.edit_message(&ch, &hash, &new_body).await;
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
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(hash) = parse_event_hash(&msg.id) {
                let _ = h.delete_message(&ch, &hash).await;
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
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(hash) = parse_event_hash(&msg.id) {
                let _ = h.react(&ch, &hash, &emoji).await;
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
        wasm_bindgen_futures::spawn_local(async move {
            if let Some(hash) = parse_event_hash(&mid) {
                if h.is_pinned(&ch, &hash).await {
                    let _ = h.unpin_message(&ch, &hash).await;
                } else {
                    let _ = h.pin_message(&ch, &hash).await;
                }
            }
        });
    }
}
