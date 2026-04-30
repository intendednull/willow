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

## Helpers layout

The legacy 703-LOC `helpers.ts` has been split into focused modules. New
specs should import directly from the focused module they need.

```
e2e/
├── helpers/
│   ├── peers.ts    -- freshStart, createServer, getPeerId, generateInvite,
│   │                  joinViaInvite, setupTwoPeers, openServerSettings, waitForApp
│   ├── ui.ts       -- visibleShell, isMobile, sendMessage, waitForMessage,
│   │                  switchChannel, openSidebar, openMemberList, createChannel,
│   │                  messageAction, editMessage, deleteMessage, reactToMessage,
│   │                  trustPeer, untrustPeer, kickPeer, openCompareFingerprints, …
│   └── touch.ts    -- longPress, longPressAvatar, swipeLeft, swipeRight
├── helpers.ts      -- re-export barrel; un-migrated specs continue to import
│                      from './helpers' with zero diff
├── test-hooks.ts   -- Peer wrapper + `peer` fixture (see "Event-based waits" below)
└── *.spec.ts
```

## Event-based waits (Peer wrapper)

The web crate exposes `window.__willow` and a `__willowEvent` push stream
when built with `--features test-hooks`. The `Peer` class in
`e2e/test-hooks.ts` wraps both:

- **Pull**: `peer.snapshot()`, `peer.heads()`, `peer.eventCount()`,
  `peer.lastEvent()` — each round-trips through `window.__willow.*`.
- **Push**: `peer.nextEvent(predicate, { timeout? })` — drains the next
  event matching `predicate` from the per-page event queue.
- **Convergence**: `peer.waitUntilHeadsEqual(otherPeer)` and
  `peer.waitUntilAllHeadsEqual([otherPeers])` — `expect.poll`-based
  CRDT convergence checks. Failure messages include a structured
  per-author-key diff so missing-author hangs are debuggable without a
  manual `console.log`.

Specs that need the wrapper import the typed `test` + `expect` from
`./test-hooks` instead of `@playwright/test`:

```ts
import { test, expect } from './test-hooks';

test('peer B converges with peer A', async ({ peer, browser }) => {
  const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
  const a = await peer(page1, 'Alice');
  const b = await peer(page2, 'Bob');
  await b.waitUntilHeadsEqual(a);   // gossip-side wait
  await expect(page2.locator('.channel-item', { hasText: 'general' }))
    .toBeVisible();                 // default 5s — DOM-only after convergence
});
```

`peer(page, label)` is async and idempotently wires `__willowEvent`
bindings on the page's `BrowserContext` on first call per context, so
contexts created via `browser.newContext()` or `setupTwoPeers(browser)`
work without per-spec setup. Call `peer()` before the page's first
`goto()` when possible — `addInitScript` only takes effect on
subsequent loads.

The full design is in
[`docs/specs/2026-04-27-event-based-waits-design.md`](../docs/specs/2026-04-27-event-based-waits-design.md).
Migration progress for the remaining 7 specs is tracked in
[#458](https://github.com/intendednull/willow/issues/458).

## Anti-patterns blocked by ESLint

`page.waitForTimeout(ms)` is blocked by `no-restricted-syntax` in
`eslint.config.js`. Specs migrated off the timeout pattern remove their
file-top `eslint-disable` header in the same PR. Each remaining
disabled file references issue #458; the rule sunsets on 2026-09-30
(per spec §"Sunset").

## `data-state` lifecycle on animated components

Five animated components (mobile drawer, grove drawer, confirm dialog,
bottom sheet, mobile action sheet) and the tab bar's active-tab indicator
expose a `data-state` attribute reflecting one of four phases:
`closed | opening | open | closing`. Tests gate on the attribute
rather than sleeping after the click that opens the component.

```ts
await openSidebarBtn.click();
await expect(drawer).toHaveAttribute('data-state', 'open');
```

The lifecycle is driven by `transitionend` on the component's specific
CSS property (transform for slides, opacity for fades). A reduced-motion
shortcut snaps to the terminal phase synchronously when computed
transition-duration is 0s, so tests under `prefers-reduced-motion: reduce`
don't hang.

**Categorical `data-state` (separate convention.)** `status_dot.rs`,
`grove_rail.rs`, and `peer_status_label.rs` use `data-state` for
orthogonal categorical states (`online`/`offline`, `idle`/`loading`).
The four-phase lifecycle does NOT apply to them. Tests that gate on
`data-state` must know which component they target.

## `page.clock` for real-duration waits

`longPressWithClock(page, selector, ms)` and `installPageClock(page)`
in `e2e/helpers/clock.ts` use Playwright's per-page clock to advance
synthetic time without paying real wall-clock seconds. Use this for
touch gestures and debounce timers.

**Multi-peer caveat.** The clock is per-page. iroh's WASM transport may
use real-time timers (gossip heartbeats, retry backoff); installing
the clock during a multi-peer test could freeze UI/HLC time while iroh
keeps running. Default e2e tests stay clock-free; only single-peer
touch specs opt in.
