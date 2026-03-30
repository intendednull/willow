//! # Call Page Component
//!
//! Full-screen call view that replaces the chat area when the user is in a
//! voice channel. Contains a top bar (channel name, participant count, timer),
//! a participant grid with [`ParticipantTile`]s, and a frosted-glass control
//! strip for mute, deafen, camera, screen share, and disconnect.

use leptos::prelude::*;
use send_wrapper::SendWrapper;

use crate::app::{VoiceManagerHandle, WebClientHandle};
use crate::components::ParticipantTile;
use crate::icons;
use crate::state::{AppState, AppWriteSignals, CallLayout, VideoSource};

/// Render a participant tile, optionally passing a video stream.
///
/// Because Leptos `#[prop(optional)]` expects the inner `T` when passing a
/// value (not `Option<T>`), we branch on whether a stream exists.
#[allow(clippy::too_many_arguments)]
fn render_tile(
    peer_id: String,
    display_name: String,
    video_stream: Option<SendWrapper<web_sys::MediaStream>>,
    is_speaking: bool,
    is_muted: bool,
    is_focused: bool,
    is_local_camera: bool,
    on_click: Callback<String>,
) -> impl IntoView {
    if let Some(stream) = video_stream {
        view! {
            <ParticipantTile
                peer_id=peer_id
                display_name=display_name
                video_stream=stream
                is_speaking=is_speaking
                is_muted=is_muted
                is_focused=is_focused
                is_local_camera=is_local_camera
                on_click=on_click
            />
        }
        .into_any()
    } else {
        view! {
            <ParticipantTile
                peer_id=peer_id
                display_name=display_name
                is_speaking=is_speaking
                is_muted=is_muted
                is_focused=is_focused
                is_local_camera=is_local_camera
                on_click=on_click
            />
        }
        .into_any()
    }
}

/// Format seconds as `MM:SS` or `HH:MM:SS` for the call duration timer.
fn format_duration(seconds: u32) -> String {
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// The call page, shown in place of the chat view when `show_call_page` is
/// true. Reads voice state from context and renders participant tiles with
/// grid or focus layout.
#[component]
pub fn CallPage(
    /// Called when the user clicks the disconnect button.
    on_disconnect: Callback<()>,
    /// Called when the user clicks the mute button.
    on_mute: Callback<()>,
    /// Called when the user clicks the deafen button.
    on_deafen: Callback<()>,
) -> impl IntoView {
    let app_state = use_context::<AppState>().unwrap();
    let write = use_context::<AppWriteSignals>().unwrap();
    let handle = use_context::<WebClientHandle>().unwrap();
    let vm = use_context::<VoiceManagerHandle>().unwrap();

    // Local video stream — stored globally in VoiceState so it survives remounts.
    let local_video_stream = app_state.voice.local_video_stream;

    // Duration timer — increments every second. Clean up on unmount so
    // timers do not stack across call-page remounts.
    let (duration, set_duration) = signal(0u32);
    let timer_handle = set_interval_with_handle(
        move || set_duration.update(|d| *d += 1),
        std::time::Duration::from_millis(1000),
    )
    .expect("set_interval failed");
    on_cleanup(move || timer_handle.clear());

    // Layout state.
    let layout = app_state.ui.call_layout;

    // Camera button click handler.
    let vm_camera = vm.clone();
    let on_camera_click = move |_| {
        let current_source = app_state.voice.video_source.get_untracked();

        if current_source == Some(VideoSource::Camera) {
            // Toggle off — stop camera.
            vm_camera.borrow_mut().stop_video_share();
            write.voice.set_video_source.set(None);
            write.voice.set_local_video_stream.set(None);
            return;
        }

        // Stop any existing share first.
        if current_source.is_some() {
            vm_camera.borrow_mut().stop_video_share();
            write.voice.set_video_source.set(None);
            write.voice.set_local_video_stream.set(None);
        }

        // MUST call getUserMedia synchronously in click handler for gesture.
        let window = web_sys::window().unwrap();
        let navigator = window.navigator();
        let Ok(media_devices) = navigator.media_devices() else {
            tracing::error!("No media devices available");
            return;
        };
        let constraints = web_sys::MediaStreamConstraints::new();
        constraints.set_video(&true.into());
        constraints.set_audio(&false.into());
        let Ok(promise) = media_devices.get_user_media_with_constraints(&constraints) else {
            tracing::error!("getUserMedia failed");
            return;
        };

        let vm2 = vm_camera.clone();
        let write2 = write;
        let on_success =
            wasm_bindgen::closure::Closure::once(move |stream: wasm_bindgen::JsValue| {
                use wasm_bindgen::JsCast;
                let stream: web_sys::MediaStream = stream.unchecked_into();
                let stream_for_signal = SendWrapper::new(stream.clone());
                vm2.borrow_mut().start_camera(stream);
                write2.voice.set_video_source.set(Some(VideoSource::Camera));
                write2
                    .voice
                    .set_local_video_stream
                    .set(Some(stream_for_signal));
            });
        let on_error = wasm_bindgen::closure::Closure::once(move |_err: wasm_bindgen::JsValue| {
            tracing::error!("Camera access denied");
        });
        let _ = promise.then2(&on_success, &on_error);
        on_success.forget();
        on_error.forget();
    };

    // Screen share button click handler.
    let vm_screen = vm.clone();
    let on_screen_click = move |_| {
        let current_source = app_state.voice.video_source.get_untracked();

        if current_source == Some(VideoSource::Screen) {
            // Toggle off — stop screen share.
            vm_screen.borrow_mut().stop_video_share();
            write.voice.set_video_source.set(None);
            write.voice.set_local_video_stream.set(None);
            return;
        }

        // Stop any existing share first.
        if current_source.is_some() {
            vm_screen.borrow_mut().stop_video_share();
            write.voice.set_video_source.set(None);
            write.voice.set_local_video_stream.set(None);
        }

        // MUST call getDisplayMedia synchronously in click handler for gesture.
        let window = web_sys::window().unwrap();
        let navigator = window.navigator();
        let Ok(media_devices) = navigator.media_devices() else {
            tracing::error!("No media devices available");
            return;
        };
        let Ok(promise) = media_devices.get_display_media() else {
            tracing::error!("getDisplayMedia failed");
            return;
        };

        let vm2 = vm_screen.clone();
        let write2 = write;
        let on_success =
            wasm_bindgen::closure::Closure::once(move |stream: wasm_bindgen::JsValue| {
                use wasm_bindgen::JsCast;
                let stream: web_sys::MediaStream = stream.unchecked_into();
                let stream_for_signal = SendWrapper::new(stream.clone());
                vm2.borrow_mut().start_screen_share(stream.clone());
                write2.voice.set_video_source.set(Some(VideoSource::Screen));
                write2
                    .voice
                    .set_local_video_stream
                    .set(Some(stream_for_signal));

                // Listen for the browser's "Stop sharing" chrome button.
                let tracks = stream.get_video_tracks();
                let track_val = tracks.get(0);
                if !track_val.is_undefined() {
                    let track: web_sys::MediaStreamTrack = track_val.unchecked_into();
                    let vm_ended = vm2.clone();
                    let on_ended = wasm_bindgen::closure::Closure::once(move || {
                        vm_ended.borrow_mut().stop_video_share();
                        write2.voice.set_local_video_stream.set(None);
                        write2.voice.set_video_source.set(None);
                    });
                    track.set_onended(Some(on_ended.as_ref().unchecked_ref()));
                    on_ended.forget();
                }
            });
        let on_error = wasm_bindgen::closure::Closure::once(move |_err: wasm_bindgen::JsValue| {
            tracing::error!("Screen share denied or cancelled");
        });
        let _ = promise.then2(&on_success, &on_error);
        on_success.forget();
        on_error.forget();
    };

    // Disconnect handler — also clear call page.
    let on_disconnect_click = move |_| {
        write.voice.set_video_source.set(None);
        write.voice.set_local_video_stream.set(None);
        write.voice.set_remote_video_streams.update(|m| m.clear());
        write.ui.set_show_call_page.set(false);
        write.ui.set_call_layout.set(CallLayout::default());
        on_disconnect.run(());
    };

    let on_mute_click = move |_| on_mute.run(());
    let on_deafen_click = move |_| on_deafen.run(());

    view! {
        <div class="call-page">
            // Top bar
            <div class="call-top-bar">
                <div class="call-channel-name">
                    <span class="call-live-dot"></span>
                    {move || app_state.voice.voice_channel_name.get()}
                </div>
                <div style="display: flex; align-items: center; gap: 12px;">
                    <span class="call-participant-count">
                        {move || {
                            let ch = app_state.voice.voice_channel.get().unwrap_or_default();
                            let map = app_state.voice.voice_participants_map.get();
                            let count = map.get(&ch).map(|v| v.len()).unwrap_or(0); // local already in map
                            format!("{count} participant{}", if count != 1 { "s" } else { "" })
                        }}
                    </span>
                    <span class="call-timer">{move || format_duration(duration.get())}</span>
                    <button
                        class="call-layout-toggle"
                        title=move || {
                            match layout.get() {
                                CallLayout::Grid => "Focus mode",
                                CallLayout::Focus(_) => "Grid mode",
                            }
                        }
                        on:click=move |_| {
                            let current = layout.get();
                            match current {
                                CallLayout::Focus(_) => write.ui.set_call_layout.set(CallLayout::Grid),
                                CallLayout::Grid => {
                                    // Focus on first participant if any
                                    let ch = app_state.voice.voice_channel.get_untracked().unwrap_or_default();
                                    let map = app_state.voice.voice_participants_map.get_untracked();
                                    if let Some(participants) = map.get(&ch) {
                                        if let Some(first) = participants.first() {
                                            write.ui.set_call_layout.set(CallLayout::Focus(first.clone()));
                                        }
                                    }
                                }
                            }
                        }
                    >
                        {move || {
                            match layout.get() {
                                CallLayout::Grid => icons::icon_maximize().into_any(),
                                CallLayout::Focus(_) => icons::icon_grid().into_any(),
                            }
                        }}
                    </button>
                </div>
            </div>

            // Participant grid
            {move || {
                let ch = app_state.voice.voice_channel.get().unwrap_or_default();
                let participants_map = app_state.voice.voice_participants_map.get();
                let local_peer_id = handle.peer_id();
                let local_name = handle.display_name();
                let remote_participants: Vec<String> = participants_map
                    .get(&ch)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| p != &local_peer_id)
                    .collect();
                let speaking = app_state.voice.speaking_peers.get();
                let remote_streams = app_state.voice.remote_video_streams.get();
                let muted = app_state.voice.voice_muted.get();
                let video_source = app_state.voice.video_source.get();
                let current_layout = layout.get();

                // Total participant count including self.
                let total = remote_participants.len() + 1;

                let grid_class = match &current_layout {
                    CallLayout::Focus(_) => "call-grid focus".to_string(),
                    CallLayout::Grid if total == 1 => "call-grid single-participant".to_string(),
                    CallLayout::Grid if total == 2 => "call-grid two-participants".to_string(),
                    _ => "call-grid".to_string(),
                };

                // Build the list of all participants (local first, then remote).
                let mut tiles: Vec<leptos::tachys::view::any_view::AnyView> = Vec::new();

                // In focus layout, render the focused tile first, then thumbnails.
                if let CallLayout::Focus(ref focused_id) = current_layout {
                    let focused_pid = focused_id.clone();

                    // Render the focused tile.
                    let (f_name, f_stream, f_is_muted, f_is_speaking, _f_is_local, f_is_local_cam) =
                        if focused_pid == local_peer_id {
                            let local_spk = speaking.contains(&local_peer_id);
                            (local_name.clone(), local_video_stream.get(), muted, local_spk, true, video_source == Some(VideoSource::Camera))
                        } else {
                            let name = if let Ok(eid) = focused_pid.parse::<willow_identity::EndpointId>() {
                                handle.peer_display_name(&eid)
                            } else {
                                focused_pid.clone()
                            };
                            let stream = remote_streams.get(&focused_pid).cloned();
                            let is_spk = speaking.contains(&focused_pid);
                            (name, stream, false, is_spk, false, false)
                        };

                    let fpid = focused_pid.clone();
                    tiles.push(render_tile(
                        focused_pid,
                        f_name,
                        f_stream,
                        f_is_speaking,
                        f_is_muted,
                        true,
                        f_is_local_cam,
                        Callback::new(move |_pid: String| {
                            write.ui.set_call_layout.set(CallLayout::Grid);
                        }),
                    ).into_any());

                    // Thumbnail strip.
                    let mut thumb_views: Vec<leptos::tachys::view::any_view::AnyView> = Vec::new();

                    // Local in thumbnails if not focused.
                    if fpid != local_peer_id {
                        let local_spk_thumb = speaking.contains(&local_peer_id);
                        thumb_views.push(render_tile(
                            local_peer_id.clone(),
                            local_name.clone(),
                            local_video_stream.get(),
                            local_spk_thumb,
                            muted,
                            false,
                            video_source == Some(VideoSource::Camera),
                            Callback::new(move |pid: String| {
                                write.ui.set_call_layout.set(CallLayout::Focus(pid));
                            }),
                        ).into_any());
                    }

                    // Remote peers in thumbnails (except focused).
                    for pid in &remote_participants {
                        if *pid == fpid {
                            continue;
                        }
                        let name = if let Ok(eid) = pid.parse::<willow_identity::EndpointId>() {
                            handle.peer_display_name(&eid)
                        } else {
                            pid.clone()
                        };
                        let stream = remote_streams.get(pid).cloned();
                        let is_spk = speaking.contains(pid);
                        thumb_views.push(render_tile(
                            pid.clone(),
                            name,
                            stream,
                            is_spk,
                            false,
                            false,
                            false,
                            Callback::new(move |pid: String| {
                                write.ui.set_call_layout.set(CallLayout::Focus(pid));
                            }),
                        ).into_any());
                    }

                    if !thumb_views.is_empty() {
                        tiles.push(view! {
                            <div class="call-thumbnails">
                                {thumb_views}
                            </div>
                        }.into_any());
                    }
                } else {
                    // Grid layout — local user tile.
                    let local_spk_grid = speaking.contains(&local_peer_id);
                    tiles.push(render_tile(
                        local_peer_id.clone(),
                        local_name.clone(),
                        local_video_stream.get(),
                        local_spk_grid,
                        muted,
                        false,
                        video_source == Some(VideoSource::Camera),
                        Callback::new(move |pid: String| {
                            write.ui.set_call_layout.set(CallLayout::Focus(pid));
                        }),
                    ).into_any());

                    // Remote participant tiles.
                    for pid in &remote_participants {
                        let name = if let Ok(eid) = pid.parse::<willow_identity::EndpointId>() {
                            handle.peer_display_name(&eid)
                        } else {
                            pid.clone()
                        };
                        let stream = remote_streams.get(pid).cloned();
                        let is_spk = speaking.contains(pid);
                        tiles.push(render_tile(
                            pid.clone(),
                            name,
                            stream,
                            is_spk,
                            false,
                            false,
                            false,
                            Callback::new(move |pid: String| {
                                write.ui.set_call_layout.set(CallLayout::Focus(pid));
                            }),
                        ).into_any());
                    }
                }

                view! {
                    <div class=grid_class>
                        {tiles}
                    </div>
                }
            }}

            // Control strip
            <div class="call-controls">
                <div class="call-controls-bar">
                    <button
                        class=move || if app_state.voice.voice_muted.get() { "call-btn muted" } else { "call-btn" }
                        title=move || if app_state.voice.voice_muted.get() { "Unmute" } else { "Mute" }
                        on:click=on_mute_click
                    >
                        {move || if app_state.voice.voice_muted.get() {
                            icons::icon_mic_off().into_any()
                        } else {
                            icons::icon_mic().into_any()
                        }}
                    </button>
                    <button
                        class=move || if app_state.voice.voice_deafened.get() { "call-btn muted" } else { "call-btn" }
                        title=move || if app_state.voice.voice_deafened.get() { "Undeafen" } else { "Deafen" }
                        on:click=on_deafen_click
                    >
                        {move || if app_state.voice.voice_deafened.get() {
                            icons::icon_headphones_off().into_any()
                        } else {
                            icons::icon_headphones().into_any()
                        }}
                    </button>

                    <div class="call-controls-separator"></div>

                    <button
                        class=move || if app_state.voice.video_source.get() == Some(VideoSource::Camera) { "call-btn active" } else { "call-btn" }
                        title=move || if app_state.voice.video_source.get() == Some(VideoSource::Camera) { "Stop Camera" } else { "Start Camera" }
                        on:click=on_camera_click
                    >
                        {move || if app_state.voice.video_source.get() == Some(VideoSource::Camera) {
                            icons::icon_video().into_any()
                        } else {
                            icons::icon_video_off().into_any()
                        }}
                    </button>
                    <button
                        class=move || if app_state.voice.video_source.get() == Some(VideoSource::Screen) { "call-btn active" } else { "call-btn" }
                        title=move || if app_state.voice.video_source.get() == Some(VideoSource::Screen) { "Stop Sharing" } else { "Share Screen" }
                        on:click=on_screen_click
                    >
                        {icons::icon_monitor()}
                    </button>

                    <div class="call-controls-separator"></div>

                    <button
                        class="call-btn disconnect"
                        title="Disconnect"
                        on:click=on_disconnect_click
                    >
                        {icons::icon_phone_off()}
                    </button>
                </div>
            </div>
        </div>
    }
}
