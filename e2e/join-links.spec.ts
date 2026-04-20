import { test, expect } from '@playwright/test';
import { freshStart, createServer, sendMessage, waitForMessage, waitForApp, openSidebar } from './helpers';

test.describe('Join via shareable link', () => {
  // Skip on Firefox — clipboard permissions are not supported.
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name.includes('firefox'), 'clipboard permissions not supported in Firefox');
  });

  test('peer joins via link URL and sees messages', async ({ browser, baseURL }) => {
    const ctxA = await browser.newContext({
      permissions: ['clipboard-read', 'clipboard-write'],
    });
    const pageA = await ctxA.newPage();
    await freshStart(pageA);
    await createServer(pageA, 'Link Test', 'Alice');

    // Generate join link from settings.
    await openSidebar(pageA);
    await pageA.locator('.server-gear-btn').click();
    await pageA.waitForTimeout(500);
    await pageA.locator('button', { hasText: 'Create Invite Link' }).click();
    await pageA.waitForTimeout(500);

    // The link was copied to clipboard — read it.
    const clipboardUrl = await pageA.evaluate(() => navigator.clipboard.readText());
    expect(clipboardUrl).toContain('#join=');

    // Extract the hash fragment and construct URL using test baseURL
    // (the app may generate a URL with a different origin).
    const hashFragment = clipboardUrl.substring(clipboardUrl.indexOf('#'));
    const joinUrl = `${baseURL}/${hashFragment}`;

    // Go back to chat.
    await pageA.locator('text=Back').click();

    // Peer B opens the join link URL directly (full page load with hash).
    const ctxB = await browser.newContext();
    const pageB = await ctxB.newPage();
    await pageB.goto(joinUrl);
    await waitForApp(pageB);

    // Should see the JoinPage with server name.
    await expect(pageB.locator('.join-card-server')).toContainText('Link Test', { timeout: 10_000 });
    await expect(pageB.locator('.join-card-inviter')).toContainText('Alice');

    // Enter name and click Join.
    await pageB.locator('.join-card-field input').fill('Bob');
    await pageB.locator('.join-card-btn').click();

    // Wait for join to complete (join page disappears, chat appears).
    await pageB.waitForSelector('.sidebar, .app', { timeout: 30000 });

    // Verify B sees the server — wait for DOM attachment first (gossip may lag).
    await expect(pageB.locator('.channel-item', { hasText: 'general' }))
      .toBeAttached({ timeout: 30_000 });
    await openSidebar(pageB);
    await expect(pageB.locator('.channel-item', { hasText: 'general' }))
      .toBeVisible({ timeout: 5_000 });

    // A sends a message.
    await sendMessage(pageA, 'Welcome Bob!');

    // B should see it.
    await waitForMessage(pageB, 'Welcome Bob!', 30000);

    await ctxA.close();
    await ctxB.close();
  });

  test('joining with invalid code does not reach the chat', async ({ browser }) => {
    // Entering a garbage invite code and attempting to join should not navigate
    // to a server. The app must fail gracefully — no channel list, no chat area.
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    try {
      await freshStart(page);
      // Walk past the name step and switch to the Join tab.
      await page.locator('.welcome-continue-btn').click();
      await page.locator('.welcome-tab-btn', { hasText: 'Join' }).click();
      await page.locator('.welcome-invite-input').fill('this-is-definitely-not-a-valid-invite-code');
      await page.locator('button', { hasText: 'Open letter' }).click();
      // Current behaviour: the confirmation step appears for any non-empty
      // input — server lookup is deferred to the actual join click.
      await page.locator('button', { hasText: 'Join grove' }).waitFor({ timeout: 3_000 });
      await page.locator('button', { hasText: 'Join grove' }).click();
      // The join should fail — no channel list should ever appear.
      await expect(page.locator('.channel-item')).toBeHidden({ timeout: 10_000 });
    } finally {
      await ctx.close();
    }
  });
});
