# Willow docs

Master index of Willow's design specs, implementation plans, and reports. Grouped by feature area for discovery and onboarding.

For build/test/dev commands and deep architecture notes, see `../CLAUDE.md`. This file does not duplicate that content.

## Where to start (new agents and humans)

Read these in order to get the conceptual map of Willow:

1. [State authority and mutations](specs/2026-04-12-state-authority-and-mutations.md) — how every state change is authorized.
2. [Per-author Merkle DAG state](specs/2026-04-01-per-author-merkle-dag-state-design.md) — the event-sourced state model.
3. [State management model](specs/2026-04-26-state-management-model-design.md) — actors, locks, and shared mutable state.
4. [E2E test architecture](specs/2026-04-21-e2e-test-architecture-design.md) — which test tier covers which behavior.

Then skim the catalog below for the area you are working in.

## Document types

Three document types, each with one job. If a doc does not fit one of these, the type list is wrong, not the doc.

- **Spec** (`specs/`) — *what we are building toward.* Target shape of the code: types, traits, invariants, public API. Long-lived, canonical.
- **Plan** (`plans/`) — *how we get from current code to the target.* Migration steps, file-by-file changes, PR breakdown. Cites the spec it realizes. Goes stale once shipped.
- **Report** (`reports/`) — *findings from a one-shot investigation.* Audits, post-mortems. Dated, immutable.

## Status tags

Every entry below carries one of:

- `[draft]` — being written, target not yet stable.
- `[active]` — current target / in-flight migration.
- `[landed]` — realized in code; canonical reference.
- `[superseded]` — replaced; entry links to successor.

## Catalog

### State & Authority

**Specs**

- [State authority and mutations](specs/2026-04-12-state-authority-and-mutations.md) — single authority model: all state changes checked in `apply_event()` before entering the DAG. `[landed]`
- [Per-author Merkle DAG state machine](specs/2026-04-01-per-author-merkle-dag-state-design.md) — replaces linear chain with per-author DAG enabling concurrent event production. `[landed]`
- [State management model](specs/2026-04-26-state-management-model-design.md) — audit and rules for actors, locks, and shared mutable state across crates. `[draft]`
- [Reactive client state — domain actor decomposition](specs/2026-03-31-reactive-client-state-design.md) — replaces monolithic `SharedState` with domain `StateActor`s and derived views. `[active]`

**Plans**

- [State authority and mutations](plans/2026-04-12-state-authority-and-mutations.md) — adds permission pre-check before event creation and a catch-all safety gate. `[landed]`
- [Per-author Merkle DAG state machine](plans/2026-04-01-per-author-merkle-dag-state-plan.md) — replaces `willow-state` internals with the per-author DAG model in-place. `[landed]`

### Networking & Sync

**Specs**

- [Relay capability document](specs/2026-04-24-relay-capability-doc.md) — NIP-11-style `/.well-known/willow` JSON sidecar for pre-connection relay discovery. `[active]`
- [History sync — heads-based delta exchange](specs/2026-04-24-negentropy-sync.md) — consolidates client and worker sync onto the same `HeadsSummary` delta protocol. `[active]`
- [Relay discovery — pkarr plus capability negotiation](specs/2026-04-24-outbox-relay-discovery.md) — composes iroh pkarr, capability doc, and `SyncProvider` grants for relay discovery. `[active]`
- [History sync completion signal](specs/2026-04-24-history-sync-eose.md) — adds `HistorySyncComplete` wire message so clients know when backfill has finished. `[active]`
- [Iroh migration design](specs/2026-03-29-iroh-migration-design.md) — replaces libp2p with iroh QUIC transport and trait abstraction (`Network`, `TopicHandle`). `[landed]`

**Plans**

- [Iroh migration](plans/2026-03-29-iroh-migration.md) — migrates networking layer from libp2p to iroh with `IrohNetwork` and `MemNetwork`. `[landed]`

### Identity, Crypto & Trust

**Specs**

- [Epoch-driven channel key rotation](specs/2026-04-24-epoch-key-rotation.md) — derives fresh channel encryption epoch from every membership-changing state event. `[active]`
- [Direct messages — seal+gift-wrap deferral to MLS](specs/2026-04-24-seal-gift-wrap-dms.md) — captures NIP-17/44/59 investigation; defers DMs to a future MLS-over-Willow spec. `[active]`
- [Bech32-with-HRP user-facing identifiers](specs/2026-04-24-bech32-identifiers.md) — all UI-visible identifiers encoded as bech32m strings with type-tagging human-readable prefix. `[active]`
- [Shareable join links](specs/2026-03-27-shareable-join-links-design.md) — single URL triggers automatic P2P key exchange, replacing multi-step invite flow. `[active]`

**Plans**

- [Shareable join links](plans/2026-03-27-shareable-join-links.md) — implements URL-based join flow with `JoinRequest`/`JoinResponse` gossip and a dedicated join page. `[active]`

### Messaging

**Specs**

- [Willow-channel removal](specs/2026-04-12-willow-channel-removal.md) — eliminates `willow-channel` crate, making `ServerState` the client's sole source of truth. `[active]`

**Plans**

- [Willow-channel removal](plans/2026-04-12-willow-channel-removal.md) — step-by-step migration removing the parallel `willow-channel::Server` representation. `[active]`

### Workers & Actors

**Specs**

- [Actor system design](specs/2026-03-29-actor-system-design.md) — `willow-actor` framework with `Actor`, `Handler<M>`, supervision, dual native+WASM target. `[landed]`
- [Actor system library — extended actor types](specs/2026-03-31-actor-system-library-design.md) — adds `StateActor<S>`, `DerivedActor`, `Broker<T>`, FSM, pool, debounce to `willow-actor`. `[landed]`
- [Worker nodes design](specs/2026-03-27-worker-nodes-design.md) — separates relay network plumbing from state storage via specialized worker peer binaries. `[landed]`

**Plans**

- [Actor system](plans/2026-03-30-actor-system.md) — builds `willow-actor` crate and migrates worker, client, and web to use it. `[landed]`
- [Actor system library](plans/2026-03-31-actor-system-library.md) — adds generic state actors, pub/sub broker, and stream output to `willow-actor`. `[landed]`
- [Actor library migration](plans/2026-03-31-actor-library-migration.md) — replaces monolithic `SharedState` with domain `StateActor`s per the reactive client spec. `[active]`
- [Worker nodes](plans/2026-03-27-worker-nodes.md) — introduces `willow-replay` and `willow-storage` binaries sharing a `willow-worker` library. `[landed]`

### Web UI & UX

**Specs**

- [Willow UI — target UX bundle](specs/2026-04-19-ui-design/README.md) — 20+ child specs covering desktop and mobile target UX across layout, components, and interactions. `[active]`
- [UX navigation improvements](specs/2026-03-25-ux-navigation-improvements-design.md) — unifies settings, adds confirmation dialogs, breadcrumbs, and command palette. `[active]`
- [Video, screen sharing + call page](specs/2026-03-26-screen-sharing-call-page-design.md) — adds camera video, screen sharing, and full call page UI to voice chat. `[active]`
- [Async client + UI refactor](specs/2026-03-24-async-client-ui-refactor-design.md) — eliminates polling by splitting `Client` into `ClientHandle` + async event loop. `[landed]`

**Plans**

- [Async client + UI refactor](plans/2026-03-24-async-client-ui-refactor.md) — replaces mpsc polling with async channels and restructures Leptos UI with context state. `[landed]`
- [UX navigation improvements](plans/2026-03-25-ux-navigation-improvements.md) — merges settings panels, adds dialogs, server context menu, and Ctrl+K palette. `[active]`
- [Video, screen sharing + call page](plans/2026-03-26-video-screen-sharing-call-page.md) — refactors `VoiceManager`, adds video track management, and builds participant tile UI. `[active]`
- [UI phase 0 — foundation](plans/2026-04-19-ui-phase-0-foundation.md) — ships new palette, typography, and motion tokens as a `foundation.css` layer. `[landed]`
- [UI phase 1a — desktop shell](plans/2026-04-20-ui-phase-1a-desktop-shell.md) — three-pane shell, grove rail, channel sidebar, and right rail for desktop. `[landed]`
- [UI phase 1b — mobile shell](plans/2026-04-20-ui-phase-1b-mobile-shell.md) — tab bar, top bar, grove drawer, bottom sheets, and 721 px breakpoint for mobile. `[landed]`
- [UI phase 1c — command palette + accessibility](plans/2026-04-20-ui-phase-1c-palette-a11y.md) — refactors command palette, extracts keybinding layer, and adds ARIA landmarks. `[landed]`
- [UI phase 1d — trust verification](plans/2026-04-20-ui-phase-1d-trust-verification.md) — SAS fingerprint grid, trust badges, and compare-friend flow on all peer surfaces. `[landed]`
- [UI phase 1e — presence](plans/2026-04-20-ui-phase-1e-presence.md) — 7-state presence catalog, `StatusDot` atom, and self-presence override menu. `[landed]`
- [UI phase 1f — notifications](plans/2026-04-20-ui-phase-1f-notifications.md) — in-app toast stack, unread badges, OS push contract, and per-surface mute overrides. `[landed]`
- [UI phase 2a — message row](plans/2026-04-20-ui-phase-2a-message-row.md) — row anatomy, mention pills, inline code, pinned marker, and jump-to-latest pill. `[active]`
- [UI phase 2b — sync queue](plans/2026-04-21-ui-phase-2b-sync-queue.md) — offline strip, per-peer queue pills, dedicated sync-queue screen, and reconnection toast. `[active]`
- [UI phase 2c — profile card](plans/2026-04-21-ui-phase-2c-profile-card.md) — 17-field profile popover/sheet, crest banner, and private nickname editor. `[landed]`
- [UI phase 2d — ephemeral channels](plans/2026-04-25-ui-phase-2d-ephemeral-channels.md) — auto-archive on inactivity, archives surface, kind chip, and revive flow. `[landed]`
- [UI phase 2e — local search](plans/2026-04-21-ui-phase-2e-local-search.md) — on-device encrypted search index with scope ladder and streamed results surface. `[active]`
- [Issue #354 — search index incremental rebuild](plans/2026-05-02-issue-354-search-incremental.md) — replaces per-message-list-change full index rebuild with incremental updates. `[active]`

See also: [`plans/STATUS.md`](plans/STATUS.md) — point-in-time audit of which UI-phase plans have landed.

### Agent / MCP

**Specs**

- [Agentic peer API design](specs/2026-03-29-agentic-peer-api-design.md) — exposes `ClientHandle` to AI agents via an MCP server binary (`willow-agent`). `[active]`
- [LLM agent UX spec](specs/2026-04-25-llm-agent-ux-spec-design.md) — design for first-class LLM agent peers with governance tools and agent-readable UI surfaces. `[active]`

**Plans**

- [Agentic peer API](plans/2026-04-01-agentic-peer-api.md) — builds `willow-agent` MCP binary and multi-peer E2E test harness in four phases. `[active]`

### Testing

**Specs**

- [E2E test architecture](specs/2026-04-21-e2e-test-architecture-design.md) — tier decision tree pushing tests to the lowest level covering each behavior. `[draft]`
- [Test architecture](specs/2026-04-13-test-architecture.md) — earlier test philosophy and per-crate coverage targets. `[superseded]`
- [Event-based waits in Playwright suite](specs/2026-04-27-event-based-waits-design.md) — replaces magic-number sleeps with `WillowTestHooks` WASM API and `data-state` lifecycle. `[active]`
- [Multi-peer E2E browser tests](specs/2026-03-24-multi-peer-e2e-tests-design.md) — Playwright suite covering sync, permissions, and mobile flows across four browser projects. `[active]`

**Plans**

- [E2E test architecture](plans/2026-04-21-e2e-test-architecture.md) — migrates tests off Playwright to lower tiers in three phases, then documents the tier rules. `[active]`
- [Multi-peer E2E browser tests](plans/2026-03-24-multi-peer-e2e-tests.md) — adds shared helpers and three Playwright spec files for multi-peer and mobile flows. `[active]`
- [Event-based waits PR 1 — test-hooks foundation](plans/2026-04-27-event-based-waits-pr1-test-hooks-foundation.md) — lands `WillowTestHooks` WASM API, push dispatcher, and ESLint rule for `waitForTimeout`. `[landed]`
- [Event-based waits PR 1 errata](plans/2026-04-28-event-based-waits-pr1-errata.md) — corrections to PR-1 plan based on real API investigation during implementation. `[landed]`
- [Event-based waits PR 2 — Playwright `Peer` wrapper](plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md) — typed `Peer` class, helpers split, and pilot migration of `multi-peer-sync.spec.ts`. `[landed]`
- [Event-based waits PR 3 — `data-state` lifecycle](plans/2026-04-30-event-based-waits-pr3-data-state-lifecycle.md) — adds four-phase `data-state` attribute on animated elements and adopts `page.clock`. `[landed]`
- [Event-based waits PR 4 — wait-timeout ratchet + flake harness](plans/2026-04-30-event-based-waits-pr4-ratchet-flake-harness.md) — CI script ratcheting `waitForTimeout` count and flake harness running suite N times. `[active]`

**Reports**

- [Test audit](reports/2026-04-13-test-audit.md) — audit of 14 crates finding coverage gaps at client, relay, and UI/state bridge layers. `[active]`

### Process & Tooling

**Specs**

- [Machine-readable wire-rejection reasons](specs/2026-04-24-error-prefixes.md) — typed `WireRejectReason` enum in `WireMessage::Reject` replacing free-form error strings. `[active]`
- [Docs organization — target structure](specs/2026-05-07-docs-organization-design.md) — target structure for `docs/`, master index, naming conventions, and nesting rules. `[active]`

**Plans**

- [Docs organization](plans/2026-05-07-docs-organization.md) — populates the master index, creates the skill mirror, and folds the design orphan into specs. `[active]`

## Reference designs

Archived design bundles (immutable inputs to specs, not specs themselves) live in [`reference-designs/`](reference-designs/README.md).

## Conventions

Cemented in [`specs/2026-05-07-docs-organization-design.md`](specs/2026-05-07-docs-organization-design.md). Mirrored on demand by the `organizing-willow-docs` skill. Summary:

### Naming

| Type | Pattern | Example |
|---|---|---|
| Spec | `specs/YYYY-MM-DD-<kebab>-design.md` | `2026-05-07-docs-organization-design.md` |
| Multi-file spec | `specs/YYYY-MM-DD-<kebab>/README.md` + children | `2026-04-19-ui-design/README.md` |
| Plan | `plans/YYYY-MM-DD-<kebab>.md` | `2026-04-21-e2e-test-architecture.md` |
| Report | `reports/YYYY-MM-DD-<kebab>.md` | `2026-04-13-test-audit.md` |

The date is when the doc was written, not the implementation target. The `-design.md` suffix on specs is what visually distinguishes specs from plans in `ls` output. Existing files predating this convention are not renamed.

### Document headers

Every new spec, plan, and report opens with:

```
**Date:** YYYY-MM-DD
**Status:** draft | active | landed | superseded
**Spec:** specs/...      (plans only — REQUIRED, points at the spec being realized)
**Supersedes:** specs/... (if applicable)
```

### Nested folders

Use a folder (`specs/YYYY-MM-DD-<topic>/README.md` + children) only when one logical document is too large for a single file *and* the children are tightly coupled. Children use kebab-case topic names (no date prefix — they inherit the parent's date). Maximum one level deep. Multiple independent docs sharing a topic stay flat — example: the `ui-phase-1a` … `ui-phase-2e` plan series.

### Adding a new spec, plan, or report

1. Pick the right type (spec = target, plan = migration, report = audit).
2. Name with `YYYY-MM-DD-<kebab>-design.md` (spec) or `YYYY-MM-DD-<kebab>.md` (plan/report).
3. Add a one-line entry to this README under the right area, with a 5–15 word summary and `[draft]` tag.
4. Plans must include `**Spec:** specs/...` in their header.
5. Multi-file specs nest under `YYYY-MM-DD-<topic>/` with a required `README.md`.
