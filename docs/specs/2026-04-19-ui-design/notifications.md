# Notifications — toast + badge + push + sound rendering contract

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md),
[`layout-primitives.md`](layout-primitives.md)

**Consumed by:** [`message-row.md`](message-row.md),
[`composer.md`](composer.md),
[`whisper-mode.md`](whisper-mode.md),
[`device-handoff.md`](device-handoff.md),
[`ephemeral-channels.md`](ephemeral-channels.md),
[`sync-queue.md`](sync-queue.md),
[`letters-dms.md`](letters-dms.md),
[`call-experience.md`](call-experience.md),
[`governance.md`](governance.md)

## Purpose

Every feature that wants to get the user's attention — a new message,
a whisper invitation, an ephemeral channel about to burn its keys, a
handoff request from another of your devices, a queued delivery —
renders through *one* notification surface. This spec owns that
surface. Settings → Notifications (see `settings-tweaks.md` §4) owns
the *preferences*; this spec owns the *delivery contract* the rest of
the app consumes.

The governing principle is *trust-first, but calm*: notifications
never leak plaintext off-device, respect quiet hours, coalesce when
the user is already reading, and refuse to chime on own-events. A
notification in Willow is an atom — small, serifless, decidable — not a
banner that brags.

## Scope

In scope:

1. **In-app toast / banner** — transient surface with icon + title +
   optional body + optional action; stacks, auto-dismisses, persists
   on action.
2. **Unread badge rendering** — the moss pill on grove tiles, channel
   rows, letter rows, mobile tab-bar tabs, and its mention / whisper /
   muted variants.
3. **OS push notification payload** — what leaves the device, what does
   not, and how visible notifications are composed after local
   decryption.
4. **Sound playback** — default willow chime, per-event variants,
   global mute, quiet hours, own-event suppression.
5. **Per-surface mute rules** — grove / channel / letter mute, and the
   overrides that bypass them for safety-critical events (ephemeral
   expiry, handoff, whisper invitation).

Out of scope: the Notifications *settings section* (`settings-tweaks.md`
§4); the sync-queue offline strip (`sync-queue.md`); Web Push
service-worker plumbing and VAPID registration wire format (tracked in
a follow-up state/network spec; this spec defines only the *payload
shape* and *privacy contract*).

## Toast / banner anatomy

### Surface

A rounded-corner card rendered in an overlay portal at `z-index: 80` —
above popovers (60), below modals (100). It is not a panel: it does
not reserve layout space.

| Platform | Position |
|----------|----------|
| Desktop  | Bottom-centre, 24 px from bottom edge. Stacks upward. |
| Mobile   | Bottom-centre, 12 px above the tab bar (or 24 px above bottom safe-area when tab bar is hidden). Stacks upward. When the mobile keyboard is open the stack anchors to the top of the keyboard. |

### Anatomy

```
┌──────────────────────────────────────┐
│ [icon] title                   [x]   │
│        optional body line             │
│                          [ action ]  │
└──────────────────────────────────────┘
```

- Container: `--bg-1` bg, `--line` border, `--radius-l`, `--shadow-2`.
  Min width 280, max 420 on desktop; `calc(100vw - 32px)` max 420 on
  mobile. Padding 12 / 14; 8 px gap between stacked toasts.
- Icon: 14 px, stroke 1.5. Optional for `info`.
- Title: body S (13 px / IBM Plex Sans / 500), `--ink-0`, single line,
  truncates.
- Body (optional): body S / 400, `--ink-1`, up to two lines, truncates.
- Close `x`: 12 px, `--ink-3`, top-right inset 8,
  `aria-label="dismiss"`.
- Action button (optional): pill, 28 px tall, `--moss-2` fill,
  `--ink-on-accent` text, 11 px label. Max one action per toast —
  except the two-choice whisper / handoff toasts noted in Copy.

### Severity

| Severity  | Icon         | Icon colour | Border          | ARIA role  | Live region |
|-----------|--------------|-------------|-----------------|------------|-------------|
| `info`    | *none*       | —           | `--line`        | `status`   | polite |
| `success` | `check`      | `--ok`      | `--line`        | `status`   | polite |
| `warn`    | `hourglass`  | `--warn`    | `--amber-soft`  | `alert`    | assertive |
| `err`     | `x`          | `--err`     | `--err`         | `alert`    | assertive |

No exclamation marks, no shake / pulse motion, no red fill. Severity
reads through icon + border accent only.

### Persistence

- No action → auto-dismiss at **4 s**.
- With action → persists until the action is taken, `x` / `Esc` is
  pressed, or the underlying state resolves (e.g. handoff withdrawn).
- Callers may mark a toast `sticky: true` (warn / err that need
  acknowledgement).
- Hovering pauses the auto-dismiss timer; leaving resumes with the
  remaining duration.

### Stacking

Max **3 visible**. A fourth arrival collapses the oldest into a
"**{n} more**" pill (24 px tall, `--bg-2` / `--ink-3`, centred at the
top of the stack). Activating the pill opens an overflow surface
(desktop: floating pane 320 × 400 at the same placement; mobile:
bottom sheet per `layout-primitives.md`) listing every active toast.
When overflow drops to ≤ 3, the pill vanishes.

### Motion

All motion uses the foundation easing `cubic-bezier(0.2, 0.8, 0.2, 1)`.

| Event           | Animation                                   | Duration |
|-----------------|---------------------------------------------|----------|
| Enter           | `willow-pop-in` (opacity 0→1, translateY +6→0) | 180 ms |
| Exit            | opacity 1→0, translateY 0→+4                | 120 ms |
| Stack shift     | translateY only                             | 180 ms |
| Collapse-to-pill | fade 1→0 then pill pops in                 | 120 + 180 ms |

`prefers-reduced-motion: reduce` collapses everything to opacity:
120 ms fade-in, 120 ms fade-out, instant stack shift, instant pill
swap.

### Programmatic contract

Callers do not build DOM. The toast service exposes:

```rust
struct Toast {
    id: ToastId,
    severity: Severity,          // info | success | warn | err
    title: String,               // required, ≤ 80 chars
    body: Option<String>,        // ≤ 140 chars
    icon: Option<IconName>,      // overrides severity default
    action: Option<ToastAction>, // at most one, with whisper / handoff exception
    sticky: bool,                // default false
    dedup_key: Option<String>,   // latest wins; replaces prior toast with same key
}
```

`dedup_key` is how coalesced messages render as a single toast whose
body updates ("3 new messages in #meander") and whose auto-dismiss
timer resets.

## Unread badge rendering

### Anatomy

- Shape: pill. Height 18 px (desktop), 16 px (mobile tab bar). Radius
  `--radius-s`. Min width = height (circle for single digits).
- Default fill: `--moss-2`, foreground `--ink-on-accent`.
- Typography: mono S (10.5 px / JetBrains Mono / 500), numerals
  tabular. A number, not chrome.
- Padding: 0 6 px multi-digit; centred circle for single digit.
- Max count: **99+** (greater counts render as `99+`).

### Variants

| Variant              | Fill           | Foreground           | Use |
|----------------------|----------------|----------------------|-----|
| default              | `--moss-2`     | `--ink-on-accent`    | Unread on an un-muted surface. |
| muted (outlined)     | transparent    | `--ink-3` (1 px `--ink-3` border) | Grove / channel / letter marked muted. Count still rendered. |
| whisper              | `--whisper`    | `--ink-on-accent`    | Letter row whose latest unread is whisper-marked. |
| mentioned            | `--moss-2`     | `--ink-on-accent`    | Count prefixed with a 10 px `@` glyph (500 weight) 2 px left of the digits. |
| sealed-announce-only | `--bg-3`       | `--ink-2`            | Channel with only governance events unread (role change, pin). Lower visual weight so noise doesn't read as chat. |

Priority when multiple conditions hold: whisper > mentioned >
announce-only > muted > default. A whisper letter with a mention
renders whisper-violet with the `@` glyph.

### Placement per surface

| Surface                        | Position | Notes |
|--------------------------------|----------|-------|
| Grove tile (rail)              | Top-right of the 44 × 44 tile, offset 2 px out. | One pill per grove; aggregates across channels + letters in scope. |
| Channel row (sidebar)          | Right-aligned, 12 px from row edge. | Rendered only when `unread > 0`. Matches current `sidebar.jsx`. |
| Letter row (letters list)      | Right-aligned, 12 px from row edge. | Whisper variant applies when latest unread is whisper-marked. Sync-queue `queued: N` marker is a *separate* atom on the row — do not merge. |
| Mobile tab-bar tab (idle)      | 6 × 6 moss dot at the top-right of the icon. | Dot only — chrome is space-constrained. |
| Mobile tab-bar tab (long-press / focus) | Number-bearing pill replaces the dot, 12 px left of the icon. | Dot scales out, pill pops in; reduced-motion: instant swap. |
| Group-letter header (in chat)  | Inline after title, mono S, `--ink-3`. | Text ("n unseen"), not a pill — the pill is reserved for list surfaces. |

### Mentions and sort priority

A mention bumps unread by 1 *and* switches the row variant to
`mentioned`. Rows sort by `(mentioned desc, whisper desc,
latest_unread_ts desc)`. Mark-as-read demotes the row. Muted surfaces
still receive the mentioned variant — mute suppresses *notifications*
(toast + push + sound), not *attribution*.

### Grove-tile aggregation

The grove tile's count sums unread across every channel and letter in
scope (cap `99+`) and inherits the highest-priority variant present
inside. The grove tells you a whisper is waiting on your own rail
(private, but your own presence is allowed to know); peers never see
this.

### Muted surfaces

A muted grove still shows the tab-bar dot when unread exists — "there
is something here" is not itself a notification. The grove tile shows
the outlined muted badge with count. Toasts, push, and sound are
suppressed.

### Own-originated events

Own-send never bumps unread. Enforced at the client before the badge
signal is emitted; the UI does not filter downstream.

## OS push notification payload contract

### Privacy contract (strict)

A push payload carries **only**:

1. A wake-up flag (constant; e.g. `"wake": 1`).
2. An **opaque encrypted reference to the event ID** (32-byte
   ciphertext, base64url in transit).
3. A routing category (`msg` | `mention` | `letter` |
   `ephemeral-expiry` | `whisper-invite` | `handoff`).

A push payload **never** contains: author name / handle / peer ID;
message body / subject / excerpt; grove or channel name; fingerprint,
crest, or any identity-linkable artefact. Push providers and relays
see only opaque ciphertext + category tag.

### Local composition

On wake-up, the service worker (or native handler):

1. Decrypts the event ID reference with the device's local key.
2. Fetches the full event from the local event store.
3. If **content preview** is enabled for this category, composes the
   visible notification with peer / channel / excerpt.
4. Otherwise falls back to the opaque default
   `willow — 1 new message`.

The visible notification is composed on-device *after* the push has
been decrypted. Providers never see the composed string.

### Per-category opt-in

| Category            | Default enabled | Content preview default | Rationale |
|---------------------|-----------------|-------------------------|-----------|
| `msg` (all messages)| **off**         | off  | Volume is high. |
| `mention`           | **on**          | off  | High signal, bounded volume. |
| `letter`            | **on**          | off  | One-on-one, always relevant. |
| `ephemeral-expiry`  | **on**          | n/a  | Safety-critical; no preview to leak. |
| `whisper-invite`    | **on**          | off  | Rare + consent-bearing. |
| `handoff`           | **on**          | off  | Your own device; always surface. |
| `governance`        | off             | off  | Deferred — see Open Questions. |

Defaults match `settings-tweaks.md` §4 (letters + mentions +
ephemeral + whisper + handoff on; all-messages off). Content preview
is a separate per-category toggle and applies only after local
decryption.

### Coalescing

Events in the same channel / letter within a **20 s window** replace
the active visible notification instead of stacking at the OS level:

- Opaque: `willow — 3 new messages`
- Preview: `3 new messages in #meander`

Window is per-surface; once elapsed the next event starts a fresh
notification.

### Platform adapters

| Platform            | Transport        | Icon        | Sound field  |
|---------------------|------------------|-------------|--------------|
| Desktop browsers    | Web Push (VAPID) | willow PWA  | chime.webm   |
| Android PWA         | Web Push (VAPID) | willow PWA  | chime.webm   |
| iOS PWA (future)    | APNS             | willow PWA  | chime.caf    |

Same opaque payload everywhere; composition is identical.

## Sound

### Default chime

The "willow chime" is a short, low, warm tone — ~400 ms —
intentionally soft. Shipped at `/assets/sounds/willow-chime.webm`
(opus); iOS APNS references the `.caf` variant.

### Per-event variants (future)

| Category           | Tone                                    | Status |
|--------------------|-----------------------------------------|--------|
| `msg` / `mention`  | default chime                           | shipped |
| `letter`           | default chime                           | shipped |
| `whisper-invite`   | violet-tinted chime (~500 ms)           | *open* |
| `ephemeral-expiry` | two-note descending                     | *open* |
| `handoff`          | default chime                           | shipped |

The `willow chime` Settings toggle is the single global chime switch
in v1; per-category tone selection is deferred.

### Global mute + quiet hours

- **Global mute** (chime switch off): no sound ever plays. Toast +
  push still surface; badges still update.
- **Quiet hours** (`{enabled, start, end, days}` from settings-tweaks):
  during the window on enabled weekdays, sound is suppressed and push
  sound is silent (push still posts). Toasts render. The first event
  per session during a quiet-hours window fires an `info` toast:
  > `muted · will ring again at {end}`
  Subsequent events during the same window do not re-toast.

### Own-event suppression

Sound never plays for notifications whose underlying event was
authored by the local peer. Enforced at the service layer; the toast
may still render for ack, but it renders silently.

### Concurrency

- Sound in progress → next sample is enqueued (max queue depth 1);
  further arrivals replace the queued sample with the newest.
- Window focused **and** a toast with a non-overlapping dedup key was
  just shown within 1 s → no sound. The toast enter animation carries
  the signal; the double-ping would be noise.

## Per-surface mute rules

Users mute at three scopes. Each silences progressively finer
audiences; overrides (below) bypass them.

| Scope   | Toast    | Push     | Sound    | Badge                | Tab-bar dot |
|---------|----------|----------|----------|----------------------|-------------|
| Grove   | silenced | silenced | silenced | muted variant        | shown |
| Channel | silenced | silenced | silenced | default (count accumulates) | shown |
| Letter  | silenced | silenced | silenced | default (count accumulates) | shown |

Muting a grove silences everything in scope (channels, letters).
Muting a single channel / letter silences only that surface.

### Overrides

| Override              | Behaviour                                           | Tweak |
|-----------------------|-----------------------------------------------------|-------|
| Whisper invitation    | Toast + push + sound even inside a muted channel.   | `allow muted channels to still surface whisper invitations` — on by default. |
| Ephemeral expiry warning | Toast + push always; sound respects global mute + quiet hours. Safety-critical — a user cannot accidentally miss "10 min until keys burn" by muting. | Cannot be disabled. |
| Handoff request       | Toast + push + sound regardless of every mute, quiet hours, and the global chime switch. The user's *own device* is asking; the choice must be live. | Cannot be disabled. |

Quiet hours silences the handoff chime by default — it is the user's
own explicit choice and trumps the handoff sound override. Whisper /
ephemeral override quiet hours visually (toast + push) but not audibly.

## Copy (exact)

All strings must appear verbatim.

### OS push titles

| Trigger                                                   | String |
|-----------------------------------------------------------|--------|
| Default opaque, single event                              | `willow — 1 new message` |
| Default opaque, coalesced                                 | `willow — {n} new messages` |
| Letter with content preview                               | `{peer} sent you a letter` |
| Mention with content preview                              | `you were mentioned in #{channel}` |
| Whisper invitation                                        | `{peer} wants to whisper` |
| Handoff request                                           | `{peer} asked to move this call` |
| Ephemeral expiry 10-min warning                           | `keys burn in 10 minutes — {channel}` |
| Ephemeral expiry 1-min warning                            | `keys burn in 1 minute — {channel}` |

The `{peer}` slot is the peer's display name, never their fingerprint.

### In-app toasts

| Trigger                                   | Severity       | Title |
|-------------------------------------------|----------------|-------|
| Sync queue drained after offline          | `success`      | `queue drained` |
| First event during quiet hours (session)  | `info`         | `muted · will ring again at {end}` |
| Notifications permission denied           | `info` sticky  | `willow works better with notifications — settings lets you pick what's loud` |
| Handoff request                           | `info` sticky  | `{peer} asked to move this call` |
| Whisper invitation                        | `info` sticky  | `{peer} wants to whisper` |
| Ephemeral 10-min warning                  | `warn` sticky  | `keys burn in 10 minutes — {channel}` |
| Ephemeral 1-min warning                   | `warn` sticky  | `keys burn in 1 minute — {channel}` |

### Toast actions

- `view` — navigates to the underlying surface.
- `dismiss` — closes the toast; sets the acknowledged flag so the
  push does not re-surface on next focus.
- `accept` / `decline` — binary choice on whisper + handoff toasts.
  These two toasts are the exception to "one action per toast";
  `decline` renders as a secondary text button to the left of the
  primary `accept` pill.

### Sync-queue chrome (header strip)

The sync-queue spec renders this strip; the copy is owned here:

> `queued · will send when reachable`

Body S, `--ink-1` on `--bg-2`, mono ` · ` separator. Chrome element —
not a toast; does not enter / exit with toast motion.

## Data dependencies

### Existing

- Event store + gossip — every notifiable event is already an
  `EventKind` variant or a derived signal.
- `willow-client` event stream — consumed by the UI for badges and
  toast dispatch.
- `crates/web` portal layer — the existing overlay container at
  `z-index: 80` is reused; this spec adds a dedicated `#toast-root`
  portal child.

### New (owned or consumed)

- **Notification-category preference** — stored in the per-identity
  settings document (`settings-tweaks.md`):
  ```json
  {
    "notifications": {
      "msg":              { "enabled": false, "preview": false },
      "mention":          { "enabled": true,  "preview": false },
      "letter":           { "enabled": true,  "preview": false },
      "ephemeral-expiry": { "enabled": true,  "preview": false },
      "whisper-invite":   { "enabled": true,  "preview": false },
      "handoff":          { "enabled": true,  "preview": false }
    }
  }
  ```
- **Quiet-hours schedule** — owned by `settings-tweaks.md`; consumed
  here. `{ enabled, start: "HH:MM", end: "HH:MM", days: [0..6] }`.
- **Per-surface mute state** — new
  `EventKind::MuteSurface { scope, target_id, muted }` emitted on
  mute / unmute. Persists in `ServerState` (grove / channel) or
  per-identity settings (letters, since letters aren't rooted in a
  grove).
- **`unread_by_surface` signal** — new derived signal on
  `willow-client`:
  `HashMap<SurfaceId, UnreadStats>`,
  `UnreadStats { count: u32, mentioned: bool, whisper: bool, announce_only: bool }`.
  UI reads; it does not compute.
- **`queued_by_peer` signal** — already published by `sync-queue.md`;
  *not* merged into unread. Remains a separate row atom.
- **Push token events** — new
  `EventKind::RegisterPushToken { device_id, endpoint, p256dh, auth }`
  and `EventKind::RotatePushToken { ... }`. One subscription per
  device; endpoints never exposed to peers.
- **"Content preview" decryption handler** — service-worker function
  taking the opaque push payload, decrypting the event-ID reference,
  fetching the event, composing the visible notification, and calling
  `registration.showNotification(...)`. Contract owned here;
  implementation lives in `crates/web/service-worker.rs`.
- **Notification-permission signal** — derived from
  `Notification.permission`, republished as a client signal so the UI
  can show the "enable notifications" banner exactly once per session
  on `denied`.

### Signals consumed

| Signal                           | Origin              | Use |
|----------------------------------|---------------------|-----|
| `unread_by_surface`              | `willow-client`     | Badge rendering. |
| `queued_by_peer`                 | `sync-queue.md`     | *Not* merged here; noted for boundary. |
| `notification_preferences`       | `settings-tweaks.md`| Per-category gating. |
| `quiet_hours`                    | `settings-tweaks.md`| Sound + push-sound gating. |
| `global_chime_enabled`           | `settings-tweaks.md`| Sound gating. |
| `mute_state[scope, id]`          | new (this spec)     | Per-surface gating. |
| `notification_permission`        | browser API         | One-time enable banner. |

## Edge cases

- **App focused when push arrives.** Service worker detects a focused
  client via `clients.matchAll({ type: 'window' })`; if one exists,
  no `showNotification` call and an in-app toast fires with the same
  severity + copy.
- **Coalesced flurry.** Within 20 s per surface, the visible
  notification is replaced in place (`n` variant) rather than stacked.
- **Notification permission denied.** On first detection per session,
  a sticky `info` toast fires with the verbatim copy and a `settings`
  action opening Settings → Notifications. Dismissing sets a session
  flag; permission flipping to `granted` later clears the flag so a
  future denial can surface again.
- **Whisper-invite push to a peer that cannot whisper.** Service
  worker decrypts, checks local trust state; if the device cannot
  participate (e.g. unverified and `allowUnverifiedWhispers=false`),
  logs silently and does not surface.
- **Background tab / minimized window.** OS push fires.
  `document.title` is prefixed with the unread count (`(3) willow`)
  while hidden; prefix strips 1 s after the tab becomes visible.
- **Push to a muted surface (no override).** Service worker decrypts,
  detects mute, no `showNotification`. Badge still updates via gossip
  (mute is a notification gate, not a state gate).
- **Ephemeral expiry for a channel the user has left.** Notification
  suppressed at dispatch. A *muted* channel still receives the
  warning (override); a *left* channel receives nothing.
- **Handoff from the currently focused device.** Service worker
  recognises the requester ID as the local device, logs, and does not
  surface — prevents loopback pings.
- **Clock skew at quiet-hours boundary.** Uses local wall clock. A
  notification arriving just past `end` will ring even if the user
  "meant" quiet hours to extend. Accepted trade; HLC is not consulted
  for quiet-hours gating.
- **Permission revoked with a sticky toast visible.** The in-app
  toast is unaffected (DOM, not OS). Subsequent events follow the
  "denied" path.
- **Decryption fails.** Fall back to the opaque default; no plaintext
  leaks. Retry decryption on next focus via the event store.
- **Sound asset fails to load.** Visual still renders; no sound; no
  error toast. Dev-build console warning only.
- **>3 rapid events with the same `dedup_key`.** The toast service
  deduplicates: each arrival updates the current toast's body /
  count, resets the auto-dismiss timer, and never produces a second
  toast.
- **Category toggled off mid-flight.** Preference is checked at
  *delivery* time, not send time. Disabled mid-flight → no visible
  notification; the event still lands in the store.

## Accessibility

### Live regions

- A single `#toast-root` portal. New toasts announce via the
  severity's live-region role: `info` / `success` use `role="status"`
  (polite); `warn` / `err` use `role="alert"` (assertive). Title
  announces first; body follows after a pause.
- `aria-relevant="additions"` so stack reflow (older toasts moving
  up) does not re-announce content.
- The "**{n} more**" pill has `role="button"` and an accessible name
  of `"{n} more notifications, activate to expand"`. Activating
  announces the expanded list via a secondary polite region.

### Keyboard

| Key                | Behaviour |
|--------------------|-----------|
| `Esc`              | Dismisses the focused toast. With no toast focused but focus inside app chrome, dismisses the newest auto-dismissable toast; sticky toasts are unaffected. |
| `Ctrl+Alt+N`       | Moves focus to the newest toast (no-op if stack is empty). |
| `Tab` / `Shift+Tab`| Cycles through toasts in the stack (newest → oldest). Focus does not leak to app chrome until `Esc` or action activation. |
| `Enter` / `Space`  | Activates the focused toast's primary action, or dismisses if there is none. |

`--focus-ring` from foundation applies to every interactive element in
the stack.

### Badges

- Each badge has an accessible name derived from count + variant:
  `"12 unread"`, `"3 unread, mentioned"`, `"1 unread whisper"`,
  `"12 unread, muted"`.
- The host row (channel row, letter row, grove tile) uses
  `aria-describedby` to point at a visually-hidden span carrying this
  name; readers announce the row label then the description.
- The mobile tab-bar dot is decorative (`aria-hidden="true"`); the
  tab's accessible name includes the aggregate unread
  (`"letters, 3 unread"`).

### Motion-reduced

Under `prefers-reduced-motion: reduce`: toast enter + exit collapse to
120 ms opacity fades; stack shift + pill swap are instant; badge
mount / unmount is instant.

### Sound never replaces visuals

Every audible notification has a visible equivalent (toast, badge, or
OS push). A user who disables sound, or who is deaf or
hard-of-hearing, receives the same information in the same time window.

### Contrast

- Default badge: `--moss-2` on `--ink-on-accent`, tested ≥ 4.5:1
  across every accent variant in `foundation.md`.
- Muted badge: `--ink-3` border + text on page bg, tested ≥ 4.5:1 on
  `--bg-0`, `--bg-1`, `--bg-2`.
- Whisper badge: `--whisper` on `--ink-on-accent`, stable across
  accents (whisper is not accent-swappable).
- Severity colours on toasts appear only as icon / border accents,
  never as text backgrounds.

## Acceptance criteria

- [ ] Toast container renders at `#toast-root` with `z-index: 80`,
      bottom-centre on desktop, bottom-above-tab-bar on mobile.
- [ ] Toast enter uses `willow-pop-in` 180 ms; exit uses 120 ms opacity
      fade; stack shift 180 ms translateY.
- [ ] Under `prefers-reduced-motion: reduce`, every toast animation
      collapses to opacity-only.
- [ ] Max 3 visible toasts; a fourth collapses the oldest into a
      "{n} more" pill; activating the pill opens the overflow pane.
- [ ] Toasts with no action auto-dismiss at 4 s; toasts with an action
      persist until acted, dismissed, or the state resolves. Hovering
      pauses the timer; leaving resumes.
- [ ] Info + success use `role="status"` (polite); warn + err use
      `role="alert"` (assertive); title announces first.
- [ ] `Ctrl+Alt+N` focuses the newest toast; `Esc` dismisses the
      focused one; `Enter` / `Space` activates its action.
- [ ] Badge pill is `--moss-2` / `--ink-on-accent` by default;
      outlined `--ink-3` when muted; `--whisper` for whisper letters;
      prefixed with `@` when mentioned; renders `99+` above 99.
- [ ] Priority when variants overlap is whisper > mentioned >
      announce-only > muted > default.
- [ ] Grove-tile pill aggregates across scope and inherits the
      highest-priority variant present.
- [ ] Mobile tab-bar shows a 6 × 6 dot for any unread; long-press /
      focus swaps it for a number-bearing pill.
- [ ] Own-authored events never bump unread and never play sound.
- [ ] OS push payload is *only* wake-up flag + encrypted event-ID
      reference + category tag. No name, body, grove, channel, or
      peer fields.
- [ ] Visible notification is composed on-device after decryption;
      opaque default `willow — 1 new message` fires when content
      preview is disabled or decryption fails.
- [ ] Per-category defaults: `mention`, `letter`, `ephemeral-expiry`,
      `whisper-invite`, `handoff` = on; `msg` = off. Content preview
      defaults to off across all categories.
- [ ] Events in the same channel / letter within 20 s coalesce into a
      single visible notification.
- [ ] Quiet hours suppresses sound + push-sound; toasts render; first
      event per session fires the `muted · will ring again at {end}`
      info toast and subsequent events do not re-toast.
- [ ] Handoff request bypasses every mute and quiet hours for
      toast + push + sound. Hard rule.
- [ ] Ephemeral expiry bypasses channel + grove mute for toast +
      push; sound still respects global mute + quiet hours. Hard rule.
- [ ] Whisper invitation bypasses channel mute by default; the
      `allow muted channels to still surface whisper invitations`
      tweak disables the override.
- [ ] Focus-visible draws `--focus-ring` on every interactive toast
      element (body, action button, close `x`).
- [ ] All copy strings match verbatim, including U+2014 em-dashes and
      U+00B7 middle-dot ` · ` separators.
- [ ] Push-permission-denied toast fires once per session with the
      verbatim copy and a `settings` action.
- [ ] When the app is focused, OS push is suppressed and an in-app
      toast fires instead.
- [ ] Background tab prefixes `document.title` with `(n) willow`
      while hidden; prefix strips 1 s after the tab becomes visible.

## Open questions

1. **Per-event tone variants.** Ship whisper / ephemeral / handoff
   variant chimes in v1, or default only? Proposed: default only in
   v1; add whisper violet tone in v1.1 pending licence resolution
   (see `settings-tweaks.md` OQ 5).
2. **In-app do-not-disturb shortcut.** A quick
   `{15 min | 1 h | end of day | until I say stop}` DND overlay on
   top of quiet hours, reachable from the grove-rail footer (`moon`
   icon). Not blocking; plan a follow-up spec.
3. **Governance-category push.** Role changes, kicks, grove
   ownership — own category (default off, under an "advanced"
   disclosure), or ride on `msg`? Proposed: separate category,
   default off.
4. **Urgent handoff override during quiet hours.** Per-request
   `urgent: bool` flag on the handoff event bypasses the quiet-hours
   sound gate. Defer to `device-handoff.md` review.
5. **Toast "undo" for destructive governance events** (e.g. "you
   were kicked from {grove}" with an `appeal` action). Belongs in
   `governance.md`; noted for cross-spec review.
6. **Badge max-count variants.** `99+` is v1; a power-user grove may
   want `999+`. Proposed: keep `99+`; revisit post-field-feedback.
7. **Wake-lock for sticky warn / err toasts on mobile** (e.g. a
   10-min ephemeral expiry). Drain risk. Proposed: no wake-lock; the
   OS push handles lock-screen surfacing.
8. **Announce-only badge variant adoption.** Defined here but not yet
   consumed by a sibling. Proposed: keep defined; `governance.md`
   review decides v1 adoption.
