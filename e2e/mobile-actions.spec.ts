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
