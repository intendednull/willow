// e2e/test-hooks.ts
//
// JS-side wrapper for window.__willow + the __willowEvent push stream
// installed by crates/web (--features test-hooks). See:
//   docs/specs/2026-04-27-event-based-waits-design.md
//
// Types here mirror the Rust WireEvent / SnapshotDto / ChannelDto shapes.
// Keep in sync with crates/web/src/test_hooks/{wire,snapshot}.rs.

import type { Page, BrowserContext } from '@playwright/test';

// ── Mirror of crates/web/src/test_hooks/wire.rs::WireEvent ─────────────

export type ClientEvent =
  | { kind: 'SyncCompleted'; opsApplied: number }
  | { kind: 'MessageReceived'; channel: string; messageId: string; isLocal: boolean }
  | { kind: 'PeerConnected'; peerId: string }
  | { kind: 'PeerDisconnected'; peerId: string }
  | { kind: 'ChannelCreated'; name: string }
  | { kind: 'ChannelDeleted'; name: string }
  | { kind: 'PeerTrusted'; peerId: string }
  | { kind: 'PeerUntrusted'; peerId: string }
  | { kind: 'ProfileUpdated'; peerId: string; displayName: string }
  | { kind: 'RoleCreated'; roleId: string; name: string };

// ── Mirror of crates/web/src/test_hooks/snapshot.rs ────────────────────

export interface AuthorHead {
  seq: number;
  /** 64-char lowercase hex (EventHash::Display). */
  hash: string;
}

export interface ChannelSummary {
  name: string;
  /** Mirror of willow_state::ChannelKind — serialized as the variant name. */
  kind: 'Text' | 'Voice';
}

export interface Snapshot {
  eventCount: number;
  /** Per-author DAG heads. Keys are EndpointId hex strings (BTreeMap → sorted). */
  heads: Record<string, AuthorHead>;
  /** Hex hash of most recently applied event, or null if the DAG is empty. */
  lastEvent: string | null;
  channels: ChannelSummary[];
}

// ── Internal: window.__willow surface ──────────────────────────────────

/** Shape installed at `window.__willow` by crates/web/src/test_hooks/mod.rs. */
interface WillowTestHooksJS {
  snapshot(): Promise<Snapshot>;
  heads(): Promise<Record<string, AuthorHead>>;
  event_count(): Promise<number>;
  last_event(): Promise<string | null>;
}

/** Sentinel: queue + Page + label. Returned by the fixture, not exported as a type. */
type PeerInternals = {
  page: Page;
  label: string;
  queue: ClientEvent[];
};

import { test as base } from '@playwright/test';

/**
 * Per-page event queue tracker. The fixture creates one `WeakMap<Page, ClientEvent[]>`
 * per `BrowserContext` and routes every `__willowEvent` callback to the queue
 * keyed by the originating Page (Playwright's `exposeBinding` callback receives
 * `{ page }` as the first argument's source).
 *
 * `Peer` reads the queue by reference, so any event the WASM dispatcher emits
 * after the binding is installed shows up in `peer.queue` synchronously.
 */
export type PeerFactory = (page: Page, label: string) => Promise<Peer>;

/**
 * Playwright fixture that installs the `__willow` test-hooks plumbing.
 *
 * Usage:
 *   import { test, expect } from './test-hooks';
 *   test('foo', async ({ peer, browser }) => {
 *     const a = await peer(page1, 'Alice');
 *     await a.waitUntilHeadsEqual(b);
 *   });
 *
 * The fixture's scope is `'test'` (default). Bindings are wired lazily on
 * the page's BrowserContext on first `peer(page, label)` call per context,
 * so the factory works for both Playwright's default test context AND any
 * extra contexts a test creates via `browser.newContext()` /
 * `setupTwoPeers(browser)`.
 *
 * `addInitScript` only takes effect on subsequent page loads, so call
 * `peer()` before the first `goto()` when possible. Bindings registered
 * via `exposeBinding` apply to existing pages too, so the read path
 * recovers events as soon as the binding lands.
 */
export const test = base.extend<{ peer: PeerFactory }>({
  peer: async ({}, use) => {
    // Per-page queues, keyed by the JS Page object the binding callback receives.
    const queues = new WeakMap<Page, ClientEvent[]>();
    // Track which contexts we've already wired so peer() is idempotent.
    const wired = new WeakSet<BrowserContext>();

    const wireContext = async (context: BrowserContext) => {
      if (wired.has(context)) return;
      wired.add(context);

      // 1. exposeBinding — registers the JS-side proxy. After this returns,
      //    `window.__willowEvent` is callable in every page of the context
      //    (existing and future).
      await context.exposeBinding(
        '__willowEvent',
        (source, ev: ClientEvent) => {
          const q = queues.get(source.page);
          if (q) q.push(ev);
          // No queue means the page wasn't registered via peer() — drop silently.
        },
      );

      // 2. Overflow → fail loudly. PR-1's dispatcher calls this with droppedCount
      //    only when the 65k buffer is exceeded (a real correctness bug, never
      //    backpressure under normal load).
      await context.exposeBinding('__willowOverflow', (_source, dropped: number) => {
        throw new Error(`__willow event queue overflow: ${dropped} dropped`);
      });

      // 3. addInitScript — pre-creates the buffer for FUTURE page loads in
      //    this context. Defence-in-depth for the dispatcher's fallback path
      //    that runs when `__willowEvent` is briefly absent.
      await context.addInitScript(() => {
        (window as unknown as { __willowEventBuffer: unknown[] }).__willowEventBuffer = [];
      });
    };

    /**
     * Allocate a queue for `page`, lazily wire its context, return a `Peer`.
     *
     * Idempotent: safe to call multiple times for the same page or context.
     */
    const factory: PeerFactory = async (page, label) => {
      await wireContext(page.context());
      let queue = queues.get(page);
      if (!queue) {
        queue = [];
        queues.set(page, queue);
      }
      return new Peer(page, label, queue);
    };

    await use(factory);
  },
});

// Re-export expect so spec authors can `import { test, expect } from './test-hooks';`
export { expect } from '@playwright/test';

/**
 * Engine-independent canonical form for a heads map.
 *
 * Object.keys(...).sort() makes the JSON serialisation order-independent so
 * `JSON.stringify` produces the same byte string regardless of insertion order.
 * The Rust side already serialises a BTreeMap (sorted) but we re-sort defensively.
 */
function canonicalHeads(heads: Record<string, AuthorHead>): string {
  return JSON.stringify(
    Object.keys(heads).sort().map(k => [k, heads[k].seq, heads[k].hash]),
  );
}

/** Build the "A is missing X / B is missing Y" diff used in failure messages. */
function authorKeyDiff(
  selfLabel: string,
  selfHeads: Record<string, AuthorHead>,
  otherLabel: string,
  otherHeads: Record<string, AuthorHead>,
): string {
  const selfKeys = new Set(Object.keys(selfHeads));
  const otherKeys = new Set(Object.keys(otherHeads));
  const selfMissing = [...otherKeys].filter(k => !selfKeys.has(k));
  const otherMissing = [...selfKeys].filter(k => !otherKeys.has(k));
  return (
    `${selfLabel} missing authors: [${selfMissing.join(', ')}]; ` +
    `${otherLabel} missing authors: [${otherMissing.join(', ')}]`
  );
}

/**
 * Test-side wrapper for one Willow peer (one Playwright Page).
 *
 * Construct via `peer` fixture in Task 3 — direct construction works for
 * the pull-API methods only (snapshot/heads/eventCount/lastEvent).
 * Push-API methods (nextEvent / waitUntil*) require the fixture's
 * exposeBinding wiring to populate `queue`.
 */
export class Peer {
  constructor(
    public readonly page: Page,
    public readonly label: string,
    /** Populated by the fixture's `__willowEvent` binding; empty array is valid. */
    public readonly queue: ClientEvent[] = [],
  ) {}

  /** Aggregated state snapshot. Round-trips through `window.__willow.snapshot()`. */
  async snapshot(): Promise<Snapshot> {
    return this.page.evaluate(
      () => (window as unknown as { __willow: WillowTestHooksJS }).__willow.snapshot(),
    );
  }

  /** Per-author DAG heads. */
  async heads(): Promise<Record<string, AuthorHead>> {
    return this.page.evaluate(
      () => (window as unknown as { __willow: WillowTestHooksJS }).__willow.heads(),
    );
  }

  /** Total events applied to the local DAG. */
  async eventCount(): Promise<number> {
    return this.page.evaluate(
      () => (window as unknown as { __willow: WillowTestHooksJS }).__willow.event_count(),
    );
  }

  /** Hex hash of the most recently applied event, or null if the DAG is empty. */
  async lastEvent(): Promise<string | null> {
    return this.page.evaluate(
      () => (window as unknown as { __willow: WillowTestHooksJS }).__willow.last_event(),
    );
  }

  /**
   * Wait for the next event matching `predicate` and consume it.
   *
   * Walks the per-Peer queue from the front; returns the first match and
   * removes it. Non-matching events stay in the queue (so a later
   * `nextEvent(other)` can still see them).
   *
   * Polls every 50 ms; rejects after `opts.timeout` ms (default 10_000)
   * with a message naming the peer and showing the queue tail.
   */
  async nextEvent(
    predicate: (e: ClientEvent) => boolean,
    opts: { timeout?: number } = {},
  ): Promise<ClientEvent> {
    const timeout = opts.timeout ?? 10_000;
    const deadline = Date.now() + timeout;

    while (Date.now() < deadline) {
      const idx = this.queue.findIndex(predicate);
      if (idx >= 0) {
        const [match] = this.queue.splice(idx, 1);
        return match;
      }
      await new Promise(r => setTimeout(r, 50));
    }

    const tail = this.queue.slice(-5).map(e => e.kind).join(', ') || '(empty)';
    throw new Error(
      `${this.label}.nextEvent timed out after ${timeout}ms. ` +
      `Queue tail (last 5 kinds): ${tail}`,
    );
  }

  /**
   * Wait until this peer's heads equal `other`'s heads.
   *
   * Uses `expect.poll` with a 30 s default timeout (matches the legacy
   * `{ timeout: 30_000 }` overrides this method replaces). Each poll
   * re-fetches BOTH sides' heads — `other` may still be advancing.
   *
   * NB: heads-equal is a CRDT pairwise check. Two peers can be equal
   * yet both still missing an event from a third; use
   * `waitUntilAllHeadsEqual` for N-peer convergence.
   */
  async waitUntilHeadsEqual(
    other: Peer,
    opts: { timeout?: number } = {},
  ): Promise<void> {
    const timeout = opts.timeout ?? 30_000;
    const { expect } = await import('@playwright/test');
    let lastSelf: Record<string, AuthorHead> = {};
    let lastOther: Record<string, AuthorHead> = {};
    try {
      await expect
        .poll(
          async () => {
            lastSelf = await this.heads();
            lastOther = await other.heads();
            return canonicalHeads(lastSelf);
          },
          {
            timeout,
            message: `${this.label} converge with ${other.label}`,
          },
        )
        .toBe(canonicalHeads(lastOther));
    } catch (e) {
      // Re-throw with the structured diff appended so missing-author hangs
      // are debuggable without a manual console.log round-trip.
      const diff = authorKeyDiff(this.label, lastSelf, other.label, lastOther);
      throw new Error(`${(e as Error).message}\n  ${diff}`);
    }
  }

  /**
   * Wait until this peer's heads equal each peer in `others`. Sequential
   * awaits — N-1 calls to `waitUntilHeadsEqual` — so any peer missing an
   * event from any other peer fails the assertion.
   */
  async waitUntilAllHeadsEqual(
    others: Peer[],
    opts: { timeout?: number } = {},
  ): Promise<void> {
    for (const other of others) {
      await this.waitUntilHeadsEqual(other, opts);
    }
  }
}
