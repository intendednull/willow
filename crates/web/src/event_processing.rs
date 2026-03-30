//! Event processing logic extracted from the old poll loop.
//!
//! [`process_event_batch`] takes a batch of [`ClientEvent`]s and updates the
//! reactive signals via [`AppWriteSignals`]. It uses the same flag-based
//! approach (`needs_msg_refresh`, `needs_peer_refresh`, `needs_channel_refresh`)
//! as the old `set_interval` poll loop.

use std::collections::HashMap;

use willow_client::{ClientEvent, VoiceSignalPayload};

use crate::app::{
    handle_voice_answer, handle_voice_create_offer, handle_voice_offer, VoiceManagerHandle,
    WebClientHandle,
};
use crate::state::{AppState, AppWriteSignals};

/// Process a batch of [`ClientEvent`]s and update signals.
///
/// Reads current values from `state` (read signals) and writes to `write`
/// (write signals). Batches refreshes to avoid redundant signal updates.
pub fn process_event_batch(
    events: &[ClientEvent],
    handle: &WebClientHandle,
    state: &AppState,
    write: &AppWriteSignals,
    voice_manager: &VoiceManagerHandle,
) {
    use leptos::prelude::*;

    let mut needs_msg_refresh = false;
    let mut needs_peer_refresh = false;
    let mut needs_channel_refresh = false;

    for event in events {
        match event {
            ClientEvent::MessageReceived { .. } => {
                needs_msg_refresh = true;
            }
            ClientEvent::MessageEdited { .. }
            | ClientEvent::MessageDeleted { .. }
            | ClientEvent::ReactionAdded { .. } => {
                needs_msg_refresh = true;
            }
            ClientEvent::SyncCompleted { .. } => {
                needs_msg_refresh = true;
                needs_channel_refresh = true;
                needs_peer_refresh = true;
            }
            ClientEvent::PeerConnected(_) => {
                needs_peer_refresh = true;
                write
                    .network
                    .set_connection_status
                    .set("connected".to_string());
                write.network.set_loading.set(false);
            }
            ClientEvent::PeerDisconnected(_) => {
                needs_peer_refresh = true;
                // If no peers remain, show reconnecting status.
                if handle.peers().is_empty() {
                    write
                        .network
                        .set_connection_status
                        .set("reconnecting".to_string());
                }
            }
            ClientEvent::Listening(ref addr) => {
                if addr == "reconnecting" {
                    write
                        .network
                        .set_connection_status
                        .set("reconnecting".to_string());
                } else {
                    let status = state.network.connection_status.get_untracked();
                    if status == "connecting" {
                        write
                            .network
                            .set_connection_status
                            .set("connecting".to_string());
                    }
                }
            }
            ClientEvent::ChannelCreated(_) | ClientEvent::ChannelDeleted(_) => {
                needs_channel_refresh = true;
            }
            ClientEvent::ProfileUpdated { .. } => {
                let h = handle.clone();
                write.server.set_display_name.set(h.display_name());
                needs_peer_refresh = true;
            }
            ClientEvent::VoiceJoined {
                channel_id,
                peer_id,
            } => {
                let ch = channel_id.clone();
                let pid = peer_id.to_string();
                write.voice.set_voice_participants_map.update(|m| {
                    let participants = m.entry(ch.clone()).or_default();
                    if !participants.contains(&pid) {
                        participants.push(pid.clone());
                    }
                });
                // If we're in this channel, create offer to new peer.
                if state.voice.voice_channel.get_untracked() == Some(ch) {
                    let vm = voice_manager.clone();
                    let p = pid;
                    wasm_bindgen_futures::spawn_local(handle_voice_create_offer(vm, p));
                }
            }
            ClientEvent::VoiceLeft {
                channel_id,
                peer_id,
            } => {
                let ch = channel_id.clone();
                let pid = peer_id.to_string();
                write.voice.set_voice_participants_map.update(|m| {
                    if let Some(v) = m.get_mut(&ch) {
                        v.retain(|p| p != &pid);
                    }
                });
                // Remove remote video stream for this peer.
                let pid_for_stream = peer_id.to_string();
                write.voice.set_remote_video_streams.update(|m| {
                    m.remove(&pid_for_stream);
                });
                voice_manager
                    .borrow_mut()
                    .close_connection(&peer_id.to_string());
            }
            ClientEvent::VoiceSignal {
                from_peer, signal, ..
            } => {
                let vm = voice_manager.clone();
                let from = from_peer.to_string();
                match signal {
                    VoiceSignalPayload::Offer(sdp) => {
                        let s = sdp.clone();
                        wasm_bindgen_futures::spawn_local(handle_voice_offer(vm, from, s));
                    }
                    VoiceSignalPayload::Answer(sdp) => {
                        let s = sdp.clone();
                        wasm_bindgen_futures::spawn_local(handle_voice_answer(vm, from, s));
                    }
                    VoiceSignalPayload::IceCandidate(json) => {
                        let _ = vm.borrow().handle_ice_candidate(&from, json);
                    }
                }
            }
            ClientEvent::JoinLinkResponse { invite_data } => {
                match handle.accept_invite(invite_data) {
                    Ok(()) => {
                        refresh_all_signals(handle, write);
                        write.ui.set_join_token.set(None);
                        write.ui.set_join_status.set(String::new());
                        // Clear URL fragment to prevent re-trigger on refresh.
                        if let Some(window) = web_sys::window() {
                            let _ = window.history().ok().and_then(|h| {
                                h.replace_state_with_url(
                                    &wasm_bindgen::JsValue::NULL,
                                    "",
                                    Some("/"),
                                )
                                .ok()
                            });
                        }
                    }
                    Err(e) => {
                        tracing::error!(%e, "join link auto-accept failed");
                        write.ui.set_join_status.set(format!("denied:{e}"));
                    }
                }
            }
            ClientEvent::JoinLinkDenied { reason } => {
                write.ui.set_join_status.set(format!("denied:{reason}"));
            }
            _ => {}
        }
    }

    if needs_msg_refresh {
        let ch = state.chat.current_channel.get_untracked();
        let h = handle.clone();
        let new_msgs = h.messages(&ch);
        // Only update if messages actually changed (avoids destroying
        // open action sheets by re-rendering the message list).
        let old_msgs = state.chat.messages.get_untracked();
        let changed = new_msgs.len() != old_msgs.len()
            || new_msgs.last().map(|m| &m.id) != old_msgs.last().map(|m| &m.id)
            || new_msgs.iter().zip(old_msgs.iter()).any(|(a, b)| {
                a.id != b.id
                    || a.body != b.body
                    || a.edited != b.edited
                    || a.deleted != b.deleted
                    || a.reactions.len() != b.reactions.len()
            });
        if changed {
            write.chat.set_messages.set(new_msgs);
        }
        // Refresh pinned messages and labels.
        write.chat.set_pinned_messages.set(h.pinned_messages(&ch));
        let mut labels = HashMap::new();
        for msg in h.messages(&ch) {
            let label = if h.is_pinned(&ch, &msg.id) {
                "Unpin"
            } else {
                "Pin"
            };
            labels.insert(msg.id.clone(), label.to_string());
        }
        write.chat.set_pin_labels.set(labels);
        // Update unread counts from the active server.
        write.server.set_unread.set(h.unread_counts());
    }
    if needs_peer_refresh {
        let h = handle.clone();
        let peer_list: Vec<(String, String, bool)> = h
            .server_members()
            .into_iter()
            .map(|(id, name, online)| (id.to_string(), name, online))
            .collect();
        let count = peer_list.iter().filter(|(_, _, online)| *online).count();
        write.network.set_peers.set(peer_list);
        write.network.set_peer_count.set(count);
        if count > 0 {
            write
                .network
                .set_connection_status
                .set("connected".to_string());
        } else {
            write
                .network
                .set_connection_status
                .set("connecting".to_string());
        }
    }
    if needs_channel_refresh {
        let h = handle.clone();
        write.chat.set_channels.set(h.channels());
        write.server.set_roles.set(extract_roles(&h));
    }
    if needs_msg_refresh || needs_peer_refresh {
        // Roles may change via sync events, so refresh on any state change.
        write.server.set_roles.set(extract_roles(handle));
    }
}

/// Full refresh of all signals from the client. Used after server
/// creation, joining, and on initial load.
pub fn refresh_all_signals(handle: &WebClientHandle, write: &AppWriteSignals) {
    use leptos::prelude::*;

    write.server.set_servers.set(handle.server_list());
    write.chat.set_channels.set(handle.channels());
    write.network.set_peer_id.set(handle.peer_id());
    write.server.set_display_name.set(handle.display_name());
    write.server.set_roles.set(extract_roles(handle));
    if let Some(id) = handle.active_server_id() {
        write.server.set_active_server_id.set(id.to_string());
    }
    write
        .server
        .set_active_server_name
        .set(handle.active_server_name());
    let ch = handle.current_channel();
    write.chat.set_current_channel.set(ch.clone());
    write.chat.set_messages.set(handle.messages(&ch));
    write.ui.set_show_settings.set(false);
    write.ui.set_show_add_server.set(false);
}

/// Extract roles from the client's event-sourced state as a list of
/// `(role_id, role_name, permission_strings)` tuples for reactive signals.
pub fn extract_roles(handle: &WebClientHandle) -> Vec<(String, String, Vec<String>)> {
    handle.roles_data()
}
