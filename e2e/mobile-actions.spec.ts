import { test, expect } from '@playwright/test';
import { freshStart, createServer, sendMessage, longPress } from './helpers';

test.describe('Mobile action sheet', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(!testInfo.project.name.startsWith('mobile'), 'mobile only');
  });

  test('long-press opens action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'LongPress Open');
    await sendMessage(page, 'press and hold');
    await page.waitForTimeout(500);

    await longPress(page, '.message');

    await expect(page.locator('.mobile-action-sheet.open')).toBeVisible({ timeout: 3000 });
  });

  test('action sheet stays open over time', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'StayOpen');
    await sendMessage(page, 'stay open');
    // Wait for any initial sync/refresh to settle.
    await page.waitForTimeout(2000);

    await longPress(page, '.message');
    await expect(page.locator('.mobile-action-sheet.open')).toBeVisible({ timeout: 3000 });

    // Wait 2 seconds — sheet should still be open.
    await page.waitForTimeout(2000);
    await expect(page.locator('.mobile-action-sheet.open')).toBeVisible();
  });

  test('cancel closes action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'CancelSheet');
    await sendMessage(page, 'cancel me');
    await page.waitForTimeout(500);

    await longPress(page, '.message');
    await expect(page.locator('.mobile-action-sheet.open')).toBeVisible({ timeout: 3000 });

    await page.locator('.sheet-cancel').click();
    await page.waitForTimeout(300);

    await expect(page.locator('.mobile-action-sheet.open')).toBeHidden();
  });

  test('overlay tap closes action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'OverlayClose');
    await sendMessage(page, 'overlay close');
    await page.waitForTimeout(2000);

    await longPress(page, '.message');
    await expect(page.locator('.mobile-action-sheet.open')).toBeVisible({ timeout: 3000 });

    // Click the overlay area (top of screen, away from sheet).
    await page.mouse.click(200, 100);
    await page.waitForTimeout(500);

    await expect(page.locator('.mobile-action-sheet.open')).toBeHidden();
  });

  test('reply from sheet shows reply bar', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'SheetReply');
    await sendMessage(page, 'reply to this');
    await page.waitForTimeout(500);

    await longPress(page, '.message');
    await expect(page.locator('.mobile-action-sheet.open')).toBeVisible({ timeout: 3000 });

    await page.locator('.sheet-item', { hasText: 'Reply' }).click();
    await page.waitForTimeout(500);

    await expect(page.locator('.reply-bar')).toBeVisible();
  });

  test('react from sheet adds reaction', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'SheetReact');
    await sendMessage(page, 'react from sheet');
    await page.waitForTimeout(500);

    await longPress(page, '.message');
    await expect(page.locator('.mobile-action-sheet.open')).toBeVisible({ timeout: 3000 });

    await page.locator('.sheet-emoji-row button').first().click();
    await page.waitForTimeout(500);

    await expect(page.locator('.reaction')).toBeVisible();
  });

  test('swipe down dismisses action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'SwipeDown');
    await sendMessage(page, 'swipe to dismiss');
    await page.waitForTimeout(500);

    await longPress(page, '.message');
    await expect(page.locator('.mobile-action-sheet.open')).toBeVisible({ timeout: 3000 });

    // Simulate a downward swipe on the action sheet.
    const sheet = page.locator('.mobile-action-sheet.open');
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

    await page.waitForTimeout(500);
    await expect(page.locator('.mobile-action-sheet.open')).toBeHidden();
  });

  test('action trigger (three-dot menu) is hidden on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'NoTrigger');
    await sendMessage(page, 'no dots');
    await page.waitForTimeout(500);

    // Hover the message (simulated) — the .message-actions should stay hidden on mobile.
    const msg = page.locator('.message').first();
    await msg.hover();
    await page.waitForTimeout(300);

    await expect(page.locator('.action-trigger')).toBeHidden();
    await expect(page.locator('.message-actions')).toBeHidden();
  });

  test('quick tap does NOT open sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'QuickTap2');
    await sendMessage(page, 'quick tap');
    await page.waitForTimeout(500);

    // Quick tap via evaluate (touchstart + immediate touchend).
    const msg = page.locator('.message').first();
    const box = await msg.boundingBox();
    if (!box) throw new Error('no msg');
    const x = box.x + box.width / 2;
    const y = box.y + box.height / 2;

    await page.evaluate(({ x, y }) => {
      const target = document.elementFromPoint(x, y);
      if (!target) return;
      const touch = new Touch({ identifier: 1, target, clientX: x, clientY: y, pageX: x, pageY: y });
      target.dispatchEvent(new TouchEvent('touchstart', { bubbles: true, cancelable: true, touches: [touch], targetTouches: [touch], changedTouches: [touch] }));
      // Immediate touchend.
      target.dispatchEvent(new TouchEvent('touchend', { bubbles: true, cancelable: true, touches: [], targetTouches: [], changedTouches: [touch] }));
    }, { x, y });

    await page.waitForTimeout(700);
    await expect(page.locator('.mobile-action-sheet.open')).toBeHidden();
  });
});
