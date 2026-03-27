//! # WebRTC Voice Manager
//!
//! Manages WebRTC peer connections for voice chat in the Leptos web app.
//! This module is WASM-only and uses web-sys for all browser API access.
//!
//! Each remote peer gets its own `RTCPeerConnection`. The local microphone
//! stream is acquired once and added to every connection. Signaling messages
//! (offers, answers, ICE candidates) are sent back through a callback that
//! forwards them via the client's gossipsub network.
//!
//! ## Perfect Negotiation
//!
//! Connections are reused across renegotiations (e.g. when adding/removing
//! video tracks). The "perfect negotiation" pattern resolves offer collisions:
//! the peer with the lower ID is "polite" and will rollback its own offer
//! when a collision is detected.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{
    AnalyserNode, AudioContext, MediaStream, MediaStreamAudioSourceNode, RtcConfiguration,
    RtcIceServer, RtcPeerConnection, RtcPeerConnectionIceEvent, RtcRtpSender, RtcSdpType,
    RtcSessionDescriptionInit, RtcTrackEvent,
};

use crate::state::VideoSource;

/// Callback invoked whenever the set of currently-speaking peer IDs changes.
type SpeakingCallback = Rc<dyn Fn(HashSet<String>)>;

/// Analyses audio volume on `AnalyserNode`s to detect which peers are speaking.
///
/// Each peer (including the local user) has an `AnalyserNode` fed by their
/// `MediaStream`. A `setInterval` timer polls all analysers at ~60 ms and
/// calls `on_speaking_change` with the set of peer IDs whose average
/// frequency-domain amplitude exceeds a threshold.
pub struct SpeakingDetector {
    audio_context: AudioContext,
    /// Analysers keyed by peer ID. Shared with the polling closure via `Rc<RefCell>`.
    analysers: Rc<RefCell<HashMap<String, AnalyserNode>>>,
    /// Source nodes kept alive so the browser does not garbage-collect them.
    /// Shared with the `ontrack` closure via `Rc<RefCell>` so it can store
    /// source nodes created for remote peer audio streams.
    sources: Rc<RefCell<HashMap<String, MediaStreamAudioSourceNode>>>,
    on_speaking_change: SpeakingCallback,
    interval_id: Option<i32>,
}

impl SpeakingDetector {
    /// Create a new `SpeakingDetector` with the given change callback.
    ///
    /// The `AudioContext` is created immediately (caller must ensure a user
    /// gesture has already occurred so the browser allows it).
    pub fn new(on_change: impl Fn(HashSet<String>) + 'static) -> Result<Self, String> {
        let audio_context =
            AudioContext::new().map_err(|e| format!("AudioContext::new failed: {e:?}"))?;
        // Resume the context in case it was created in a suspended state.
        let _ = audio_context.resume();
        Ok(Self {
            audio_context,
            analysers: Rc::new(RefCell::new(HashMap::new())),
            sources: Rc::new(RefCell::new(HashMap::new())),
            on_speaking_change: Rc::new(on_change),
            interval_id: None,
        })
    }

    /// Connect a peer's `MediaStream` to an `AnalyserNode` for volume monitoring.
    pub fn add_stream(&mut self, peer_id: &str, stream: &MediaStream) {
        // Create a source node from the stream.
        let source = match self.audio_context.create_media_stream_source(stream) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("SpeakingDetector: create_media_stream_source failed: {e:?}");
                return;
            }
        };

        // Create an analyser node.
        let analyser = match self.audio_context.create_analyser() {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("SpeakingDetector: create_analyser failed: {e:?}");
                return;
            }
        };
        analyser.set_fft_size(256);

        // Connect source -> analyser (no output needed; we poll manually).
        if let Err(e) = source.connect_with_audio_node(&analyser) {
            tracing::warn!("SpeakingDetector: connect failed: {e:?}");
            return;
        }

        self.analysers
            .borrow_mut()
            .insert(peer_id.to_string(), analyser);
        self.sources
            .borrow_mut()
            .insert(peer_id.to_string(), source);
    }

    /// Remove a peer's analyser and source node (e.g. when they disconnect).
    pub fn remove_peer(&mut self, peer_id: &str) {
        self.analysers.borrow_mut().remove(peer_id);
        self.sources.borrow_mut().remove(peer_id);
    }

    /// Start polling all analysers every ~60 ms.
    ///
    /// Each poll computes the average byte-frequency amplitude for every peer
    /// and fires `on_speaking_change` with the set of peers above threshold.
    pub fn start_polling(&mut self) {
        if self.interval_id.is_some() {
            return; // already running
        }

        let analysers = self.analysers.clone();
        let on_change = self.on_speaking_change.clone();
        let prev_speaking: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));

        let closure = Closure::wrap(Box::new(move || {
            let map = analysers.borrow();
            let mut speaking = HashSet::new();
            for (peer_id, analyser) in map.iter() {
                let buf_len = analyser.frequency_bin_count() as usize;
                if buf_len == 0 {
                    continue;
                }
                let mut data = vec![0u8; buf_len];
                analyser.get_byte_frequency_data(&mut data);
                let avg: f64 = data.iter().map(|&b| b as f64).sum::<f64>() / buf_len as f64;
                if avg > 25.0 {
                    speaking.insert(peer_id.clone());
                }
            }
            // Only fire the callback when the speaking set actually changes.
            let mut prev = prev_speaking.borrow_mut();
            if *prev != speaking {
                *prev = speaking.clone();
                on_change(speaking);
            }
        }) as Box<dyn FnMut()>);

        let window = web_sys::window().unwrap();
        let id = window
            .set_interval_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                60,
            )
            .unwrap();
        // Intentional leak: the closure must outlive the interval.
        closure.forget();

        self.interval_id = Some(id);
    }

    /// Stop polling and release resources.
    pub fn destroy(&mut self) {
        if let Some(id) = self.interval_id.take() {
            if let Some(window) = web_sys::window() {
                window.clear_interval_with_handle(id);
            }
        }
        let _ = self.audio_context.close();
        self.analysers.borrow_mut().clear();
        self.sources.borrow_mut().clear();
    }
}

/// Callback for sending voice signaling data back through the network.
///
/// Arguments: `(target_peer_id, signal_type, payload)`
/// where `signal_type` is `"offer"`, `"answer"`, or `"ice"`.
type SignalCallback = Rc<dyn Fn(&str, &str, &str)>;

/// Callback invoked when a remote peer's video track arrives or ends.
///
/// Arguments: `(remote_peer_id, Option<MediaStream>)` — `Some` when a video
/// track arrives, `None` when it ends.
type VideoTrackCallback = Rc<dyn Fn(&str, Option<MediaStream>)>;

/// Per-peer connection state, including the RTCPeerConnection and a shared
/// flag used by the perfect negotiation protocol to detect offer collisions.
struct PeerConnectionState {
    /// The underlying WebRTC peer connection.
    pc: RtcPeerConnection,
    /// Shared flag: `true` while we are in the process of creating and sending
    /// an offer. Shared with the `onnegotiationneeded` closure via `Rc<Cell>`.
    making_offer: Rc<Cell<bool>>,
}

/// Manages WebRTC connections for voice chat.
///
/// Each remote peer has a dedicated `RTCPeerConnection`. The local microphone
/// stream is shared across all connections. Video tracks (camera or screen
/// share) are added/removed via `start_video` / `stop_video_share`, which
/// trigger renegotiation automatically through the `onnegotiationneeded`
/// handler.
pub struct VoiceManager {
    /// Our own peer ID, used for polite/impolite determination.
    local_peer_id: String,
    /// One `PeerConnectionState` per remote peer.
    connections: HashMap<String, PeerConnectionState>,
    /// Local microphone stream (acquired once).
    local_stream: Option<MediaStream>,
    /// Callback to send signaling data: `(target_peer, signal_type, payload)`.
    on_signal: SignalCallback,
    /// Callback invoked when a remote video track arrives or ends.
    on_video_track: VideoTrackCallback,
    /// Active video stream (camera or screen share).
    video_stream: Option<MediaStream>,
    /// Which video source is currently active.
    video_source: Option<VideoSource>,
    /// RTP senders for the video track, keyed by remote peer ID.
    /// Stored so we can call `remove_track` later.
    video_senders: HashMap<String, RtcRtpSender>,
    /// Audio volume analyser for speaking detection.
    speaking_detector: Option<SpeakingDetector>,
    /// Stored so `close_all()` can recreate the `SpeakingDetector` for the next session.
    speaking_change_cb: Option<Rc<dyn Fn(HashSet<String>)>>,
}

impl VoiceManager {
    /// Create a new `VoiceManager` with signaling, video track, and speaking callbacks.
    ///
    /// `local_peer_id` is this node's peer ID string, used to determine
    /// polite vs impolite role during offer collisions.
    ///
    /// `on_signal` is invoked with `(target_peer_id, signal_type, payload)`
    /// whenever a signaling message needs to be sent to a remote peer.
    ///
    /// `on_video_track` is invoked with `(remote_peer_id, Some(stream))` when
    /// a remote video track arrives, and `(remote_peer_id, None)` when it ends.
    ///
    /// `on_speaking_change` is invoked with the set of currently-speaking peer
    /// IDs every ~60 ms while audio is active.
    pub fn new(
        local_peer_id: String,
        on_signal: impl Fn(&str, &str, &str) + 'static,
        on_video_track: impl Fn(&str, Option<MediaStream>) + 'static,
        on_speaking_change: impl Fn(HashSet<String>) + 'static,
    ) -> Self {
        // Store callback so close_all() can recreate the detector for future sessions.
        let speaking_cb: Rc<dyn Fn(HashSet<String>)> = Rc::new(on_speaking_change);
        // Create the detector eagerly — the caller is inside a user-gesture
        // handler (voice join click) so AudioContext creation is allowed.
        let cb_clone = speaking_cb.clone();
        let mut detector = SpeakingDetector::new(move |s| cb_clone(s)).ok();
        if let Some(ref mut d) = detector {
            d.start_polling();
        }
        Self {
            local_peer_id,
            connections: HashMap::new(),
            local_stream: None,
            on_signal: Rc::new(on_signal),
            on_video_track: Rc::new(on_video_track),
            video_stream: None,
            video_source: None,
            video_senders: HashMap::new(),
            speaking_detector: detector,
            speaking_change_cb: Some(speaking_cb),
        }
    }

    /// Set the local microphone stream (acquired externally to avoid RefCell across await).
    ///
    /// Also adds the stream to the speaking detector so the local user's
    /// volume is analysed alongside remote peers.
    pub fn set_local_stream(&mut self, stream: MediaStream) {
        if let Some(ref mut detector) = self.speaking_detector {
            detector.add_stream(&self.local_peer_id, &stream);
        }
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

    /// Add local audio tracks (and video track if sharing) to a peer connection.
    ///
    /// Returns `Some(RtcRtpSender)` if a video track was added, so the caller
    /// can store it in `video_senders` for later removal.
    fn add_local_tracks(&self, pc: &RtcPeerConnection) -> Option<RtcRtpSender> {
        // Audio tracks.
        if let Some(ref stream) = self.local_stream {
            let tracks = stream.get_audio_tracks();
            for i in 0..tracks.length() {
                let track: web_sys::MediaStreamTrack = tracks.get(i).unchecked_into();
                pc.add_track_0(&track, stream);
            }
        }
        // Video track if currently sharing.
        if let Some(ref video_stream) = self.video_stream {
            let tracks = video_stream.get_video_tracks();
            if tracks.length() > 0 {
                let track: web_sys::MediaStreamTrack = tracks.get(0).unchecked_into();
                return Some(pc.add_track_0(&track, video_stream));
            }
        }
        None
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

    /// Set up the `ontrack` handler to play remote audio and forward video.
    ///
    /// Audio tracks create `<audio>` elements appended to the document body
    /// and are added to the speaking detector for volume analysis.
    /// Video tracks are forwarded to the `on_video_track` callback with a
    /// listener for `ended` that fires `on_video_track(peer_id, None)`.
    fn setup_track_handler(&self, pc: &RtcPeerConnection, remote_peer: &str) {
        let peer_id = remote_peer.to_string();
        let on_video = self.on_video_track.clone();

        // Share the detector's analysers map, sources map, and audio context
        // with the closure so it can register incoming remote audio streams
        // for speaking detection.
        let detector_analysers = self.speaking_detector.as_ref().map(|d| d.analysers.clone());
        let detector_sources = self.speaking_detector.as_ref().map(|d| d.sources.clone());
        let detector_ctx = self
            .speaking_detector
            .as_ref()
            .map(|d| d.audio_context.clone());

        let on_track = Closure::wrap(Box::new(move |ev: RtcTrackEvent| {
            let track: web_sys::MediaStreamTrack = ev.track();
            let streams = ev.streams();
            // Renegotiated tracks may arrive without an associated stream
            // (empty streams array when SDP lacks a=msid). Create one from
            // the track so video still works.
            let stream: MediaStream = if streams.length() > 0 {
                streams.get(0).unchecked_into()
            } else {
                let s = MediaStream::new().unwrap();
                s.add_track(&track);
                s
            };

            if track.kind() == "audio" {
                // Register the remote audio stream with the speaking detector.
                if let (Some(ref ctx), Some(ref analysers), Some(ref sources)) =
                    (&detector_ctx, &detector_analysers, &detector_sources)
                {
                    if let Ok(source) = ctx.create_media_stream_source(&stream) {
                        if let Ok(analyser) = ctx.create_analyser() {
                            analyser.set_fft_size(256);
                            if source.connect_with_audio_node(&analyser).is_ok() {
                                analysers.borrow_mut().insert(peer_id.clone(), analyser);
                                // Store source node in the shared map so it stays
                                // alive and is properly cleaned up by remove_peer().
                                sources.borrow_mut().insert(peer_id.clone(), source);
                            }
                        }
                    }
                }

                // Create <audio> element for remote audio playback.
                // Uses a peer-id-based DOM id so close_connection can remove it.
                if let Some(window) = web_sys::window() {
                    if let Some(document) = window.document() {
                        // Remove any existing audio element for this peer first.
                        let audio_id = format!("willow-audio-{}", peer_id);
                        if let Some(existing) = document.get_element_by_id(&audio_id) {
                            existing.remove();
                        }
                        if let Ok(el) = document.create_element("audio") {
                            el.set_id(&audio_id);
                            let audio: web_sys::HtmlMediaElement = el.unchecked_into();
                            audio.set_src_object(Some(&stream));
                            audio.set_autoplay(true);
                            let _ = audio.play();
                            if let Some(body) = document.body() {
                                let _ = body.append_child(&audio);
                            }
                        }
                    }
                }
            } else if track.kind() == "video" {
                let pid = peer_id.clone();
                on_video(&pid, Some(stream));

                // Listen for track ended to clear the video.
                let pid_end = peer_id.clone();
                let on_video_end = on_video.clone();
                let on_ended = Closure::once(move || {
                    on_video_end(&pid_end, None);
                });
                track.set_onended(Some(on_ended.as_ref().unchecked_ref()));
                on_ended.forget();
            }
        }) as Box<dyn FnMut(RtcTrackEvent)>);

        pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));
        // Intentional leak: closure must outlive the peer connection.
        on_track.forget();
    }

    /// Set up the `onnegotiationneeded` handler for automatic renegotiation.
    ///
    /// When tracks are added or removed, the browser fires this event. The
    /// handler creates a new offer, sets the local description, and sends
    /// it via the signal callback. The `making_offer` flag is set during
    /// this process for the perfect negotiation collision detection.
    fn setup_negotiation_handler(
        &self,
        pc: &RtcPeerConnection,
        remote_peer: &str,
        making_offer: Rc<Cell<bool>>,
    ) {
        let signal_cb = self.on_signal.clone();
        let peer_id = remote_peer.to_string();
        let pc_clone = pc.clone();

        let on_negotiation = Closure::wrap(Box::new(move || {
            let signal = signal_cb.clone();
            let pid = peer_id.clone();
            let pc_inner = pc_clone.clone();
            let flag = making_offer.clone();

            wasm_bindgen_futures::spawn_local(async move {
                // Guard: skip if another offer is already in progress
                // (e.g., create_offer set making_offer before this fires).
                if flag.get() {
                    return;
                }
                flag.set(true);

                let offer_result =
                    wasm_bindgen_futures::JsFuture::from(pc_inner.create_offer()).await;
                let Ok(offer) = offer_result else {
                    flag.set(false);
                    return;
                };

                let offer_sdp = js_sys::Reflect::get(&offer, &"sdp".into())
                    .unwrap_or_default()
                    .as_string()
                    .unwrap_or_default();

                let desc = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
                desc.set_sdp(&offer_sdp);
                let set_result =
                    wasm_bindgen_futures::JsFuture::from(pc_inner.set_local_description(&desc))
                        .await;
                if set_result.is_err() {
                    flag.set(false);
                    return;
                }

                signal(&pid, "offer", &offer_sdp);
                flag.set(false);
            });
        }) as Box<dyn FnMut()>);

        pc.set_onnegotiationneeded(Some(on_negotiation.as_ref().unchecked_ref()));
        // Intentional leak: closure must outlive the peer connection.
        on_negotiation.forget();
    }

    /// Get an existing connection or create a new one with all handlers wired up.
    ///
    /// If a connection already exists for `remote_peer`, it is returned as-is.
    /// Otherwise a new `RTCPeerConnection` is created, local tracks are added,
    /// and ICE / track / negotiation handlers are installed.
    ///
    /// Returns a reference to the `PeerConnectionState` and an optional
    /// `RtcRtpSender` for the video track (if one was added to a new connection).
    fn get_or_create_connection(
        &mut self,
        remote_peer: &str,
    ) -> Result<(&PeerConnectionState, Option<RtcRtpSender>), String> {
        if self.connections.contains_key(remote_peer) {
            let state = self.connections.get(remote_peer).unwrap();
            return Ok((state, None));
        }

        let pc = RtcPeerConnection::new_with_configuration(&Self::rtc_config())
            .map_err(|_| "failed to create peer connection")?;

        let video_sender = self.add_local_tracks(&pc);
        self.setup_ice_handler(&pc, remote_peer);
        self.setup_track_handler(&pc, remote_peer);

        let making_offer = Rc::new(Cell::new(false));
        self.setup_negotiation_handler(&pc, remote_peer, making_offer.clone());

        self.connections.insert(
            remote_peer.to_string(),
            PeerConnectionState { pc, making_offer },
        );

        let state = self.connections.get(remote_peer).unwrap();
        Ok((state, video_sender))
    }

    /// Create an SDP offer and send it to a remote peer.
    ///
    /// If a connection already exists it is reused; otherwise a new one is
    /// created with local tracks and all handlers. The offer is created on
    /// the (possibly existing) connection and sent via the signal callback.
    pub async fn create_offer(&mut self, remote_peer: &str) -> Result<(), String> {
        let (state, video_sender) = self.get_or_create_connection(remote_peer)?;
        let pc = state.pc.clone();
        let making_offer = state.making_offer.clone();

        // Prevent the onnegotiationneeded handler from firing a duplicate offer
        // while we are creating one here.
        making_offer.set(true);

        // Store video sender if a new connection was created with video.
        if let Some(sender) = video_sender {
            self.video_senders.insert(remote_peer.to_string(), sender);
        }

        // Create offer.
        let offer = match wasm_bindgen_futures::JsFuture::from(pc.create_offer()).await {
            Ok(o) => o,
            Err(_) => {
                making_offer.set(false);
                return Err("create_offer failed".to_string());
            }
        };

        let offer_sdp = js_sys::Reflect::get(&offer, &"sdp".into())
            .unwrap_or_default()
            .as_string()
            .unwrap_or_default();

        // Set local description.
        let desc = RtcSessionDescriptionInit::new(RtcSdpType::Offer);
        desc.set_sdp(&offer_sdp);
        if wasm_bindgen_futures::JsFuture::from(pc.set_local_description(&desc))
            .await
            .is_err()
        {
            making_offer.set(false);
            return Err("set_local_description failed".to_string());
        }

        // Send offer to remote peer.
        (self.on_signal)(remote_peer, "offer", &offer_sdp);
        making_offer.set(false);

        Ok(())
    }

    /// Handle an incoming SDP offer from a remote peer.
    ///
    /// Implements the "perfect negotiation" pattern:
    /// - If we are the **impolite** peer (higher ID) and are currently making
    ///   an offer, we ignore the incoming offer (our offer wins).
    /// - If we are the **polite** peer (lower ID) and are making an offer,
    ///   we rollback our local description and accept the remote offer.
    /// - Otherwise we accept the offer normally.
    pub async fn handle_offer(&mut self, remote_peer: &str, sdp: &str) -> Result<(), String> {
        let (state, video_sender) = self.get_or_create_connection(remote_peer)?;
        let pc = state.pc.clone();
        let currently_making_offer = state.making_offer.get();

        // Store video sender if a new connection was created with video.
        if let Some(sender) = video_sender {
            self.video_senders.insert(remote_peer.to_string(), sender);
        }

        // Perfect negotiation collision detection.
        let polite = self.local_peer_id.as_str() < remote_peer;

        if currently_making_offer {
            if !polite {
                // We are impolite and already making an offer — ignore incoming.
                return Ok(());
            }
            // We are polite — rollback our pending local description.
            let rollback = RtcSessionDescriptionInit::new(RtcSdpType::Rollback);
            let _ = wasm_bindgen_futures::JsFuture::from(pc.set_local_description(&rollback)).await;
        }

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

        Ok(())
    }

    /// Handle an incoming SDP answer from a remote peer.
    pub async fn handle_answer(&self, remote_peer: &str, sdp: &str) -> Result<(), String> {
        let state = self
            .connections
            .get(remote_peer)
            .ok_or("no connection for peer")?;

        let desc = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
        desc.set_sdp(sdp);
        wasm_bindgen_futures::JsFuture::from(state.pc.set_remote_description(&desc))
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
        let state = self
            .connections
            .get(remote_peer)
            .ok_or("no connection for peer")?;

        let candidate_obj =
            js_sys::JSON::parse(candidate_json).map_err(|_| "invalid ICE candidate JSON")?;
        // Use add_ice_candidate with the parsed JS object directly.
        // The browser accepts RTCIceCandidateInit dictionaries natively.
        let _ = state
            .pc
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

    /// Start sharing video (camera or screen) to all connected peers.
    ///
    /// Stops any existing video share first. The video track is added to every
    /// existing peer connection; `onnegotiationneeded` fires automatically and
    /// handles the renegotiation.
    pub fn start_video(&mut self, stream: MediaStream, source: VideoSource) {
        self.stop_video_share();
        self.video_stream = Some(stream.clone());
        self.video_source = Some(source);

        let video_tracks = stream.get_video_tracks();
        if video_tracks.length() > 0 {
            let track: web_sys::MediaStreamTrack = video_tracks.get(0).unchecked_into();
            // Collect peer IDs first to avoid borrowing `self` in the loop.
            let peer_ids: Vec<String> = self.connections.keys().cloned().collect();
            for peer_id in peer_ids {
                if let Some(state) = self.connections.get(&peer_id) {
                    let sender = state.pc.add_track_0(&track, &stream);
                    self.video_senders.insert(peer_id, sender);
                }
            }
        }
        // onnegotiationneeded fires automatically from addTrack.
    }

    /// Stop sharing video and remove the track from all peer connections.
    ///
    /// Stops the underlying `MediaStreamTrack` (turns off camera LED) and
    /// removes the RTP sender from each connection, triggering renegotiation.
    pub fn stop_video_share(&mut self) {
        let senders: Vec<(String, RtcRtpSender)> = self.video_senders.drain().collect();
        for (peer_id, sender) in senders {
            if let Some(state) = self.connections.get(&peer_id) {
                state.pc.remove_track(&sender);
            }
        }
        if let Some(ref stream) = self.video_stream {
            let tracks = stream.get_video_tracks();
            for i in 0..tracks.length() {
                let track: web_sys::MediaStreamTrack = tracks.get(i).unchecked_into();
                track.stop();
            }
        }
        self.video_stream = None;
        self.video_source = None;
    }

    /// Start sharing the screen. Convenience wrapper around `start_video`.
    pub fn start_screen_share(&mut self, stream: MediaStream) {
        self.start_video(stream, VideoSource::Screen);
    }

    /// Start sharing the camera. Convenience wrapper around `start_video`.
    pub fn start_camera(&mut self, stream: MediaStream) {
        self.start_video(stream, VideoSource::Camera);
    }

    /// Return the currently active video source, if any.
    pub fn video_source(&self) -> Option<VideoSource> {
        self.video_source
    }

    /// Close the connection to a specific remote peer.
    ///
    /// Also removes the `<audio>` element created for this peer's remote
    /// audio playback so elements do not accumulate across reconnects.
    pub fn close_connection(&mut self, remote_peer: &str) {
        self.video_senders.remove(remote_peer);
        if let Some(ref mut detector) = self.speaking_detector {
            detector.remove_peer(remote_peer);
        }
        (self.on_video_track)(remote_peer, None);
        if let Some(state) = self.connections.remove(remote_peer) {
            state.pc.close();
        }
        // Remove the <audio> element from the DOM.
        if let Some(window) = web_sys::window() {
            if let Some(document) = window.document() {
                let audio_id = format!("willow-audio-{}", remote_peer);
                if let Some(el) = document.get_element_by_id(&audio_id) {
                    el.remove();
                }
            }
        }
    }

    /// Close all connections, stop video sharing, and release the microphone.
    ///
    /// Each peer connection is closed via [`close_connection`] so that
    /// `<audio>` elements are removed from the DOM and the speaking detector
    /// is cleaned up per-peer. The detector is then destroyed and recreated
    /// so speaking detection works on the next voice session.
    pub fn close_all(&mut self) {
        self.stop_video_share();
        // Close each peer individually to remove <audio> elements and clean
        // up per-peer detector state.
        let peers: Vec<String> = self.connections.keys().cloned().collect();
        for peer in peers {
            self.close_connection(&peer);
        }
        // Destroy the detector after per-peer cleanup.
        if let Some(ref mut detector) = self.speaking_detector {
            detector.destroy();
        }
        self.speaking_detector = None;
        // Recreate the detector so speaking detection works on rejoin.
        if let Some(ref cb) = self.speaking_change_cb {
            let cb_clone = cb.clone();
            self.speaking_detector = SpeakingDetector::new(move |s| cb_clone(s)).ok();
            if let Some(ref mut d) = self.speaking_detector {
                d.start_polling();
            }
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
