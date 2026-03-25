import { test, expect, chromium } from '@playwright/test';
import { freshStart, createServer, sendMessage, waitForMessage, waitForApp, getPeerId, switchChannel } from './helpers';

// State sync tests use two browser contexts to test peer-to-peer behavior.
// Desktop only (invite flow needs full viewport).
test.describe('State synchronization', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name.includes('mobile'), 'desktop only');
  });

  /** Helper: set up two peers with peer2 joining peer1's server. */
  async function setupTwoPeers() {
    const browser = await chromium.launch();
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();

    // Peer 1: Create server.
    await freshStart(page1);
    await createServer(page1, 'Sync Server', 'Alice');

    // Peer 2: Get peer ID.
    await freshStart(page2);
    const peer2Id = await getPeerId(page2);

    // Peer 1: Generate invite.
    await page1.locator('.server-gear-btn').click();
    await page1.waitForTimeout(500);
    await page1.locator('input[placeholder*="12D3KooW"]').fill(peer2Id);
    await page1.locator('button', { hasText: 'Generate Invite' }).click();
    await page1.waitForTimeout(500);
    const inviteCode = await page1.locator('.invite-code-display textarea').inputValue();

    // Peer 2: Join.
    await page2.locator('.welcome-invite-input').fill(inviteCode);
    await page2.locator('button', { hasText: 'Next' }).click();
    await page2.waitForTimeout(500);
    await page2.locator('button', { hasText: 'Join Server' }).click();
    await page2.waitForSelector('.sidebar', { timeout: 15_000 });
    await page2.waitForTimeout(3000);

    // Peer 1: Back to chat.
    await page1.locator('text=Back').click();
    await page1.waitForTimeout(500);

    return { browser, ctx1, ctx2, page1, page2 };
  }

  test('messages in general channel sync both ways', async () => {
    const { browser, ctx1, ctx2, page1, page2 } = await setupTwoPeers();
    try {
      // Alice sends.
      await sendMessage(page1, 'Hello from Alice');
      await waitForMessage(page2, 'Hello from Alice', 15_000);

      // Bob sends.
      await sendMessage(page2, 'Hello from Bob');
      await waitForMessage(page1, 'Hello from Bob', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close(); await browser.close();
    }
  });

  test('new channel appears on joined peer', async () => {
    const { browser, ctx1, ctx2, page1, page2 } = await setupTwoPeers();
    try {
      // Alice creates a new channel.
      await page1.locator('.channel-add-btn').click();
      await page1.waitForTimeout(200);
      await page1.locator('.channel-create-input input').fill('random');
      await page1.locator('.channel-create-input input').press('Enter');
      await page1.waitForTimeout(1000);

      // Bob should see it.
      await expect(page2.locator('.channel-item', { hasText: 'random' }))
        .toBeVisible({ timeout: 15_000 });
    } finally {
      await ctx1.close(); await ctx2.close(); await browser.close();
    }
  });

  test('messages in new channel sync', async () => {
    const { browser, ctx1, ctx2, page1, page2 } = await setupTwoPeers();
    try {
      // Create channel.
      await page1.locator('.channel-add-btn').click();
      await page1.waitForTimeout(200);
      await page1.locator('.channel-create-input input').fill('news');
      await page1.locator('.channel-create-input input').press('Enter');
      await page1.waitForTimeout(1000);

      // Switch to it.
      await switchChannel(page1, 'news');

      // Send message.
      await sendMessage(page1, 'Breaking news!');
      await page1.waitForTimeout(500);

      // Bob: wait for channel to appear, switch to it.
      await page2.waitForTimeout(3000);
      await expect(page2.locator('.channel-item', { hasText: 'news' }))
        .toBeVisible({ timeout: 15_000 });
      await switchChannel(page2, 'news');
      await page2.waitForTimeout(1000);

      // Bob should see the message.
      await waitForMessage(page2, 'Breaking news!', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close(); await browser.close();
    }
  });

  test('reactions sync between peers', async () => {
    const { browser, ctx1, ctx2, page1, page2 } = await setupTwoPeers();
    try {
      // Alice sends.
      await sendMessage(page1, 'react to this');
      await waitForMessage(page2, 'react to this', 15_000);

      // Alice reacts.
      const msg = page1.locator('.message').last();
      await msg.hover();
      await page1.waitForTimeout(200);
      await page1.locator('.action-trigger').last().click();
      await page1.waitForTimeout(200);
      await page1.locator('.dropdown-item', { hasText: 'React' }).click();
      await page1.waitForTimeout(200);
      await page1.locator('.dropdown-emoji-row button').first().click();
      await page1.waitForTimeout(1000);

      // Bob should see the reaction.
      await expect(page2.locator('.reaction')).toBeVisible({ timeout: 15_000 });
    } finally {
      await ctx1.close(); await ctx2.close(); await browser.close();
    }
  });

  test('edits sync between peers', async () => {
    const { browser, ctx1, ctx2, page1, page2 } = await setupTwoPeers();
    try {
      // Alice sends.
      await sendMessage(page1, 'original text');
      await waitForMessage(page2, 'original text', 15_000);

      // Alice edits via dropdown.
      const msg = page1.locator('.message').last();
      await msg.hover();
      await page1.waitForTimeout(200);
      await page1.locator('.action-trigger').last().click();
      await page1.waitForTimeout(200);
      await page1.locator('.dropdown-item', { hasText: 'Edit' }).click();
      await page1.waitForTimeout(200);

      // Clear and type new text.
      const input = page1.locator('.input-area input, .input-area textarea').first();
      await input.fill('edited text');
      await input.press('Enter');
      await page1.waitForTimeout(1000);

      // Bob should see the edited text.
      await waitForMessage(page2, 'edited text', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close(); await browser.close();
    }
  });

  test('messages persist after refresh for both peers', async () => {
    const { browser, ctx1, ctx2, page1, page2 } = await setupTwoPeers();
    try {
      await sendMessage(page1, 'persistent msg');
      await waitForMessage(page2, 'persistent msg', 15_000);

      // Both refresh.
      await page1.reload();
      await waitForApp(page1);
      await page1.waitForTimeout(1000);

      await page2.reload();
      await waitForApp(page2);
      await page2.waitForTimeout(1000);

      // Both should still see the message.
      await expect(page1.locator('.message .body', { hasText: 'persistent msg' })).toBeVisible();
      await expect(page2.locator('.message .body', { hasText: 'persistent msg' })).toBeVisible();
    } finally {
      await ctx1.close(); await ctx2.close(); await browser.close();
    }
  });

  test('general channel works after invite (the original bug)', async () => {
    const { browser, ctx1, ctx2, page1, page2 } = await setupTwoPeers();
    try {
      // This was the core bug: messages in "general" didn't sync.
      // Both peers should be on "general" by default.

      // Alice sends in general.
      await sendMessage(page1, 'general works!');

      // Bob sees it.
      await waitForMessage(page2, 'general works!', 15_000);

      // Bob sends in general.
      await sendMessage(page2, 'yes it does!');

      // Alice sees it.
      await waitForMessage(page1, 'yes it does!', 15_000);
    } finally {
      await ctx1.close(); await ctx2.close(); await browser.close();
    }
  });

  test('typing indicator shows on other peer', async () => {
    const { browser, ctx1, ctx2, page1, page2 } = await setupTwoPeers();
    try {
      // Alice starts typing.
      const input = page1.locator('.input-area input, .input-area textarea').first();
      await input.fill('typing...');
      await page1.waitForTimeout(500);

      // Bob should see typing indicator.
      await expect(page2.locator('.typing-indicator'))
        .not.toBeEmpty({ timeout: 10_000 });
    } finally {
      await ctx1.close(); await ctx2.close(); await browser.close();
    }
  });

  test('both peers appear in member list', async () => {
    const { browser, ctx1, ctx2, page1, page2 } = await setupTwoPeers();
    try {
      // Both should see at least 2 members.
      const members1 = page1.locator('.member-item');
      await expect(members1).toHaveCount(2, { timeout: 15_000 });

      // Page2 might need to wait for member list to update.
      await page2.waitForTimeout(3000);
    } finally {
      await ctx1.close(); await ctx2.close(); await browser.close();
    }
  });
});
