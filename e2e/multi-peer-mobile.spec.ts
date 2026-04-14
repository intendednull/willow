import { test, expect } from '@playwright/test';
import {
  sendMessage,
  waitForMessage,
  setupTwoPeers,
  createChannel,
  openSidebar,
  openMemberList,
  closeMemberList,
  switchChannel,
} from './helpers';

test.describe('Multi-peer mobile', () => {
  // Mobile two-peer tests need extra time for setup + P2P sync + mobile navigation.
  test.setTimeout(120_000);

  test.beforeEach(({}, testInfo) => {
    test.skip(!testInfo.project.name.startsWith('mobile'), 'mobile only');
  });

  test('invite flow on mobile — sidebar accessible via hamburger', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Invite', 'Alice', 'Bob');
    try {
      // Open sidebar on both peers via hamburger.
      await openSidebar(page1);
      await expect(page1.locator('.sidebar.open, .sidebar')).toBeVisible();
      await expect(page1.locator('.channel-item', { hasText: 'general' })).toBeVisible();

      await openSidebar(page2);
      await expect(page2.locator('.sidebar.open, .sidebar')).toBeVisible();
      await expect(page2.locator('.channel-item', { hasText: 'general' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('new channels visible via hamburger menu', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Chan', 'Alice', 'Bob');
    try {
      // Alice creates a new channel.
      await createChannel(page1, 'mobile-news');
      // Brief settle: give gossip a moment to process the event before polling.
      await page2.waitForTimeout(500);

      // Wait for the channel event to reach Bob (attached to DOM means synced,
      // regardless of sidebar visibility). Use a generous timeout since gossip
      // establishment can be slow when the relay is handling previous teardown.
      await expect(page2.locator('.channel-item', { hasText: 'mobile-news' }))
        .toBeAttached({ timeout: 60_000 });

      // Open sidebar so the user can see it.
      await openSidebar(page2);
      // Sidebar item should now be in view.
      await expect(page2.locator('.channel-item', { hasText: 'mobile-news' }))
        .toBeVisible({ timeout: 5_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('messages visible while sidebar is closed', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Msg', 'Alice', 'Bob');
    try {
      // Alice sends a message (sidebar is closed on mobile after setup).
      await sendMessage(page1, 'mobile hello');

      // Bob should see the message in the chat area without opening sidebar.
      await waitForMessage(page2, 'mobile hello', 30_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('channel switch during active sync — messages in new channel', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Switch', 'Alice', 'Bob');
    try {
      // Alice creates a channel.
      await createChannel(page1, 'mobile-dev');
      // Brief settle: give gossip a moment to process the event before polling.
      await page2.waitForTimeout(500);

      // Wait for the channel to appear in Bob's DOM (synced via gossip).
      await expect(page2.locator('.channel-item', { hasText: 'mobile-dev' }))
        .toBeAttached({ timeout: 60_000 });

      // Alice switches to the new channel and sends a message.
      await switchChannel(page1, 'mobile-dev');
      await sendMessage(page1, 'dev channel msg');

      // Bob switches to the new channel and should see the message.
      await switchChannel(page2, 'mobile-dev');
      await waitForMessage(page2, 'dev channel msg', 30_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
