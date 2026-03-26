//! # Participant Tile Component
//!
//! Renders a single call participant as a tile showing either a video stream
//! or a peer-ID-derived gradient avatar. Includes a display name overlay,
//! speaking glow, and muted badge.

use leptos::prelude::*;
use send_wrapper::SendWrapper;
use wasm_bindgen::JsCast;

use crate::icons;

/// Derive a unique gradient from a peer ID for the avatar background.
///
/// Splits the peer ID bytes in half, hashes each half, and picks two hue
/// values separated by 40-100 degrees for a visually distinct gradient.
fn peer_gradient(peer_id: &str) -> String {
    let hash1 = peer_id
        .bytes()
        .take(peer_id.len() / 2)
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let hash2 = peer_id
        .bytes()
        .skip(peer_id.len() / 2)
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let hue1 = hash1 % 360;
    let hue2 = (hue1 + 40 + hash2 % 60) % 360;
    format!("linear-gradient(135deg, hsl({hue1}, 45%, 35%), hsl({hue2}, 45%, 30%))")
}

/// A single participant tile in the call grid.
///
/// Shows either a `<video>` element (when a stream is available) or a
/// gradient avatar with the participant's initial. Clicking the tile fires
/// `on_click` with the peer ID for focus toggling.
#[component]
pub fn ParticipantTile(
    /// The participant's peer ID.
    peer_id: String,
    /// The participant's display name.
    display_name: String,
    /// Optional video stream to render in a `<video>` element.
    #[prop(optional)]
    video_stream: Option<SendWrapper<web_sys::MediaStream>>,
    /// Whether this participant is currently speaking.
    #[prop(default = false)]
    is_speaking: bool,
    /// Whether this participant's microphone is muted.
    #[prop(default = false)]
    is_muted: bool,
    /// Whether this tile is in focused (enlarged) mode.
    #[prop(default = false)]
    is_focused: bool,
    /// Whether this is the local camera feed (mirrors the video).
    #[prop(default = false)]
    is_local_camera: bool,
    /// Callback fired with the peer ID when the tile is clicked.
    #[prop(optional)]
    on_click: Option<Callback<String>>,
) -> impl IntoView {
    let video_ref = NodeRef::<leptos::html::Video>::new();

    // Bind srcObject when the video ref becomes available.
    let stream_for_effect = video_stream.clone();
    Effect::new(move |_| {
        if let Some(el) = video_ref.get() {
            if let Some(ref stream) = stream_for_effect {
                let media_el: &web_sys::HtmlMediaElement = el.as_ref();
                media_el.set_src_object(Some(stream));
                let _ = media_el.play();
            }
        }
    });

    let has_video = video_stream.is_some();
    let gradient = peer_gradient(&peer_id);
    let initial = display_name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();

    let pid_for_click = peer_id.clone();

    // Build CSS class string.
    let mut classes = String::from("participant-tile");
    if is_speaking {
        classes.push_str(" speaking");
    }
    if is_focused {
        classes.push_str(" focused");
    }
    if is_local_camera {
        classes.push_str(" local-camera");
    }

    let video_class = if is_local_camera {
        "local-camera"
    } else if has_video && !is_local_camera {
        "screen-share"
    } else {
        ""
    };

    view! {
        <div
            class=classes
            on:click=move |_| {
                if let Some(ref cb) = on_click {
                    cb.run(pid_for_click.clone());
                }
            }
        >
            {if has_video {
                view! {
                    <video
                        node_ref=video_ref
                        class=video_class
                        autoplay
                        playsinline
                        muted
                    />
                }.into_any()
            } else {
                view! {
                    <div class="tile-avatar" style=format!("background: {gradient}")>
                        {initial}
                    </div>
                }.into_any()
            }}
            <div class="tile-name">
                <span>{display_name}</span>
            </div>
            {if is_muted {
                Some(view! {
                    <div class="tile-muted-badge">
                        {icons::icon_mic_off()}
                    </div>
                })
            } else {
                None
            }}
        </div>
    }
}
