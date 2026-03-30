# Video, Screen Sharing + Call Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a full call page with camera video, screen sharing, speaking detection, and grid/focus layout — turning Willow's minimal voice chat into a complete video calling experience.

**Architecture:** Refactor VoiceManager for connection reuse + perfect negotiation, add unified video track management (camera/screen mutually exclusive), build call page UI with participant tiles, and add AudioContext-based speaking detection. All video/screen/camera streams flow through the existing WebRTC peer connections and gossipsub signaling.

**Tech Stack:** Rust, Leptos 0.7 (CSR), web_sys WebRTC APIs (RTCPeerConnection, getDisplayMedia, getUserMedia, AudioContext, AnalyserNode), CSS Grid

**Spec:** `docs/specs/2026-03-26-screen-sharing-call-page-design.md`

---

## File Map

### New files

| File | Responsibility |
|------|----------------|
| `crates/web/src/components/call_page.rs` | Call page layout: top bar (channel name, participant count, timer), grid/focus participant area, control strip (mute, deafen, camera, screen share, disconnect) |
| `crates/web/src/components/participant_tile.rs` | Individual tile: peer-ID-derived gradient avatar, video element, display name overlay, speaking glow, muted badge |

### Modified files

| File | Changes |
|------|---------|
| `crates/web/src/voice.rs` | **Major rewrite:** Connection reuse (no new PC on renegotiation), perfect negotiation (polite/impolite), unified video track management (`start_video`/`stop_video_share`), `ontrack` redesign (audio vs video routing), `SpeakingDetector`, new callbacks (`on_video_track`, `on_speaking_change`). ~320 lines → ~600 lines. |
| `crates/web/src/state.rs` | Add `CallLayout` enum, `VideoSource` enum, new signals: `show_call_page`, `call_layout`, `video_source`, `speaking_peers`, `remote_video_streams` |
| `crates/web/src/app.rs` | Call page in main view routing, voice channel click shows call page, wire VoiceManager callbacks to signals, camera/screen share click handlers |
| `crates/web/src/components/sidebar.rs` | Voice channel click opens call page instead of just joining |
| `crates/web/src/components/mod.rs` | Register `call_page` and `participant_tile` modules |
| `crates/web/src/event_processing.rs` | Clean up `remote_video_streams` on `VoiceLeft` |
| `crates/web/src/icons.rs` | Add `icon_monitor`, `icon_video`, `icon_video_off`, `icon_grid`, `icon_maximize` |
| `crates/web/style.css` | Call page, grid/focus layouts, tiles, controls, speaking glow, avatar gradients |
| `crates/web/Cargo.toml` | Add web-sys features: `DisplayMediaStreamConstraints`, `AudioContext`, `BaseAudioContext`, `AudioNode`, `AnalyserNode`, `MediaStreamAudioSourceNode`, `RtcSignalingState`, `RtcRtpTransceiver` |

---

## Task Ordering

The tasks build up in layers — each produces a compiling state:

1. **Task 1:** State additions + web-sys features + icons (foundation)
2. **Task 2:** VoiceManager refactor — connection reuse + perfect negotiation (no new features yet, just refactor)
3. **Task 3:** Video track management in VoiceManager (camera + screen share infrastructure)
4. **Task 4:** Participant tile component (standalone, renders avatar or video)
5. **Task 5:** Call page component + routing (brings it all together in the UI)
6. **Task 6:** Speaking detection
7. **Task 7:** CSS polish + full verification

---

### Task 1: State additions, web-sys features, icons

**Files:**
- Modify: `crates/web/src/state.rs`
- Modify: `crates/web/src/icons.rs`
- Modify: `crates/web/Cargo.toml`

- [ ] **Step 1: Add enums and signals to state.rs**

Add after the `SettingsTab` enum:

```rust
#[derive(Clone, Copy, PartialEq)]
pub enum VideoSource {
    Camera,
    Screen,
}

#[derive(Clone, PartialEq, Default)]
pub enum CallLayout {
    #[default]
    Grid,
    Focus(String), // focused peer_id
}
```

Add to `UiState`:
```rust
pub show_call_page: ReadSignal<bool>,
pub call_layout: ReadSignal<CallLayout>,
```

Add to `UiWriteSignals`:
```rust
pub set_show_call_page: WriteSignal<bool>,
pub set_call_layout: WriteSignal<CallLayout>,
```

Add to `VoiceState`:
```rust
pub video_source: ReadSignal<Option<VideoSource>>,
pub speaking_peers: ReadSignal<std::collections::HashSet<String>>,
pub remote_video_streams: ReadSignal<std::collections::HashMap<String, send_wrapper::SendWrapper<web_sys::MediaStream>>>,
```

Add to `VoiceWriteSignals`:
```rust
pub set_video_source: WriteSignal<Option<VideoSource>>,
pub set_speaking_peers: WriteSignal<std::collections::HashSet<String>>,
pub set_remote_video_streams: WriteSignal<std::collections::HashMap<String, send_wrapper::SendWrapper<web_sys::MediaStream>>>,
```

Update `create_signals()` to create these signal pairs and wire them into the structs.

- [ ] **Step 2: Add web-sys features to Cargo.toml**

Add to the web-sys features list:
```toml
"DisplayMediaStreamConstraints",
"AudioContext", "BaseAudioContext", "AudioNode",
"AnalyserNode", "MediaStreamAudioSourceNode",
"RtcSignalingState", "RtcRtpTransceiver",
```

- [ ] **Step 3: Add icons**

Add to `crates/web/src/icons.rs`:
- `icon_monitor()` — screen/monitor SVG (for screen share)
- `icon_video()` — video camera SVG
- `icon_video_off()` — video camera with slash
- `icon_grid()` — grid layout SVG (2x2 squares)
- `icon_maximize()` — maximize/focus SVG

Use Lucide-style SVG paths (24x24, stroke-based, currentColor).

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p willow-web && just check-wasm`

- [ ] **Step 5: Commit**

```bash
git add crates/web/
git commit -m "feat: add video/call page state, web-sys features, and icons"
```

---

### Task 2: VoiceManager refactor — connection reuse + perfect negotiation

**Files:**
- Modify: `crates/web/src/voice.rs`

This is the critical infrastructure task. Refactor without adding new features — existing voice chat must continue working.

- [ ] **Step 1: Add per-connection state struct**

Add a `PeerConnectionState` struct to track connection metadata:

```rust
struct PeerConnectionState {
    pc: RtcPeerConnection,
    making_offer: bool,
}
```

Change `connections: HashMap<String, RtcPeerConnection>` to `connections: HashMap<String, PeerConnectionState>`.

Add a `local_peer_id: String` field to `VoiceManager` (needed for polite/impolite determination). Pass it in the constructor.

- [ ] **Step 2: Refactor create_offer for connection reuse**

`create_offer` should check if a connection already exists:
- If exists: reuse the existing PC (call `pc.create_offer()` on it, don't create a new one)
- If not: create a new PC, add local tracks, set up handlers, THEN create offer

The handlers (`onicecandidate`, `ontrack`) should only be set up once per connection (when the PC is first created).

Extract a `get_or_create_connection(peer_id) -> &PeerConnectionState` helper.

- [ ] **Step 3: Refactor handle_offer for connection reuse**

Same pattern — check for existing connection:
- If exists: set remote description on existing PC, create answer
- If not: create new PC, add tracks, set up handlers, then set remote description and create answer

- [ ] **Step 4: Add onnegotiationneeded handler**

When creating a new PC, set up `onnegotiationneeded`:

```rust
let on_negotiation = Closure::wrap(Box::new(move || {
    // Set making_offer = true
    // Create offer, set local description, send via signal callback
    // Set making_offer = false
}) as Box<dyn FnMut()>);
pc.set_onnegotiationneeded(Some(on_negotiation.as_ref().unchecked_ref()));
on_negotiation.forget();
```

This fires automatically when `addTrack` or `removeTrack` is called.

**Note:** The `onnegotiationneeded` handler needs to be async (creates offer, sets description). Since it's a JS closure, use `wasm_bindgen_futures::spawn_local` inside it.

- [ ] **Step 5: Add perfect negotiation collision handling to handle_offer**

When receiving an offer while `making_offer` is true:
- Determine polite/impolite: compare `self.local_peer_id` with `remote_peer` lexicographically. Lower ID = polite.
- If impolite and making_offer: ignore the incoming offer (return early)
- If polite: rollback local description, then set remote description from incoming offer, create answer

```rust
let am_polite = self.local_peer_id < remote_peer;
if let Some(state) = self.connections.get(remote_peer) {
    if !am_polite && state.making_offer {
        return Ok(()); // Ignore collision
    }
    // If polite, rollback:
    if am_polite && state.making_offer {
        let rollback = RtcSessionDescriptionInit::new(RtcSdpType::Rollback);
        JsFuture::from(state.pc.set_local_description(&rollback)).await.ok();
    }
}
```

- [ ] **Step 6: Update all connection access sites**

Update `handle_answer`, `handle_ice_candidate`, `close_connection`, `close_all`, `set_muted` to access `state.pc` instead of the raw `RtcPeerConnection`.

- [ ] **Step 7: Verify existing voice still works**

Run: `cargo check -p willow-web && just check-wasm`

The refactored VoiceManager should be API-compatible with existing usage in `app.rs` and `event_processing.rs`.

- [ ] **Step 8: Commit**

```bash
git add crates/web/src/voice.rs
git commit -m "refactor: VoiceManager connection reuse + perfect negotiation

Connections are reused on renegotiation instead of recreated.
Adds onnegotiationneeded handler for addTrack/removeTrack flows.
Implements polite/impolite pattern for offer collision handling."
```

---

### Task 3: Video track management (camera + screen share)

**Files:**
- Modify: `crates/web/src/voice.rs`

- [ ] **Step 1: Add video state and callbacks to VoiceManager**

Add fields:
```rust
video_stream: Option<MediaStream>,
video_source: Option<VideoSource>,
video_senders: HashMap<String, RtcRtpSender>,
on_video_track: Rc<dyn Fn(&str, Option<MediaStream>)>,
```

Update the constructor to accept `on_video_track` callback.

- [ ] **Step 2: Redesign ontrack handler**

Replace the current `setup_track_handler` (which creates `<audio>` elements) with one that distinguishes audio from video:

```rust
fn setup_track_handler(&self, pc: &RtcPeerConnection, remote_peer: &str) {
    let peer_id = remote_peer.to_string();
    let on_video = self.on_video_track.clone();
    let on_track = Closure::wrap(Box::new(move |ev: RtcTrackEvent| {
        let track: MediaStreamTrack = ev.track();
        let streams = ev.streams();
        if streams.length() == 0 { return; }
        let stream: MediaStream = streams.get(0).unchecked_into();

        if track.kind() == "audio" {
            // Create <audio> element as before
            // ...
        } else if track.kind() == "video" {
            // Route to signal via callback
            let pid = peer_id.clone();
            on_video(&pid, Some(stream));

            // Listen for track ended
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
    on_track.forget();
}
```

- [ ] **Step 3: Implement start_video and stop_video_share**

```rust
pub fn start_video(&mut self, stream: MediaStream, source: VideoSource) {
    // Stop existing video if any
    self.stop_video_share();

    self.video_stream = Some(stream.clone());
    self.video_source = Some(source);

    // Add video track to all existing connections
    let video_tracks = stream.get_video_tracks();
    if video_tracks.length() > 0 {
        let track: MediaStreamTrack = video_tracks.get(0).unchecked_into();
        for (peer_id, state) in &self.connections {
            let sender = state.pc.add_track_0(&track, &stream);
            self.video_senders.insert(peer_id.clone(), sender);
        }
    }
    // onnegotiationneeded fires automatically
}

pub fn stop_video_share(&mut self) {
    // Remove track from all connections
    for (peer_id, sender) in self.video_senders.drain() {
        if let Some(state) = self.connections.get(&peer_id) {
            state.pc.remove_track(&sender);
        }
    }
    // Stop video tracks
    if let Some(ref stream) = self.video_stream {
        let tracks = stream.get_video_tracks();
        for i in 0..tracks.length() {
            let track: MediaStreamTrack = tracks.get(i).unchecked_into();
            track.stop();
        }
    }
    self.video_stream = None;
    self.video_source = None;
}
```

- [ ] **Step 4: Update add_local_tracks to include video if active**

When creating a new connection for a peer who joins mid-call, add both audio AND video tracks:

```rust
fn add_local_tracks(&self, pc: &RtcPeerConnection) -> Option<RtcRtpSender> {
    // Add audio (existing logic)
    if let Some(ref stream) = self.local_stream { ... }

    // Add video if sharing
    if let Some(ref video_stream) = self.video_stream {
        let tracks = video_stream.get_video_tracks();
        if tracks.length() > 0 {
            let track: MediaStreamTrack = tracks.get(0).unchecked_into();
            return Some(pc.add_track_0(&track, video_stream));
        }
    }
    None
}
```

Store the returned sender in `video_senders` for the new peer.

- [ ] **Step 5: Add convenience methods**

```rust
pub fn start_screen_share(&mut self, stream: MediaStream) {
    self.start_video(stream, VideoSource::Screen);
}

pub fn start_camera(&mut self, stream: MediaStream) {
    self.start_video(stream, VideoSource::Camera);
}

pub fn video_source(&self) -> Option<VideoSource> {
    self.video_source
}
```

- [ ] **Step 6: Update close_connection and close_all**

`close_connection`: also remove the peer's video sender from `video_senders` and fire `on_video_track(peer_id, None)`.

`close_all`: also call `stop_video_share()`.

- [ ] **Step 7: Verify compilation**

Run: `cargo check -p willow-web && just check-wasm`

- [ ] **Step 8: Commit**

```bash
git add crates/web/src/voice.rs
git commit -m "feat: unified video track management (camera + screen share)

Adds start_video/stop_video_share for mutually exclusive camera and
screen sharing. Redesigned ontrack handler routes video tracks via
callback. Video tracks included for peers joining mid-share."
```

---

### Task 4: Participant tile component

**Files:**
- Create: `crates/web/src/components/participant_tile.rs`
- Modify: `crates/web/src/components/mod.rs`
- Modify: `crates/web/style.css`

- [ ] **Step 1: Create participant_tile.rs**

A component that renders a single participant in the call:

```rust
#[component]
pub fn ParticipantTile(
    peer_id: String,
    display_name: String,
    #[prop(optional)] video_stream: Option<SendWrapper<MediaStream>>,
    #[prop(default = false)] is_speaking: bool,
    #[prop(default = false)] is_muted: bool,
    #[prop(default = false)] is_focused: bool,
    #[prop(optional)] on_click: Option<Callback<String>>,
) -> impl IntoView
```

The tile:
- If `video_stream` is Some: renders a `<video>` element with `srcObject` set to the stream
- If None: renders a gradient avatar (colors derived from peer_id hash) with initial letter
- Display name overlay at bottom
- Speaking glow when `is_speaking`
- Muted mic badge when `is_muted`
- Clicks fire `on_click(peer_id)` for focus toggling

For the avatar gradient:
```rust
fn peer_gradient(peer_id: &str) -> String {
    let hash1 = peer_id.bytes().take(peer_id.len() / 2).fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let hash2 = peer_id.bytes().skip(peer_id.len() / 2).fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let hue1 = hash1 % 360;
    let hue2 = (hue1 + 40 + hash2 % 60) % 360;
    format!("linear-gradient(135deg, hsl({hue1}, 45%, 35%), hsl({hue2}, 45%, 30%))")
}
```

For the video element, set `srcObject` via a Leptos `NodeRef` and `Effect`:
```rust
let video_ref = NodeRef::<leptos::html::Video>::new();
Effect::new(move |_| {
    if let Some(el) = video_ref.get() {
        if let Some(stream) = &video_stream_signal.get() {
            el.set_src_object(Some(&**stream));
            let _ = el.play();
        }
    }
});
```

- [ ] **Step 2: Add to mod.rs**

```rust
mod call_page;
mod participant_tile;
pub use call_page::*;
pub use participant_tile::*;
```

(Add `call_page` now too even though it's empty — avoids a separate mod.rs edit later.)

Create a placeholder `call_page.rs`:
```rust
use leptos::prelude::*;

#[component]
pub fn CallPage() -> impl IntoView {
    view! { <div class="call-page">"Call page placeholder"</div> }
}
```

- [ ] **Step 3: Add CSS for tiles**

Add to `style.css`:

```css
/* ── Participant Tile ───────────────────────────────────────────── */

.participant-tile {
    position: relative;
    border-radius: 16px;
    overflow: hidden;
    aspect-ratio: 16 / 9;
    cursor: pointer;
    transition: border-color 0.2s ease, box-shadow 0.2s ease, transform 0.3s cubic-bezier(0.4, 0, 0.2, 1);
    border: 2px solid rgba(255, 255, 255, 0.06);
    box-shadow: 0 4px 24px rgba(0, 0, 0, 0.4), inset 0 1px 0 rgba(255, 255, 255, 0.04);
    animation: tile-enter 0.4s cubic-bezier(0.2, 0, 0.2, 1) backwards;
}

/* Stagger tile entrance animations */
.participant-tile:nth-child(1) { animation-delay: 0s; }
.participant-tile:nth-child(2) { animation-delay: 0.06s; }
.participant-tile:nth-child(3) { animation-delay: 0.12s; }
.participant-tile:nth-child(4) { animation-delay: 0.18s; }
.participant-tile:nth-child(5) { animation-delay: 0.24s; }
.participant-tile:nth-child(6) { animation-delay: 0.3s; }

.participant-tile:hover {
    transform: scale(1.02);
    border-color: rgba(255, 255, 255, 0.12);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5), inset 0 1px 0 rgba(255, 255, 255, 0.06);
}

.participant-tile.speaking {
    border-color: var(--online);
    box-shadow: 0 0 0 3px var(--accent-green-glow), 0 0 16px var(--accent-green-glow), 0 4px 24px rgba(0, 0, 0, 0.4);
    animation: speaking-pulse 1.8s ease-in-out infinite;
}

.participant-tile video {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
    /* Subtle zoom to avoid black borders from webcam aspect ratios */
    transform: scale(1.01);
}

.participant-tile video.screen-share {
    object-fit: contain;
    background: #0a0a0c;
    transform: none;
}

/* Mirror local camera feed (feels natural like a mirror) */
.participant-tile.local-camera video { transform: scaleX(-1); }

.tile-avatar {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 48px;
    font-weight: 600;
    color: rgba(255, 255, 255, 0.85);
    text-shadow: 0 2px 12px rgba(0, 0, 0, 0.4);
    /* Subtle noise texture overlay for visual richness */
    position: relative;
}

.tile-avatar::after {
    content: '';
    position: absolute;
    inset: 0;
    background: radial-gradient(ellipse at 30% 20%, rgba(255,255,255,0.06) 0%, transparent 60%);
    pointer-events: none;
}

.tile-name {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    padding: 10px 14px;
    background: linear-gradient(transparent 0%, rgba(0, 0, 0, 0.5) 40%, rgba(0, 0, 0, 0.7) 100%);
    font-size: 13px;
    font-weight: 500;
    color: rgba(255, 255, 255, 0.95);
    letter-spacing: 0.01em;
    display: flex;
    align-items: center;
    gap: 6px;
}

/* Small "sharing screen" or "camera" indicator next to name */
.tile-name .tile-source-badge {
    font-size: 10px;
    background: rgba(255, 255, 255, 0.15);
    padding: 1px 6px;
    border-radius: 4px;
    color: rgba(255, 255, 255, 0.7);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}

.tile-muted-badge {
    position: absolute;
    bottom: 10px;
    right: 10px;
    width: 26px;
    height: 26px;
    background: rgba(237, 66, 69, 0.9);
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    color: white;
    backdrop-filter: blur(4px);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
}

@keyframes speaking-pulse {
    0%, 100% { box-shadow: 0 0 0 3px var(--accent-green-glow), 0 0 16px var(--accent-green-glow), 0 4px 24px rgba(0, 0, 0, 0.4); }
    50% { box-shadow: 0 0 0 5px var(--accent-green-glow), 0 0 24px var(--accent-green-glow), 0 4px 24px rgba(0, 0, 0, 0.4); }
}

@keyframes tile-enter {
    from { opacity: 0; transform: scale(0.92) translateY(8px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
}
```

- [ ] **Step 4: Verify and commit**

```bash
cargo check -p willow-web && just check-wasm
git add -A
git commit -m "feat: add ParticipantTile component with avatar gradients and speaking glow"
```

---

### Task 5: Call page component + routing

**Files:**
- Rewrite: `crates/web/src/components/call_page.rs`
- Modify: `crates/web/src/app.rs`
- Modify: `crates/web/src/components/sidebar.rs`
- Modify: `crates/web/src/event_processing.rs`
- Modify: `crates/web/style.css`

- [ ] **Step 1: Implement call_page.rs**

The call page has three sections:
1. **Top bar**: channel name, participant count pill, duration timer, grid/focus toggle
2. **Participant grid**: renders `<ParticipantTile>` for each voice participant
3. **Control strip**: Mute, Deafen, Camera, Screen Share, Disconnect

The component reads from context:
- `AppState` for voice state (participants, speaking, video streams, etc.)
- `WebClientHandle` for peer data (display names)
- `VoiceManagerHandle` for starting/stopping video

Props:
```rust
#[component]
pub fn CallPage(
    on_disconnect: Callback<()>,
    on_mute: Callback<()>,
    on_deafen: Callback<()>,
) -> impl IntoView
```

Camera and screen share buttons are handled locally:
- Camera: calls `getUserMedia({video:true})` synchronously in click, then `vm.start_camera(stream)`
- Screen: calls `getDisplayMedia()` synchronously in click, then `vm.start_screen_share(stream)`
- Both update `set_video_source` signal
- Both call `stop_video_share` on the other first (mutually exclusive)

Grid/focus: managed by local signal `call_layout`. Click tile → `Focus(peer_id)`. Click focused tile or grid button → `Grid`.

Duration timer: `set_interval` incrementing a seconds counter, formatted as `HH:MM:SS` with `font-variant-numeric: tabular-nums`.

- [ ] **Step 2: Update app.rs — add call page to view routing**

The main content area rendering priority becomes:
`show_add_server` → `show_settings` → `show_call_page` → chat

When `show_call_page` is true, render `<CallPage>` instead of the chat area.

Wire the VoiceManager's `on_video_track` callback to update `set_remote_video_streams` signal. This happens in the App component setup (similar to how `on_signal` is wired).

- [ ] **Step 3: Update sidebar.rs — voice channel click opens call page**

When a voice channel is clicked:
- If not in a voice channel: join voice (existing flow) AND set `show_call_page = true`
- If already in this voice channel: just set `show_call_page = true` (navigate back to call page)
- If in a different voice channel: leave current, join new, set `show_call_page = true`

When a text channel is clicked: set `show_call_page = false` (existing channel switch + close call page view, but call continues in background).

- [ ] **Step 4: Update event_processing.rs**

On `VoiceLeft` event: remove the peer from `remote_video_streams` signal.

On disconnect (local): set `show_call_page = false`, clear `video_source`, clear `remote_video_streams`.

- [ ] **Step 5: Add CSS for call page**

```css
/* ── Call Page ──────────────────────────────────────────────────── */

.call-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    /* Deep ambient gradient — signals "you're in a live session" */
    background:
        radial-gradient(ellipse at 50% 40%, rgba(88, 101, 242, 0.04) 0%, transparent 60%),
        radial-gradient(ellipse at center, #16161a 0%, #0e0e12 100%);
    animation: call-page-enter 0.3s ease;
}

@keyframes call-page-enter {
    from { opacity: 0; }
    to { opacity: 1; }
}

.call-top-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 20px;
    flex-shrink: 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
}

.call-channel-name {
    font-weight: 600;
    font-size: 15px;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    gap: 8px;
}

/* Subtle live dot next to channel name */
.call-live-dot {
    width: 8px;
    height: 8px;
    background: var(--online);
    border-radius: 50%;
    animation: live-dot-pulse 2s ease-in-out infinite;
}

@keyframes live-dot-pulse {
    0%, 100% { opacity: 1; box-shadow: 0 0 0 0 var(--accent-green-glow); }
    50% { opacity: 0.7; box-shadow: 0 0 0 4px var(--accent-green-glow); }
}

.call-participant-count {
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.06);
    border-radius: 12px;
    padding: 3px 12px;
    font-size: 12px;
    color: var(--text-secondary);
    font-weight: 500;
}

.call-timer {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    font-size: 13px;
    font-family: 'IBM Plex Mono', monospace;
    letter-spacing: 0.02em;
}

.call-layout-toggle {
    background: transparent;
    border: 1px solid rgba(255, 255, 255, 0.08);
    color: var(--text-muted);
    cursor: pointer;
    padding: 6px 8px;
    border-radius: 8px;
    font-size: 16px;
    display: flex;
    align-items: center;
    transition: all var(--transition-fast);
}
.call-layout-toggle:hover {
    color: var(--text-primary);
    border-color: rgba(255, 255, 255, 0.15);
    background: rgba(255, 255, 255, 0.04);
}

.call-grid {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
    gap: 12px;
    padding: 20px;
    align-content: center;
    overflow-y: auto;
}

/* Single participant: constrain width so it doesn't stretch full-screen */
.call-grid.single-participant {
    justify-content: center;
}
.call-grid.single-participant .participant-tile {
    max-width: 640px;
}

/* Two participants: equal side-by-side */
.call-grid.two-participants {
    grid-template-columns: 1fr 1fr;
}

/* Focus layout */
.call-grid.focus {
    display: flex;
    flex-direction: column;
    gap: 12px;
}

.call-grid.focus .participant-tile.focused {
    flex: 1;
    min-height: 0;
    aspect-ratio: auto;
}

.call-thumbnails {
    display: flex;
    gap: 8px;
    overflow-x: auto;
    padding: 4px 0;
    flex-shrink: 0;
    /* Subtle fade at edges for scroll indication */
    mask-image: linear-gradient(90deg, transparent 0%, black 16px, black calc(100% - 16px), transparent 100%);
}

.call-thumbnails .participant-tile {
    width: 160px;
    min-width: 160px;
    height: 100px;
    flex-shrink: 0;
    border-radius: 10px;
    aspect-ratio: auto;
}

/* Control strip — frosted glass floating bar */
.call-controls {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px;
    flex-shrink: 0;
}

.call-controls-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    background: rgba(0, 0, 0, 0.5);
    backdrop-filter: blur(16px);
    -webkit-backdrop-filter: blur(16px);
    border-radius: 20px;
    padding: 8px 12px;
    border: 1px solid rgba(255, 255, 255, 0.06);
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}

.call-btn {
    width: 48px;
    height: 48px;
    border-radius: 50%;
    border: none;
    background: rgba(255, 255, 255, 0.08);
    color: var(--text-primary);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 20px;
    transition: all 0.15s ease;
    position: relative;
}

.call-btn:hover {
    background: rgba(255, 255, 255, 0.14);
    transform: scale(1.06);
}

.call-btn:active { transform: scale(0.96); }

.call-btn.active {
    background: var(--accent);
    color: white;
    box-shadow: 0 0 12px var(--accent-glow);
}

.call-btn.muted {
    background: rgba(237, 66, 69, 0.2);
    color: var(--danger);
}
.call-btn.muted:hover { background: rgba(237, 66, 69, 0.3); }

/* Disconnect button — wider red pill */
.call-btn.disconnect {
    background: var(--danger);
    color: white;
    border-radius: 24px;
    width: auto;
    padding: 0 20px;
    gap: 6px;
    font-size: 14px;
    font-weight: 500;
}
.call-btn.disconnect:hover {
    background: var(--danger-hover);
    box-shadow: 0 0 16px var(--danger-glow);
}

/* Separator between control groups */
.call-controls-separator {
    width: 1px;
    height: 24px;
    background: rgba(255, 255, 255, 0.08);
    margin: 0 4px;
}

/* Tooltip on hover */
.call-btn[title]::after {
    content: attr(title);
    position: absolute;
    bottom: calc(100% + 8px);
    left: 50%;
    transform: translateX(-50%);
    background: var(--bg-elevated);
    color: var(--text-primary);
    font-size: 11px;
    padding: 4px 8px;
    border-radius: 6px;
    white-space: nowrap;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.15s ease;
    box-shadow: var(--shadow-md);
    border: 1px solid var(--border);
}
.call-btn[title]:hover::after { opacity: 1; }

@media (max-width: 900px) {
    .call-btn { width: 52px; height: 52px; }
    .call-btn[title]::after { display: none; } /* No tooltips on mobile */
    .call-grid { grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 8px; padding: 8px; }
    .call-controls-bar { padding: 6px 10px; }
    .call-top-bar { padding: 10px 12px; }
}
```

- [ ] **Step 6: Verify and commit**

```bash
cargo check -p willow-web && just check-wasm
git add -A
git commit -m "feat: call page with grid/focus layout, camera, screen share, and controls

Full call page replaces chat view when in voice channel.
Participant tiles show avatar gradients or video streams.
Control strip: mute, deafen, camera, screen share, disconnect.
Grid and focus layout modes with click-to-focus tiles."
```

---

### Task 6: Speaking detection

**Files:**
- Modify: `crates/web/src/voice.rs`

- [ ] **Step 1: Add SpeakingDetector struct**

```rust
pub struct SpeakingDetector {
    audio_context: web_sys::AudioContext,
    analysers: HashMap<String, web_sys::AnalyserNode>,
    on_speaking_change: Rc<dyn Fn(std::collections::HashSet<String>)>,
    interval_id: Option<i32>,
}
```

- [ ] **Step 2: Implement SpeakingDetector methods**

```rust
impl SpeakingDetector {
    pub fn new(on_change: impl Fn(HashSet<String>) + 'static) -> Result<Self, String> {
        let ctx = web_sys::AudioContext::new().map_err(|_| "AudioContext failed")?;
        Ok(Self {
            audio_context: ctx,
            analysers: HashMap::new(),
            on_speaking_change: Rc::new(on_change),
            interval_id: None,
        })
    }

    pub fn add_stream(&mut self, peer_id: &str, stream: &MediaStream) {
        // Create MediaStreamAudioSourceNode from stream
        // Connect to AnalyserNode
        // Store analyser
    }

    pub fn remove_peer(&mut self, peer_id: &str) {
        self.analysers.remove(peer_id);
    }

    pub fn start_polling(&mut self) {
        // setInterval at 60ms
        // Check each analyser's getByteFrequencyData
        // Build HashSet of speaking peers (volume > threshold)
        // Call on_speaking_change
    }

    pub fn destroy(&mut self) {
        if let Some(id) = self.interval_id.take() {
            web_sys::window().and_then(|w| w.clear_interval_with_handle(id).ok());
        }
        let _ = self.audio_context.close();
        self.analysers.clear();
    }
}
```

- [ ] **Step 3: Integrate SpeakingDetector into VoiceManager**

Add a `speaking_detector: Option<SpeakingDetector>` field.

In the `ontrack` handler, when an audio track arrives, also call `speaking_detector.add_stream()`.

In `close_connection`, call `speaking_detector.remove_peer()`.

In `close_all`, call `speaking_detector.destroy()`.

Initialize the detector in VoiceManager constructor (after the first peer connects, or lazily).

- [ ] **Step 4: Wire callback in app.rs**

The `on_speaking_change` callback updates `set_speaking_peers` signal. Wire this in the App component when creating the VoiceManager.

- [ ] **Step 5: Update ParticipantTile to use speaking state**

Read `speaking_peers` from `AppState` context. Check if the tile's peer_id is in the set.

- [ ] **Step 6: Verify and commit**

```bash
cargo check -p willow-web && just check-wasm
git add -A
git commit -m "feat: speaking detection with AudioContext analysers

SpeakingDetector polls AnalyserNodes at 60ms intervals.
Participant tiles show green pulse glow when peer is speaking.
Local mic also analysed for self-feedback."
```

---

### Task 7: CSS polish + full verification

- [ ] **Step 1: Run full checks**

```bash
just check   # fmt + clippy + test + wasm
```

Fix any warnings or errors.

- [ ] **Step 2: Build and deploy**

```bash
cd crates/web && trunk build --release && cd ../..
sshpass -p 'WillowP2P2026deploy!' scp -o StrictHostKeyChecking=no crates/web/dist/* root@172.234.217.219:/var/www/willow/
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'chmod 644 /var/www/willow/*'
```

- [ ] **Step 3: Manual testing**

Test with 2 browser windows on the deployed site:
- Join same voice channel from both
- Verify call page renders with participant tiles
- Test mute/deafen controls
- Test camera (if webcam available)
- Test screen share
- Test switching camera ↔ screen share
- Test grid/focus layout toggle
- Verify speaking indicators work
- Test disconnect

- [ ] **Step 4: Final commit**

```bash
git add -A
git commit -m "fix: polish and stabilize call page"
```
