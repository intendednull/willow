# Letters — peer letters and group letters

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md), [`message-row.md`](message-row.md), [`composer.md`](composer.md), [`trust-verification.md`](trust-verification.md), [`whisper-mode.md`](whisper-mode.md), [`sync-queue.md`](sync-queue.md), [`profile-card.md`](profile-card.md), [`local-search.md`](local-search.md)

## Purpose

A **letter** is Willow's one-on-one or small-group persistent thread
between peers. Letters are the primary surface for private
correspondence and stand *apart from* channels: channels belong to
groves (shared communities), while letters live outside any grove and
carry their own per-letter key material and verification state.

The word is load-bearing. A letter is considered correspondence, not
a throwaway message. The UX reinforces the tempo: display serifs,
soft timestamps, patient offline behaviour, no typing-indicator
urgency. A letter is slow by default.

Message rendering inside a letter delegates to `message-row.md` and
`composer.md` (and to `reactions-pins.md` / `files-inline.md` for
reactions and attachments); trust
badges delegate to `trust-verification.md`; whisper marking delegates
to `whisper-mode.md`; queue markers delegate to `sync-queue.md`;
tapping a peer opens the profile card per `profile-card.md`.

## Scope

- Letters list (desktop left pane / mobile tab screen) with two
  sections: peer letters and group letters.
- Letter thread view (desktop right pane / mobile pushed screen).
- Composition surface for starting letters.
- 1:1 → group conversion, group membership, self-leave.
- Per-letter read-receipts toggle, off by default.
- Local-first search across the list.
- Mobile navigation, desktop keyboard shortcuts.
- Empty states, edge cases, accessibility, exact copy.

Out of scope: message primitives (`message-row.md`, `composer.md`,
`reactions-pins.md`, `files-inline.md`), SAS compare flow
(`trust-verification.md`), whisper activation (`whisper-mode.md`),
sync-queue inspector (`sync-queue.md`), profile cards
(`profile-card.md`).

## Letters list

### Layout

**Desktop.** Left pane when the letters tab is active, filling the
slot a grove's channel sidebar would use (see `layout-primitives.md`
for chrome). Width 290 px, background `--bg-1`, right border
`--line-soft`. Right pane shows the selected letter's thread.

**Mobile.** Full-screen list reached from the bottom tab bar's
"letters" entry. Tapping a row pushes the thread screen; back
returns here. Single scrollable surface with a padded search row at
top, sections, and (safely above the bottom inset) the footer.

### Pane header

Desktop title "letters" — Fraunces italic display-M (22 px / 500),
`--ink-0`. Subtitle "direct · e2e, device-local" at body S (13 px),
`--ink-3`. Search input on `--bg-2` with `--radius`, 12 px text,
inline `Search` icon, placeholder `search letters…`.

Mobile header uses the standard top bar (`layout-primitives.md`):
title "letters", same subtitle, right action "+" (44 × 44 hit
target, Plus icon in `--moss-3`).

### Sections

Two labelled sections, rendered in order:

1. **peer letters** — 1:1 letters between the user and one peer.
2. **group letters** — named small groups (≥ 3 participants).

Section header uses the "meta" type role (11.5 px / 500 / uppercase,
tracked +1.2), colour `--ink-3`. Sections are collapsible (open by
default, chevron on the right, state persisted per-device). An empty
section is hidden entirely. See Empty states for the full-list case.

### Row anatomy — 1:1 peer letter

Row height 56 px desktop, 62 px mobile. Padding `10px 12px` desktop,
`10px 14px` mobile. Desktop row uses `--radius` corners with
`0 6px` margin so hover / active highlight sits inset; mobile is
edge-to-edge with bottom `--line-soft` separator.

Structure, left to right:

1. **Avatar** 30 px (desktop) / 38 px (mobile), circular, tinted to
   the peer's crest colour. Bottom-right status dot (9 px);
   `willowPulse` when `status === 'online'`, static otherwise.
2. **Main column** (`flex: 1`, `min-width: 0` for truncation):
   - Top line: display name in Fraunces italic when a display name
     exists, else handle in `--font-ui`. 13 px desktop, 14 px mobile,
     `--ink-0`. Immediately after the name:
     - verified → moss filled check (9 px)
     - unverified → dashed amber ring with "?" glyph (13 px)
     - pendingVerify → same amber ring *plus* a "compare →"
       chip on hover / focus (desktop) or tap-and-hold (mobile)
     - whisper-marked → `Ear` icon 11 px in `--whisper` after the
       trust badge.
   - Bottom line (11.5 px, `--ink-1` when unread else `--ink-3`):
     last-message preview, single line, truncated. Preceded by any
     inline chips listed below.
3. **Right column** (vertical stack, right-aligned):
   - Timestamp (top), mono 10 px, `--ink-3`.
   - Unread pill (if `unread > 0`), `--moss-2` background,
     `#14130f` text, 11 px / 600, `--radius-s` 10 px, padding
     `1px 6px`. Minimum 18 × 18 on mobile.

### Row anatomy — group letter

Structure, left to right:

1. **Avatar stack** (46 px column). First two members render at
   30 px desktop / 34 px mobile, overlapping by -10 px / -12 px.
   If `members.length > 3`, a trailing "+N" chip on `--bg-2` with
   `--ink-2` 11 px bold at the same circular size.
2. **Main column**: group name in `--font-ui` body L, `--ink-0`
   (group names do not take Fraunces italic — that weight is
   reserved for peer display names). Inline mono "users" chip
   after the name (`<icon> <count>`). Bottom line: last-message
   preview, same truncation as 1:1.
3. **Right column**: timestamp + unread pill, identical to 1:1.

Group rows show no per-member verification badge in the list —
verification is per-relationship and cannot be usefully aggregated.

### Inline chips on the preview line

Letter-level state chips render inline on the bottom line, left of
the preview text (left-to-right if multiple):

- **queued** — amber `Hourglass` + mono `{n}`, colour `--amber`,
  border `--amber-soft`, `--bg-0` background. Present when
  `queued > 0`. Tooltip `queued · {n}`.
- **pendingVerify** — amber `Fingerprint` + mono `verify`, same
  colours as queued. Tapping opens the SAS compare flow per
  `trust-verification.md`.

### Icon stacking priority

Multiple state markers stack in a fixed order so scanning is
predictable. Timestamp is always topmost in the right column.
Desktop shows all markers; on narrow widths, queued and
pendingVerify migrate from the right column into the preview-line
chips.

Colour is never the only signifier (foundation): queued pairs amber
with `Hourglass`; whisper pairs violet with `Ear`; verified pairs
moss with filled check; unverified pairs amber with dashed ring.

### Timestamps

Soft-format literals (fixture strings verbatim):

| Window           | Format |
|------------------|--------|
| Today            | `HH:MM` (`10:29`) in 24h locales; `h:mma` lowercased in 12h locales |
| Yesterday        | `yst` |
| 2 – 6 days ago   | `{n}d` |
| 7 – 30 days ago  | `{n}w` |
| Older            | month abbrev (`feb`, `nov`) |

Full timestamp on hover / long-press via `title`; screen readers
receive the full ISO form via the composed row label.

### Row states

`default` transparent · `hover` (desktop) `--bg-2` · `active`
(selected, desktop) `--bg-3` · `pressed` (mobile) `--bg-3` 120 ms
fade · `focus-visible` uses `--focus-ring`. Hover / active never
recolour text.

### Footer

Desktop only: `Plus` icon 12 px + label `start a letter ·
by fingerprint` at 11 px, `--ink-3`. Opens compose. Mobile uses the
top-bar "+" instead.

## Letter thread view

### Header

**Desktop.** 52 px tall, padded `0 16px`, `--bg-1`, bottom border
`--line-soft`. Left-to-right:

- Peer avatar (1:1) or compact avatar stack (group), 28 px.
- Name: Fraunces italic display-S (17 px), `--ink-0`.
- Handle (1:1 only): mono 12 px, `--ink-3`.
- Trust badge (verified check or "unverified — compare" chip). The
  chip opens the SAS compare flow.
- Direct chip: mono `<Lock> direct · no relay` in `--moss-3` with
  `--line` border when the connection is peer-to-peer. Replaced
  with `sealed` when relay is bridging.
- Spacer (flex 1).
- Short fingerprint chip (3-word form), per
  `trust-verification.md`.
- Action bar (32 × 32 icon buttons, 4 px gap): `Phone` (call),
  `Cam` (video), `Ear` (whisper), `Search` (desktop), and an
  overflow `more-horizontal` opening a popover with "letter
  settings", "view profile", "report".

**Mobile.** Uses the `MTopBar` from `layout-primitives.md`. Left:
chevron-left back (44 × 44). Title row: `<avatar 26> <italic name
15 px> <trust badge 9>`. Subtitle: `direct · no relay` /
`sealed` (1:1) or `small circle · {n} members` (group). Right:
`Phone` + `Cam` icons (44 × 44 each). Long-press on the header
title opens letter settings as a bottom sheet.

### Group participants strip

Group letters only. Directly beneath the header, 32 px, padded
`4px 16px`, `--bg-1`, bottom border `--line-soft`. Avatar stack
(up to 5 at 20 px, `-6px` margin), trailing "+N" tile if more,
right-aligned mono meta `{n} people`, and a chevron to expand.
Expansion opens a sheet (mobile) or popover (desktop) listing every
member with avatar, name, verification badge, optional "(left)"
chip, and (for the owner) a remove control.

Tapping the strip or pressing Enter toggles the expanded view.
1:1 letters render no strip.

### Message flow

Delegates to `message-row.md` for author grouping, hover toolbar,
long-press sheet, and code; to `reactions-pins.md` for reactions; to
`composer.md` for edits and replies; and to `files-inline.md` for
files. Letter-specific additions:

- **Meeting-card header** (optional, top of scroll): when the two
  peers have completed SAS, render a centered card with
  `WillowMark`, Fraunces italic "you met {name} in person", and a
  mono meta line "keys verified {date} · no one between you".
  Hidden for group letters or unverified 1:1 letters.
- **Conversion divider**: inserted at the point a 1:1 was
  converted to a group — centered dusk meta: `new sealed key ·
  older messages stay private`.
- **Per-message queue note**: messages that traveled through the
  queue append a mono meta `· queued · arrived` (see
  `sync-queue.md`).

Self bubbles use `--moss-1` background with `--moss-4` text; peer
bubbles `--bg-2` with `--ink-1`; both carry `--line` borders and
`--radius-l` corners with the tail corner reduced to 4 px.

### Composer

Delegates to `composer.md`. Letters-specific placeholder:
`write to {peer.name}…` (1:1) or `write to {group.name}…` (group).
Whisper-marked letters tint the composer violet (`--whisper` at 8 %
alpha, `--whisper` border); see `whisper-mode.md`.

Composer state variants:

- **peer offline**: composer enabled; send queues via sync-queue.
  Send-button tooltip: `{name} is offline · will send on reconnect`.
- **both offline**: above the input, non-blocking hint row
  `we'll wait · no peer reachable` in `--ink-3` with `Hourglass`.
- **blocked**: composer disabled (read-only), submit removed. Hint:
  `{peer.name} isn't accepting letters right now`.
- **peer deleted identity**: composer removed; replaced with the
  footer line `peer no longer exists — letter is archived
  read-only`.

### Right-rail behaviour

1:1 letters: no members pane; the thread uses the full pane.
Group letters: no persistent right-rail either — the participants
strip at top is the only member surface.

## Starting a letter

Entry points: desktop footer "start a letter · by fingerprint",
top-right "+" on desktop list pane or mobile list top bar, and
"write a letter" on a peer's profile card (`profile-card.md`).

### Compose surface

Desktop: modal popover. Mobile: pushed full screen. Structure:

1. Header: Fraunces italic `write a letter to…` at 22 px with a
   sub-line `by name, handle, or six-word fingerprint` at 11 px
   `--ink-3`. Close "x" top-right (SR label `close`).
2. Recipient input: search-as-you-type across known letters,
   trusted fingerprint book, and verbatim six-word fingerprints.
   Results render avatar + name + trust badge + select action.
3. Selected recipient chips row. Picking ≥ 2 recipients silently
   morphs target from 1:1 to group letter.
4. Group-name field (2+ recipients only): `name this group
   (optional)`. Blank falls back to `"{a}, {b} & N more"`.
5. **Whisper toggle** `start as whisper` — disabled unless every
   selected recipient is verified. Disabled hint:
   `whisper requires verified peers — compare fingerprints first`.
6. Primary CTA `start letter` (disabled until ≥ 1 recipient).
   Cmd/Ctrl-Enter submits.

On submit, the letter is created (see Data dependencies), compose
closes, and the thread view is selected. A confirmation meta row
renders at the top of the thread: `letter started · say hi`.

## Converting 1:1 → group

Invoked from the overflow menu (desktop) or letter settings sheet
(mobile) via "add peer". The flow reuses compose recipient
selection. On confirmation a warning modal appears — both peers
see it (remote copy triggered by the membership event):

> adding someone creates a new sealed key — old messages stay
> private, future ones are shared with {new peer}

Actions: "continue" / "cancel". On continue, new letter keys are
derived (Data dependencies), the letter is reclassified as group,
its row migrates from "peer letters" to "group letters" retaining
unread counts, and a conversion divider renders at the change
point. The new peer sees only post-conversion messages; pre-
conversion content remains decryptable only to the original two.

## Group membership

- **Owner**: the peer who created or converted the group letter.
  Only the owner may add or remove members. Transfer of ownership
  is out of scope here.
- **Add**: reuses compose recipient selection.
- **Remove** (owner-only): confirmation dialog
  `remove {name}? they keep their copy of old messages but stop
  receiving new ones.` On confirm, a meta divider reads
  `{name} removed by {owner.name}`.
- **Self-leave**: any member can leave via letter settings ("leave
  letter"). Confirmation: `leave this letter? you'll keep old
  messages locally but stop receiving new ones.` Divider:
  `{name} left the letter`.

Departing peers appear in the remaining members' participants
strip with a faded avatar and a "(left)" label. From the self-
left user's side, the row stays but the composer is replaced with
the footer `you left this letter`; reading and search remain
available.

## Read-receipts toggle

Receipts are off by default — the privacy-preserving default
aligns with the parent spec's principles. Each letter has an
independent per-letter toggle that either party may enable or
disable for their own outgoing receipts.

**Settings.** In the letter settings panel (overflow → "letter
settings" on desktop; long-press header → "letter settings" on
mobile):

- Label: `share read receipts in this letter`
- Sub-copy: `when on, your last-read mark is visible to the other
  peer{s}. off keeps reads private — unread stays local to your
  own view.`

Toggling is instant and authored per-user per-letter.

**Visual effect.**

- Off: unread counts are local-only, no per-message receipt
  meta renders.
- On: self-sent messages gain a mono meta line `seen by {name}`
  (1:1) or `seen by {n} of {total}` (group). Tapping the group
  form reveals a popover listing opted-in viewers; peers with
  receipts off appear in a greyed "didn't share" tail.

Delivery ("sealed · delivered") always renders on self-sent
messages and is never opt-out — it's a sync-layer property.

## Search

Letters consumes [`local-search.md`](local-search.md) with scope
`this letter` (default when a thread is focused) and `all letters`
(default on the letters list, and the escalation from `this letter`).
Behaviour, index, query language, streaming, and privacy footer are
all owned there; this spec owns only the letter-specific placeholder,
no-match copy, and the shortcut cues listed below.

- **Keyboard cues.** `/` on the letters list focuses the shared search
  input with scope defaulted to `all letters`. `⌘F` / `Ctrl+F` inside
  an open letter focuses the same input with scope narrowed to
  `this letter`. `Esc` clears a non-empty query and a second `Esc`
  returns focus to the first row of the list. `⌘K` / `Ctrl+K` stays
  reserved for the command palette (see `layout-primitives.md`).
- **Placeholder** (owned here): `search letters` on the letters list,
  `search this letter` when scoped to an open letter.
- **No match** (owned here): the list region renders a centered
  Fraunces italic meta `no letters match "{q}"` in `--ink-3`, query
  escaped. This replaces the generic `nothing matches "{q}" in
  {scope}` copy from `local-search.md` when the active scope is
  `this letter` or `all letters` and the results surface renders inside
  the letters pane.
- **Clearing.** Inline "x" inside the input clears; `Esc` clears and
  returns focus to the first row per the keyboard cues above.

All other behaviour — index construction, horizon, operators,
streaming, rebuild action, and privacy copy — defers to
[`local-search.md`](local-search.md).

## Mobile navigation

Letters is a root tab in the bottom tab bar (see
`layout-primitives.md`). Label `letters`, icon `Inbox` at 20 px.
Badge: sum of unread counts across both sections.

Tapping the tab presents the list screen; tapping a row pushes the
thread screen; back gesture or button returns. Deep links from
profile cards push the thread directly with the list as the
underlying frame.

**Safe-area handling.** Top bar flush with the safe-area top.
Composer pinned to the safe-area bottom; when the keyboard shows,
the composer lifts by (keyboard height − bottom inset) with a
180 ms slide (`--motion`).

**Swipe actions.** Right-to-left on a row reveals:

- **archive** (cedar): hides the letter into an "archived letters"
  sub-view (reachable from the list overflow); undo toast 4 s.
- **mark read** (moss): clears unread only.

Release past 50 % triggers the primary (archive). Left-to-right
swipe is reserved for a future "pin" and is inert in v1. Reduced
motion collapses the reveal to an instant snap.

## Desktop keyboard

When the letters list has focus:

| Key          | Action |
|--------------|--------|
| `↑` / `↓`   | Move selection across list items (wraps within the filtered set) |
| `Enter`      | Open the selected letter |
| `n`          | Start a new letter (open compose) |
| `/`          | Focus the search input |
| `Esc`        | Clear search if non-empty, else blur |

When the thread view has focus:

| Key               | Action |
|-------------------|--------|
| `Cmd/Ctrl-K`      | Focus letter search (back into list) |
| `Cmd/Ctrl-.`      | Open letter settings popover |
| `Cmd/Ctrl-Shift-P`| Open profile card (1:1 only) |
| `Cmd/Ctrl-Enter`  | Submit composer (messaging binding) |

Other bindings delegate to `message-row.md` and `composer.md`.

## Empty states

**No letters at all.** List body centers `WillowMark` 44 px +
Fraunces italic headline `no letters yet — send the first` +
sub-copy `letters are for one-to-one and small circles. compose
starts a new one.` + CTA button `write a letter` (moss filled).

**Search no-match.** Inline meta `no letters match "{q}"`
(above).

**Thread empty (just created).** `WillowMark` 34 px + Fraunces
italic `letter started · say hi` + sub-copy `{name} will see this
when their client wakes up.` The confirmation meta also renders
inline in the flow so it stays visible once messages arrive.

## Copy (exact)

Lowercase unless proper noun; no exclamation marks.

- Navigation label: `letters`
- Tab subtitle: `direct · e2e, device-local`
- Section headers: `peer letters` · `group letters`
- Search placeholder: `search letters…`
- Desktop footer row: `start a letter · by fingerprint`
- Compose header: `write a letter to…`
- Compose sub-line: `by name, handle, or six-word fingerprint`
- Compose whisper toggle: `start as whisper`
- Compose whisper disabled: `whisper requires verified peers —
  compare fingerprints first`
- Compose CTA: `start letter`
- Confirmation meta: `letter started · say hi`
- Whisper icon label: `whisper`
- Queued chip: inline `{n}` · tooltip `queued · {n}`
- PendingVerify: inline `verify` · hover / focus chip `compare →`
- Direct chip: `direct · no relay`
- Relay-bridged chip: `sealed`
- Composer placeholder: `write to {peer.name}…` (1:1) /
  `write to {group.name}…` (group)
- Offline hint: `we'll wait · no peer reachable`
- Offline send tooltip: `{name} is offline · will send on
  reconnect`
- Blocked hint: `{peer.name} isn't accepting letters right now`
- Deleted-peer footer: `peer no longer exists — letter is
  archived read-only`
- Conversion warning: `adding someone creates a new sealed key —
  old messages stay private, future ones are shared with {new
  peer}`
- Conversion divider: `new sealed key · older messages stay
  private`
- Member removed divider: `{name} removed by {owner.name}`
- Member left divider: `{name} left the letter`
- Self-leave confirmation: `leave this letter? you'll keep old
  messages locally but stop receiving new ones.`
- Self-left footer: `you left this letter`
- Read-receipts label: `share read receipts in this letter`
- Read-receipts sub-copy: `when on, your last-read mark is
  visible to the other peer{s}. off keeps reads private — unread
  stays local to your own view.`
- Seen meta: `seen by {name}` (1:1) / `seen by {n} of {total}`
  (group)
- Empty list headline: `no letters yet — send the first`
- Empty list sub-copy: `letters are for one-to-one and small
  circles. compose starts a new one.`
- Empty list CTA: `write a letter`
- Empty-search meta: `no letters match "{q}"`
- Empty-thread meta: `letter started · say hi`
- Empty-thread sub-copy: `{name} will see this when their client
  wakes up.`
- Timestamp literals: `10:29`, `yst`, `2d`, `1w`, month abbrev.

## Data dependencies

Letters sit outside any grove; the current `willow-state` event set
does not model letter membership. Each dependency is flagged
**existing** (reuses a known event or surface) or **new** (requires
a new type, artifact, or API).

| Dependency | Status | Owner | Notes |
|---|---|---|---|
| Per-letter key material | **new** | `willow-crypto` | Each letter carries its own sealed-session key. Group letters derive a fresh key on every membership change. |
| Letter identity (id, participants, created-at, owner) | **new** | `willow-messaging` | New `Letter` record stored locally per device. Not a `ServerState` event — letters are peer-to-peer objects. |
| Letter membership events (add, remove, self-leave, rename) | **new** | `willow-messaging` | New event kind emitted into the letter's own signed message stream. Drives the conversion / removal / leave dividers. |
| Letter message storage | existing | `willow-messaging` | Reuses `MessageStore` per-letter topic, HLC ordering, signing, sealing, `SealedContent`. |
| Read-receipt high-water mark | **new** | `willow-messaging` | Per-letter opt-in `ReadMark` event carrying the peer's last-read message id. |
| Verified / unverified / pendingVerify | existing | `willow-identity` | Reuses SAS state per `trust-verification.md`. |
| Whisper-marking on a letter | existing | `willow-crypto` + `whisper-mode.md` | Reuses the whisper-session primitive; not a new key type. |
| Per-letter queued state | existing | `sync-queue.md` | Aggregated from the per-peer sync queue. |
| Peer blocked state | existing | `willow-identity` | Reuses peer-block list. |
| Peer identity deletion | **new** | `willow-identity` | A "peer no longer exists" signal (tombstone on announcement or local eviction after sustained unreachability). |
| Local archive flag | **new** | `willow-messaging` | Device-local only; not synced. |

"New" items are tracked in the implementation plan
(`docs/plans/YYYY-MM-DD-letters-dms.md`, to follow). The UX here
does not block on landing order as long as fallbacks hold during
partial rollout.

**Fallback during partial rollout.** If group re-keying is not
ready, "add peer" inside a 1:1 letter is disabled with tooltip
`group letters coming soon`. If read-receipt events are not ready,
the toggle is hidden (not rendered-disabled).

## Edge cases

- **Both peers offline.** Header's direct chip swaps to an amber
  `Hourglass` + mono `waiting`. Composer hint above input:
  `we'll wait · no peer reachable`. New messages enter the queue
  and carry the per-message note once delivered. Row shows the
  `queued · {n}` pill.
- **Peer blocks you.** Old messages remain visible (decryptable
  locally). Composer disabled with `{peer.name} isn't accepting
  letters right now`. Header action bar hides call / video /
  whisper. Row is greyed (preview colour forced to `--ink-4`).
  Trust badge is *not* recoloured — it stays accurate.
- **Peer deletes identity.** Row dims (preview + name forced to
  `--ink-4`); trust badge becomes a small dusk `ghost`; thread
  footer replaces the composer with `peer no longer exists —
  letter is archived read-only`. Actions reduce to export and
  delete. The letter auto-archives (with an undo toast).
- **Group letter where only you remain.** Composer disabled;
  footer `you're the last one here · letter is archived
  read-only`. Row moves to archived and can be deleted.
- **Duplicate / long names.** Display names fall back to
  `"{a}, {b} & N more"` when unset. Ties break by latest HLC
  activity. Long names truncate with ellipsis; full name in the
  row `title` and SR label.
- **Unread overflow.** Pill shows `99+` above 99; mono font.
- **Search while scrolled.** Typing scrolls the list to top and
  resets; clearing restores the prior scroll position.
- **Fingerprint compose with no match.** Result list shows a
  single row `no match for these six words — double-check
  spacing and spelling`. Enter is inert.
- **Switching letters with a draft.** Drafts are per-letter and
  device-local; switching away does not clear them.
- **Accent swap.** Row highlight `--moss-1` and self-bubble
  background pick up the active accent; whisper violet never
  swaps.

## Accessibility

**Row announcements.** Each list row is a `<button>` with a
composed accessible name:

- display name (or group name)
- unread count if `> 0`: `{n} unread`
- verification state: `verified` / `unverified` / `pending
  verification`
- whisper state: `whisper-marked` when applicable
- queued state: `{n} queued` when applicable
- last activity (full ISO form)

Example: `"Mira, verified, whisper-marked, 2 unread. last activity
2026-04-18 10:29 local. press to open letter."`

Group example: `"saturday saw crew, 5 members, 0 unread. last
activity 2026-04-18 10:29 local. press to open letter."`

**Timestamps.** `title` attribute carries the ISO plus a
human-readable local form ("18 feb 2026 10:29 local"). Screen
readers receive the full form via the row's composed label.

**Row icons.** Each carries an SR-only label:

| Icon | Label |
|------|-------|
| verified check | `verified peer` |
| unverified ring | `unverified — compare fingerprints to verify` |
| whisper `Ear` | `whisper-marked` |
| queued `Hourglass` | `{n} queued` |
| pendingVerify chip | `pending verification — tap to compare` |

**Focus.** Row focus-visible uses `--focus-ring`. Keyboard focus
cycles peer letters → group letters → search → desktop footer.
Opening a letter moves focus to the thread scroll region; `F6`
cycles list / thread / composer. Compose popover traps focus and
returns it to the opener on close.

**Touch.** Row hit area ≥ 44 × 44. Long-press opens a context
sheet (open / mark read / archive / mute notifications); keyboard
equivalent is Shift-F10 or the context-menu key on a focused row.

**Composer.** Mobile composer respects the keyboard safe-area
inset. `aria-describedby` wires the composer to the offline /
blocked / deleted-peer hint row. Send button SR label
`send to {name}`. Placeholder is *not* a label substitute — a
visually-hidden `<label>` reads `message {name}`.

**Reduced motion.** Status dot `willowPulse` collapses to steady
opacity; row transitions collapse to instant; mobile screen push
collapses to opacity fade; swipe reveal is instant.

**Contrast.** Meta text (`--ink-3` on `--bg-1`) verified ≥ 4.65:1.
Amber chips on `--bg-0` verified ≥ 4.5:1. Unread pill (`#14130f`
on `--moss-2`) verified ≥ 4.5:1 across every accent.

## Acceptance criteria

- [ ] Letters list renders peer-letters and group-letters sections
      with the section headers, fixtures, and row anatomy
      described (avatar vs stack, truncation, right-column
      timestamp, unread pill, trust badge, whisper icon, queued /
      pending-verify chips).
- [ ] Soft timestamps render per the table (today / `yst` / `{n}d`
      / `{n}w` / month).
- [ ] Desktop row selection persists across tab switches until a
      new selection.
- [ ] Thread header renders correctly on both desktop and mobile
      with the composed name + handle + trust + direct / sealed
      chip + fingerprint + action bar.
- [ ] Composer placeholder matches exact copy.
- [ ] Offline, blocked, and deleted-peer hints render the exact
      copy above.
- [ ] Compose surface gates whisper behind per-recipient
      verification with the exact disabled hint.
- [ ] 1:1 → group conversion shows the warning (both sides),
      renders the conversion divider, and migrates the row to the
      group section retaining unread.
- [ ] Members can leave; the "left" state renders in the strip
      (faded avatar + "(left)"); self-left view shows the
      `you left this letter` footer.
- [ ] Read-receipts toggle is off by default per letter; toggling
      on renders the `seen by …` meta; toggling off hides it
      immediately.
- [ ] Search filters by name + last-message content; empty match
      renders the exact copy.
- [ ] Empty list renders `WillowMark` + headline + sub-copy +
      CTA, each matching exact copy.
- [ ] Desktop keyboard: `↑/↓`, `Enter`, `n`, `/`, `Esc` all behave
      per the table.
- [ ] Mobile: letters tab in bottom bar; swipe actions (archive,
      mark read) respect reduced motion.
- [ ] Accessibility: row SR labels are composed per the template;
      every icon has its SR label; composer safe-area lifts with
      keyboard; focus ring uses `--focus-ring`.
- [ ] Accent swap flows through row highlight, self-bubble, and
      verified check; whisper violet never swaps.
- [ ] All colours, fonts, radii, shadows, motion durations, and
      copy voice conform to `foundation.md`.

## Open questions

- **Letter export.** The deleted-peer edge case implies export
  should ship, but the full UX is out of scope here. Track in a
  follow-up spec.
- **Ownership transfer.** Group-letter ownership transfer is
  hand-waved; likely belongs in a dedicated letter-governance
  spec (overlaps with `governance.md`).
- **Cross-letter mentions / replies.** Intentionally deferred —
  leaking identifiers across sealed sessions is the cost; letters
  stay self-contained.
- **Pinned letters.** Not in v1. Left-to-right swipe on mobile is
  reserved for it.
- **Notifications.** Per-letter mute / mention-only / all.
  Deferred to a notifications spec; this surface exposes only
  "mute notifications" via the long-press action sheet.
- **Multi-device letters.** How letters sync across a user's two
  signed-in devices is unspecified here; opens a cross-spec
  review with `device-handoff.md`.
- **Storage quotas.** Local-cache eviction policy for old letter
  messages (note the interaction with search). Deferred.
- **Grove-hosted small groups?** Decided: no — letters are always
  outside groves. Grove-scoped small groups use threads or
  dedicated channels (`thread-pane.md`).
