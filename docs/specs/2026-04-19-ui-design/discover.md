# Discover — opt-in grove directory

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:**
- [`foundation.md`](foundation.md) — tokens, typography, accent variants, motion, copy voice
- [`layout-primitives.md`](layout-primitives.md) — desktop three-pane shell and mobile bottom-tab chrome; Discover is a root surface within both
- [`letters-dms.md`](letters-dms.md) — invitation responses arrive as letters; the grove's letter-of-introduction hand-off lives there
- [`governance.md`](governance.md) — steward-side invite-request queue, moderation reports, "hours accepting" policy surface
- [`trust-verification.md`](trust-verification.md) — owner SAS badge rendered on cards

## Purpose

Discover lets a user *find* groves they might want to join without any
platform ever indexing them. The design is dual opt-in:

- **Opt-in for the grove.** A grove is invisible unless its owner
  publishes it. Private groves and ephemeral channels never appear.
- **Opt-in for the discoverer.** Opening Discover is an affirmative
  act. Willow does not passively index what you see, search, or request.

This is not Discord's public directory — it's closer to "who is
currently inviting in public," a voluntary listing owners maintain.

## Scope

In scope: root surface on desktop and mobile; grove card grid / list;
search, category chips; request-invite flow and pending state;
submit-your-grove form for owners; owner SAS badge; empty and no-match
states; per-card safety report filing to the local-first moderation
list in `governance.md`.

Out of scope: global moderation or takedown; trending / popularity
ranking; any per-peer surfacing (Discover lists groves, not people);
non-opt-in ephemeral listings; algorithmic recommendation beyond
declared-category filtering.

## Entry point

Desktop: a toolbar action on the grove rail surfaces a `compass` icon
labelled **discover**. Activating it replaces the main pane content
and deselects any active grove; the rail stays visible.

Mobile: Discover is a root tab in the bottom tab bar (per
`layout-primitives.md`), icon `compass`, label *discover*.

Deep-link: `willow://discover`, with `?q=` and `?tag=` reserved for
search and category state. Re-tapping the root while on Discover
scrolls to top and clears filters.

## Layout

### Desktop

Discover takes the full main pane. Top-down:

1. **Header strip** on `--bg-0` with a soft
   `linear-gradient(180deg, rgba(106,141,94,0.06), transparent)` wash.
   Left: `compass` icon (`md`) + display L Fraunces italic title
   *find a grove*. Right: quiet *submit your grove* link (`--ink-3`,
   underlines to `--ink-1` on hover, no button chrome). Below the
   title, body L copy explainer (see Copy).
2. **Filter strip.** Search input left (≈ two-thirds width, `search`
   icon, placeholder *search groves*, 120 ms debounce). Category chips
   to the right, wrapping on narrow widths, rendered as a listbox (see
   Accessibility).
3. **Grid region** (scrollable, `.scroll`, padding `20px 32px 32px`,
   14 px gap). Columns:
   - `≥ 1200 px` → 3 columns.
   - `900–1199 px` → 2 columns.
   - `< 900 px` → falls through to the mobile single-column layout
     (desktop chrome survives).

Cards do **not** include the reference bundle's "via mira · ori ·
perrin" mutual-peer attribution strip. That pattern implies passive
observation of who's in what, which this spec rejects.

### Mobile

Reuses `MScreen` / `MTopBar` / `MBody` from `layout-primitives.md`:

- **Top bar**: title *discover*, subtitle *groves that opted in*. No
  back chevron (root tab).
- **Search + chips** pin to the top of the scroll region. Chips scroll
  horizontally with `overflow-x: auto`.
- **List**: single-column rows separated by `--line-soft`, no card
  borders — reads as rows rather than tiles.
- **Submit-your-grove** appears as the final row on mobile: `tree`
  glyph, label, chevron, `--bg-2` background.

## Grove card anatomy

Each card is a single semantic `<article>` on `--bg-1` with
`border: 1px solid var(--line)` and `border-radius: 14`. Top to bottom:

1. **Accent strip.** 6 px band of the grove's accent colour (from the
   grove config; see Data dependencies). The only place accent leaks
   into Discover chrome.
2. **Header row** (padding 14 px):
   - **Glyph.** 44 × 44, `border-radius: 12`, background = grove
     accent, text = `--bg-0`, `--font-display` italic 22 px weight 500.
     `grove.glyph` is a single character or small graphic (e.g.
     `F`, `∿`, `△`).
   - **Name.** Fraunces italic, display S, `--ink-0`. Verified owner
     badge (see §Verified owner badge) sits immediately after the name.
     Member count below in body S `--ink-3` (*42 members* / *1 member*).
   - **Category pills.** Top-right: 1–3 chips with `--radius-s`,
     `--bg-2` fill, `--ink-2` text at meta size. Wraps to a second
     line if needed.
3. **Description.** Body S, `--ink-2`, 1.5 line-height, clamped to two
   lines via `-webkit-line-clamp: 2`.
4. **Meta row** (padding-top 12 px). Left: *public since {date}* in
   `--ink-3` hint size, soft time. Right: action area.
5. **Action row** (right-aligned):
   - Member already → *joined · {when}* tag in `--ink-3`, no button.
   - Request pending → *request sent · {time}* pill (`--amber`
     background, `--amber-soft` text) with a ghost **cancel** next to it.
   - Otherwise → primary **request invite** button, 32 px tall, padding
     `6px 12px`, `--radius` corners. Fill is `--moss-2` when the owner
     is SAS-verified to the user, `--amber` otherwise; text is
     `--bg-0` in both cases. The amber fill communicates "proceed with
     intent," not "blocked."
   - To the right of the primary action, a ghost **preview** link
     (`--ink-3`, underlines on hover) renders *only if* the grove
     opted in to preview.

Cards never carry presence dots, activity counters, or "active now"
signals — Discover is a catalogue, not a presence surface.

### Preview

Activating *preview* opens a popover (desktop) or bottom sheet
(mobile): grove name (Fraunces italic, display S), one or two pinned
messages rendered read-only using `message-row.md` styling (author name
+ timestamp, no reactions, reply, or thread entry), and a footer —
*this is a preview. nothing here is sealed to you yet.* Preview
renders only the owner-published preview bundle (see Data
dependencies); nothing live is fetched.

## Request invite flow

Activating **request invite** opens a surface — popover on desktop
(`willow-pop-in`, 180 ms), bottom sheet on mobile (`--radius-l`,
`--shadow-2`, swipe-to-dismiss).

Contents:

1. Header: grove glyph + name.
2. Explainer: *your request goes to this grove's steward. you'll see a
   response in letters.*
3. Optional textarea *why this grove (optional)*. Placeholder: *a
   short note, if you'd like.* 280-char limit; counter appears after 200.
4. Footer: *cancel* (ghost) and *send request* (moss primary).

Submit emits a signed invite-request event (see Data dependencies),
closes the surface, and toasts *request sent*. The card's action area
switches to *request sent · just now* with a *cancel* ghost next to it.

Cancelling confirms first (*withdraw your request?* with *keep* /
*withdraw*), then emits a signed cancellation and restores the request
button.

If the user has been declined before, the button stays enabled but
dims to `--ink-3` with the hint tooltip *declined earlier. you can ask
again.* The UI enforces no cross-grove rate limit; per-grove throttling
is handled by `governance.md`.

## Invitation response — hand-off to letters

Discover is the outbound half only. Incoming responses land in
**letters** (`letters-dms.md`):

- Accept → a letter from the steward containing the grove's **letter
  of introduction** (per `governance.md`), rendered with an inline
  *join now* CTA. Activating it adds the grove to the rail and opens
  its first channel.
- Decline, or no response within the grove's declared *hours
  accepting* window → card state updates on Discover (see Edge cases)
  and a quiet notice lands in letters: *no response yet from {grove}.*

Discover never renders the letter-of-introduction surface itself; it
only reflects state as the letter arrives.

## Verified owner badge

Owner SAS status (from `trust-verification.md`), shown after the
grove name:

- **Verified** → `--moss-2` filled `check` icon at `sm` size. Tooltip:
  *owner verified · moss*.
- **Unverified** (owner known but SAS not compared) → no badge; the
  amber **request invite** button carries the signal. Hover / long-
  press on the name shows the hint *owner not verified yet — you can
  verify after joining.*
- **Pending verification** → dashed `--amber` `fingerprint` icon.
  Rare on Discover.
- **Previously failed SAS** → `--err` outlined `x` icon; card demotes
  to the bottom with 0.6 opacity. The request action stays enabled —
  demotion communicates caution, not prohibition.

The badge never gates the action; trust is visible, not blocking.

## Search + categories

**Search** filters on grove name and description, case-insensitive
substring match. Ordering: exact-name → prefix-name → description
matches. Matches are highlighted with `<mark>` using `--moss-1`
background, no decoration.

**Categories** are self-declared tags. Chip bar: *all* (default,
selected), *reading*, *music*, *local*, *research*. Owners may
free-type more tags on submit; additional observed tags surface via a
trailing *more…* action so the header stays calm.

Single-select only in v1: tapping a chip replaces the filter; tapping
the active chip returns to *all*. Chips render as:

- Inactive: `--bg-2` / `--ink-2`, `--radius-s`.
- Active: `--moss-1` / `--moss-4`.
- Hover: background lifts to `--bg-3`.

## Submit your grove

Owners see *submit your grove* as a quiet header link (desktop) or the
last list row (mobile). Non-owners see the link disabled with tooltip
*you need to own a grove to list one here.*

Activating opens a form surface (popover / sheet):

1. Header: *submit your grove*.
2. Grove selector (dropdown of groves the user owns, required).
3. Description textarea, 240-char limit, placeholder *what kind of
   place is this?*.
4. Category chips, 1–3 selectable, tap to toggle (free-type input
   below accepts additional tags).
5. Preview toggle: *allow preview · shows 1–2 pinned messages to
   visitors* (default off).
6. **Hours accepting** (collapsed by default). Day-of-week row +
   from/to time pickers. Local-time; payload carries the owner's
   timezone. Hint: *requests outside these hours auto-decline with a
   note. leave empty to accept anytime.*
7. Above the footer, an explainer line:
   *this publishes your grove to everyone who opens Discover — but
   peers still need an invite to join.*
8. Footer: *cancel* (ghost), *publish* (moss primary).

Submit emits a signed grove-publish event, closes the form, and toasts
*grove published.* Re-opening the form for an already-published grove
loads existing values and relabels the primary action *update.* A
ghost **unlist** link appears in the footer in this state; activating
it confirms *unlist your grove from Discover?* and emits an unpublish
event.

## Safety / reporting

Each card has a quiet **report** affordance: `more-horizontal` icon at
the top-right of the header row, `sm` size, 44 × 44 hit box. The menu
shows *report · this grove posted harmful content*.

Selecting opens a report form (popover / sheet):

- Header: *report this grove*.
- Reason textarea (required, 500 chars).
- Footer: *cancel*, *submit report* (`--err` primary).
- Explainer: *willow has no global moderation. this goes to your own
  moderation list — it stays with you, helps you remember, and is
  visible only to you in governance → reports.*

Submit emits a signed but local-only report record. It lands in the
reporter's governance → reports tab; no other peer ever sees it. A
reported card gains a subtle `--ink-3` outline and a `shield` icon in
its header row as a reminder. No global list, no report counter, no
auto-hide.

## Copy (exact)

All copy strings owned by this spec. Lowercase except proper grove
names.

- Root: *discover*
- Display title: *find a grove*
- Explainer (desktop header body):
  *willow has no directory — a grove appears here only if its owner
  publishes it.*
- Primary action: *request invite*
- Pending action: *request sent · {time}*
- Joined tag: *joined · {when}*
- Submit entry: *submit your grove*
- Submit publish explainer:
  *this publishes your grove to everyone who opens Discover — but
  peers still need an invite to join.*
- Request flow explainer:
  *your request goes to this grove's steward. you'll see a response in
  letters.*
- Hours-accepting section label: *hours accepting*
- Hours-accepting hint:
  *requests outside these hours auto-decline with a note. leave empty
  to accept anytime.*
- Preview footer: *this is a preview. nothing here is sealed to you
  yet.*
- Empty (no groves): *quiet out here — nothing to discover today*
- Empty (search / filter no match): *no groves match '{q}'*
- Report menu item: *report · this grove posted harmful content*
- Report explainer:
  *willow has no global moderation. this goes to your own moderation
  list — it stays with you, helps you remember, and is visible only to
  you in governance → reports.*
- Unverified owner hover hint:
  *owner not verified yet — you can verify after joining.*
- Declined earlier hint: *declined earlier. you can ask again.*

## Data dependencies

Flagged **new** (requires a new `EventKind`) or **existing** (already
in `willow-state`).

- **Grove publish event** — *new*. Signed by owner; fields:
  `grove_id`, `description`, `categories[]`, `glyph`, `accent`,
  `preview_opt_in`, `preview_pinned_refs[]` (0–2), `hours_accepting[]`
  (optional), `timezone`. Re-emit to update; newest wins per
  `(grove_id, owner)`.
- **Grove unpublish event** — *new*. Signed by owner; field
  `grove_id`.
- **Invite request event** — *new*. Signed by requester; fields
  `grove_id`, `note` (optional, 280 chars), `requested_at`. Delivered
  to the steward set per `governance.md`.
- **Invite-request cancel event** — *new*. Signed by requester;
  field `request_id`.
- **Invite response** — *existing*. Reuses the letter-of-introduction
  payload (`governance.md`, `letters-dms.md`).
- **Owner SAS verification status** — *existing*. Sourced from
  `trust-verification.md`; read-only here.
- **Moderation report (local)** — *existing*. Spec writes into the
  `governance.md` reports list; does not render it.

Distribution: publish events ride a dedicated *Discover topic*
(blake3-hashed via `network::topics`). Users subscribe only while
Discover is mounted, preserving opt-in-for-the-discoverer.

## Edge cases

- **Owner offline long-term.** If the owner's most recent signed
  heartbeat is older than 48 h, the card adds a small `--ink-3` hint
  under the meta row: *owner last seen {time} · request may take a
  while.* Request stays enabled.
- **Duplicate grove names.** Each duplicate-name card appends a
  short-form (3-word) owner fingerprint in mono S under the name.
  Hidden when the name is unique.
- **Grove withdraws from Discover.** Cards disappear on the next
  sync. A viewer with a pending request sees their card transition to
  a terminal *grove is no longer listed.* text (`--ink-3`, no CTA).
  The in-flight request event is not auto-cancelled; a steward can
  still accept and the letter still arrives.
- **Owner outside hours-accepting window.** Card shows an `hourglass`
  chip *accepting later · {next window}.* The primary action becomes
  a ghost *queue my request* which batch-sends when the window opens.
- **User already joined via another path** (letter of introduction
  from a friend). Card renders *joined · {when}* instead of the
  request button; no duplicate-join path exists.
- **Large grid / slow sync.** Off-viewport cards render as skeleton
  tiles using the foundation `shimmer` keyframe on `--bg-2` / `--bg-3`.
  Cap concurrent skeletons at 12.
- **First open with empty data.** Empty state copy plus a ghost link
  *plant your own grove* linking to grove-creation.
- **Zero results with filter active.** Same no-match copy plus a
  ghost *clear filter* link that resets the chip to *all*.

## Accessibility

- Each card is a single `<article>` with `aria-labelledby` pointing at
  the grove-name heading (`<h3>`) inside. Member count, description,
  and action area read in order.
- Category chips are a `role="listbox"` with `role="option"` and
  `aria-selected`. Keyboard: `←` / `→` move, `Enter` / `Space`
  activate, `Home` / `End` jump.
- Search input lives inside `role="search"` with
  `aria-label="search groves"`.
- Primary button `aria-label`: *request invite to {grove name}*; in
  pending state, *request sent to {grove name} · cancel*. Report
  button `aria-label`: *report {grove name}*.
- Verified badge: `aria-label="owner verified"`, announced before the
  member count.
- Focus ring is `--focus-ring` on every interactive element. Touch
  targets are ≥ 44 × 44 CSS px.
- Colour is never the only signifier: verified = moss + filled check;
  unverified = amber button + hover hint; pending = amber pill +
  *sent · time* text.
- Reduced motion: shimmer collapses to a static `--bg-2` block;
  popover / sheet entries use opacity only.
- Report has a keyboard path (context-menu key, or long-focus-Enter)
  equivalent to mobile long-press.

### Labels (screen-reader)

*discover tab*, *find a grove*, *search groves*, *category {name},
selected / not selected*, *submit your grove*, *request invite to
{grove name}*, *request sent to {grove name}, cancel*, *joined
{grove name} {when}*, *preview {grove name}*, *report {grove name}*,
*owner verified*, *owner not verified*.

## Acceptance criteria

- [ ] Discover exists on desktop (toolbar action) and mobile (root
      tab); `willow://discover` resolves.
- [ ] Grid is 3 col ≥ 1200 px, 2 col 900–1199 px, single column below.
- [ ] Card renders accent strip, glyph, name, member count, 2-line-
      clamped description, category chips, meta row, and action area.
- [ ] *request invite* emits a signed event; card transitions to
      *request sent · {time}* with *cancel*.
- [ ] *cancel* confirms, emits cancellation, restores the button.
- [ ] Verified-owner badge renders only on SAS-verified owners.
- [ ] Primary button is amber for unverified owner, moss for verified.
- [ ] Preview opens a read-only pinned-message surface only when the
      grove opted in; no live content is fetched.
- [ ] Search filters by name + description; chips filter by tag;
      combined filter + search works.
- [ ] Empty / no-match states render the exact Copy strings above.
- [ ] *submit your grove* is hidden/disabled for non-owners and
      posts/updates grove-publish events for owners. *unlist* emits
      an unpublish event.
- [ ] Report flow emits a local-only report and flags the card with
      a `shield` icon for the reporter only.
- [ ] Cards never render presence, activity, or per-peer attribution.
- [ ] `prefers-reduced-motion` suppresses shimmer and entry transforms.
- [ ] Every interactive element is keyboard-focusable and labelled.
- [ ] CSS uses only foundation tokens; no literal colours.

## Open questions

- **Per-grove accent on the verified badge.** Verified stays moss so
  the signal is consistent across accents. Revisit if designers feel
  it conflicts with the grove's own accent on the glyph.
- **Cross-grove rate-limiting.** UI throttles to one pending request
  per grove; no global cap. Stewards handle per-grove throttling in
  `governance.md`.
- **Decline reason surface.** Decline notes live in the letter from
  the steward; Discover only reflects state. Reconsider if users
  struggle to correlate the two surfaces.
- **Mutual-grove hinting.** Deferred: "three peers you know are in
  this grove" requires a membership-observation mechanism that
  conflicts with opt-in-for-discoverer. Re-open if a privacy-
  preserving mechanism (e.g. PSI) becomes practical.
- **Trending / activity signals.** Deferred indefinitely. Discover
  is a catalogue, not a feed.
- **Category taxonomy.** The curated five (`reading`, `music`,
  `local`, `research`, `all`) is a judgement call; revisit after the
  first real publish dataset.
- **Ephemeral grove listings.** Explicitly excluded. If ongoing
  ephemeral containers need discovery, they get their own spec
  (`ephemeral-discovery.md`), not an extension here.
