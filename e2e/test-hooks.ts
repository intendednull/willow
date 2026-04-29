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
export type PeerFactory = (page: Page, label: string) => Peer;

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
 * The fixture's scope is `'test'` (default): each test gets a fresh
 * BrowserContext (Playwright's default) and therefore a fresh queue map.
 */
export const test = base.extend<{ peer: PeerFactory }>({
  peer: async ({ context }, use) => {
    // Per-page queues, keyed by the JS Page object the binding callback receives.
    const queues = new WeakMap<Page, ClientEvent[]>();

    // 1. exposeBinding — must be called before any page.goto.
    await context.exposeBinding(
      '__willowEvent',
      (source, ev: ClientEvent) => {
        const q = queues.get(source.page);
        if (q) q.push(ev);
        // No queue means the page wasn't registered via peer() — drop silently.
        // peer() is the gatekeeper that allocates a queue and reloads the page.
      },
    );

    // 2. Overflow → fail loudly. PR-1's dispatcher calls this with droppedCount
    //    only when the 65k buffer is exceeded (a real correctness bug, never
    //    backpressure under normal load).
    await context.exposeBinding('__willowOverflow', (_source, dropped: number) => {
      throw new Error(`__willow event queue overflow: ${dropped} dropped`);
    });

    // 3. addInitScript — pre-creates the buffer so the WASM dispatcher's
    //    fallback path has somewhere to push if it fires before the
    //    binding is callable. Defence-in-depth; under normal Playwright
    //    ordering the buffer stays empty.
    await context.addInitScript(() => {
      (window as unknown as { __willowEventBuffer: unknown[] }).__willowEventBuffer = [];
    });

    /**
     * Allocate a queue for `page`, then return a `Peer` bound to it.
     *
     * Caller must invoke this AFTER `context.newPage()` but BEFORE the page's
     * first `goto()` — the queue must exist when the WASM dispatcher first
     * tries to push an event after the page loads.
     */
    const factory: PeerFactory = (page, label) => {
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
}
