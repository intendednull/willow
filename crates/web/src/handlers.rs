//! Handler constructors for UI actions.
//!
//! Each function takes a `WebClientHandle`, `AppState`, and `AppWriteSignals`
//! and returns a closure suitable for use as a Leptos event handler.

use std::collections::HashMap;

use leptos::prelude::*;

use crate::app::WebClientHandle;
use crate::state::{AppState, AppWriteSignals};

/// Create a handler for sending messages (including replies).
pub fn make_send_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn(String) + Clone + 'static {
    move |body: String| {
        let ch = state.chat.current_channel.get_untracked();
        if let Some(reply_msg) = state.chat.replying_to.get_untracked() {
            let _ = handle.send_reply(&ch, &reply_msg.id, &body);
            write.chat.set_replying_to.set(None);
        } else {
            let _ = handle.send_message(&ch, &body);
        }
        write.chat.set_messages.set(handle.messages(&ch));
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
        let _ = handle.edit_message(&ch, &message_id, &new_body);
        write.chat.set_editing.set(None);
        write.chat.set_messages.set(handle.messages(&ch));
    }
}

/// Create a handler for deleting messages.
pub fn make_delete_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn(willow_client::DisplayMessage) + Clone + 'static {
    move |msg: willow_client::DisplayMessage| {
        let ch = state.chat.current_channel.get_untracked();
        let _ = handle.delete_message(&ch, &msg.id);
        write.chat.set_messages.set(handle.messages(&ch));
    }
}

/// Create a handler for adding/toggling reactions.
pub fn make_react_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn((willow_client::DisplayMessage, String)) + Clone + 'static {
    move |(msg, emoji): (willow_client::DisplayMessage, String)| {
        let ch = state.chat.current_channel.get_untracked();
        let _ = handle.react(&ch, &msg.id, &emoji);
        write.chat.set_messages.set(handle.messages(&ch));
    }
}

/// Create a handler for switching channels.
pub fn make_channel_click_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn(String) + Clone + 'static {
    let _ = state; // state used via write signals only
    move |name: String| {
        write.chat.set_current_channel.set(name.clone());
        write.ui.set_show_sidebar.set(false);
        write.ui.set_show_pinned.set(false);
        write.ui.set_show_call_page.set(false);
        write.chat.set_messages.set(handle.messages(&name));
        write
            .chat
            .set_pinned_messages
            .set(handle.pinned_messages(&name));
        let mut labels = HashMap::new();
        for msg in handle.messages(&name) {
            let label = if handle.is_pinned(&name, &msg.id) {
                "Unpin"
            } else {
                "Pin"
            };
            labels.insert(msg.id.clone(), label.to_string());
        }
        write.chat.set_pin_labels.set(labels);
        handle.switch_channel(&name);
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
        handle.switch_server(&id);
        write.server.set_active_server_id.set(id);
        write.server.set_servers.set(handle.server_list());
        let chs = handle.channels();
        write.chat.set_channels.set(chs.clone());
        let first_ch = chs
            .first()
            .cloned()
            .unwrap_or_else(|| "general".to_string());
        write.chat.set_current_channel.set(first_ch.clone());
        write.chat.set_messages.set(handle.messages(&first_ch));
        write
            .server
            .set_active_server_name
            .set(handle.active_server_name());
        write.ui.set_show_settings.set(false);
        write.ui.set_show_add_server.set(false);
    }
}

/// Create a handler for pinning/unpinning messages.
pub fn make_pin_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn(willow_client::DisplayMessage) + Clone + 'static {
    move |msg: willow_client::DisplayMessage| {
        let ch = state.chat.current_channel.get_untracked();
        if handle.is_pinned(&ch, &msg.id) {
            let _ = handle.unpin_message(&ch, &msg.id);
        } else {
            let _ = handle.pin_message(&ch, &msg.id);
        }
        write
            .chat
            .set_pinned_messages
            .set(handle.pinned_messages(&ch));
        let mut labels = HashMap::new();
        for m in handle.messages(&ch) {
            let label = if handle.is_pinned(&ch, &m.id) {
                "Unpin"
            } else {
                "Pin"
            };
            labels.insert(m.id.clone(), label.to_string());
        }
        write.chat.set_pin_labels.set(labels);
    }
}
