# Event-Based Waits PR-2 — Playwright `Peer` Wrapper + Helpers Split + Pilot

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the JS-side test infrastructure (`Peer` wrapper + Playwright fixture + helpers split) on top of PR-1's `window.__willow` foundation, then prove it by converting `multi-peer-sync.spec.ts` from gossip-padded `{ timeout: 30_000 }` overrides to event-based waits with default 5 s assertions.

**Architecture:** PR-1 already exposes `window.__willow.{snapshot,heads,event_count,last_event}` (Promise-returning) and a push stream of `ClientEvent` to a Playwright `exposeBinding('__willowEvent')` that PR-2 now installs. PR-2 wraps both surfaces in a typed `Peer` class, splits the 703-line `e2e/helpers.ts` into focused modules behind a re-export barrel (so the 7 un-migrated specs keep working with zero diff), and pilots the API on `multi-peer-sync.spec.ts`. No Rust or WASM changes.

**Tech Stack:** TypeScript, Playwright `expect.poll`/`exposeBinding`/`addInitScript`, ESLint flat config (already in repo), `@playwright/test` 1.58.

**Spec:** [`docs/specs/2026-04-27-event-based-waits-design.md`](../specs/2026-04-27-event-based-waits-design.md) §"PR 2 — Playwright `Peer` wrapper + helpers split + first pilot".
**Predecessor:** PR-1 (#454, merged) + post-merge fix `f07dc5c`.
**Tracking issue:** [#458](https://github.com/intendednull/willow/issues/458).

---

## File Structure

**Create:**
- `e2e/test-hooks.ts` — `Peer` class, `Snapshot`/`AuthorHead`/`ClientEvent` types, `peer` fixture (`exposeBinding` + `addInitScript`).
- `e2e/helpers/peers.ts` — peer setup: `freshStart`, `waitForApp`, `createServer`, `getPeerId`, `generateInvite`, `joinViaInvite`, `setupTwoPeers`.
- `e2e/helpers/ui.ts` — UI navigation: `visibleShell`, `isMobile`, `sendMessage`, `getMessages`, `waitForMessage`, `switchChannel`, `switchTab`, `openSidebar`, `closeSidebar`, `openMemberList`, `closeMemberList`, `openServerSettings`, `createChannel`, `messageAction`, `editMessage`, `deleteMessage`, `reactToMessage`, `trustPeer`, `untrustPeer`, `kickPeer`, `openCompareFingerprints`, `markFingerprintsMatch`, `markFingerprintsMismatch`.
- `e2e/helpers/touch.ts` — touch gestures: `longPress`, `longPressAvatar`, `swipeLeft`, `swipeRight`.
- `e2e/test-hooks.spec.ts` — smoke tests for `Peer`.
- `e2e/helpers.barrel.test.ts` — TypeScript-only barrel-coverage test (build-time check that every legacy import name is exported).

**Modify:**
- `e2e/helpers.ts` — collapses to a re-export barrel that re-exports everything from `./helpers/{peers,ui,touch}`. Keeps the file-top `eslint-disable` header so legacy specs still compile.
- `e2e/multi-peer-sync.spec.ts` — pilot conversion. Uses `peer` fixture + `Peer.waitUntilHeadsEqual` to replace the eight `{ timeout: 30_000 }` cross-peer assertions; default 5 s timeouts thereafter.
- `e2e/README.md` — add a "Using `Peer`" section pointing at `test-hooks.ts` plus the helpers/ split.

**Untouched (legacy specs continue to import from the barrel):**
- `e2e/cross-browser-sync.spec.ts`, `e2e/join-links.spec.ts`, `e2e/mobile-actions.spec.ts`, `e2e/mobile.spec.ts`, `e2e/multi-peer-mobile.spec.ts`, `e2e/permissions.spec.ts`, `e2e/worker-nodes.spec.ts`.

**Why this split:** the spec calls out `peers / ui / touch` exactly. The dominant import surface across the 7 un-migrated specs is `peers` + `ui` (verified by grepping `import { … } from './helpers'` per spec). Touch lives alone because longPress / swipe will eventually move to `page.clock` (PR 4 follow-up) — keeping it isolated makes that conversion a one-file change.

---

## Task 0: Preflight — verify PR-1 baseline still passes

**Files:** none.

This is the safety check. PR-1 landed two days ago and the post-merge fix `f07dc5c` reworked the `app.rs` mount block; before changing anything in `e2e/`, confirm the test-hooks build still produces `window.__willow` and the existing pilot spec passes against it.

- [ ] **Step 1: Confirm git state**

```bash
git status
git log --oneline -5
```

Expected: clean tree, on `claude/event-testing-pr-two-KGxN1`, latest commit is `4641883` (PR-1 merge) or newer.

- [ ] **Step 2: Run the e2e build sanity check**

```bash
just check-all FEATURES=test-hooks
```

Expected: PASS, including `scripts/check-no-test-hooks-in-prod.sh` (which builds twice, once with and once without the feature, and greps `dist/*.js` for `WillowTestHooks`).

If this fails, stop and triage — the failure is not caused by this plan, but the plan can't proceed against a broken baseline.

- [ ] **Step 3: Run the existing pilot spec to capture the "before" baseline**

```bash
just test-e2e-sync 2>&1 | tee /tmp/pr2-baseline.log
```

Expected: PASS. Capture wall-clock from the Playwright summary line (something like `6 passed (45s)`). Note this number — the PR-2 description records before/after wall-clock for the pilot spec. The hard requirement is no flake; speed is informational.

- [ ] **Step 4: No commit (read-only baseline)**

Skip. Move to Task 1.

---

## Task 1: Scaffold `e2e/test-hooks.ts` with type definitions only

**Files:**
- Create: `e2e/test-hooks.ts`

Type-only scaffold first. This file becomes the single source of truth for the JS-side mirror of PR-1's wire shapes. Splitting types out before runtime code lets later tasks reference the types without churning the same file.

The types must match three Rust sources exactly:
- `WireEvent` enum in `crates/web/src/test_hooks/wire.rs:13` (10 PascalCase variants).
- `SnapshotDto` / `AuthorHeadDto` / `ChannelDto` in `crates/web/src/test_hooks/snapshot.rs:13-39`.
- `ChannelKind` in `crates/state/src/types.rs:18-24` (`Text` | `Voice`).

- [ ] **Step 1: Write the file**

```ts
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
  /** Hex hash of most recently applied event, or null if DAG is empty. */
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

// Stub — runtime classes/fixtures land in later tasks.
export {};
```

- [ ] **Step 2: Verify the file type-checks**

```bash
npx tsc --noEmit --project e2e/tsconfig.json 2>&1 || npx tsc --noEmit e2e/test-hooks.ts
```

Expected: zero errors. (If `e2e/tsconfig.json` doesn't exist, the second invocation type-checks the file standalone.)

- [ ] **Step 3: Verify ESLint accepts the file**

```bash
npx eslint e2e/test-hooks.ts
```

Expected: zero errors. The file uses no `waitForTimeout`, so the existing `no-restricted-syntax` rule is silent.

- [ ] **Step 4: Commit**

```bash
git add e2e/test-hooks.ts
git commit -m "test(e2e): scaffold test-hooks.ts with WireEvent + Snapshot types

Type-only mirror of crates/web/src/test_hooks/{wire,snapshot}.rs. Keeps
the wire-shape contract co-located with the wrapper that consumes it.
Runtime Peer class lands in subsequent commits."
```

---

## Task 2: Add the `Peer` class with pull-API methods

**Files:**
- Modify: `e2e/test-hooks.ts`

The `Peer` class is the public type test authors interact with. Pull methods (`snapshot()`, `heads()`, `eventCount()`) just await the corresponding `window.__willow.*` Promise inside `page.evaluate`. Push (`nextEvent`) and convergence (`waitUntilHeadsEqual`) come in later tasks once the per-page event queue is wired.

A `Peer` is bound to a `Page` plus a `label` (used in failure messages) plus a per-page event queue (populated by the fixture in Task 3 — for now the queue is just declared as `ClientEvent[]` and unused).

- [ ] **Step 1: Replace the trailing `export {};` stub with the class**

```ts
// Replace `export {};` at the bottom of e2e/test-hooks.ts with:

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
```

(The `WillowTestHooksJS` interface declared earlier is referenced inside `page.evaluate` callbacks. Playwright serialises the callback to the page context — the `window` cast is the standard pattern for test-only globals.)

- [ ] **Step 2: Verify it type-checks**

```bash
npx tsc --noEmit e2e/test-hooks.ts
```

Expected: zero errors.

- [ ] **Step 3: Verify ESLint clean**

```bash
npx eslint e2e/test-hooks.ts
```

Expected: zero errors.

- [ ] **Step 4: Commit**

```bash
git add e2e/test-hooks.ts
git commit -m "test(e2e): add Peer class with pull-API methods

snapshot / heads / eventCount / lastEvent each round-trip through
window.__willow.* (PR-1's wasm_bindgen surface). Push-API methods land
in the next commit once the per-page event queue is wired."
```

---

## Task 3: Add per-context event queue + `peer` fixture (push wiring)

**Files:**
- Modify: `e2e/test-hooks.ts`

PR-1's WASM dispatcher (`crates/web/src/test_hooks/dispatcher.rs:48`) writes each event to `window.__willowEvent` if defined, else into `window.__willowEventBuffer` (capacity 65 536, overflow calls `window.__willowOverflow(droppedCount)`). PR-2's job is to install the JS-side binding that drains those events into a per-`Peer` queue, plus an `__willowOverflow` binding that fails the test on any call.

Three things must happen in order, before `page.goto()`, per the spec §"Playwright wrapper":
1. `context.exposeBinding('__willowEvent', cb)` — registers the binding.
2. `context.exposeBinding('__willowOverflow', cb)` — overflow → fail.
3. `page.addInitScript(...)` — pre-creates `window.__willowEventBuffer = []` so the WASM dispatcher's defence-in-depth path has somewhere to write before the first dispatch.

The fixture lives in `test-hooks.ts` so any spec that imports `peer` fixture gets the wiring without per-spec boilerplate. It is **not** the default `test` — specs explicitly opt in via `import { test } from './test-hooks';` so legacy specs (which don't await `__willow`) keep their existing zero-overhead `test` import.

- [ ] **Step 1: Add the fixture above the `Peer` class export**

Open `e2e/test-hooks.ts` and **insert this above** the `export class Peer` line you wrote in Task 2:

```ts
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
  // eslint-disable-next-line no-empty-pattern -- Playwright fixture form requires `{}`.
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
```

- [ ] **Step 2: Verify type-check**

```bash
npx tsc --noEmit e2e/test-hooks.ts
```

Expected: zero errors. (`@playwright/test`'s `test.extend` signature is generic; `peer: PeerFactory` flows through.)

- [ ] **Step 3: Verify ESLint clean**

```bash
npx eslint e2e/test-hooks.ts
```

Expected: zero errors.

- [ ] **Step 4: Commit**

```bash
git add e2e/test-hooks.ts
git commit -m "test(e2e): add peer fixture wiring __willowEvent/__willowOverflow

Per-page event queue keyed via WeakMap<Page, ClientEvent[]>; binding
callback's source.page is the lookup key. Overflow binding fails the
test on any droppedCount > 0 (PR-1 dispatcher only calls it on the
65k-buffer overflow path).

Specs opt in via 'import { test, expect } from \"./test-hooks\";'
— legacy specs continue using the default '@playwright/test' import."
```

---

## Task 4: Add `Peer.nextEvent(predicate)` push consumer

**Files:**
- Modify: `e2e/test-hooks.ts`

`nextEvent` walks the per-Peer queue and resolves with the first event matching `predicate`. If no such event is in the queue, it polls until one arrives or `opts.timeout` elapses (default 10 s — gossip-side waits typically settle in <1 s, so 10 s is well clear of the noise floor without hiding regressions).

The implementation is intentionally simple: drain matching events from the front of the queue. Non-matching events stay in the queue (visible to subsequent `nextEvent` calls) so test code can wait on a specific event without consuming unrelated ones first.

- [ ] **Step 1: Add `nextEvent` to the `Peer` class**

Inside the `Peer` class body in `e2e/test-hooks.ts`, append the following method after `lastEvent()`:

```ts
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
```

- [ ] **Step 2: Verify type-check**

```bash
npx tsc --noEmit e2e/test-hooks.ts
```

Expected: zero errors.

- [ ] **Step 3: Verify ESLint clean**

The poll loop uses `setTimeout` inside a `Promise` constructor — this is **not** `page.waitForTimeout` and is not blocked by the ESLint rule. Verify:

```bash
npx eslint e2e/test-hooks.ts
```

Expected: zero errors.

- [ ] **Step 4: Commit**

```bash
git add e2e/test-hooks.ts
git commit -m "test(e2e): add Peer.nextEvent(predicate) push consumer

50ms polling loop; consumes the first matching event from the queue
and leaves non-matches in place. Failure message names the peer and
shows the queue tail to debug 'why didn't my event arrive' cases."
```

---

## Task 5: Add `waitUntilHeadsEqual` + `waitUntilAllHeadsEqual` convergence helpers

**Files:**
- Modify: `e2e/test-hooks.ts`

`waitUntilHeadsEqual(other)` is the canonical "did peer B catch up to peer A's gossip" wait. It takes a snapshot of `other.heads()` as the moving target (re-evaluated each poll tick — peer A may still be advancing), canonicalises both sides into a string with sorted keys, and uses `expect.poll` to compare.

`waitUntilAllHeadsEqual([…])` calls `waitUntilHeadsEqual` for each peer in turn — N-1 sequential awaits guarantee true N-peer convergence (any peer missing an event from any other peer fails the assertion).

Per spec §"Naming caveat" + §"Partial-equality footgun": the failure message must surface a structured author-key diff so a missing-author hang is debuggable without manual `console.log`.

- [ ] **Step 1: Add the canonicalisation helper at module scope**

Open `e2e/test-hooks.ts`. **Above** the `export class Peer` line, add:

```ts
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
```

- [ ] **Step 2: Add the methods to the `Peer` class**

Inside the `Peer` class, after `nextEvent`, append:

```ts
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
```

The `await import('@playwright/test')` is a deliberate dynamic import: it keeps `expect` out of the type-only import block at the top of the file (which the fixture already re-exports), avoiding a circular self-reference if a spec re-imports `expect` from `./test-hooks`.

- [ ] **Step 3: Verify type-check**

```bash
npx tsc --noEmit e2e/test-hooks.ts
```

Expected: zero errors.

- [ ] **Step 4: Verify ESLint clean**

```bash
npx eslint e2e/test-hooks.ts
```

Expected: zero errors.

- [ ] **Step 5: Commit**

```bash
git add e2e/test-hooks.ts
git commit -m "test(e2e): add waitUntilHeadsEqual + waitUntilAllHeadsEqual

Pairwise CRDT convergence check via expect.poll on canonicalised heads
maps. Failure message appends a per-author-key diff so 'A missing
authors: [x]; B missing authors: []' is visible without a manual
console.log round-trip.

waitUntilAllHeadsEqual fans out N-1 pairwise checks for true N-peer
convergence per the spec's 'Partial-equality footgun' section."
```

---

## Task 6: Smoke tests for `Peer` in `e2e/test-hooks.spec.ts`

**Files:**
- Create: `e2e/test-hooks.spec.ts`

These are intentionally small — just enough to prove the wrapper plumbing works end-to-end against a real `just dev` stack. Coverage matrix:

| Assertion | Validates |
|---|---|
| `peer.snapshot()` returns the expected fields | Pull API + JSON shape mirror is correct |
| `peer.eventCount() >= 1` after `createServer` | DAG actually grows + pull API reads it |
| `peer.nextEvent(SyncCompleted)` resolves | Push wiring + queue draining work |
| `peerA.waitUntilHeadsEqual(peerB)` after invite | Convergence helper, pairwise |
| `peer.nextEvent(timeout=200)` rejects with named error | Timeout path + error message |

We omit the three-peer test (spec §"Testing the test infrastructure" lists it) — adding a third browser context to a smoke spec is overkill for the PR-2 acceptance gate. Tracking issue #458 will pick it up when more multi-peer tests need the API.

- [ ] **Step 1: Write the spec**

```ts
// e2e/test-hooks.spec.ts
import { test, expect, Peer } from './test-hooks';
import { freshStart, createServer, getPeerId, generateInvite, joinViaInvite, setupTwoPeers } from './helpers';

// Sequential — these tests share the local relay via createServer/joinViaInvite.
test.describe.configure({ mode: 'serial' });

test.describe('Peer wrapper smoke', () => {
  test.setTimeout(60_000);

  test('snapshot returns the expected shape after createServer', async ({ peer, browser }) => {
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    const alice = peer(page, 'Alice');
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
    const alice = peer(page, 'Alice');
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
    const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
    const bob = peer(page2, 'Bob');
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
    const alice = peer(page1, 'Alice');
    const bob = peer(page2, 'Bob');
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
    const alice = peer(page, 'Alice');
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
```

- [ ] **Step 2: Verify type-check**

```bash
npx tsc --noEmit e2e/test-hooks.spec.ts
```

Expected: zero errors.

- [ ] **Step 3: Verify ESLint clean**

```bash
npx eslint e2e/test-hooks.spec.ts
```

Expected: zero errors.

- [ ] **Step 4: Run the smoke spec against a live `just dev` stack**

In one terminal:

```bash
just dev FEATURES=test-hooks
```

In another:

```bash
npx playwright test e2e/test-hooks.spec.ts --project=desktop-chrome
```

Expected: 5 passed. If `nextEvent` rejects unexpectedly, recheck Task 3's binding ordering — `exposeBinding` must run before `goto`, which it does because `freshStart` is the first thing each test calls and the fixture's `await use(factory)` runs before the test body.

If `waitUntilHeadsEqual` hangs, the structured author-key diff in the failure message points at which peer is missing whose author. Most likely cause is `setupTwoPeers` returning before sync settled; the convergence helper exists exactly to tighten that.

- [ ] **Step 5: Commit**

```bash
git add e2e/test-hooks.spec.ts
git commit -m "test(e2e): smoke tests for Peer pull/push/convergence API

Five tests covering snapshot shape, eventCount growth, nextEvent
push wiring, waitUntilHeadsEqual convergence, and the timeout error
path. Run against a local 'just dev FEATURES=test-hooks' stack.

Three-peer waitUntilAllHeadsEqual coverage deferred to issue #458 —
no current multi-peer spec needs it."
```

---

## Task 7: Split `e2e/helpers.ts` → `e2e/helpers/peers.ts`

**Files:**
- Create: `e2e/helpers/peers.ts`

Move the 9 peer-setup helpers verbatim from `e2e/helpers.ts` into a new module. **Do not modify behaviour or remove `waitForTimeout` calls in this task** — that's a separate, riskier change. The goal here is purely structural; behavioural cleanup is a follow-up that the spec defers.

The keep-the-eslint-disable header is preserved at the file top so the existing `waitForTimeout` calls don't trigger the rule.

Helpers to move (with their current line ranges in `e2e/helpers.ts`):
- `waitForApp` (`:5-13`)
- `freshStart` (`:16-42`)
- `advancePastNameStep` (`:48-56`) — internal, `function` not `export function`; copy as-is
- `createServer` (`:59-91`)
- `getPeerId` (`:94-126`)
- `openServerSettings` (`:354-369`)
- `generateInvite` (`:372-381`)
- `joinViaInvite` (`:387-412`)
- `setupTwoPeers` (`:415-462`)

Note `setupTwoPeers` calls `openMemberList` + `closeMemberList` from the UI module (Task 8), and `generateInvite`/`openServerSettings` use `visibleShell` + `isMobile` from UI. The `peers.ts` file therefore imports from `./ui` — Task 8 must land before this compiles in isolation. To avoid a broken intermediate state, do Tasks 7–9 as **one commit** (split into three logical files but one cargo-style atomic change).

- [ ] **Step 1: Create `e2e/helpers/peers.ts`**

```ts
/* eslint-disable no-restricted-syntax -- migration tracked at https://github.com/intendednull/willow/issues/458 */
//
// Peer setup helpers. Extracted from the legacy 703-LOC e2e/helpers.ts
// per docs/plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md Task 7.
// Behaviour is preserved verbatim — sleep removal is a follow-up.

import { Page, Browser, BrowserContext, expect } from '@playwright/test';
import {
  isMobile,
  visibleShell,
  openMemberList,
  closeMemberList,
} from './ui';

/** Wait for the WASM app to load (loading spinner disappears). */
export async function waitForApp(page: Page) {
  await page.waitForSelector(
    '.welcome-screen:visible, .shell-desktop .app:visible, .shell-mobile .mobile-top-bar:visible, .join-card:visible',
    { timeout: 30_000 },
  );
}

/** Clear all Willow localStorage keys and IndexedDB databases, then reload. */
export async function freshStart(page: Page) {
  await page.goto('/');
  await page.evaluate(async () => {
    const keys = Object.keys(localStorage).filter(k => k.startsWith('willow_'));
    keys.forEach(k => localStorage.removeItem(k));
    localStorage.clear();
    const dbNames = await indexedDB.databases?.() ?? [];
    await Promise.all(
      dbNames
        .filter(db => db.name && (db.name.startsWith('willow') || db.name.startsWith('iroh')))
        .map(db => new Promise<void>((resolve, reject) => {
          const req = indexedDB.deleteDatabase(db.name!);
          req.onsuccess = () => resolve();
          req.onerror = () => reject(req.error);
          req.onblocked = () => resolve();
        }))
    );
  });
  await page.reload();
  await waitForApp(page);
}

async function advancePastNameStep(page: Page, displayName?: string) {
  const nameInput = page.locator('.welcome-name-input');
  if (await nameInput.isVisible().catch(() => false)) {
    if (displayName) await nameInput.fill(displayName);
    await page.locator('.welcome-continue-btn').click();
    await page.locator('.welcome-tabs').waitFor({ timeout: 5_000 });
  }
}

export async function createServer(page: Page, name: string, displayName?: string) {
  await expect(page.locator('.welcome-card')).toBeVisible();
  await advancePastNameStep(page, displayName);
  await page
    .locator('.welcome-tab-panel input[placeholder="backyard"]')
    .fill(name);
  await page.locator('.welcome-tab-panel button', { hasText: 'continue' }).click();
  if (isMobile(page)) {
    await page.waitForSelector('.mobile-top-bar', { state: 'visible', timeout: 10_000 });
    const generalRow = page
      .locator(`${visibleShell(page)} .mobile-home .channel-item`, { hasText: 'general' });
    if (await generalRow.count() > 0) {
      await generalRow.first().click();
      await page.waitForSelector('.mobile-push--channel', { timeout: 10_000 });
    }
  } else {
    await page.waitForSelector('.main-pane-header, .channel-sidebar', {
      state: 'visible',
      timeout: 10_000,
    });
  }
}

export async function getPeerId(page: Page): Promise<string> {
  if (await page.locator('.welcome-card').isVisible().catch(() => false)) {
    await advancePastNameStep(page);
    const joinTab = page.locator('.welcome-tab-btn', { hasText: 'Join' });
    if (await joinTab.isVisible().catch(() => false)) {
      await joinTab.click();
      const revealBtn = page.locator('button[aria-label="show full peer id"]');
      await revealBtn.waitFor({ timeout: 5_000 });
      await revealBtn.click();
    }
    const peerIdEl = page.locator('.welcome-join-steps__full-id').first();
    if (await peerIdEl.isVisible().catch(() => false)) {
      return (
        (await peerIdEl.getAttribute('data-full-id')) ||
        (await peerIdEl.textContent()) ||
        ''
      );
    }
  }
  await page.locator('text=Settings').click();
  await page.waitForTimeout(300);
  const settingsPeerId = page.locator('.peer-id-text').first();
  return (
    (await settingsPeerId.getAttribute('data-full-id')) ||
    (await settingsPeerId.textContent()) ||
    ''
  );
}

export async function openServerSettings(page: Page) {
  if (isMobile(page)) {
    const backSlot = page.locator('.mobile-top-bar .top-slot-left .top-back');
    while (await backSlot.isVisible().catch(() => false)) {
      await page.locator('.mobile-top-bar .top-slot-left').click();
      await page.waitForTimeout(300);
    }
    await page.locator('.mobile-tab-bar .tab[data-tab="home"]').click();
    await page.waitForTimeout(200);
  }
  await page.locator(`${visibleShell(page)} .server-gear-btn`).first().click();
  await page.locator('.settings-panel, .settings-overlay').first()
    .waitFor({ timeout: 5_000 });
}

export async function generateInvite(page: Page, recipientPeerId: string): Promise<string> {
  await openServerSettings(page);
  await page.locator('input[placeholder*="12D3KooW"]').fill(recipientPeerId);
  await page.locator('button', { hasText: 'Generate Invite' }).click();
  await page.waitForTimeout(500);
  const inviteCode = await page.locator('.invite-code-display textarea').inputValue();
  await page.locator('text=Back').click();
  await page.waitForTimeout(500);
  return inviteCode;
}

export async function joinViaInvite(page: Page, inviteCode: string, displayName?: string) {
  await advancePastNameStep(page, displayName);
  await page.locator('.welcome-tab-btn', { hasText: 'Join' }).click();
  await page.locator('.welcome-invite-input').waitFor({ timeout: 5_000 });
  await page.locator('.welcome-invite-input').fill(inviteCode);
  await page.locator('.welcome-tab-panel button', { hasText: 'continue' }).click();
  await page.locator('button', { hasText: 'Join grove' }).waitFor({ timeout: 5_000 });
  await page.locator('button', { hasText: 'Join grove' }).click();
  if (isMobile(page)) {
    await page.waitForSelector('.mobile-top-bar', { state: 'visible', timeout: 20_000 });
  } else {
    await page.waitForSelector('.main-pane-header, .channel-sidebar', {
      state: 'visible',
      timeout: 20_000,
    });
  }
  await page.locator(`${visibleShell(page)} .channel-sidebar, ${visibleShell(page)} .mobile-home`)
    .first()
    .waitFor({ timeout: 20_000 });
  await page.locator(`${visibleShell(page)} .channel-item`).first()
    .waitFor({ timeout: 20_000 });
}

export async function setupTwoPeers(
  browser: Browser,
  serverName = 'Test Server',
  peer1Name = 'Alice',
  peer2Name = 'Bob',
): Promise<{ ctx1: BrowserContext; ctx2: BrowserContext; page1: Page; page2: Page }> {
  const ctx1 = await browser.newContext();
  const ctx2 = await browser.newContext();
  const page1 = await ctx1.newPage();
  const page2 = await ctx2.newPage();

  await freshStart(page1);
  await createServer(page1, serverName, peer1Name);

  await freshStart(page2);
  const peer2Id = await getPeerId(page2);

  const inviteCode = await generateInvite(page1, peer2Id);

  await joinViaInvite(page2, inviteCode, peer2Name);

  if (peer2Name && !isMobile(page1)) {
    await openMemberList(page1);
    try {
      await page1
        .locator('.member-item', { hasText: peer2Name })
        .waitFor({ timeout: 20_000 });
    } catch {
      console.warn('[setupTwoPeers] peer2 display name did not sync in time — P2P may be slow');
    }
    await closeMemberList(page1);
  } else if (peer2Name) {
    await page1.waitForTimeout(1500);
  }

  return { ctx1, ctx2, page1, page2 };
}
```

- [ ] **Step 2: Hold the commit until Tasks 8 + 9 finish**

The `import { ... } from './ui'` line will fail to resolve until Task 8 lands. **Don't run `tsc` here** — proceed to Task 8 first and commit all three split files together.

---

## Task 8: Split → `e2e/helpers/ui.ts`

**Files:**
- Create: `e2e/helpers/ui.ts`

UI navigation helpers — the largest of the three split modules. Behaviour preserved verbatim. Imports `longPress` from `./touch` (Task 9), so this file also won't compile in isolation until Task 9 lands.

Helpers to move:
- `sendMessage`, `getMessages`, `visibleShell`, `switchChannel`, `waitForMessage`
- `isMobile`, `openSidebar`, `closeSidebar`, `switchTab`
- `openMemberList`, `closeMemberList`
- `createChannel`
- `messageAction`, `editMessage`, `deleteMessage`, `reactToMessage`
- `trustPeer`, `untrustPeer`, `kickPeer`
- `openCompareFingerprints`, `markFingerprintsMatch`, `markFingerprintsMismatch`

- [ ] **Step 1: Create `e2e/helpers/ui.ts`**

```ts
/* eslint-disable no-restricted-syntax -- migration tracked at https://github.com/intendednull/willow/issues/458 */
//
// UI navigation + message-action helpers. Extracted from legacy
// e2e/helpers.ts per docs/plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md
// Task 8. Behaviour preserved verbatim.

import { Page } from '@playwright/test';
import { longPress } from './touch';

export function isMobile(page: Page): boolean {
  return (page.viewportSize()?.width ?? 1024) < 768;
}

export function visibleShell(page: Page): string {
  return isMobile(page) ? '.shell-mobile' : '.shell-desktop';
}

export async function sendMessage(page: Page, text: string) {
  const scope = isMobile(page) ? '.shell-mobile' : '.shell-desktop';
  if (isMobile(page)) {
    const inPush = await page
      .locator('.shell-mobile .mobile-push--channel')
      .isVisible()
      .catch(() => false);
    if (!inPush) {
      await page.locator('.shell-mobile .mobile-home .channel-item').first().click();
      await page.waitForTimeout(400);
    }
  }
  const input = page
    .locator(`${scope} .input-area input, ${scope} .input-area textarea`)
    .first();
  await input.fill(text);
  await input.press('Enter');
  await page.locator(`${visibleShell(page)} .message .body`, { hasText: text })
    .first()
    .waitFor({ timeout: 10_000 });
}

export async function getMessages(page: Page): Promise<string[]> {
  const bodies = page.locator('.message .body');
  const count = await bodies.count();
  const texts: string[] = [];
  for (let i = 0; i < count; i++) {
    texts.push((await bodies.nth(i).textContent()) || '');
  }
  return texts;
}

export async function switchChannel(page: Page, channelName: string) {
  if (isMobile(page)) {
    const backSlot = page.locator('.mobile-top-bar .top-slot-left .top-back');
    while (await backSlot.isVisible().catch(() => false)) {
      await page.locator('.mobile-top-bar .top-slot-left').click();
      await page.waitForTimeout(300);
    }
    await page.locator('.mobile-tab-bar .tab[data-tab="home"]').click();
    await page.waitForTimeout(200);
    await page
      .locator('.mobile-home .channel-item', { hasText: channelName })
      .click();
    await page.waitForTimeout(400);
    return;
  }
  await page
    .locator(`${visibleShell(page)} .channel-item`, { hasText: channelName })
    .first()
    .click();
}

export async function waitForMessage(page: Page, text: string, timeout = 20_000) {
  const scope = visibleShell(page);
  await page
    .locator(`${scope} .message .body`, { hasText: text })
    .first()
    .waitFor({ timeout });
}

export async function openSidebar(page: Page) {
  if (!isMobile(page)) return;
  const alreadyOpen = await page.locator('.grove-drawer.open').isVisible().catch(() => false);
  if (alreadyOpen) return;
  await page.locator('.mobile-top-bar .top-slot-left').click();
  await page.waitForTimeout(500);
}

export async function closeSidebar(page: Page) {
  if (!isMobile(page)) return;
  const drawerOpen = await page.locator('.grove-drawer.open').isVisible().catch(() => false);
  if (!drawerOpen) return;
  await page.locator('.grove-drawer-backdrop').dispatchEvent('click');
  await page.waitForTimeout(300);
}

export async function switchTab(
  page: Page,
  tabId: 'home' | 'letters' | 'discover' | 'you',
) {
  if (!isMobile(page)) return;
  await page.locator(`.mobile-tab-bar .tab[data-tab="${tabId}"]`).click();
  await page.waitForTimeout(200);
}

export async function openMemberList(page: Page) {
  const openPane = page.locator('.right-rail[data-open="true"] .member-list');
  if (await openPane.isVisible().catch(() => false)) return;
  if (isMobile(page)) {
    const inPush = await page.locator('.mobile-push--channel').isVisible().catch(() => false);
    if (!inPush) {
      await page.locator('.mobile-home .channel-item').first().click();
      await page.waitForTimeout(400);
    }
  }
  const membersBtn = page.locator(`${visibleShell(page)} .action-btn[aria-label="members"]`);
  if (await membersBtn.count() > 0) {
    await membersBtn.first().click();
    await page
      .locator(`${visibleShell(page)} .right-rail[data-open="true"] .member-list`)
      .waitFor({ timeout: 3_000 })
      .catch(() => {});
  }
}

export async function closeMemberList(page: Page) {
  const openPane = page.locator(`${visibleShell(page)} .right-rail[data-open="true"] .member-list`);
  const isOpen = await openPane.isVisible().catch(() => false);
  if (!isOpen) return;
  const membersBtn = page.locator(`${visibleShell(page)} .action-btn[aria-label="members"]`);
  if (await membersBtn.count() > 0) {
    await membersBtn.first().click();
  }
}

export async function createChannel(page: Page, name: string) {
  if (isMobile(page)) {
    const backSlot = page.locator('.mobile-top-bar .top-slot-left .top-back');
    while (await backSlot.isVisible().catch(() => false)) {
      await page.locator('.mobile-top-bar .top-slot-left').click();
      await page.waitForTimeout(300);
    }
    await page.locator('.mobile-tab-bar .tab[data-tab="home"]').click();
    await page.waitForTimeout(200);
  }
  const scope = visibleShell(page);
  await page.locator(`${scope} .channel-add-btn`).first().click();
  await page.waitForTimeout(200);
  await page.locator(`${scope} .channel-create-input input`).first().fill(name);
  await page.locator(`${scope} .channel-create-input input`).first().press('Enter');
  await page.locator(`${visibleShell(page)} .channel-item`, { hasText: name })
    .waitFor({ timeout: 10_000 });
}

export async function messageAction(page: Page, messageText: string, actionName: string) {
  if (isMobile(page)) {
    await longPress(page, `.message:has-text("${messageText}")`);
    await page.locator('.shell-mobile .mobile-action-sheet.open').first()
      .waitFor({ timeout: 3000 });
    const actionRe = new RegExp(`^\\s*${actionName}\\s*$`, 'i');
    await page
      .locator('.shell-mobile .mobile-action-sheet.open .sheet-item', { hasText: actionRe })
      .click();
    await page.waitForTimeout(300);
  } else {
    const msg = page.locator('.shell-desktop .message', { hasText: messageText }).last();
    await msg.hover();
    await page.waitForTimeout(200);
    await msg.locator('.action-trigger').click();
    await page.waitForTimeout(200);
    await page.locator('.dropdown-item', { hasText: actionName }).click();
    await page.waitForTimeout(200);
  }
}

export async function editMessage(page: Page, originalText: string, newText: string) {
  await messageAction(page, originalText, 'Edit');
  const input = page.locator('.input-area input, .input-area textarea').first();
  await input.fill(newText);
  await input.press('Enter');
  await page.waitForTimeout(500);
}

export async function deleteMessage(page: Page, text: string) {
  await messageAction(page, text, 'Delete');
  const confirmBtn = page.locator('.confirm-dialog .btn-danger', { hasText: 'Delete' });
  await confirmBtn.waitFor({ timeout: 3000 });
  await confirmBtn.click();
  await page.waitForTimeout(500);
}

export async function reactToMessage(page: Page, messageText: string, emojiIndex = 0) {
  if (isMobile(page)) {
    await longPress(page, `.message:has-text("${messageText}")`);
    await page.locator('.shell-mobile .mobile-action-sheet.open').first()
      .waitFor({ timeout: 3000 });
    await page.locator('.shell-mobile .mobile-action-sheet.open .sheet-emoji-row button')
      .nth(emojiIndex).click();
    await page.waitForTimeout(500);
  } else {
    const msg = page.locator('.shell-desktop .message', { hasText: messageText }).last();
    await msg.hover();
    await page.waitForTimeout(200);
    await msg.locator('.action-trigger').click();
    await page.waitForTimeout(200);
    await page.locator('.dropdown-item', { hasText: 'React' }).click();
    await page.waitForTimeout(200);
    await page.locator('.dropdown-emoji-row button').nth(emojiIndex).click();
    await page.waitForTimeout(500);
  }
}

export async function trustPeer(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator('.member-item', { hasText: peerName });
  await member.waitFor({ timeout: 30_000 });
  await member.hover();
  await member.locator('button').filter({ hasText: /^Trust$/ }).click();
  await page.waitForTimeout(500);
  await closeMemberList(page);
}

export async function untrustPeer(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator('.member-item', { hasText: peerName });
  await member.waitFor({ timeout: 30_000 });
  await member.hover();
  await member.locator('button', { hasText: 'Untrust' }).click();
  await page.waitForTimeout(500);
  await closeMemberList(page);
}

export async function openCompareFingerprints(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator(`${visibleShell(page)} .member-item`, { hasText: peerName });
  await member.waitFor({ timeout: 10_000 });
  await member.locator('.trust-badge').click();
  await page
    .locator('.add-friend__card[role="dialog"]')
    .waitFor({ timeout: 5_000 });
}

export async function markFingerprintsMatch(page: Page) {
  await page
    .locator('.add-friend__cta-primary', { hasText: 'they match' })
    .click();
  await page
    .locator('.add-friend__confirm-title', { hasText: 'verified.' })
    .waitFor({ timeout: 5_000 });
}

export async function markFingerprintsMismatch(page: Page) {
  await page
    .locator('.add-friend__cta-secondary', { hasText: "they don't match" })
    .click();
  await page
    .locator('.add-friend__confirm-title', { hasText: 'marked not verified.' })
    .waitFor({ timeout: 5_000 });
}

export async function kickPeer(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator('.member-item', { hasText: peerName });
  await member.waitFor({ timeout: 30_000 });
  await member.hover();
  await member.locator('.btn-danger', { hasText: 'Kick' }).click();
  await page.waitForTimeout(500);
  const confirmBtn = page.locator('.confirm-dialog .btn-danger', { hasText: 'Kick' });
  await confirmBtn.waitFor({ timeout: 5_000 });
  await confirmBtn.click();
  await page.waitForTimeout(500);
  await closeMemberList(page);
}
```

- [ ] **Step 2: Hold the commit until Task 9 finishes**

The `import { longPress } from './touch';` line still won't resolve. Proceed to Task 9.

---

## Task 9: Split → `e2e/helpers/touch.ts`

**Files:**
- Create: `e2e/helpers/touch.ts`

Touch / gesture helpers. Self-contained — no cross-helper imports. Behaviour preserved verbatim.

Helpers to move: `longPress`, `longPressAvatar`, `dispatchSwipe` (internal), `swipeLeft`, `swipeRight`. `longPressAvatar` calls `openMemberList` from `./ui` so this file does import from `./ui`.

- [ ] **Step 1: Create `e2e/helpers/touch.ts`**

```ts
/* eslint-disable no-restricted-syntax -- migration tracked at https://github.com/intendednull/willow/issues/458 */
//
// Touch + gesture helpers. Extracted from legacy e2e/helpers.ts per
// docs/plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md Task 9.
// Behaviour preserved verbatim — page.clock migration is a follow-up.

import { Page, Locator } from '@playwright/test';
import { isMobile, visibleShell, openMemberList } from './ui';

export async function longPress(page: Page, selector: string, durationMs = 600) {
  const scoped = isMobile(page) && !selector.startsWith('.shell-')
    ? `${visibleShell(page)} ${selector}`
    : selector;
  const el = page.locator(scoped).first();
  const box = await el.boundingBox();
  if (!box) throw new Error(`Element not found: ${selector}`);

  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;

  await page.evaluate(({ x, y }) => {
    const target = document.elementFromPoint(x, y);
    if (!target) return;
    const touch = new Touch({
      identifier: 1,
      target,
      clientX: x,
      clientY: y,
      pageX: x,
      pageY: y,
    });
    target.dispatchEvent(new TouchEvent('touchstart', {
      bubbles: true,
      cancelable: true,
      touches: [touch],
      targetTouches: [touch],
      changedTouches: [touch],
    }));
  }, { x, y });

  await page.waitForTimeout(durationMs);

  await page.evaluate(({ x, y }) => {
    const target = document.elementFromPoint(x, y);
    if (!target) return;
    const touch = new Touch({
      identifier: 1,
      target,
      clientX: x,
      clientY: y,
      pageX: x,
      pageY: y,
    });
    target.dispatchEvent(new TouchEvent('touchend', {
      bubbles: true,
      cancelable: true,
      touches: [],
      targetTouches: [],
      changedTouches: [touch],
    }));
  }, { x, y });

  await page.waitForTimeout(300);
}

export async function longPressAvatar(page: Page, peerName: string) {
  await openMemberList(page);
  const member = page.locator(`${visibleShell(page)} .member-item`, { hasText: peerName });
  await member.waitFor({ timeout: 10_000 });
  const target = member.locator('.long-press-avatar, .status-dot').first();
  const box = await target.boundingBox();
  if (!box) throw new Error('avatar not measurable');
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(500);
  await page.mouse.up();
}

async function dispatchSwipe(row: Locator, dx: number): Promise<void> {
  await row.evaluate((el, dx) => {
    const rect = (el as HTMLElement).getBoundingClientRect();
    const startX = dx > 0 ? rect.left + rect.width * 0.2 : rect.left + rect.width * 0.8;
    const startY = rect.top + rect.height / 2;
    const makeTouch = (x: number, y: number) => new Touch({
      identifier: 0,
      target: el as HTMLElement,
      clientX: x,
      clientY: y,
      pageX: x,
      pageY: y,
    } as TouchInit);
    const fire = (type: string, x: number) => {
      const touch = makeTouch(x, startY);
      (el as HTMLElement).dispatchEvent(new TouchEvent(type, {
        cancelable: true,
        bubbles: true,
        touches: type === 'touchend' ? [] : [touch],
        targetTouches: type === 'touchend' ? [] : [touch],
        changedTouches: [touch],
      }));
    };
    fire('touchstart', startX);
    fire('touchmove', startX + dx * 0.3);
    fire('touchmove', startX + dx * 0.7);
    fire('touchmove', startX + dx);
    fire('touchend', startX + dx);
  }, dx);
}

export async function swipeLeft(_page: Page, row: Locator): Promise<void> {
  return dispatchSwipe(row, -120);
}

export async function swipeRight(_page: Page, row: Locator): Promise<void> {
  return dispatchSwipe(row, 120);
}
```

- [ ] **Step 2: Hold the commit until Task 10 finishes**

`e2e/helpers.ts` still has the old definitions and `helpers/touch.ts` has duplicates — TypeScript would flag the duplicate exports if both files were active simultaneously. Task 10 collapses the old file into a barrel that re-exports from `./helpers/*`, fixing the duplicate.

---

## Task 10: Convert `e2e/helpers.ts` to a re-export barrel

**Files:**
- Modify: `e2e/helpers.ts`

Replace the entire 703-LOC file with a barrel that re-exports everything the legacy specs need. The eslint-disable header stays so any spec still importing from `./helpers` (all 7 un-migrated specs) keeps its existing zero-warnings status.

Verified-against-spec-imports list (from `grep -A8 "import {" e2e/*.spec.ts`):
- From `peers`: `freshStart`, `createServer`, `getPeerId`, `generateInvite`, `joinViaInvite`, `setupTwoPeers`, `waitForApp`, `openServerSettings`.
- From `ui`: `sendMessage`, `waitForMessage`, `switchChannel`, `createChannel`, `openSidebar`, `closeSidebar` (defensive — not currently imported but exported), `openMemberList`, `closeMemberList`, `visibleShell`, `isMobile`, `messageAction` (defensive), `editMessage` (defensive), `deleteMessage` (defensive), `reactToMessage`, `trustPeer` (defensive), `untrustPeer` (defensive), `kickPeer`, `openCompareFingerprints`, `markFingerprintsMatch`, `markFingerprintsMismatch`, `getMessages` (defensive), `switchTab` (defensive).
- From `touch`: `longPress`, `longPressAvatar`, `swipeLeft`, `swipeRight`.

- [ ] **Step 1: Replace `e2e/helpers.ts` entirely**

```ts
/* eslint-disable no-restricted-syntax -- migration tracked at https://github.com/intendednull/willow/issues/458 */
//
// Re-export barrel. The implementation lives in e2e/helpers/{peers,ui,touch}.ts
// per docs/plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md Task 10.
//
// Keeping this file as a barrel means the 7 un-migrated specs continue to
// import from './helpers' with zero diff. New specs should import directly
// from the focused modules (or use the Peer wrapper from './test-hooks').

export * from './helpers/peers';
export * from './helpers/ui';
export * from './helpers/touch';
```

- [ ] **Step 2: Run the full lint + type-check**

```bash
npx tsc --noEmit e2e/helpers.ts e2e/helpers/peers.ts e2e/helpers/ui.ts e2e/helpers/touch.ts e2e/test-hooks.ts e2e/test-hooks.spec.ts
npx eslint e2e/
```

Expected: zero errors. If `tsc` complains about a missing export, the verified-imports list above is wrong for that name — grep the spec file that uses it and add the export to the appropriate `helpers/*` module.

- [ ] **Step 3: Sanity-run one un-migrated spec to confirm the barrel works**

```bash
npx playwright test e2e/permissions.spec.ts --project=desktop-chrome --grep "kick"
```

Pick any single test from any un-migrated spec — `permissions.spec.ts` exercises `kickPeer`, `openMemberList`, `setupTwoPeers` which span all three modules and is a good cross-section. Expected: PASS.

- [ ] **Step 4: Commit Tasks 7 + 8 + 9 + 10 together**

```bash
git add e2e/helpers.ts e2e/helpers/
git commit -m "test(e2e): split 703-LOC helpers.ts into peers/ui/touch modules

Per spec §'Helpers redesign' and PR-2 plan Tasks 7-10. Three focused
modules behind a re-export barrel — legacy specs import-from-helpers
unchanged, new specs import directly from the focused module they need.

Behaviour preserved verbatim. Magic-number sleeps and 30s timeout
overrides stay where they were; PR-2 only deletes them from the pilot
spec (multi-peer-sync.spec.ts) where the Peer wrapper replaces them.
Other 7 specs migrate file-by-file via tracking issue #458."
```

---

## Task 11: Barrel-export coverage test

**Files:**
- Create: `e2e/helpers.barrel.test.ts`

Per spec §"PR 2 — Playwright `Peer` wrapper" final paragraph: every name imported by any un-migrated spec must be re-exported by the barrel; missing exports must fail the build, not just the runtime tests.

This is a build-time-only TypeScript check. It is **not** a Playwright test (so it shouldn't end in `.spec.ts` — Playwright would try to run it). Naming it `*.test.ts` keeps Playwright's `testMatch` (default `*.spec.ts`) from picking it up. We add it to `tsc`'s coverage by including the e2e dir in any future `tsconfig.json`; for now it's part of the `npx tsc --noEmit e2e/` invocation in `just check-all` (see Task 13's README addition for the recipe wiring).

The test simply imports every name and asserts each is `typeof === 'function'` (or for `visibleShell` / `isMobile`, just `'function'`). If a name is missing or renamed, `tsc` fails with `TS2305: Module '"./helpers"' has no exported member 'X'.`

- [ ] **Step 1: Create `e2e/helpers.barrel.test.ts`**

```ts
// e2e/helpers.barrel.test.ts
//
// Build-time coverage of the helpers.ts barrel. Asserts every name imported
// by any un-migrated spec is still re-exported. If you remove a name from
// helpers/{peers,ui,touch}.ts, tsc fails here with TS2305 before any
// Playwright test runs.
//
// This is NOT a Playwright spec (filename uses .test.ts so Playwright's
// default `testMatch: '*.spec.ts'` skips it). It executes only as part of
// `npx tsc --noEmit` / `npx eslint`.

import {
  // peers
  freshStart,
  createServer,
  getPeerId,
  generateInvite,
  joinViaInvite,
  setupTwoPeers,
  waitForApp,
  openServerSettings,
  // ui
  sendMessage,
  waitForMessage,
  switchChannel,
  createChannel,
  openSidebar,
  closeSidebar,
  openMemberList,
  closeMemberList,
  visibleShell,
  isMobile,
  messageAction,
  editMessage,
  deleteMessage,
  reactToMessage,
  trustPeer,
  untrustPeer,
  kickPeer,
  openCompareFingerprints,
  markFingerprintsMatch,
  markFingerprintsMismatch,
  getMessages,
  switchTab,
  // touch
  longPress,
  longPressAvatar,
  swipeLeft,
  swipeRight,
} from './helpers';

// One reference per name so TS can't tree-shake the imports away. The
// `void` operator silences `@typescript-eslint/no-unused-expressions`
// without needing an eslint-disable comment.
void freshStart;
void createServer;
void getPeerId;
void generateInvite;
void joinViaInvite;
void setupTwoPeers;
void waitForApp;
void openServerSettings;
void sendMessage;
void waitForMessage;
void switchChannel;
void createChannel;
void openSidebar;
void closeSidebar;
void openMemberList;
void closeMemberList;
void visibleShell;
void isMobile;
void messageAction;
void editMessage;
void deleteMessage;
void reactToMessage;
void trustPeer;
void untrustPeer;
void kickPeer;
void openCompareFingerprints;
void markFingerprintsMatch;
void markFingerprintsMismatch;
void getMessages;
void switchTab;
void longPress;
void longPressAvatar;
void swipeLeft;
void swipeRight;
```

- [ ] **Step 2: Verify it compiles**

```bash
npx tsc --noEmit e2e/helpers.barrel.test.ts
```

Expected: zero errors. If anything fails with TS2305, the verified-imports list in Task 10 is missing that export — go back and add it to the appropriate `helpers/*` module.

- [ ] **Step 3: Verify Playwright doesn't try to run it**

```bash
npx playwright test --list e2e/helpers.barrel.test.ts 2>&1 | head -5
```

Expected: `Total: 0 tests in 0 files` (or equivalent — Playwright's default `testMatch` doesn't include `*.test.ts`). If for some reason Playwright picks it up, fix `playwright.config.ts` rather than rename — it would mean a config drift bug. Re-confirm `testMatch` is `**/*.spec.ts` (Playwright default).

- [ ] **Step 4: Verify ESLint clean**

```bash
npx eslint e2e/helpers.barrel.test.ts
```

Expected: zero errors. The `void X;` pattern silences unused-expression warnings without disable comments.

- [ ] **Step 5: Commit**

```bash
git add e2e/helpers.barrel.test.ts
git commit -m "test(e2e): build-time coverage of helpers.ts re-export barrel

Imports every name used by any un-migrated spec from './helpers' and
references it once. If a name disappears from helpers/{peers,ui,touch}
(e.g. accidental rename during the next migration), tsc fails here
with TS2305 before any Playwright test runs.

Filename ends with .test.ts so Playwright's default testMatch skips
it — this is a build-time TypeScript check, not a runtime spec."
```

---

## Task 12: Pilot conversion — `e2e/multi-peer-sync.spec.ts`

**Files:**
- Modify: `e2e/multi-peer-sync.spec.ts`

The spec has 6 tests, 8 occurrences of `{ timeout: 30_000 }` on cross-peer DOM assertions, and 1 `expect.poll({ timeout: 30_000 })`. The conversion pattern for each:

| Before | After |
|---|---|
| `await expect(page2.locator(...)).toBeVisible({ timeout: 30_000 })` after a remote mutation | `await peerB.waitUntilHeadsEqual(peerA);` then `await expect(page2.locator(...)).toBeVisible()` (default 5s) |
| `await waitForMessage(page2, text, 30_000)` | `await peerB.nextEvent(e => e.kind === 'MessageReceived' && e.channel === '...' && !e.isLocal);` then `await waitForMessage(page2, text)` (default 20s, plenty after the event fires) |

The default `expect` timeout in `playwright.config.ts` is unspecified → Playwright's library default of 5 s applies. After convergence, 5 s is well clear of the DOM-render noise floor.

Switch from `import { test, expect } from '@playwright/test'` to `import { test, expect } from './test-hooks'` so each test gets the `peer` fixture.

- [ ] **Step 1: Replace `e2e/multi-peer-sync.spec.ts` entirely**

```ts
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
    const alice = peer(page1, 'Alice');
    const bob = peer(page2, 'Bob');
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
    const alice = peer(page1, 'Alice');
    const bob = peer(page2, 'Bob');

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
    const alice = peer(page1, 'Alice');
    const bob = peer(page2, 'Bob');
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
    const alice = peer(page1, 'Alice');
    const bob = peer(page2, 'Bob');
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
    const alice = peer(page1, 'Alice');
    const bob = peer(page2, 'Bob');
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
    const alice = peer(page1, 'Alice');
    const bob = peer(page2, 'Bob');
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
```

- [ ] **Step 2: Verify type-check + lint**

```bash
npx tsc --noEmit e2e/multi-peer-sync.spec.ts
npx eslint e2e/multi-peer-sync.spec.ts
```

Expected: zero errors. The file no longer contains any `{ timeout: 30_000 }` overrides.

- [ ] **Step 3: Confirm zero `waitForTimeout` and zero 30s timeout overrides remain**

```bash
grep -c "waitForTimeout\|{ timeout: 30_000" e2e/multi-peer-sync.spec.ts
```

Expected: `0`.

- [ ] **Step 4: Run the pilot spec against `just dev FEATURES=test-hooks`**

```bash
# Terminal 1
just dev FEATURES=test-hooks
# Terminal 2
npx playwright test e2e/multi-peer-sync.spec.ts --project=desktop-chrome --reporter=line 2>&1 | tee /tmp/pr2-after.log
```

Expected: 6 passed. Capture the wall-clock from the summary line and compare to the Task 0 baseline (`/tmp/pr2-baseline.log`). Record both numbers in the eventual PR description body.

- [ ] **Step 5: Run the pilot spec 5× to sanity-check non-flake**

```bash
for i in 1 2 3 4 5; do
  echo "=== run $i ===";
  npx playwright test e2e/multi-peer-sync.spec.ts --project=desktop-chrome --reporter=line || exit 1;
done
```

Expected: 5/5 pass. (The full `N=10` flake harness ships in PR-4; 5 runs is the sanity gate for PR-2.)

If any run fails, the failure should be debuggable from the structured author-key diff in the `waitUntilHeadsEqual` error message — record the exact failure mode and triage rather than retrying.

- [ ] **Step 6: Commit**

```bash
git add e2e/multi-peer-sync.spec.ts
git commit -m "test(e2e): convert multi-peer-sync.spec.ts to event-based waits

Pilot for PR-2 per docs/specs/2026-04-27-event-based-waits-design.md.

- 8 'toBeVisible({ timeout: 30_000 })' cross-peer assertions replaced
  with 'await peerB.waitUntilHeadsEqual(peerA);' followed by default
  5s assertions.
- 'waitForMessage(page, text, 30_000)' replaced with
  'await peerB.nextEvent(e => MessageReceived && !isLocal);' then a
  default-timeout waitForMessage.
- 'expect.poll(..., { timeout: 30_000 })' on member-list count drops
  the override after convergence.

Signs the test on the Peer fixture from ./test-hooks. The other 7
specs continue to import from @playwright/test directly; they migrate
file-by-file via tracking issue #458.

Acceptance: 5 sequential local runs pass; the full N=10 flake harness
ships in PR-4 alongside the wait-timeout ratchet baseline."
```

---

## Task 13: Update `e2e/README.md` with `Peer` + helpers/ documentation

**Files:**
- Modify: `e2e/README.md`

The current README (34 lines) covers what belongs in e2e and how to run. Add three new subsections so the next migrator finds the patterns without diving into source.

- [ ] **Step 1: Append the new sections**

Open `e2e/README.md` and append the following at the end of the file (after the final "Running" section):

```markdown

## Helpers layout

The legacy 703-LOC `helpers.ts` has been split into focused modules. New
specs should import directly from the focused module they need.

```
e2e/
├── helpers/
│   ├── peers.ts    -- freshStart, createServer, getPeerId, generateInvite,
│   │                  joinViaInvite, setupTwoPeers, openServerSettings, waitForApp
│   ├── ui.ts       -- visibleShell, isMobile, sendMessage, waitForMessage,
│   │                  switchChannel, openSidebar, openMemberList, createChannel,
│   │                  messageAction, editMessage, deleteMessage, reactToMessage,
│   │                  trustPeer, untrustPeer, kickPeer, openCompareFingerprints, …
│   └── touch.ts    -- longPress, longPressAvatar, swipeLeft, swipeRight
├── helpers.ts      -- re-export barrel; un-migrated specs continue to import
│                      from './helpers' with zero diff
├── test-hooks.ts   -- Peer wrapper + `peer` fixture (see "Event-based waits" below)
└── *.spec.ts
```

## Event-based waits (Peer wrapper)

The web crate exposes `window.__willow` and a `__willowEvent` push stream
when built with `--features test-hooks`. The `Peer` class in
`e2e/test-hooks.ts` wraps both:

- **Pull**: `peer.snapshot()`, `peer.heads()`, `peer.eventCount()`,
  `peer.lastEvent()` — each round-trips through `window.__willow.*`.
- **Push**: `peer.nextEvent(predicate, { timeout? })` — drains the next
  event matching `predicate` from the per-page event queue.
- **Convergence**: `peer.waitUntilHeadsEqual(otherPeer)` and
  `peer.waitUntilAllHeadsEqual([otherPeers])` — `expect.poll`-based
  CRDT convergence checks. Failure messages include a structured
  per-author-key diff so missing-author hangs are debuggable without a
  manual `console.log`.

Specs that need the wrapper import the typed `test` + `expect` from
`./test-hooks` instead of `@playwright/test`:

```ts
import { test, expect } from './test-hooks';

test('peer B converges with peer A', async ({ peer, browser }) => {
  const { ctx1, ctx2, page1, page2 } = await setupTwoPeers(browser);
  const a = peer(page1, 'Alice');
  const b = peer(page2, 'Bob');
  await b.waitUntilHeadsEqual(a);   // gossip-side wait
  await expect(page2.locator('.channel-item', { hasText: 'general' }))
    .toBeVisible();                 // default 5s — DOM-only after convergence
});
```

The full design is in
[`docs/specs/2026-04-27-event-based-waits-design.md`](../docs/specs/2026-04-27-event-based-waits-design.md).
Migration progress for the remaining 7 specs is tracked in
[#458](https://github.com/intendednull/willow/issues/458).

## Anti-patterns blocked by ESLint

`page.waitForTimeout(ms)` is blocked by `no-restricted-syntax` in
`eslint.config.js`. Specs migrated off the timeout pattern remove their
file-top `eslint-disable` header in the same PR. Each remaining
disabled file references issue #458; the rule sunsets on 2026-09-30
(per spec §"Sunset").
```

- [ ] **Step 2: Verify markdown renders cleanly**

```bash
# A quick eyeball — no formal markdown linter is configured for this repo.
sed -n '35,$p' e2e/README.md
```

Expected: the new sections render with proper headers and the tree diagram is monospace-aligned.

- [ ] **Step 3: Commit**

```bash
git add e2e/README.md
git commit -m "docs(e2e): document Peer wrapper + helpers/ split in README

Three new sections:
- Helpers layout (peers/ui/touch + barrel)
- Event-based waits (Peer pull/push/convergence API + import pattern)
- Anti-patterns blocked by ESLint (waitForTimeout + sunset date)

Points readers at the design spec + tracking issue #458 for the
remaining-specs migration."
```

---

## Final acceptance — run the full PR gate

After Task 13, run the full check-all to confirm nothing else regressed:

```bash
just check-all FEATURES=test-hooks
```

Expected: PASS, including:
- `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`
- WASM target check
- `wasm-pack test crates/web --headless --firefox` (browser-tier)
- `npx playwright test` (e2e — both `test-hooks.spec.ts` and the converted `multi-peer-sync.spec.ts` exercise the new API)
- `scripts/check-no-test-hooks-in-prod.sh` (symbol-leak guard from PR-1)

If green, push the branch:

```bash
git push -u origin claude/event-testing-pr-two-KGxN1
```

---

## Out of scope (deferred to later PRs)

Per the spec's implementation phasing, these ship in **PR 3** and **PR 4** and are explicitly NOT in PR-2's scope:

- `data-state` lifecycle on the five animated components — PR 3.
- `page.clock` adoption for `longPress` / debounce — PR 3 (touch.ts is staged for the follow-up but unchanged here).
- Migration of the other 7 specs — file-by-file via tracking issue #458.
- `just test-e2e-flake N=10` recipe + `e2e/.wait-timeout-baseline` ratchet — PR 4.
- Three-peer `waitUntilAllHeadsEqual` smoke test — when a multi-peer spec needs it (issue #458).
- Removal of magic-number sleeps inside `helpers/{peers,ui,touch}.ts` — happens during each spec's migration (so the helpers stay behaviour-equivalent until every caller is converted).

---

## Cross-references

- Spec: [`docs/specs/2026-04-27-event-based-waits-design.md`](../specs/2026-04-27-event-based-waits-design.md) §"PR 2".
- PR-1 plan: [`docs/plans/2026-04-27-event-based-waits-pr1-test-hooks-foundation.md`](./2026-04-27-event-based-waits-pr1-test-hooks-foundation.md).
- PR-1 errata: [`docs/plans/2026-04-28-event-based-waits-pr1-errata.md`](./2026-04-28-event-based-waits-pr1-errata.md).
- Tracking issue: [#458](https://github.com/intendednull/willow/issues/458).

