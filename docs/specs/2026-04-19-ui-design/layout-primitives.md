# Layout primitives — desktop three-pane + mobile bottom-tab chrome

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md)
**Consumed by:** `message-row.md`, `composer.md`, `reactions-pins.md`,
`files-inline.md`, `thread-pane.md`, `call-experience.md`,
`letters-dms.md`, `ephemeral-channels.md`, `sync-queue.md`, `governance.md`

## Purpose

Layout primitives define the outermost chrome: fixed panels, their
dimensions, how they swap and slide, and the gesture / keyboard contract
for moving between them. This spec owns *where* content goes. Every
sibling child spec owns *what* that content looks like.

The target is a reading-room shell — calm by default, three panes on
desktop, one pane deep on mobile. Anything that isn't the reading
surface recedes until the user asks for it.

## Scope

**In scope**

- Desktop three-pane shell: grove rail + channel sidebar + main pane,
  plus an optional right rail hosting exactly one of {members, thread,
  pinned} at a time.
- Main pane chrome: channel header strip and its action bar.
- Mobile shell: bottom tab bar, swipe drawer, full-screen pushes,
  bottom sheets, safe-area handling.
- Single breakpoint at 720 px. No tablet mode.
- Focus order and keyboard shortcuts that move between panes.
- Edge cases: rail overflow, zero groves / channels, narrow desktop,
  long names.

**Out of scope (owned elsewhere)**

- Message rendering, hover toolbar containers, action-sheet containers (`message-row.md`).
- Composer, reply preview bar, edit bar, typing indicator (`composer.md`).
- Reactions strip, emoji picker, pinned panel contents (`reactions-pins.md`).
- File cards, inline images, voice notes, upload dialog (`files-inline.md`).
- Thread parent card, reply list (`thread-pane.md`).
- Profile popover / sheet internals (`profile-card.md`).
- Command palette content (this spec only reserves the ⌘K search button).
- Tweaks panel (`settings-tweaks.md`), call surface (`call-experience.md`).
- Sync queue pull-down reveal internals (`sync-queue.md`); layout provides
  only the scroll container contract.

## Desktop layout

### Viewport

- Minimum: 960 × 600. Comfortable: ≥ 1280 × 800. Above 1680 px the
  shell does not grow; excess width becomes main-pane gutter.

### Panel stack

```
┌──────┬────────────┬─────────────────────────┬───────────────┐
│grove │ channel    │ main pane               │ right rail    │
│ rail │ sidebar    │ channel header          │ members |     │
│      │            │ message list (messaging)│ thread  |     │
│ 68px │ 232 px     │ composer                │ pinned        │
│ bg-0 │ bg-1       │ bg-0                    │ 280 px  bg-1  │
└──────┴────────────┴─────────────────────────┴───────────────┘
```

Widths are fixed; the main pane flexes between the channel sidebar and
the right rail (or the viewport edge when the rail is closed). Heights
are 100 vh. No top title bar in v1 — window chrome is the browser's.

### Panel dimensions

| Panel | Width | Background | Border |
|-------|-------|------------|--------|
| grove rail | 68 px | `--bg-0` | right: 1 px `--line-soft` |
| channel sidebar | 232 px | `--bg-1` | right: 1 px `--line-soft` |
| main pane | flex | `--bg-0` | none |
| right rail | 280 px | `--bg-1` | left: 1 px `--line-soft` |

### Grove rail

Leftmost panel — the only always-visible navigation surface on desktop.
Contents top-to-bottom:

1. 4 px top spacer.
2. **Letters tile** — `inbox` icon, tooltip `letters · direct messages`.
   Active on `letters-dms` route.
3. 1 × 28 px `--line` divider, 6 px vertical margin.
4. **Grove tiles** — one per joined grove, in `willow-state` order.
   Each 44 × 44 with the grove glyph in `--font-display` italic 17 px.
5. **New grove tile** — `plus` icon, tooltip `new grove`.
6. **Discover tile** — `compass` icon, tooltip `discover`, accent
   `--amber` on active.
7. Flex spacer.
8. **Settings tile** — `settings` icon, tooltip `settings`.

#### Grove tile anatomy

Four visual states. Transitions use `--motion` (180 ms) on
`border-radius`, `background`, `color`.

| State | radius | background | foreground | indicator |
|-------|--------|------------|------------|-----------|
| idle | 22 px (circle) | `--bg-2` | `--ink-1` | none |
| hover | 14 px | grove accent | `#14130f` | none |
| active | 12 px | grove accent | `#14130f` | 3 × 22 px bar, `--ink-0`, left −12 px |
| with-unread (idle) | 22 px | `--bg-2` | `--ink-1` | 3 × 8 px *pebble*, `--ink-0`, left −12 px |

The unread indicator is shorter than the active bar (8 vs 22 px) — a
pebble, not a stripe — so it doesn't compete with the selection cue.

The grove `accent` is read from `willow-state`; default `--moss-2`.

Keyboard: the rail is a single tab stop with roving arrow keys between
tiles, Enter activates, Home / End jump to ends. `aria-label="groves"`;
each tile's label is the grove name plus `" · current"` when active.

### Channel sidebar

Shows the currently-selected grove. Contents top-to-bottom:

1. **Grove header** (108 px, shrinks to 92 when topic absent).
   - 26 × 26 glyph tile in grove accent, radius 8, italic glyph.
   - Grove name: `--font-display` 16 px / 500, `--ink-0`.
   - Italic "grove" chip: display italic 10.5 px `--moss-3` on moss-1-
     tinted pill. Tooltip: `a grove is a small private network of peers
     — no central server`.
   - Status row at 11 px `--ink-3`: `<members> peers` with `users` icon,
     `·`, then `e2e` with `lock` icon.
   - Tagline in display italic 10.5 px `--ink-4`, default
     `not a server — held between us` (quoted verbatim from the
     reference bundle; load-bearing copy — see `foundation.md` §Copy
     voice).
   - 26 × 26 chevron-down button opens grove menu (owned by `governance.md`).
   - Bottom: 1 px `--line-soft`.
2. **Channel scroll region** — fills remaining height.
3. **Me strip** — 58 px card (`--bg-2`, `--line-soft` border, radius 10).
4. **Net status footer** — 32 px, top border 1 px `--line-soft`.

#### Channel groups

Four canonical labels in this order when non-empty: `commons`
(`--ink-3`), `voice` (`--moss-3`), `ephemeral` (`--whisper`, trailing
`hourglass` at 11 px), `archives` (`--ink-3`).

Labels: 10.5 px / 600 / uppercase / letter-spacing 1.2, with a 10 px
`chevron` that rotates 90° when open. Clicking toggles. Empty groups
are hidden. A grove with no channels falls through to the zero-channels
empty state.

#### Channel row variants

Row: 32 px tall, 6/10 px padding, 4 / 6 px margins, 6 px radius, 13.5
px `--font-ui`. Structure: `[unread-bar?][kind-icon][name][inline-chip?]
[trailing]`.

| Variant | Icon | Trailing | Notes |
|---------|------|----------|-------|
| text idle | `hash` | — | `--ink-3` |
| text unread | `hash` | count pill | `--ink-1`, 3 × 16 px left bar `--ink-0` |
| text current | `hash` | pill (if any) | `--bg-3`, `--ink-0` |
| voice | `volume` | `<n> listening` pulse chip when `active > 0` | 6 × 6 `willowPulse` dot `--moss-3`, chip bg `color-mix(in oklab, var(--moss-2) 18%, var(--bg-2))`, border `--moss-1` |
| ephemeral | `hourglass` | mono timer chip e.g. `2h 14m` | chip in `--whisper` |
| muted | inherited | `mute` 13 px | color drops to `--ink-4` |

Hover (non-current): bg `--bg-2`, color `--ink-1`. Unread pill: radius
10, `--moss-2` bg, `#14130f` ink, 11 px / 600, min-width 18; > 99
renders `99+`. Unread hidden on current channel.

Keyboard: single tab stop, arrow keys move between rows (skipping
collapsed groups), Right / Left toggle parent group, Enter selects.
`aria-label="channels"`; each row is a link labelled `<kind> channel
<name>`, suffixed ` · <n> new` or ` · <n> listening`.

#### Me strip

Left-to-right:
- 30 × 30 circular avatar with a 10 px status dot bottom-right
  (foundation `StatusDot`).
- Column: display name in 13 px / 500 `--ink-0` with a 10 px verified
  badge (from `trust-verification.md`); beneath, short three-word
  fingerprint in `--font-mono` 10.5 px `--ink-3`, format
  `word·word·word`, ellipsised.
- 26 × 26 buttons `mic` (`mic` tooltip), `headset` (`deafen`).

Clicking the strip opens the self profile (owned by `profile-card.md`).

#### Net status footer

32 px row, `--font-mono` 11 px `--ink-3`, 8 / 14 padding, top border
1 px `--line-soft`. Left-to-right: `Pulse` glyph in `--moss-2`, `<n>
peers` in `--moss-3`, `·`, relay id (e.g. `relay-3`), `·`, round-trip
hint (e.g. `184ms`).

When offline: pulse drops to `--ink-4`, peer count omitted, relay +
rtt replaced by `queued · waiting for peers` in `--amber`. Full queue
chrome lives in `sync-queue.md`.

### Main pane header

Full main-pane width, 52 px, on `--bg-0` with 1 px `--line-soft`
bottom border. Left-to-right:

1. Kind icon (`hash` / `volume` / `hourglass`), 17 px stroke 1.5, `--ink-2`.
2. Channel title, `--font-display` italic 17 px / 500, `--ink-0`.
3. Topic, `--font-ui` 12 px `--ink-3`, prefixed by a 1 × 14 px `--line`
   separator bar; truncated.
4. Flex spacer.
5. **Action bar** — 32 × 32 buttons, gap 2 px, fixed order:

    | Icon | Label | Purpose |
    |------|-------|---------|
    | `users` | `members` | toggle right-rail members list |
    | `pin` | `pinned` | toggle right-rail pinned panel |
    | `thread` | `thread` | toggle thread pane (disabled with no selection) |
    | `phone` / `phone-off` | `join call` / `leave call` | call toggle |
    | `search` | `search (⌘K)` | opens command palette |
    | `more-horizontal` | `more` | opens channel menu |

Active: bg `--bg-3`, color `--ink-0`. Only one of
`members` / `pinned` / `thread` is active at a time. On voice channels
during a call, those three buttons and `phone` merge with the call
chrome (owned by `call-experience.md`). On ephemeral channels, a mono
timer chip sits between the title and topic, styled like the sidebar
chip.

### Right rail

280 px wide on `--bg-1`. Closed by default. Hosts exactly one of:

| Pane | Opened by |
|------|-----------|
| members list | `users` action button |
| pinned panel | `pin` action button |
| thread pane | opening a thread from a message |

Opening any closes the others. Rail slides in from right via
transform-only over 180 ms (`--motion`); pointer events re-enabled at
animation end. Internals owned by `profile-card.md`,
`reactions-pins.md`, and `thread-pane.md` respectively.

The members rail borrows the member-row primitives (avatar + italic
display name + steward tag + status + queued-count meta) defined in
`profile-card.md` plus this spec; no separate spec owns it. Member row
anatomy lives here; per-peer behaviour (popover open, verified badge,
private nickname) lives in `profile-card.md` + `trust-verification.md`.

### Desktop transitions

- Switch channels within a grove: immediate.
- Switch groves: immediate; channel sidebar re-renders; main pane
  auto-selects the grove's first unread channel, falling back to the
  first channel in the first group.
- Right rail open / close: transform slide 180 ms.
- Right-rail swap (members → thread): 120 ms cross-fade in place; no
  slide-out-and-back.
- Hover feedback: `--motion-fast` (120 ms).

## Mobile layout

### Viewport

Mobile chrome applies at viewport width ≤ 720 px. The shell consumes
`env(safe-area-inset-top)` above content and `env(safe-area-inset-
bottom)` below the tab bar.

### Shell shape

```
┌──────────────────────────────────────┐
│ safe-area-inset-top                  │
├──────────────────────────────────────┤
│ screen content (one route at a time) │
├──────────────────────────────────────┤
│ bottom tab bar (primary routes only) │
│ safe-area-inset-bottom               │
└──────────────────────────────────────┘
```

The tab bar is visible on the four primary routes only:

| Tab id | Label | Icon | Destination |
|--------|-------|------|-------------|
| `home` | "groves" | `hash` | home screen: current grove + channels |
| `dms` | "letters" | `thread` | letter list (`letters-dms.md`) |
| `discover` | "discover" | `compass` | grove discovery (`discover.md`) |
| `you` | "you" | `user` | self profile + settings (`profile-card.md` + `settings-tweaks.md`) |

Non-primary routes (channel chat, thread, call, bottom sheets,
onboarding) push a full-screen surface that hides the tab bar.
Returning restores the bar with its previously-active tab.

### Tab bar

- Height 52 px + `env(safe-area-inset-bottom)`. Top border 1 px `--line-soft`.
- iOS bg: `color-mix(in oklab, var(--bg-1) 85%, transparent)` with
  `backdrop-filter: blur(16px)`. Android bg: solid `--bg-1`.
- Four tabs flex-1; column of icon (20–22 px) + label (10–11 px).
  Active `--moss-3`, inactive `--ink-3`; active label jumps to 500 weight.
- Android only: active tab draws a 54 × 28 px pill behind the icon in
  `color-mix(in oklab, var(--moss-2) 22%, transparent)`, radius 14
  (Material-3 cue without impersonating Material chrome).
- Badge (any tab): top-right of icon, 14 × 14 min, radius 7, `--err`
  bg, `#14130f` ink, 9 px / 600.

### Platform-agnostic native reading

The shell adopts the *cues* that read native without faking native
chrome. iOS gets the blurred bottom bar (its bottom-bar shorthand).
Android gets the active-tab pill (Material-3 shorthand). The top
status strip is not painted — no fake notch, no fake time, battery, or
signal; safe-area insets absorb whatever the OS draws. Likewise the
home indicator is not painted. Typography, spacing, and colour are
identical across iOS and Android — both inherit the foundation tokens.
The goal is "reads native without impersonating native."

### Top bar (mobile)

Primary routes: 52 px tall, `--bg-1` bg, 1 px `--line-soft` bottom,
10 / 14 padding.

- **Left slot** (32 × 32): grove glyph tile on `home`; back chevron on
  pushed screens; hamburger on routes lacking a back target.
- **Title**: `--font-display` italic 17 px / 500 `--ink-0`, with
  subtitle in `--font-ui` 11 px `--ink-3` (e.g. `42 members · 6 online`).
- **Right slot**: up to two 32 × 32 icon buttons (commonly `search`,
  `bell`).

Tapping the grove glyph on `home` opens the grove drawer.

### Grove drawer

Left-edge drawer overlaying the current screen, home route only.

- 280 px wide. `--bg-1` bg, right border 1 px `--line-soft`, shadow
  `10px 0 40px rgba(0,0,0,0.4)`.
- Transform `translateX(-100%) → translateX(0)` over 240 ms (`--motion-slow`).
- Backdrop `rgba(0,0,0,0.55)`, 200 ms fade.

Internal layout (left → right):

1. 64 px grove rail column, vertically scrolling, tiles with the
   desktop anatomy on `--bg-0`; active tile keeps the left indicator bar.
2. 216 px body column:
   - Wordmark (18 px italic) plus `{n} groves · {m} peers online` at
     `--ink-3` 11 px.
   - Scroll of grove rows: 30 × 30 accent glyph tile, name, member
     count subline.
   - Footer me strip: 28 × 28 avatar, `you` in 12 px `--ink-0`,
     identifier in `--font-mono` 10 px `--moss-3` (e.g.
     `willow · phone`), trailing `settings` icon.

Open: tap the home grove glyph, or swipe right from the left edge on
`home` (commit at > 60 px; snap back otherwise).
Close: tap backdrop, select a grove, swipe left > 60 px, or Escape.

### Full-screen pushes

Routes: channel chat, thread, call, onboarding. Animation
`translateX(100% → 0)` over 240 ms `--motion-slow` with `--motion`
easing. Back is the reverse — swipe right from the left edge > 60 px,
tap back chevron, or Escape. Edge-swipe means "open drawer" on home
only; everywhere else it means back.

### Bottom sheets

Mobile equivalent of desktop popovers.

- 100 vw wide, top corners `--radius-l`. `--bg-1` bg, top border 1 px
  `--line`, shadow `--shadow-2` (vertically flipped).
- Centred 4 × 36 px handle in `--ink-4`, 8 px from the top.
- Animation `translateY(100% → 0)` over `--motion-slow` with a
  `rgba(0,0,0,0.55)` backdrop fade.
- Dismiss: swipe down > 120 px, tap backdrop, or Escape.

Variants this spec ships:
1. **Action sheet** — long-press on a message / channel row / grove row;
   content owned by the consuming child spec.
2. **Profile sheet** — tapping any avatar or peer name; content owned
   by `profile-card.md`.

### Mobile gestures

| Gesture | Context | Effect |
|---------|---------|--------|
| swipe-right from left edge | home route | open grove drawer |
| swipe-right from left edge | pushed screen | back |
| swipe-left inside drawer | drawer open | close drawer |
| swipe-down on sheet handle | sheet open | dismiss |
| long-press on message | channel chat | open action sheet (`message-row.md`) |
| pull-down on message list | scroll top, threshold 64 px | reveal sync queue row (`sync-queue.md`). Layout guarantees only the scroll container contract: the list is a scroll region whose `scrollTop <= 0` and overscroll drag are measurable by child code. |

All gestures degrade under `prefers-reduced-motion`: transforms collapse
to opacity cross-fades at the same duration.

### Mobile transitions

- Primary-route switch (tab change): 120 ms cross-fade.
- Pushed screen: slide left over 240 ms.
- Drawer: slide right over 240 ms.
- Sheet: slide up over 240 ms.

## Window / viewport breakpoints

Exactly one breakpoint in v1:

- ≤ 720 px: mobile chrome.
- > 720 px: desktop chrome.

No tablet mode. A 768 px iPad in portrait renders the desktop three-
pane at its minimum size (68 + 232 + 420 + 0 = 720 px usable main
pane). A 600 px phone in landscape renders the mobile chrome. Landscape
phones simply fit the mobile chrome wider.

Implemented as a `min-width: 721px` media query — a single toggle
point. SSR is not required; the shell is chosen client-side after
first paint.

## States + transitions (cross-cutting)

### Right-rail swap

Only one of `members` / `pinned` / `thread` is visible. Swap:
outgoing fades to `opacity: 0` over 120 ms, unmounts; incoming mounts
and fades in over 120 ms. The rail itself does not move — preserves
the user's anchor and avoids a double slide.

### Main pane ↔ call surface

On voice channels, joining the call replaces the main-pane content
with the call surface (owned by `call-experience.md`). The header
keeps the channel title but swaps `phone` → `phone-off`, and members /
pinned / thread buttons hide. Leaving restores the chat main pane.

### Narrow-desktop collapse

- < 960 px wide: right rail closes automatically if open; reopening
  overlays the main pane (position absolute, `rgba(0,0,0,0.4)` backdrop).
- < 840 px: channel sidebar collapses to a 60 px glyph-only column;
  grove rail keeps 68 px. An escape hatch, not a mode.
- ≤ 720 px: mobile chrome takes over.

## Copy

Strings owned by layout-primitives. `{n}` / `{m}` substituted at
runtime. Lowercase unless quoting a proper noun.

### Tooltips / aria-labels (desktop)

| Surface | Label |
|---------|-------|
| rail · letters | `letters · direct messages` |
| rail · new grove | `new grove` |
| rail · discover | `discover` |
| rail · settings | `settings` |
| header · members | `members` |
| header · pinned | `pinned` |
| header · thread | `thread` |
| header · call (idle / active) | `join call` / `leave call` |
| header · search | `search (⌘K)` |
| header · more | `more` |
| me strip · mic / deafen | `mic` / `deafen` |
| grove menu chevron | `grove menu` |

### Grove header

Default tagline under the grove status row:

> not a server — held between us

"grove" chip tooltip:

> a grove is a small private network of peers — no central server

### Section labels

`commons`, `voice`, `ephemeral`, `archives` — rendered lowercase with
CSS uppercase-transform. The `ephemeral` trailing label on mobile home
is `self-destructs` (`--amber`, 9 px `--font-mono`).

### Empty states

| Surface | Copy |
|---------|------|
| zero groves (desktop) | centred block in the main-pane flex region: `no groves yet. start one, or discover.` with CTA `new grove`. |
| zero groves (mobile home) | `no groves yet — start one` with CTA. |
| zero channels | main-pane italic display 22 px `--ink-2`: `this grove is quiet. say hi?` with 13 px `--ink-3` sub-line: `add a channel from the grove menu.` |
| zero members in right rail | italic display 17 px `just you so far`, sub-line `invite someone` linking to grove menu. |

### Net status footer — offline

Footer replaces relay / rtt with `queued · waiting for peers` in
`--amber` mono 11 px.

### Tab bar labels (mobile)

`groves`, `letters`, `discover`, `you`.

### Grove drawer

- Wordmark header, no tagline.
- Summary at `--ink-3` 11 px: `{n} groves · {m} peers online`.
- Me strip: `you` in `--ink-0`, subline `willow · <device>` in mono
  `--moss-3` (e.g. `willow · phone`, `willow · laptop`). Device label
  is editable in settings.

## Data dependencies

This spec consumes `willow-state` reads; it declares no new events or signals.

| Surface | Reads | Source |
|---------|-------|--------|
| grove rail tiles | joined groves; per-grove glyph, accent, unread | `ServerState` aggregate per joined grove via `willow-client::ClientHandle` |
| channel sidebar · grove header | name, member count, e2e status | `ServerState::metadata` |
| channel sidebar · channel groups | channel kind / name / unread / timer / listener count / muted | `ServerState::channels`, `ChannelViewState` for unread, voice participants from `willow-client` voice membership |
| me strip | display name, avatar, short fingerprint, verified, status | `willow-identity::Identity`, `willow-state` display-name events, `trust-verification.md` verify state |
| net status footer | peer count, relay id, rtt, online / queued | `willow-network` connection status + `willow-client` network state |
| main pane header | channel name / kind / topic / ephemeral timer | `ServerState::channels` for current channel |
| tab bar badges | per-route pending (letters unread) | aggregate across grove letter states |
| right rail · members | members + presence | `ServerState::members` + `willow-network` presence |
| grove drawer summary | grove count + peers online | per-grove `ServerState` + presence roll-up |

Reads are observed via existing Leptos signals on `AppState`. Where a
signal does not yet exist, the contract is that `willow-client` exposes
the data as a read-only stream and the derivation lands in
`state::wire_derived_signals`. Any future `EventKind` (ephemeral
timers, grove accent, section ordering) is owned by the consuming
child spec, not here.

## Edge cases

**Very long grove names.** Channel-sidebar header uses
`text-overflow: ellipsis` / `white-space: nowrap`; tooltip on truncation
shows the full name. Drawer row: same treatment. Desktop rail tile
always renders a single-character glyph (first grapheme of first word,
upper-cased), so long names never affect it.

**Zero groves.** Desktop: rail keeps its letters / divider / new-grove /
discover / settings tiles; channel sidebar is not rendered; the main
pane becomes full flex and shows the zero-groves empty state. Mobile:
home route shows the zero-groves empty state centred; tab bar stays.

**Zero channels.** The channel scroll region renders no group labels;
instead a single italic display line centred 40 px from the top reads
`this grove is quiet.` with sub-line `add a channel from the grove
menu.` The main pane shows the zero-channels empty state.

**Grove rail overflow.** The rail scrolls vertically (scrollbar hidden
via `.noscroll`). Letters / divider / new-grove / discover scroll with
the grove tiles; the settings tile is pinned in a flex row below the
scroll region so it never disappears.

**Narrow desktop (≤ 840 px).** Right rail overlays main pane rather
than flexing; channel sidebar collapses to 60 px glyph-only with
tooltip-reveal of name. At ≤ 720 px the mobile shell takes over.

**Pull-to-reveal precision.** Only a direct over-scroll pull (64 px
positive overscroll) triggers the sync queue reveal; inertial scroll-
up does not. The scroll container contract guarantees this so that
`sync-queue.md` can wire the rest.

**Many-channel grove.** Sidebar simply scrolls; no "jump to unread"
button in v1. ⌘K command palette is the escape hatch.

**Channel name with emoji.** Plain text rendering; emoji fall back to
`ui-sans-serif` inside `--font-ui`. Mentions in channel names are
disallowed at event level (enforced by `governance.md`).

## Command palette

The command palette is a cross-surface jump + action surface. It belongs
to this spec rather than a standalone file because it is fundamentally
part of the shell chrome — the same input that jumps between channels
also reaches every action and hands search off to `local-search.md`.

### Entry

- Desktop keybind: `⌘K` on macOS, `Ctrl-K` elsewhere.
- Desktop click: the `search (⌘K)` button reserved in the main-pane
  header (see §Main pane header) and in the mobile top bar right slot.
- Mobile: tap the search button in the top bar; optional swipe-down
  gesture from the top edge is reserved for v2.

### Anatomy

- Centered overlay; `z-index` above all panes but below toasts.
- Max width `560px`; top-third vertically placed on desktop; full-width
  sliding from top on mobile (with `--radius-l` bottom corners).
- Container: `--bg-1` background, `1px solid --line`, `--radius-l`,
  `--shadow-2`. Motion: `willow-pop-in` at `--motion` (180 ms).
- Structure: search input (top, 48 px), results list (scrollable),
  footer hint strip (bottom, `--ink-3`, small).
- Backdrop: 40 % `--bg-0` with `backdrop-filter: blur(4px)`.

### Input

- Placeholder: `jump or search…`
- Modifier prefixes:
  - `#` — scope to channels (any grove)
  - `@` — scope to peers and letters-by-peer
  - `>` — scope to actions
  - (empty prefix) — mixed: channels + letters + groves + people + actions, plus a `search` entry that delegates to `local-search.md`
- Quote a phrase to force literal: `"reading list"`.
- `Esc` clears if non-empty, else closes.
- `↑ / ↓` move selection, `Enter` activates.

### Result groups

Results render grouped by kind with small group labels (meta style).
Each row is a single focusable element announcing its kind and label.

| Group | Row anatomy | Activate |
|-------|-------------|----------|
| Channels | `#` icon + channel name + grove meta in `--ink-3` | open channel |
| Letters | avatar(s) + name + last-message snippet in `--ink-3` | open letter thread |
| Groves | glyph + name + member count | switch to grove |
| People | avatar + display name + handle | open profile card |
| Actions | icon + label (see table below) | run action |
| Search | magnifier + `search "{q}" in {scope}` | hand off to `local-search.md` with the scope ladder |

### Actions catalog (v1)

| Action | Label | Notes |
|--------|-------|-------|
| tweaks | `open tweaks` | opens `settings-tweaks.md` tweaks panel |
| settings | `open settings` | opens `settings-tweaks.md` settings modal |
| new channel | `new channel…` | gated on ManageChannels |
| new letter | `write a letter…` | always available |
| create grove | `new grove…` | always available |
| sync queue | `open sync queue` | opens `sync-queue.md` screen |
| move this call | `move this call` | only when a call is active (delegates to `device-handoff.md`) |
| toggle theme | `toggle light / dark` | deferred until light theme ships |
| sign out | `sign out` | confirm-dialog guarded |

Actions are feature-flagged so the catalog grows as surfaces land.

### Recents

When the input is empty and the user has used the palette before, show
up to 8 recents (any group). Recents are local-only, toggleable in
`settings-tweaks.md` privacy (`remember palette recents`, default on).

### Empty / loading / error states

| State | Body |
|-------|------|
| empty + no recents | `jump or search across willow — try #channel, @peer, > for actions` |
| empty + has recents | recent list |
| no matches | `nothing matches '{q}' — try > for actions or /search` |
| search running | `searching… (local only)` (live region) |
| error | `search indexer is rebuilding — try again in a moment` |

### Copy (exact)

- Placeholder: `jump or search…`
- Empty hint: `jump or search across willow — try #channel, @peer, > for actions`
- No-match: `nothing matches '{q}' — try > for actions or /search`
- Search running: `searching… (local only)`
- Error: `search indexer is rebuilding — try again in a moment`
- Footer hint strip: `↑↓ move · ⏎ open · esc close`

### Accessibility

- Role `dialog` with aria-label `command palette`; focus trapped inside.
- Input → results list is a `combobox` pattern with `listbox` results
  (`aria-activedescendant` tracks selection).
- Each result row has a concrete name + kind announced via screen reader.
- Motion-reduced: no scale animation; opacity only.

### Data dependencies

- Channels / groves / letters / people enumerated from existing state
  signals (no new events).
- Action catalog is static, gated by permission signals already in use.
- Search delegates to `local-search.md` — palette contributes scope
  but does not own the index.

## Accessibility

### Focus order

Desktop: grove rail (composite, arrow keys between tiles) → channel
sidebar (grove chevron → channel list composite → me strip mic →
deafen) → main pane (message list → composer, composer is owned by
`composer.md`) → right rail (close button → content) → overlay chrome
(focus-trapped). Mobile: top bar (back / menu → right slot) → body →
tab bar (roving index).

### Keyboard shortcuts

| Keys | Action |
|------|--------|
| ⌘K / Ctrl-K | open command palette |
| Esc | close right rail, else sheet, else drawer, else palette |
| Tab | cycle landmarks above |
| ↑ / ↓ (rail) | move between grove tiles |
| ↑ / ↓ (channel list) | move between rows |
| ← / → (channel list) | collapse / expand group |
| Alt+↑ / Alt+↓ | prev / next grove (optional v1) |

### ARIA landmarks

| Region | role | aria-label |
|--------|------|------------|
| grove rail | `navigation` | `groves` |
| channel sidebar | `navigation` | `channels` |
| main pane header | `banner` | `channel header` |
| main pane body | `main` | `{channel name}` |
| right rail | `complementary` | `members` / `pinned` / `thread` |
| mobile tab bar | `navigation` | `primary` |
| grove drawer | `dialog` (`aria-modal="true"`) | `groves` |
| bottom sheet | `dialog` (`aria-modal="true"`) | sheet-specific |

### Touch targets

All interactive elements in mobile chrome ≥ 44 × 44 CSS px in hit
area, regardless of visual size.

### Reduced motion

Rail hover radius animation removed (colour only). Right rail slide →
180 ms opacity cross-fade. Drawer and sheet slide → 240 ms opacity
fade. Pull-to-reveal transform removed; the queue row appears
instantly once threshold is reached.

### Colour-independent cues

Every state pairs colour with a shape or icon:

- Active grove: accent fill *and* left indicator bar.
- Unread grove: `--bg-2` fill *and* short pebble indicator.
- Active channel row: `--bg-3` fill *and* `--ink-0` type.
- Unread channel row: `--ink-1` type *and* left bar *and* count pill.
- Offline net status: `--amber` text *and* word `queued` *and* dropped
  pulse glyph.

## Acceptance criteria

- [ ] Desktop shell renders panels at 68 / 232 / flex / 280 px; single
      breakpoint honoured at 720 px.
- [ ] Grove rail tile hover / active transitions via border radius
      within 180 ms.
- [ ] Right rail hosts only one of members / thread / pinned; toggling
      one closes the others.
- [ ] Main pane header renders the six action buttons in order, with
      `search` reserved for ⌘K.
- [ ] Channel sidebar renders the four canonical groups in order;
      empty groups are hidden.
- [ ] Ephemeral rows show a mono timer chip; voice rows with `active >
      0` show a pulsing listener chip; muted rows show the mute icon.
- [ ] Mobile shell renders four tabs on primary routes and hides the
      tab bar on pushed screens.
- [ ] Swipe-right from the left edge opens the drawer on home only;
      everywhere else it navigates back.
- [ ] Long-press on a message opens an action sheet via this spec's
      contract (internals in `message-row.md`).
- [ ] Pull-down on the mobile home scroll container overscrolls
      measurably past the 64 px threshold without triggering any
      layout-owned action.
- [ ] Safe-area insets apply top and bottom; tab bar sits above any OS
      home indicator.
- [ ] All focusable elements show `--focus-ring` under keyboard focus.
- [ ] `prefers-reduced-motion: reduce` collapses all transform-based
      transitions to opacity or instant.
- [ ] ARIA landmarks apply as declared.
- [ ] Touch targets ≥ 44 × 44 CSS px on every mobile interactive.
- [ ] Long names truncate with ellipsis and expose full text via
      tooltip (desktop) or push-screen title (mobile).
- [ ] Offline state demotes the net status footer to `queued · waiting
      for peers` in `--amber` without removing the footer.

## Open questions

- **Per-grove accent scope.** The design bundle stores `accent` on
  each grove. This spec assumes accent affects only the grove's own
  tile fill and header underline, not the whole app. Decision needed
  before `governance.md` ships edit UI.
- **Jump-to-unread.** Deferred in v1; ⌘K is the escape hatch. Reopen
  if users cite it.
- **Collapsed desktop sidebar (≤ 840 px).** The 60 px glyph-only
  fallback is described but not prototyped. May need to drop listener
  / timer chips to tooltips if layout squeezes.
- **Empty-group ordering.** Spec hides empty channel groups; the
  design bundle always shows them. Hide reduces noise on empty groves;
  confirm with governance owners.
- **Tab bar on iPad portrait.** Currently desktop shell applies. A
  Tweaks toggle for mobile shell on larger viewports is deferred.
- **Bottom sheet stack.** v1 answer: only one sheet at a time; an
  action sheet dismisses before a profile sheet opens. Confirm with
  `profile-card.md` owners.
- **Backdrop-filter fallback.** Modern iOS Safari + Android Chrome
  support it; older Android falls back to the solid `--bg-1` tab bar
  (which is the Android path anyway).
