# Message row — rendering, grouping, mentions, code, scroll anchor

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md)
**Consumed by:** [`composer.md`](composer.md), [`reactions-pins.md`](reactions-pins.md),
[`files-inline.md`](files-inline.md), [`thread-pane.md`](thread-pane.md),
[`sync-queue.md`](sync-queue.md), [`whisper-mode.md`](whisper-mode.md),
[`ephemeral-channels.md`](ephemeral-channels.md)

## Purpose

The message row is the primary rendering unit of a conversation. This
spec owns how a single message row renders on desktop and mobile,
author-run grouping, day separators, mentions (parsing + pills +
self-mention row highlight), inline code rendering, the pinned-row
marker, queue notes, whisper hand-off styling placeholders, empty /
loading states, and scroll anchoring. The hover toolbar *rendering* is
owned here; each toolbar action delegates to the owning feature spec
(reactions → `reactions-pins.md`, reply / edit → `composer.md`,
pin → `reactions-pins.md`).

## Scope

**In scope.** Row anatomy (avatar, author, time, body, hover highlight).
Author-run grouping. Day separators. Mention parsing + pills + self-
mention highlight. Inline code (single backtick) and fenced code blocks.
Pinned indicator (left-rail amber line + pin badge). Queue notes
(queued, synced from queue, arrived now). Whisper hand-off placeholder
(defers full styling to `whisper-mode.md`). Scroll anchoring and the
jump-to-latest pill. Empty channel state copy. Loading skeletons. Swipe
gestures (both swipe-right and swipe-left on a row). Hover toolbar
*rendering* (actions delegate).

**Out of scope (hand-off).** Reactions strip + add-reaction chip + emoji
picker + pin action → [`reactions-pins.md`](reactions-pins.md).
Attachments (file cards, images, voice notes, upload flow) →
[`files-inline.md`](files-inline.md). Composer, reply preview bar, edit
bar, typing indicator, mention autocomplete popover →
[`composer.md`](composer.md). Thread-pane interior →
[`thread-pane.md`](thread-pane.md). Whisper body styling →
[`whisper-mode.md`](whisper-mode.md). Pinned-messages *panel* interior →
[`reactions-pins.md`](reactions-pins.md). Sync-queue screen and pull-
down → [`sync-queue.md`](sync-queue.md). Profile popover →
[`profile-card.md`](profile-card.md). Verified / unverified / pending-
verify badges → [`trust-verification.md`](trust-verification.md).

## Message row — desktop

### Anatomy

```
┌────────┬───────────────────────────────────────────────────────────┐
│ avatar │ author · handle · time · badges                           │
│  32px  │ body text (mentions, urls, inline code)                   │
│        │ [artefact: image / code block / file card]                │
│        │ [reactions strip]                                         │
│        │ [thread stub]                                             │
└────────┴───────────────────────────────────────────────────────────┘
```

- **Padding.** `--msg-pad` from foundation: `8px 24px` at `balanced`
  (default), `10px 24px` at `cozy`, `4px 24px` at `dense`.
  `margin-top: 4px` between runs, `0` inside a run.
- **Layout.** Flex row, `12px` gap. Avatar column fixed at `38px`
  (32 px avatar + 6 px air) so bodies always align.
- **Avatar.** First-of-run only. 32 × 32, circle, clickable. Collapsed
  rows leave the column empty.
- **Author meta.** Flex, `baseline`, `8px` gap.
  Order: display-name → handle → verified-badge → timestamp → pin badge →
  whisper badge → queued badge.
- **Body.** `font-ui`, body L (14.5 px), `line-height: 1.5`,
  `--ink-1` default. `min-width: 0` so long words wrap.

### Author-run grouping

Consecutive messages collapse into a *run* when all hold:

1. `prev.author_peer_id == current.author_peer_id`.
2. `current.timestamp_ms - prev.timestamp_ms < 5 min`.
3. `prev` is not a day separator.
4. Neither message is `whisper`, `pin`, or `queueNote` — those always
   break a run so their hand-off badge is visible.

Collapsed rows omit the avatar and the author/time/badges row, tighten
padding to `1px 24px`, and reveal the timestamp on hover as a mono
micro-label (`font-mono`, 10 px, `--ink-3`) inside the empty avatar
column.

### Author name + timestamp

- **Display name.** First-of-run only. `font-display` Fraunces, 15 px,
  weight 500, `--ink-0`. Italic only if the display name is literally
  `you`. Click opens the profile popover.
- **Handle (mono).** Immediately after the name. `font-mono`, 10 px,
  `--ink-2`. Omitted when identical to the display name.
- **Timestamp.** `font-ui`, 11 px, `--ink-3`. Format `HH:MM` 24-hour.
  The whole row carries a `title` attribute with the precise datetime
  (`friday 19 april · 10:02:14`) for hover reveal.

### Row states

| State | Background | Left rule (2 px inside-border) |
|---|---|---|
| default | `transparent` | `transparent` |
| hover (non-mention) | `rgba(255,255,255,0.015)` | `transparent` |
| self-mention | `color-mix(in oklab, var(--amber) 8%, transparent)` | `var(--amber)` |
| whisper | see `whisper-mode.md` | `var(--whisper)` |
| pinned | `transparent` | `var(--amber)` *1 px thin* |
| deleted | `transparent` | — |

On mobile the rule is 3 px wide for clarity. Hover triggers the toolbar
reveal (§Hover toolbar).

### Inline artefacts

Each is a direct child of the content column, preceded by `margin-top: 8px`:

- **Image.** Owned by [`files-inline.md`](files-inline.md).
- **Fenced code block.** `<pre>` on `--bg-0` with `--line` border,
  radius 8 px, `8px 12px` padding. Mono M (12 px), `--ink-2`,
  `white-space: pre-wrap`, `max-width: 520px`. No syntax highlighting in
  v1. Copy IconBtn (24 × 24) appears top-right on block hover
  (desktop), flips to `check` for 900 ms after copy.
- **Inline code (single backtick).** Mono pill on `--bg-2`, `--line`
  border, 3 px radius, `0 4px` padding, `--ink-1`.
- **File card.** Owned by [`files-inline.md`](files-inline.md).
- **Thread stub.** Inline-flex pill, `8px` gap, `6px 10px` padding,
  `--bg-2`, `--line`, radius 8 px. Contents: `thread` icon (14 px),
  overlapping participant avatars (up to 3, 18 px, `-8px` overlap),
  `{count} replies` (`--moss-3`, weight 500), `· last at HH:MM`
  (`--ink-3`), flex spacer, `open thread` label, `chevron` icon.
  Click opens the thread pane.

## Message row — mobile

Same anatomy, touch-first adjustments:

- **Avatar.** 36 × 36; column width `42px`.
- **Padding.** `6px 14px` first-of-run, `1px 14px` collapsed.
- **Author meta.** 14 px display italic. Handle omitted. Verified badge
  inline. Timestamp shown first-of-run only; no hover reveal in runs.
- **Swipe-right-to-thread.** Horizontal drag reveals a `thread` icon +
  `open thread` label. Release at `dx > 60 px` opens the thread;
  release below snaps back over 200 ms. Gesture requires horizontal
  motion to exceed vertical by ≥ 1.2× before the row captures it, so
  vertical scroll always wins.
- **Long-press.** ≥ 500 ms hold opens the action sheet. `navigator.vibrate(25)`
  on trigger; the row shows a `scale(0.98)` and `long-press-active`
  class during the hold. Action-sheet *contents* are enumerated below;
  reaction entries delegate to [`reactions-pins.md`](reactions-pins.md).
- **Inline artefacts.** Tighter: code block mono 11 px with `6px 10px`
  padding; thread stub 11.5 px, up to 2 avatars, 10 px chevron. File
  card and image adjustments owned by [`files-inline.md`](files-inline.md).

### Swipe gestures

Two horizontal swipe gestures are available on a message row. Both
require the horizontal drag to exceed vertical motion by ≥ 1.2× before
the row captures the gesture, so vertical scroll always wins:

- **swipe-right on a message row** → opens the reply in the thread pane
  (existing behaviour; reuses the thread reply flow defined above and
  in `thread-pane.md`).
- **swipe-left on a message row** → quote-reply inline in the channel
  composer. Distinct from the thread reply: it stays in the current
  channel and populates the composer's `replying_to` context (reply
  preview bar defined in [`composer.md`](composer.md)).

Both gestures ship enabled; a per-user setting can limit the row to
exactly one direction (see the open question in `settings-tweaks.md`).
Release thresholds mirror the existing thread reveal: commit at
`dx > 60 px`; snap back over 200 ms otherwise. Reduced motion collapses
the reveal animation to an instant state change.

## Day separator

A full-width row between messages from different local days:

- Centered, `10px` gap. Left/right: `flex: 1 height: 1px`, `--line`.
- Label: `font-display` italic, 11 px, `--ink-3`, uppercase with
  `letter-spacing: 1.2px`, wrapped in em-dashes.
- Padding `18px 24px 10px` desktop, `16px 16px 8px` mobile.

Forms (English locale, lowercase enforced):

- `— today —`
- `— yesterday —`
- `— friday · 14 april —` (older than yesterday, within the year)
- `— friday · 14 april · 2025 —` (older than a year)

## Mentions

### Parsing

Match `@handle` with `/@([a-z][a-z0-9._-]*)/gi`. For each token, resolve
in order against the current channel's peers: exact handle match, first
handle segment match (`split('.')[0]`), display-name match, then the
literal `@you` → local peer alias. Unresolved tokens render as plain
text (no pill). Mention parsing runs *before* inline-code parsing so
`@mira` inside backticks stays a code span.

### Rendering

A `MentionPill`: inline-flex, baseline, `1px 6px`, radius 5 px
(`--radius-s`), cursor pointer, opens the profile popover. Label is `@`
+ first handle segment; self-mentions always render as `@you`.

- **Peer mention.** Background `color-mix(in oklab, var(--moss-2) 22%, var(--bg-2))`,
  foreground `--moss-3`, border `1px solid var(--moss-1)`.
- **Self-mention.** Background `color-mix(in oklab, var(--amber) 28%, var(--bg-2))`,
  foreground `--amber`, border `1px solid var(--amber-soft)`.

### Self-mention row highlight

`messageMentionsMe(m)` is true when `m.mentions[]` contains the local
peer ID *or* body parsing produces any pill with `mention.self === true`.
Self-mention rows carry the `--amber` left rule and an
`color-mix(in oklab, var(--amber) 8%, transparent)` row background.
They are the signal source for the "unread mentions" counter in the
sidebar (owned by layout).

## Hover toolbar (desktop only) — rendering

The row-level *rendering* of the hover toolbar lives here; individual
actions delegate to the owning feature spec.

A floating toolbar appears on row mouseenter, absolute-positioned at
the top-right with `-14px` top offset:

- `--bg-1` on `--line`, radius 10 px, `--shadow-2`.
- Inline-flex of 26 × 26 icon buttons, `2px` gap, `3px` padding.
- Contents: five quick reactions (see [`reactions-pins.md`](reactions-pins.md))
  → thin `--line` divider → `smile` (more reactions, opens picker —
  [`reactions-pins.md`](reactions-pins.md)) → `thread` → `ear` (whisper
  reply, permission-gated) → `more-horizontal` (overflow menu with copy,
  reply, pin, edit, delete per ownership + permission rules; reply /
  edit handled by [`composer.md`](composer.md), pin handled by
  [`reactions-pins.md`](reactions-pins.md)).

Fade in 120 ms (`--motion-fast`), opacity-only under reduced motion.
Keyboard path: `Tab` into the row, `F10` or context-menu key opens the
overflow menu.

## Long-press action sheet (mobile) — rendering

Triggered by ≥ 500 ms touch hold:

- Bottom sheet, `--bg-1`, top radius 16 px (`--radius-l`), `--shadow-2`.
- Quick-emoji row: six 36 × 36 hit targets from the recency list
  (entries owned by [`reactions-pins.md`](reactions-pins.md)).
- Actions (vertical stack): `reply`, `reply in thread`, `add reaction`
  (opens full picker sheet — [`reactions-pins.md`](reactions-pins.md)),
  `pin` / `unpin` (permission-gated, action owned by
  [`reactions-pins.md`](reactions-pins.md)), `copy text`, `edit` (own,
  not deleted; handled by [`composer.md`](composer.md)), `delete` (own
  or owner/admin, `--err` foreground), trailing `cancel`.
- Swipe-down dismiss: drag ≥ 80 px, *or* release with velocity
  > 200 px/s. Transition disables during drag. Tapping the overlay
  dismisses. Haptic tick on open.

## Pins — row marker

Pinning *action*, permission rules, the pinned panel, and the header
entry point live in [`reactions-pins.md`](reactions-pins.md). The
in-row visual is owned here:

- **Row marker.** 1 px thin `--amber` left rule (not the 2 px accent —
  pin is a quiet mark), plus the `pinned` badge in the author meta row.
  Pinned messages always break a run.

## Code (inline + fenced)

- **Inline code.** Single backtick → mono pill as in §Inline artefacts.
- **Fenced code.** Triple backtick → `<pre>` as in §Inline artefacts.
  Fence language parsed but unused (future highlighting).

## Queue notes

`message.queue_note` enum: `None | LateArrival | Pending`.

- **LateArrival.** Peer sent this while offline; it reached us after
  authoring. Inline hint below the body:
  `sent earlier · arrived now` — display italic, 11.5 px, `--amber`,
  preceded by a 10 px `hourglass` icon. Plus the `queued` badge in the
  author row.
- **Pending.** Local user sent this while offline; not yet acked by any
  peer. Inline hint: `queued · will send on reconnect`. Row opacity
  `0.7` until delivery. On delivery: opacity fades to 1 over 180 ms;
  the `queued` badge flashes to a `check` + `sent` for 900 ms then
  hides.

Full sync-queue UX lives in [`sync-queue.md`](sync-queue.md).

## Whisper hand-off

Full rules in [`whisper-mode.md`](whisper-mode.md). Declared here only
so layout reserves space:

- Left rule `2px solid var(--whisper)`.
- Row background `color-mix(in oklab, var(--whisper) 8%, transparent)`.
- Body text `--ink-2` italic (body font, not display — display italic
  for message bodies breaks reading rhythm).
- `whisper` badge (violet pill + `ear` icon) in the author row.
- Whisper rows never collapse into a run.

## Empty / loading states

### Empty channel (no messages ever)

Below the preamble (owned by layout):

- Single open-leaf SVG illustration, 48 × 48, `currentColor=var(--willow)`.
- Headline: `font-display` italic, 18 px, `--ink-0` —
  `this channel is quiet. say hi?`
- Subtext: body, `--ink-2` —
  `messages here are sealed to everyone in the grove.`

### Empty after deletions

Headline: `cleared — nothing here yet.`

### Loading

Five skeleton rows using the foundation `shimmer` keyframes: 32 px
avatar circle + two shimmer bars (name + body). First real message
cross-fades skeletons out over 180 ms. Reduced motion uses static
`--bg-2` rectangles (no shimmer).

### Scroll anchoring (jump-to-latest pill)

Auto-scroll to bottom fires only when the user is within 120 px of
bottom. Otherwise:

- A `jump to latest` pill appears at `bottom: 16px; right: 16px`
  desktop or `bottom: 80px; right: 12px` mobile (above tab
  bar / composer).
- `--bg-1` on `--line`, radius `999px`, `6px 12px`, `--shadow-2`.
- Contents: `chevron` (down) + `jump to latest` label + ` · {N} new`
  when count > 0.
- Click: `scrollIntoView({ behavior: 'smooth' })`, clears the count.
- Auto-hides when the user returns within 120 px of bottom.

## Copy (exact strings)

Lowercase unless proper noun. This is the source of truth for
translation work for the strings owned here.

### Overflow menu items
- `reply`, `reply in thread`, `add reaction`, `pin` / `unpin`,
  `copy text`, `copy link`, `edit`, `delete`

### Delete confirmation
- Title: `withdraw message?`
- Body: `this removes it from every peer's view. it was already
  read by some.`
- Confirm: `withdraw`
- Cancel: `keep`

### Queue notes
- Late-arrival hint: `sent earlier · arrived now`
- Pending hint: `queued · will send on reconnect`
- Delivered flash: `sent` (900 ms)

### Badges (visual + ARIA label)
- `pinned`, `whisper`, `queued` (compact), `synced from queue` (full),
  `(edited)` suffix

### Day separator
- `today`, `yesterday`, `{weekday} · {day} {month}`,
  `{weekday} · {day} {month} · {year}`

### Empty state
- `this channel is quiet. say hi?`
- `messages here are sealed to everyone in the grove.`
- Cleared: `cleared — nothing here yet.`

### Scroll anchor
- `jump to latest` + ` · {N} new` (ARIA `jump to latest messages`)

### Deleted placeholder
- `this message was withdrawn`

## Data dependencies

### Existing `DisplayMessage` fields

`id`, `author_peer_id`, `author_display_name`, `body`, `timestamp_ms`,
`deleted`, `edited`, `reply_to`, `reply_preview`,
`reactions: Vec<(String, Vec<PeerId>)>`.

### Fields requiring extension

- `pinned: bool` — derived from the channel's pin event projection
  in `willow-state::ServerState`. Events exist today; the projection
  that stamps each `DisplayMessage` in the current channel list is
  new surface area for `willow-client`.
- `whisper: bool` — depends on [`whisper-mode.md`](whisper-mode.md)
  landing a new `EventKind` variant. Until then, always `false`.
- `queue_note: QueueNote` — enum `None | LateArrival | Pending`.
  Derived client-side in `willow-client::DisplayMessage` by comparing
  `MessageStore` delivery state and the peer's known online status at
  authoring time.
- `mentions: Vec<PeerId>` — explicit list. Body parsing suffices for
  v1, but an explicit list supports future non-textual mentions
  (roles, groups) and accurate self-mention detection when display
  names drift.

### Existing methods

`delete_message`.

## Edge cases

- **Very long single-word body.** `word-break: break-word` +
  `overflow-wrap: anywhere` on the body container. Test fixture: a
  500-char single-word message.
- **Right-to-left.** v1 UI is not localized, but user content may be
  RTL. Body container uses `unicode-bidi: plaintext; direction: auto`
  so each paragraph picks its own base direction. Avatar column stays
  on the start side (LTR chrome). Mention pills flip internally so the
  `@` is at the logical start.
- **Zero-width characters.** Bodies render as plain text
  (no `innerHTML`), so ZWJ / ZWNJ pass through harmlessly.
  Mention matching ignores zero-width characters inside handles.
- **Empty / whitespace-only body.** Empty bodies arriving from peers
  (migration edge case) render as `empty message` in `--ink-3` italic.
- **Long handle in mention pill.** Handles > 32 characters truncate to
  `first 28 + …` with the full handle in `title`.
- **No display name.** Falls back to handle in body font (display
  italic for a handle reads wrong). If handle is also missing:
  `unknown peer` in `--ink-3` italic.
- **Mention of a peer who left.** Resolver fails; token stays as plain
  text `@formerpeer`. No stale profile exposed.
- **Edit after 24 h.** Permitted. `(edited)` is the only marker; no
  timeline in v1.

## Accessibility

### ARIA labels

| Element | Label |
|---|---|
| avatar button | `{display_name} — open profile` |
| author name button | `{display_name} — open profile` |
| message row | `message from {display_name} at {timestamp}` |
| toolbar thread | `start thread` / `open thread` |
| toolbar whisper | `whisper reply` |
| toolbar more | `more actions` |
| jump-to-latest pill | `jump to latest messages` |
| thread stub | `open thread with {count} replies` |

### Keyboard path

- `Tab` from composer moves focus into the message list (single focus
  stop, arrow-key navigation within).
- `ArrowUp` / `ArrowDown` moves the focused row.
- `Enter` opens the overflow menu on the focused row.
- `R` reply, `T` reply in thread, `P` pin/unpin (if permitted),
  `E` edit (if own), `Delete` delete (with confirm), `C` copy body,
  `+` or `:` add reaction.
- `Escape` returns focus to the composer.

### Color-independent cues

Mentions: amber background + amber rule + bold weight. Whisper: violet
rule + `ear` icon + italic. Queued: `hourglass` + text. Pinned: `pin`
icon + rule + text. Color is never the sole signifier.

### Motion

All animations respect `prefers-reduced-motion: reduce` per foundation:
jump-to-latest pill crossfades without translate; delivered flash is an
opacity blink. Long-press haptic is unaffected (a11y benefit).

### Screen reader flow

- Each message is a single announced unit: author, timestamp, body,
  "X replies in thread" (if present), "pinned" / "whisper" / "queued"
  (if present).
- Incoming messages while the list is focused announce via
  `aria-live="polite"`. Not focused → no announcement (notifications
  handle the OS-level cue, out of scope here).

## Acceptance criteria

- [ ] Message row renders avatar (32 px desktop, 36 px mobile),
      display name (Fraunces 15 px, italic for `you`), mono handle,
      `11 px --ink-3` timestamp, and body with density-aware padding.
- [ ] Consecutive same-author messages within 5 min collapse into a
      run: avatar hidden, meta row hidden, padding tightened, hover
      reveals a mono timestamp in the avatar column.
- [ ] Whisper, pinned, and queueNote always break a run.
- [ ] Day separators render between messages from different local
      dates using copy in §Copy.
- [ ] `@mention` tokens become pills in the correct variant;
      `messageMentionsMe` matches either `mentions[]` or parsed body;
      self-mention rows carry the amber left rule + background.
- [ ] Hover toolbar (desktop) appears on mouseenter, offers five
      quick reactions, thread, whisper, more; all buttons carry the
      ARIA labels in §Accessibility.
- [ ] Long-press ≥ 500 ms opens the bottom action sheet; swipe-down
      at 80 px *or* velocity > 200 px/s dismisses; haptic fires on
      open.
- [ ] Pinned messages render with a 1 px amber left rule and a
      `pinned` badge.
- [ ] Fenced code renders in mono with `--bg-0` + `--line` border; a
      copy button appears on hover (desktop).
- [ ] Queue notes render the inline hint + badge; pending messages
      dim to 0.7 opacity until delivered; delivery flashes `sent`.
- [ ] Whisper rows carry the violet left rule, tinted background,
      and whisper badge (full styling in `whisper-mode.md`).
- [ ] Empty channel shows the leaf illustration and the copy in
      §Copy.
- [ ] Scroll anchoring: auto-scroll only when within 120 px of
      bottom; otherwise a `jump to latest` pill with unread count
      appears.
- [ ] Every interactive element has an ARIA label per §Accessibility.
- [ ] Every interaction has a keyboard path; reduced motion collapses
      animations per foundation.
- [ ] 500-char single-word messages wrap without breaking layout.

## Open questions

- **Quick-reaction recency scope.** Channel-scoped in v1. Revisit
  after use data: users may expect reaction muscle memory across
  groves. (Tracked here because the recency list feeds the row's
  hover toolbar and long-press sheet; consumed by
  [`reactions-pins.md`](reactions-pins.md).)
- **Permission feedback in the action sheet.** Currently we grey
  disallowed actions with an explanatory tooltip. Alternative: hide
  them. Default: grey + tooltip to teach the permission model;
  revisit once governance UX lands.
- **Reply-in-thread from an already-threaded message.** Replies stay
  in the thread pane; the parent's thread stub updates counts.
  Confirmed against `thread-pane.md` dependency; flag if it changes.
- **"Everyone verified" badge in channel preamble.** Currently
  hardcoded in the design bundle. Compute it live from channel
  membership and verification state. Tracked as a data-dependency
  extension under `trust-verification.md`.
