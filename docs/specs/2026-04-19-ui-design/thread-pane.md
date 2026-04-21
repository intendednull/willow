# Thread pane — sealed replies under a parent message

**Parent:** [README.md](README.md)
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md), [`message-row.md`](message-row.md), [`composer.md`](composer.md)
**Status:** draft

## Purpose

A thread is a sealed conversation rooted at a single parent message in a
channel. Replies are encrypted to the set of *thread participants* only,
not the whole grove — a key is derived from the parent message content
and the participant set. The UI makes the parent↔replies relationship
legible and the narrower cryptographic boundary obvious without turning
it into a warning. Desktop: right-rail pane. Mobile: full-screen push.

## Scope

In scope: desktop right-rail layout, mobile full-screen push, parent
card, reply list, participants strip, sealed-participants footer,
thread composer, thread creation / leaving / unread pill on the
channel stub, edge-case copy.

Out of scope: the crypto derivation algorithm itself; whisper inside
threads; thread search; cross-channel thread indexes.

## Desktop layout

### Position

The thread pane occupies the right rail. It **replaces** the members
pane while it is open — the two panes are mutually exclusive on
desktop. Only one may be visible at a time. The toggle on the channel
header preserves the member-pane open state so that closing a thread
restores the member pane if it was open before.

### Width

- `340px` minimum, `380px` preferred, `420px` maximum.
- Default: `360px`, matching the reference.
- The pane is not user-resizable in v1; Tweaks may expose it later.

### Surface

- Background: `--bg-1` (primary panel).
- Border-left: `1px solid var(--line-soft)`.
- Internal cards (parent card, composer frame) use `--bg-2` with
  `--line` border per foundation.

### Vertical sections (top → bottom)

1. **Header** — `12px 14px` padding, `1px solid var(--line-soft)`
   bottom border. Contains: thread icon (14 px), a two-line title
   block (title + subtitle), participants avatar strip (right-aligned
   when it fits), close (X) button.
2. **Scrollable body** — `flex: 1`, `10px 0` inner padding, uses
   `.scroll` from foundation. Contains the parent card followed by
   the reply list.
3. **Composer** — `padding: 0 12px 12px`, the composer frame is
   `--bg-2` with `--radius` and `--line` border.
4. **Sealed footer** — small italic copy + lock icon, centered below
   the composer with `6px` top margin.

### Header title block

- Line 1: the word "thread" in Fraunces italic, `display S` size
  (`17px / 500 / italic`). Foreground `--ink-0`.
- Line 2: `hint` size (`10.5px / 400`), `--ink-3`. Copy:
  `from #<channel> · <n> replies` (see Copy section).

### Open / close animation

- **Open:** the pane slides in from the right over `--motion-slow`
  (240 ms) with easing `cubic-bezier(0.2, 0.8, 0.2, 1)`.
  `transform: translateX(100%) → translateX(0)` and opacity `0 → 1`.
  The main pane does **not** shift; the thread pane displaces the
  members pane (or is laid over an empty right rail if members was
  already closed).
- **Close:** the reverse — `translateX(0) → translateX(100%)` over
  `--motion`. Focus returns to the triggering message's hover toolbar
  (or to the channel composer if the triggering element is no longer
  in the DOM — e.g. a scrolled-away message).
- `prefers-reduced-motion: reduce` collapses both to opacity only.

## Mobile layout

### Push behaviour

Thread on mobile is a full-screen push, not a sheet. From the channel
view, long-pressing a message → "reply in thread" (or tapping the
thread stub under a parent) pushes the thread screen over the channel
screen using `MScreen` semantics defined in `layout-primitives.md`.

- Duration: `--motion-slow` (240 ms).
- Direction: right-to-left push (`translateX(100%) → 0`); the channel
  screen underneath shifts `-10%` to suggest depth (foundation motion
  rules already define this for `MScreen`).
- No right-rail concept on mobile. The members pane is its own screen
  push from the header menu.

### Top bar

- Left: back chevron (canonical chevron rotated `180deg`), label
  "back" for screen readers.
- Title: thread icon + Fraunces italic "thread" at `display S` weight.
- Subtitle under title: `from #<channel> · <n> replies`.
- Right: overflow menu (more-horizontal icon). Menu contains
  "jump to message", "leave thread", "copy link to thread".

### Body

The scrollable body renders:

1. Parent card (inline at top, `margin: 12px 12px 8px`,
   `--radius-l` `12px`, `--bg-2` surface with `--line`).
2. Section divider: horizontal line + italic uppercase label
   "— replies —" centered, with `--ink-3` ink (mirror the reference).
3. Reply list.
4. A 20 px trailing spacer so the last reply clears the composer.

### Composer

- Frame: `--bg-2`, `border-radius: 20px` (pill), `--line` border,
  `padding: 6px 8px 6px 12px`.
- Thread icon on the left (14 px, stroke 1.6).
- Input expands to fill.
- Send button is a `32 × 32` circular `--moss-2` fill with a
  12–14 px send glyph in `--bg-0` ink.
- Border-top on the composer container: `1px solid var(--line-soft)`.

### Sealed footer

Centered under the composer, same copy as desktop, slightly smaller
(`10px` vs `10.5px`). Lock icon 9 px, stroke 1.6, inline before text.

## Parent card

The parent card is a replayed rendering of the message that opened the
thread. It is *not* a live link into the main message list — editing
the source message updates the card via the shared `DisplayMessage`,
but interactions on the card go through the card's own affordances.

- Surface: `--bg-2` with `1px solid var(--line)`, radius `--radius`
  (10 px on desktop) or `12px` on mobile.
- Internal padding: `6px 14px 10px` desktop, `10px 12px` mobile.
- Margin: `0 10px 12px` desktop, `12px 12px 8px` mobile.
- Top row: avatar (22 px on desktop, 24 px on mobile) + display name
  in Fraunces italic (13 px, `--ink-0`) + timestamp in body-S
  (`11px`, `--ink-3`).
- Body: `body S` size (`13 px / 400`, `line-height 1.5`, `--ink-1`).
  Uses the same `MessageBody` primitive as `message-row.md` and
  `files-inline.md` (mentions, code, inline files render identically).

### Pinned indicator

If the parent is pinned in its channel, a small pin glyph (11 px,
filled variant from foundation) appears in the timestamp cluster
immediately after the timestamp, with tooltip "pinned in #channel".

### Jump affordance

The card is focusable and click/tap-activatable. Activation scrolls
the parent channel to the source message and flashes it briefly
(`--moss-1` background fade, 600 ms, then clears). Copy on hover /
long-press:

> jump to message in #<channel>

On desktop the affordance is an inline chevron (14 px) at the right
edge of the top row, revealed on hover. On mobile the whole card is
tappable and the chevron is always visible.

## Reply list

The reply list uses the same `MessageView` primitives described in
`message-row.md`, with the following thread-specific overrides:

- **Density:** always one step denser than the current channel density
  mode. `balanced` channel → dense replies; `cozy` → balanced;
  `dense` → dense (no further compression).
- **Avatars shrink to 22 px** (desktop) / 24 px (mobile). Avatar
  column width reduces from 40 px to 28 px.
- **Consecutive same-author replies collapse even tighter:** 1 px
  vertical padding between grouped replies (vs 2 px in the main
  channel).
- Reactions, hover toolbar, per-reply edit/delete/react/pin behave
  identically to `message-row.md` and `reactions-pins.md`. Pinning
  inside a thread pins the reply *within the thread only*; it does
  not surface as a channel pin.
- Reply-to-a-reply is **not** supported in v1; the reply composer
  targets the thread root.
- URLs, mentions, inline files, and images render identically to
  channel messages.

## Participants

A horizontal strip at the top-right of the header shows the
participants: up to 5 avatars at 18 px, overlapping by 6 px. If
there are more participants, the sixth slot is a `+N` chip using
`--bg-3`, `--ink-1` text, `--radius-s` and meta type scale.

- Hover on an avatar shows the display name (desktop tooltip
  convention from foundation).
- Hover on the `+N` chip shows the full list as a popover: one line
  per participant, 16 px avatar + display name.
- Tap on mobile: opens a small participants sheet listing names.

Participants are derived client-side from the set of distinct authors
among replies plus the parent author plus anyone who has been
explicitly added. The set is append-only — see "Leaving a thread" for
removal semantics.

## Sealed footer

The most load-bearing copy element in the pane. It is present on both
desktop and mobile and always visible.

- Typography: Fraunces italic, `10.5px` desktop / `10px` mobile,
  `--ink-3` ink, centered.
- Icon: lock, 9 px, stroke 1.6, inline-left of text with 4 px gap.
- Copy: `sealed to thread participants ({n}) — not the whole grove`
  (see Copy section).
- The footer is focusable. Activation (click, tap, or Enter) opens a
  small popover (desktop) or bottom sheet (mobile) explaining key
  derivation:

> **why this is its own seal**
>
> when a thread starts, willow derives a fresh key from the parent
> message and the starting participant set. new replies use that key.
> people in the grove who aren't in this thread cannot read these
> replies — not now, not if they're added to the grove later.
>
> when someone new joins the thread, a new key is derived. previous
> replies remain readable by the earlier set plus the joiner; later
> replies use the new key. leaving a thread stops you from receiving
> future key updates.

- The popover closes on outside click, Escape, or the explicit
  "got it" button inside it.

## Composer

The thread composer follows the same rules as the channel composer
(`composer.md`) with these adjustments:

- **Placeholder:** `reply to thread…` (quoted verbatim from the
  reference bundle's `THREAD_COPY.composePlaceholder`, with the
  ellipsis).
- **Icon on the left:** thread icon, not hash.
- Mentions (`@`) open the mention popover scoped to the *thread
  participants* first, then grove members (visually grouped as two
  sections).
- Attach button works identically (opens the attachment sheet).
- Enter sends; Shift+Enter inserts newline.
- Slash-commands are not available in threads in v1.
- Typing indicators (if shipped in `composer.md`) display below the
  composer in the same style as the channel composer but scoped to
  thread participants.

### Disabled states

- **Parent archived:** composer input is non-focusable, placeholder
  becomes `archived — replies are read-only`, send button is disabled
  (`--ink-4` foreground, `--bg-2` background, no hover). Sealed
  footer copy changes to "archived — participants can still read".
- **Parent deleted:** composer hidden entirely; body shows the empty
  state copy described below. Sealed footer unchanged.
- **You've left the thread:** composer hidden; body shows a small
  banner: `you left this thread — new replies are sealed to current
  participants`. A "rejoin" affordance is shown only if the thread
  allows rejoin (controlled by the owner's settings; v1 default is
  no rejoin).

## Creating, leaving, archiving

### Create

From a channel message's hover toolbar (desktop) or long-press sheet
(mobile), the `reply in thread` action: if a thread exists, open it;
otherwise open an empty thread with that message as the parent.
Participant set starts with parent author + local peer. The thread
key is derived **lazily on first reply**, not at open — reading does
not commit a key. First-time menu description: `start a thread from
this message`.

### Thread stub on the parent message

Once a thread has ≥ 1 reply, the parent in the channel shows a stub
directly under its body: thread icon + `<N> replies` + `last reply
<relative time>` + stacked participant dots (up to 3, 14 px,
`-4px` overlap). Surface transparent, `--bg-3` on hover. Count in
`--ink-2`, timestamp in `--ink-3`. Activation opens the pane / screen.

### Unread pill on the stub

When unread replies exist, a pill appears on the stub: `--moss-1`
fill, `--ink-0` foreground, `meta` type, copy `N new`. Clears when
the pane is open and the list has been scrolled to bottom for
≥ 500 ms (same heuristic as `message-row.md`).

### Leave

From the thread overflow menu: `leave thread`. Confirmation uses the
Leave copy (see Copy). Confirm button is `--err` tinted.

### Archiving the parent

- **Archived:** thread still openable; composer disabled; subtitle
  gains `· archived` suffix (`--ink-3`, italic).
- **Deleted:** parent card body replaced with deleted-parent copy;
  replies remain visible; composer hidden.

## Copy

Exact strings owned by this spec. All lowercase (per foundation
voice) except proper nouns; no exclamation marks.

- Header title: `thread`
- Header subtitle: `from #<channel> · <n> replies`
  (zero-reply form: `from #<channel> · no replies yet`)
- Composer placeholder: `reply to thread…`
- Send tooltip: `reply`
- Sealed footer: `sealed to thread participants ({n}) — not the whole grove`
- Sealed footer (archived): `archived — participants can still read`
- Sealed popover title: `why this is its own seal`
- Parent-card jump tooltip: `jump to message in #<channel>`
- Channel toolbar action: `reply in thread`
- First-time description: `start a thread from this message`
- Overflow menu: `jump to message`, `leave thread`, `copy link to thread`
- Leave confirmation title: `leave thread?`
- Leave body: `leaving stops you receiving new keys for this thread.
  replies already visible stay visible.`
- Leave confirm button: `leave`
- Empty state: `no replies yet — write the first`
- Deleted-parent body: `this thread's parent was removed — replies
  remain visible to participants`
- Archived-parent subtitle suffix: `archived`
- Left-thread banner: `you left this thread — new replies are sealed
  to current participants`
- Participants `+N` tooltip prefix: `and N more`

## Data dependencies

### State signals (`crates/web/src/state.rs`)

- `active_thread_parent_id: RwSignal<Option<String>>` — open thread's
  parent event ID, `None` when closed.
- `thread_pane_mode: RwSignal<ThreadPaneMode>` —
  `Closed | DesktopRail | MobileScreen`, derived from viewport +
  `active_thread_parent_id`.
- `thread_unread_by_parent: RwSignal<HashMap<String, usize>>` —
  per-parent unread count for the stub pill.

Existing signal re-used: `members_pane_open` (mutually exclusive with
thread pane on desktop; prior value stored and restored on close).

### Client methods (`crates/client/src/lib.rs`)

- `open_thread(parent: EventHash) -> ThreadHandle` — filtered reply
  stream + participant set.
- `send_thread_reply(parent, content)` — reply sealed with thread key.
- `leave_thread(parent)` — emits `ThreadLeave` for the local peer.
- `thread_participants(parent) -> Vec<PeerId>`.

### Events — new state-machine work required

`EventKind` currently has `reply_to` on chat messages but no thread
concept. This spec depends on these new variants:

- `ThreadStart { parent, initial_participants }` — emitted on first
  reply, not on open. Permission: `SendMessages`.
- `ThreadReply { parent, content: SealedContent, reply_to }` — reply
  sealed with thread key (not channel key). `reply_to` is scoped to
  the thread.
- `ThreadJoin { parent, peer }` — adds a participant. Permission:
  owner / admin / `ManageChannels` (confirm during planning).
- `ThreadLeave { parent, peer }` — always self-initiated.
- `ThreadArchive { parent }` — freezes composer. Permission:
  `ManageChannels` or thread starter.

Materialization adds `threads: HashMap<EventHash, ThreadState>` on
`ServerState` (`participants: BTreeSet<PeerId>`, `archived: bool`,
cached `reply_count`).

**This is new state-machine work and must be tracked in its own
implementation plan.** Until the events exist, the pane can be built
behind a feature flag backed by a local-only cache.

### Display types (`crates/client/src/lib.rs`)

`DisplayMessage` already carries `reply_to` and `reply_preview`. Add:
`thread_count: Option<usize>`, `thread_last_at_ms: Option<u64>`,
`thread_participant_ids: Vec<PeerId>` (first 3 for the stub dots).

## Edge cases

- **Archived parent:** thread still openable. Composer disabled;
  footer copy becomes `archived — participants can still read`;
  italic `archived` tag next to timestamp.
- **Parent deleted:** parent card body replaced with the deleted-
  parent copy; author, avatar, and timestamp still render; replies
  unaffected; composer hidden.
- **Empty thread:** body shows only the parent card + centered empty
  line `no replies yet — write the first`. Sealed footer uses the
  peer opening the thread as one participant.
- **Thread you've left:** composer hidden; left-thread banner shown.
  Previously visible replies remain; newer replies are not fetched.
- **Single-participant thread:** footer reads `sealed to thread
  participants (1) — not the whole grove`.
- **Lost channel access:** thread stub hidden; if pane is open it
  closes with a toast `you no longer have access to this channel.`.
- **Network partition:** replies queue with the normal queued
  indicator (see `sync-queue.md`). Participants chip tooltip shows
  `some members may still be catching up` when state-hash diverges.
- **Participant count > 99:** display `99+` in subtitle and footer.

## Accessibility

- Opening the pane moves focus to the composer textarea. SR announces
  the `thread` region label followed by the subtitle copy.
- Pane root: `role="region"`, `aria-label="thread"`.
- Close button SR label: `close thread`. Escape closes from any
  focused element, returning focus to the triggering message's action
  button (or the channel composer if the trigger is gone).
- Participants strip is `role="list"`. Each avatar is a
  `role="listitem"` button with `aria-label="<display name>"`. The
  `+N` chip has `aria-label="<N> more participants"` and opens a
  focus-trapped popover.
- Sealed footer: lock icon `aria-hidden`; wrapping button label
  `how thread keys work`.
- Parent card is a button with `aria-label="parent message by
  <name> — jump to it in #<channel>"`.
- Reply list: `role="log"`, `aria-live="polite"`,
  `aria-relevant="additions"`. New replies announce author + first
  ~60 chars of body.
- Reduced motion: slide collapses to opacity; jump flash becomes a
  static `--moss-1` border for 1.5 s.
- Touch targets: close, back, overflow, send ≥ 44 × 44 on mobile.
- Colour independence: pinned, archived, queued each pair a glyph
  with their tint.
- Keyboard shortcuts: `Esc` closes / goes back; Tab cycles close →
  participants → parent card → replies → composer → sealed footer;
  `Ctrl+Enter` / `Cmd+Enter` is the alternative send if Enter is
  remapped via Tweaks.

## Acceptance criteria

- [ ] Desktop: opening a thread slides the pane in from the right over
      240 ms; members pane hides and its prior state restores on close.
- [ ] Desktop pane width is 360 px default, clamped to 340–380 px.
- [ ] Mobile: thread is a full-screen push with back chevron reversing
      the animation.
- [ ] Parent card renders avatar, Fraunces italic name, timestamp, and
      body via the shared `MessageBody` primitive; activation scrolls
      to and flashes the source message.
- [ ] Pinned parents show a pin glyph with tooltip.
- [ ] Reply list avatars are 22 px (desktop) / 24 px (mobile); grouped
      replies collapse to 1 px padding.
- [ ] Participants strip shows up to 5 avatars + `+N` chip; hover /
      tap reveals full list.
- [ ] Sealed footer copy is exactly
      `sealed to thread participants ({n}) — not the whole grove`, in
      Fraunces italic, centered, with inline lock glyph; focusable;
      opens derivation popover / sheet dismissable via outside click,
      Escape, or confirm.
- [ ] Thread stub on the parent shows reply count, last-reply time,
      and stacked participant dots; unread pill appears and clears on
      read.
- [ ] Composer placeholder is `reply to thread…`; disables on archived
      parent; hides on deleted parent and for left threads.
- [ ] "reply in thread" available from channel hover toolbar (desktop)
      and long-press sheet (mobile).
- [ ] Leaving a thread shows confirmation, emits `ThreadLeave`, and
      hides the composer.
- [ ] ARIA region label is `thread`; opening moves focus to composer;
      Escape closes and restores focus to the trigger.
- [ ] All animations respect `prefers-reduced-motion: reduce`.
- [ ] Empty / archived / deleted / left states use exact copy above.

## Open questions

- **Who can add a new participant?** Default: thread starter plus
  `ManageChannels`. Confirm during planning whether any participant
  can add others.
- **Should leaving redact local history?** Current spec keeps replies
  visible post-leave; redaction is an alternative under security
  review.
- **Rejoin policy:** default no rejoin in v1. Per-thread flag in
  governance deferred to `governance.md`.
- **Right-rail width on small desktops (< 1100 px):** at 340 px, the
  main pane is cramped. Flag for `layout-primitives.md` review (auto-
  float sheet? narrower rail?).
- **Typing indicators in threads:** must be encrypted under the thread
  key if shown. Confirm during messaging plan.
- **Cross-channel thread links:** not in v1; note for a future spec.
