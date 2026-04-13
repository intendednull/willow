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
    await pageB.waitForTimeout(5000);

    // Verify B sees the server.
    await openSidebar(pageB);
    await expect(pageB.locator('.channel-item', { hasText: 'general' }))
      .toBeVisible({ timeout: 20000 });

    // A sends a message.
    await sendMessage(pageA, 'Welcome Bob!');

    // B should see it.
    await waitForMessage(pageB, 'Welcome Bob!', 30000);

    await ctxA.close();
    await ctxB.close();
  });
});
