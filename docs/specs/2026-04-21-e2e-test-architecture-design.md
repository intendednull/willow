# E2E Test Architecture Design

**Date:** 2026-04-21
**Status:** draft
**Branch:** `e2e/test-architecture`

## Problem

Full E2E run (`just test-e2e-full`) takes ~40 minutes wall-clock. Root cause audit: 52% is fixed `waitForTimeout` sleeps, 25% is repeated `setupTwoPeers` overhead (freshStart + welcome + invite + join settle), 12% is actual work, 10% is retry amplification on flakes. Iteration is slow enough that developers skip running the full suite locally, which lets selector drift and dead tests accumulate.

Underlying architectural cause: Playwright covers concerns that belong at lower test tiers. Single-peer DOM assertions, client API behaviour, and state-machine logic have all leaked into multi-browser E2E specs. Each of those runs a full browser + Trunk serve + iroh relay just to verify a signal update — dozens of times per suite.

## Goal

Make the full test suite as fast as possible at all times. Every test runs at the lowest tier that can cover the behaviour. Developers run the full gate locally without friction; CI completes well under prior wall-clock.

## Scope

- **In scope:** Full rearchitecture of where tests live (Rust state / client / wasm-pack / Playwright). Parallelism + deterministic-wait cleanup in existing Playwright specs. New `just check-all` umbrella recipe. Classification rules documented in CLAUDE.md + `e2e/README.md`. Guideline enforced via PR checklist.
- **Out of scope:** Adding new test coverage for behaviours that aren't already exercised. Native client tests. Rust integration tests against real iroh QUIC (use MemNetwork where a Rust tier is possible; real iroh stays in Playwright).
- **Not a rewrite:** existing coverage is preserved; tests move tiers or get replaced with equivalent coverage at the new tier.

## Architecture — 3-tier test pyramid

### Tier 1 — Rust tests (`just test` / `cargo test`, seconds)
- State machine logic: event application, permission enforcement, merge convergence, dedup, HLC ordering.
- Client actor flows: `ClientHandle` API methods, mutations (create/join/mute/trust/verify/send/edit/delete/react/pin), derived view computation (`channels`, `unread`, `presence`, `roles`, `connection`).
- Multi-peer sync scenarios that can use `MemNetwork` (pure in-memory gossip fixture) — invite + join + message replay + reconnect + SyncBatch semantics.
- Parallel by default. Ground floor of coverage.

### Tier 2 — wasm-pack browser tests (`just test-browser`, ~1 min)
- Component DOM rendering + signal reactivity (already exists).
- Keyboard shortcuts, ARIA landmarks, focus traps, aria-live announcements, reduced-motion CSS paths.
- Single-client user flows with a mocked `Network` trait (MemNetwork compiled to wasm) — channel create / send message / edit / delete / react / mute — the class of behaviour currently in `basic-flow.spec.ts`.
- Desktop vs mobile shell selection via an explicit `data-shell` attribute hook or a `mount_test_with_shell(Shell::Mobile)` helper that bypasses the viewport media query — both shells mount into isolated test containers.

### Tier 3 — Playwright (`just test-e2e-ui`, target <3 min after migration)
- Multi-peer real-network P2P flows (real iroh + relay gossip, SyncBatch over the wire).
- Cross-browser compatibility (Firefox-specific quirks, Safari if/when added).
- Touch gestures: swipe, long-press, pull-down — anything that needs real browser touch-event dispatch.
- Viewport-driven responsive breakpoints when the media query itself is the behaviour under test.
- Browser-integration paths: service worker registration + push dispatch, clipboard, navigator, browser back/forward + hash routing, fullscreen.

## Classification rules

Decision tree for every new test:

1. **State-machine logic only?** → Rust state crate test.
2. **Client API + derivation, no DOM?** → Rust client crate test.
3. **Multi-peer sync semantics?** → Rust client crate test with `MemNetwork` (unless validating real iroh/QUIC behaviour specifically).
4. **Needs DOM rendering or event dispatch?**
   - **Single client + single viewport** → wasm-pack browser test.
   - **Multi-client or multi-viewport** → Playwright.
5. **Needs cross-browser quirk coverage?** → Playwright.
6. **Needs touch / gesture / mobile-shell media query?** → Playwright mobile-chrome.
7. **Needs service worker, push, or navigator APIs?** → Playwright.

**Default to the lowest tier that can cover the behaviour.**

**Rewrite trigger:** when a Playwright test fails because a selector or helper drifts — not because behaviour broke — that's a signal the test is at the wrong tier. Migrate it down on the same commit.

## `just check-all` contract

Single umbrella recipe — the gate for "full test suite". Runs sequentially with fail-fast:

1. `just fmt && just clippy` — lint gate (seconds).
2. `just test` — Rust unit + integration (parallel, ~1 min).
3. `just test-browser` — wasm-pack headless Firefox (~1 min).
4. `just test-e2e-ui` — Playwright desktop-chrome + mobile-chrome (<3 min target post-migration).

Upper tiers only run if lower tiers pass. First failure aborts the chain.

**`just check` unchanged** — stays as the quick dev gate (fmt + clippy + Rust + WASM compile).

**`just test-e2e-full` unchanged** — still wraps setup + test + teardown for CI; delegates to `just test-e2e-ui` internally.

**CI pipeline** — runs `just check-all`. Branch protection: this goes green before merge.

## Migration plan

Three phases. Each phase is shippable on its own; the architecture doesn't require completing all three to see value.

### Phase A — Playwright speedup (no migration yet)

Target: <8 min full E2E wall-clock.

- Set `fullyParallel: true` + `workers: 4` in `playwright.config.ts`, env-toggled so CI or flaky-relay environments can opt out.
- Guard shared-relay-heavy specs (`cross-browser-sync.spec.ts`, `multi-peer-sync.spec.ts`, `multi-peer-mobile.spec.ts`) with `test.describe.configure({ mode: 'serial' })` so tests inside a file don't stampede the relay, while different files still run concurrently.
- Replace the hard-coded `waitForTimeout(3000)` post-join + `waitForTimeout(1000)` waitForApp settle in `e2e/helpers.ts` with `locator().waitFor()` on a deterministic post-join signal (first `.channel-item` or `.app-shell`).
- Audit every remaining `waitForTimeout(200..500)` in `e2e/helpers.ts` and the `e2e/*.spec.ts` files; replace each with an explicit `expect().toBeVisible()` on the resulting state, or delete if auto-wait already covers it.
- Fix `scripts/setup-e2e.sh` to skip `playwright install --with-deps` (fails on sandboxes without interactive sudo); check the filesystem for an existing `chromium-*` cache directory instead.

### Phase B — Tier migration

Target: Playwright suite <3 min wall-clock; only multi-peer + cross-browser + gesture remain.

For each Playwright test:
1. Apply the classification rules. Identify the correct tier.
2. Add an equivalent test at the new tier (Rust or wasm-pack). Confirm it passes and covers the same behaviour.
3. Delete the Playwright test. Commit the migration as a single change so bisect stays clean.

Expected migrations:
- `basic-flow.spec.ts` single-peer tests → `crates/web/tests/browser.rs` (new module `single_client_flow`).
- `mobile.spec.ts` non-gesture tests → `crates/web/tests/browser.rs` with a shell selector helper.
- `mobile-actions.spec.ts` action-sheet behaviour → wasm-pack; gesture (pointer-down timing) stays in Playwright.
- `worker-nodes.spec.ts` member-list / CSS-class probes → wasm-pack.
- `permissions.spec.ts` trust + untrust flows → Rust client crate tests (MemNetwork); kick + member-count drop stays in Playwright because it exercises real-peer member sync.
- `multi-peer-sync.spec.ts` → audit one at a time. Tests asserting state sync only (counts, signal updates) move to Rust with MemNetwork; tests asserting DOM reflection of sync stay in Playwright.
- `multi-peer-mobile.spec.ts` → stay in Playwright (touch + viewport + multi-peer combined).
- `cross-browser-sync.spec.ts` → stay in Playwright.
- `join-links.spec.ts` → clipboard + URL parsing stays in Playwright.

### Phase C — Guideline + enforcement

- Add `CLAUDE.md` section "Which test tier to use" with the decision tree and a short rationale.
- Add `e2e/README.md` explaining what belongs in Playwright and the migration rewrite trigger.
- Update the PR template (`.github/pull_request_template.md` if it exists, else `docs/contributing.md`) with a checklist: "any new Playwright test justified against the decision tree?".
- Memory: save an entry under `feedback_` so future Claude sessions apply the guideline by default.

## Acceptance criteria

- [ ] `just check-all` exists and runs all four stages in order with fail-fast.
- [ ] Phase A shipped: full E2E wall-clock < 8 min on developer box with `fullyParallel: true` + `workers: 4`.
- [ ] Phase A shipped: `setup-e2e.sh` runs without sudo prompt on a fresh sandbox.
- [ ] Phase B shipped: `basic-flow.spec.ts` removed (or reduced to only the clipboard/browser tests that genuinely need Playwright); equivalent coverage at lower tier.
- [ ] Phase B shipped: Playwright suite wall-clock < 3 min (green path).
- [ ] Phase C shipped: `CLAUDE.md` + `e2e/README.md` document the tier rules.
- [ ] No test behaviour is lost in migration. Each removed Playwright assertion has a matching lower-tier assertion committed in the same PR.

## Risks + mitigations

- **Relay stampede under parallel Playwright.** Mitigated by `serial` mode on multi-peer specs (files still run in parallel, tests within one multi-peer file stay sequential).
- **wasm-pack harness doesn't yet mount both shells.** Mitigated by adding a `mount_test_with_shell` helper in Phase B Task 1 before migrations depend on it.
- **MemNetwork behaviour diverges from real iroh.** Mitigated by keeping one representative multi-peer test per sync category (SyncBatch, reconnect, cross-grove) in Playwright as a real-network canary; everything else covered at the Rust tier for speed.
- **Flaky migration causes coverage gap.** Mitigated by writing the new tier's test first, confirming it passes, and only then deleting the Playwright original.
- **Developers ignore the guideline.** Mitigated by PR checklist + memory note + by making the lower tier faster to iterate on than Playwright (faster feedback = natural incentive).

## Open questions

None blocking. Two deferred:
- Whether to add a Rust-integration layer that exercises real iroh (not MemNetwork) in a lightweight relay harness — would close the real-network gap without Playwright. Revisit after Phase B once the Playwright suite is small.
- Whether to replace Trunk serve with a pre-built static bundle for Playwright runs — removes WASM-compile time from the E2E startup budget. Revisit if the Phase A target isn't hit.
