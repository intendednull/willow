import { test, expect } from './test-hooks';
import {
  freshStart,
  createServer,
  getPeerId,
  generateInvite,
  joinViaInvite,
  sendMessage,
} from './helpers';

// ── History-sync EOSE: `HistorySyncComplete` fires ───────────────────────────
//
// Spec:  docs/specs/2026-04-24-history-sync-eose.md
// Plan:  docs/plans/2026-05-28-relay-upgrade-bundle.md (PR 5 Task 5.4)
//
// When a peer joins a server, a trusted SyncProvider (the relay/worker provider
// class, or a peer holding an explicit `SyncProvider` grant) streams the stored
// history and, on completion, broadcasts a `HistorySyncComplete` end-of-stored-
// events marker. The joining client surfaces it as the `HistorySynced` event
// for the synced topic. This spec pins that the marker FIRES — i.e. the joining
// peer observes a `HistorySynced` event after backfill — and that the boundary
// marker is distinct from the session-wide `SyncCompleted` per-batch progress
// event (pinned decision 5: the two answer different questions).
//
// This is the "`HistorySyncComplete` fires" assertion deferred from PR 4 to
// PR 5 (plan § Conflicts resolved). It runs in CI against a live relay + worker
// mesh; the worker provider class is what emits the marker (peer-to-peer
// provider class is a documented follow-up), so the test joins through the
// shared dev relay/worker stack rather than asserting a specific provider id.

// Shared relay + gossip mesh — keep tests inside this file sequential so they
// don't stampede the relay while `fullyParallel: true` runs different spec
// files concurrently.
test.describe.configure({ mode: 'serial' });

test.describe('History-sync EOSE marker', () => {
  // Join + backfill + EOSE need the full P2P + worker round-trip.
  test.setTimeout(120_000);

  test('HistorySyncComplete fires for a joining peer after backfill', async ({ peer, browser }) => {
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();

    // Register both peers with the event fixture BEFORE the first navigation so
    // no `HistorySynced` marker is missed between join and observation.
    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');

    try {
      // Alice creates the server and seeds some stored history so the provider
      // has a non-empty store to stream (and a real `last_event_hash` to carry
      // in the marker, rather than the empty-store `None` case).
      await freshStart(page1);
      await createServer(page1, 'History Sync Server', 'Alice');
      await sendMessage(page1, 'first historical message');
      await sendMessage(page1, 'second historical message');

      // Bob joins via invite. On join his client subscribes to the server
      // topics and a trusted provider backfills the stored history.
      await freshStart(page2);
      const bobId = await getPeerId(page2, 'Bob');
      const inviteCode = await generateInvite(page1, bobId);
      await joinViaInvite(page2, inviteCode, 'Bob');

      // Bob must converge to Alice's DAG — backfill has delivered the history.
      await bob.waitUntilHeadsEqual(alice);

      // The EOSE marker must fire: Bob observes a `HistorySynced` event for the
      // synced topic. `topic` is the lowercase-hex of the marker's 32-byte
      // topic_id (64 hex chars); `provider` is the verified envelope signer.
      const synced = await bob.nextEvent((e) => e.kind === 'HistorySynced', {
        timeout: 60_000,
      });

      // Narrow the union so the field assertions are type-checked.
      if (synced.kind !== 'HistorySynced') {
        throw new Error(`expected HistorySynced, got ${synced.kind}`);
      }
      expect(synced.topic).toMatch(/^[0-9a-f]{64}$/);
      expect(synced.provider).toBeTruthy();
      expect(synced.stillPending).toBeGreaterThanOrEqual(0);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });

  test('HistorySynced is distinct from session-wide SyncCompleted', async ({ peer, browser }) => {
    const ctx1 = await browser.newContext();
    const ctx2 = await browser.newContext();
    const page1 = await ctx1.newPage();
    const page2 = await ctx2.newPage();

    const alice = await peer(page1, 'Alice');
    const bob = await peer(page2, 'Bob');

    try {
      await freshStart(page1);
      await createServer(page1, 'EOSE Distinct Server', 'Alice');
      await sendMessage(page1, 'history before join');

      await freshStart(page2);
      const bobId = await getPeerId(page2, 'Bob');
      const inviteCode = await generateInvite(page1, bobId);
      await joinViaInvite(page2, inviteCode, 'Bob');

      await bob.waitUntilHeadsEqual(alice);

      // The topic-scoped boundary marker fires...
      const synced = await bob.nextEvent((e) => e.kind === 'HistorySynced', {
        timeout: 60_000,
      });
      if (synced.kind !== 'HistorySynced') {
        throw new Error(`expected HistorySynced, got ${synced.kind}`);
      }

      // ...and the session-wide per-batch progress marker is its own event:
      // applying the backfilled batch surfaces at least one `SyncCompleted`.
      // Pinned decision 5 keeps these two events separate; observing both from
      // the same join proves the boundary marker did NOT repurpose the
      // existing per-batch event.
      const batch = await bob.nextEvent((e) => e.kind === 'SyncCompleted', {
        timeout: 60_000,
      });
      if (batch.kind !== 'SyncCompleted') {
        throw new Error(`expected SyncCompleted, got ${batch.kind}`);
      }
      expect(batch.opsApplied).toBeGreaterThan(0);
    } finally {
      await ctx1.close();
      await ctx2.close();
    }
  });
});
