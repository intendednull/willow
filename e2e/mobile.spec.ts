/* eslint-disable no-restricted-syntax -- migration tracked at https://github.com/intendednull/willow/issues/458 */
import { test, expect } from '@playwright/test';
import {
  freshStart,
  createServer,
  sendMessage,
  reactToMessage,
} from './helpers';

// Mobile tests only run with mobile viewport. Most non-gesture mobile
// UX has migrated to wasm-pack (`crates/web/tests/browser.rs`
// `mod mobile_ux`). The tests that remain here need a real browser
// engine — real TouchEvent timing, auto-scroll layout, or link target
// attribute semantics that the headless wasm-pack harness can't model.
//
// Note: `createServer` on mobile already pushes into `general`, so tests
// below go straight from `sendMessage` after creation.
test.describe('Mobile UX', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(!testInfo.project.name.startsWith('mobile'), 'mobile only');
  });

  // ── Reaction tap on a real message row (touch hit-test) ───────────

  test('can tap reaction on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Tap React');
    await sendMessage(page, 'react to me');

    // Add a reaction (handles both desktop and mobile).
    await reactToMessage(page, 'react to me');

    // Reaction should be visible.
    const reaction = page.locator('.shell-mobile .reaction').first();
    await expect(reaction).toBeVisible();

    // Tap the reaction (should toggle — this was bug #2).
    await reaction.click();

    // Should still be visible (either incremented or decremented).
    // The key test: clicking didn't crash or get blocked.
    await expect(page.locator('.shell-mobile .message').first()).toBeVisible();
  });

  // ── Link semantics (target="_blank") ──────────────────────────────

  test('links in messages are tappable', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Link Tap');
    await sendMessage(page, 'visit https://example.com please');

    // Link should be rendered.
    const link = page.locator('.shell-mobile a.message-link').first();
    await expect(link).toBeVisible();

    // Should have correct href.
    const href = await link.getAttribute('href');
    expect(href).toContain('https://example.com');

    // Should be tappable (has target="_blank" so it won't navigate away).
    const target = await link.getAttribute('target');
    expect(target).toBe('_blank');
  });

  // ── Bug #8: Auto-scroll on new messages (needs real layout) ───────

  test('auto-scrolls to bottom on new message', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'AutoScroll');

    // Send several messages.
    for (let i = 0; i < 10; i++) {
      await sendMessage(page, `Msg ${i + 1}`);
    }

    // The last message should be visible (auto-scrolled).
    await expect(
      page.locator('.shell-mobile .message .body', { hasText: 'Msg 10' }).first(),
    ).toBeVisible();
  });

  // ── Bug #6: Long-press threshold uses real TouchEvent timing ──────
  // NOTE: "quick tap does NOT open action sheet" (via raw TouchEvent)
  // lives in mobile-actions.spec.ts. These two cover the
  // tap-then-wait variants that depend on real-touch timing.

  test('single tap then wait does NOT open action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'TapWait');
    await sendMessage(page, 'just tap me');

    // Single tap (touchstart + touchend quickly).
    const msg = page.locator('.shell-mobile .message').first();
    await msg.tap();

    // Wait longer than the 500ms long-press threshold — intentional sleep.
    await page.waitForTimeout(1000);

    // Action sheet should NOT have opened from a quick tap.
    await expect(page.locator('.shell-mobile .mobile-action-sheet.open')).toHaveCount(0);
  });

  test('multiple quick taps do NOT open action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'MultiTap');
    await sendMessage(page, 'tap tap tap');

    // Rapid taps on message — intentional spacing to exercise the
    // quick-tap gesture vs. 500ms long-press threshold.
    const msg = page.locator('.shell-mobile .message').first();
    await msg.tap();
    await page.waitForTimeout(100);
    await msg.tap();
    await page.waitForTimeout(100);
    await msg.tap();
    await page.waitForTimeout(600);

    // Action sheet should NOT be visible.
    await expect(page.locator('.shell-mobile .mobile-action-sheet.open')).toHaveCount(0);
  });
});
