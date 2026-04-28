/* eslint-disable no-restricted-syntax -- migration tracked at https://github.com/intendednull/willow/issues/458 */
import { test, expect } from '@playwright/test';
import {
  freshStart,
  createServer,
  sendMessage,
  longPress,
  swipeLeft,
  visibleShell,
} from './helpers';

// Mobile action-sheet behaviour. Non-gesture sheet behaviour (cancel,
// overlay tap, reply, react, three-dot hidden, quick-tap no-op) has
// migrated to wasm-pack (`crates/web/tests/browser.rs` `mod
// mobile_actions`). The tests that remain here depend on a real
// browser's TouchEvent timing model (500 ms long-press threshold, swipe
// velocity thresholds) — the headless wasm-pack harness can't model
// those reliably.
test.describe('Mobile action sheet', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(!testInfo.project.name.startsWith('mobile'), 'mobile only');
  });

  test('long-press opens action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'LongPress Open');
    await sendMessage(page, 'press and hold');

    await longPress(page, '.message');

    await expect(page.locator('.shell-mobile .mobile-action-sheet.open').first()).toBeVisible({ timeout: 3000 });
  });

  test('action sheet renders spec copy verbatim', async ({ page }) => {
    // Spec §Long-press action sheet (message-row.md): items render
    // all-lowercase in the order `reply`, `reply in thread`,
    // `add reaction`, `pin`/`unpin` (permission-gated), `copy text`,
    // `edit` (own), `delete`, trailing `cancel`. This test opens the
    // sheet on an own message so the superset (minus pin, which is
    // permission-gated off for a solo peer) is visible.
    await freshStart(page);
    await createServer(page, 'SheetCopy', 'Alice');
    await sendMessage(page, 'spec copy');

    await longPress(page, '.message');
    const sheet = page.locator('.shell-mobile .mobile-action-sheet.open').first();
    await expect(sheet).toBeVisible({ timeout: 3000 });

    // Each of these strings must match exactly — no title-case drift.
    for (const label of ['reply', 'reply in thread', 'add reaction', 'copy text', 'edit', 'delete', 'cancel']) {
      await expect(sheet.locator('.sheet-item', { hasText: new RegExp(`^\\s*${label}\\s*$`) }))
        .toBeVisible({ timeout: 3_000 });
    }
  });

  test('fast downward swipe dismisses by velocity', async ({ page }) => {
    // Spec §Long-press action sheet: dismiss on drag ≥ 80 px OR
    // release velocity > 200 px/s. This test fires a short (60 px)
    // fast swipe spread across ~50 ms of real wall-time — distance
    // alone wouldn't trip the 80 px threshold, so the dismiss must
    // come from the velocity branch. The `touchend` is fired in a
    // separate `evaluate` so the elapsed Date.now() delta is non-
    // trivial (same trick Playwright uses for its swipe timings).
    await freshStart(page);
    await createServer(page, 'FastSwipe');
    await sendMessage(page, 'velocity dismiss');

    await longPress(page, '.message');
    const sheetSel = '.shell-mobile .mobile-action-sheet.open';
    await expect(page.locator(sheetSel).first()).toBeVisible({ timeout: 3_000 });

    const sheet = page.locator(sheetSel).first();
    const box = await sheet.boundingBox();
    if (!box) throw new Error('sheet not found');

    const startX = box.x + box.width / 2;
    const startY = box.y + 20;
    const endY = startY + 60; // below the 80 px distance threshold

    // touchstart + first touchmove → record start time.
    await page.evaluate(({ startX, startY }) => {
      const target = document.elementFromPoint(startX, startY);
      if (!target) return;
      const makeTouch = (y: number) => new Touch({
        identifier: 1, target, clientX: startX, clientY: y, pageX: startX, pageY: y,
      });
      target.dispatchEvent(new TouchEvent('touchstart', {
        bubbles: true, cancelable: true,
        touches: [makeTouch(startY)], targetTouches: [makeTouch(startY)], changedTouches: [makeTouch(startY)],
      }));
    }, { startX, startY });

    // Sleep ~60 ms so elapsed time at touchend is non-zero. 60 px /
    // 60 ms ≈ 1000 px/s, comfortably past the 200 px/s threshold.
    await page.waitForTimeout(60);

    await page.evaluate(({ startX, startY, endY }) => {
      const target = document.elementFromPoint(startX, startY);
      if (!target) return;
      const makeTouch = (y: number) => new Touch({
        identifier: 1, target, clientX: startX, clientY: y, pageX: startX, pageY: y,
      });
      target.dispatchEvent(new TouchEvent('touchmove', {
        bubbles: true, cancelable: true,
        touches: [makeTouch(endY)], targetTouches: [makeTouch(endY)], changedTouches: [makeTouch(endY)],
      }));
      target.dispatchEvent(new TouchEvent('touchend', {
        bubbles: true, cancelable: true,
        touches: [], targetTouches: [], changedTouches: [makeTouch(endY)],
      }));
    }, { startX, startY, endY });

    await expect(page.locator(sheetSel).first()).toBeHidden();
  });

  test('swipe down dismisses action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'SwipeDown');
    await sendMessage(page, 'swipe to dismiss');

    await longPress(page, '.message');
    await expect(page.locator('.shell-mobile .mobile-action-sheet.open').first()).toBeVisible({ timeout: 3000 });

    // Simulate a downward swipe on the action sheet.
    const sheet = page.locator('.shell-mobile .mobile-action-sheet.open').first();
    const box = await sheet.boundingBox();
    if (!box) throw new Error('sheet not found');

    const startX = box.x + box.width / 2;
    const startY = box.y + 20; // Near the drag handle at top
    const endY = startY + 150; // Swipe 150px down (past 80px threshold)

    await page.evaluate(({ startX, startY, endY }) => {
      const target = document.elementFromPoint(startX, startY);
      if (!target) return;
      const makeTouch = (y: number) => new Touch({
        identifier: 1, target, clientX: startX, clientY: y, pageX: startX, pageY: y,
      });
      target.dispatchEvent(new TouchEvent('touchstart', {
        bubbles: true, cancelable: true,
        touches: [makeTouch(startY)], targetTouches: [makeTouch(startY)], changedTouches: [makeTouch(startY)],
      }));
      target.dispatchEvent(new TouchEvent('touchmove', {
        bubbles: true, cancelable: true,
        touches: [makeTouch(endY)], targetTouches: [makeTouch(endY)], changedTouches: [makeTouch(endY)],
      }));
      target.dispatchEvent(new TouchEvent('touchend', {
        bubbles: true, cancelable: true,
        touches: [], targetTouches: [], changedTouches: [makeTouch(endY)],
      }));
    }, { startX, startY, endY });

    await expect(page.locator('.shell-mobile .mobile-action-sheet.open').first()).toBeHidden();
  });

  test('swipe-left on message populates composer replying_to', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'SwipeLeft', 'Alice');
    await sendMessage(page, 'hello swipe-left');

    // Target the visible-shell's first message row to avoid hitting the
    // hidden copy in the inactive desktop shell.
    const row = page
      .locator(`${visibleShell(page)} .message`, { hasText: 'hello swipe-left' })
      .first();
    await row.waitFor({ timeout: 5_000 });
    await swipeLeft(page, row);

    // Reply-preview bar (see `crates/web/src/components/input.rs`) is
    // the source-of-truth for "composer is replying_to Some(..)".
    await expect(page.locator(`${visibleShell(page)} .reply-bar`))
      .toBeVisible({ timeout: 3_000 });
  });
});
