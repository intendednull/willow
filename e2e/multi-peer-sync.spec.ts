import { test, expect } from '@playwright/test';
import {
  freshStart,
  createServer,
  sendMessage,
  waitForMessage,
  waitForApp,
  getPeerId,
  switchChannel,
  setupTwoPeers,
  generateInvite,
  joinViaInvite,
  createChannel,
  editMessage,
  deleteMessage,
  reactToMessage,
  waitForPeerCount,
  openSidebar,
} from './helpers';

test.describe('Multi-peer state synchronization', () => {
  // Two-peer tests need extra time for setup + P2P sync.
  test.setTimeout(120_000);

  test('invite flow — both peers see sidebar and general channel', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Both peers should see the sidebar.
      await expect(page1.locator('.sidebar')).toBeVisible();
      await expect(page2.locator('.sidebar')).toBeVisible();

      // Both peers should see the general channel.
      await expect(page1.locator('.channel-item', { hasText: 'general' })).toBeVisible();
      await expect(page2.locator('.channel-item', { hasText: 'general' })).toBeVisible();
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('messages sync both directions', async ({ browser }) => {
    // Also covers the "general channel works after invite" regression.
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Alice sends a message.
      await sendMessage(page1, 'Hello from Alice');
      await waitForMessage(page2, 'Hello from Alice', 30_000);

      // Bob sends a message.
      await sendMessage(page2, 'Hello from Bob');
      await waitForMessage(page1, 'Hello from Bob', 30_000);
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

      // Peer 2: Get peer ID.
      await freshStart(page2);
      const peer2Id = await getPeerId(page2);

      // Peer 1: Generate invite.
      const inviteCode = await generateInvite(page1, peer2Id);

      // Peer 2: Join.
      await joinViaInvite(page2, inviteCode, 'Bob');

      // Peer 2 should see all 3 channels (open sidebar on mobile).
      await openSidebar(page2);
      await expect(page2.locator('.channel-item', { hasText: 'general' }))
        .toBeVisible({ timeout: 30_000 });
      await expect(page2.locator('.channel-item', { hasText: 'announcements' }))
        .toBeVisible({ timeout: 30_000 });
      await expect(page2.locator('.channel-item', { hasText: 'random' }))
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
      await expect(page2.locator('.channel-item', { hasText: 'new-channel' }))
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
      await expect(page2.locator('.channel-item', { hasText: 'dev' }))
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

  test('reactions sync between peers', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Alice sends a message.
      await sendMessage(page1, 'react to this');
      await waitForMessage(page2, 'react to this', 30_000);

      // Alice reacts.
      await reactToMessage(page1, 'react to this');

      // Reaction should appear on Alice's side.
      await expect(page1.locator('.reaction')).toBeVisible({ timeout: 5_000 });

      // Bob should see the reaction (P2P sync).
      await expect(page2.locator('.reaction')).toBeVisible({ timeout: 30_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('edits sync between peers', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Alice sends a message.
      await sendMessage(page1, 'original text');
      await waitForMessage(page2, 'original text', 30_000);

      // Alice edits the message.
      await editMessage(page1, 'original text', 'edited text');

      // Bob should see the edited text.
      await waitForMessage(page2, 'edited text', 30_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('deletes sync between peers', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Alice sends a message.
      await sendMessage(page1, 'delete me soon');
      await waitForMessage(page2, 'delete me soon', 30_000);

      // Alice deletes the message.
      await deleteMessage(page1, 'delete me soon');

      // Alice should see the deleted style locally (italic/muted).
      await expect(page1.locator('.message .body.deleted'))
        .toBeVisible({ timeout: 5_000 });

      // Bob should see the deleted style sync.
      await expect(page2.locator('.message .body.deleted'))
        .toBeVisible({ timeout: 30_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('messages persist after refresh for both peers', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      await sendMessage(page1, 'persistent msg');
      await waitForMessage(page2, 'persistent msg', 30_000);

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
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('both peers appear in member list', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Peer 1 should see at least 2 members (may include relay).
      const memberCount = await page1.locator('.member-item').count();
      expect(memberCount).toBeGreaterThanOrEqual(2);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('typing indicator shows on other peer', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Alice starts typing.
      const input = page1.locator('.input-area input, .input-area textarea').first();
      await input.fill('typing...');
      await page1.waitForTimeout(500);

      // Bob should see typing indicator.
      await expect(page2.locator('.typing-indicator'))
        .not.toBeEmpty({ timeout: 10_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('display names shown in messages', async ({ browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser, 'Name Server', 'Alice', 'Bob');
    try {
      // Alice sends a message.
      await sendMessage(page1, 'check my name');
      await waitForMessage(page2, 'check my name', 25_000);

      // Bob should see Alice's display name in the message author.
      const author = page2.locator('.message .author', { hasText: 'Alice' });
      await expect(author).toBeVisible({ timeout: 10_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('pre-existing messages visible to peer who joins later', async ({ browser }) => {
    // Manual setup — Peer 1 sends messages BEFORE Peer 2 joins.
    // Verifies the SyncBatch history-replay path in the WASM client.
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();

    try {
      // Peer 1: Create server and send messages before anyone else joins.
      await freshStart(page1);
      await createServer(page1, 'History Server', 'Alice');

      await sendMessage(page1, 'msg before join 1');
      await sendMessage(page1, 'msg before join 2');
      await sendMessage(page1, 'msg before join 3');

      // Peer 2: Get peer ID.
      await freshStart(page2);
      const peer2Id = await getPeerId(page2);

      // Peer 1: Generate invite.
      const inviteCode = await generateInvite(page1, peer2Id);

      // Peer 2: Join (Peer 1 is still online — Peer 2 gets history via SyncBatch).
      await joinViaInvite(page2, inviteCode, 'Bob');

      // All three pre-existing messages should arrive via SyncBatch from Peer 1.
      await waitForMessage(page2, 'msg before join 1', 30_000);
      await waitForMessage(page2, 'msg before join 2', 30_000);
      await waitForMessage(page2, 'msg before join 3', 30_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('missed messages received after peer reconnects', async ({ browser }) => {
    // Peer 2 goes offline, Peer 1 sends a message, Peer 2 comes back and
    // receives the missed message via SyncRequest on reconnect.
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Establish baseline — both peers online.
      await sendMessage(page1, 'before disconnect');
      await waitForMessage(page2, 'before disconnect', 30_000);

      // Peer 2 closes its page (simulates browser tab close / brief offline).
      await page2.close();

      // Peer 1 sends a message while Peer 2 is offline.
      await sendMessage(page1, 'sent while offline');
      await page1.waitForTimeout(1000);

      // Peer 2 reopens in the same context (localStorage preserved — server key intact).
      const page2new = await ctx2.newPage();
      await page2new.goto('/');
      await waitForApp(page2new);

      // On reconnect Peer 2 sends a SyncRequest; Peer 1 responds with the missed event.
      await waitForMessage(page2new, 'sent while offline', 30_000);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('concurrent channel creation — both channels appear on both peers', async ({ browser }) => {
    // Both peers create a channel at the same time; the gossip merge must
    // converge so both channels are visible on both sides. Mirrors the
    // state-machine stress_concurrent_channel_creates test at the E2E layer.
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    try {
      // Create channels concurrently.
      await Promise.all([
        createChannel(page1, 'chan-alice'),
        createChannel(page2, 'chan-bob'),
      ]);

      // Both channels should appear on both peers after gossip merge.
      await openSidebar(page1);
      await expect(page1.locator('.channel-item', { hasText: 'chan-alice' }))
        .toBeVisible({ timeout: 30_000 });
      await expect(page1.locator('.channel-item', { hasText: 'chan-bob' }))
        .toBeVisible({ timeout: 30_000 });

      await openSidebar(page2);
      await expect(page2.locator('.channel-item', { hasText: 'chan-alice' }))
        .toBeVisible({ timeout: 30_000 });
      await expect(page2.locator('.channel-item', { hasText: 'chan-bob' }))
        .toBeVisible({ timeout: 30_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
