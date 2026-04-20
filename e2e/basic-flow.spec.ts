import { test, expect } from '@playwright/test';
import { freshStart, createServer, sendMessage, getMessages, waitForApp, waitForMessage, openSidebar, reactToMessage, visibleShell } from './helpers';

test.describe('Basic app flow', () => {
  test('welcome screen shows on fresh start', async ({ page }) => {
    await freshStart(page);
    await expect(page.locator('.welcome-card')).toBeVisible();
    await expect(page.locator('h1')).toContainText('What do we call you?');
  });

  test('can create a server from welcome screen', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Test Server', 'Alice');

    // Should now see the sidebar with server name.
    await expect(page.locator(`${visibleShell(page)} .sidebar-header, ${visibleShell(page)} .mobile-top-bar`).first()).toContainText('Test Server');

    // Should have a general channel.
    await expect(page.locator(`${visibleShell(page)} .channel-item`).first()).toContainText('general');
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
    await page.locator(`${visibleShell(page)} .channel-add-btn`).click();

    // Type channel name and press Enter.
    const input = page.locator(`${visibleShell(page)} .channel-create-input input`);
    await input.waitFor({ timeout: 5_000 });
    await input.fill('random');
    await input.press('Enter');

    // Should see the new channel.
    await expect(page.locator(`${visibleShell(page)} .channel-item`, { hasText: 'random' })).toBeVisible();
  });

  test('can create a voice channel', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'Voice Test');

    // Open sidebar on mobile (no-op on desktop).
    await openSidebar(page);

    // Click +.
    await page.locator(`${visibleShell(page)} .channel-add-btn`).click();

    // Click Voice toggle.
    await page.locator(`${visibleShell(page)} .type-btn`, { hasText: 'Voice' }).click();

    // Type name and submit.
    const input = page.locator(`${visibleShell(page)} .channel-create-input input`);
    await input.waitFor({ timeout: 5_000 });
    await input.fill('voice-chat');
    await input.press('Enter');

    // Should see the voice channel with speaker icon.
    const voiceChannel = page.locator(`${visibleShell(page)} .channel-item`, { hasText: 'voice-chat' });
    await expect(voiceChannel).toBeVisible();
    // Voice channels show a volume SVG icon prefix.
    await expect(voiceChannel.locator('.icon-volume, .icon-volume-1')).toBeVisible();
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

    // Message should still be there (auto-waits up to 10s).
    await expect(
      page.locator(`${visibleShell(page)} .message .body`, { hasText: 'persistent message' }).first(),
    ).toBeVisible({ timeout: 10_000 });
    const msgs2 = await getMessages(page);
    expect(msgs2).toContain('persistent message');
  });

  test('reactions persist after refresh', async ({ page }) => {
    await freshStart(page);
    await createServer(page, 'React Persist');

    await sendMessage(page, 'react to me');

    // React to the message (handles both desktop and mobile).
    await reactToMessage(page, 'react to me');

    // Should see reaction.
    await expect(page.locator(`${visibleShell(page)} .reaction`).first()).toBeVisible();

    // Reload.
    await page.reload();
    await waitForApp(page);

    // Reaction should persist (auto-waits for re-render).
    await expect(page.locator(`${visibleShell(page)} .reaction`).first()).toBeVisible({ timeout: 10_000 });
  });
});
