// e2e/helpers/clock.ts
//
// Per-page Playwright clock helpers. Opt-in: tests that install the
// clock are explicit about which timers they advance. Default e2e tests
// run with real time so iroh background timers (gossip heartbeats,
// retry backoff) are unaffected.
//
// Per docs/specs/2026-04-27-event-based-waits-design.md §`page.clock`
// for real durations.

import type { Page } from '@playwright/test';

/**
 * Install the Playwright clock on `page`. Patches Date/setTimeout/
 * setInterval/requestAnimationFrame. After install, time only advances
 * via runFor / fastForward / pauseAt.
 *
 * Idempotent: calling twice on the same page is safe (Playwright no-ops
 * the second install).
 */
export async function installPageClock(page: Page): Promise<void> {
  await page.clock.install();
}

/** Advance the page's clock by `durationMs` synthetic milliseconds. */
export async function runForMs(page: Page, durationMs: number): Promise<void> {
  await page.clock.runFor(durationMs);
}
