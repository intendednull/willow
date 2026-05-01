import { test, expect } from './test-hooks';
import {
  freshStart,
  createServer,
  sendMessage,
  waitForMessage,
  getPeerId,
  switchChannel,
  setupTwoPeers,
  generateInvite,
  joinViaInvite,
  createChannel,
  openSidebar,
  visibleShell,
} from './helpers';

// Shared relay + gossip mesh — keep tests inside this file sequential
// so they don't stampede the relay while `fullyParallel: true` runs
// different spec files concurrently.
test.describe.configure({ mode: 'serial' });

test.describe('Multi-peer state synchronization', () => {
  // Two-peer tests need extra time for setup + P2P sync.
  test.setTimeout(120_000);

  // Sync-semantic tests (messages/edits/deletes/reactions/typing/display-names/
  // history-replay/reconnect-replay/persist-after-refresh) live in
  // crates/client/src/tests/multi_peer_sync.rs against MemNetwork — the
  // DAG merge path is identical and the test runs in < 200 ms.
  // Only DOM-reflection tests stay here.
  //
  // Migration to event-based waits per PR-2 (issue #458). Cross-peer
  // assertions now gate on Peer.waitUntilHeadsEqual / Peer.nextEvent;
  // DOM checks then run with the default 5s assertion timeout.

  test('invite flow — both peers see sidebar and general channel', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Both peers should converge before we assert UI state.
      await bob.waitUntilHeadsEqual(alice);

      // Both peers should see the sidebar (default 5s timeout — convergence already done).
      await expect(page1.locator(`${visibleShell(page1)} .channel-sidebar, ${visibleShell(page1)} .mobile-home`).first()).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-sidebar, ${visibleShell(page2)} .mobile-home`).first()).toBeVisible();

      // Both peers should see the general channel.
      await expect(page1.locator(`${visibleShell(page1)} .channel-item`, { hasText: 'general' })).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'general' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('pre-existing channels visible after join', async ({ peer, browser }) => {
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');

    try {
      // Peer 1: Create server.
      await freshStart(page1);
      await createServer(page1, 'PreChan Server', 'Alice');

      // Create 2 extra channels BEFORE invite.
      await createChannel(page1, 'announcements');
      await createChannel(page1, 'random');

      // Peer 2: Get peer ID.
      await freshStart(page2);
      const peer2Id = await getPeerId(page2);

      // Peer 1: Generate invite.
      const inviteCode = await generateInvite(page1, peer2Id);

      // Peer 2: Join.
      await joinViaInvite(page2, inviteCode, 'Bob');

      // Bob should converge to Alice's heads — including the two pre-existing channels.
      await bob.waitUntilHeadsEqual(alice);

      // Peer 2 should see all 3 channels (open sidebar on mobile).
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'general' })).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'announcements' })).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'random' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('new channel created mid-session syncs to peer', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Alice creates a new channel after both are connected.
      await createChannel(page1, 'new-channel');

      // Wait for Bob's DAG to converge to Alice's (includes the new channel event).
      await bob.waitUntilHeadsEqual(alice);

      // Bob should see the new channel (open sidebar on mobile).
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'new-channel' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('messages in non-general channel sync', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Alice creates a new channel.
      await createChannel(page1, 'dev');

      // Wait for Bob's DAG to include the channel.
      await bob.waitUntilHeadsEqual(alice);

      // Bob can now see the channel without padding.
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'dev' })).toBeVisible();

      // Both switch to the new channel.
      await switchChannel(page1, 'dev');
      await switchChannel(page2, 'dev');

      // Alice sends a message → wait for Bob's MessageReceived event,
      // then assert the DOM-rendered body.
      await sendMessage(page1, 'message in dev');
      await bob.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        e.channel === 'dev' &&
        !e.isLocal
      );
      await waitForMessage(page2, 'message in dev');

      // Bob sends a reply, Alice consumes the event then asserts the body.
      await sendMessage(page2, 'bob in dev too');
      await alice.nextEvent(e =>
        e.kind === 'MessageReceived' &&
        e.channel === 'dev' &&
        !e.isLocal
      );
      await waitForMessage(page1, 'bob in dev too');
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('both peers appear in member list', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Wait for the membership events to converge before opening the panel.
      await bob.waitUntilHeadsEqual(alice);

      await page1.locator(`${visibleShell(page1)} button[aria-label="members"]`)
        .first().click();

      // Default expect timeout (5s) is plenty after convergence.
      const memberList = page1.locator(`${visibleShell(page1)} .member-item`);
      await expect(memberList.first()).toBeVisible();
      await expect.poll(() => memberList.count()).toBeGreaterThanOrEqual(2);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('rapid channel creation by owner — both channels propagate to peer', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Alice (owner) creates two channels back-to-back.
      await createChannel(page1, 'chan-a');
      await createChannel(page1, 'chan-b');

      // Wait for Bob's DAG to include both.
      await bob.waitUntilHeadsEqual(alice);

      // Both should appear on Bob's side after gossip delivery.
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'chan-a' })).toBeVisible();
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'chan-b' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
