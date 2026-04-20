# Sync queue — offline indicator, per-peer queue, dedicated screen

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md), [`messaging.md`](messaging.md), [`letters-dms.md`](letters-dms.md)

## Purpose

The sync queue is the visible representation of patient peer-to-peer:
*messages wait on your device until a peer is reachable. nothing is stored
server-side.* Offline is a *mode*, not an error. Users can see what's
waiting for unreachable peers, what has just arrived from peers that were
offline earlier, and how many of each. All language follows
`foundation.md` rule 4 — *offline is patient, never broken* — without
exception.

Surfaces covered (each ships independently over a shared signal set):

1. Amber offline status strip at the top of the app.
2. Per-peer queued pills in letter rows and member lists.
3. Per-message inline queue notes under message bodies.
4. Pull-down gesture on mobile / chevron on desktop.
5. Dedicated sync queue screen with outbound / inbound tabs.
6. Relay-awareness badge when the active relay is unreachable.
7. Reconnection toast and "queued-welcome" banner after absence.

## Scope

In scope: visual affordances for offline / queued / just-arrived states
across desktop + mobile web, the dedicated sync queue screen, all copy,
accessibility semantics, and the signal contract between `crates/web`
and `willow-client`.

Out of scope: the on-device queue storage itself (a `willow-messaging`
dependency), reachability probing and retry scheduling (UI consumes
signals; it does not author them), and the letters / DMs surface as a
whole (`letters-dms.md` owns the row; this spec owns only the markers).

## Offline status strip

A single-line strip anchored to the top of the app chrome (below any
window chrome / mobile status bar) appears *only* when the local device
has at least one outbound message queued for an unreachable peer. When
no queue depth exists, the strip is fully removed from layout — it does
not reserve space.

### Visual

- Background: `--bg-2` with a 1 px `--amber-soft` top border.
- Text colour: `--ink-1`.
- Left icon: `hourglass` at 14 px, stroke 1.5, colour `--amber`.
- Right edge: small `chevron` at 12 px (desktop), hidden on mobile (mobile
  uses pull-down gesture described below).
- Height: 36 px on desktop, 40 px on mobile (touch target).
- Typography: body S (13 px / IBM Plex Sans) for the summary string; the
  peer-count number uses mono M (12 px / JetBrains Mono) inline so the
  count reads as a concrete quantity rather than chrome flourish.

### Copy

- Default:
  `waiting for {n} peers · {m} messages queued`
- Singular peer:
  `waiting for {peer} · {m} messages queued`
  (when `n == 1`; uses the display name, not the fingerprint.)
- Zero peers:
  Strip is hidden. Never render with `n == 0`.
- With relay unreachable (see **Relay awareness**):
  appends ` · relay unreachable`
  and prepends a small `signal` icon before the hourglass.

### Interaction

- Click / tap → navigates to the sync queue screen (on mobile, pushes the
  full screen; on desktop, opens the right-pane sync queue view).
- Hover (desktop) → background lifts to `--bg-3`.
- Focus-visible → `--focus-ring`.
- Role: `button`, with `aria-label="open sync queue"`.

### Return-of-peer animation

When a queued peer becomes reachable and its row drains: the strip
briefly flashes `--moss-0` background for 240 ms (`--motion-slow`),
returning to amber. Text swaps to `delivered to {peer}` for 2 s, then
returns to the base summary. Multiple peers returning within 2 s batch
into `delivered to {n} peers`. Under `prefers-reduced-motion: reduce`
the flash collapses to opacity-only fade; the count update still
announces.

## Per-peer badge

Used in two places:

1. Letter rows (`letters-dms.md` owns the row; this spec owns the pill).
2. Channel member list (`layout-primitives.md` right rail).

### Visual

A small pill at the end of the row (before any unread count), using the
standard `MTag` pattern from the bundle:

- Shape: `--radius-s` pill, padding `0 6px`, height 16 px.
- Background: transparent.
- Border: 1 px `--amber-soft`.
- Colour: `--amber` (icon and text).
- Icon: `hourglass` at 9 px.
- Text: `queued · {n}` in mono S (10.5 px).

When `n > 99`, render `queued · 99+`. When `n > 500`, render `queued ·
500+` per edge cases below.

### Dual meaning

The same pill is shown whether:

- *Outbound by you to an offline peer* — the local device is holding `n`
  messages waiting for that peer to reappear.
- *Inbound you're expecting* — the peer is known to have authored `n`
  messages still being held somewhere upstream (e.g. they were offline
  when authored; we haven't yet received them via direct sync or relay
  catch-up).

A tooltip (desktop) / long-press popover (mobile) disambiguates:

- Outbound only: `you have {n} messages waiting for {peer}`
- Inbound only: `{peer} has {n} messages pending for you`
- Both: `{out} waiting for {peer} · {in} pending from them`

Screen-reader text always reads the disambiguated form, not the compact
pill text. Use `aria-label` on the pill container; hide the pill text
from AT via `aria-hidden="true"` on the visible label.

### Placement rules

- Letter rows: pill appears after the peer name and verified / unverified
  marker. If both queued and whisper apply, render the queued pill
  first, then the whisper ear.
- Member list rows: pill appears after the display name, right-aligned.
- Do not stack multiple pills on one row. If a peer is both queued *and*
  pending-verify, show `verify` and move the queued count into the
  tooltip only — verification is the more urgent affordance.

## Per-message queue note

Inline note under a message body, small italic Fraunces hint.

### States

| State | Trigger | Copy | Persistence |
|-------|---------|------|-------------|
| `queued` | Outbound message authored while at least one recipient is unreachable; not yet delivered to all recipients. | `queued · will send when {peer} reachable` (single recipient) or `queued · will send when {grove} reachable` (multi) | Persists until the message delivers to all intended recipients. |
| `just-delivered` | Message was previously `queued` and just delivered to its last outstanding recipient. | `queued earlier · delivered just now` | Fades after 30 s (opacity → 0 over 180 ms, `--motion`). |
| `inbound-held` | Inbound message whose HLC timestamp is materially earlier than its receive time (i.e. authored while the peer was offline). | `sent earlier · arrived now` | Hides 5 min after the local receive time. |

Messages older than 5 min with no state transition do not render a note
at all. A message that has transitioned through both `queued` → `just-
delivered` shows only the `just-delivered` note; the prior `queued` note
is replaced in place (no stacking).

### Visual

- Typography: body S (13 px), Fraunces italic (`--font-display`, italic
  400).
- Colour: `--ink-3` for `queued` and `inbound-held`; `--ink-2` for
  `just-delivered` (slightly brighter — a beat of moss warmth without
  being loud).
- Placement: directly under the message body, flush-left with the body
  (same 38 px avatar gutter), with 4 px top margin.
- Icon: a small 11 px `hourglass` before the text for `queued`; a small
  11 px filled `check` for `just-delivered`; a small 11 px `leaf` for
  `inbound-held`. Icon colour matches text colour.

### Interaction

- No click / hover affordance. The note is purely informational.
- Screen-readers announce the note as part of the message row via
  `aria-describedby` pointing at the note span.

### Rendering rules

- Grove messages use `{grove}`; letters (1:1) use the peer's display name.
- Partial delivery: the note stays on `queued` until all outstanding
  recipients have received. A tooltip exposes per-peer breakdown (see
  edge cases).

## Pull-down gesture

### Mobile

On the letters list and on channel message lists, a pull-down gesture
beyond the normal bounce distance reveals a summary card:

- Trigger distance: 48 px of over-scroll (a firm pull, not a mis-swipe).
- Card: fades in at 48 px, fully opaque at 72 px. Releasing before 72 px
  springs back with no navigation.
- Continued pull past 72 px commits to navigating to the full sync queue
  screen with the standard push transition (240 ms `--motion-slow`).
- Haptic cue at commit threshold where available (`navigator.vibrate(8)`
  via the wrapped helper; no-op elsewhere).

Summary card content:

- Title: `sync queue` in display S (17 px, Fraunces italic 500).
- Line 1: `{n} peers waiting · {m} messages`
- Line 2: `oldest waiting: {time}` in mono S — omitted if `n == 0`.
- Card background: `--bg-2`, border `--line`, radius `--radius-l`,
  shadow `--shadow-1`.

When the queue is empty, the card shows the idle form: `all peers
reachable · queue drained` and mono S `last synced {time}`. No commit
threshold — the card is informational only and springs back on release.

### Desktop

No pull-down; the status strip's trailing chevron (12 px, `--ink-3`)
opens the same summary as a popover below the strip using `--shadow-2`
and `willow-pop-in` (180 ms), dismissing on outside click or `Esc`. The
popover's only action is a text link `open sync queue` that navigates
to the full screen.

## Sync queue screen

A full surface (mobile: pushed screen; desktop: right-pane view that
replaces the members / thread pane or a dedicated route at
`/sync-queue`).

### Layout

Header (top bar):

- Left: back chevron (mobile) or pane-close `x` (desktop).
- Centre: title `sync queue` (display S italic), subtitle
  `what's pending · what's reachable` (hint, 10.5 px, `--ink-3`).
- Right: signal icon button — shows the active relay status; tapping
  opens relay status detail (see **Relay awareness**).

Status card (top of body):

- Pulsing moss dot (`willowPulse`, 1200 ms).
- Display S italic label `reaching out…` while any outbound queue has
  items and at least one peer is being attempted.
- Right-aligned mono M count: `{reached} / {total} peers`.
- Progress bar: 6 px height, `--bg-0` track, `--moss-2` fill; width is
  `reached / total` as a percentage.
- When the queue is empty: label becomes `queue drained` with a filled
  `check` icon in `--moss-3`, no progress bar.
- Background `--bg-2`, border `--line`, radius 14 px, margin 14 px,
  padding 16 px (matches the reference bundle's
  `m_sync.jsx` card exactly).

### Tabs

Two tabs: `outbound` (default — peers to whom *you* have pending
messages) and `inbound` (peers you are *expecting* messages from).
Standard layout-primitives tab chrome: 2 px `--moss-2` underline on the
active tab, inactive `--ink-2`, active `--ink-0`. Switching tabs is an
immediate CSS swap.

### Row structure

Each row represents one peer (or grove, for grove-directed queues):

- Left: 34 px avatar (mobile) / 28 px (desktop).
- Centre top line: display name + amber `queued` pill (outbound tab) or
  amber `pending` pill (inbound tab).
- Centre subline: single-line preview of the oldest queued message body,
  ellipsised. If the message is a whisper, render in `--whisper` italic;
  otherwise `--ink-3`. Never show the body on the lock screen — see
  **Privacy**.
- Right: mono S elapsed time label (`2d`, `6h`, `18m`), `--ink-3`.
- Divider: 1 px `--line-soft` between rows.

Tapping a row expands it (180 ms) to show each queued message as a
sub-row with mono S timestamp, single-line preview, and an inline
`retry now` that retries just that message.

For grove-directed messages, the sub-row shows grove name + channel
hash and a per-recipient chip list: avatars tagged `reachable` (filled
`--moss-0` + moss text) or `queued` (`--amber-soft` border, amber text).

### Recent · arrived from queue

A read-only section below the active tab holds up to 24 h of from-queue
deliveries: 32 px avatar, display name, moss `synced` pill with check
icon, and a single-line summary
(e.g. `14 messages synced overnight · from 4 peers`). Tapping a row
opens the relevant channel or letter. Rows older than 24 h disappear
silently.

### Global controls

Footer of the screen body:

- Primary: `retry now` — button with `refresh` icon, moss styling
  (`--moss-1` bg, `--moss-4` fg). Triggers a reconnect attempt on all
  queued peers. Disabled (spinner replaces icon) while a retry is in
  flight or when the queue is empty.
- Secondary (inbound tab only): `mark as read locally` — ghost button,
  stops new-message notifications for inbound items already in queue.
- No `delete` action. The queue is authoritative; removing an item
  would silently lose a message the user authored — never acceptable.

### Reference footnote

Below controls, a small 11 px `--ink-3` hint with a `signal` icon:

> willow holds unsent messages on this device and tries again
> automatically. nothing is stored on a server.

Verbatim from the reference bundle — do not paraphrase.

## Relay awareness

When the active relay is unreachable, the offline status strip and the
sync queue screen both surface this fact:

### Status strip

Appended to the right of the base summary (separator `·`):

`relay unreachable — direct-peer attempts continue`

A small `signal` icon (11 px, colour `--amber`) is prepended before the
base hourglass to disambiguate "peers offline" from "relay offline".
Both are amber; the icon pair communicates which applies.

### Sync queue screen

The top-right signal icon button's colour reflects relay state:

- Reachable relay: `--moss-3` (matches bundle).
- Unreachable relay: `--amber`, with a 2 s `willowPulse` loop at 40%
  intensity (opacity 0.7 ↔ 1) — noticeable but not agitating.

Tapping the icon opens a small popover / sheet (mobile: bottom sheet;
desktop: popover anchored to the icon) with:

- Relay address (mono, `--ink-1`).
- Last successful sync time (mono, `--ink-2`).
- Number of direct-peer attempts in progress (count, moss).
- Secondary link `change relay in settings` (if user has permission).

### Reconnection toast

When the device itself comes back online (WebSocket / iroh connectivity
restored after a period of being fully offline):

- Toast appears at the top-centre on desktop, bottom-centre on mobile.
- Background `--bg-2`, border `--moss-1`, radius `--radius`, padding
  `10px 14px`.
- Icon: `check` in `--moss-3`, 14 px.
- Copy: `reconnected · delivering {n} messages` (where `n` is the total
  outbound queue depth).
- If `n == 0`: `reconnected` (no trailing clause).
- Dismissible via `x` button on the right, auto-hides after 4 s.
- Motion: slides in via `willow-pop-in` (180 ms), slides out via
  opacity-only fade (120 ms).
- Only one reconnection toast is visible at a time. Rapid reconnect
  cycles collapse to the most recent.

## Welcome-back banner

When the app is re-opened after being fully offline and one or more
queued messages arrived during that window, a banner appears at the top
of the home view (letters list on mobile, main pane on desktop):

- Height: 48 px.
- Background: `--moss-0`.
- Left icon: small `willow` wordmark glyph (14 px, `--willow`).
- Copy:
  `willow queued {n} messages while you were away — everything arrived`
- Dismiss: small `x` button on the right (14 px, `--ink-3`).
- Persistence: banner dismisses on first interaction with any message
  row, or on explicit `x` tap. It does not auto-hide; the user chooses
  when to close it.
- Appears only once per "reopen after offline" session. Do not re-render
  on reconnection toasts or after a short backgrounding (< 60 s).

## Privacy

1. Queue contents are stored encrypted at rest on-device. The UI *never*
   reads plaintext from the queue except to render it inside the app.
2. Mobile lock-screen / system notifications for inbound queued messages
   must be content-free. Notification body text is limited to:
   `a letter is waiting` (for letters) or
   `a message in {grove}` (for groves).
   The sender's display name and the message body are not included.
   Notifications for locally-authored queued items (i.e. a local retry
   failure) do not exist — local retries are silent.
3. The sync queue screen, once opened, does render message previews.
   The implication is explicit: opening the screen counts as reading.
4. The `mark as read locally` control does not decrypt anything new; it
   writes a local "last seen" marker so the badge counts drop. Bodies
   remain sealed until the user actually opens the message.
5. Screen-reader output for pills and badges always uses the display
   name, never the fingerprint or the raw peer ID.

## Copy (exact)

These are the canonical strings. All other surfaces defer to this table
— do not paraphrase. All strings follow `foundation.md` copy voice
(lowercase, no exclamation marks, patient tone).

| Key | String |
|-----|--------|
| `strip_default` | `waiting for {n} peers · {m} messages queued` |
| `strip_singular` | `waiting for {peer} · {m} messages queued` |
| `strip_relay_suffix` | ` · relay unreachable` |
| `strip_delivered_peer` | `delivered to {peer}` |
| `strip_delivered_many` | `delivered to {n} peers` |
| `pill_queued` | `queued · {n}` |
| `pill_queued_max` | `queued · 500+` |
| `pill_tooltip_out` | `you have {n} messages waiting for {peer}` |
| `pill_tooltip_in` | `{peer} has {n} messages pending for you` |
| `pill_tooltip_both` | `{out} waiting for {peer} · {in} pending from them` |
| `msg_note_queued_peer` | `queued · will send when {peer} reachable` |
| `msg_note_queued_grove` | `queued · will send when {grove} reachable` |
| `msg_note_just_delivered` | `queued earlier · delivered just now` |
| `msg_note_inbound_held` | `sent earlier · arrived now` |
| `pull_summary` | `{n} peers waiting · {m} messages` |
| `pull_oldest` | `oldest waiting: {time}` |
| `pull_idle_line1` | `all peers reachable · queue drained` |
| `pull_idle_line2` | `last synced {time}` |
| `screen_title` | `sync queue` |
| `screen_subtitle` | `what's pending · what's reachable` |
| `screen_card_label` | `reaching out…` |
| `screen_card_drained` | `queue drained` |
| `screen_card_count` | `{reached} / {total} peers` |
| `screen_section_outbound` | `queued — will deliver when peer is reachable` |
| `screen_section_inbound` | `pending — still sealed on other devices` |
| `screen_section_recent` | `recent · arrived from queue` |
| `screen_pill_waiting` | `waiting` |
| `screen_pill_synced` | `synced` |
| `screen_footnote` | `willow holds unsent messages on this device and tries again automatically. nothing is stored on a server.` |
| `relay_unreachable` | `relay unreachable — direct-peer attempts continue` |
| `action_retry` | `retry now` |
| `action_mark_read` | `mark as read locally` |
| `toast_reconnected_many` | `reconnected · delivering {n} messages` |
| `toast_reconnected_zero` | `reconnected` |
| `banner_welcome_back` | `willow queued {n} messages while you were away — everything arrived` |
| `notif_letter` | `a letter is waiting` |
| `notif_grove` | `a message in {grove}` |

## Data dependencies

The UI consumes the following signals. Each is marked **(existing)** or
**(new)** relative to the current Leptos client. New signals require
corresponding additions in `willow-client` and `willow-messaging`.

### Existing

- `connection_status: ReadSignal<String>` — currently
  `"connecting" | "connected"`. Needs to extend to include `"offline"`.
  (See `crates/web/src/state.rs:68`.)
- `peers: ReadSignal<Vec<(String, String, bool)>>` — per-peer online flag.

### New

| Signal | Type | Semantics |
|--------|------|-----------|
| `queue_depth: ReadSignal<usize>` | total outbound items pending | sum across peers |
| `queue_peer_count: ReadSignal<usize>` | distinct unreachable peers with at least 1 queued item | drives `strip_default` `n` |
| `queue_per_peer: ReadSignal<HashMap<PeerId, QueueSummary>>` | per-peer queue stats | see `QueueSummary` below |
| `queue_inbound_per_peer: ReadSignal<HashMap<PeerId, usize>>` | expected inbound count per peer | best-effort; may be zero if unknown |
| `queue_oldest_at: ReadSignal<Option<HlcTime>>` | oldest queued item's HLC timestamp | drives `pull_oldest` |
| `queue_recent_arrivals: ReadSignal<Vec<ArrivedSummary>>` | last 24 h of from-queue deliveries | drives `screen_section_recent` |
| `relay_status: ReadSignal<RelayStatus>` | `{ Reachable, Unreachable, NotConfigured }` | drives relay-aware chrome |
| `device_online: ReadSignal<bool>` | local device's own network reachability | drives reconnection toast |

`QueueSummary` (new struct, owned by `willow-client`):

```rust
pub struct QueueSummary {
    pub outbound: usize,
    pub oldest_outbound_at: Option<HlcTime>,
    pub last_attempt_at: Option<HlcTime>,
    pub last_attempt_error: Option<String>, // internal, not surfaced verbatim
}
```

### New queue semantics in `willow-messaging`

Queue primitives do not currently exist in `crates/messaging`. This spec
declares them as a dependency:

- An on-device outbound queue keyed by `(message_id, recipient_peer_id)`.
  A 1:1 letter produces 1 entry; a grove message produces one entry per
  recipient that was unreachable at send time.
- Entries drain when the recipient's peer becomes reachable and the
  gossip delivery ACK is observed.
- Entries are encrypted at rest using the same sealed-content path that
  writes to on-disk storage (`SealedContent`).
- A small bounded in-memory projection (`QueueView`) is published via the
  client's view system (mirroring `channels`, `members`, etc.) so that
  Leptos signals can derive from it without reading storage on every
  tick.

These new primitives are the responsibility of the implementation plan
that ships this spec. The UI contract is exactly the signal set above.

## Edge cases

1. **Peer permanently unreachable.** If a peer has been unreachable for
   more than 14 days, the sync queue row for that peer shows an inline
   card offering two actions:
   `you haven't seen {peer} since {date} — keep messages queued or move to a separate archive?`
   Buttons: `keep queued` (default, moss) and `archive` (ghost). Archived
   items are moved to a per-peer local archive surface (`letters-dms.md`
   owns the archive UI itself); this spec only owns the prompt.
2. **More than 500 queued.** Per-peer pill and screen counts cap at
   `500+`. The strip summary caps its `m` at `500+` as well. The screen
   still lists individual rows lazily (virtualised list).
3. **Partial fan-out.** When a grove message is queued because some
   recipients are unreachable, the row in the outbound tab shows a
   per-recipient breakdown expanded inline, each with a status chip
   (`reachable` / `queued`). The top-level message keeps its `queued`
   note until the last recipient delivers.
4. **Relay-only peers.** If the only path to a peer is via the relay and
   the relay is unreachable, the per-peer pill stays `queued`; the
   status strip shows the `relay unreachable` suffix; the sync queue
   screen row shows a small `signal` icon next to the peer name with
   tooltip `reachable only via relay · waiting`.
5. **Clock drift / HLC regression.** `oldest waiting` copy uses the HLC
   timestamp, not wall-clock. If HLC regression is detected (inbound
   older than oldest-outbound but merged), the `sent earlier · arrived
   now` copy still applies and the elapsed time uses the HLC difference,
   not negative values.
6. **Device backgrounded briefly.** If the device goes offline for less
   than 60 s, the reconnection toast does not fire and the welcome-back
   banner does not appear. The strip transiently shows `waiting` during
   the window, but no toast spam.
7. **Queue drained while on the screen.** If the user is viewing the
   sync queue screen and the queue drains to empty, the screen remains
   open with the status card showing `queue drained`. Do not auto-close.
   A manual back / close is required.
8. **User retries while a retry is already in flight.** The `retry now`
   button becomes disabled with a spinner; subsequent taps are no-ops.
   The button re-enables when the retry wave completes or times out.

## Accessibility

1. The offline status strip is a live region with
   `aria-live="polite"` and `role="status"`. Count changes are announced
   but not interruptive. The full announcement uses
   `strip_default` expanded with the numeric counts.
2. Per-peer pills use `aria-label` set to the disambiguated tooltip
   string (`pill_tooltip_out` / `pill_tooltip_in` / `pill_tooltip_both`).
   The visible pill text is hidden from AT via `aria-hidden="true"`.
3. Per-message queue notes are attached to the message row via
   `aria-describedby`. The note itself is non-interactive and has no
   tab stop.
4. The sync queue screen uses `role="list"` for each section with
   `role="listitem"` on each row. The expanded sub-rows use
   `role="group"` with `aria-label` describing the peer.
5. `retry now` is a `button` with clear label; busy state uses
   `aria-busy="true"` while a retry is in flight.
6. Pull-down gesture has a keyboard equivalent: pressing `PageDown` at
   the top of the letters list or a channel, when already scrolled to
   the top, navigates to the sync queue screen after a brief confirm
   toast (`open sync queue? press PageDown again`). No accidental
   keyboard navigation on a single keypress.
7. Focus management: when the sync queue screen opens, focus moves to
   the back button (mobile) or the close button (desktop). When the
   screen closes, focus returns to the element that opened it.
8. Colour is never the sole signifier. Queued states always pair amber
   with an `hourglass` icon; synced states pair moss with a `check`;
   relay unreachable pairs amber with a `signal` icon. See
   `foundation.md` §Accessibility baseline.
9. Touch targets: pills on mobile are wrapped in a 44 × 44 px hit box
   via padded parent, even though the visible pill is 16 px tall.
10. Reduced motion: the `willowPulse` on the status card dot collapses
    to a static 70% opacity dot. The amber-to-moss flash on delivery
    collapses to an opacity-only fade. The `willow-pop-in` on the
    reconnection toast collapses to opacity-only fade in.

## Acceptance criteria

- [ ] Status strip is absent when `queue_peer_count == 0` and present
      otherwise; it never reserves layout space when absent.
- [ ] Strip copy matches `strip_default` / `strip_singular` exactly,
      including the middle-dot separator and lowercase casing.
- [ ] Per-peer pill renders on letter rows and member rows when
      `queue_per_peer[peer].outbound > 0` or `queue_inbound_per_peer[peer] > 0`.
- [ ] Tooltip / long-press popover produces the correct disambiguated
      string for outbound-only, inbound-only, and both cases.
- [ ] Inline message note renders in `queued`, `just-delivered`, and
      `inbound-held` states with the exact copy strings in the table.
- [ ] `just-delivered` fades after 30 s; `inbound-held` hides after 5 min.
- [ ] Pull-down on mobile at 48 px reveals the summary card; at 72 px it
      navigates to the sync queue screen; release before 72 px springs
      back with no navigation.
- [ ] Desktop chevron on the strip opens the summary popover; the
      popover's `open sync queue` link navigates to the full screen.
- [ ] Sync queue screen has outbound / inbound tabs and a recent-
      arrivals section; each row structure matches this spec.
- [ ] `retry now` triggers an implementation-defined retry action and is
      disabled while a retry is in flight.
- [ ] `mark as read locally` exists only on the inbound tab and never
      surfaces message bodies.
- [ ] No `delete` action is exposed anywhere in the sync queue screen.
- [ ] Relay unreachable state appends `strip_relay_suffix` to the status
      strip and tints the signal icon amber on the sync queue screen.
- [ ] Reconnection toast renders on device online transition after
      ≥ 60 s offline, auto-hides after 4 s, and is dismissible.
- [ ] Welcome-back banner renders once per reopen-after-offline session
      with the exact `banner_welcome_back` copy.
- [ ] Notification bodies for queued items contain no peer names or
      message text — only `notif_letter` or `notif_grove`.
- [ ] All exact copy strings match the `Copy (exact)` table verbatim.
- [ ] Screen-reader announces count changes on the strip politely
      without interrupting the user.
- [ ] All animations respect `prefers-reduced-motion: reduce`.
- [ ] Keyboard path exists for every interactive element; focus-visible
      is present per foundation.

## Open questions

1. **Inbound queue discovery.** How does the UI learn that a peer has
   `N` messages pending for us when they are offline? Options:
   (a) a best-effort hint baked into the peer's last seen heartbeat,
   (b) omit inbound counts entirely until the peer returns. The spec
   currently allows `queue_inbound_per_peer` to be zero when unknown;
   the data layer decides whether to populate it.
2. **Archive surface.** The permanent-unreachable card offers an
   `archive` action. The archive surface is currently only implied —
   whether it lives under `letters-dms.md` or a new spec is unresolved.
3. **Retry throttling feedback.** If the user taps `retry now`
   repeatedly and the system rate-limits them, do we surface the rate
   limit? Current assumption: no visible error; the button stays in
   busy state until the backoff elapses.
4. **Cross-device queue.** If a user has two devices (e.g. desktop +
   mobile) with partial queues, does one device's drain affect the
   other's indicator? This depends on cross-device sync, which is
   out of scope here; current behaviour is per-device only.
5. **"reconnected" toast vs banner overlap.** If both the reconnection
   toast and the welcome-back banner would fire at the same time, the
   banner takes precedence (it is more persistent and more informative).
   The toast is suppressed in that window.
6. **Grove-directed partial delivery copy.** The per-message note for a
   grove message with partial delivery currently says
   `queued · will send when {grove} reachable`, which is technically a
   lie if only two of ten recipients are unreachable. Alternate copy
   could read `queued · will send to 2 more peers` — deferred pending
   user research.
7. **Wordmark in banner.** Whether the welcome-back banner uses the
   `willow` wordmark glyph or the app mark is not fully decided; the
   `settings-tweaks.md` wordmark toggle may affect its presence.
