# Local search — on-device, encrypted-at-rest, whole-client

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md), [`message-row.md`](message-row.md)
**Consumed by:** [`letters-dms.md`](letters-dms.md), [`message-row.md`](message-row.md), [`composer.md`](composer.md), [`discover.md`](discover.md)

## Purpose

Willow has no server-side search. Every query — across groves,
channels, letters, and historical messages — runs against an index
that lives only on the user's device and is built from the same
encrypted-at-rest stores as the rest of the client. This is a
privacy-defining property of the product: queries never leave the
device, never hit the relay, and never produce telemetry.

This spec defines the shared search primitive consumed by every
surface that needs to find things across the local corpus. Letters,
channel message history, and the discover surface's optional
message-search mode all delegate here. Grove directory search
(`discover.md`) is separate: it searches the public directory, not
the local corpus, and is owned there.

The search UI is intentionally quiet. Trust-first / reading-room
cadence from `foundation.md` applies: search is a tool, not a
dashboard; results prioritise context and legibility over density.

## Scope

- Scope ladder (`this letter` / `this channel` / `all letters` /
  `all groves + letters`), default behaviour, persistence.
- Entry points on desktop (keyboard shortcuts, top-right slot) and
  mobile (pull-down, per-surface shortcut, command palette bridge).
- The on-device index: build, rebuild, horizon, storage location.
- Query language (plain text + prefix operators + quoted phrases).
- Results presentation, grouping, navigation, highlight behaviour.
- Performance envelope and streamed-result UX.
- Privacy copy, empty states, accessibility.
- Data dependencies flagged **new** vs existing.

Out of scope: grove directory search (`discover.md`), the
command-palette atom itself (see `layout-primitives.md` for the
⌘K reservation), server-side or federated search (does not exist).

## Entry points

### Desktop

- **Top-right search slot** (owned by `layout-primitives.md`):
  a single input in the header chrome with the `Search` icon and
  placeholder `search groves + letters`. Focused by pressing `/`
  anywhere except inside a text input or the message composer.
  Submitting opens the results surface over the main pane.
- **Scoped search inside a container**: pressing `⌘F` (macOS) or
  `Ctrl+F` (other) while a channel or letter thread has focus
  scopes the search to `this channel` or `this letter`
  respectively. The input inherits the top-right slot; the scope
  chip flips to the narrow value and the placeholder switches to
  `search this {channel|letter}`.
- **Escape contract**: `Esc` with a non-empty query clears the
  query and keeps focus in the input. A second `Esc` closes the
  results surface and returns focus to the invoking pane.
- **Command-palette bridge (`⌘K`)**: the palette is owned by
  `layout-primitives.md`. Its text field inspects each keystroke:
  - Leading `#` → filter to channels by name; Enter jumps.
  - Leading `@` → filter to peers by handle / display; Enter
    opens the profile card.
  - Anything else → on Enter, forward the string to local search
    with scope `all letters` (the palette closes, results
    surface opens, input is prefilled).

### Mobile

- The search bar is not persistent in list top bars. On the
  letters list, channel list, or message list, a pull-down
  gesture (≥ 44 px drag while `scrollTop ≤ 0`) reveals a sticky
  search bar that pins to the top of the scroll region. Tapping
  `cancel` or pulling the bar back up hides it.
- Inside an open letter or channel thread, the top-bar overflow
  exposes `search this {letter|channel}`; selecting it opens the
  results surface scoped accordingly.
- Tapping the search field anywhere focuses it and opens the
  keyboard; focus behaviour respects the safe-area keyboard inset
  from `layout-primitives.md`.

### Discover bridge

`discover.md` owns grove directory search. When the user types
text that returns zero directory results, discover offers an
inline `search my messages instead →` affordance that delegates
to this primitive with scope `all groves + letters` and the same
query string.

## Scope ladder

A **scope chip** sits directly above the results list. It shows the
active value and opens a small popover on click / tap that lists the
four options:

| Value | Default context | Indexes |
|-------|-----------------|---------|
| `this letter` | Active when a letter thread is focused and search was invoked via `⌘F` or the overflow "search this letter" item. | Only that letter's message history. |
| `this channel` | Active when a channel is focused and search was invoked via `⌘F` or the overflow "search this channel" item. | Only that channel's message history. |
| `all letters` | Default when invoked from the letters list, the top-right slot with no prior context, or the palette bridge. | Every peer- and group-letter on this device. |
| `all groves + letters` | Widest. Default when invoked from anywhere outside the letters surface with no narrower context. | Every grove channel plus every letter on this device. |

Rules:

- The default for each invocation path is listed above. Users may
  widen or narrow the scope at any time via the chip; the chosen
  scope persists **per device** (not per grove, not per letter).
- `this letter` and `this channel` are only selectable when such a
  container is currently focused — otherwise they are greyed in
  the chip popover with tooltip `open a {letter|channel} first`.
- Changing scope re-runs the current query incrementally; results
  don't wipe until the new scan completes.

### Scope chip visual

Pill on `--bg-2` with `--line` border, `--radius-s` radius, 11 px
mono label, 4 px gap between label and a trailing chevron. Active
chip colour is `--moss-3`; inactive variants inside the popover
use `--ink-2`. Selected scope carries a leading `check` glyph.
Reduced motion collapses the chevron rotation to no-op.

## Index

### Where it lives

- **Native**: a dedicated SQLite column (FTS5 virtual table) alongside
  the per-grove / per-letter message stores. Encrypted at rest via the
  same disk-encryption pathway already used for message blobs — the
  index re-uses the per-store key material, it does not introduce a new
  key. **New dependency** (SQLite FTS5 is not currently linked into the
  client; the implementation plan must flag the added build feature).
- **WASM**: an in-memory inverted index (HashMap of token → postings
  list) kept warm for the session. On startup the client paginates
  through decrypted messages and rebuilds; on shutdown it is dropped.
  The horizon cap (below) keeps memory bounded.

In both cases the index consumes **already-decrypted** message payloads
from the existing message-decrypt path — no new crypto primitives are
introduced. Encrypted-at-rest is inherited from the store layer.

### Build behaviour

- **Incremental on arrival**: every newly-decrypted message is added
  to the index synchronously with its append to the message store, so
  live results include messages that arrived seconds ago.
- **Lazy on historical scan**: the first time a scope is searched,
  historical messages are indexed on demand. Streaming results
  (below) let the user start reading matches before the full scan
  completes.
- **Silent incremental rebuild**: if the index detects a schema bump
  or missed inserts (e.g. a crash mid-write), it rebuilds in the
  background without prompting. The status signal drives the
  `indexing… (local only)` placeholder.
- **Explicit rebuild**: settings-tweaks privacy section exposes a
  `rebuild search index` action (see `settings-tweaks.md`). Used when
  the user has reason to suspect drift or after changing the horizon.
  Confirmation: `rebuild the local search index? this takes a minute
  and runs only on this device.` Action: `rebuild`.

### Eviction horizon

- **Default**: 90 days. Messages older than the horizon are not in
  the index (they remain in the message store and can still be
  scrolled to directly).
- **Tweakable**: `settings-tweaks.md` privacy section exposes
  `indexed horizon` with values `30 days`, `90 days` (default),
  `1 year`, `all history`. The last option has a sub-label
  `bigger index · more disk`. Changing the horizon triggers a silent
  incremental rebuild.
- **Per-grove quick toggle**: per-grove settings expose a
  `search this grove's history` toggle (on by default). When off, the
  grove's channels are dropped from the index and from all scopes.
  This is the local equivalent of opting out of archival without
  losing the history itself.

### Status signal

A `SearchIndexBuildStatus` signal (one of `idle`, `building`,
`indexing` + message count, `error`) feeds the "indexing…" placeholder
and the settings diagnostics row. Exposed to consuming specs as a
read-only signal; not user-controlled.

## Query language

- **Plain text**: case-insensitive substring / token match against
  message body, author display name and handle, and channel / letter
  name (scope-dependent — `this letter` does not match against other
  letter names). Tokens split on whitespace; multi-token queries are
  AND-joined.
- **Quoted phrases**: `"two words"` forces exact adjacent match; quote
  chars are stripped from the displayed query echo.
- **Prefix operators**: appear anywhere in the query and AND-combine
  with the free text.

| Operator | Matches | Notes |
|----------|---------|-------|
| `from:@peer` | Author whose handle or display name equals `peer`. | `@` is optional; completion dropdown suggests handles as you type. |
| `in:#channel` | Channel whose name equals `channel`. | Only meaningful in scopes that include channels. |
| `since:YYYY-MM-DD` | Messages timestamped at or after the date. | Local timezone. |
| `before:YYYY-MM-DD` | Messages timestamped before the date. | Local timezone. |
| `has:image` | Messages with an attached image. | |
| `has:file` | Messages with an attached file (non-image). | |
| `has:link` | Messages containing a URL in the body. | |

- **Empty query** is a no-op: results list is empty, the placeholder
  prompt renders, no scan runs, no "no matches" copy.
- **Malformed operator** (e.g. `since:yesterday`, unknown prefix): the
  operator is greyed in the echoed query and the query tooltip reads
  `unknown filter — treated as plain text`. The plain-text fallback is
  applied so the user still sees something.

## Results presentation

### Layout

The results surface takes the main pane on desktop (full width, no
second pane) and the full screen on mobile. Structure top-to-bottom:

1. Search input (sticky).
2. Scope chip + result count (e.g. `47 matches in all letters`).
3. Optional streaming banner (`searching… · 47 matches so far`).
4. Results list, grouped.
5. Privacy footer (below).

### Grouping

Results group by container:

- `all groves + letters` → groups by grove (header: grove name in
  Fraunces italic 14 px) then by channel / letter under each grove.
  Letters cluster under a synthetic `letters` group at the bottom.
- `all letters` → groups by letter (peer or group).
- `this channel` / `this letter` → single implicit group, header
  hidden.

Each group header shows the container name, a count (`{n}`), and a
chevron. Groups default to expanded; collapsing is per-session (not
persisted). Empty groups (zero matches) are never rendered.

### Row anatomy

Each result row is a `<button>` sized ≥ 44 × 44 on mobile. Structure:

1. **Context line** (top, 11 px `--ink-3`):
   `{channel|letter name}` in Fraunces italic · author display name
   in `--font-ui` · `·` · soft timestamp (today → `HH:MM`,
   yesterday → `yst`, older → `{n}d` / `{n}w` / month abbrev).
2. **Body excerpt** (body S, 13 px, `--ink-1`): up to three lines of
   message body centered on the first matched span. Ellipsis on both
   sides if truncated. Matched range is rendered with underline +
   `--moss-3` background at 18 % alpha beneath the matched
   characters.
3. **Right column** (desktop only): small `ArrowUpRight` icon 12 px
   in `--ink-3` signalling "open in place". Hidden on mobile (the
   full row is tappable).

Hover (desktop) raises background to `--bg-2`; pressed (mobile)
`--bg-3` with 120 ms fade. Focus uses `--focus-ring`.

### Navigation

- Click / tap / Enter on a result: the results surface closes,
  focus jumps to the native container (the result's channel or
  letter), the container scrolls so the matched message sits 1/3
  down the viewport, and the message renders with a brief
  `willow-pop-in` highlight (240 ms) plus a persistent underline
  on the matched span for 6 s or until the user scrolls.
- Back / swipe-back / `Esc`: returns to the results surface with
  scroll position preserved.
- Keyboard (desktop): `↑` / `↓` moves the selection within the
  results list; `Enter` opens; `Esc` clears the query, a second
  `Esc` closes the surface.

## Performance envelope

- **First-result latency**: ≤ 150 ms from keystroke to first visible
  row on a 10 000-message index, ≤ 500 ms on a 100 000-message index.
  Benchmarks run on a reference device (mid-range laptop, Firefox
  stable). These are hard numbers for the implementation plan.
- **Streaming**: while a historical scan is still in progress, the
  surface shows `searching… · {n} matches so far` under the scope
  chip. The counter updates at most every 250 ms to avoid thrash.
  Rows append in place as they're found.
- **Debounce**: 120 ms keystroke debounce before the live index is
  queried. Under debounce the prior result set stays visible and
  dims by 15 % to show it's stale.
- **Cancellation**: typing a new query cancels the in-flight scan;
  the stale result set clears as the new scan begins.

Reduced motion: no row fade on streaming append; results snap in.

## Privacy

- **Footer text**, rendered quietly at the bottom of the results
  surface in `--ink-3` 11 px: `search runs on this device only.
  queries never leave your device.` Always present; not dismissable.
- **No telemetry**: the implementation plan must document that no
  analytics, logging, or metrics capture query strings, match counts,
  or scope selection.
- **Local recents**: an optional per-device recent-queries list (up
  to 8 entries) renders as suggestion chips under the empty input.
  Toggled in `settings-tweaks.md` privacy section (`remember recent
  searches · on this device`, default **on**). Individual entries
  can be removed via long-press / right-click → `forget`. A
  `clear all recents` action lives beside the toggle.
- **No cross-device sync**: recents, scope selection, horizon, and
  per-grove search toggles stay local. They do not ride the event
  stream and are not restored on device-handoff.

## Empty states

| State | Copy |
|-------|------|
| Empty input, no recents | `type to search {scope}` (e.g. `type to search all letters`) as placeholder; below it a meta line `searches stay on this device.` |
| Empty input, with recents | Placeholder as above; recents render as chips (`Search` icon + clipped query, max width 180 px, mono font for operator parts). |
| Running first index | `indexing… (local only)` centered with `Hourglass` 16 px in `--amber`. Counter line if available: `{n} of {total} messages`. |
| Running subsequent index | Inline banner under scope chip: `searching… · {n} matches so far`. |
| Scan complete, zero matches | Centered meta: `nothing matches "{q}" in {scope}` with a subline `try a broader scope · or check the indexed horizon in tweaks`. |
| Scope unreachable (e.g. per-grove search disabled) | Centered meta: `this grove's history isn't indexed · turn on in grove settings`. |
| Error (index rebuild failed) | Centered meta: `couldn't rebuild the index. open tweaks to retry.` plus inline `retry` button. |

All strings are lowercase; quoted query echoes use the user's exact
casing. Empty-state strings are owned by *this* spec except the
letter-specific `search letters` placeholder and no-match line, which
are owned by `letters-dms.md`.

## Copy (exact)

Lowercase unless proper noun; no exclamation marks.

- Placeholder, widest: `search groves + letters`
- Placeholder, all letters: `search all letters`
- Placeholder, this channel: `search this channel`
- Placeholder, this letter: `search this letter` *(owned by `letters-dms.md`, listed here for reference)*
- Privacy footer: `search runs on this device only. queries never leave your device.`
- Indexing (first run): `indexing… (local only)`
- Streaming banner: `searching… · {n} matches so far`
- No matches: `nothing matches "{q}" in {scope}`
- No matches subline: `try a broader scope · or check the indexed horizon in tweaks`
- Recents toggle label: `remember recent searches · on this device`
- Recents clear action: `clear all recents`
- Unknown-operator tooltip: `unknown filter — treated as plain text`
- Rebuild-index action: `rebuild search index`
- Rebuild-index confirmation: `rebuild the local search index? this takes a minute and runs only on this device.`
- Open-grove-indexing meta: `this grove's history isn't indexed · turn on in grove settings`
- Rebuild failed: `couldn't rebuild the index. open tweaks to retry.`

## Accessibility

- **Landmark**: the search input is wrapped in a `<form
  role="search" aria-label="local search">`. On open, focus moves
  to the input.
- **Results list**: rendered as `role="listbox"` with rows as
  `role="option"`. The list's `aria-activedescendant` tracks
  keyboard selection so screen readers announce each result as it
  receives focus.
- **Matched span**: the underlined / moss-highlighted range inside
  each excerpt is wrapped in `<mark>` with `aria-label="match"` so
  assistive tech announces "match" around the span. The surrounding
  excerpt is unlabelled so it reads as natural text.
- **Scope chip**: rendered as a `<button aria-haspopup="listbox"
  aria-expanded>`. Popover options are `role="option"` with
  `aria-selected` on the active scope. Keyboard: arrow keys move
  within the popover, Enter selects, Esc closes.
- **Shortcut listing**: `/`, `⌘F` / `Ctrl+F`, and `Esc` appear in
  the global `?` shortcut overlay under a `search` heading. The
  overlay lookup is owned by `layout-primitives.md`; this spec
  declares the entries to add.
- **Announcements**: result-count changes during streaming
  announce politely (`aria-live="polite"`) at most every 500 ms to
  avoid flooding. The `indexing… (local only)` state announces
  once on entry.
- **Focus management**: opening a result moves focus to the
  matched message in its native container; returning to the
  results surface restores focus to the invoking row. Compose
  popovers / overflow menus invoked from the results list trap
  focus and return it to the row on close.
- **Reduced motion**: streaming row append, highlight flash, and
  scope-chip chevron rotation all collapse to instant.
- **Contrast**: `--moss-3` at 18 % alpha on `--bg-1` verified for
  ≥ 3:1 on the underlined match text. Meta (`--ink-3` on `--bg-1`)
  verified ≥ 4.65:1. Match underline is `1.5px` minimum so the
  cue survives even if the background tint is suppressed.

## Data dependencies

Every entry is either **new** (requires a new type / config / store)
or existing (reuses a known primitive).

| Dependency | Status | Owner | Notes |
|---|---|---|---|
| Message decrypt path | existing | `willow-messaging` + `willow-crypto` | Index consumes already-decrypted payloads; no new crypto. |
| Encrypted-at-rest message store | existing | `willow-messaging` | Index inherits the store's key material. |
| `SearchIndexConfig` | **new** | client library | Per-device record: `{ enabled: bool, horizon_days: u32, remember_recents: bool, per_grove_enabled: HashMap<GroveId, bool> }`. Device-local, not synced. |
| `SearchIndexBuildStatus` | **new** | client library | Signal for UI: `idle` / `building` / `indexing{done, total}` / `error{reason}`. Read-only to consumers. |
| FTS5 column (native) | **new** | `willow-messaging` | Virtual table alongside per-store message tables. Implementation plan must flag added SQLite feature. |
| In-memory postings index (WASM) | **new** | client library | Built at session start from decrypted messages; dropped on shutdown. |
| Recent queries list | **new** | client library | Device-local ring buffer, length 8, cleared on `clear all recents`. |
| No new events | — | `willow-state` | Search is purely local; no `EventKind` changes. |

The implementation plan enumerates the build-feature gates (SQLite
FTS5 on native, in-memory fallback on WASM) and the migration path
(first-run populates the index from existing stores).

## Consumer contracts

Each consuming spec binds to this primitive through a narrow surface:

- **`letters-dms.md`**: embeds the shared search input in the letters
  list top area, defaults scope to `all letters`, escalates to `this
  letter` via `⌘F` inside a thread, and owns its own placeholder and
  no-match copy for the letter-specific scope.
- **`message-row.md` and `composer.md`**: expose `⌘F` inside a
  channel to scope search to `this channel`. `message-row.md` renders
  the match highlight in message bodies when scrolled into view from a
  result tap.
- **`discover.md`**: owns grove directory search but offers the
  "search my messages instead" fallthrough that delegates here with
  scope `all groves + letters`.
- **`layout-primitives.md`**: owns the top-right search input slot
  and the ⌘K command-palette slot. Local search consumes both.
- **`settings-tweaks.md`**: owns the privacy UI for horizon,
  per-grove toggles, recents toggle, and rebuild action. Local
  search exposes the config record and rebuild entry point.

Other specs are free to delegate to this primitive — no new
consumer requires a spec change here, only a new row in the
"Consumed by" list.

## Edge cases

- **Per-grove search disabled**: results simply omit that grove;
  group headers don't render. If the user selects `this channel`
  inside such a grove, the results surface shows the
  `this grove's history isn't indexed` meta with an inline
  `enable for this grove` link that deep-links to grove settings.
- **Horizon shrink mid-scan**: if the user reduces the horizon
  while a scan is running, the scan cancels and restarts with the
  narrower bound. The banner reads `reindexing · narrower
  horizon…`.
- **Very large groves** (> 1M messages): the 500 ms budget applies
  per scope, not per grove. If `all groves + letters` scope
  exceeds budget during first-result, stream starts immediately
  and the banner reads `scanning… this might take a moment`.
- **Empty horizon** (`all history` turned on, then off): messages
  that were searchable become unsearchable; a one-time toast on
  next scope selection reads `horizon shortened · older messages
  no longer searchable`.
- **Crash during rebuild**: on next startup, the status signal
  reports `error`; the results surface shows the rebuild-failed
  meta. Searches still run against whatever partial index exists
  (graceful degradation).
- **Peer deleted identity**: messages from a peer whose identity
  was deleted remain in the index; the context line renders the
  peer's last-known display name plus a `(gone)` suffix in
  `--ink-4`.
- **Encrypted-to-another-device messages**: if a message is
  present in the store but couldn't be decrypted locally (e.g.
  key not yet arrived), it is *not* indexed. On decrypt, it's
  indexed incrementally.
- **Accent swap**: matched-span highlight inherits the active
  accent via `--moss-3` at the current accent.

## Acceptance criteria

- [ ] Top-right search input is focusable via `/` on desktop and
      submits against scope `all groves + letters` by default
      (unless a narrower container is focused).
- [ ] `⌘F` / `Ctrl+F` inside a focused channel or letter scopes the
      search to `this channel` or `this letter`; placeholder and
      chip update; `Esc` clears.
- [ ] Command palette (`⌘K`) forwards non-prefix text to local
      search with scope `all letters`.
- [ ] Mobile pull-down (≥ 44 px with `scrollTop ≤ 0`) reveals the
      search bar on letters, channel, and message lists.
- [ ] Scope chip renders the four scope values, greying
      unreachable ones with the `open a {…} first` tooltip, and
      persists the selection per-device.
- [ ] Prefix operators `from:`, `in:`, `since:`, `before:`,
      `has:image`, `has:file`, `has:link` apply; unknown operators
      are treated as plain text with the tooltip shown.
- [ ] Quoted phrases match adjacent tokens exactly; empty query
      shows the placeholder and renders no results.
- [ ] Results group by grove then channel / letter (wide scope)
      or by letter (letters scope); groups collapse / expand and
      display counts.
- [ ] Result rows show context (channel / letter italic), author,
      timestamp, three-line excerpt with the matched span
      underlined on a `--moss-3` 18 %-alpha background.
- [ ] Clicking a result jumps to the message in its native
      container, scrolls it to 1/3 viewport height, and highlights
      it via `willow-pop-in` for 240 ms plus a 6 s persistent
      underline.
- [ ] First result visible ≤ 150 ms on 10 k-message index,
      ≤ 500 ms on 100 k-message index (reference device).
- [ ] Streaming banner shows `searching… · {n} matches so far`
      while scanning historical content; counter throttled to
      ≤ once per 250 ms.
- [ ] Privacy footer `search runs on this device only. queries
      never leave your device.` is always visible below results.
- [ ] Rebuild-index action in `settings-tweaks.md` privacy runs
      the background rebuild and surfaces the confirmation copy.
- [ ] Horizon changes silently trigger incremental rebuild; the
      toast appears only when the horizon shortens.
- [ ] No query string, match count, or scope is emitted to any
      network path or log — verified by implementation plan.
- [ ] Recents ring buffer caps at 8; individual `forget` and
      `clear all recents` controls work; toggle in tweaks
      disables recents entirely.
- [ ] Accessibility: `role="search"` on the form,
      `role="listbox"` on results, `<mark aria-label="match">`
      around matched spans, `aria-live="polite"` count updates
      ≤ once per 500 ms, focus restoration on back-navigation.
- [ ] Reduced motion collapses streaming fade, highlight flash,
      and chevron rotation.
- [ ] All colours, fonts, radii, shadows, motion durations, and
      copy voice conform to `foundation.md`.

## Open questions

- **Regex / fuzzy**: v1 is substring + operators only. Regex and
  fuzzy (Levenshtein) are deferred; tracked in a follow-up.
- **Index export / backup**: the index is a derived artefact — no
  need to back it up, since messages can always be re-indexed.
  Explicitly out of scope.
- **Shared-device hardening**: on multi-user machines, should the
  index auto-evict on user switch? Deferred to a threat-model
  review; for v1 the index is protected by the OS user account
  plus the disk-encryption layer.
- **Attachment text extraction** (PDFs, docs): not in v1. `has:file`
  finds messages with attachments; OCR / body extraction is a
  future spec.
- **Voice / call transcripts**: once `call-experience.md` lands
  transcripts, opt-in indexing applies. Tracked in the call spec.
- **Discover's delegation**: is the "search my messages instead"
  fallthrough promoted to a persistent toggle on the discover
  surface, or always a CTA after a zero-directory result? Deferred
  to discover's review.
- **Cross-letter mentions**: search within a letter intentionally
  does not surface matches from other letters (privacy) — confirm
  in security review.
