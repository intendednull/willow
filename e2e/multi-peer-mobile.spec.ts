import { test, expect } from './test-hooks';
import {
  sendMessage,
  waitForMessage,
  setupTwoPeers,
  createChannel,
  openSidebar,
  switchChannel,
} from './helpers';

// Shared relay + gossip mesh — keep tests inside this file sequential
// so they don't stampede the relay while `fullyParallel: true` runs
// different spec files concurrently.
test.describe.configure({ mode: 'serial' });

test.describe('Multi-peer mobile', () => {
  // Two-peer tests need extra time for setup + P2P sync.
  test.setTimeout(120_000);

  test.beforeEach(({}, testInfo) => {
    test.skip(!testInfo.project.name.startsWith('mobile'), 'mobile only');
  });

  // Migration to event-based waits per PR-2 (issue #458). Cross-peer
  // assertions gate on Peer.waitUntilHeadsEqual / Peer.nextEvent;
  // DOM checks then run with the default 5s assertion timeout.

  test('invite flow on mobile — channels list is visible on home', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Invite', 'Alice', 'Bob');
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      await bob.waitUntilHeadsEqual(alice);

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

  test('new channels visible on home tab after sync', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Chan', 'Alice', 'Bob');
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Alice creates a new channel.
      await createChannel(page1, 'mobile-news');

      // Wait for Bob's DAG to converge — includes the channel-create event.
      await bob.waitUntilHeadsEqual(alice);

      // It is rendered on the mobile home tab.
      await expect(page2.locator('.mobile-home .channel-item', { hasText: 'mobile-news' }))
        .toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('messages sync while grove drawer is closed', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Msg', 'Alice', 'Bob');
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Both peers push into the default channel's chat view.
      await page1.locator('.mobile-home .channel-item').first().click();
      await page1.locator('.shell-mobile .mobile-push--channel').waitFor();
      await page2.locator('.mobile-home .channel-item').first().click();
      await page2.locator('.shell-mobile .mobile-push--channel').waitFor();

      // Alice sends a message (drawer stays closed).
      await sendMessage(page1, 'mobile hello');

      // Wait for the cross-peer MessageReceived event before asserting DOM.
      await bob.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        !e.isLocal
      );
      await waitForMessage(page2, 'mobile hello');
      // Sanity: heads should match after the message round-trip.
      await bob.waitUntilHeadsEqual(alice);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('channel switch during active sync — messages in new channel', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Mobile Switch', 'Alice', 'Bob');
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Alice creates a channel.
      await createChannel(page1, 'mobile-dev');

      // Wait for Bob's DAG to include the channel.
      await bob.waitUntilHeadsEqual(alice);

      // Alice switches to the new channel and sends a message.
      // On mobile the `switchChannel` helper routes through the home tab
      // and taps the channel row — which also pushes the chat view.
      await switchChannel(page1, 'mobile-dev');
      await sendMessage(page1, 'dev channel msg');

      // Bob switches to the new channel; wait for the cross-peer
      // MessageReceived event before asserting the DOM body.
      await switchChannel(page2, 'mobile-dev');
      await bob.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        e.channel === 'mobile-dev' &&
        !e.isLocal
      );
      await waitForMessage(page2, 'dev channel msg');
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
