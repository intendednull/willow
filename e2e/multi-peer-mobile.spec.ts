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

// Shared relay + gossip mesh — keep tests inside this file sequential
// so they don't stampede the relay while `fullyParallel: true` runs
// different spec files concurrently.
test.describe.configure({ mode: 'serial' });

test.describe('Multi-peer mobile', () => {
  // Mobile two-peer tests need extra time for setup + P2P sync + mobile
  // navigation.
  test.setTimeout(120_000);

  test.beforeEach(({}, testInfo) => {
    test.skip(!testInfo.project.name.startsWith('mobile'), 'mobile only');
  });

  test('invite flow on mobile — channels list is visible on home', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Invite', 'Alice', 'Bob');
    try {
      // New mobile shell renders the channel list directly on the home tab;
      // the grove drawer is a separate overlay reached from the top-bar glyph.
      await expect(page1.locator('.mobile-home .channel-item', { hasText: 'general' })).toBeVisible();
      await expect(page2.locator('.mobile-home .channel-item', { hasText: 'general' })).toBeVisible();

      // And the grove drawer opens on demand.
      await openSidebar(page1);
      await expect(page1.locator('.grove-drawer.open')).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('new channels visible on home tab after sync', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Chan', 'Alice', 'Bob');
    try {
      // Alice creates a new channel.
      await createChannel(page1, 'mobile-news');
      // Brief settle: give gossip a moment to process the event before polling.
      await page2.waitForTimeout(500);

      // Wait for the channel event to reach Bob (attached to DOM means synced,
      // regardless of scroll position). Use a generous timeout since gossip
      // establishment can be slow when the relay is handling previous teardown.
      await expect(page2.locator('.shell-mobile .channel-item', { hasText: 'mobile-news' }).first())
        .toBeAttached({ timeout: 60_000 });

      // It is rendered on the mobile home tab.
      await expect(page2.locator('.mobile-home .channel-item', { hasText: 'mobile-news' }))
        .toBeVisible({ timeout: 5_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('messages sync while grove drawer is closed', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Msg', 'Alice', 'Bob');
    try {
      // Both peers push into the default channel's chat view.
      await page1.locator('.mobile-home .channel-item').first().click();
      await page1.waitForTimeout(400);
      await page2.locator('.mobile-home .channel-item').first().click();
      await page2.waitForTimeout(400);

      // Alice sends a message (drawer stays closed).
      await sendMessage(page1, 'mobile hello');

      // Bob should see the message in the chat area.
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
      await expect(page2.locator('.shell-mobile .channel-item', { hasText: 'mobile-dev' }).first())
        .toBeAttached({ timeout: 60_000 });

      // Alice switches to the new channel and sends a message.
      // On mobile the `switchChannel` helper routes through the home tab
      // and taps the channel row — which also pushes the chat view.
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
