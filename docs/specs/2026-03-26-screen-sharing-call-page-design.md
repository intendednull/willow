# Video, Screen Sharing + Call Page Design

**Date:** 2026-03-26
**Status:** landed — call page, participant tiles, camera, screen share, speaking detection, grid/focus layout, perfect-negotiation collision avoidance, video track management all shipped via [`docs/plans/2026-03-26-video-screen-sharing-call-page.md`](../plans/2026-03-26-video-screen-sharing-call-page.md). One substantive limitation remains in §Realised state below: remote video tiles cannot distinguish camera vs screen-share at render time (no source-type signaling yet).
**Implementation plan:** [`docs/plans/2026-03-26-video-screen-sharing-call-page.md`](../plans/2026-03-26-video-screen-sharing-call-page.md)

> **Realised state (post-2026-05 audit).** The system works as designed.
> The body below drifts from the realised implementation in several
> places, and one substantive limitation is tracked for follow-up:
>
> - **Source-type signaling for remote video (deferred).** Remote tiles
>   currently default to the `screen-share` CSS class regardless of
>   whether the inbound track is camera or screen share. This means
>   remote cameras get `object-fit: contain` (letterboxed) instead of
>   `object-fit: cover` (filled), which the spec's §Visual design
>   promises. The remote-source-type distinction requires extending
>   signaling to carry a `VideoSource` enum (camera | screen-share)
>   alongside the SDP. Tracked as follow-up; the file comment at
>   `crates/web/src/components/participant_tile.rs:110-118`
>   acknowledges this limitation in code.
> - **File path drift.** §Modify section references
>   `components/sidebar.rs`; the actual file is
>   `crates/web/src/components/channel_sidebar.rs`. The plan file is
>   already correct; the spec was not updated.
> - **`local_video_stream` signal added beyond spec.** `VoiceState`
>   in `crates/web/src/state.rs:314` carries
>   `local_video_stream: ReadSignal<Option<SendWrapper<MediaStream>>>`
>   to drive the local preview tile, because `VoiceManager` does not
>   expose the local stream synchronously to Leptos. The spec's §6
>   State Additions enumerates only `video_source`, `speaking_peers`,
>   `remote_video_streams`.
> - **Perfect negotiation simplified.** §4 implies
>   `RtcSignalingState`-driven collision detection. The realised code
>   relies only on the `making_offer` flag + `local_peer_id <
>   remote_peer` comparison; `signaling_state()` is never called.
>   `RtcSignalingState` was speculative; the realised approach is
>   sufficient for the current use case.
> - **`handle_offer` get-or-create.** §Camera handling at L58 says
>   "sets remote description on the *existing* peer connection."
>   `crates/web/src/voice.rs:611-619`'s `handle_offer` calls
>   `get_or_create_connection`, which DOES create a new PC if none
>   exists — necessary because remote-initiated calls may arrive before
>   any local outbound state exists. Same applies to the analogous
>   `create_offer` claim. The realised behaviour (reuse if exists,
>   create otherwise) is correct in both directions.
> - **Speaking-tile glow values tuned.** §Visual design at L174 and
>   the keyframes at L294-297 list `0 0 8px` / `0 0 12px → 0 0 20px`.
>   `crates/web/style.css:3671` uses
>   `0 0 0 3px <halo>, 0 0 16px <halo>, 0 4px 24px rgba(0,0,0,0.4)`
>   plus keyframes `0 0 16px → 0 0 24px` (halo ring 3px → 5px). Values
>   were tuned during implementation; treat the spec numbers as
>   indicative.
> - **Trust badge + presence dot on participant tiles** (not mentioned
>   in spec). `crates/web/src/components/participant_tile.rs:166-194`
>   renders `<TrustBadge … context=TileCorner/>` in the top-left and
>   `<StatusDot state=presence …/>` in the bottom-right. Both are
>   legitimate cross-spec affordances introduced by Phase 1d
>   (trust-verification) and Phase 1e (presence); the visual-design
>   section of this spec does not document them.
>
> The body below is preserved as the original target. The *Realised
> state* list above is authoritative for current implementation shape;
> do not edit the body in place to match it.

## Problem

Willow's voice chat is audio-only with minimal UI — just mute/deafen/disconnect buttons in the sidebar. There's no camera video, no screen sharing, no visual call page, no speaking indicators, and no way to see who's in the call at a glance.

## Scope

- **In scope:** Call page (main view), camera video, screen sharing via WebRTC, speaking detection, grid/focus layout switching, participant tiles with video and speaking indicators.
- **Out of scope:** SFU/MCU topology changes (stays full mesh), recording, virtual backgrounds, simultaneous camera + screen share (mutually exclusive — one video track per direction).

## Prior Art

This design builds on WebRTC standards and on the topologies and patterns proven by existing conferencing systems:

| System / Spec | Relevance to Willow |
|---|---|
| **WebRTC: Real-Time Communication in Browsers** (W3C Recommendation, 2021-01-26; re-published 2024-10-08) | Defines the `RTCPeerConnection` API Willow renegotiates over: `addTrack()`/`removeTrack()` on a live connection fires `negotiationneeded` and triggers a fresh offer/answer round. Willow adopts this in-place renegotiation model rather than tearing down and rebuilding peer connections when a peer starts/stops video. |
| **WebRTC "Perfect Negotiation" pattern** (W3C `webrtc-pc` example added by Jan-Ivar Bruaroey, 2019; documented on MDN / webrtc.org) | Source of Willow's glare handling: polite/impolite roles plus a `makingOffer` flag and `setLocalDescription({type:"rollback"})` to resolve simultaneous offers. Willow assigns roles deterministically by lexicographic peer-ID comparison (lower ID = polite) instead of "first to connect," and — per the realised state — relies on the `making_offer` flag alone rather than polling `signalingState`. |
| **W3C Media Capture and Streams** (`getUserMedia`, CR Draft) and **W3C Screen Capture** (`getDisplayMedia`, WD) | Define camera/mic and screen acquisition. Willow feeds both into a single outbound video track, switching the active source (camera ↔ screen) rather than capturing both at once — and must call `getDisplayMedia()` synchronously in the click handler to preserve transient activation. |
| **Jitsi Meet** (peer-to-peer full-mesh mode vs. Jitsi Videobridge SFU) | Closest architectural analog: Jitsi runs direct full-mesh P2P for small calls and escalates to the Videobridge SFU as participants grow. Willow deliberately stops at the full-mesh half of that spectrum — SFU/MCU explicitly out of scope — accepting the O(n²) upload cost the bridge exists to avoid. |
| **LiveKit** and **mediasoup** (open-source WebRTC SFU servers) | Canonical selective-forwarding-unit implementations that scale calls by routing all media through a server. Cited as the rejected alternative: Willow forgoes their scalability to keep media strictly peer-to-peer, never routing call media through an operator-run server. |
| **Discord voice/video** (custom C++ WebRTC SFU) | The product Willow replaces routes all voice/video/screen-share through Discord-operated selective-forwarding servers that hold the stream keys. Willow's full-mesh choice is the direct decentralization/privacy counterpoint — no operator server ever sees or relays call media. |
| **Matrix / Element Call** (full-mesh MSC3401 → MatrixRTC + LiveKit SFU, MSC4143/MSC4195) | Element Call's evolution makes Willow's tradeoff explicit: it began full-mesh (MSC3401) and later moved to a LiveKit-based SFU precisely because full-mesh does not scale past small groups — the same scaling ceiling Willow knowingly accepts to stay serverless-for-media. |
| **W3C Web Audio API** (`AnalyserNode.getByteFrequencyData`) | Provides the frequency/amplitude analysis Willow's `VoiceManager`/`SpeakingDetector` polls (~60 ms `setInterval`) for client-side speaking detection, instead of inferring activity from server-side RTP or a dedicated voice-activity-detection service — fitting Willow's no-media-server model. |

## Design

### 1. Call Page (Main View)

When a user joins a voice channel or clicks an active voice channel, the main content area switches to a Call Page replacing the chat view.

**Layout:**
- **Top bar:** Voice channel name, participant count, call duration timer
- **Center:** Participant grid — video/screen tiles or audio-only avatars with speaking indicators
- **Bottom bar:** Control strip — Mute, Deafen, Camera, Share Screen, Disconnect

**Participant tiles:**
- Each participant gets a tile showing their display name
- Audio-only: initial/avatar letter with a pulsing ring when speaking
- Screen-sharing: `<video>` element playing their display stream
- Speaking indicator: green border glow when audio level exceeds threshold

**Layout switching:**
- **Grid mode** (default): All tiles equally sized in a responsive CSS grid
- **Focus mode:** Click any tile to focus it (large center view). Other tiles shrink to a row of thumbnails at the bottom. Click focused tile or "Grid" button to return to grid.

**Navigation:**
- Joining voice channel → main view switches to call page
- Clicking active voice channel again → returns to call page if on chat
- Clicking a text channel → main view switches to chat (call continues, sidebar mini controls remain)
- Existing `VoiceControls` in sidebar remain as mini indicator when viewing chat during active call

**New files:**
- `crates/web/src/components/call_page.rs` — Main call page (layout, controls, grid/focus)
- `crates/web/src/components/participant_tile.rs` — Individual tile (avatar, video, name, speaking glow)

### 2. Screen Sharing

Screen sharing piggybacks on existing voice peer connections. Only available when in a voice channel.

**Starting a share:**
1. User clicks "Share Screen" in call page control strip
2. `getDisplayMedia()` called **synchronously in the click handler** (preserves user gesture — critical for browser permission). The `get_display_media()` call must happen before any `.await` or microtask boundary. The promise result is handled asynchronously.
3. Video track added to every existing peer connection via `addTrack()`
4. Triggers `onnegotiationneeded` — VoiceManager creates new offer via renegotiation on the **existing** connection
5. User's tile switches from avatar to screen feed; "Sharing" indicator appears

**Receiving a share:**
- Remote peer gets renegotiation offer with new video track
- Sets remote description on the **existing** peer connection (not a new one)
- `ontrack` handler fires with screen video track
- Participant tile switches from avatar to `<video>` element

**Stopping a share:**
- User clicks "Stop Sharing" in controls, OR browser fires `track.onended` (user clicks browser "Stop sharing")
- The `RtcRtpSender` (stored at addTrack time) is passed to `removeTrack()` on each connection
- Renegotiation fires, remote peers see track removed
- Tile reverts to avatar
- VoiceManager clears stored screen share stream and sender references

**Multiple simultaneous shares:**
- Each participant can share independently — mesh topology gives each pair their own connection
- Each connection carries 0 or 1 video tracks from each side
- Grid naturally shows all active screen shares as video tiles

**Peer joining mid-share:**
- When a new peer joins the voice channel (triggers `create_offer`), the VoiceManager checks if a local screen share is active
- If active, both the audio tracks AND the screen share video track are added to the new connection before the offer is created
- The new peer immediately receives the screen share in the initial offer

**`getDisplayMedia()` in WASM:**
- `web_sys::MediaDevices::get_display_media()` is stable (not behind unstable flag)
- Returns `Promise<MediaStream>` with video track
- Must be called from synchronous click handler (user gesture requirement — transient activation expires quickly)
- `track.onended` event detects when user stops sharing via browser chrome
- Add `DisplayMediaStreamConstraints` feature to web-sys if using the constraints variant

### 3. Camera Video

Camera video uses the same single-video-track-per-direction infrastructure as screen sharing. Camera and screen share are **mutually exclusive** — starting one stops the other.

**Starting camera:**
1. User clicks "Camera" button in call page control strip
2. `getUserMedia({ video: true, audio: false })` called synchronously in click handler (preserves user gesture). Audio is already handled by the existing voice stream — camera only adds video.
3. If a screen share video track is active, it is first removed via `stop_video_share()` (shared cleanup method).
4. Camera video track added to all peer connections via `addTrack()`, triggering renegotiation.
5. User's tile switches from avatar to camera feed.

**Stopping camera:**
- User clicks "Camera" button again (toggle off)
- Calls `stop_video_share()` which removes the video track, clears senders, and fires renegotiation
- Camera stream's video tracks are stopped (`track.stop()`) to turn off the camera indicator
- Tile reverts to avatar

**Switching between camera and screen share:**
- Starting screen share while camera is on: `stop_video_share()` runs first (stops camera), then screen share track is added
- Starting camera while screen share is on: `stop_video_share()` runs first (stops screen share), then camera track is added
- The VoiceManager tracks the current video source type: `video_source: Option<VideoSource>` where `VideoSource` is `Camera` or `Screen`

**Receiving camera:**
- Identical to receiving screen share — the `ontrack` handler fires with a video track
- The remote peer doesn't need to distinguish camera from screen share at the WebRTC level — both are just video tracks
- The tile shows the video feed regardless of source

`VideoSource` enum:
```rust
#[derive(Clone, Copy, PartialEq)]
pub enum VideoSource {
    Camera,
    Screen,
}
```

Stored in VoiceManager to know what cleanup to do when switching. Also exposed in `VoiceState` so the UI can show the right button state (camera active vs screen active).

### 4. VoiceManager Refactoring

The current VoiceManager creates a **new** `RtcPeerConnection` every time `create_offer()` or `handle_offer()` is called. This must be refactored to support renegotiation.

**Connection reuse:**
- `create_offer(peer_id)` checks `self.connections` first. If a connection exists, reuses it (calls `create_offer()` on the existing PC). Only creates a new PC if none exists.
- `handle_offer(peer_id, sdp)` checks `self.connections` first. If a connection exists, sets the remote description on it and creates an answer. Only creates a new PC if none exists.
- `ontrack` and `onicecandidate` handlers are set up once per connection (not on every renegotiation).

**Perfect negotiation (collision handling):**
When both peers try to renegotiate simultaneously (e.g., both start screen sharing at the same time), offers collide. The "perfect negotiation" pattern resolves this:

- **Polite/impolite roles:** Determined by comparing peer IDs lexicographically. The peer with the lower ID is "polite" (rolls back on collision).
- **Per-connection state:** Each connection tracks `making_offer: bool` and `ignore_offer: bool`.
- **Flow:** When `onnegotiationneeded` fires, set `making_offer = true`, create offer, set local description, send offer, set `making_offer = false`. When receiving an offer: if we're "impolite" and `making_offer` is true, ignore it. If we're "polite", rollback our local description (`setLocalDescription({type: "rollback"})`) and accept the incoming offer.
- Requires `RtcSignalingState` web-sys feature to check `pc.signaling_state()`.

**Video track management (shared for camera and screen share):**
- VoiceManager stores: `video_stream: Option<MediaStream>`, `video_source: Option<VideoSource>`, `video_senders: HashMap<String, RtcRtpSender>` (per peer)
- `start_video(stream, source: VideoSource)`: stops any existing video first via `stop_video_share()`, then stores the stream + source, calls `addTrack()` on every existing connection, stores the returned `RtcRtpSender` per peer
- `stop_video_share()`: calls `removeTrack(sender)` on each connection using stored senders, stops the stream's video tracks (`track.stop()`), clears video_stream, video_source, and senders map
- `start_screen_share(stream)`: calls `start_video(stream, VideoSource::Screen)`
- `start_camera(stream)`: calls `start_video(stream, VideoSource::Camera)`
- When creating a new connection (new peer joins), also adds the video track if active

**`ontrack` handler redesign:**
The current handler creates anonymous `<audio>` elements. The new handler must distinguish track types and route video to the signal layer:

- Audio tracks: continue creating `<audio>` elements as before
- Video tracks: fire a callback (`on_video_track`) that the app layer uses to update `remote_video_streams` signal
- The callback follows the same pattern as the existing `on_signal` callback — passed to VoiceManager at construction time
- Signature: `Rc<dyn Fn(&str, Option<MediaStream>)>` — `(peer_id, Some(stream))` when video arrives, `(peer_id, None)` when video track ends

**Cleanup on peer leave:**
When `VoiceLeft` event fires and `close_connection(peer_id)` is called, also fire `on_video_track(peer_id, None)` to clean up the `remote_video_streams` signal entry.

### 5. Speaking Detection

Visual indicators on participant tiles showing who's talking.

**Implementation:**
- `SpeakingDetector` struct in `voice.rs` wrapping `HashMap<String, AnalyserNode>` and a shared `AudioContext`
- When remote audio track arrives (`ontrack` with audio kind), create `MediaStreamAudioSourceNode` → `AnalyserNode` and store
- Single polling loop (~60ms via `setInterval`) checks all analysers' `getByteFrequencyData()` for volume above threshold
- Updates `speaking_peers: WriteSignal<HashSet<String>>` (passed at construction as a callback, same pattern as `on_video_track`)
- Local microphone stream also connected to an analyser for self-feedback

**Cleanup:**
- `remove_peer(peer_id)`: disconnect and remove analyser for that peer
- `destroy()`: close the AudioContext, clear all analysers. Called when leaving voice.

**UI:**
- Speaking tile: `border: 2px solid var(--online)` with glow `box-shadow: 0 0 8px var(--accent-green-glow)`
- Non-speaking tile: default border
- Note: speaking detection polling may be throttled in background tabs. This is acceptable — speaking indicators are a visual-only enhancement.

### 6. State Additions

**`UiState` / `UiWriteSignals`:**

```
show_call_page: ReadSignal<bool>
call_layout: ReadSignal<CallLayout>
```

`CallLayout` enum:
```rust
#[derive(Clone, PartialEq, Default)]
pub enum CallLayout {
    #[default]
    Grid,
    Focus(String),  // focused peer_id
}
```

**`VoiceState` / `VoiceWriteSignals`:**

```
video_source: ReadSignal<Option<VideoSource>>       // what the local user is sharing (Camera, Screen, or None)
speaking_peers: ReadSignal<HashSet<String>>          // peers currently speaking
remote_video_streams: ReadSignal<HashMap<String, SendWrapper<web_sys::MediaStream>>>
```

`web_sys::MediaStream` is not `Send` — wrapped in `SendWrapper` consistent with existing patterns.

`video_source` replaces the old `screen_sharing: bool` — the UI checks `video_source.get()` to show the correct button state (camera active, screen active, or neither).

**Signal update mechanism:** VoiceManager does NOT hold Leptos signals directly. It fires callbacks (`on_video_track`, `on_speaking_change`) set at construction, same as the existing `on_signal` pattern. The app layer wires these callbacks to update the appropriate `WriteSignal`s.

### 7. Files Modified

| File | Changes |
|------|---------|
| **Create:** `components/call_page.rs` | Call page component (top bar, grid/focus, controls including camera + screen share) |
| **Create:** `components/participant_tile.rs` | Individual tile (avatar, video, speaking glow) |
| **Modify:** `voice.rs` | Refactor connection reuse, perfect negotiation, unified video track management (camera + screen), `ontrack` redesign, `SpeakingDetector`, new callbacks (`on_video_track`, `on_speaking_change`) |
| **Modify:** `state.rs` | `CallLayout` enum, `VideoSource` enum, new signals in `UiState` and `VoiceState` |
| **Modify:** `components/mod.rs` | Register new components |
| **Modify:** `app.rs` | Call page in main view routing, voice channel click shows call page, wire VoiceManager callbacks to signals, camera/screen share click handlers |
| **Modify:** `components/sidebar.rs` | Voice channel click opens call page |
| **Modify:** `event_processing.rs` | Clean up `remote_video_streams` on `VoiceLeft` |
| **Modify:** `icons.rs` | Add `icon_monitor`, `icon_video`, `icon_video_off`, `icon_grid`, `icon_maximize` |
| **Modify:** `style.css` | Call page, grid, tiles, controls, speaking glow, focus mode, camera/screen toggle states |
| **Modify:** `Cargo.toml` | Add web-sys features: `DisplayMediaStreamConstraints`, `AudioContext`, `BaseAudioContext`, `AudioNode`, `AnalyserNode`, `MediaStreamAudioSourceNode`, `RtcSignalingState` |

### 8. Visual Design

The call page should feel **immersive and focused** — a distinct mode shift from chat. When you enter a call, the UI should signal "you're in a live session" through atmosphere, not just layout.

**Call page background:**
- Darker than `--bg-main` — use `#121215` or a very subtle radial gradient from center (`#16161a`) to edges (`#0e0e12`). This creates depth and draws attention to the participant tiles.
- No hard borders between call page and sidebar — the call page fills the main content area seamlessly.

**Participant tiles:**

*Audio-only tiles:*
- Rounded rectangle, `border-radius: 16px`, `aspect-ratio: 16/9` in grid mode
- Background: unique per-peer gradient derived from their peer ID hash. Generate 2 colors from the peer ID and create a diagonal gradient (`135deg`). This gives every participant a distinct visual identity without requiring profile pictures.
- Large initial letter centered: `font-size: 48px`, `font-weight: 600`, `color: rgba(255,255,255,0.9)`, slight `text-shadow: 0 2px 8px rgba(0,0,0,0.3)`
- Display name at bottom: small label overlaid with subtle dark gradient backdrop (`linear-gradient(transparent, rgba(0,0,0,0.6))`)

*Video tiles:*
- Same rounded rectangle, `border-radius: 16px`, `overflow: hidden`
- Video element: `object-fit: cover`, fills the tile
- Display name overlay at bottom-left with semi-transparent backdrop
- When screen sharing: `object-fit: contain` instead (shows full screen content with letterboxing)
- Subtle `box-shadow: 0 4px 24px rgba(0,0,0,0.4)` for depth

*Speaking indicator:*
- `border: 2px solid var(--online)` with `box-shadow: 0 0 0 3px var(--accent-green-glow), 0 0 12px var(--accent-green-glow)`
- Animated: the glow pulses gently with `animation: speaking-pulse 1.5s ease-in-out infinite`
- Transition: `border-color 0.2s ease, box-shadow 0.2s ease` for smooth on/off

*Muted indicator:*
- Small crossed-out mic icon badge in the bottom-right corner of the tile, `background: var(--danger)`, `border-radius: 50%`, `width: 24px; height: 24px`

**Grid layout:**
- CSS Grid: `grid-template-columns: repeat(auto-fit, minmax(280px, 1fr))`, `gap: 12px`, `padding: 16px`
- 1 participant: tile takes full width, max-width ~640px, centered
- 2 participants: side-by-side, equal width
- 3-4: 2x2 grid
- 5+: auto-fit wrapping
- Tiles animate in with staggered `animation-delay` on entry (`fade-in + scale` from 0.95)

**Focus layout:**
- Focused tile: `grid-column: 1 / -1`, takes ~75% of vertical space, `max-height: 70vh`
- Thumbnail row below: `display: flex`, `gap: 8px`, `height: 120px`, `overflow-x: auto`
- Thumbnails: fixed `width: 160px`, `height: 120px`, `border-radius: 10px`, `cursor: pointer`
- Clicking thumbnail swaps it to focus; clicking focused tile returns to grid
- Transition between layouts: `transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1)`

**Control strip:**
- Fixed at bottom of call page, `padding: 12px 0`, centered
- Dark translucent bar: `background: rgba(0,0,0,0.4)`, `backdrop-filter: blur(12px)`, `border-radius: 16px`, `padding: 8px 16px`
- Buttons: circular, `width: 48px; height: 48px`, `border-radius: 50%`
  - Default: `background: var(--bg-elevated)`, `color: var(--text-primary)`
  - Hover: `background: var(--bg-input)`, slight scale `transform: scale(1.05)`
  - Active (muted/camera on/sharing): `background: var(--accent)`, `color: white`
  - Danger (disconnect): `background: var(--danger)`, `color: white`, wider pill shape `border-radius: 24px; padding: 0 24px`
- Tooltip on hover showing button name: `font-size: 11px`, appears above with arrow
- Button icons: the Lucide SVG icons from `icons.rs`, `font-size: 20px`
- Spacing between buttons: `gap: 8px`
- On mobile: slightly larger buttons (`56px`), no tooltips

**Top bar:**
- Minimal: channel name left-aligned, participant count center, duration right-aligned
- Duration timer: `font-variant-numeric: tabular-nums` (monospace digits for stable width), `color: var(--text-muted)`, `font-size: 13px`
- Participant count: pill badge `background: var(--bg-input)`, `border-radius: 12px`, `padding: 2px 10px`
- Layout toggle button (grid/focus): top-right corner, subtle icon button

**Animations:**
```css
@keyframes speaking-pulse {
    0%, 100% { box-shadow: 0 0 0 3px var(--accent-green-glow), 0 0 12px var(--accent-green-glow); }
    50% { box-shadow: 0 0 0 5px var(--accent-green-glow), 0 0 20px var(--accent-green-glow); }
}

@keyframes tile-enter {
    from { opacity: 0; transform: scale(0.95); }
    to { opacity: 1; transform: scale(1); }
}
```

**Color generation for avatar gradients:**
Derive two hue values from the peer ID string (hash the first and second half). Use HSL with `saturation: 45%, lightness: 35%` for muted but distinct colors that don't clash with the UI:
```
hue1 = hash(peer_id[0..len/2]) % 360
hue2 = (hue1 + 40 + hash(peer_id[len/2..]) % 60) % 360
background: linear-gradient(135deg, hsl(hue1, 45%, 35%), hsl(hue2, 45%, 30%))
```

## Testing

- WASM compilation (`just check-wasm`) must pass.
- Existing voice E2E tests should still work (video is additive).
- Manual testing: join voice channel from 2 browsers, verify call page renders, camera works, screen share works, switching between camera and screen share works, speaking indicators pulse, grid/focus layout switching works.
- Screen sharing cannot be automated in Playwright (browser blocks `getDisplayMedia` in headless mode) — manual test only.
- Camera can potentially be tested with Playwright's `--use-fake-device-for-media-stream` flag.
- Test edge cases manually: peer joins mid-share, peer leaves while sharing, simultaneous video start by two peers, switching camera↔screen mid-call.
