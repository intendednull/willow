# Willow UI — target UX (parent spec)

**Date:** 2026-04-19
**Status:** draft
**Source bundle:** `docs/reference-designs/2026-04-19-willow-design-bundle.tar.gz`
**Branch:** `design/ui-target-ux`

## Purpose

This folder is the target UX for the Willow client across desktop and mobile
web. It is a *destination*, not a description of current state. The existing
Leptos client ships a Discord-style layout with a blurple palette; the spec
below describes the design Willow is moving toward. Feature disparity between
spec and client is expected and acceptable — each child spec is scoped to be
implementable independently via its own plan in `docs/plans/`.

## Scope

- `crates/web` as the single deliverable surface (desktop + mobile web via
  breakpoint swap + platform-aware chrome).
- Shared design tokens exposed as CSS variables so future native clients
  inherit the language.
- Copy, iconography, motion, and terminology changes are in scope everywhere
  they appear in the client.

## Non-goals

- Native iOS / Android apps. Design references those platforms as *vocabulary*
  (status bar, material chrome, liquid glass) but production targets only the
  web runtime; mobile web respects platform conventions when unobtrusive.
- Rewriting state, network, or storage layers. Every UX in scope maps to
  existing `willow-state` events or explicitly marks a dependency on a new
  one. Backend work is called out in the child spec and tracked separately.
- Data migration. Copy changes and CSS variable renames do not require state
  migration.

## Design principles

1. **Trust-first, but calm.** Crypto state is always visible — never loud.
   Subtle by default; explicit only where it protects a decision (SAS
   verification, whisper activation, device handoff, ephemeral expiration).
2. **Reading-room cadence.** A literary pace: serifs for display, mono for
   crypto artefacts, sans for body. Density defaults to *balanced*; dashboard
   energy is avoided.
3. **Nature metaphor carries meaning.** "Grove" = community boundary.
   "Channel" = room. "Letter" = direct message. "Whisper" = side-channel.
   "Queue" = store-and-forward. "Sealed" = encrypted. Metaphor is
   load-bearing — it tells users what the system does.
4. **Platform-native chrome.** Desktop uses hover affordances and keyboard
   flows; mobile uses touch and gesture. Content is shared; chrome is not.
5. **Decentralization is the product.** There is no hidden server authority.
   Offline is *queued*, not *broken*. Identity is *verified*, not *claimed*.
   Every peer's trust state is reachable from their profile.
6. **Accessibility is a baseline, not a polish pass.** Focus rings, reduced
   motion, screen-reader labels, colour-independent state indicators are
   requirements in every child spec.

## Design language at a glance

See `foundation.md` for full definitions.

- **Palette:** bark + moss on ink-on-deep-bark darks. Accent swaps between
  `moss`, `willow`, `amber`, `dusk`, `cedar`, `lichen`, `ember` variants.
  Whisper is violet (`#a88fc9`); semantics are `ok`, `warn`, `err`.
- **Typography:** Fraunces (display, italic-friendly), IBM Plex Sans (UI),
  JetBrains Mono (crypto artefacts).
- **Iconography:** 1.5px stroke, round caps, 24 px viewBox, `currentColor`.
- **Motion:** slow easings, never jumpy. `willowPulse` for presence; `leafFall`
  for ambient ephemeral warning; `willow-pop-in` for toasts and popovers.
- **Density:** `cozy`, `balanced` (default), `dense` — changes message vertical
  rhythm only.
- **States:** empty, loading (shimmer skeleton or mono `loading…`), error
  (token + icon + recovery action), skeleton. Defined in `foundation.md` §States.

## Terminology map

User-visible copy only. Internal code identifiers (`server`, `channel`, `dm`,
`peer`, `event`) are unchanged; this is a UX vocabulary swap, not a rename
refactor. Each child spec lists the exact copy strings it owns.

| Concept (code)     | User-visible term        | Notes |
|--------------------|--------------------------|-------|
| server             | grove                    | plural: groves |
| channel            | channel                  | kinds: text, voice, *ephemeral* |
| dm (peer)          | letter                   | plural: letters; one-on-one |
| dm (group)         | letter · group           | |
| member             | member                   | |
| peer               | peer                     | |
| fingerprint        | fingerprint              | short form: 3 words; full form: 6 words |
| join code          | letter of introduction   | invite link UI |
| pinned             | pinned                   | |
| reaction           | reaction                 | |
| thread             | thread                   | sealed to thread participants |
| private side-chan  | whisper                  | |
| ephemeral channel  | ephemeral                | |
| offline queue      | sync queue               | |
| device transfer    | handoff                  | verb: *move this call* |
| verified (SAS)     | verified peer            | inverse: *unverified — compare fingerprints* |
| presence           | here / away / whispering / in a call / queued / gone | see `presence.md` |

## Spec inventory

Two classes of document live in this folder:

- **UX specs** — the behavioural / visual / copy source of truth for a
  specific surface or atom. Each has its own dependency header and drives
  one implementation plan.
- **Reference docs** — cross-cutting artefacts derived from the specs.
  They do not drive plans; they exist to prevent drift and keep the spec
  set coherent.

### UX specs

| # | Spec | Purpose |
|---|------|---------|
| 1 | [`foundation.md`](foundation.md) | Palette, typography, iconography, motion, density, states, accent variants, copy voice, accessibility baseline. Everything else depends on this. |
| 2 | [`layout-primitives.md`](layout-primitives.md) | Desktop three-pane + mobile bottom-tab chrome. Grove rail, channel sidebar, main pane, right rail, mobile tab bar, swipe drawer, bottom sheets, command palette. |
| 3 | [`trust-verification.md`](trust-verification.md) | SAS 6-word fingerprint grid, verified / unverified / pending-verify badges, `add a friend` compare flow, long-press SAS on mobile, holder counts per channel. |
| 4 | [`presence.md`](presence.md) | Canonical presence state catalog + `StatusDot` + `PeerStatusLabel` atoms + self-presence overrides. |
| 5 | [`notifications.md`](notifications.md) | In-app toast stack, unread badge rendering, OS push content contract, sound, per-surface mute overrides. |
| 6 | [`local-search.md`](local-search.md) | On-device encrypted-at-rest search index + scope ladder + `/`, `⌘F`, palette entry points. |
| 7 | [`message-row.md`](message-row.md) | Message rendering, author grouping, day separators, mentions, inline code, pin indicator, queue notes, whisper-marked row, hover toolbar container, long-press action-sheet container, swipe gestures. |
| 8 | [`composer.md`](composer.md) | Textarea autosize, attach / emoji / send buttons, reply preview, edit mode, keybindings, mention autocomplete, typing indicator, placeholder copy per channel kind. |
| 9 | [`reactions-pins.md`](reactions-pins.md) | Reactions strip, reactor tooltip, emoji picker, pin action + permission gate, pinned panel contents. |
| 10 | [`files-inline.md`](files-inline.md) | File card, inline image, voice note, upload dialog, drag-and-drop, paste-to-upload. |
| 11 | [`thread-pane.md`](thread-pane.md) | Thread parent card, reply list, sealed-participants footer, thread composer. Right-rail on desktop, full-screen on mobile. |
| 12 | [`profile-card.md`](profile-card.md) | Enriched profile: pronouns, nickname, bio, tagline, crest pattern, pinned fragment, shared groves, elsewhere. Desktop popover, mobile bottom sheet. |
| 13 | [`ephemeral-channels.md`](ephemeral-channels.md) | Timer surfacing, expiration warnings, "keys burned" copy, lock-screen-style timer on mobile, creation + extension flow. |
| 14 | [`sync-queue.md`](sync-queue.md) | Offline indicator, per-peer queue count, per-message queue note, pull-down-to-reveal on mobile, dedicated sync-queue screen. |
| 15 | [`call-experience.md`](call-experience.md) | Grove / grid / focus layouts, participant tile, screen-share canvas, speaking stats, controls strip. |
| 16 | [`whisper-mode.md`](whisper-mode.md) | Whisper pill inside calls, whisper-marked letters, whisper-marked messages, whisper status on profile, activation + teardown UX. |
| 17 | [`device-handoff.md`](device-handoff.md) | `move this call` popover, device list, re-seal messaging, handoff confirmation, desktop popover vs mobile sheet. |
| 18 | [`letters-dms.md`](letters-dms.md) | Peer letters + group letters list, verified / unverified / pending markers, whisper / queued markers, thread, composition. |
| 19 | [`discover.md`](discover.md) | Grove discovery directory, cards, join flow. |
| 20 | [`governance.md`](governance.md) | Governance, manage (roles / invites / files), event log. Owner/admin surfaces. |
| 21 | [`onboarding.md`](onboarding.md) | Identity creation, crest + pronouns, `add a friend` intro, first-grove flow, first-SAS ceremony. |
| 22 | [`settings-tweaks.md`](settings-tweaks.md) | Account settings + Tweaks panel (accent swap, density, crypto visibility, wordmark toggle). |

### Reference docs

| Doc | Purpose |
|-----|---------|
| [`flows.md`](flows.md) | 15 canonical user journeys stitched across specs. Surfaces hand-off gaps between specs. |
| [`security-posture.md`](security-posture.md) | Threat model, trust boundaries, UX invariants, red-line prohibitions. Vetoes future spec changes that violate security properties. |
| [`data-deps-rollup.md`](data-deps-rollup.md) | Aggregated view of every new `willow-state` event, signal, storage shape, and protocol the UX demands. Consult before writing plans. |
| [`audit.md`](audit.md) | Fresh-agent audit of specs vs reference bundle (2026-04-19). Historical record of gaps found + fixed. |

## Dependency graph

Every spec declares its own dependencies in its header. The tier table below
is the full DAG assembled from those declarations. Edges read "requires":
`a → b` means `a` needs `b` defined and agreed first. The graph is acyclic.
Reference docs have no tier — they stand outside the implementation order.

### Tier table

| Tier | Spec | Requires |
|------|------|----------|
| 0 | `foundation.md` | — |
| 1 | `layout-primitives.md` | foundation |
| 1 | `trust-verification.md` | foundation |
| 1 | `presence.md` | foundation, layout-primitives |
| 1 | `notifications.md` | foundation, layout-primitives |
| 2 | `message-row.md` | foundation, layout-primitives |
| 2 | `profile-card.md` | foundation, layout-primitives, trust-verification |
| 2 | `ephemeral-channels.md` | foundation, layout-primitives |
| 2 | `sync-queue.md` | foundation, layout-primitives, message-row |
| 2 | `local-search.md` | foundation, layout-primitives |
| 3 | `composer.md` | foundation, layout-primitives, message-row |
| 3 | `reactions-pins.md` | foundation, layout-primitives, message-row |
| 3 | `files-inline.md` | foundation, layout-primitives, message-row, composer |
| 3 | `thread-pane.md` | foundation, layout-primitives, message-row, composer |
| 3 | `call-experience.md` | foundation, layout-primitives, message-row, composer |
| 3 | `letters-dms.md` | foundation, layout-primitives, message-row, composer, profile-card, trust-verification, sync-queue, local-search |
| 3 | `governance.md` | foundation, layout-primitives, profile-card, trust-verification |
| 4 | `whisper-mode.md` | call-experience, letters-dms, message-row, trust-verification |
| 4 | `device-handoff.md` | call-experience |
| 4 | `discover.md` | layout-primitives, letters-dms, governance, trust-verification |
| 5 | `onboarding.md` | trust-verification, profile-card, letters-dms, discover |
| 5 | `settings-tweaks.md` | layout-primitives, profile-card, trust-verification, device-handoff, whisper-mode, presence, notifications, local-search |

### Critical path

The DAG has wide parallelism in the middle tiers but narrow critical paths
at the ends. Schedule around the long chain, not the wide ones.

- **Longest chain (5 hops):** foundation → layout-primitives → message-row →
  letters-dms → discover → onboarding. This is the rate-limiter for the
  fresh-install journey.
- **Tier-1 atoms are load-bearing.** `trust-verification.md`, `presence.md`,
  and `notifications.md` show up in every peer-facing surface. Slipping any
  of the three slips every feature spec that consumes it.
- **message-row is the single tallest dependency** — thread-pane, composer,
  reactions-pins, files-inline, call-experience, letters-dms, sync-queue,
  and whisper-mode all sit above it.
- **Leaf specs** (onboarding, settings-tweaks, discover) ship last because
  they integrate every atom. Plans for leaves are larger and riskier than
  plans for atoms; budget accordingly.

## Implementation phases

Plans are written one phase at a time, on request, and reviewed between
phases. No phase is allowed to start before its predecessor has shipped.

| Phase | Specs |
|-------|-------|
| 0 · foundation shell | `foundation.md` |
| 1 · shell + tier-1 atoms | `layout-primitives.md`, `trust-verification.md`, `presence.md`, `notifications.md` |
| 2 · content primitives + data atoms | `message-row.md`, `profile-card.md`, `ephemeral-channels.md`, `sync-queue.md`, `local-search.md` |
| 3 · feature surfaces | `composer.md`, `reactions-pins.md`, `files-inline.md`, `thread-pane.md`, `call-experience.md`, `letters-dms.md`, `governance.md` |
| 4 · cross-cutting novel mechanics | `whisper-mode.md`, `device-handoff.md`, `discover.md` |
| 5 · entry + prefs | `onboarding.md`, `settings-tweaks.md` |

## Status ladder

Every spec + every plan goes through this ladder. Status is recorded in
the document's front-matter.

| Status | Meaning | Gate |
|--------|---------|------|
| `draft` | Initial state | author satisfied |
| `reviewed` | A second human has read it end-to-end | reviewer sign-off |
| `approved` | Locked; plan can be written from it | reviewer + maintainer sign-off |
| `implementing` | Plan is running against this spec | plan PR open |
| `shipped` | Plan merged to main | merge commit |
| `deprecated` | Replaced by a successor doc | successor link in front-matter |

Security-critical specs (see `security-posture.md`) require **two** human
reviewers before moving past `draft`.

## Test strategy

Every child spec adds (or updates) a `## Tests to add` section before moving
to `approved`. The section must list, where applicable:

- **Browser tests** (`crates/web/tests/browser.rs`) — component assertions,
  signal flips, DOM queries.
- **Playwright E2E** (`e2e/*.spec.ts`) — cross-peer or mobile-only flows.
- **State machine tests** (`crates/state/src/tests.rs`) — if the spec names
  new events, permissions, or merges.
- **Client tests** (`crates/client/src/lib.rs`) — if the spec adds or
  changes `ClientHandle` methods.
- **Accessibility automation** — axe rules to run in browser tests;
  manual-only checks flagged for review.

The lowest tier at which a feature can be tested is the right tier. Prefer
state tests over client tests, client tests over browser tests, browser
tests over Playwright E2E.

## Copy voice

- Lowercase, intimate, slightly literary. Avoid corporate voice.
- Nature metaphor is intentional and consistent. Don't break it for a single
  button.
- Security copy explains *why*, not just *how*: "compare six words — if they
  match, no one can impersonate either of you in this conversation, ever."
- Avoid pejorative framings of the offline case. Offline is *queued*, not
  *failed*. The queue waits patiently.
- Placeholder / empty-state copy has a voice. "no letters yet — send the
  first" beats "no items".
- `not a server — held between us` is load-bearing grove vocabulary; keep it
  where space allows (foundation §Copy voice owns this rule).

## Motion

- Transitions: 120 ms for hover, 180 ms for open/close, 240 ms for drawer
  slide. Always `cubic-bezier(0.2, 0.8, 0.2, 1)` unless a child spec declares
  otherwise.
- Ambient: `willowPulse` for presence dots, voice "listening" counters.
  `leafFall` allowed in ephemeral-channels and onboarding moments only.
- Reduced motion: every animation must have a `prefers-reduced-motion: reduce`
  path that collapses to opacity-only.

## Accessibility baseline

- Colour is never the only signifier. Whisper is violet *and* has an ear
  icon; unverified is amber *and* has a dashed ring; queued is amber *and*
  has an hourglass.
- Focus-visible is required on every interactive element. The `--focus-ring`
  token is defined in `foundation.md`.
- Touch targets ≥ 44 × 44 CSS px on mobile.
- Screen-reader labels are required on every icon-only button. Each child
  spec lists its labels.
- Keyboard path exists for every interaction. Mobile long-press has a
  keyboard-accessible equivalent (Enter opens menu).

## Review process

Every spec set goes through this process before plans land:

1. Each child spec is written to `draft`.
2. A fresh audit agent compares the set against the reference bundle and
   writes `audit.md`. First pass: 2026-04-19 (committed).
3. Blocker + major findings are addressed before any plan starts.
4. Each child moves to `reviewed` after a second human reads it end-to-end.
5. Security-critical specs need a second reviewer sign-off to reach
   `approved`.
6. Plans are written one phase at a time (see §Implementation phases) and
   reviewed before the next phase begins.

## After spec approval

1. Phase-scoped implementation plan under `docs/plans/YYYY-MM-DD-ui-phase-N-<slug>.md`
   using the writing-plans skill.
2. Each plan is reviewed before implementation begins.
3. Implementation ships as small PRs — one per spec or tight cluster
   within a phase.
4. Phase gate check (visual + functional + test sign-off) before moving
   to the next phase.
