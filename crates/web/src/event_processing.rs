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
                write
                    .network
                    .set_connection_state
                    .set(crate::state::ConnectionState::Reconnecting);
            }
            // Phase 2b sync-queue pipeline.
            ClientEvent::QueueChanged(view) => {
                write.queue.set_view.set(view.clone());
            }
            ClientEvent::RelayStatusChanged(status) => {
                write.queue.set_relay_status.set(*status);
            }
            ClientEvent::DeviceOnlineChanged(online) => {
                write.queue.set_device_online.set(*online);
                // Keep the legacy `connection_status` string + tight
                // `connection_state` enum in lockstep with the
                // device-online transition.
                if *online {
                    write
                        .network
                        .set_connection_status
                        .set("connected".to_string());
                    write
                        .network
                        .set_connection_state
                        .set(crate::state::ConnectionState::Connected);
                } else {
                    write
                        .network
                        .set_connection_status
                        .set("offline".to_string());
                    write
                        .network
                        .set_connection_state
                        .set(crate::state::ConnectionState::Offline);
                }
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
            // Phase 1f: the first local MessageReceived per session
            // unlocks the notification-permission prompt path. We only
            // request permission after the user has shown intent
            // (their first send) — otherwise a cold-start prompt is
            // annoying and likely to be denied.
            ClientEvent::MessageReceived { is_local: true, .. } => {
                if let Some(notifier) = crate::notifications::use_notifier() {
                    if !notifier.local_send_seen() {
                        notifier.mark_local_send();
                        request_notification_permission(notifier);
                    }
                }
            }
            // All other events (MessageReceived, ChannelCreated, PeerConnected,
            // ProfileUpdated, etc.) are handled by derived signal selectors
            // that auto-update when the state actor notifies subscribers.
            _ => {}
        }
    }
}

/// Request `Notification.permission` from the browser and fire the
/// spec's sticky info toast if it resolves to `denied`. Called once
/// per session after the first local send.
fn request_notification_permission(notifier: crate::notifications::Notifier) {
    // `Notification.requestPermission()` returns a Promise. We don't
    // `await` it — the SendWrapper + notifier handle survives across
    // the closure.
    let js = "(function(){if(typeof Notification==='undefined')return 'unsupported';\
                if(Notification.permission==='granted')return 'granted';\
                if(Notification.permission==='denied')return 'denied';\
                try{return Notification.requestPermission();}catch(e){return 'error';}})()";
    let Ok(result) = js_sys::eval(js) else {
        return;
    };
    // `result` is either a string (immediate) or a Promise. Handle the
    // Promise path — catch both arms and fire the sticky toast on
    // `denied`.
    if let Some(s) = result.as_string() {
        if s == "denied" {
            notifier.show_permission_denied_once();
        }
        return;
    }
    use wasm_bindgen::JsCast;
    if let Ok(promise) = result.dyn_into::<js_sys::Promise>() {
        let notifier_ok = notifier.clone();
        let notifier_err = notifier;
        let on_ok: wasm_bindgen::closure::Closure<dyn FnMut(wasm_bindgen::JsValue)> =
            wasm_bindgen::closure::Closure::new(move |v: wasm_bindgen::JsValue| {
                if v.as_string().as_deref() == Some("denied") {
                    notifier_ok.show_permission_denied_once();
                }
            });
        let on_err: wasm_bindgen::closure::Closure<dyn FnMut(wasm_bindgen::JsValue)> =
            wasm_bindgen::closure::Closure::new(move |_err: wasm_bindgen::JsValue| {
                notifier_err.show_permission_denied_once();
            });
        let _ = promise.then2(&on_ok, &on_err);
        on_ok.forget();
        on_err.forget();
    }
}
