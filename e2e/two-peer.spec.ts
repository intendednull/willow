import { test, expect, chromium } from '@playwright/test';
import { freshStart, createServer, sendMessage, waitForMessage, waitForApp, getPeerId } from './helpers';

// Two-peer tests only run on desktop (need full viewport for invite flow).
test.describe('Two-peer messaging', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name.includes('mobile'), 'desktop only');
  });
  test('messages sync between two peers', async () => {
    const browser = await chromium.launch();

    // Create two isolated browser contexts (separate localStorage).
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();

    try {
      // Peer 1: Create a server.
      await freshStart(page1);
      await createServer(page1, 'Sync Test', 'Alice');

      // Get Peer 2's ID for the invite.
      await freshStart(page2);
      const peer2Id = await getPeerId(page2);
      expect(peer2Id).toBeTruthy();

      // Peer 1: Generate invite for Peer 2.
      // Open server settings (gear icon).
      await page1.locator('.server-gear-btn').click();
      await page1.waitForTimeout(500);

      // Fill recipient peer ID.
      const recipientInput = page1.locator('input[placeholder*="12D3KooW"]');
      await recipientInput.fill(peer2Id);

      // Generate invite.
      await page1.locator('button', { hasText: 'Generate Invite' }).click();
      await page1.waitForTimeout(500);

      // Copy the invite code.
      const inviteTextarea = page1.locator('.invite-code-display textarea');
      const inviteCode = await inviteTextarea.inputValue();
      expect(inviteCode).toBeTruthy();

      // Peer 2: Join the server.
      // Click "Next" after pasting invite.
      const joinInput = page2.locator('.welcome-invite-input');
      await joinInput.fill(inviteCode);
      await page2.locator('button', { hasText: 'Next' }).click();
      await page2.waitForTimeout(500);

      // Set display name and join.
      await page2.locator('button', { hasText: 'Join Server' }).click();
      await page2.waitForTimeout(2000);

      // Wait for sync to complete.
      await page2.waitForSelector('.sidebar', { timeout: 15_000 });
      await page2.waitForTimeout(3000);

      // Peer 1: Go back to chat.
      await page1.locator('text=Back').click();
      await page1.waitForTimeout(500);

      // Peer 1: Send a message.
      await sendMessage(page1, 'Hello from Alice!');

      // Peer 2: Should see the message (wait for sync).
      await waitForMessage(page2, 'Hello from Alice!', 15_000);

      // Peer 2: Send a reply.
      await sendMessage(page2, 'Hi Alice, from Bob!');

      // Peer 1: Should see Bob's message.
      await waitForMessage(page1, 'Hi Alice, from Bob!', 15_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
      await browser.close();
    }
  });

  test('new channels appear on both peers', async () => {
    const browser = await chromium.launch();
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();

    try {
      // Setup: Peer 1 creates server, Peer 2 joins.
      await freshStart(page1);
      await createServer(page1, 'Channel Sync', 'Alice');

      await freshStart(page2);
      const peer2Id = await getPeerId(page2);

      await page1.locator('.server-gear-btn').click();
      await page1.waitForTimeout(500);
      await page1.locator('input[placeholder*="12D3KooW"]').fill(peer2Id);
      await page1.locator('button', { hasText: 'Generate Invite' }).click();
      await page1.waitForTimeout(500);
      const inviteCode = await page1.locator('.invite-code-display textarea').inputValue();

      await page2.locator('.welcome-invite-input').fill(inviteCode);
      await page2.locator('button', { hasText: 'Next' }).click();
      await page2.waitForTimeout(500);
      await page2.locator('button', { hasText: 'Join Server' }).click();
      await page2.waitForSelector('.sidebar', { timeout: 15_000 });
      await page2.waitForTimeout(3000);

      // Peer 1: Go back and create a new channel.
      await page1.locator('text=Back').click();
      await page1.waitForTimeout(500);
      await page1.locator('.channel-add-btn').click();
      await page1.waitForTimeout(200);
      await page1.locator('.channel-create-input input').fill('announcements');
      await page1.locator('.channel-create-input input').press('Enter');
      await page1.waitForTimeout(1000);

      // Peer 2: Should see the new channel.
      await page2.waitForTimeout(3000);
      await expect(page2.locator('.channel-item', { hasText: 'announcements' }))
        .toBeVisible({ timeout: 15_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
      await browser.close();
    }
  });
});
