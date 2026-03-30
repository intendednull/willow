import { test, expect } from '@playwright/test';
import { freshStart, createServer, sendMessage, getMessages, waitForApp, waitForMessage, openSidebar, reactToMessage } from './helpers';

test.describe('Basic app flow', () => {
  test('welcome screen shows on fresh start', async ({ page }) => {
    await freshStart(page);
    await expect(page.locator('.welcome-card')).toBeVisible();
    await expect(page.locator('h1')).toContainText('Welcome to Willow');
  });

  test('can create a server from welcome screen', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Test Server', 'Alice');

    // Should now see the sidebar with server name.
    await expect(page.locator('.sidebar-header')).toContainText('Test Server');

    // Should have a general channel.
    await expect(page.locator('.channel-item')).toContainText('general');
  });

  test('can send and see own message', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Chat Test', 'Alice');

    await sendMessage(page, 'Hello world!');

    const msgs = await getMessages(page);
    expect(msgs).toContain('Hello world!');
  });

  test('can create a new text channel', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Channel Test');

    // Open sidebar on mobile (no-op on desktop).
    await openSidebar(page);

    // Click the + button.
    await page.locator('.channel-add-btn').click();
    await page.waitForTimeout(200);

    // Type channel name and press Enter.
    const input = page.locator('.channel-create-input input');
    await input.fill('random');
    await input.press('Enter');
    await page.waitForTimeout(500);

    // Should see the new channel.
    await expect(page.locator('.channel-item', { hasText: 'random' })).toBeVisible();
  });

  test('can create a voice channel', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Voice Test');

    // Open sidebar on mobile (no-op on desktop).
    await openSidebar(page);

    // Click +.
    await page.locator('.channel-add-btn').click();
    await page.waitForTimeout(200);

    // Click Voice toggle.
    await page.locator('.type-btn', { hasText: 'Voice' }).click();
    await page.waitForTimeout(100);

    // Type name and submit.
    const input = page.locator('.channel-create-input input');
    await input.fill('voice-chat');
    await input.press('Enter');
    await page.waitForTimeout(500);

    // Should see the voice channel with speaker icon.
    const voiceChannel = page.locator('.channel-item', { hasText: 'voice-chat' });
    await expect(voiceChannel).toBeVisible();
    // Voice channels show a volume SVG icon prefix.
    await expect(voiceChannel.locator('.icon-volume')).toBeVisible();
  });

  test('messages persist after refresh', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Persist Test');

    await sendMessage(page, 'persistent message');
    const msgs1 = await getMessages(page);
    expect(msgs1).toContain('persistent message');

    // Reload.
    await page.reload();
    await waitForApp(page);
    await page.waitForTimeout(1000);

    // Message should still be there.
    const msgs2 = await getMessages(page);
    expect(msgs2).toContain('persistent message');
  });

  test('reactions persist after refresh', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'React Persist');

    await sendMessage(page, 'react to me');
    await page.waitForTimeout(500);

    // React to the message (handles both desktop and mobile).
    await reactToMessage(page, 'react to me');

    // Should see reaction.
    await expect(page.locator('.reaction')).toBeVisible();

    // Reload.
    await page.reload();
    await waitForApp(page);
    await page.waitForTimeout(1000);

    // Reaction should persist.
    await expect(page.locator('.reaction')).toBeVisible();
  });
});
