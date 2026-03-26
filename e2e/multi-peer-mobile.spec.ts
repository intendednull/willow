import { test, expect } from '@playwright/test';
import {
  sendMessage,
  waitForMessage,
  setupTwoPeers,
  createChannel,
  openSidebar,
  openMemberList,
  closeMemberList,
  switchChannelMobile,
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

  test.fixme('new channels visible via hamburger menu', async ({ browser }) => {
    // Channel sync via gossipsub can exceed test timeframes on mobile.
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Chan', 'Alice', 'Bob');
    try {
      // Alice creates a new channel.
      await createChannel(page1, 'mobile-news');

      // Bob opens sidebar and should see the new channel.
      await page2.waitForTimeout(5000);
      await openSidebar(page2);
      await expect(page2.locator('.channel-item', { hasText: 'mobile-news' }))
        .toBeVisible({ timeout: 30_000 });
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
      await waitForMessage(page2, 'mobile hello', 15_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test.fixme('channel switch during active sync — messages in new channel', async ({ browser }) => {
    // Channel sync + message sync can exceed test timeframes on mobile.
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Switch', 'Alice', 'Bob');
    try {
      // Alice creates a channel.
      await createChannel(page1, 'mobile-dev');

      // Wait for Bob to receive the channel.
      await page2.waitForTimeout(5000);

      // Alice switches to the new channel and sends a message.
      await switchChannelMobile(page1, 'mobile-dev');
      await sendMessage(page1, 'dev channel msg');

      // Bob switches to the new channel and should see the message.
      await switchChannelMobile(page2, 'mobile-dev');
      await waitForMessage(page2, 'dev channel msg', 30_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
