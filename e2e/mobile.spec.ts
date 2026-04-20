import { test, expect } from '@playwright/test';
import {
  freshStart,
  createServer,
  sendMessage,
  waitForApp,
  reactToMessage,
  switchTab,
} from './helpers';

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
    // New mobile shell renders the top bar and tab bar after join.
    await expect(page.locator('.mobile-top-bar')).toBeVisible();
    await expect(page.locator('.mobile-tab-bar')).toBeVisible();
  });

  test('can send message on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Mobile Chat');
    // Mobile: tap the first channel to push into the chat view.
    await page.locator('.mobile-home .channel-item').first().click();
    await page.locator('.mobile-push--channel').waitFor({ timeout: 5_000 });
    await sendMessage(page, 'mobile message!');
    await expect(
      page.locator('.shell-mobile .message .body', { hasText: 'mobile message!' }).first(),
    ).toBeVisible();
  });

  // ── Tab bar ───────────────────────────────────────────────────────

  test('tab bar renders four primary tabs with aria-label="primary"', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'TabBar Test');

    const tabBar = page.locator('.mobile-tab-bar');
    await expect(tabBar).toBeVisible();
    await expect(tabBar).toHaveAttribute('aria-label', 'primary');

    const tabs = page.locator('.mobile-tab-bar .tab');
    await expect(tabs).toHaveCount(4);
  });

  test('tab bar hides on pushed screens (channel chat)', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'TabHide Test');

    // Primary route: tab bar visible.
    await expect(page.locator('.mobile-tab-bar')).toHaveAttribute('data-visible', 'true');

    // Tap a channel to push into chat — tab bar hides.
    await page.locator('.mobile-home .channel-item').first().click();
    await expect(page.locator('.mobile-tab-bar')).toHaveAttribute('data-visible', 'false');
  });

  test('tab bar returns on back', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'TabReturn');

    await page.locator('.mobile-home .channel-item').first().click();
    await expect(page.locator('.mobile-tab-bar')).toHaveAttribute('data-visible', 'false');

    // Tap the back chevron (top-slot-left on a pushed screen).
    await page.locator('.mobile-top-bar .top-slot-left').click();
    await expect(page.locator('.mobile-tab-bar')).toHaveAttribute('data-visible', 'true');
  });

  test('switchTab helper lands on letters empty state', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'LettersTab');
    await switchTab(page, 'letters');
    await expect(page.locator('.mobile-tab-empty')).toBeVisible();
  });

  // ── Grove drawer ──────────────────────────────────────────────────

  test('drawer opens when the top-bar grove glyph is tapped', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'DrawerOpen');

    await page.locator('.mobile-top-bar .top-slot-left').click();
    await expect(page.locator('.grove-drawer.open')).toBeVisible();
  });

  test('drawer closes on backdrop tap', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'DrawerClose');

    await page.locator('.mobile-top-bar .top-slot-left').click();
    await expect(page.locator('.grove-drawer.open')).toBeVisible();

    await page.locator('.grove-drawer-backdrop').dispatchEvent('click');
    await expect(page.locator('.grove-drawer.open')).toBeHidden();
  });

  // ── Channel creation ──────────────────────────────────────────────

  test('voice channel creation works on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Voice Mobile');

    // Channel creation lives in the inline sidebar on the home tab.
    await page.locator('.shell-mobile .channel-add-btn').first().click();
    await page.locator('.shell-mobile .type-btn', { hasText: 'Voice' }).first().click();
    const input = page.locator('.shell-mobile .channel-create-input input').first();
    await input.waitFor({ timeout: 5_000 });
    await input.fill('vc');
    await input.press('Enter');

    await expect(
      page.locator('.shell-mobile .channel-item', { hasText: 'vc' }).first(),
    ).toBeVisible();
  });

  // ── Bug #7: Input zoom prevention ─────────────────────────────────

  test('message input font size >= 16px (prevents iOS zoom)', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Zoom Test');
    // Push into a channel so the composer mounts.
    await page.locator('.mobile-home .channel-item').first().click();
    const input = page.locator('.shell-mobile .input-area input, .shell-mobile .input-area textarea').first();
    await input.waitFor({ timeout: 5_000 });
    const fontSize = await input.evaluate(el => getComputedStyle(el).fontSize);
    expect(parseInt(fontSize)).toBeGreaterThanOrEqual(16);
  });

  // ── Bug #1,2,3,4: Scrolling and tapping work after long-press fix ─

  test('message list is scrollable on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Scroll Test');

    // Push into a channel before sending messages.
    await page.locator('.mobile-home .channel-item').first().click();
    await page.locator('.mobile-push--channel').waitFor({ timeout: 5_000 });

    // Send enough messages to overflow.
    for (let i = 0; i < 25; i++) {
      await sendMessage(page, `Message ${i + 1}`);
    }

    // Last message should be visible (auto-scrolled to bottom).
    await expect(
      page.locator('.shell-mobile .message .body', { hasText: 'Message 25' }).first(),
    ).toBeVisible();

    // First message should NOT be visible (scrolled out of view).
    await expect(
      page.locator('.shell-mobile .message .body').filter({ hasText: /^Message 1$/ }).first(),
    ).not.toBeInViewport();
  });

  test('can tap reaction on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Tap React');
    await page.locator('.mobile-home .channel-item').first().click();
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

  test('links in messages are tappable', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Link Tap');
    await page.locator('.mobile-home .channel-item').first().click();
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

  // ── Bug #8: Auto-scroll on new messages ───────────────────────────

  test('auto-scrolls to bottom on new message', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'AutoScroll');

    await page.locator('.mobile-home .channel-item').first().click();
    await page.locator('.mobile-push--channel').waitFor({ timeout: 5_000 });

    // Send several messages.
    for (let i = 0; i < 10; i++) {
      await sendMessage(page, `Msg ${i + 1}`);
    }

    // The last message should be visible (auto-scrolled).
    await expect(
      page.locator('.shell-mobile .message .body', { hasText: 'Msg 10' }).first(),
    ).toBeVisible();
  });

  // ── Bug #6: Long-press doesn't trigger on quick tap ───────────────
  // NOTE: "quick tap does NOT open action sheet" is covered with stronger
  // raw-TouchEvent assertions in mobile-actions.spec.ts. The test below
  // covers the complementary "tap-then-wait" variant.

  test('single tap then wait does NOT open action sheet', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'TapWait');
    await page.locator('.mobile-home .channel-item').first().click();
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
    await page.locator('.mobile-home .channel-item').first().click();
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

  // ── Persistence on mobile ─────────────────────────────────────────

  test('messages persist after mobile refresh', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Mobile Persist');
    await page.locator('.mobile-home .channel-item').first().click();
    await sendMessage(page, 'survives refresh');

    await page.reload();
    await waitForApp(page);

    // After reload, navigate back into the channel.
    await page.locator('.mobile-home .channel-item').first().click();

    await expect(
      page.locator('.shell-mobile .message .body', { hasText: 'survives refresh' }).first(),
    ).toBeVisible({ timeout: 10_000 });
  });
});
