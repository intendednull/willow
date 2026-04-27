import { test, expect } from '@playwright/test';
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

  test('invite flow — both peers see sidebar and general channel', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Both peers should see the sidebar.
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

  test('pre-existing channels visible after join', async ({ browser }) => {
    // This test does NOT use setupTwoPeers — manual setup with channels before invite.
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();

    try {
      // Peer 1: Create server.
      await freshStart(page1);
      await createServer(page1, 'PreChan Server', 'Alice');

      // Create 2 extra channels BEFORE invite.
      await createChannel(page1, 'announcements');
      await createChannel(page1, 'random');

      // Peer 2: Get peer ID. Pass the display name so welcome step 1
      // commits with 'Bob' — `joinViaInvite` below cannot re-set it
      // because step 1 has already been advanced past.
      await freshStart(page2);
      const peer2Id = await getPeerId(page2, 'Bob');

      // Peer 1: Generate invite.
      const inviteCode = await generateInvite(page1, peer2Id);

      // Peer 2: Join.
      await joinViaInvite(page2, inviteCode, 'Bob');

      // Peer 2 should see all 3 channels (open sidebar on mobile).
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'general' }))
        .toBeVisible({ timeout: 30_000 });
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'announcements' }))
        .toBeVisible({ timeout: 30_000 });
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'random' }))
        .toBeVisible({ timeout: 30_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('new channel created mid-session syncs to peer', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Alice creates a new channel after both are connected.
      await createChannel(page1, 'new-channel');

      // Bob should see the new channel (open sidebar on mobile).
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'new-channel' }))
        .toBeVisible({ timeout: 30_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('messages in non-general channel sync', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Alice creates a new channel.
      await createChannel(page1, 'dev');

      // Wait for Bob to see it (open sidebar on mobile).
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'dev' }))
        .toBeVisible({ timeout: 30_000 });

      // Both switch to the new channel.
      await switchChannel(page1, 'dev');
      await switchChannel(page2, 'dev');

      // Alice sends a message.
      await sendMessage(page1, 'message in dev');
      await waitForMessage(page2, 'message in dev', 30_000);

      // Bob sends a reply.
      await sendMessage(page2, 'bob in dev too');
      await waitForMessage(page1, 'bob in dev too', 30_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('both peers appear in member list', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // The right-rail members pane is hidden by default — open it via
      // the header's "members" action button before asserting counts.
      await page1.locator(`${visibleShell(page1)} button[aria-label="members"]`)
        .first().click();

      // Peer 1 should see at least 2 members (may include relay).
      const memberList = page1.locator(`${visibleShell(page1)} .member-item`);
      await expect(memberList.first()).toBeVisible({ timeout: 30_000 });
      await expect
        .poll(() => memberList.count(), { timeout: 30_000 })
        .toBeGreaterThanOrEqual(2);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('rapid channel creation by owner — both channels propagate to peer', async ({ browser }) => {
    // Owner creates two channels in quick succession; the gossip mesh must
    // deliver both events to the remote peer without dropping or reordering.
    // E2E companion to the state-machine stress_concurrent_channel_creates test.
    // Note: only the owner (Alice) can create channels — non-owners lack
    // ManageChannels permission and their creation attempts are rejected.
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Alice (owner) creates two channels back-to-back.
      await createChannel(page1, 'chan-a');
      await createChannel(page1, 'chan-b');

      // Both should appear on Bob's side after gossip delivery.
      await openSidebar(page2);
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'chan-a' }))
        .toBeVisible({ timeout: 30_000 });
      await expect(page2.locator(`${visibleShell(page2)} .channel-item`, { hasText: 'chan-b' }))
        .toBeVisible({ timeout: 30_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
