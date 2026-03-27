import { test, expect, chromium, firefox, devices } from '@playwright/test';
import { freshStart, createServer, sendMessage, waitForMessage, waitForApp, getPeerId, openSidebar } from './helpers';

/**
 * Cross-browser sync tests.
 *
 * These tests launch DIFFERENT browser types (e.g., mobile Chrome + desktop Firefox)
 * to replicate real-world cross-browser P2P connectivity.
 * They do NOT use the Playwright project's browser fixture — they launch browsers directly.
 */
test.describe('Cross-browser peer sync', () => {
  // These tests are slow — they launch two separate browser engines.
  test.setTimeout(120_000);

  // Only run from one project to avoid duplicating (each test launches its own browsers).
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name !== 'desktop-chrome', 'cross-browser tests run once from desktop-chrome');
  });

  test('mobile Chrome to desktop Firefox — invite + messaging', async () => {
    // Launch mobile Chrome (Pixel 7 viewport).
    const mobileBrowser = await chromium.launch();
    const mobileCtx = await mobileBrowser.newContext({
      ...devices['Pixel 7'],
    });
    const mobilePage = await mobileCtx.newPage();

    // Launch desktop Firefox.
    const desktopBrowser = await firefox.launch();
    const desktopCtx = await desktopBrowser.newContext({
      ...devices['Desktop Firefox'],
    });
    const desktopPage = await desktopCtx.newPage();

    try {
      // Desktop Firefox: create server.
      await freshStart(desktopPage);
      await createServer(desktopPage, 'CrossBrowser Test', 'DesktopUser');

      // Mobile Chrome: get peer ID from welcome screen.
      await freshStart(mobilePage);
      const mobilePeerId = await getPeerId(mobilePage);
      expect(mobilePeerId).toBeTruthy();

      // Desktop Firefox: generate invite for mobile peer.
      await desktopPage.locator('.server-gear-btn').click();
      await desktopPage.waitForTimeout(500);
      await desktopPage.locator('input[placeholder*="12D3KooW"]').fill(mobilePeerId);
      await desktopPage.locator('button', { hasText: 'Generate Invite' }).click();
      await desktopPage.waitForTimeout(500);
      const inviteCode = await desktopPage.locator('.invite-code-display textarea').inputValue();
      expect(inviteCode).toBeTruthy();

      // Desktop Firefox: go back to chat.
      await desktopPage.locator('text=Back').click();
      await desktopPage.waitForTimeout(500);

      // Mobile Chrome: join via invite.
      await mobilePage.locator('.welcome-invite-input').fill(inviteCode);
      await mobilePage.locator('button', { hasText: 'Next' }).click();
      await mobilePage.waitForTimeout(500);
      await mobilePage.locator('button', { hasText: 'Join Server' }).click();
      await mobilePage.waitForSelector('.sidebar, .app', { timeout: 15_000 });
      await mobilePage.waitForTimeout(5000); // Wait for P2P sync.

      // Verify mobile sees the server (sidebar should have "general" channel).
      await openSidebar(mobilePage);
      await expect(mobilePage.locator('.channel-item', { hasText: 'general' }))
        .toBeVisible({ timeout: 15_000 });

      // Desktop Firefox: send a message.
      await sendMessage(desktopPage, 'Hello from Firefox desktop');

      // Mobile Chrome: should see the message.
      await waitForMessage(mobilePage, 'Hello from Firefox desktop', 30_000);

      // Mobile Chrome: send a reply.
      await sendMessage(mobilePage, 'Hello from Chrome mobile');

      // Desktop Firefox: should see the reply.
      await waitForMessage(desktopPage, 'Hello from Chrome mobile', 30_000);

    } finally {
      await mobileCtx.close();
      await mobileBrowser.close();
      await desktopCtx.close();
      await desktopBrowser.close();
    }
  });

  test('desktop Firefox to mobile Chrome — invite + channel sync', async () => {
    const mobileBrowser = await chromium.launch();
    const mobileCtx = await mobileBrowser.newContext({
      ...devices['Pixel 7'],
    });
    const mobilePage = await mobileCtx.newPage();

    const desktopBrowser = await firefox.launch();
    const desktopCtx = await desktopBrowser.newContext({
      ...devices['Desktop Firefox'],
    });
    const desktopPage = await desktopCtx.newPage();

    try {
      // Mobile Chrome creates the server this time.
      await freshStart(mobilePage);
      await createServer(mobilePage, 'Mobile Server', 'MobileUser');

      // Desktop Firefox gets peer ID.
      await freshStart(desktopPage);
      const desktopPeerId = await getPeerId(desktopPage);
      expect(desktopPeerId).toBeTruthy();

      // Mobile Chrome: open settings to generate invite.
      await openSidebar(mobilePage);
      await mobilePage.locator('.server-gear-btn').click();
      await mobilePage.waitForTimeout(500);
      await mobilePage.locator('input[placeholder*="12D3KooW"]').fill(desktopPeerId);
      await mobilePage.locator('button', { hasText: 'Generate Invite' }).click();
      await mobilePage.waitForTimeout(500);
      const inviteCode = await mobilePage.locator('.invite-code-display textarea').inputValue();
      expect(inviteCode).toBeTruthy();

      // Mobile Chrome: go back.
      await mobilePage.locator('text=Back').click();
      await mobilePage.waitForTimeout(500);

      // Desktop Firefox: join via invite.
      await desktopPage.locator('.welcome-invite-input').fill(inviteCode);
      await desktopPage.locator('button', { hasText: 'Next' }).click();
      await desktopPage.waitForTimeout(500);
      await desktopPage.locator('button', { hasText: 'Join Server' }).click();
      await desktopPage.waitForSelector('.sidebar', { timeout: 15_000 });
      await desktopPage.waitForTimeout(5000);

      // Desktop should see "general" channel.
      await expect(desktopPage.locator('.channel-item', { hasText: 'general' }))
        .toBeVisible({ timeout: 15_000 });

      // Mobile sends a message.
      await sendMessage(mobilePage, 'Cross browser works!');

      // Desktop should see it.
      await waitForMessage(desktopPage, 'Cross browser works!', 30_000);

    } finally {
      await mobileCtx.close();
      await mobileBrowser.close();
      await desktopCtx.close();
      await desktopBrowser.close();
    }
  });
});
