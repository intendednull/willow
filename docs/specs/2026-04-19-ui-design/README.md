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

## Child spec index

Each child spec is a self-contained unit that can be planned and implemented
on its own. The parent spec (this file) is the integration contract: anything
a child needs from another child is declared explicitly as a dependency in its
header.

| # | Spec | Purpose |
|---|------|---------|
| 1 | [`foundation.md`](foundation.md) | Palette, typography, iconography, motion, density, accent variants, copy voice, accessibility baseline. Everything else depends on this. |
| 2 | [`layout-primitives.md`](layout-primitives.md) | Desktop three-pane + mobile bottom-tab chrome. Grove rail, channel sidebar, main pane, thread/members right rail, mobile tab bar, swipe drawer, bottom sheets. |
| 3 | [`messaging.md`](messaging.md) | Message rendering, mentions, reactions, pins, code blocks, inline files, hover toolbar (desktop), long-press action sheet (mobile), edit/delete/reply, composer, typing. |
| 4 | [`thread-pane.md`](thread-pane.md) | Thread parent card, reply list, sealed-participants footer, composer. Right-rail on desktop, full-screen on mobile. |
| 5 | [`profile-card.md`](profile-card.md) | Enriched profile: pronouns, nickname, bio, tagline, crest pattern, pinned fragment, shared groves, elsewhere. Desktop popover, mobile bottom sheet. Self vs peer variants. |
| 6 | [`trust-verification.md`](trust-verification.md) | SAS 6-word fingerprint grid, verified / unverified / pending-verify badges, "compare fingerprints" flow, long-press SAS on mobile, holder counts per channel. |
| 7 | [`whisper-mode.md`](whisper-mode.md) | Whisper pill inside calls, whisper-marked letters, whisper-marked messages, whisper status on profile, activation + teardown UX. |
| 8 | [`ephemeral-channels.md`](ephemeral-channels.md) | Timer surfacing, expiration warnings, "keys burned" copy, lock-screen-style timer on mobile, creation + extension flow. |
| 9 | [`sync-queue.md`](sync-queue.md) | Offline indicator, per-peer queue count, per-message queue note, pull-down-to-reveal on mobile, dedicated sync-queue screen. |
| 10 | [`device-handoff.md`](device-handoff.md) | "Move this call" popover, device list, re-seal messaging, handoff confirmation, desktop popover vs mobile sheet. |
| 11 | [`call-experience.md`](call-experience.md) | Grove call layout, grid / focus / spotlight, screen-share canvas, speaking stats, controls strip, whisper + handoff integration points. |
| 12 | [`letters-dms.md`](letters-dms.md) | Peer letters + group letters list, verified / unverified / pending markers, whisper / queued markers, open thread, composition surface. |
| 13 | [`discover.md`](discover.md) | Grove discovery directory, cards, join flow. |
| 14 | [`governance.md`](governance.md) | Governance, manage (roles / invites / files), event log. Owner/admin surfaces. |
| 15 | [`onboarding.md`](onboarding.md) | Identity creation, crest + pronouns, fingerprint intro, first-grove flow, first-SAS ceremony. |
| 16 | [`settings-tweaks.md`](settings-tweaks.md) | Account settings + Tweaks panel (accent swap, density, crypto visibility, wordmark toggle). |

## Dependency graph

Each child declares its dependencies in its own header. Below is the full
DAG assembled from those declarations. Edges read "requires" (a → b means
"a requires b to be defined and agreed first"). The graph is acyclic; a
valid implementation order is any topological sort.

### Tier table

| Tier | Spec | Requires |
|------|------|----------|
| 0 | `foundation.md` | — |
| 1 | `layout-primitives.md` | foundation |
| 1 | `trust-verification.md` | foundation |
| 2 | `messaging.md` | layout-primitives |
| 2 | `profile-card.md` | layout-primitives, trust-verification |
| 2 | `ephemeral-channels.md` | layout-primitives |
| 2 | `sync-queue.md` | layout-primitives, messaging |
| 3 | `thread-pane.md` | messaging |
| 3 | `call-experience.md` | layout-primitives, messaging |
| 3 | `letters-dms.md` | messaging, profile-card, trust-verification, sync-queue |
| 3 | `governance.md` | layout-primitives, profile-card, trust-verification |
| 4 | `whisper-mode.md` | call-experience, letters-dms, messaging, trust-verification |
| 4 | `device-handoff.md` | call-experience |
| 4 | `discover.md` | layout-primitives, letters-dms, governance, trust-verification |
| 5 | `onboarding.md` | trust-verification, profile-card, letters-dms, discover |
| 5 | `settings-tweaks.md` | layout-primitives, profile-card, trust-verification, device-handoff, whisper-mode |

### ASCII DAG

```
foundation
  ├─ layout-primitives
  │   ├─ messaging
  │   │   ├─ thread-pane
  │   │   ├─ sync-queue  (also uses layout-primitives directly)
  │   │   ├─ call-experience
  │   │   │   ├─ whisper-mode  (also uses letters-dms, messaging, trust-verification)
  │   │   │   └─ device-handoff
  │   │   └─ letters-dms  (also uses profile-card, trust-verification, sync-queue)
  │   │       └─ discover  (also uses governance, trust-verification)
  │   ├─ ephemeral-channels
  │   ├─ profile-card  (also uses trust-verification)
  │   └─ governance  (also uses profile-card, trust-verification)
  └─ trust-verification
      └─ onboarding  (also uses profile-card, letters-dms, discover)
      └─ settings-tweaks  (also uses layout-primitives, profile-card,
                              device-handoff, whisper-mode)
```

`foundation.md` is the only root. `trust-verification.md` sits next to
layout-primitives at tier 1 because it exposes atoms (SAS grid + badge)
that later specs render directly — `profile-card.md` renders the badge;
it does not define it. Visual composition (a whisper-pill appearing in
the call header) is not the same as spec dependency: `call-experience.md`
exposes the header slot, `whisper-mode.md` owns the pill that goes into
it.

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

## Review

When every child is committed, dispatch a fresh audit agent that compares the
set of specs against the reference bundle at
`docs/reference-designs/2026-04-19-willow-design-bundle.tar.gz` and reports:

- Screens in the bundle not covered by any child
- Novel mechanics in the bundle not covered by any child
- Copy strings that drift from the bundle
- Internal contradictions between children
- Anything in a child that invents UX not grounded in the bundle

Audit output goes to `docs/specs/2026-04-19-ui-design/audit.md` and is
reviewed before implementation plans are written.

## After spec approval

1. Human review of every child (or batch review if dependencies hold).
2. Per-child implementation plans under `docs/plans/` using the
   writing-plans skill.
3. Plans are merged independently — a child spec + its plan can ship without
   waiting for siblings, as long as dependencies are respected.
