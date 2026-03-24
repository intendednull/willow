//! # WebRTC Voice Manager
//!
//! Manages WebRTC peer connections for voice chat in the Leptos web app.
//! This module is WASM-only and uses web-sys for all browser API access.
//!
//! Each remote peer gets its own `RTCPeerConnection`. The local microphone
//! stream is acquired once and added to every connection. Signaling messages
//! (offers, answers, ICE candidates) are sent back through a callback that
//! forwards them via the client's gossipsub network.

use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{
    MediaStream, RtcConfiguration, RtcIceServer, RtcPeerConnection, RtcPeerConnectionIceEvent,
    RtcSdpType, RtcSessionDescriptionInit, RtcTrackEvent,
};

/// Callback for sending voice signaling data back through the network.
///
/// Arguments: `(target_peer_id, signal_type, payload)`
/// where `signal_type` is `"offer"`, `"answer"`, or `"ice"`.
type SignalCallback = Rc<dyn Fn(&str, &str, &str)>;

/// Manages WebRTC connections for voice chat.
///
/// Each remote peer has a dedicated `RTCPeerConnection`. The local microphone
/// stream is shared across all connections.
pub struct VoiceManager {
    /// One `RTCPeerConnection` per remote peer.
    connections: HashMap<String, RtcPeerConnection>,
    /// Local microphone stream (acquired once).
    local_stream: Option<MediaStream>,
    /// Callback to send signaling data: `(target_peer, signal_type, payload)`.
    on_signal: SignalCallback,
}

impl VoiceManager {
    /// Create a new `VoiceManager` with a signaling callback.
    ///
    /// The callback is invoked with `(target_peer_id, signal_type, payload)`
    /// whenever a signaling message needs to be sent to a remote peer.
    pub fn new(on_signal: impl Fn(&str, &str, &str) + 'static) -> Self {
        Self {
            connections: HashMap::new(),
            local_stream: None,
            on_signal: Rc::new(on_signal),
        }
    }

    /// Set the local microphone stream (acquired externally to avoid RefCell across await).
    pub fn set_local_stream(&mut self, stream: MediaStream) {
        self.local_stream = Some(stream);
    }

    /// Build an `RTCConfiguration` with a public STUN server.
    fn rtc_config() -> RtcConfiguration {
        let config = RtcConfiguration::new();
        let ice_servers = js_sys::Array::new();
        let server = RtcIceServer::new();
        let urls = js_sys::Array::new();
        urls.push(&"stun:stun.l.google.com:19302".into());
        server.set_urls(&urls);
        ice_servers.push(&server);
        config.set_ice_servers(&ice_servers);
        config
    }

    /// Request microphone access from the browser.
    ///
    /// Must be called before creating offers or handling offers so that local
    /// audio tracks can be added to peer connections.
    pub async fn acquire_microphone(&mut self) -> Result<(), String> {
        let window = web_sys::window().ok_or("no window")?;
        let navigator = window.navigator();
        let media_devices = navigator.media_devices().map_err(|_| "no media devices")?;

        let constraints = web_sys::MediaStreamConstraints::new();
        constraints.set_audio(&true.into());
        constraints.set_video(&false.into());

        let promise = media_devices
            .get_user_media_with_constraints(&constraints)
            .map_err(|_| "getUserMedia failed")?;

        let stream_js = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(|_| "microphone access denied")?;

        let stream: MediaStream = stream_js.unchecked_into();
        self.local_stream = Some(stream);
        Ok(())
    }

    /// Add local audio tracks to a peer connection.
    fn add_local_tracks(&self, pc: &RtcPeerConnection) {
        if let Some(ref stream) = self.local_stream {
            let tracks = stream.get_audio_tracks();
            for i in 0..tracks.length() {
                let track: web_sys::MediaStreamTrack = tracks.get(i).unchecked_into();
                pc.add_track_0(&track, stream);
            }
        }
    }

    /// Set up the `onicecandidate` handler for a peer connection.
    fn setup_ice_handler(&self, pc: &RtcPeerConnection, remote_peer: &str) {
        let signal_cb = self.on_signal.clone();
        let peer_id = remote_peer.to_string();
        let on_ice = Closure::wrap(Box::new(move |ev: RtcPeerConnectionIceEvent| {
            if let Some(candidate) = ev.candidate() {
                let json = js_sys::JSON::stringify(&candidate.to_json())
                    .unwrap_or_default()
                    .as_string()
                    .unwrap_or_default();
                signal_cb(&peer_id, "ice", &json);
            }
        }) as Box<dyn FnMut(RtcPeerConnectionIceEvent)>);
        pc.set_onicecandidate(Some(on_ice.as_ref().unchecked_ref()));
        // Intentional leak: closure must outlive the peer connection.
        on_ice.forget();
    }

    /// Set up the `ontrack` handler to play remote audio.
    fn setup_track_handler(pc: &RtcPeerConnection) {
        let on_track = Closure::wrap(Box::new(move |ev: RtcTrackEvent| {
            let streams = ev.streams();
            if streams.length() > 0 {
                let stream: MediaStream = streams.get(0).unchecked_into();
                if let Some(window) = web_sys::window() {
                    if let Some(document) = window.document() {
                        if let Ok(el) = document.create_element("audio") {
                            let audio: web_sys::HtmlMediaElement = el.unchecked_into();
                            audio.set_src_object(Some(&stream));
                            audio.set_autoplay(true);
                            let _ = audio.play();
                        }
                    }
                }
            }
        }) as Box<dyn FnMut(RtcTrackEvent)>);
        pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));
        // Intentional leak: closure must outlive the peer connection.
        on_track.forget();
    }

    /// Create an SDP offer and send it to a remote peer.
    ///
    /// This creates a new `RTCPeerConnection`, adds local audio tracks,
    /// sets up ICE and track handlers, creates an offer, sets the local
    /// description, and sends the offer via the signal callback.
    pub async fn create_offer(&mut self, remote_peer: &str) -> Result<(), String> {
        let pc = RtcPeerConnection::new_with_configuration(&Self::rtc_config())
            .map_err(|_| "failed to create peer connection")?;

        self.add_local_tracks(&pc);
        self.setup_ice_handler(&pc, remote_peer);
        Self::setup_track_handler(&pc);

        // Create offer.
        let offer = wasm_bindgen_futures::JsFuture::from(pc.create_offer())
            .await
            .map_err(|_| "create_offer failed")?;

        let offer_sdp = js_sys::Reflect::get(&offer, &"sdp".into())
            .unwrap_or_default()
            .as_string()
            .unwrap_or_default();

        // Set local description.
        let desc = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
        desc.set_sdp(&offer_sdp);
        wasm_bindgen_futures::JsFuture::from(pc.set_local_description(&desc))
            .await
            .map_err(|_| "set_local_description failed")?;

        // Send offer to remote peer.
        (self.on_signal)(remote_peer, "offer", &offer_sdp);

        self.connections.insert(remote_peer.to_string(), pc);
        Ok(())
    }

    /// Handle an incoming SDP offer from a remote peer.
    ///
    /// Creates a new `RTCPeerConnection`, sets the remote description,
    /// creates an answer, and sends it back.
    pub async fn handle_offer(&mut self, remote_peer: &str, sdp: &str) -> Result<(), String> {
        let pc = RtcPeerConnection::new_with_configuration(&Self::rtc_config())
            .map_err(|_| "failed to create peer connection")?;

        self.add_local_tracks(&pc);
        self.setup_ice_handler(&pc, remote_peer);
        Self::setup_track_handler(&pc);

        // Set remote description (the offer).
        let remote_desc = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
        remote_desc.set_sdp(sdp);
        wasm_bindgen_futures::JsFuture::from(pc.set_remote_description(&remote_desc))
            .await
            .map_err(|_| "set_remote_description failed")?;

        // Create answer.
        let answer = wasm_bindgen_futures::JsFuture::from(pc.create_answer())
            .await
            .map_err(|_| "create_answer failed")?;

        let answer_sdp = js_sys::Reflect::get(&answer, &"sdp".into())
            .unwrap_or_default()
            .as_string()
            .unwrap_or_default();

        let local_desc = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        local_desc.set_sdp(&answer_sdp);
        wasm_bindgen_futures::JsFuture::from(pc.set_local_description(&local_desc))
            .await
            .map_err(|_| "set_local_description failed")?;

        // Send answer back.
        (self.on_signal)(remote_peer, "answer", &answer_sdp);

        self.connections.insert(remote_peer.to_string(), pc);
        Ok(())
    }

    /// Handle an incoming SDP answer from a remote peer.
    pub async fn handle_answer(&self, remote_peer: &str, sdp: &str) -> Result<(), String> {
        let pc = self
            .connections
            .get(remote_peer)
            .ok_or("no connection for peer")?;

        let desc = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        desc.set_sdp(sdp);
        wasm_bindgen_futures::JsFuture::from(pc.set_remote_description(&desc))
            .await
            .map_err(|_| "set_remote_description failed")?;
        Ok(())
    }

    /// Handle an incoming ICE candidate from a remote peer.
    pub fn handle_ice_candidate(
        &self,
        remote_peer: &str,
        candidate_json: &str,
    ) -> Result<(), String> {
        let pc = self
            .connections
            .get(remote_peer)
            .ok_or("no connection for peer")?;

        let candidate_obj =
            js_sys::JSON::parse(candidate_json).map_err(|_| "invalid ICE candidate JSON")?;
        // Use add_ice_candidate with the parsed JS object directly.
        // The browser accepts RTCIceCandidateInit dictionaries natively.
        let _ = pc
            .add_ice_candidate_with_opt_rtc_ice_candidate_init(Some(candidate_obj.unchecked_ref()));
        Ok(())
    }

    /// Mute or unmute the local microphone.
    pub fn set_muted(&self, muted: bool) {
        if let Some(ref stream) = self.local_stream {
            let tracks = stream.get_audio_tracks();
            for i in 0..tracks.length() {
                let track: web_sys::MediaStreamTrack = tracks.get(i).unchecked_into();
                track.set_enabled(!muted);
            }
        }
    }

    /// Close the connection to a specific remote peer.
    pub fn close_connection(&mut self, remote_peer: &str) {
        if let Some(pc) = self.connections.remove(remote_peer) {
            pc.close();
        }
    }

    /// Close all connections and release the microphone.
    pub fn close_all(&mut self) {
        for (_, pc) in self.connections.drain() {
            pc.close();
        }
        if let Some(ref stream) = self.local_stream {
            let tracks = stream.get_tracks();
            for i in 0..tracks.length() {
                let track: web_sys::MediaStreamTrack = tracks.get(i).unchecked_into();
                track.stop();
            }
        }
        self.local_stream = None;
    }
}

/// Acquire the microphone as a standalone async function.
/// This avoids holding a `RefCell` borrow across an `.await` boundary.
pub async fn acquire_microphone_async() -> Result<MediaStream, String> {
    let window = web_sys::window().ok_or("no window")?;
    let navigator = window.navigator();
    let media_devices = navigator.media_devices().map_err(|_| "no media devices")?;

    let constraints = web_sys::MediaStreamConstraints::new();
    constraints.set_audio(&true.into());
    constraints.set_video(&false.into());

    let promise = media_devices
        .get_user_media_with_constraints(&constraints)
        .map_err(|_| "getUserMedia failed")?;

    let stream_js = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|_| "microphone access denied")?;

    Ok(stream_js.unchecked_into())
}
