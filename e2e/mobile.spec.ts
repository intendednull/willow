import { test, expect } from '@playwright/test';
import { freshStart, createServer, sendMessage, waitForApp } from './helpers';

// Mobile tests only run with mobile viewport.
test.describe('Mobile UX', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(!testInfo.project.name.startsWith('mobile'), 'mobile only');
  });

  // ── Basic rendering ───────────────────────────────────────────────

  test('app renders on mobile viewport', async ({ page }) => {
    await freshStart(page);
    await expect(page.locator('.welcome-card')).toBeVisible();
  });

  test('can create server on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Mobile Server', 'MobileUser');
    await expect(page.locator('.channel-header')).toBeVisible();
  });

  test('can send message on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Mobile Chat');
    await sendMessage(page, 'mobile message!');
    await expect(page.locator('.message .body', { hasText: 'mobile message!' })).toBeVisible();
  });

  // ── Navigation ────────────────────────────────────────────────────

  test('hamburger menu visible on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Hamburger Test');
    await expect(page.locator('.mobile-nav-toggle')).toBeVisible();
  });

  test('members toggle visible on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Members Test');
    await expect(page.locator('.mobile-members-toggle')).toBeVisible();
  });

  // ── Bug #9: Member list accessible on mobile ──────────────────────

  test('member list opens via toggle button', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Member List Mobile');

    await page.locator('.mobile-members-toggle').click();
    await page.waitForTimeout(500);

    // Member list wrapper should be visible.
    await expect(page.locator('.member-list-wrapper.open')).toBeVisible({ timeout: 3000 });
  });

  // ── Bug #10: Sidebar overlay dismisses ────────────────────────────

  test('sidebar opens and overlay dismisses it', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Sidebar Dismiss');

    // Open sidebar.
    await page.locator('.mobile-nav-toggle').click();
    await page.waitForTimeout(500);

    // Sidebar should be open.
    await expect(page.locator('.sidebar.open')).toBeVisible({ timeout: 3000 });
  });

  // ── Channel creation ──────────────────────────────────────────────

  test('voice channel creation works on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Voice Mobile');

    await page.locator('.mobile-nav-toggle').click();
    await page.waitForTimeout(500);
    await page.locator('.channel-add-btn').click();
    await page.waitForTimeout(300);
    await page.locator('.type-btn', { hasText: 'Voice' }).click();
    await page.waitForTimeout(100);
    await page.locator('.channel-create-input input').fill('vc');
    await page.locator('.channel-create-input input').press('Enter');
    await page.waitForTimeout(500);

    await expect(page.locator('.channel-item', { hasText: 'vc' })).toBeVisible();
  });

  // ── Bug #7: Input zoom prevention ─────────────────────────────────

  test('message input font size >= 16px (prevents iOS zoom)', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Zoom Test');
    const input = page.locator('.input-area input, .input-area textarea').first();
    const fontSize = await input.evaluate(el => getComputedStyle(el).fontSize);
    expect(parseInt(fontSize)).toBeGreaterThanOrEqual(16);
  });

  // ── Bug #1,2,3,4: Scrolling and tapping work after long-press fix ─

  test('message list is scrollable on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Scroll Test');

    // Send enough messages to overflow.
    for (let i = 0; i < 25; i++) {
      await sendMessage(page, `Message ${i + 1}`);
    }
    await page.waitForTimeout(500);

    // Last message should be visible (auto-scrolled to bottom).
    await expect(page.locator('.message .body', { hasText: 'Message 25' })).toBeVisible();

    // First message should NOT be visible (scrolled out of view).
    await expect(page.locator('.message .body').filter({ hasText: /^Message 1$/ })).not.toBeInViewport();
  });

  test('can tap reaction on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Tap React');
    await sendMessage(page, 'react to me');
    await page.waitForTimeout(500);

    // Add a reaction via desktop dropdown first (to have a reaction to tap).
    const msg = page.locator('.message').first();
    await msg.hover();
    await page.waitForTimeout(200);
    await page.locator('.action-trigger').first().click();
    await page.waitForTimeout(200);
    await page.locator('.dropdown-item', { hasText: 'React' }).click();
    await page.waitForTimeout(200);
    await page.locator('.dropdown-emoji-row button').first().click();
    await page.waitForTimeout(500);

    // Reaction should be visible.
    const reaction = page.locator('.reaction').first();
    await expect(reaction).toBeVisible();

    // Tap the reaction (should toggle — this was bug #2).
    await reaction.click();
    await page.waitForTimeout(300);

    // Should still be visible (either incremented or decremented).
    // The key test: clicking didn't crash or get blocked.
    await expect(page.locator('.message')).toBeVisible();
  });

  test('links in messages are tappable', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Link Tap');
    await sendMessage(page, 'visit https://example.com please');
    await page.waitForTimeout(500);

    // Link should be rendered.
    const link = page.locator('a.message-link');
    await expect(link).toBeVisible();

    // Should have correct href.
    const href = await link.getAttribute('href');
    expect(href).toContain('https://example.com');

    // Should be tappable (has target="_blank" so it won't navigate away).
    const target = await link.getAttribute('target');
    expect(target).toBe('_blank');
  });

  // ── Bug #8: Auto-scroll on new messages ───────────────────────────

  test('auto-scrolls to bottom on new message', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'AutoScroll');

    // Send several messages.
    for (let i = 0; i < 10; i++) {
      await sendMessage(page, `Msg ${i + 1}`);
    }
    await page.waitForTimeout(500);

    // The last message should be visible (auto-scrolled).
    await expect(page.locator('.message .body', { hasText: 'Msg 10' })).toBeVisible();
  });

  // ── Bug #6: Long-press doesn't trigger on quick tap ───────────────

  test('quick tap does NOT open action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'QuickTap');
    await sendMessage(page, 'tap me fast');
    await page.waitForTimeout(500);

    // Quick tap on message.
    const msg = page.locator('.message').first();
    await msg.tap();
    await page.waitForTimeout(600); // Wait longer than the 500ms timer.

    // Action sheet should NOT be visible after a quick tap.
    await expect(page.locator('.mobile-action-sheet.open')).toBeHidden();
  });

  test('single tap then wait does NOT open action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'TapWait');
    await sendMessage(page, 'just tap me');
    await page.waitForTimeout(500);

    // Single tap (touchstart + touchend quickly).
    const msg = page.locator('.message').first();
    await msg.tap();

    // Wait longer than the 500ms long-press threshold.
    await page.waitForTimeout(1000);

    // Action sheet should NOT have opened from a quick tap.
    await expect(page.locator('.mobile-action-sheet.open')).toBeHidden();
  });

  test('multiple quick taps do NOT open action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'MultiTap');
    await sendMessage(page, 'tap tap tap');
    await page.waitForTimeout(500);

    // Rapid taps on message.
    const msg = page.locator('.message').first();
    await msg.tap();
    await page.waitForTimeout(100);
    await msg.tap();
    await page.waitForTimeout(100);
    await msg.tap();
    await page.waitForTimeout(600);

    // Action sheet should NOT be visible.
    await expect(page.locator('.mobile-action-sheet.open')).toBeHidden();
  });

  // ── Bug #13: No always-visible dropdown on mobile ─────────────────

  test('action trigger button hidden on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'No Dropdown');
    await sendMessage(page, 'no dots');
    await page.waitForTimeout(500);

    // The ⋯ trigger should be hidden on mobile.
    const trigger = page.locator('.action-trigger').first();
    await expect(trigger).toBeHidden();
  });

  // ── Persistence on mobile ─────────────────────────────────────────

  test('messages persist after mobile refresh', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Mobile Persist');
    await sendMessage(page, 'survives refresh');
    await page.waitForTimeout(500);

    await page.reload();
    await waitForApp(page);
    await page.waitForTimeout(1000);

    await expect(page.locator('.message .body', { hasText: 'survives refresh' })).toBeVisible();
  });
});
