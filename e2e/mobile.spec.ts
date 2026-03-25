import { test, expect } from '@playwright/test';
import { freshStart, createServer, sendMessage, waitForApp } from './helpers';

// Mobile tests only run with mobile viewport.
test.describe('Mobile UX', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(!testInfo.project.name.includes('mobile'), 'mobile only');
  });
  test('app renders on mobile viewport', async ({ page }) => {
    await freshStart(page);
    await expect(page.locator('.welcome-card')).toBeVisible();
  });

  test('can create server on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Mobile Server', 'MobileUser');
    // Should see the channel header (main content area).
    await expect(page.locator('.channel-header')).toBeVisible();
  });

  test('can send message on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Mobile Chat');
    await sendMessage(page, 'mobile message!');
    await expect(page.locator('.message .body', { hasText: 'mobile message!' })).toBeVisible();
  });

  test('hamburger menu visible on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Hamburger Test');
    // The hamburger should be visible on mobile viewport.
    await expect(page.locator('.mobile-nav-toggle')).toBeVisible();
  });

  test('members toggle visible on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Members Test');
    await expect(page.locator('.mobile-members-toggle')).toBeVisible();
  });

  test('voice channel creation works on mobile', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Voice Mobile');

    // Open sidebar via hamburger.
    await page.locator('.mobile-nav-toggle').click();
    await page.waitForTimeout(500);

    // Click + to create channel.
    await page.locator('.channel-add-btn').click();
    await page.waitForTimeout(300);

    // Click Voice toggle.
    await page.locator('.type-btn', { hasText: 'Voice' }).click();
    await page.waitForTimeout(100);

    // Type and submit.
    await page.locator('.channel-create-input input').fill('vc');
    await page.locator('.channel-create-input input').press('Enter');
    await page.waitForTimeout(500);

    // Should see voice channel.
    await expect(page.locator('.channel-item', { hasText: 'vc' })).toBeVisible();
  });

  test('message input prevents iOS zoom (16px font)', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Zoom Test');
    const input = page.locator('.input-area input, .input-area textarea').first();
    const fontSize = await input.evaluate(el => getComputedStyle(el).fontSize);
    const size = parseInt(fontSize);
    expect(size).toBeGreaterThanOrEqual(16);
  });
});
