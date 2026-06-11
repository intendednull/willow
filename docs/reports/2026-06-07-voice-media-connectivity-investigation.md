# Voice/Video Media Failure — Root-Cause Investigation

**Date:** 2026-06-07
**Status:** investigation complete. Immediate code fixes (RC0, RC2, RC3, RC4,
perfect-negotiation, RC5 logging, RC1 *config plumbing*) implemented, tested,
and proven end-to-end (2-peer Playwright media test: remote audio both ways +
screen-share renegotiation). Cross-NAT traversal (RC1 — self-hosted coturn
deployment) tracked as follow-up.
**Area:** voice/video (WebRTC media + iroh signaling)

## Symptom

A user can join a voice/video call and see the other participants in the roster,
but media does not work: the remote peer cannot hear the microphone and cannot
see the camera/screen share. The user's **own** screen-share shows correctly in
their **local** preview.

That split is the whole story: **presence/roster rides iroh gossip (works)**;
**media rides browser WebRTC (broken)**; **local preview is a local `MediaStream`
bound to a `<video>` with no peer connection (works)**. Remote media needs a
negotiated WebRTC path with working ICE, which is broken at three layers
simultaneously.

## Method

Multi-agent review: 4 parallel code-audit passes (WebRTC manager, signaling +
concurrency, UI/state, design docs), 4 parallel web-research passes (perfect
negotiation, ICE/STUN/TURN behaviour, P2P reference apps, Rust-WebRTC/iroh
capabilities), then adversarial verification of every candidate root cause
(15 verdicts) and a synthesis pass. Headline causes (RC1–RC3) were
re-verified by hand against source.

## How the system works — two planes

**Signaling plane (iroh gossip — WORKS).** Presence via
`WireMessage::VoiceJoin`/`VoiceLeave` on `SERVER_OPS_TOPIC`. When a peer joins,
existing participants each spawn `create_offer` toward the joiner; the joiner
answers (one offerer per pair). SDP/ICE serialized to strings and gossiped as
`WireMessage::VoiceSignal { channel_id, target_peer, signal }`. Sent in
`crates/client/src/voice.rs::send_voice_signal`; dispatched to the `VoiceManager`
in `crates/web/src/event_processing.rs:113-133`.

**Media plane (browser WebRTC — BROKEN).** One `RtcPeerConnection` per remote
peer, owned by `VoiceManager` (`crates/web/src/voice.rs`). `RtcConfiguration`
built from `resolve_stun_urls()` → **empty `iceServers`** by default. Remote
video renders only when `ontrack` fires → `on_video_track` callback →
`set_remote_video_streams` → `ParticipantTile` binds `srcObject`. The UI plane is
correct; it never receives a stream because the media plane never connects.

## Root causes (ranked, all confirmed)

### RC0 — Voice wire messages addressed by name, gates validate by UUID
- **Severity:** Critical (broke **all voice**, before media even mattered).
  Discovered by the new end-to-end media test after the RC1-RC4 fixes landed.
- **Where:** the UI passes the channel *name* into `join_voice` /
  `send_voice_signal`, so `VoiceJoin`/`VoiceLeave`/`VoiceSignal` carried the
  name — but the SEC-V-03 existence gates (`crates/client/src/listeners.rs`)
  validate `channel_id` against `ServerState.channels`, which is keyed by the
  canonical UUID (`mutations.rs` `create_channel` generates
  `uuid::Uuid::new_v4()`). Every voice message was dropped with
  `"dropping VoiceJoin: channel_id not in ServerState.channels"` — no
  participant ever appeared in a call, no signaling was ever delivered.
- **Fix (landed):** boundary translation. The wire now carries the canonical
  `channel_id` (UUID): senders resolve name→id via
  `ClientMutations::channel_id_for_voice` (accepts either form); the three
  listener gates resolve id→name and key voice state / `ClientEvent`s by name
  (consistent with the name-keyed UI). Security tests updated to assert the
  UUID-on-wire / name-in-state contract.

### RC1 — Empty ICE config: no STUN/TURN, no NAT traversal
- **Severity:** Critical (breaks **all cross-network calls**, any media type).
- **Where:** `crates/web/src/voice.rs:867-893` (`resolve_stun_urls()` returns
  empty `Vec` by design); `899-913` (`build_rtc_config()` calls `set_urls()` only —
  never `set_username()`/`set_credential()`, so it cannot drive TURN even if
  configured); unimplemented intent at `voice.rs:862-866`.
- **Mechanism:** With zero `iceServers`, the browser gathers only **host**
  candidates (obfuscated as `.local` mDNS). No server-reflexive (needs STUN), no
  relay (needs TURN). Same-machine and flat same-LAN may connect; **any NAT
  boundary fails** — no candidate pair forms, connection stalls and times out.
- **Note:** The empty default is a deliberate privacy choice (issue #179: Google
  STUN was leaking every caller's IP). The bug is that **no privacy-preserving
  replacement was built** — NAT traversal is simply absent.

### RC2 — 4 KB `SIGNALING_CAP` silently drops video SDP
- **Severity:** Critical (breaks **video calls on any network**, incl. same-LAN).
- **Where:** `crates/common/src/wire.rs:118` (`SIGNALING_CAP = 4*1024`), `172`
  (`VoiceSignal` mapped to `SIGNALING_CAP`), enforced post-decode in `unpack_wire`
  (drops oversize with a `tracing::warn`, returns `None`).
- **Mechanism:** Audio-only SDP (< ~2 KB) passes. Video offers/answers with
  H.264/VP8/VP9/AV1 codec lists + RTP header extensions routinely run 5–15 KB.
  The message deserializes, then the per-variant cap drops it. The offerer never
  receives an answer; no user-visible error. The cap's own doc comment
  (`wire.rs:152-155`) wrongly claims "SDP/ICE blobs — all small."

### RC3 — Early/ordering-lost remote ICE candidates (silent failure)
- **Severity:** Critical for connectivity.
- **Where:** `crates/web/src/voice.rs:679-697` — `handle_ice_candidate` early-
  returns `no connection for peer` (687) if the connection doesn't exist yet, and
  discards the `add_ice_candidate` result with `let _` (693-695). Candidate
  handling is **synchronous** (`event_processing.rs:128`, `vm.borrow()`) while
  offer/answer are **async** (`spawn_local`, `event_processing.rs:121,125`).
- **Mechanism:** Per W3C/MDN, `addIceCandidate()` rejects (`InvalidStateError` /
  `OperationError`) when `remoteDescription` is null — browsers do **not** buffer
  *remote* candidates. Gossip gives no ordering guarantee, and within one event
  batch the synchronous ICE handler runs before the spawned offer/answer task.
  Candidates arriving before `setRemoteDescription` are rejected, the rejection is
  swallowed, and the candidate is lost forever → missing candidate pairs → no
  media even when the SDP exchange succeeds. Classic "negotiation completes but
  media never flows."

### RC4 — `RefCell<VoiceManager>` borrow held across `await`
- **Severity:** Critical when it fires; intermittent.
- **Where:** `crates/web/src/app.rs:1465-1488` — `handle_voice_create_offer`/
  `handle_voice_offer` hold `borrow_mut()` across `.await`; `handle_voice_answer`
  holds `borrow()` across `.await`. Incorrect "no preemption" comment at
  `1462-1463`; `#[allow(clippy::await_holding_refcell_ref)]` suppresses the lint.
- **Mechanism:** Single-threaded WASM has no OS preemption, but `.await` is a
  cooperative yield point — `spawn_local` tasks interleave there. While one
  handler holds the borrow across its await, a second inbound `VoiceSignal`
  handler (incl. the synchronous `vm.borrow()` ICE path, fired many times by
  trickle ICE) → `BorrowMutError` panic, leaving the connection half-initialised
  and not retried. Adversarial verdicts split confirmed/partial on whether this is
  *the* reproducing cause, but agree it is a real hazard that must be fixed.

### Ruled out / secondary
- **Perfect-negotiation collision check uses only `making_offer`, not
  `signalingState != "stable"`** (`voice.rs:464-502, 622`) — real defect, breaks
  renegotiation/glare (screen-share added after connect), but not the primary
  cause of total media absence. Fix alongside RC3.
- **Self-echo offer to self** (no `local_peer_id` filter at
  `event_processing.rs:76-92`) — secondary; amplifies RC4.
- **Audio `RtcRtpSender` discarded** (`voice.rs:314-322`) — ruled out as a cause;
  tracks are still transmitted (monitoring gap only).
- **UI rendering pipeline** (`participant_tile.rs`, `call_page.rs`, `state.rs`) —
  ruled out; correct and ready to display the instant a stream arrives.

## Reference projects & how they solve this

| Project | Topology | Transport / NAT traversal | Lesson for Willow |
|---|---|---|---|
| **Jami** | full-mesh | ICE; public IP from OpenDHT (no 3rd-party STUN); TURN fallback | Serverless app deriving peer IP from its own overlay — Willow's "iroh replaces STUN" aspiration, but native. |
| **SimpleX Chat** | 1:1 mesh | browser WebRTC; signal over existing E2E connection; self-hosted relay hides IPs | Best precedent for "signal SDP over the channel you already have" + IP-hiding via own relay. |
| **Matrix / Element Call + LiveKit** | mesh → SFU | WebRTC + STUN/TURN; E2E frames through SFU | Where mesh breaks (>4–6) and how to keep E2E through an SFU. Long-term only. |
| **Jitsi Meet** | mesh small / SFU large | operator-run STUN/TURN/SFU | Canonical small=mesh / large=SFU boundary. |
| **Tox / qTox** | P2P mesh | custom UDP, DHT, own hole-punch + TCP relay, no 3rd-party STUN/TURN | Direct P2P media works without SFU/STUN — but bespoke native, not browser. |
| **js-libp2p WebRTC** | browser↔browser | WebRTC data channels; still needs STUN + signaling relay | Even with known peer identity, **browser↔browser still needs STUN** to learn its own public IP. Data only. |
| **iroh callme / iroh-roq / iroh-live (n0)** | native P2P | RTP/MoQ over QUIC over iroh; hole-punch + relay; no STUN/TURN | Native media-over-iroh is proven — but native Rust; browser viewing needs a WebTransport↔WebRTC bridge. |

**Full-mesh vs SFU:** full-mesh is correct for Willow's target size (2–6); mesh
fails past ~4–6 (quadratic upload, N-1 decoders). The call-page spec already
scopes SFU out — that matches. SFU is a future concern, not a fix for this bug.

**RTP-over-QUIC / iroh-carried media for the browser leg — not feasible today.**
In browsers iroh is **relay-only over WebSocket**, does **not** hole-punch (no raw
UDP in browsers), has **no native A/V**, and its relays are **proprietary, not
TURN/ICE-compatible** — a browser `RtcPeerConnection` cannot use an iroh relay as
an ICE candidate. The `voice.rs:862-866` TODO ("use the iroh relay path for ICE")
is therefore **infeasible as written** and should be retired. Native↔native media
over iroh (iroh-roq/MoQ) is the right long-horizon direction but means leaving
browser WebRTC media + adding a WebTransport↔WebRTC bridge — a major rewrite.

## Recommended fixes

### (a) Immediate code fixes — no architecture change

1. **Stop holding `RefCell` across `await` (RC4).** Restructure `VoiceManager`
   methods so the borrow scope ends before the first `await`: clone the
   `RtcPeerConnection` (cheap JS handle) out under a short borrow, drop the
   borrow, then await on the clone. Remove the `#[allow(...)]` and the false
   "no preemption" comment. Add the self-peer filter while here.
2. **Add a per-connection ICE candidate queue (RC3).** `pending_candidates` per
   connection. In `handle_ice_candidate`: create/queue if no connection yet; push
   to queue if `remoteDescription` is null; else add immediately. Drain after
   `setRemoteDescription` resolves in `handle_offer`/`handle_answer`. Stop
   discarding the result — `await` it and log rejections (simple-peer/Pion pattern).
3. **Fix perfect-negotiation collision condition.** Guard on
   `offer && (making_offer || signaling_state() != Stable)`. Prefer no-arg
   `setLocalDescription()` in `onnegotiationneeded`; wrap the `making_offer`
   flag in a finally-equivalent; drop explicit `{type:'rollback'}` (modern
   browsers auto-rollback in `setRemoteDescription`).
4. **Raise the signaling cap for SDP (RC2).** Add a dedicated `SDP_CAP` (e.g.
   64 KB = `DEFAULT_CAP`) and map `VoiceSignal` to it instead of `SIGNALING_CAP`
   (`wire.rs:156-177`). Fix the misleading comment. Add a wire round-trip test
   with a realistic ~10 KB video SDP. (Runner-up rejected: splitting `VoiceSignal`
   into offer/answer vs candidate variants — more churn, no benefit.)
5. **Add connection-state + ICE-failure logging.** Wire
   `oniceconnectionstatechange` / `onconnectionstatechange` /
   `onicegatheringstatechange` to `tracing::warn` on `failed`/`disconnected` and
   log the selected candidate pair. Surface the `unpack_wire` cap-drop warn for
   `VoiceSignal` during bring-up. Converts silent failures into signal.

### (b) NAT traversal strategy

**Shortest path — self-host coturn (STUN+TURN) co-located with the Willow relay.**
Cross-network traversal fundamentally requires a reflexive path (STUN, fails on
symmetric NAT) or a media relay (TURN, universal). Because the TURN server is
*ours*, no third party learns participant IPs — this **preserves the privacy
property** that motivated the empty default. Extend `build_rtc_config()` to carry
TURN URL + username + **time-limited HMAC credential** (coturn `use-auth-secret` /
TURN REST), served to the client at boot (extend/replace the `__WILLOW_STUN_URLS`
hook to carry full `iceServers` entries). Keep mDNS for same-LAN; optionally add a
self-hosted STUN entry so only hard-NAT pairs consume relay bandwidth.

**Long-term (record, don't build now):** native↔native media over iroh
(iroh-roq/MoQ) + a WebTransport↔WebRTC bridge for browsers. Rejected for
near/medium term: the browser leg still requires WebRTC, MoQ still needs
relay/SFU infra, and it's a major rewrite that loses the mature browser A/V
pipeline.

## Verification plan

| Fix | Tier | Assert | Location |
|---|---|---|---|
| RC2 cap | client/common (Rust) | ~10 KB video SDP survives `pack_wire`/`unpack_wire` as `VoiceSignal::Offer` | `crates/common/src/wire.rs` tests |
| RC4 borrow | client (Rust) | interleaved offer/answer/ICE handlers → no `BorrowMutError` | `crates/client/src/tests/` |
| RC3 queue | browser (wasm-pack) | candidate before `setRemoteDescription` is applied after, not lost | `crates/web/tests/browser.rs` |
| neg. guard | browser (wasm-pack) | simulated glare → both reach `stable` | `crates/web/tests/browser.rs` |
| RC1 TURN wiring | browser (wasm-pack) | `build_rtc_config()` emits `RtcIceServer` with URL **and** username/credential | `crates/web/tests/browser.rs` |
| E2E media | Playwright | remote `ontrack` fires, remote tile renders live stream | `e2e/voice-video.spec.ts` (new) |

Automated tiers cannot prove cross-NAT (shared network). **Manual 2-machine
protocol** (genuinely different NATs, e.g. home Wi-Fi + mobile tether): join,
confirm roster, open `chrome://webrtc-internals` and verify ICE state reaches
`connected`/`completed`, `srflx`/`relay` candidates appear (not host-only), the
selected pair is `relay`↔`relay` cross-NAT, and `getStats` inbound-rtp
`packetsReceived`/`bytesReceived` increase on both ends (do **not** trust
`iceConnectionState=connected` alone). Repeat for audio / camera / screen share.
Negative control: disable TURN → cross-NAT fails.

## Docs to update (when fixes land)

- **This report** documents the investigation.
- **Create `docs/specs/2026-06-07-webrtc-nat-traversal-design.md`** — target:
  self-hosted coturn beside the relay; ephemeral TURN credentials; client config
  delivery; privacy rationale; explicit rejection of the infeasible "iroh relay
  for ICE"; SFU/MoQ recorded as out-of-scope future; candidate-queue +
  perfect-negotiation correctness as the media-signaling contract.
- **Qualify `[landed]` on `docs/specs/2026-03-26-screen-sharing-call-page-design.md`**
  / its plan — the "joiner immediately receives screen share in the initial
  offer" assumption is false (no NAT traversal; video SDP dropped by the cap).
  Downgrade to `[active]` or add a Realised-state addendum; cross-link the new
  spec as a blocking dependency.
- **Update `docs/specs/2026-03-29-iroh-migration-design.md`** Voice/WebRTC section
  to point at the new traversal spec and correct the implication that the iroh
  relay can serve browser ICE.
- **Update `docs/README.md`** to register the new report + spec.
- **Code-comment cleanup:** replace the infeasible "iroh relay path for ICE" at
  `voice.rs:862-866`; fix the false "no preemption" comment at `app.rs:1462-1463`;
  fix the "SDP/ICE blobs — all small" comment at `wire.rs:152-155`.
