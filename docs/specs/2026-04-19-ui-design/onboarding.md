# Onboarding — first-run, identity, add a friend, first grove

**Parent:** [README.md](README.md)
**Dependencies:** [`foundation.md`](foundation.md), [`trust-verification.md`](trust-verification.md), [`profile-card.md`](profile-card.md), [`letters-dms.md`](letters-dms.md), [`discover.md`](discover.md)
**Status:** draft

## Purpose

Onboarding is the first two minutes of willow. It has three jobs: create or
recover a local identity (Ed25519 keypair + display profile); teach the
security model without being technical, so the user leaves knowing what a
fingerprint is and why nothing is stored on a server; and deposit the user
in a grove — created, joined, or discovered — with at least one verified
peer if the flow could produce one.

The experience is literary and slow on purpose. Fraunces italic carries the
display copy, IBM Plex Sans the body, JetBrains Mono the fingerprint. The
`leafFall` animation is permitted here — foundation lists onboarding as one
of only two surfaces allowed to use it.

## Scope

- All first-run flows on `crates/web`, desktop and mobile web.
- Recovery flow for users who already have willow on another device.
- Progressive disclosure — everything past step 2 is skippable, with subtle
  follow-up prompts appearing in the main UI later.
- Copy, layout, motion, and accessibility requirements.
- Data-flow into identity creation, SAS derivation (reused from
  `trust-verification.md`), invite consumption (from `letters-dms.md`), and
  the first `CreateServer` event.

**Out of scope.** Actual keygen (`willow-identity`); SAS word derivation
(`willow-crypto`, surfaced via `trust-verification.md`); the invite-link URL
scheme (`letters-dms.md`); the discover directory (`discover.md`). This spec
stitches those capabilities into a single first-run flow.

## Flow overview

A six-step linear flow with a recovery branch off step 1.

```
  step 1 welcome
        │
        ├── "i already have willow" ──▶ recovery ──┐
        ▼                                          │
  step 2 identity (required — committed here)      │
        ▼                                          │
  step 3 add a friend (skippable)                  │
        ▼                                          │
  step 4 pair with peer (skippable)                │
        ├── "i have an invite"  → join flow ───┐   │
        ├── "show me mine"      → share flow ──┤   │
        └── skip                → step 5       │   │
                                                ▼   ▼
  step 5 first grove (skippable only on recovery) ◀─┘
        ▼
  step 6 tour (skippable)
        ▼
  enter chat
```

Only steps 1 and 2 are required. Steps 3–6 each render a "skip"; skipped
steps register subtle follow-up prompts in the chat UI later (see
**Skippability**).

### Progress presentation

- **Desktop.** Left ambient panel, 360 px, `--bg-1` with `--line-soft` right
  border, holds wordmark + tagline + a vertical step list. Each step is a
  numbered 26 px dot in `--font-mono`. Current step: `--bg-3` fill,
  `--moss-2` border, `--ink-0` label. Done: `--moss-2` fill with a check
  glyph, `--ink-1` label. Upcoming: `--bg-2`, `--line`, `--ink-3`.
- **Mobile.** No side panel. Progress is a `meta`-size tag in the top-right:
  `step 3 · 6`, `--font-mono`, not tappable.

Step labels: `welcome` · `identity` · `add a friend` · `peer` · `grove` ·
`tour`. Recovery uses: `welcome` · `recover` · `grove` · `tour`.

The step-3 public label `add a friend` is quoted verbatim from the
reference bundle (`onboarding.jsx` Step 3 label + SAS dialog title);
the step's internal name remains *fingerprint intro*.

### Layout

- **Desktop.** Full viewport; 360 px ambient panel on the left, right pane
  `--bg-0` with 40 × 56 px pad. Prose width 720 px, grids 820 px. `--shadow-2`
  only for internal popovers (accent picker, mobile crest picker).
- **Mobile.** Single column. 14 × 16 px top bar (wordmark + progress).
  Body pad 20 × 18 px. Primary CTA pinned 18 px from bottom, `--radius-l`,
  14 px vertical pad. Secondaries stack above it with 8 px gap.

## Step 1 · Welcome

**Required.** Back is disabled.

Full-viewport hero; on welcome only, the ambient panel is collapsed so the
hero fills the screen. Centred vertically, max-width 560 px on desktop.

- **Wordmark.** `willow` in `display XL` (Fraunces 54 px italic 400),
  `--ink-0`.
- **Tagline.** `a grove of your own — small group chat that lives on your
  devices, not on a server.` In `display L`, `--ink-1`, max-width 480 px.
- **Primary CTA.** `begin` — button on `--moss-2`, `#14130f` foreground,
  `--radius`, 12 × 20 px pad, `body L`. Focus: `--focus-ring`.
- **Secondary.** `i already have willow` — text-only, `--ink-2`, `body S`,
  20 px top margin. Underline on hover / focus.
- **Privacy foot.** `no account · no signup · nothing leaves this device
  without your action` — `hint` size, `--ink-3`, pinned near the bottom.

### Motion

`leafFall` plays once on entry: three leaves across the viewport, staggered
240 ms apart, each 3200 ms, opacity 0 → 0.35 → 0, `--willow` at 0.5 alpha,
behind content. Wordmark fades over 320 ms ease-out willow; tagline at
120 ms delay; CTAs at 240 ms delay. Reduced motion replaces `leafFall` with
a single static `leaf` icon at 20% opacity in the upper-left third and
collapses fades to instant.

### Interactions

- `begin` → step 2.
- `i already have willow` → recovery.
- Esc / browser back: no-op (no upstream).

## Step 2 · Identity

**Required.** Identity is committed at the end of this step. Resume returns
here if the user bails after committing and before reaching step 5.

Ambient panel re-appears on desktop. Form max-width 560 px, single column.
On mobile: 18 px horizontal pad, 14 px vertical gap between fields.

- **Heading.** `who are you, here?` — `display L` italic.
- **Subhead.** `this is how you look to peers before you share more. you can
  change any of this later.` — `body L`, `--ink-2`.

### Fields

1. **display name (required).** Input 100% width, `--bg-2`, `--line`,
   `--radius`, 12 × 14 px pad, `body L`. Placeholder
   `what should peers call you?`. Max 40 chars (soft warning at 32).
   Validation: non-empty after trim.
2. **pronouns · optional.** Free-text, 64 char max. Placeholder
   `she/her · they/them · any — whatever fits`.
3. **crest.** Three option cards (`fronds`, `rings`, `leaf`) drawn as
   1.5 px-stroke SVGs. 84 × 84 px each, 12 px gap, `--bg-2` / `--line` /
   `--radius`. Hover `--bg-3`; selected: `--moss-2` border + `--moss-0` tint
   at 60% alpha. Default `fronds`.
4. **accent.** Six swatches (circular, 36 px, filled with `--moss-2` of each
   accent variant — `moss`, `willow`, `amber`, `dusk`, `cedar`, `lichen`)
   plus one `surprise me` swatch. 10 px gap. Selected swatch has a 2 px
   `--moss-2` ring and scales 1.08 at `--motion-fast`. `surprise me` picks
   deterministically from `PeerId` bytes (first byte mod 7 → variant index).
   Selection writes `--moss-*` and `--willow` variables live so the rest of
   onboarding reflects the choice.

### Live preview

Right of the form on desktop (below on mobile), a 240 px self-variant
profile card from `profile-card.md`, populated live. Empty fields render as
`--ink-4` placeholders.

### Privacy callout

`--bg-1` card, `--line`, `--radius`, 14 px pad, `body S`, `--ink-2`:

> no account · no signup · nothing leaves this device without your action.
> your keys are generated on this device, right now. they never touch a
> server. if you reset your browser, your identity resets too — unless you
> export it.

### Primary CTA

`continue` — disabled until display name is non-empty. On click: generate
Ed25519 keypair via `willow-identity`; build
`Profile { display_name, pronouns?, crest: (pattern, accent) }`; persist;
advance to step 3. Secondary: `back` returns to welcome. No skip.

## Step 3 · Add a friend

**Skippable.** Pure teaching moment; no verification happens here.
The public step label is `add a friend` (from the reference bundle);
the content is the fingerprint-intro teaching moment described below.

Right pane, max-width 720 px desktop / full-width mobile (18 px horizontal
pad).

- **Heading.** `your six words` — `display L` italic.
- **Intro.** `body L`, `--ink-2`, max-width 620 px, 1.6 line-height:

  > these six words are your willow fingerprint. when you meet a peer,
  > compare your words on two screens — if they match, no one can
  > impersonate you in that conversation, ever. repetition makes
  > verification stronger.

- **Fingerprint card.** `--bg-1` / `--line` / `--radius-l`, 22 px pad.
  Header row: self avatar 36 px (crest + accent from step 2), `body` text
  `you · this device`, `hint` text `just now · keys created`. Below:
  `FingerprintLabel which="you"` (exact label copy from
  `trust-verification.md`: `your fingerprint — read this aloud`) +
  `FingerprintGrid size="md"` on desktop, `size="sm"` on mobile — 3-column
  `--font-mono`, numbered 1–6.
- **Reassurance.** Shield-icon line, `hint` size, `--ink-3`, below the grid:

  > these six words are derived from your keys. repeat them out loud to a
  > friend across the table or across the world — if what they hear
  > matches what they see, willow is a safer place for both of you.

### Motion

Grid cells fade in staggered 60 ms per cell, each 180 ms ease-out willow.
Reduced motion: instant opacity.

### Interactions

- **Primary.** `i'll remember` — button on `--moss-2`. Advances to step 4.
- **Secondary.** `skip for now` — text button, `--ink-2`. Registers a
  deferred prompt: a one-time amber dot on the settings gear labelled
  `your fingerprint · see it now`.

## Step 4 · Pair with a peer

**Skippable.** Creates the user's first verified peer if completed.

Two branches: `i have an invite` (inbound letter of introduction) and
`show me mine` (outbound letter). Either branch culminates in a SAS
ceremony (per `trust-verification.md`) producing a verified peer. A
grove-scoped inbound invite also seeds step 5.

### Layout

- **Desktop.** Two side-by-side cards, 340 px wide, 22 px pad, `--bg-1` /
  `--line` / `--radius-l`.
- **Mobile.** Stacked cards, 14 px gap.

### Intro

- Heading: `find your first peer` — `display L` italic.
- Subhead, `body L`, `--ink-2`: `willow starts when two devices meet. this
  step connects you to one person so you can see each other's fingerprints.`

### Branch A — "i have an invite"

Card title `body L`: `i have an invite`. Body `body S`, `--ink-2`:
`paste the letter of introduction you were sent — or scan its qr code.`

Controls:

- **Paste field.** 3-line textarea, `--bg-2`, `--line`, `--radius`,
  `--font-mono` `mono M`, 10 px pad. Placeholder: `paste willow://… here`.
- **Scan QR.** `scan qr` button — opens camera on mobile (requires
  permission; see Edge cases). Desktop hides unless a camera is available.
- **Primary CTA.** `open letter` — disabled until the field parses as a
  valid letter of introduction.

On success the parsed invite payload is handed to the join flow defined in
`letters-dms.md`. Onboarding hosts the invite UI until the peer responds,
then renders the SAS ceremony from `trust-verification.md` (size `md`
desktop, `sm` mobile). On match: a `verified peer` toast fades in
(`willow-pop-in`, 4 s), and onboarding advances to step 5 with the invite's
grove (if any) pre-selected. On "they don't match": non-fatal amber card —
`fingerprints don't match — this is worth pausing over. ask your peer to
re-read them, or try again. if it still doesn't match, don't verify.` —
retry or `skip this peer`.

### Branch B — "show me mine"

Card title: `show me mine`. Body: `generate a letter so someone else can
find this device.`

- **Primary CTA.** `write a letter` — creates an invite via the path in
  `letters-dms.md` and reveals the share surface.
- **Share surface.** `--bg-2` / `--line` / `--radius` / 14 px pad. QR
  200 × 200 px desktop, 240 × 240 px mobile, rendered on `--ink-0` with
  `--bg-0` modules. Below: the full `willow://…` URL in `--font-mono`
  `mono M`, `--ink-1`, tap-to-copy. Button `copy letter` shows a
  `willow-pop-in` toast `copied · share it only with the person you mean`.
  Caution line `hint` / `--ink-3`:
  `share this only in person or on a channel you trust.`
- Below share surface: `waiting for peer` indicator using `willowPulse` on
  a silhouette avatar. When the peer connects, the SAS ceremony replaces
  the share surface — same flow as branch A.

### Skip

Both cards share a footer text button `skip — find a peer later` in
`--ink-2`. Skipping registers a sidebar nudge: `you have no verified peers
yet — pair`. The next screen adds a single `body S` note:
`verification unlocks whisper and call handoff.`

## Step 5 · First grove

**Skippable only on recovery** (when the identity already has groves).
Otherwise the user must create, join, or browse discover — the minimum
state for the chat UI to render anything useful.

Three cards. The most likely path floats to the top based on state (step-4
grove invite → B first; no invite → A first; recovery → C first).

### Card A — create

Title `display M`: `plant a new grove`. Body `body S`: `a grove is a small
space shared by a handful of people. you'll own this one — you decide who
joins.`

Fields: **grove name** (`body L`, 80 char, placeholder rotates among
`backyard`, `family grove`, `sunday crew`); **short description · optional**
(200 char, placeholder `what is this grove for?`); **default accent** —
same six swatches from step 2, inheriting the user's identity accent,
writing the grove's `accent` token (pending the foundation open question
on per-grove accent; if app-level-only ships, this field is dropped).

Primary CTA `plant grove` emits a single `CreateServer` event with the user
as owner, then advances to step 6 with the grove id in hand.

### Card B — join an invited grove

Visible only if step 4 yielded a grove invite. Title `step into <grove
name>`. Body shows a compact preview: name, tagline (if any), member count,
inviter avatar + verified badge. CTA `join <grove name>` delegates to the
existing `JoinPage` flow (`letters-dms.md` / `discover.md`), then advances
to step 6.

### Card C — browse discover

Title: `look around`. Body: `public groves that chose to be seen. joining
any of these is optional.` CTA `browse discover` opens the discover surface
inside the onboarding shell so the progress indicator persists. The
sub-screen's own join flow returns control here on success; then onboarding
advances to step 6. A `done browsing` text-button returns to the three-card
view.

### Skip

`skip — i'll look around later` — only shown when the identity already has
at least one grove (recovery). Suppressed for fresh identities.

## Step 6 · Final tour

**Skippable.** Not a product demo; a literary postcard.

Four cards. Each has an icon (24 px, `--moss-2`, from the foundation icon
set) + a `display M` italic title + a single body sentence.

1. **`tree` · `groves and channels`.** `a grove is the house. channels are
   the rooms. text, voice, or ephemeral — rooms that forget themselves.`
2. **`ear` · `whispers live inside calls`.** `whispers are a violet
   side-channel you can open with a verified peer during a call, so two of
   you can say one thing the rest don't hear.`
3. **`hourglass` · `ephemeral channels burn their keys`.** `set a timer.
   when it runs out, the channel's keys are shredded and the conversation
   is unreadable — even to you.`
4. **`activity` · `nothing is stored on a server`.** `willow queues your
   messages until your peers are reachable. the queue is patient. your
   devices do the work.`

### Layout

- **Desktop.** 4 stacked cards, 560 px wide, 22 px pad, `--bg-1` / `--line`
  / `--radius`. Entry stagger: 180 ms between cards, each 220 ms ease-out
  willow with opacity + 8 px Y.
- **Mobile.** One card at a time, full-width, `--radius-l`. Swipe-left or
  tap-right to advance. Four-dot indicator at the bottom; a subtle
  `willowPulse` sits behind the active dot.

### Motion

`leafFall` replays once across the viewport on the last card (closing
ceremony). Reduced motion: static leaf, no drop.

### Primary CTA

`enter willow` — below all four cards (desktop) or at the bottom of the
last card (mobile). Marks onboarding complete; routes to the grove created
/ joined / selected, or to an empty state if none (unlikely recovery edge).
Secondary: `skip tour` — top-right (desktop) or below the progress tag
(mobile), routes directly to the chat UI.

## Recovery flow

Entry: step 1's `i already have willow` link. Progress becomes `welcome` ·
`recover` · `grove` · `tour`.

- **Heading.** `bring your identity with you` — `display L` italic.
- **Subhead.** `willow identities live on your devices. import the one you
  already made on another device.` — `body L`.

Two tabs (pills, 36 px tall, `--bg-2` inactive / `--bg-3` active):

1. **paste keyfile.** Multi-line `--font-mono` textarea, `mono M`.
   Placeholder `paste the contents of your willow identity file`. Parse on
   every change; `--err` border if invalid.
2. **enter mnemonic.** Plain-text textarea, `--font-mono`. Placeholder
   `twelve or twenty-four words, separated by spaces`.

Below the tabs, `hint` / `--ink-3`: `your identity never leaves this
device. pasting it here reconstructs your keys locally.`

Primary CTA: `restore identity`. Validates material, reconstructs Ed25519
keypair via `willow-identity`, rehydrates cached profile (name, pronouns,
crest, accent) — the profile-card preview populates live. Then probes for
peers that recognise this identity; known groves surface in a `what we
found` banner.

- If identity has ≥ 1 known grove → jump to step 6 (tour); the tour's
  `enter willow` routes into the first grove. Tour banner reads:
  `welcome back. your fingerprint is the same — your peers will recognise
  you.`
- If no known groves (or the probe timed out) → jump to step 5.
- Fingerprint and peer steps are always skipped on recovery.

If the source device exported with a passphrase, an additional `passphrase`
input appears inline; retry is in-place. Wrong passphrase:
`that passphrase didn't decrypt — try again.` Invalid material:
`this doesn't look like a valid willow identity. check for missing words
or extra whitespace.`

If the profile is absent (keyfile without metadata), an extra pre-step
`rename yourself` appears before step 5 so the user can fill in display
name + crest.

## Privacy

Onboarding is client-side. No outbound network requests occur until the
user takes an action that implies networking (step-4 invite exchange;
step-5 grove join or discover browse).

Explicit privacy surfaces:

- Welcome foot: `no account · no signup · nothing leaves this device
  without your action`.
- Identity step callout repeats the same phrase as headline of the privacy
  card.
- Recovery hint: `your identity never leaves this device. pasting it here
  reconstructs your keys locally.`
- Pair-with-peer (branch B) caution: `share this only in person or on a
  channel you trust.`

No telemetry. No analytics opt-in prompt. No third-party scripts beyond
foundation's configured font source. If offline, system fallback fonts
render and the flow still completes.

## Skippability

| Step | Required? | Skip affordance | Later nudge if skipped |
|------|-----------|-----------------|------------------------|
| 1 welcome | Yes | none | n/a |
| 2 identity | Yes | none | n/a |
| 3 add a friend | No | `skip for now` | one-time amber dot on settings gear: `your fingerprint · see it now` |
| 4 peer | No | `skip — find a peer later` | self-profile sidebar row: `you have no verified peers yet — pair` |
| 5 grove | No (recovery only) | `skip — i'll look around later` | chat empty state: `no groves yet — plant one or look around` |
| 6 tour | No | `skip tour` | Help menu always exposes `show the tour` |

Deferred prompts also live in the Tweaks panel's `first-run prompts`
section (`settings-tweaks.md`). Dismissing a prompt (X, or "don't show
again") suppresses it permanently; Help continues to expose the tour.

**Back button.** Absent on steps 1–2. Visible on 3–6 in the top-left
(desktop) or above the heading (mobile). Returns with state preserved;
committed identity is never rolled back.

## Mobile adaptations

- Full-screen per step; no side panel. Progress is `step N · 6` in
  `--font-mono` in the top-right.
- Back button is hidden on steps 1 and 2 (no chevron rendered) and visible
  on steps 3–6.
- Primary CTA pins 18 px from the bottom, `--radius-l`, 14 px vertical pad.
  Secondaries stack above with 8 px gap.
- Crest accent swatches remain 36 px visually inside 44 × 44 px hit boxes
  (foundation touch-target baseline).
- SAS grid uses `size="sm"` (per `trust-verification.md`); word numbers
  shrink to 9 px.
- SAS match control is a 900 ms hold-to-confirm button with left-to-right
  progress fill in `--moss-2` at 70% alpha. Keyboard `Enter` bypasses the
  hold (no long-press possible via keyboard).
- Tour cards swipe horizontally with snap; large portrait devices also
  accept edge-taps.
- `leafFall` on mobile reduces to two leaves (less crowding).
- Safe areas: `env(safe-area-inset-bottom)` on pinned CTAs;
  `safe-area-inset-top` on the top bar.

## Copy (exact — consolidated)

All strings this spec owns, in first appearance order. Progress labels:
`welcome` · `identity` · `add a friend` · `peer` · `grove` · `tour`.
Recovery labels: `welcome` · `recover` · `grove` · `tour`.

`willow` · `a grove of your own — small group chat that lives on your devices, not on a server.` · `begin` · `i already have willow` · `no account · no signup · nothing leaves this device without your action` · `who are you, here?` · `this is how you look to peers before you share more. you can change any of this later.` · `display name` · `pronouns · optional` · `crest` · `accent` · `surprise me` · `continue` · `your six words` · `these six words are your willow fingerprint. when you meet a peer, compare your words on two screens — if they match, no one can impersonate you in that conversation, ever. repetition makes verification stronger.` · `i'll remember` · `skip for now` · `find your first peer` · `willow starts when two devices meet. this step connects you to one person so you can see each other's fingerprints.` · `i have an invite` · `show me mine` · `paste willow://… here` · `open letter` · `scan qr` · `write a letter` · `copy letter` · `copied · share it only with the person you mean` · `share this only in person or on a channel you trust.` · `verified peer` · `fingerprints don't match — this is worth pausing over. ask your peer to re-read them, or try again. if it still doesn't match, don't verify.` · `skip — find a peer later` · `verification unlocks whisper and call handoff.` · `plant a new grove` · `a grove is a small space shared by a handful of people. you'll own this one — you decide who joins.` · `plant grove` · `step into <grove name>` · `join <grove name>` · `look around` · `public groves that chose to be seen. joining any of these is optional.` · `browse discover` · `skip — i'll look around later` · `groves and channels` · `a grove is the house. channels are the rooms. text, voice, or ephemeral — rooms that forget themselves.` · `whispers live inside calls` · `whispers are a violet side-channel you can open with a verified peer during a call, so two of you can say one thing the rest don't hear.` · `ephemeral channels burn their keys` · `set a timer. when it runs out, the channel's keys are shredded and the conversation is unreadable — even to you.` · `nothing is stored on a server` · `willow queues your messages until your peers are reachable. the queue is patient. your devices do the work.` · `enter willow` · `skip tour` · `bring your identity with you` · `willow identities live on your devices. import the one you already made on another device.` · `paste keyfile` · `enter mnemonic` · `paste the contents of your willow identity file` · `twelve or twenty-four words, separated by spaces` · `your identity never leaves this device. pasting it here reconstructs your keys locally.` · `restore identity` · `welcome back. your fingerprint is the same — your peers will recognise you.` · `this doesn't look like a valid willow identity. check for missing words or extra whitespace.` · `that passphrase didn't decrypt — try again.` · `this letter of introduction has expired — ask the peer for a new one.`

## Data dependencies

### Identity creation (step 2)

- **Keys.** `willow-identity` Ed25519 keygen, called at the *end* of step
  2 — so a user who abandons between steps 1 and 2 leaves no artefacts.
- **Profile.** `{ display_name, pronouns?, crest: { pattern, accent } }`
  where pattern ∈ `fronds` | `rings` | `leaf` and accent is an
  `AccentVariant` from `foundation.md`. Persisted via the existing
  `AppState.server.display_name` surface plus new fields introduced in
  `profile-card.md` (pronouns, crest).
- **Persistence.** LocalStorage-backed, same path the current client uses.
  Identity is fully committed before step 3 renders.

### Identity recovery

- Calls `willow-identity`'s keyfile / mnemonic import APIs — no new backend
  work specific to onboarding.
- Profile rehydrates from storage alongside the identity material. If
  absent, the flow inserts a `rename yourself` sub-step before step 5.

### SAS derivation reuse (steps 3 and 4)

- Uses the shared SAS components from `trust-verification.md`; onboarding
  does *not* derive words itself. Consumes `FingerprintGrid` and
  `FingerprintLabel`.
- Step 3 renders the self-only form (one card, label
  `your fingerprint — read this aloud`).
- Step 4 renders the pair form (side-by-side on desktop, stacked on
  mobile; long-press confirm on mobile; explicit `they match` / `they
  don't match` on desktop).

### Invite flow (step 4)

- Owns zero invite logic. Delegates to `letters-dms.md` for parsing
  inbound letters and producing outbound ones.
- On a grove-scoped inbound invite, the grove id forwards to step 5 (card
  B materialises with the grove preview from the invite manifest).
- Expiry surfaces via the error path declared in `letters-dms.md`;
  onboarding renders `this letter of introduction has expired — ask the
  peer for a new one.`

### Grove creation event (step 5, card A)

- Emits exactly one `CreateServer` event via the `Client` API with the
  user as owner (per the authority model in
  `docs/specs/2026-04-12-state-authority-and-mutations.md`). The default
  accent field populates the grove's `accent` token if per-grove accent
  ships.

### Grove join (step 5, card B)

- Delegates to the existing `JoinPage` flow in
  `crates/web/src/components/join_page.rs`, rendered inside the onboarding
  shell so the progress indicator persists.

### Tour (step 6)

- Pure presentation; no backend calls.

## Edge cases

- **Mic permission declined.** Onboarding never requests mic; no-op. Any
  voice teaching moves to `call-experience.md`.
- **Camera permission declined (step 4A).** `scan qr` disables with a
  `hint`: `camera access was declined — paste the letter instead`. Paste
  remains available.
- **Browser closed mid-flow.** Before step-2 commit: nothing persisted;
  reopen lands on step 1. After step-2 commit and before step-5
  completion: reopen lands on the last step touched. Step-level drafts
  (paste-field contents, form-field edits) are ephemeral; not persisted.
  After step 5: onboarding is complete; tour is available from Help.
- **SAS compare fails.** Non-fatal — mismatch copy, retry, or skip-this-
  peer. No event recorded; trust store untouched.
- **Invite expired.** `this letter of introduction has expired — ask the
  peer for a new one.` in a `--warn`-bordered card above the paste field.
  `open letter` stays disabled; paste remains available for a fresh
  invite.
- **Recovery material corrupted.** Invalid-error copy; retry in place;
  no rate-limit (local-only).
- **Recovery passphrase wrong.** `that passphrase didn't decrypt — try
  again.` inline.
- **Discover returns zero groves.** Sub-screen renders its own empty state
  per `discover.md`; onboarding does not special-case.
- **Reduced-motion toggled mid-flow.** Next animation respects the new
  value; no full re-render.
- **Small viewport (< 360 px).** Ambient panel is dropped on desktop too;
  hero + forms respect the narrow column. Handled by the mobile layout
  rather than a separate breakpoint.
- **Offline during peer wait (step 4B).** `willowPulse` continues; a
  sync-queue-style badge appears near the QR: `waiting for network — the
  letter is valid, willow is patient.`
- **Double-tap `begin`.** Navigation is idempotent — the state machine
  guards against advancing twice.

## Accessibility

- **Landmarks.** Each step is `<main role="main">` with `aria-labelledby`
  pointing at its heading. Desktop ambient panel is
  `<aside role="complementary">`.
- **Headings.** Each step has a single `<h1>`. Focus moves to the heading
  on entry (imperative `.focus()` after paint); `tabindex="-1"` so it's
  focusable programmatically but not in the tab ring.
- **Step indicator.** Desktop: `<nav aria-label="onboarding progress">`,
  `<li>` with `aria-current="step"` on current. Mobile tag:
  `<span role="status" aria-live="polite">`.
- **Buttons.** Explicit `<button>` with descriptive names matching visible
  copy. Icon-only buttons carry `aria-label` (`copy letter`,
  `scan qr code`).
- **SAS grid.** Inherits from `trust-verification.md`: each word is a
  `<span>` with a number prefix that reads correctly ("one, willow; two,
  copper; …"). Grid wrapped as `<section aria-label="your fingerprint">`.
- **Forms.** Every `<input>` / `<textarea>` has a `<label>` with matching
  `for` / `id`. Required fields carry `aria-required="true"`. Invalid
  fields carry `aria-invalid="true"` and associate to inline errors via
  `aria-describedby`.
- **Focus ring.** `--focus-ring` from foundation on every interactive
  element via `:focus-visible`.
- **Reduced motion.** Disables `leafFall`, fade-ins, `willowPulse` on the
  dot indicator, and the hold-to-confirm progress fill (instant confirm).
- **Colour independence.** Selection has shape + colour (2 px ring + 1.08
  scale on swatches; tint + border on crest cards; verified badges
  include the `check` icon).
- **Touch targets.** All tappables ≥ 44 × 44 CSS px. Small visuals
  (swatches at 36 px) centred inside larger invisible hit-boxes.
- **Screen-reader announcements.** Step changes announce via
  `aria-live="polite"`: `step 3 of 6, fingerprint`. Toasts
  (`verified peer`, `copied`) announce via a second polite region.
- **Keyboard flow.** Tab order: step indicator (desktop) → heading → body
  controls → secondary CTA → primary CTA. `Enter` triggers focused button.
  `Escape` does not exit onboarding; on steps 3+ it focuses `back`.
- **No focus traps.** Tabbing past the last control wraps to browser
  chrome normally.

## Acceptance criteria

- [ ] First run with empty localStorage lands on step 1 with `leafFall`
      playing once (unless reduced motion is set).
- [ ] `begin` advances to step 2; `i already have willow` branches into
      recovery.
- [ ] `continue` on step 2 is disabled until display name is non-empty
      after trim.
- [ ] Committing step 2 generates a new Ed25519 identity via
      `willow-identity` and persists identity + profile (name, pronouns,
      crest pattern, crest accent).
- [ ] Selected crest accent immediately re-tints the app via `--moss-*`
      and `--willow`.
- [ ] Step 3 renders the user's 6-word fingerprint via the shared SAS
      components (`size="md"` desktop, `size="sm"` mobile).
- [ ] Step 4 branch A parses an inbound `willow://…` letter and delegates
      to the invite flow in `letters-dms.md`.
- [ ] Step 4 branch B generates a shareable letter with QR + copyable URL;
      `copy letter` shows the `willow-pop-in` toast.
- [ ] Completing a SAS ceremony records a verified peer and shows the
      `verified peer` toast.
- [ ] Skipping step 4 advances to step 5 with zero verified peers and no
      errors.
- [ ] Step 5 offers three paths; if step 4 produced a grove invite, card
      B is pre-populated.
- [ ] Creating a grove in step 5 emits exactly one `CreateServer` event
      with the user as owner.
- [ ] Step 6 renders four tour cards with the exact copy in this spec and
      advances to the chat UI on `enter willow`.
- [ ] Skipping steps 3/4/5/6 registers the corresponding deferred prompt.
- [ ] Closing the browser mid-flow and reopening resumes at the last step
      reached; step-level drafts are ephemeral.
- [ ] Recovery parses a valid keyfile or mnemonic, reconstructs identity,
      and routes to step 5 (or 6 if groves are known).
- [ ] Recovery failure shows invalid-error copy in `--err` and allows
      in-place retry; passphrase failure shows the passphrase error.
- [ ] SAS mismatch is non-fatal and offers retry or skip.
- [ ] Expired invite surfaces `this letter of introduction has expired —
      ask the peer for a new one.` without advancing the flow.
- [ ] `prefers-reduced-motion: reduce` disables `leafFall`, fade-ins,
      hold-to-confirm progress, and `willowPulse` indicators.
- [ ] Every copy string in this spec matches character-for-character.
- [ ] Every step has a single `<h1>`; focus moves to it on entry;
      step-change announcements appear in the polite live region.
- [ ] Every interactive element has a visible `--focus-ring` when focused
      via keyboard.
- [ ] Mobile touch targets measure ≥ 44 × 44 CSS px across all controls.
- [ ] Back button is absent on steps 1–2 (mobile) / disabled (desktop),
      present on steps 3–6.
- [ ] No telemetry or outbound network requests occur before a user
      action on step 4 or step 5.

## Open questions

- **Crest preview contract.** Step 2 reuses `profile-card.md` self variant.
  If that spec splits into self-card / peer-card components, onboarding
  follows the self-card API. Tracked in `profile-card.md`.
- **"surprise me" peek.** Should the swatch render a hash-of-peer-id preview
  circle before identity commit, so it feels less random? Deferred pending
  Tweaks spec.
- **Cross-browser recovery.** WebCrypto keystores vary. Is there a
  canonical keyfile format that works across Firefox and Chromium? Likely
  yes (JSON + mnemonic), but intersects `willow-identity`'s export API;
  tracked outside this spec.
- **Recovery skip to grove.** Current answer: go through step 6 tour.
  Alternative: drop straight into the first known grove with tour via
  Help. Revisit after early-tester feedback.
- **PWA install prompts.** Out of scope. If shipped, they appear *after*
  onboarding completes — tracked in `settings-tweaks.md`.
- **Per-grove accent vs. app-level accent.** Foundation open question.
  Step 5A's default-accent control anticipates per-grove; if app-level-
  only ships, the control is dropped and the grove inherits silently.
- **Telemetry-free completion metric.** How do we answer "are people
  finishing onboarding?" without telemetry? Likely: qualitative feedback
  only. Confirm before implementation.
- **Voice cue on step 3.** Optional soft tone to emphasise "read this
  aloud" framing. Permission-free, silenced by system mute. Deferred.
- **Tour on first-grove-entry.** Alternative: defer the tour to the first
  grove entry with inline hints. Current answer: keep dedicated step 6 +
  expose via Help. Revisit after first user study.
