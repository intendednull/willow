//! Side-effect handler for ClientEvents.
//!
//! With derived signals handling all state-to-UI updates automatically,
//! this module only processes side effects: voice signaling, join link
//! responses, and connection status changes that require imperative action.

use willow_client::{ClientEvent, VoiceSignalPayload};

use crate::app::{
    handle_voice_answer, handle_voice_create_offer, handle_voice_offer, VoiceManagerHandle,
    WebClientHandle,
};
use crate::state::{AppState, AppWriteSignals};

/// Process a batch of [`ClientEvent`]s for side effects only.
///
/// State-derived signal updates (messages, channels, peers, roles, etc.)
/// are handled automatically by `DerivedStateActor` selectors. This function
/// only handles imperative side effects that can't be expressed as selectors.
pub fn process_event_batch(
    events: &[ClientEvent],
    handle: &WebClientHandle,
    state: &AppState,
    write: &AppWriteSignals,
    voice_manager: &VoiceManagerHandle,
) {
    use leptos::prelude::*;

    for event in events {
        match event {
            ClientEvent::PeerConnected(_) => {
                write.network.set_loading.set(false);
            }
            ClientEvent::Listening(ref addr) if addr == "reconnecting" => {
                write
                    .network
                    .set_connection_status
                    .set("reconnecting".to_string());
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
                let h = handle.clone();
                let data = invite_data.clone();
                let w = *write;
                wasm_bindgen_futures::spawn_local(async move {
                    match h.accept_invite(&data).await {
                        Ok(()) => {
                            w.ui.set_join_token.set(None);
                            w.ui.set_join_status.set(String::new());
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
                            w.ui.set_join_status.set(format!("denied:{e}"));
                        }
                    }
                });
            }
            ClientEvent::JoinLinkDenied { reason } => {
                write.ui.set_join_status.set(format!("denied:{reason}"));
            }
            // All other events (MessageReceived, ChannelCreated, PeerConnected,
            // ProfileUpdated, etc.) are handled by derived signal selectors
            // that auto-update when the state actor notifies subscribers.
            _ => {}
        }
    }
}
