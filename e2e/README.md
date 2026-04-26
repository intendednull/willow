# E2E tests (Playwright)

This directory is reserved for tests that *need* Playwright. Everything
else belongs at a lower tier — see `CLAUDE.md` §Which test tier to use
or `docs/specs/2026-04-21-e2e-test-architecture-design.md`.

## What belongs here

- Multi-peer real-network P2P flows (real iroh + relay gossip, real SyncBatch).
- Cross-browser compatibility (Firefox-specific quirks, Safari if added).
- Touch gestures: swipe, long-press, pull-down.
- Viewport-driven responsive breakpoints when the media query itself is under test.
- Browser integration paths: service worker, push, clipboard, browser navigation.

## What does NOT belong here

- Single-client DOM flows — put them in `crates/web/tests/browser.rs`.
- Client API + state assertions — put them in `crates/client/src/tests/`.
- State-machine logic — put them in `crates/state/src/tests.rs`.
- CSS class probes — `crates/web/tests/browser.rs` can inspect `document.styleSheets`.

## Rewrite trigger

A Playwright test that fails because of selector drift (not behaviour
change) is a signal the test is at the wrong tier. Migrate it down on
the same commit rather than fixing the selector.

## Running

- `just test-e2e-ui` — desktop-chrome + mobile-chrome, requires `just dev`.
- `just test-e2e-full` — full setup + teardown + run, good for CI.
- `PLAYWRIGHT_WORKERS=N npx playwright test ...` — override worker count.
- `PLAYWRIGHT_FULLY_PARALLEL=0 npx playwright test ...` — disable intra-file parallelism.
