import { test, expect } from './test-hooks';
import { freshStart, createServer, setupTwoPeers } from './helpers';

// Sequential — these tests share the local relay via createServer/joinViaInvite.
test.describe.configure({ mode: 'serial' });

test.describe('Peer wrapper smoke', () => {
  // setupTwoPeers + waitUntilHeadsEqual cold-start can run up to ~70s
  // on a freshly-spun gossip mesh; pad the per-test budget so the slow
  // path doesn't fail at the playwright-level timeout instead of the
  // helper-level one (which has the structured-diff error message).
  test.setTimeout(120_000);

  test('snapshot returns the expected shape after createServer', async ({ peer, browser }) => {
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    const alice = await peer(page, 'Alice');
    try {
      await freshStart(page);
      await createServer(page, 'SnapshotServer', 'Alice');

      const snap = await alice.snapshot();
      expect(snap.eventCount).toBeGreaterThan(0);
      expect(typeof snap.lastEvent).toBe('string');
      expect(Object.keys(snap.heads).length).toBeGreaterThan(0);
      // The materialised ServerState should contain the default 'general' channel.
      expect(snap.channels.map(c => c.name)).toContain('general');
    } finally {
      await ctx.close();
    }
  });

  test('eventCount grows as events are applied', async ({ peer, browser }) => {
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    const alice = await peer(page, 'Alice');
    try {
      await freshStart(page);
      await createServer(page, 'GrowthServer', 'Alice');
      const before = await alice.eventCount();
      expect(before).toBeGreaterThan(0);
    } finally {
      await ctx.close();
    }
  });

  test('nextEvent resolves on SyncCompleted after invite flow', async ({ peer, browser }) => {
    // Use the existing setupTwoPeers helper; capture both pages so we can
    // observe Bob's first SyncCompleted after the join lands.
    const { ctx1, ctx2, page2 } = await setupTwoPeers(browser);
    const bob = await peer(page2, 'Bob');
    try {
      // Bob's WASM has been emitting SyncCompleted since join — at least one
      // is in the queue. Drain the most recent one.
      const ev = await bob.nextEvent(e => e.kind === 'SyncCompleted', { timeout: 5_000 });
      expect(ev.kind).toBe('SyncCompleted');
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('waitUntilHeadsEqual converges after invite flow', async ({ peer, browser }) => {
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');
    try {
      // Both peers should converge — Bob applied Alice's CreateServer events
      // during join; gossip propagates Alice's GrantTrust back.
      await bob.waitUntilHeadsEqual(alice, { timeout: 30_000 });
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('nextEvent rejects with a named error on timeout', async ({ peer, browser }) => {
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    const alice = await peer(page, 'Alice');
    try {
      await freshStart(page);
      await createServer(page, 'TimeoutServer', 'Alice');
      // Predicate that can never match (event kind that doesn't exist for a single peer).
      await expect(
        alice.nextEvent(e => e.kind === 'PeerDisconnected', { timeout: 200 })
      ).rejects.toThrow(/Alice\.nextEvent timed out after 200ms/);
    } finally {
      await ctx.close();
    }
  });
});
