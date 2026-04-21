# Ephemeral channels — timer, expiration, key burn

**Parent:** [README.md](README.md)
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md)
**Status:** draft

## Purpose

Ephemeral channels are channels with a declared, finite lifespan. When
the timer reaches zero, the channel key is burned on every participating
device; prior message bodies become unrecoverable; the channel is
removed from the sidebar; and the history left on any archival peer is
cryptographically opaque. The feature exists so members can have
conversations that are *durably forgettable* — gatherings, bookclubs,
night sessions, after-hours vents — without needing to trust that
someone will delete a log later.

Ephemeral is a first-class channel kind, not a retention policy. The
UX must make the lifecycle legible at every moment: how long is left,
what happens at zero, who can extend, and what extension can and
cannot undo. The surface borrows clock-face vocabulary — serifs,
lock-screen digits, amber warmth — not alarm-system vocabulary.
Finality is communicated through motion and typography, not red alerts.

## Scope

In scope:

- Channel-sidebar row for ephemeral channels, with trailing time chip.
- Fixed banner inside ephemeral channels, above the message list.
- Mobile full-screen ephemeral timer (lock-screen style) during
  countdown, and as a preview in the creation sheet.
- Creation flow (kind selector + duration presets + custom up to 30 d).
- Extension flow (`ManageChannels` permission, once per channel, bounded
  by the original duration).
- Destruction lifecycle: T-5 min pulse, T-1 min "say goodbye" toast,
  T-0 burn transition + leaf-fall, return to grove home.
- Discovery / archive behaviour (not listed, not archivable).
- Copy strings, edge-case copy, accessibility announcements.

Out of scope:

- Backend event shapes beyond what the UX depends on (state team owns
  exact wire format; this spec lists data dependencies only).
- Ephemeral letters / DMs. Ephemeral is a *channel* concept in v1;
  ephemeral letters are a future extension.
- Per-message self-destruct (distinct feature, not covered here).
- Media retention inside ephemeral channels beyond "same lifetime as the
  channel key" — images and files are sealed with the same key and are
  burned together with it.

## Sidebar row

Ephemeral channels live in their own sidebar group, ordered
immediately below "voice" and above "archives". Group header copy is
the literal string `ephemeral`, using the `meta` type scale from
[`foundation.md`](foundation.md) (IBM Plex Sans, 11.5 px, uppercase,
tracked +1.2). The group label colour is `--whisper` to distinguish it
from the default `--ink-3` label — a deliberate echo of the "finite
and private" semantic whisper also carries. A small `hourglass` icon
(11 px, stroke 1.4, `currentColor` inheriting `--whisper`) sits at the
trailing edge of the group header.

Row anatomy, left to right:

1. `hourglass` icon, 14 px, stroke 1.6, colour `--amber`. Icon never
   changes weight based on timer phase — the row itself stays
   approachable; the chip carries urgency.
2. Channel name in `--font-ui`, body size, weight 400 when read and
   500 when the row has unread messages. Colour `--ink-1` normally,
   `--ink-0` when unread.
3. Flex spacer.
4. Trailing amber pill (`--radius-s`, 1 px border `--amber-soft`,
   background `transparent`, foreground `--amber`, padding `2px 6px`).
   Pill contents: a 9 px `hourglass` and the time string.

Time-string format rules:

- Greater than 24 h: show days + hours, rounded down. `"5d"` or
  `"5d 3h"` (days only once over 48 h to avoid oscillation).
- 1 h to 24 h: show hours + minutes. `"2h 14m"`.
- 1 min to 60 min: show minutes. `"42m"`.
- Under 1 min: show seconds every 10 s boundary. `"40s"`, `"30s"`,
  `"20s"`. Not every second — see accessibility.
- At or past zero: row is removed (see Destruction lifecycle).

The pill is *subtle* at normal phase. Under one hour remaining, the
pill gains a `box-shadow: 0 0 0 2px rgba(214, 165, 74, 0.18)` glow
using `--warn`. The glow is fixed, not animated (ambient warning,
not urgent alarm).

Hover state on desktop: entire row uses `--bg-3`, pill background
becomes `rgba(201, 155, 85, 0.10)`. Active / selected state uses
`--bg-4` with the same pill glow treatment independent of remaining
time, so the user can always see the timer clearly on the current
channel.

Mobile rendering (see [`m_home.jsx`](…/reference) for the canonical
shape) collapses the row to a single-line chip-trailing layout.
Touch target is the full row, ≥ 44 × 44 CSS px.

## Channel header banner

When the user is inside an ephemeral channel, the standard channel
header (title, members, call / threads / pin buttons) is followed by
a fixed banner that occupies a second header row. Banner anatomy:

- Height: 32 px desktop, 36 px mobile.
- Background: `--bg-1` with a 1 px bottom border using `--line`.
- Left: a 13 px `hourglass` icon in `--amber`, `gap: 8px`.
- Copy: `"ephemeral — keys will be burned in {time}"`.
  Copy uses `--font-ui`, body S, colour `--ink-1`. The time string
  uses `--font-mono` for the number and unit cluster, inherited
  colour. The em-dash is a real `—`, not a hyphen.
- Right: a quiet `more-horizontal` button (members with
  `ManageChannels` see "extend"; others see no trailing action).

Under-10-minute phase:

- Border changes from solid `--line` to dashed `--amber-soft`, 1 px.
- Copy changes to `"expires soon — keys burned at {clock}"`, where
  `{clock}` is the local-time rendering of the expiry instant in
  24-hour format with a leading zero if needed (`"10:42"`, `"03:07"`).
  No seconds.
- The banner itself does not pulse in this phase; the mobile timer
  and the T-5 toast carry that weight.

T-5 min phase (banner pulse):

- A very soft `box-shadow` crossfade animation keyed to
  `--motion-ambient` (1200 ms), alternating between the standard
  `--shadow-1` and a `box-shadow` that adds `0 0 24px -8px var(--warn)`.
- Under `prefers-reduced-motion: reduce`, pulse is replaced by a
  static shadow at the midpoint opacity — no animation.

The banner never scrolls away; it is part of the chrome, not the
message stream.

## Mobile ephemeral screen

The mobile ephemeral surface is used in two places:

1. **Creation preview**, driven from the "new channel" flow when kind
   is `ephemeral`. A lock-screen-style clock renders the duration
   currently selected as hours, so the user *sees* the lifespan before
   confirming.
2. **Active countdown view**, shown when a user taps the banner of an
   ephemeral channel or expands the timer from the `more-horizontal`
   menu. Lets the user read the timer in a distraction-free surface.

Layout (mirrors [`m_ephemeral.jsx`](…/reference) in the bundle):

- Background: `--bg-0`.
- Top bar: transparent, no bottom border. Left = chevron-back.
  Centre title: a 14 px `hourglass` in `--amber` followed by the
  italic display-S copy `"ephemeral channel"` in `--amber`.
- Subtitle under the title: `"self-destructs · keys erased after"`
  in `hint` type (IBM Plex Sans 10.5 px, `--ink-3`).
- Centre block, vertically positioned with generous padding (20 px
  top, 32 px side):
  - Clock numerals. Fraunces italic, 72 px on phones with viewport
    ≥ 360 px wide, scaled to 64 px on narrower devices. Line-height
    1. Letter-spacing −2. Colour `--amber` for HH and MM; the colon
    separators, and the trailing seconds cluster (`":SS"` at 32 px),
    use `--amber-soft` so the "live" fields read stronger than the
    supporting digits.
  - HH always has two digits. When expiry is < 1 h, HH is `"00"`; the
    numerals stay visible. When expiry is < 1 min, MM shows `"00"` and
    only SS ticks.
  - Below the clock, a mono label in `meta` style: `"until keys burn"`,
    `--ink-3`, uppercase, tracked +1.8.
  - Below the label, a 180 × 10 px progress bar: track `--bg-2`,
    fill `--amber`, with 0 / 25 / 50 / 75 / 100 percent ticks drawn
    in `--line`. The fill shrinks over time (left-anchored, 100 % at
    creation, 0 % at burn). This is the only timer element that
    updates more frequently than the numerals (once per second);
    the numerals update on minute boundary except inside the final
    minute.

Ambient motion — the leaf fall:

- Tiny drifting `leaf` particles (the foundation keyframe `leafFall`)
  fall vertically through the screen from `translateY(-12vh)` to
  `translateY(120vh)` over 9–14 s per particle (randomized), with
  `rotate(-8deg → 24deg)` and opacity `0 → 1 → 0`.
- Particle density: 3 simultaneous on mobile, 5 on desktop if the
  timer is rendered inside a mobile-preview surface on a wide
  viewport. Particle colour is `--amber` at 35 % alpha.
- As the timer nears zero (< 5 min), particle density rises to
  6 on mobile, 10 on desktop, and fall duration shortens to 6–9 s.
- Particles are decorative. They carry `aria-hidden="true"` and
  `pointer-events: none`.
- Under `prefers-reduced-motion: reduce`, particles are not rendered
  at all (not a fade-collapse; fully omitted — the keyframe is
  motion-only and has nothing to degrade to). The clock and the
  progress bar remain.

Below the centre block (creation surface only), a card with the
channel name input (mono font), duration preset grid (`1h / 6h / 24h
/ 3d / 7d`), and a dashed-amber key-forge callout:

> a single-use key is forged now and burned when the timer ends.
> messages can't be recovered.

A primary "open #{name}" button uses the `--amber` fill with
`--bg-0` text; a secondary "cancel" uses `--ink-3` colour only,
no background.

## Creation flow

Entry points:

1. Sidebar `+` button (desktop) → "new channel" popover, kind
   selector offers `text`, `voice`, `ephemeral`.
2. Mobile FAB on the home screen → bottom-sheet with the same kind
   selector.

Steps:

1. **Name.** Standard channel-name input, lower-kebab-case enforced,
   max 32 characters. Mono input font for ephemeral channels to
   reinforce the key-forge metaphor.
2. **Kind.** Three segmented chips. Selecting `ephemeral` reveals:
3. **Duration.** Five presets as equal-width buttons: `1h`, `6h`,
   `24h`, `3d`, `7d`. A sixth "custom…" tile opens a secondary
   surface: a stepper (hours + days) with a cap of 30 days.
   Selecting custom anchors the duration on submission; it is not
   editable after creation.
4. **Warning block.** A persistent dashed-amber callout reads:
   > this channel will self-destruct in {duration} — keys burned on
   > every device.

   Immediately beneath, a secondary explainer carries the bundle's
   load-bearing crypto framing verbatim:
   > a single-use key is forged now and burned when the timer ends.
   > messages can't be recovered.

   The two sentences coexist — the first states the outcome; the second
   (from `m_ephemeral.jsx`) teaches the mechanism.

   The `{duration}` token is a human phrase: `"1 hour"`, `"6 hours"`,
   `"1 day"`, `"3 days"`, `"1 week"`, or for custom `"N hours"` /
   `"N days"` depending on magnitude.
5. **Confirm.** The primary CTA carries an `hourglass` icon: `open #{name}`.
   Hitting confirm creates the channel (new state event, see Data
   dependencies) and routes the user into it.

If the user lacks `ManageChannels` in the current grove, the
ephemeral option is disabled with a `hint`-type tooltip: `"only
grove stewards can create ephemeral channels"`.

## Extension

Only peers holding `ManageChannels` may extend an ephemeral channel.
The extend affordance is:

- Desktop: inside the banner's `more-horizontal` menu, entry `"extend…"`.
- Mobile: on the full-screen timer view, a secondary button `"extend…"`
  appears below the clock block when the viewer has permission.

Extension is a **one-time operation per channel.** Once the channel
has been extended, the affordance is hidden for everyone, including
the original extender, and subsequent attempts are rejected at the
state layer. If a second steward tries to open the menu, the entry
is replaced by a disabled `hint` label `"already extended — cannot
extend again"`.

Confirmation surface (modal on desktop, bottom sheet on mobile):

- Title: display M, `"extend this ephemeral"`.
- Body copy:
  > add time to this ephemeral — this resets the timer but does not
  > restore messages if any keys have rotated since creation.
- Duration picker: the same preset grid, but **capped at the
  original duration.** If the channel was created with `6h`, the
  picker shows `1h / 2h / 4h / 6h` and dims `24h / 3d / 7d`. Custom
  values are allowed up to the original duration.
- Confirm button copy: `"extend by {duration}"`.

On confirmation the countdown resets to `now + chosen duration`, the
banner and sidebar pill update within the next render tick, and a
toast announces `"timer extended — new burn at {clock}"` in the
standard toast slot. The toast uses `--warn` accent (not `--ok`),
because extension is a deliberate delay of destruction, not a
success state.

If the original duration was custom and very short (< 1 h), the
cap still applies. A custom 20-minute channel can be extended by up
to 20 minutes, once.

## Destruction lifecycle

The countdown is computed from the HLC creation timestamp + duration
recorded at channel creation. Every client derives expiry independently
from the event's stored values; no "countdown broadcast" is needed.
See Edge cases for clock skew handling.

Phase table:

| Phase         | Trigger | UX change |
|---------------|---------|-----------|
| normal        | `> 1h` remaining | Sidebar pill plain; banner solid border. |
| warn          | `≤ 1h` remaining | Sidebar pill gains `--warn` glow. Banner still solid. |
| near          | `≤ 10 min` remaining | Banner switches to dashed `--amber-soft` + "expires soon" copy. Mobile particle density rises. |
| pulse         | `≤ 5 min` remaining | Banner pulses via `--motion-ambient` shadow animation. Polite announce: `"ephemeral {name} expires in five minutes"`. |
| final minute  | `≤ 1 min` remaining | Toast appears: `"this channel will burn in 1 minute — say goodbye"`. Toast uses `--warn` accent and persists the full minute. Banner pulse continues. SS now updates on-screen in the mobile view. |
| burn          | `= 0` | See below. |

Burn transition (T-0):

1. Channel is immediately removed from the sidebar list. Any pinned
   entries referencing the channel are scrubbed.
2. If the user is currently inside the channel, the message list,
   composer, and member list fade out over 180 ms (`--motion`).
3. A single centred card replaces the main pane:
   - Serif display M italic: `"keys burned"`.
   - Body line: `"conversations cannot be recovered"`.
   - Subtle colour: `--ink-2` on `--bg-0`.
4. A brief leaf-fall burst plays for ~2.5 s over the card: 12 to
   16 particles falling, `--amber` at 55 % alpha, duration 3.5 s
   each. Under reduced motion this burst is omitted entirely; the
   card simply appears.
5. After 3.5 s, or immediately on user tap / keypress, the user is
   navigated back to the grove home. No modal dismiss button is
   needed — the experience is meant to end gently on its own.

The burn surface is shown once, on the device that was viewing the
channel at T-0. On devices that were not viewing, the sidebar row
simply disappears on next render; navigating to the old URL returns
the user to the grove home with no error toast.

## Discovery policy

Ephemeral channels must never appear in Discover or public grove
directories, regardless of grove discovery settings. The contract:

- The [`discover.md`](discover.md) spec cannot list ephemeral
  channels in directory cards, counts, or previews.
- Grove pages on third-party share surfaces (invite previews, crest
  pages) list only non-ephemeral channel counts.
- Invites to an ephemeral channel are **not directory events**; they
  are key-derivation events — the inviter computes a one-off
  derivation and sends it directly. The receiving client treats the
  ephemeral channel as a first-class member of the grove only after
  the key has been installed.
- Ephemeral channels cannot be pinned in the Discover favourites
  list.
- Ephemeral channels cannot be archived. Attempting to archive
  surfaces the `hint`-type copy: `"ephemeral channels cannot be
  archived"`. The sidebar "archive" affordance is hidden entirely
  for ephemeral rows.

## Copy (exact strings)

Use these literally; do not paraphrase.

| Context | String |
|---------|--------|
| Banner, normal | `ephemeral — keys will be burned in {time}` |
| Banner, under 10 min | `expires soon — keys burned at {clock}` |
| Creation warning | `this channel will self-destruct in {duration} — keys burned on every device.` |
| Creation key callout | `a single-use key is forged now and burned when the timer ends. messages can't be recovered.` |
| Final-minute toast | `this channel will burn in 1 minute — say goodbye` |
| Burn card title | `keys burned` |
| Burn card body | `conversations cannot be recovered` |
| Extend body | `add time to this ephemeral — this resets the timer but does not restore messages if any keys have rotated since creation.` |
| Extend button | `extend by {duration}` |
| Extend toast | `timer extended — new burn at {clock}` |
| Already-extended hint | `already extended — cannot extend again` |
| Archive refusal | `ephemeral channels cannot be archived` |
| Offline-at-burn copy | `keys burned on all online devices — will burn on others when they reconnect` |
| Creator tooltip, insufficient permission | `only grove stewards can create ephemeral channels` |
| Group label | `ephemeral` |
| Self-destruct tagline (group header, mobile only) | `self-destructs` |

`{time}` formatting rules are listed in "Sidebar row". `{clock}`
uses local-time 24-hour `HH:MM`. `{duration}` uses the human
phrasing listed in Creation flow.

No exclamation marks anywhere. All copy lowercase except where the
copy itself contains proper nouns (none in this spec).

## Data dependencies

Required from `willow-state`. Items marked **new** are new event
kinds; items marked **extend** reuse existing kinds with new fields.

- **extend `ChannelCreate`** — add an optional `EphemeralConfig`
  payload: `{ duration_ms: u64, created_at_hlc: HLC, custom: bool }`.
  Absence of the payload means a regular channel. The `custom` flag
  is UI-only (it controls the preset-vs-custom label in the banner
  menu) and does not affect state semantics. State team may reject
  `duration_ms` outside `[60_000, 30 * 24 * 3600 * 1000]` (1 minute
  to 30 days).
- **new `ChannelExpiryTick`** — (deferred / possibly unnecessary)
  A periodic marker that materialization uses to prune ephemeral
  channels from the computed `ServerState`. If the state machine
  can derive expiration purely from `ChannelCreate.created_at_hlc +
  duration_ms` against the merge frontier's current HLC, then no
  tick event is required. The UX does not depend on which
  implementation is chosen — **flag as open question to state team:
  is `ChannelExpiryTick` a new `EventKind`, or does materialize
  compute expiry from `ChannelCreate` alone?**
- **new `ChannelExtend`** — emitted by a peer with
  `ManageChannels`, payload `{ channel_id, added_ms: u64 }`, with
  state-level enforcement that (a) at most one `ChannelExtend` is
  accepted per channel, and (b) `added_ms ≤ original duration_ms`.
  Idempotency: duplicate `ChannelExtend` events for the same channel
  are rejected at `apply()`; the UI surfaces this via the
  `"already extended"` hint.
- **key derivation** — the per-channel symmetric key derives from
  grove key material plus the channel id and the creation HLC. Key
  burn is implemented as the per-device action of scrubbing the
  derived key and any message-body cache keyed by it. The UI does
  not broadcast a burn event; burn is a client-local reaction to
  the creation-HLC + duration elapsing. See `willow-crypto` for the
  exact derivation (out of scope here).

Permissions:

- `CreateChannel` (already exists) required to create any channel,
  ephemeral or not.
- `ManageChannels` (already exists) required to extend an ephemeral.
- No new permission is introduced for this feature.

## Edge cases

**Participant offline at destruction.** Online devices remove the
channel from their sidebars at T-0. Offline devices retain the
derived key until next reconcile; they compute that the HLC has
passed the burn instant and scrub the key locally. While known-
offline members remain, the banner carries a meta line `"keys
burned on all online devices — will burn on others when they
reconnect"`, which disappears once reconciliation confirms all
members present at creation have caught up beyond the burn HLC.

**Ephemeral parent with active threads.** Threads nested under an
ephemeral channel inherit the parent's expiry. Thread creation
inside an ephemeral channel uses the same channel key derivation,
so thread key burn is automatic when the channel key is burned.
This inheritance is communicated at creation time via the creation-
flow warning: threads are implicitly included in the "self-destruct"
statement; no separate thread-specific copy is needed. If a thread
is open at T-0, the thread pane transitions to the same `"keys
burned"` card and closes after 3.5 s, returning the user to the
grove home (not to the old parent, which no longer exists).

**Clock skew.** Timers use the HLC recorded in `ChannelCreate`,
not local wall-clock. Display is derived as `creation_hlc.physical
+ duration_ms - local_hlc.physical()`, where `local_hlc` is the
reader's HLC. This means two devices with drifted system clocks
still agree on the remaining time to within HLC granularity, and
the burn event happens *deterministically on every device* at the
same logical moment. Display clocks (`{clock}` in copy) render
the HLC's physical millisecond converted to the device's local
timezone.

**User leaves then rejoins before burn.** Rejoin is a normal member-
add; the channel key is re-delivered via key derivation. The banner
resumes showing the HLC-derived remaining time. No catch-up copy.

**User created channel with `1h` custom then extended once by 30 m.**
After extension, future UI states read `"already extended — cannot
extend again"`. The extension cap (≤ original) permits any value
from 1 minute up to 60 minutes in this case; `1h` is the ceiling.

**Timer reaches zero while the channel is unfocused on a desktop
with many tabs open.** The sidebar row disappears on next render;
no notification is surfaced, because a notification reading
"something you can no longer read has ended" would be worse than
silence. If the user navigates to the channel URL after burn, they
land on the grove home.

**Duration of 1 min (minimum).** The full phase table collapses:
`normal` phase is skipped; the channel opens directly in `near`
phase. The creation confirmation surface additionally warns:
`"this channel will burn within a minute of opening."` (hint-style,
below the main warning).

## Accessibility

- The sidebar pill is announced as `"ephemeral — {time} remaining"`
  via `aria-label`, where `{time}` uses the same phrasing shown
  visually (`"2 hours 14 minutes"` rather than `"2h 14m"`, to avoid
  abbreviated screen-reader speech).
- The channel banner is a live region: `role="status"`,
  `aria-live="polite"`. It re-announces on every minute boundary
  only. Per-second updates would be hostile.
- The "near" phase (≤ 10 min) additionally announces once on entry:
  `"{name} expires in ten minutes"`. The "pulse" phase announces
  `"{name} expires in five minutes"`. The final-minute toast is a
  `role="alert"` with `aria-live="assertive"` and copy `"this
  channel will burn in 1 minute — say goodbye"` — the one place a
  more urgent live region is appropriate.
- The mobile clock numerals are wrapped in a visually-hidden span
  that reads the time in words once per minute (`"2 hours 14
  minutes remaining"`), so users navigating with a screen reader
  do not hear `"0 2 colon 1 4 colon 0 9"`.
- All leaf-fall particles carry `aria-hidden="true"` and are omitted
  entirely under `prefers-reduced-motion: reduce`.
- Colour is never the only cue for a state. Ephemeral = `--amber` +
  `hourglass` icon + italic display type. The "near" phase adds a
  dashed border on top of the colour change. The "pulse" phase adds
  a shadow on top of the border change.
- Keyboard path: every affordance (sidebar row, banner menu button,
  extend menu entry, creation flow, confirm button) is reachable
  via tab order, with `--focus-ring` from [`foundation.md`](foundation.md).
- Touch targets on mobile meet the ≥ 44 × 44 CSS px baseline. The
  trailing time pill on a mobile channel row expands its hit box to
  the full row; the pill itself is purely decorative for touch.

## Acceptance criteria

- [ ] Sidebar ephemeral group renders with `--whisper` label colour
      and a trailing `hourglass` icon in the group header.
- [ ] Rows show the `hourglass` icon in `--amber`, the channel name,
      and a trailing time pill with the correct formatting across
      phase boundaries (24h, 1h, 1m boundaries).
- [ ] Under one-hour remaining, the pill gains a `--warn` glow that
      does not animate.
- [ ] Banner renders with correct copy at normal, near, and pulse
      phases. Dashed border appears at ≤ 10 min. Pulse shadow
      animation respects reduced motion.
- [ ] Mobile lock-screen timer renders HH:MM:SS in Fraunces italic,
      with SS at 32 px using `--amber-soft`, plus a progress bar
      with 0/25/50/75/100 % ticks.
- [ ] Leaf-fall particles render with `leafFall` keyframe, omitted
      under reduced motion.
- [ ] Creation flow offers `1h / 6h / 24h / 3d / 7d` presets and
      custom up to 30 d, shows the self-destruct warning, and
      confirms with `open #{name}`.
- [ ] Extension is gated by `ManageChannels`, allowed at most once,
      and capped at the original duration.
- [ ] T-0 burn transition: sidebar row disappears, open view shows
      the `"keys burned"` card for 3.5 s, user returns to grove
      home.
- [ ] Ephemeral channels never appear in Discover listings.
- [ ] Archive affordance is hidden for ephemeral rows; archive API
      returns the refusal copy when called programmatically.
- [ ] Screen-reader announcements fire on minute boundaries, not
      every second, and the final-minute toast uses
      `aria-live="assertive"`.

## Open questions

- **`ChannelExpiryTick` event or pure derivation?** Does materialize
  compute expiry from `ChannelCreate.created_at_hlc + duration_ms`
  against the HLC frontier, or is an explicit tick event needed for
  merge determinism? — State team.
- **Key-burn persistence.** Should burned keys be remembered in an
  append-only "ghosted channels" index to reject re-creation with a
  colliding id? Likely unnecessary (channel ids include HLC).
- **Extension count.** v1 ships with "once per channel". Per-grove
  governance override is deferred — finality is the point.
- **Calendar-style expiry.** Rejected for v1 in favour of durations;
  revisit if users ask.
- **Ephemeral letters (DMs).** Out of scope for v1. If added later,
  [`letters-dms.md`](letters-dms.md) will depend on this spec.
