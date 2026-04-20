# Messaging — message rendering, composer, reactions, mentions, pins

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md)
**Consumed by:** [`thread-pane.md`](thread-pane.md), [`sync-queue.md`](sync-queue.md),
[`whisper-mode.md`](whisper-mode.md), [`ephemeral-channels.md`](ephemeral-channels.md)

## Purpose

Messaging is the primary surface of Willow. Everything else in the client
(groves, letters, threads, calls) frames the conversation inside a channel.
This spec owns how a single message row renders on desktop and mobile,
author grouping, mentions, reactions, pinned markers, the hover toolbar,
the long-press action sheet, reply and edit surfaces, inline artefacts
(code, files, images, queue notes), the composer, the typing indicator,
and empty / loading / scroll-anchor states.

## Scope

**In scope.** Message row (desktop + mobile). Author-run grouping.
Day separators. Mention parsing and pills with self-mention row
highlight. Reactions (strip + add-chip + hover tooltip). Hover toolbar
(desktop) and long-press action sheet (mobile). Reply and edit flows.
Pin indicator. Inline code and fenced blocks. File card and inline
images. Queue note hints. Composer autogrow, affordances, keyboard
flows, mention autocomplete. Typing indicator. Empty channel, loading
skeletons, scroll anchoring.

**Out of scope (hand-off).** Thread-pane interior → [`thread-pane.md`](thread-pane.md).
Whisper body styling and whisper compose mode → [`whisper-mode.md`](whisper-mode.md).
Pinned-messages *panel* (only the entry point and the in-row marker are
here). Ephemeral timer UI → [`ephemeral-channels.md`](ephemeral-channels.md).
Sync-queue screen and pull-down → [`sync-queue.md`](sync-queue.md).
Profile popover → [`profile-card.md`](profile-card.md). Verified /
unverified / pending-verify badges → [`trust-verification.md`](trust-verification.md).

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

- **Image.** `max-width: 380px` desktop / `280px` mobile, radius 10 px,
  `loading="lazy"`. Mono caption below: `filename · size · e2e encrypted`
  in `--ink-3`.
- **Fenced code block.** `<pre>` on `--bg-0` with `--line` border,
  radius 8 px, `8px 12px` padding. Mono M (12 px), `--ink-2`,
  `white-space: pre-wrap`, `max-width: 520px`. No syntax highlighting in
  v1. Copy IconBtn (24 × 24) appears top-right on block hover
  (desktop), flips to `check` for 900 ms after copy.
- **Inline code (single backtick).** Mono pill on `--bg-2`, `--line`
  border, 3 px radius, `0 4px` padding, `--ink-1`.
- **File card.** `FileCard` on `--bg-2`, `--line` border, radius 10 px,
  `10px 12px` padding. Contents: mime icon (24 × 24, `--ink-2`),
  filename (body L, `--ink-0`, truncated), size + mime hint
  (`--ink-3`), `download` IconBtn. `max-width: 420px`.
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
- **Long-press.** ≥ 500 ms hold opens the action sheet (§Long-press
  action sheet). `navigator.vibrate(25)` on trigger; the row shows a
  `scale(0.98)` and `long-press-active` class during the hold.
- **Inline artefacts.** Tighter: code block mono 11 px with `6px 10px`
  padding; file card `max-width: 100%` within the content column;
  thread stub 11.5 px, up to 2 avatars, 10 px chevron.

### Swipe gestures

Two horizontal swipe gestures are available on a message row. Both
require the horizontal drag to exceed vertical motion by ≥ 1.2× before
the row captures the gesture, so vertical scroll always wins:

- **swipe-right on a message row** → opens the reply in the thread pane
  (existing behaviour; reuses the thread reply flow defined above and
  in `thread-pane.md`).
- **swipe-left on a message row** → quote-reply inline in the channel
  composer. Distinct from the thread reply: it stays in the current
  channel and populates the composer's `replying_to` context (same
  preview bar as §Reply preview).

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

## Reactions

### Reaction strip

Rendered below the body when `message.reactions` is non-empty:

- Flex row, `5px` gap, `flex-wrap: wrap`, `margin-top: 6px`.
- Each pill: inline-flex, `5px` gap, `2px 8px` padding, radius `999px`,
  `--bg-2` on `--line`, `--ink-1` text. 12 px desktop, 11 px mobile.
  Contents: emoji glyph + count (`--ink-2`, weight 500).
- Clicking toggles the local user's reaction (`client.react(id, emoji)`).
- If the local user has reacted, the pill uses `--moss-1` border and
  `color-mix(in oklab, var(--moss-2) 18%, var(--bg-2))` background.
- Hover (desktop) shows a tooltip: `mira, ori, kes reacted`; >3
  reactors uses `first two, and N others`. Mobile tap-and-hold on the
  pill exposes the same list as a small card.

### Add-reaction chip

On desktop it appears at the tail of the reaction strip on row hover:
dashed `--line` border, transparent background, `--ink-3`, containing
a `plus` + `smile` icon. Click opens the emoji picker. Hidden on
mobile — the add action is in the action sheet.

## Hover toolbar (desktop only)

A floating toolbar appears on row mouseenter, absolute-positioned at
the top-right with `-14px` top offset:

- `--bg-1` on `--line`, radius 10 px, `--shadow-2`.
- Inline-flex of 26 × 26 icon buttons, `2px` gap, `3px` padding.
- Contents: five quick reactions → thin `--line` divider →
  `smile` (more reactions) → `thread` → `ear` (whisper reply,
  permission-gated) → `more-horizontal` (overflow menu with copy,
  reply, pin, edit, delete per ownership + permission rules).

Quick reactions default to `👍 ❤️ 🍃 💚 👀`. Override with the five
most recent reactions used in *this channel*. Fade in 120 ms
(`--motion-fast`), opacity-only under reduced motion. Keyboard path:
`Tab` into the row, `F10` or context-menu key opens the overflow menu.

## Long-press action sheet (mobile)

Triggered by ≥ 500 ms touch hold:

- Bottom sheet, `--bg-1`, top radius 16 px (`--radius-l`), `--shadow-2`.
- Quick-emoji row: six 36 × 36 hit targets from the recency list.
- Actions (vertical stack): `reply`, `reply in thread`, `add reaction`
  (opens full picker sheet), `pin` / `unpin` (permission-gated),
  `copy text`, `edit` (own, not deleted), `delete` (own or
  owner/admin, `--err` foreground), trailing `cancel`.
- Swipe-down dismiss: drag ≥ 80 px, *or* release with velocity
  > 200 px/s. Transition disables during drag. Tapping the overlay
  dismisses. Haptic tick on open.

## Reply and edit

### Reply preview (above composer)

When reply is chosen:

- Bar above composer. `--bg-2` on `--line`, radius 10 px (top only,
  attaches to composer). `6px 12px` padding.
- Contents: 2 px `--moss-2` left rule, `replying to` label
  (`--ink-3`, hint size), parent author (display italic, `--ink-1`),
  truncated body preview (single line, `--ink-2`, ellipsis), flex
  spacer, `cancel` text button (`--ink-2`, hover `--ink-0`).
  `Escape` also cancels.
- Click the preview: scrolls the list to the parent and flashes it
  with a 180 ms `willow-pop-in`.

The sent reply carries `reply_to = parent_id` and a `reply_preview`
(~120 chars of parent body). The rendered reply shows a small preview
block above its own body, clickable to jump.

### Edit mode

Choosing edit on an own message:

- Composer pre-fills with the original body, text selected.
- Above the composer: `editing message · esc to cancel` (hint size,
  `--ink-3`).
- Send button label flips to `save`. Submit calls `edit_message`.
  Escape cancels.
- Edited messages show `(edited)` in `--ink-3` after the timestamp.

## Pins

Pinning is permission-gated on `ManageChannels`. The pin menu item is
greyed (no-op) for users without the permission, with tooltip
`only stewards can pin here`.

- **Row marker.** 1 px thin `--amber` left rule (not the 2 px accent —
  pin is a quiet mark), plus the `pinned` badge in the author meta row.
  Pinned messages always break a run.
- **Header entry point.** The channel header's pin IconBtn (owned by
  `layout-primitives.md`) tints `--amber` when the channel has pinned
  messages, with a mono superscript count. Clicking opens the pinned
  panel (panel interior not owned here).

## Code + files

- **Inline code.** Single backtick → mono pill as in §Inline artefacts.
- **Fenced code.** Triple backtick → `<pre>` as in §Inline artefacts.
  Fence language parsed but unused (future highlighting).
- **File cards.** As in §Inline artefacts. Files above 10 MB get a
  `large · downloads on click` warning badge in `--amber`. Images above
  4 MB degrade to a file card instead of inline.
- **Inline images.** `<img loading="lazy" decoding="async">` wrapped in
  an anchor that opens full-size in a new tab. Failed loads fall back
  to the file-card variant.

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

## Composer

Vertical stack: optional reply bar → optional edit bar → compose surface.

### Desktop compose surface

- `--bg-2` on `--line`, radius 12 px, `--shadow-1`, outer `8px 10px`,
  inner vertical flex `8px` gap.
- **Upper row.** Attach IconBtn (`plus`) → auto-grow textarea →
  reserved `gift` IconBtn (feature-flagged, v1 hides) → emoji IconBtn
  (`smile`) → send button.
- **Send button.** Pill, `6px 10px`, `--moss-1` background,
  `--moss-4` foreground, `send` icon + text `send`. Disabled when
  the trimmed textarea is empty.
- **Textarea.** Transparent, no border, `--ink-0`, 14 px.
  `min-height: 1.45em`, grows by `scrollHeight` up to 8 lines then
  scrolls. Placeholder: `message #{channel} — encrypted to {N} peers`;
  for letters, `message {name}`; offline, `offline — messages queue
  until reconnect`.
- **Meta row.** `lock` + `sealed with grove-keys` (`--ink-3`), `·`
  separator (`--ink-4`), `ear` + `hold shift to whisper` (`shift` in
  mono `--whisper`), flex spacer, optional `{name} is whispering`
  status with a 3-dot `willowPulse` in `--whisper`.

### Mobile compose surface

- `--bg-2` on `--line`, radius 22 px (pill), `6px 8px 6px 12px`.
- Attach button (`plus`, 18 px, `--ink-2`) → input (single-line
  default; multi-line on external Shift+Enter) → whisper button
  (`ear`, 18 px, `--whisper`) → circular 34 × 34 send button
  (`--moss-2` bg, `#14130f` fg, `send` icon; dims to `--ink-4` when
  empty).
- Meta row below: `lock` · `sealed to {N} peers in grove` · `tap ear
  to whisper`.

### Keyboard (desktop)

| Key | Action |
|---|---|
| `Enter` | Send. |
| `Shift + Enter` | Newline. |
| `Ctrl + Enter` / `Cmd + Enter` | Force-send (for users who prefer Enter as newline). |
| `Escape` | Unwinds in order: cancel edit → cancel reply → blur. |
| `Tab` inside textarea | Inserts two spaces (no focus move). |
| `ArrowUp` when textarea empty | Enters edit mode on most recent own message. |
| `@` | Opens mention autocomplete. |
| `:` | Opens emoji shortcode autocomplete. |

### Keyboard (mobile)

`Enter` inserts newline (mobile convention); send button is the only
way to submit. `@` still opens mention autocomplete.

### Mention autocomplete

Triggered on `@` at a word boundary:

- Popover above the composer, `-8px` offset, aligned to the `@`.
- `--bg-1` on `--line`, radius 10 px, `--shadow-2`.
- List of peers in the current channel, filtered by prefix match on
  handle / first segment / display name. Each row: avatar (20 px),
  display name, handle (mono), status dot.
- Arrow keys move selection; `Enter` / `Tab` inserts the handle pill;
  `Escape` dismisses. Max 8 rows visible; scrolls above.
- Special row `@channel` (mentions all members) visible only with
  `ManageChannels`.

### Offline state

- Compose surface background softens to
  `color-mix(in oklab, var(--amber) 10%, var(--bg-2))`.
- Meta line prepends `hourglass` + `offline · queuing messages`.
- Send still works; messages enter the pending queue.

### Emoji picker

Popover, 320 × 360, `--bg-1` on `--line`, radius 10 px, `--shadow-2`.
Top search input (mono placeholder `search emoji`). Categories
scrollable: recent (reuses the quick-react recency), smileys, nature,
food, travel, objects, symbols. Arrow keys move selection, `Enter`
inserts, `Escape` closes.

## Typing indicator

Thin row just above the composer:

- Padding `4px 24px` desktop, `8px 14px` mobile.
- 3-dot `willowPulse` (staggered 0 / 200 / 400 ms), `--ink-3`.
- Label: `font-display` italic, `--ink-2`.
- Copy:
  - 1: `{name} is writing…`
  - 2: `{name} and {name} are writing…`
  - 3: `{name}, {name}, and {name} are writing…`
  - 4+: `{count} people are writing…`
- Own typing is never shown to self. A peer is "typing" for 4 s after
  their last typing ping. The local client emits at most one ping per
  3 s while textarea is focused and non-empty.

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
translation work.

### Composer placeholders
- Channel: `message #{channel} — encrypted to {N} peers`
- Letter: `message {name}`
- Offline: `offline — messages queue until reconnect`
- No channel selected: `choose a channel to start`

### Composer meta
- Default: `sealed with grove-keys`
- Offline: `offline · queuing messages`
- Whisper hint (desktop): `hold shift to whisper`
- Whisper hint (mobile): `tap ear to whisper`

### Reply bar
- Label: `replying to`
- Cancel: `cancel` (ARIA `cancel reply`)

### Edit bar
- Label: `editing message · esc to cancel`
- Send label during edit: `save`

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

### Typing
- `{name} is writing…`
- `{name} and {name} are writing…`
- `{name}, {name}, and {name} are writing…`
- `{count} people are writing…`

### Empty state
- `this channel is quiet. say hi?`
- `messages here are sealed to everyone in the grove.`
- Cleared: `cleared — nothing here yet.`

### Scroll anchor
- `jump to latest` + ` · {N} new` (ARIA `jump to latest messages`)

### Deleted placeholder
- `this message was withdrawn`

### Mention autocomplete
- Empty: `no peer by that handle in this channel`
- `@channel` row label: `everyone in this channel`
- `@channel` row hint: `notifies all members`

### Send error (rare, real failure not offline)
- `couldn't send — you're not permitted to post here.`

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

### New reactive signals on `Client`

- `typing(channel_id) -> ReadSignal<Vec<PeerId>>` — peers who pinged
  `TypingPing` in the last 4 s. Ephemeral gossip, not persisted.
- `connection_state() -> ReadSignal<ConnectionState>` with variants
  `Connected | Degraded | Offline`. Drives composer placeholder, meta
  line, and offline tinting.

### Existing methods

`send_message`, `edit_message`, `delete_message`, `react`, `pin_message`,
`unpin_message`.

### New methods required

- `send_typing(channel_id)` — rate-limited (3 s) typing ping.
- `whisper_send(channel_id, body, reply_to)` — defined in
  [`whisper-mode.md`](whisper-mode.md); referenced here as the
  compose-surface extension point.

### Local (view-scope) state

Active reply target and active edit target are local `RwSignal<Option<DisplayMessage>>`
in the chat view, matching the current `input.rs` contract.

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
- **Huge attachment.** Files > 10 MB get the `large · downloads on
  click` warning badge. Images > 4 MB render as file cards instead of
  inline, to avoid silently spending mobile bandwidth.
- **Empty / whitespace-only body.** Cannot be sent (send disabled,
  Enter no-op). Empty bodies arriving from peers (migration edge case)
  render as `empty message` in `--ink-3` italic.
- **Long handle in mention pill.** Handles > 32 characters truncate to
  `first 28 + …` with the full handle in `title`.
- **No display name.** Falls back to handle in body font (display
  italic for a handle reads wrong). If handle is also missing:
  `unknown peer` in `--ink-3` italic.
- **Mention of a peer who left.** Resolver fails; token stays as plain
  text `@formerpeer`. No stale profile exposed.
- **Reaction to a deleted message.** Allowed in v1.
- **Edit after 24 h.** Permitted. `(edited)` is the only marker; no
  timeline in v1.

## Accessibility

### ARIA labels

| Element | Label |
|---|---|
| avatar button | `{display_name} — open profile` |
| author name button | `{display_name} — open profile` |
| message row | `message from {display_name} at {timestamp}` |
| toolbar react (each) | `react with {emoji}` |
| toolbar thread | `start thread` / `open thread` |
| toolbar whisper | `whisper reply` |
| toolbar more | `more actions` |
| reply bar cancel | `cancel reply` |
| edit bar cancel | `cancel edit` |
| send button | `send` |
| attach button | `attach file` |
| emoji button | `open emoji picker` |
| pin IconBtn (header) | `pinned messages ({count})` |
| file download | `download {filename}` |
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

### Focus moves

- On *reply*: focus → composer textarea.
- On *edit*: focus → composer textarea, caret at end.
- On *reaction add*: focus stays.
- On *dismiss sheet / toolbar*: focus returns to the originating row.
- Every focusable element uses `--focus-ring` from foundation.

### Color-independent cues

Mentions: amber background + amber rule + bold weight. Whisper: violet
rule + `ear` icon + italic. Queued: `hourglass` + text. Pinned: `pin`
icon + rule + text. Reactions: emoji + numeric count. Color is never
the sole signifier.

### Motion

All animations respect `prefers-reduced-motion: reduce` per foundation:
`willowPulse` becomes a static opacity dot; jump-to-latest pill
crossfades without translate; delivered flash is an opacity blink.
Long-press haptic is unaffected (a11y benefit).

### Screen reader flow

- Each message is a single announced unit: author, timestamp, body,
  "X replies in thread" (if present), "pinned" / "whisper" / "queued"
  (if present).
- Reactions announce as `{emoji} {count}` for others; for the local
  user, `{emoji} {count}, including you`.
- Typing indicator announced via `aria-live="polite"` with debouncing
  (at most once per 5 s).
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
- [ ] Reaction pills render with count; click toggles the local
      user's reaction; hover shows reactor list (desktop); add-chip
      appears on row hover (desktop); mobile adds via the action
      sheet.
- [ ] Hover toolbar (desktop) appears on mouseenter, offers five
      quick reactions, thread, whisper, more; all buttons carry the
      ARIA labels in §Accessibility.
- [ ] Long-press ≥ 500 ms opens the bottom action sheet; swipe-down
      at 80 px *or* velocity > 200 px/s dismisses; haptic fires on
      open.
- [ ] Reply: choosing reply focuses the composer and shows a preview
      bar with parent author + excerpt + cancel; `Escape` cancels.
      Clicking the preview scrolls to the parent and flashes it.
- [ ] Edit: choosing edit pre-fills the composer, shows the edit
      bar, submits via the edit path, and marks the message `(edited)`.
- [ ] Pinned messages render with a 1 px amber left rule and a
      `pinned` badge; the header pin IconBtn shows a superscript
      count when > 0.
- [ ] Fenced code renders in mono with `--bg-0` + `--line` border; a
      copy button appears on hover (desktop).
- [ ] Inline images render at `max-width: 380 / 280 px` with
      `loading="lazy"`.
- [ ] Queue notes render the inline hint + badge; pending messages
      dim to 0.7 opacity until delivered; delivery flashes `sent`.
- [ ] Whisper rows carry the violet left rule, tinted background,
      and whisper badge (full styling in `whisper-mode.md`).
- [ ] Composer autogrows up to 8 lines; `Enter` sends; `Shift+Enter`
      newlines; `Ctrl/Cmd+Enter` always sends; `Escape` unwinds
      edit → reply → blur.
- [ ] Mention autocomplete opens on `@` with peer filter; arrow keys
      + `Enter`/`Tab` insert; `Escape` dismisses.
- [ ] Offline state: composer applies amber tint; meta line becomes
      `offline · queuing messages`; pending messages show the queue
      hint.
- [ ] Typing indicator shows the correct form for 1 / 2 / 3 / 4+
      typers, driven by a per-channel typing signal.
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

- **Edit history.** Should we surface prior edit versions (the event
  log has them)? Default: no in v1 — just `(edited)`. Data remains
  so a later version can.
- **Quick-reaction recency scope.** Channel-scoped in v1. Revisit
  after use data: users may expect reaction muscle memory across
  groves.
- **Image dimensions in the envelope.** To render a correct-ratio
  placeholder while bytes stream, we need `width` and `height` in
  file attachment metadata. Propose extending the messaging schema.
- **Permission feedback in the action sheet.** Currently we grey
  disallowed actions with an explanatory tooltip. Alternative: hide
  them. Default: grey + tooltip to teach the permission model;
  revisit once governance UX lands.
- **`@channel` confirmation.** Skip a confirm step below 20 members;
  show one above that threshold. Revisit after governance.
- **Typing ping transport.** A new ephemeral event type that does
  not enter the log needs design review in `willow-state` /
  `willow-network` before this spec's acceptance can be met.
- **Reply-in-thread from an already-threaded message.** Replies stay
  in the thread pane; the parent's thread stub updates counts.
  Confirmed against `thread-pane.md` dependency; flag if it changes.
- **Reactions on deleted messages.** Allowed in v1. Confirm, or lock
  reactions post-delete.
- **"Everyone verified" badge in channel preamble.** Currently
  hardcoded in the design bundle. Compute it live from channel
  membership and verification state. Tracked as a data-dependency
  extension under `trust-verification.md`.
