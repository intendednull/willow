# Call experience — grove / grid / focus, controls, whisper, handoff, share

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md), [`whisper-mode.md`](whisper-mode.md), [`device-handoff.md`](device-handoff.md), [`messaging.md`](messaging.md)

## Purpose

The call experience is where voice and video become tangible — a live
gathering inside a grove. It is the most attention-heavy surface in
Willow: every tile is a face, every control a commitment to presence.

The call should feel *like a room you are in*, not a dashboard you are
operating. Dense information (latency, jitter, codec) is on demand;
ambient information (who is speaking, whispering, sharing) is always
legible at a glance. Trust state stays subtle — a single lock chip —
until a user asks for detail.

## Scope

- Three user-switchable layouts (`grove`, `grid`, `focus`), persisted
  per-channel, cycled by keyboard shortcut `L`.
- Participant tile + per-tile context menu.
- Bottom controls strip with left / center / right clusters.
- Whisper (pill in header + popover/sheet) and handoff (popover/sheet)
  integration points.
- Screen share source picker, sharer indicator, viewer canvas, zoom
  and pin.
- Local-only speaking stats popover.
- Call start / join / leave motion and chime.
- Call header (grove · channel · whisper pill · sealed chip · close).
- Mobile adaptations — full-screen, auto-hiding controls, bottom
  sheets. Swipe-to-minimize deferred to v2.
- In-call chat side panel / bottom sheet (inherits [`messaging.md`](messaging.md)).
- Edge cases: empty call, poor connection, auto-reconnect, permission
  denial.
- Accessibility contract for the grid, controls, and shortcuts.

## Non-goals

- Recording, transcription, captions, breakout rooms, virtual
  backgrounds, in-call reactions. Deferred or handled elsewhere.
- Native picture-in-picture. Gesture reserved on mobile; no impl in
  v1.

## Layouts

Three layouts, switchable from the header and controls strip. Active
mode persisted per-channel in `localStorage` (`willow.call.layout.<channel_id>`).
Keyboard `L` cycles grove → grid → focus → grove.

### grove

Default for calls < 8 participants. Tiles arranged as a loose
irregular circle around a centred moss clearing — a very low-contrast
radial gradient (`rgba(106, 141, 94, 0.06)` over `--bg-0`). The
clearing also hosts the now-speaking ticker when someone is active.

Placement:

- ≤ 4 tiles: loose diamond with 45° rotational offset so no two align.
- 5–7 tiles: distributed around the clearing, radii jittered 4–10 px
  to avoid a clock-face look. Position deterministic from peer id hash
  (no jumping between renders).
- Self always closest to bottom-centre, angled toward the viewer.

Active speaker: 2 px `--moss-3` halo + scale 1.02 (transform,
`var(--motion)`) + soft outer glow
`0 0 0 8px color-mix(in oklab, var(--moss-3) 18%, transparent)`.

Screen share: collapses to a centred card (60% stage width, `--radius-l`,
`--shadow-2`); tiles re-flow at smaller radius around it.

### grid

Uniform tiles, auto rows/cols. Columns by participant count:

| Count | Cols | | Count | Cols |
|-------|------|--|-------|------|
| 1–2   | 1–2  | | 10–12 | 4    |
| 3–4   | 2    | | 13–16 | 4    |
| 5–6   | 3    | | 17+   | 5 (scrollable) |
| 7–9   | 3    | |       |      |

Aspect targets 16:10; compresses to 4:3 before introducing a scrollbar.

Active speaker: 2 px `--moss-3` border ring + speaking-level bars at
tile bottom (no scale — the grid rewards stability).

Screen share: spanning cell (`grid-column: 1 / -1`), ≥ 50% of stage
height; participants fill a compact strip below.

### focus

One large primary tile (70–80% of stage, 16:9) + a filmstrip of
others (bottom on desktop and mobile, 96 px tall, scrolls
horizontally).

Primary selection order:

1. Pinned participant (local pin).
2. Active screen sharer.
3. Active speaker (debounced: promote after 1.2 s continuous voice,
   hold ≥ 3 s before replacing). Pin overrides debounce.
4. Most-recent speaker.
5. First peer by deterministic sort.

Keyboard: `Tab` cycles filmstrip focus; `Space` pins / unpins the
focused tile.

### Persistence

Extend `AppState.ui.call_layout` (`CallLayout` enum) with `Grove`.
Persist the chosen value per channel id. Reset to `grove` when no key
is stored.

## Participant tile

Shared by all layouts, layout-specific sizing.

Contents:

- **Plate.** `--bg-2`, `--radius-l` (14 px). Radial gradient from the
  peer's crest colour at 30% alpha fading to `--bg-0`.
- **Video layer.** `object-fit: cover` for camera (mirrored for self),
  `object-fit: contain` for screen share.
- **Avatar fallback.** Centred circle, Fraunces italic initial. 72 px
  (grid), 120 px (focus primary), 40 px (mobile filmstrip). Background:
  peer avatar colour, foreground `#14130f`.
- **Bottom-left name chip.** Frosted pill `rgba(20, 19, 15, 0.78)` +
  `backdrop-filter: blur(8px)`, `--ink-0`, `body S`. Contains display
  name, `· you` suffix if local in `--ink-3`, verified check (9 px, moss)
  if SAS-verified, ear icon (11 px, `--whisper`) if peer is in the
  local user's whisper.
- **Bottom-right mute badge.** Muted: `--err` icon on
  `rgba(201, 122, 90, 0.25)`; unmuted: frosted dark, shown on hover /
  focus only.
- **Top-right connection-quality chip.** 2-dot signal + mono latency
  (`120 ms`). Moss (healthy, < 200 ms, < 1% loss); mixed moss+amber
  (degraded); amber (poor); `--err` + "reconnecting…" (dropped).
- **Top-left sharing strip** (sharer only). Small moss pill "you are
  sharing" with inline `x` to stop.
- **Speaking-level bars** (active speaker in grid / focus only).
  Five 2 px bars, heights 3–10 px, `--moss-3`, `willowPulse` with
  120 ms stagger.

Context menu (right-click / 500 ms long-press / `Shift+F10`):

1. **pin** / **unpin** — overrides focus primary slot.
2. **mute for you** / **unmute for you** — local-only audio mute;
   shows "muted for you" on the tile while active.
3. **view profile** — opens [`profile-card.md`](profile-card.md).
4. **start a whisper with** *{name}* — delegates to
   [`whisper-mode.md`](whisper-mode.md).

Self-tile menu swaps "start a whisper with" for **open whisper
controls**, and surfaces **stop camera** / **stop sharing** when
applicable.

All active-speaker visuals respect `prefers-reduced-motion`: scale
collapses to 1.0, glow collapses to static border, bars collapse to a
single static moss dot.

## Controls strip

Fixed bottom of the stage. Desktop: inset 16 px, floating on a
frosted plate — `--bg-1` at 90% alpha, `backdrop-filter: blur(20px)`,
`--line-soft` top border, `--radius-l` top corners.

Three clusters. Intra-cluster gap 10 px, inter-cluster gap 24 px.
Buttons are circular 44 × 44 (`.call-btn`) unless noted.

### Left cluster — media toggles

- **mic** (`mic` / `mic-off`). Muted: `--err` icon, red-tinted bg.
  Keyboard `M`.
- **deafen** (`headphones` / `headphones-off`). Deafened implies
  muted (existing behaviour); tooltip notes it. Keyboard `D`.
- **camera** (`video` / `video-off`). Active: moss tint. Keyboard `V`.
- **screen share** (`monitor`). Active: moss tint + inner ring.
  Keyboard `S`.

Permission-denied: icon is struck-through with a diagonal 1.5 px
`--err` stroke; button disabled; tooltip "mic blocked in browser
settings" / "camera blocked in browser settings" with a `?` linking to
browser-specific help.

### Center cluster — room actions

- **layout switcher.** Three-segment pill (see
  [`layout-primitives.md`](layout-primitives.md)) with `tree` / `grid` /
  `maximize` icons and labels `grove` / `grid` / `focus`. Active
  segment: `--bg-3` fill, `--ink-0`. Inactive: transparent,
  `--ink-2`. `role="radiogroup"`. Keyboard `L` cycles; arrow keys
  navigate when focused.
- **whisper** (`ear`). Border `color-mix(in oklab, var(--whisper)
  40%, var(--line))`, foreground `--whisper`. Active (in whisper):
  filled `color-mix(in oklab, var(--whisper) 30%, var(--bg-2))`. Opens
  the whisper popover / sheet. Keyboard `W`.
- **handoff** (`device`). Neutral tone. Opens the handoff popover /
  sheet. While a handoff is in progress, renders a 6 px `--amber`
  `willowPulse` dot at its top-right. Keyboard `H`.
- **in-call chat** (`thread`). Toggles the chat panel. Red dot when
  new messages arrive while closed. Keyboard `/` toggles + focuses
  composer.

### Right cluster

- **participants count.** Pill: `users` icon + count. Opens a
  compact participant list with mute / speaking / quality per row.
  Keyboard `P`.
- **overflow** (`more-horizontal`). Menu: **speaking time**, **invite**,
  **call settings** (input / output / video device, mic test),
  **report an issue** (diagnostics). Keyboard `O`.
- **disconnect.** The only non-circular button — a pill, 40 px tall,
  `var(--err)` bg, `#14130f` fg, `phone-off` icon + label
  "disconnect". Keyboard `Esc` held 600 ms (prevents accidental
  leave); `Enter` when focused fires immediately.

## Whisper integration

Whisper is a side-channel inside the call; the surfaces live in
[`whisper-mode.md`](whisper-mode.md). This spec guarantees:

1. **Whisper button** in the center cluster. Opens the popover
   (desktop) or sheet (mobile); opening animation `willow-pop-in`
   over `var(--motion)`.
2. **Whisper pill** in the call header, rendered when the local user
   is in an active whisper. Copy: `whisper · {name}` for 1:1;
   `whisper · {n} others` for groups. Style:
   `color-mix(in oklab, var(--whisper) 14%, var(--bg-1))` bg,
   `--whisper` border + text, ear icon. Click opens the same
   popover / sheet. Never accent-swappable.
3. **Per-tile cue** — ear icon in the name chip and violet border ring
   only for peers in the *same* whisper as the local user. Whispers
   the user is not part of are invisible — no silhouette, no dot.

## Handoff integration

Opens the handoff surface from [`device-handoff.md`](device-handoff.md).
This spec guarantees:

- Anchor: bottom-centre of the controls strip.
- Popover (desktop): `--shadow-2`, `--radius` 14 px, min-width 300 px,
  max-width 340 px.
- Sheet (mobile): `--radius-l` top corners, full width, safe-area
  bottom inset.
- In-progress badge: 6 px `--amber` pulsing dot top-right of the
  handoff button.

## Screen share

### Starting

1. User clicks the screen-share toggle.
2. Browser native source picker (window / screen / tab) — we do not
   build a custom UI for this; `getDisplayMedia` handles it.
3. Success: toggle activates (moss), sharer's own tile gets the top
   "you are sharing" strip (20 px, moss-tinted, `body S`, with `x`
   to stop).
4. Cancel / deny: no UI change; a toast may surface hard-block
   denials.

### Viewing

- Grid / focus: share takes the primary slot.
- Grove: collapses to a right-side panel, 360–420 px wide, full stage
  height. The clearing shrinks, tiles reflow around it.
- Canvas overlay controls (top-right, fade-in on hover; on mobile
  appear for 3 s after a tap):
  - **zoom in** (`plus`) — 1.25× per click, cap 3×.
  - **zoom out** (`minus`) — 1.25× down, floor `fit`.
  - **fit / 1:1 toggle** — `object-fit: contain` ↔ `object-fit: none`.
  - **pin share** / **unpin share** (`pin`) — pins the share in the
    primary slot. Persists past the sharer stopping, showing
    "this share has ended" until unpinned.

### While sharing — sharer UI

- Own tile shows the screen being shared (doubles as preview).
- Controls strip toggle in active moss state.
- Persistent top-of-stage strip: "you are sharing · reading-list.md"
  (or window title), 24 px tall, `--bg-2`, `--ink-1`, with `x` to
  stop. Fades in without slide under reduced motion.

### Multiple shares

Grid stacks up to 2 shares in the spanning cell; beyond 2 a toast
offers grove. Focus cycles via the filmstrip (share peers carry a
screen-share glyph). Grove stacks 2 shares vertically in the side
panel.

## Speaking stats

Opened via overflow → "speaking time". Popover anchored to the
overflow button.

Layout:

- Header: `mic` icon + "speaking time" (display S italic) + right-
  aligned "last 5 min" mono meta.
- Body rows, sorted by speaking time desc: 22 px avatar · name
  (`body S`, 64 px) · bar (flex-grow, 4 px tall, `--bg-2` track,
  `--moss-2` fill) · time (`mono S`, 40 px right-aligned, `M:SS` or
  `H:MM`).
- Footer: `--line-soft` top border, diagnostics chips (bitrate,
  jitter, loss). Last line, `--ink-3`, `hint` type: "visible only to
  you".

Purely local: computed from the voice actor's VAD, not networked, no
event. The footer copy is load-bearing.

Window toggle (last 5 min / this call) deferred to v2.

## Ringing / joining / leaving

- **Call-start chime** when the local user joins: soft willow chime
  ≤ 1 s, ≤ −24 dB. Opt-in via **settings → notifications → call
  sounds**, default on. Chime is not gated by `prefers-reduced-motion`.
- **Join** — remote tile slides in with `willow-pop-in` (translateY
  +12 → 0, opacity 0 → 1, 240 ms). Grove places the tile at its
  deterministic position; grid / focus reflows remaining tiles with
  `var(--motion)`.
- **Leave** — opacity 1 → 0 over 180 ms, then 120 ms cell collapse;
  grove re-centres.
- **Live region** (`aria-live="polite"`): "{name} joined the call" /
  "{name} left the call". One message per event; no batching.

## Call header

52 px tall, `--line-soft` bottom border. Left to right:

1. **channel glyph** — `volume`, 16 px, `--ink-1`.
2. **grove name** (only when the call spans two groves) — `display S`
   italic, `--ink-1`.
3. **channel name** — `display S` italic, `--ink-0`. e.g. *the porch*.
4. **count + timer chip** — mono `{n} · {MM:SS}`, `--ink-2`.
5. **whisper pill** (conditional — see Whisper integration).
6. **now-speaking ticker** — avatar + name + 5 bars, updates every
   400 ms. Shows the last speaker within 2 s; hides when silent.
7. **spacer** (flex 1).
8. **sealed chip** — `lock` icon + mono `{n} peers`. Tooltip: "end-
   to-end encrypted with {n} peers"; appends `· {m} relay` when the
   relay is active. Opens the trust panel (see
   [`trust-verification.md`](trust-verification.md)).
9. **layout switcher** (desktop only; shares `call_layout` signal
   with the controls-strip switcher).
10. **close button** — `x`, 32 px hit box, `--ink-2`. Fires the same
    disconnect confirm as the bottom pill. If the user is sharing,
    first click shows "stop sharing and disconnect?" tooltip before
    proceeding.

## Mobile adaptations

- **Full-screen.** Grove rail and channel sidebar not rendered. The
  app's bottom tab bar hides while in a call.
- **Safe-area insets** — `padding-top: env(safe-area-inset-top)` on
  the header; `padding-bottom: env(safe-area-inset-bottom)` on the
  dock.
- **Auto-hide.** Controls strip + header fade out after 3 s of
  inactivity (no touch, no local audio). Tap anywhere on the call
  brings them back. Tapping a tile opens its context sheet
  (equivalent to right-click). Transition: `var(--motion)` opacity +
  8 px slide. Disabled when focus is inside the call surface
  (proxy for screen-reader / keyboard use).
- **Bottom sheets** for whisper, handoff, speaking stats, chat:
  `--bg-1`, `--radius-l` top corners, `--line` top border, 36×4
  grabber pill (`--ink-4`) 14 px from top, dismissable by swipe-down,
  tap-backdrop, or `Esc`.
- **Swipe-to-minimize** — gesture reserved, deferred to v2.
- **Touch targets** — controls strip buttons 50 × 50 with 9.5 px
  labels below.
- **Mobile dock** — only mic, cam, share, whisper, handoff, leave
  (red). Layout switcher, stats, participants, chat move into an
  overflow menu before leave.

## In-call chat panel

Inherits [`messaging.md`](messaging.md) wholesale; no novel copy.

- Desktop: right-side overlay, 360 px wide, slides in over tiles
  (tiles do not resize), 180 ms. `--bg-1`, `--line` left border.
- Mobile: bottom sheet, 70% stage height, `--radius-l` top corners.
- New-message indicator (red dot) on the chat button when closed.

## Copy (exact)

- `sealed call`
- `end-to-end encrypted with {n} peers`
- `end-to-end encrypted with {n} peers · {m} via relay`
- `you are sharing`
- `pin share` / `unpin share`
- `visible only to you`
- `disconnect`
- `hand off`
- `start a whisper`
- `start a whisper with {name}`
- `speaking time`
- `last 5 min` / `this call`
- `nobody here yet · share the grove link`
- `reconnecting…`
- `this share has ended`
- `mic blocked in browser settings`
- `camera blocked in browser settings`
- `{name} joined the call` / `{name} left the call`
- `stop sharing and disconnect?`
- layout names: `grove`, `grid`, `focus`
- control labels: `mute` / `unmute`, `deafen` / `undeafen`, `camera`,
  `share`, `whisper`, `hand off`, `chat`, `participants`, `more`,
  `disconnect`
- tile menu: `pin` / `unpin`, `mute for you` / `unmute for you`,
  `view profile`

No exclamation marks. No corporate verbs ("end call"). Leaving is
"leave"; terminating is "disconnect".

## Data dependencies

### Existing (no changes)

- Speaking indicator from the voice actor's VAD:
  `AppState.voice.speaking_peers`.
- Mute signalling: `voice_participants_map` + per-peer mute.
- Local mute / deafen / video source: `AppState.voice`.
- Media streams: `local_video_stream` + `remote_video_streams`.
- Layout signal: `AppState.ui.call_layout` (extend with `Grove`
  variant).
- Connection quality: `RTCPeerConnection.getStats()` sampled at 1 Hz
  per peer.

### New (non-breaking, UI-only or local-memory)

- **Pinned peer** — `AppState.ui.call_pinned_peer: Option<PeerId>`.
  Local.
- **Pinned share** — `AppState.ui.call_pinned_share: Option<PeerId>`.
  Local. Ephemeral to the call instance.
- **Speaking-time history** — 5-minute rolling window per peer id in
  the voice actor. Not persisted. Not networked.
- **Layout persistence** — `localStorage` key
  `willow.call.layout.<channel_id>`, value `"grove" | "grid" | "focus"`.

### Flag: no new events

No new `EventKind` variants are introduced. Whisper membership
continues to be owned by [`whisper-mode.md`](whisper-mode.md); this
spec consumes it only.

## Edge cases

- **Empty call (only me).** Centred column: grove-clearing gradient,
  local tile at bottom-centre, poetic copy "nobody here yet · share
  the grove link" + `copy invite` button.
- **Poor connection for one peer.** Quality chip amber. If loss
  > 10% for 5 s, tile border ring tints amber; tooltip "connection is
  choppy". No scale change (don't compete with speaker cues).
- **Network blip + auto-reconnect.** Tile dims to 70%, dashed 2 px
  `--amber` border, centred amber `reconnecting…` chip + spinner
  (static dot under reduced motion). Fades out over 600 ms on
  success. After 30 s without success, tile fades out as a leave.
- **Mic / cam permission denied.** Icon struck-through in `--err`,
  button disabled, tooltip "mic blocked in browser settings" /
  "camera blocked in browser settings". The user remains in the call
  (listen-only for mic block).
- **Screen-share cancelled.** No error; toggle simply stays
  inactive.
- **Layout reflow during share.** Grove → focus: share becomes focus
  primary automatically; others fill the filmstrip. Focus → grove:
  share collapses into the right-side panel, tiles re-form the
  clearing.
- **All peers leave.** Empty-state copy reappears after the last
  leave animation completes. The stage breathes.

## Accessibility

### Tile grid

- `role="grid"` with roving `tabindex` (0 on focused, −1 elsewhere).
  Arrow keys move focus between tiles.
- Each tile is a `button` with an accessible name
  `{display-name}, {status}` — status is a comma list of
  "speaking", "muted", "camera on", "sharing screen", "connection
  degraded", "reconnecting", "verified", "whispering".
- `Enter` / `Space` toggles focus layout with that tile as primary.
- `Shift+F10` (or context-menu key) opens the per-tile action menu.
- `Shift+Space` on a filmstrip tile (focus layout) pins / unpins.

### Live regions

- Join / leave announcements via `aria-live="polite"`.
- Speaker changes are **not** announced (too chatty). Screen readers
  pick up the status change via the focused tile's dynamic
  accessible name.
- Reconnect: "{name} reconnecting" at start, "{name} reconnected" on
  success. Polite, not assertive.

### Controls

- Icon-only buttons have `<span class="sr-only">` labels matching the
  Copy list.
- Toggle buttons use `aria-pressed`.
- Layout switcher is a `role="radiogroup"` with each segment
  `role="radio"` + `aria-checked`.
- Disconnect is a normal `button` with `aria-label="disconnect"`;
  requires `Esc` held 600 ms to confirm.

### Keyboard-shortcuts overlay

`?` (Shift+/) opens a modal listing:

- `M` mic · `D` deafen · `V` camera · `S` screen share · `L` layout
- `W` whisper · `H` handoff · `/` chat · `P` participants · `O` more
- `Tab` / `Shift+Tab` filmstrip focus
- `Shift+Space` on filmstrip tile pins / unpins
- `Esc` (hold 600 ms) disconnect

The overlay follows the modal contract in
[`layout-primitives.md`](layout-primitives.md). Initial focus on close;
`Esc` closes.

### Focus management

- Opening a popover / sheet moves focus to the first interactive
  element inside it; closing returns focus to the trigger.
- Focus is trapped in modals and bottom sheets, *not* in popovers
  (which dismiss on click-outside and `Esc`).

### Colour independence

Every state pairs colour with shape or icon:

- Muted: `mic-off` + red.
- Speaking: halo / border / bars (shape-based).
- Reconnecting: text + spinner + dashed border.
- Verified: check + moss.
- Whispering: ear + violet.

### Reduced motion

- Speaker scale → 1.0.
- Speaker glow → static border.
- Speaking bars → single static dot.
- Join slide → opacity fade only.
- `willowPulse` → static at full opacity.
- Mobile auto-hide disabled (controls stay visible).

### Contrast

All name-chip text against frosted-dark backdrop ≥ 4.5:1. Quality
chip (mono 10 px) ≥ 4.5:1 in amber and red states. Speaker border
ring visible against every accent-palette crest colour.

## Acceptance criteria

- [ ] `CallLayout` extended with `Grove`; default for new calls.
- [ ] Layout switcher in header and controls strip; both sync.
- [ ] `L` cycles grove → grid → focus → grove.
- [ ] Layout persisted per-channel in `localStorage`.
- [ ] Grove places tiles around a centred clearing; positions
      deterministic from peer-id hash; self near bottom-centre.
- [ ] Grid columns match the table; 16:10 target, 4:3 before
      scrolling.
- [ ] Focus primary selected by pin > share > speaker (1.2 s
      promote, 3 s hold) > most-recent > deterministic sort.
- [ ] Participant tile shows avatar fallback, name chip with
      verified check, muted badge, quality chip, whisper ear icon
      (only when peer is in the local user's whisper).
- [ ] Active-speaker treatment differs per layout: halo + scale
      (grove), border + bars (grid / focus).
- [ ] Tile context menu opens via right-click, long-press, or
      `Shift+F10`: pin, mute-for-you, view profile, start-whisper-
      with.
- [ ] Controls strip uses frosted glass over `--bg-1` with backdrop
      blur.
- [ ] All controls have keyboard shortcuts as listed.
- [ ] Screen share uses the native `getDisplayMedia` picker.
- [ ] Sharer's own tile shows "you are sharing" strip with stop
      affordance.
- [ ] Share canvas carries zoom-in, zoom-out, fit/1:1, and pin/
      unpin overlay controls.
- [ ] Pinned share holds the primary slot past the sharer stopping,
      with "this share has ended" until unpinned.
- [ ] Speaking-stats popover shows per-peer bars and time + footer
      "visible only to you"; no network traffic.
- [ ] No new `EventKind` variants.
- [ ] Whisper pill renders in the header when the local user is in
      an active whisper, using `--whisper` tokens.
- [ ] Handoff button pulses an `--amber` dot while a handoff is in
      progress.
- [ ] Empty call shows "nobody here yet · share the grove link" +
      `copy invite`.
- [ ] Sustained poor connection tints quality chip and border
      amber; reconnecting tile dims + shows chip + dashed border.
- [ ] Permission-denied shows struck-through icon + tooltip; button
      still retries on click.
- [ ] Mobile: full-screen, controls auto-hide after 3 s, tap brings
      back. Whisper / handoff / stats / chat use bottom sheets.
      Safe-area insets respected.
- [ ] In-call chat panel shows the channel's stream and inherits
      the messaging composer.
- [ ] Live region announces joins / leaves; never announces
      speaker changes.
- [ ] `?` overlay lists all shortcuts.
- [ ] Controls have `aria-label` / `aria-pressed`; tile grid has
      `role="grid"` + roving tabindex.
- [ ] Reduced motion collapses scale, glow, bars, join slide, and
      pulse animations.
- [ ] Disconnect requires `Esc` hold 600 ms; `Enter` on focused
      button fires immediately.
- [ ] All colours come from tokens in `foundation.md`; no ad-hoc
      values.

## Open questions

1. **Swipe-to-minimize (v2).** Floating pill vs dock into the app's
   own tab bar? Leaning: app dock, since mobile-web PiP needs a
   video element and audio-only calls would miss out.
2. **Grove above 8.** Auto-switch to grid at 8, or scale the
   clearing radius and keep grove viable to 12? Needs visual test.
3. **Speaker debounce tuning.** 1.2 s / 3 s lifted from Discord;
   revisit once telemetry exists.
4. **Pin across rejoin.** Pinning survives a layout switch but not
   the call ending; pin is ephemeral. Confirm this feels right.
5. **Audio device hot-swap.** Auto-switch when headphones connect
   mid-call, with a 5 s "switched to AirPods Pro · undo" toast?
   Needs settings plumbing.
6. **Cross-grove bridge calls.** Header structure supports two
   grove names; no blocker, just needs product direction.
7. **Raise hand / reactions (v2).** Is raise-hand a call-local
   ephemeral signal or a messaging-layer reaction? Not blocked.
8. **Recording / captions.** Deferred. Would need a consent
   ceremony grounded in [`trust-verification.md`](trust-verification.md).
