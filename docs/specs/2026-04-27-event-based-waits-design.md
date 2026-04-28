# Event-Based Waits in Playwright Suite — Design

**Date:** 2026-04-27
**Status:** draft
**Branch:** `claude/event-based-waits-RNFZ9`

## Problem

The Playwright suite leans on time-based waits as flake compensation. Audit of `e2e/` (8 spec files, 1814 LOC):

- **53** `waitForTimeout(ms)` calls in helpers and specs (200ms–2000ms each).
- **71** `{ timeout: <ms> }` overrides on `expect`/`locator` assertions, including 23 occurrences of `30_000ms`, 8 of `60_000ms`, and 8 of `120_000ms`.
- **3** polling loops that sleep 300ms between iterations, gating on UI visibility rather than driving on a real signal.
- **0** uses of `waitForFunction`, `expect.poll`, `waitForResponse`, or any app-emitted event.

Two consequences: arbitrary sleeps mask race conditions instead of fixing them (per Playwright's own guidance, replacing `waitForTimeout` removes ~45% of flake), and the suite's wall-clock is dominated by sleeps that succeed long before they expire.

The underlying cause is that the web crate exposes nothing for tests to synchronise on — no `#[wasm_bindgen]` exports, no `data-testid` attributes, no readiness events. So tests guess at delays. The Willow client *does* already publish a `ClientEvent::SyncCompleted { ops_applied }` after every applied `SyncBatch` (`crates/client/src/listeners.rs:289`); it is not currently visible to JS.

## Goal

Every wait in the Playwright suite gates on a real signal: a DOM state, an applied `ClientEvent`, or a deterministic fake-clock advance. No wait is a guess.

## Scope

**In scope:**
- New cargo feature `test-hooks` on `willow-web`, off in production builds.
- WASM-exported `WillowTestHooks` API (snapshot, heads, event count, last event).
- Push-side instrumentation: WASM dispatches every `ClientEvent` to a Playwright `exposeBinding('__willowEvent', …)` callback.
- TypeScript wrapper `Peer` in `e2e/test-hooks.ts` providing `nextEvent(predicate)`, `snapshot()`, `heads()`, `eventCount()`, `waitUntilConverged(other)`.
- `data-state="<phase>"` attribute pattern on animated UI elements (drawer, modal, dropdown, tab) tied to CSS `transitionend`.
- `page.clock` adoption for the few legitimate real-duration waits (longPress, debounce).
- Helpers split: `e2e/helpers/{peers,ui,touch}.ts` replacing the current 702-line monolith.
- Pilot conversions: `helpers.ts` (full) and `multi-peer-sync.spec.ts`.
- ESLint rule blocking new `page.waitForTimeout` calls plus per-file allowlist for un-migrated specs.
- CI symbol-leak check (`! grep WillowTestHooks dist/*.wasm` for prod build) and flake harness (`just test-e2e-flake N=10`).
- GitHub tracking issue listing remaining specs for follow-up migration.

**Out of scope:**
- Migrating `permissions.spec.ts`, `mobile.spec.ts`, `mobile-actions.spec.ts`, `multi-peer-mobile.spec.ts`, `cross-browser-sync.spec.ts`, `join-links.spec.ts`, `worker-nodes.spec.ts` (tracked, migrated incrementally).
- Browser tests under `crates/web/tests/browser.rs` (already deterministic via Leptos signals).
- Rust `state` / `client` tests (already synchronous).
- Adding new test coverage for behaviours not already exercised.
- Replacing the relay/worker docker-compose harness.

## Three categories of wait, three tools

Time-based waits in the suite today fall into three buckets. The spec assigns one canonical tool per bucket. No silver bullet replaces all three.

| Bucket | What the suite waits for today | Tool | Rationale |
|---|---|---|---|
| **State convergence** | Peer B applied event H emitted by peer A; gossip settled; channel membership updated after a remote mutation | Push (`__willowEvent`) for ordered events; pull (`expect.poll(snapshot)`) for "eventually X" | Push has zero polling latency and matches multi-peer assertions. Pull runs on the test runner, supports typed matchers, has built-in failure messages, and can call across peers. |
| **DOM / animation settle** | Drawer slide, dropdown fade, modal open, tab transition | `data-state="<phase>"` attribute on the element, flipped on `transitionend`. Tests assert via `expect(el).toHaveAttribute('data-state', 'open')` | Driven by the CSS transition itself, not a guess. Auto-retried by Playwright's web-first assertions. |
| **Real durations** | `longPress` 600ms, debounce timers, HLC drift simulation | `page.clock.runFor('600ms')` or `clock.fastForward('05:00')` | Native Playwright since 1.45. Patches `Date`, `setTimeout`, `setInterval`, `requestAnimationFrame`. Covers `js_sys::Date::now()` calls inside WASM. |

Anti-patterns explicitly forbidden in the migrated suite:
- `page.waitForTimeout(ms)` — banned by ESLint rule (see CI section).
- `waitForLoadState('networkidle')` — flagged by Playwright docs as unsafe for gossip apps; not used today and must not be introduced.
- `expect(await locator.isVisible()).toBe(true)` — defeats the auto-retry. Always `await expect(locator).toBeVisible()`.
- Setting up `waitForResponse` *after* the action that triggers the response — race; promise must precede the trigger.

## `test-hooks` cargo feature

`crates/web/Cargo.toml` gains:

```toml
[features]
default = []
test-hooks = []
```

All new instrumentation code lives behind `#[cfg(feature = "test-hooks")]`. Production `trunk build --release` is unchanged: no exported symbols, no event subscription, no `window.__willow`.

The e2e build switches to `trunk build --features test-hooks`. Just recipes affected: `setup-e2e`, `test-e2e-ui`, `test-e2e-sync`, `test-e2e-perms`, `test-e2e-full`, `test-e2e-flake` (new), `check-all`.

## WASM API surface

New file `crates/web/src/test_hooks.rs`, gated:

```rust
#![cfg(feature = "test-hooks")]

use wasm_bindgen::prelude::*;
use willow_client::ClientHandle;

#[wasm_bindgen]
pub struct WillowTestHooks {
    client: ClientHandle,
}

#[wasm_bindgen]
impl WillowTestHooks {
    /// Aggregated state snapshot for `expect.poll` matchers.
    /// Shape: { event_count, heads: { author_id_hex: head_hash_hex, ... },
    ///          last_event: Option<hash_hex>, channels: [{ name, member_count }] }
    pub fn snapshot(&self) -> Result<JsValue, JsValue>;

    /// Compact per-author DAG heads. Stable across calls when DAG unchanged.
    pub fn heads(&self) -> Result<JsValue, JsValue>;

    /// Total events applied to local DAG.
    pub fn event_count(&self) -> u32;

    /// Hash of the most recently applied event (any author), or `None`.
    pub fn last_event(&self) -> Option<String>;
}
```

Push side, in the same module:

```rust
/// Subscribes to `client.subscribe_events()` and dispatches every
/// `ClientEvent` to `window.__willowEvent` if present. Bounded ring buffer
/// (capacity 1024) drops oldest on overflow and logs a warning.
pub fn install_push_dispatcher(client: ClientHandle);
```

Mounted from `app.rs` behind the same `cfg`:

```rust
#[cfg(feature = "test-hooks")]
{
    let hooks = test_hooks::WillowTestHooks::new(client_handle.clone());
    js_sys::Reflect::set(&window, &"__willow".into(), &hooks.into()).unwrap();
    test_hooks::install_push_dispatcher(client_handle.clone());
}
```

The pull API serializes via `serde_wasm_bindgen` from the existing `HeadsSummary` (`crates/state/src/sync.rs:267`) and a new lightweight `Snapshot` struct that reads from the materialised `ServerState` already held by the client. Both serializer outputs use `#[serde(rename_all = "camelCase")]` so the JS-side shape matches the TypeScript `Snapshot` interface (`eventCount`, `lastEvent`) without per-call key remapping.

The push dispatcher reuses `ClientHandle::subscribe_events()` (`crates/client/src/lib.rs:226`); no new emit points are added inside `willow-client` or `willow-state`.

## Playwright wrapper — `Peer`

New file `e2e/test-hooks.ts`:

```ts
export type ClientEvent =
  | { kind: 'SyncCompleted'; opsApplied: number }
  | { kind: 'EventApplied'; hash: string; author: string }
  | { kind: 'PeerJoined'; peerId: string }
  | { kind: 'ChannelCreated'; name: string }
  // …mirror of willow-client's ClientEvent, kept in sync via codegen check

export interface Snapshot {
  eventCount: number;
  heads: Record<string, string>;
  lastEvent: string | null;
  channels: Array<{ name: string; memberCount: number }>;
}

export class Peer {
  constructor(public readonly page: Page, public readonly label: string);

  /** Drain the next event matching `predicate` from the per-page push queue. */
  async nextEvent(
    predicate: (e: ClientEvent) => boolean,
    opts?: { timeout?: number },
  ): Promise<ClientEvent>;

  async snapshot(): Promise<Snapshot>;
  async heads(): Promise<Record<string, string>>;
  async eventCount(): Promise<number>;

  /** Wait until this peer's heads equal `other`'s heads. Uses expect.poll. */
  async waitUntilConverged(
    other: Peer,
    opts?: { timeout?: number },
  ): Promise<void>;
}
```

Per-page push queue is set up in a Playwright fixture before `page.goto`:

```ts
test.beforeEach(async ({ page, context }) => {
  const queue: ClientEvent[] = [];
  await context.exposeBinding('__willowEvent', (_src, ev: ClientEvent) => {
    queue.push(ev);
  });
  await page.addInitScript(() => {
    // Placeholder for events emitted before the binding is wired.
    (window as any).__willowEventBuffer = [];
    const origDispatch = (window as any).__willowEvent;
    (window as any).__willowEvent = (ev: any) => {
      if (origDispatch) origDispatch(ev);
      else (window as any).__willowEventBuffer.push(ev);
    };
  });
  // Peer construction stores `queue` so nextEvent can drain it.
});
```

`waitUntilConverged` uses `expect.poll` with default intervals `[100, 250, 500, 1000]` and a 30s timeout:

```ts
async waitUntilConverged(other: Peer, opts) {
  await expect.poll(
    async () => JSON.stringify(await this.heads()),
    { timeout: opts?.timeout ?? 30_000, message: `${this.label} converge with ${other.label}` },
  ).toBe(JSON.stringify(await other.heads()));
}
```

## `data-state` attribute pattern

For animated UI elements (drawer, modal, dropdown, tab transition, action sheet), the component sets a `data-state` attribute reflecting the transition phase, and tests gate on the attribute rather than sleeping for the animation.

States: `closed`, `opening`, `open`, `closing`. The `opening`/`closing` phases are set imperatively when the transition starts; `open`/`closed` are flipped on the `transitionend` event.

```rust
// crates/web/src/components/drawer.rs (illustrative)
let state = RwSignal::new("closed");
view! {
    <div
        class="drawer"
        data-state=move || state.get()
        on:transitionend=move |_| {
            state.set(match state.get_untracked() {
                "opening" => "open",
                "closing" => "closed",
                other => other,
            });
        }
    >
        ...
    </div>
}
```

Tests:

```ts
await openSidebarBtn.click();
await expect(drawer).toHaveAttribute('data-state', 'open');
```

This replaces every `await page.waitForTimeout(300)` after a UI transition. Components affected: mobile drawer (`mobile_shell.rs`), action sheet (`mobile_action_sheet.rs`), modal dialog (`dialog.rs`), dropdown menu (`dropdown.rs`), tab bar transitions (`tab_bar.rs`).

Existing `data-*` attributes (`data-tab`, `data-open`, `data-state` already on `status_dot.rs` / `grove_rail.rs`) are reused or extended; no new attribute namespace is introduced.

## `page.clock` for real durations

Three legitimate real-duration waits exist today and are migrated to `page.clock`:

1. **`longPress(locator, duration)`** in `e2e/helpers.ts:264`. Today: `mouse.down()` + `waitForTimeout(duration)` + `mouse.up()`. New: `clock.runFor(duration)` between down and up. Test must `await context.clock.install()` in the relevant `beforeEach`.
2. **Debounce timers in the input box** (typing-indicator throttle, message-edit autosave). Tests that exercise these flows install the clock and `runFor` past the debounce window.
3. **HLC drift simulation** (no current test, but a likely future need): `clock.setFixedTime(date)` per peer, then `clock.runFor` to advance.

Clock install is **opt-in per test** (or `describe` block), not global. Default e2e tests run with real time so iroh background timers are unaffected.

## Helpers redesign

`e2e/helpers.ts` (702 LOC) is split into three focused modules. Magic-number sleeps are removed in the same change.

```
e2e/
├── helpers/
│   ├── peers.ts    -- setupTwoPeers, joinViaInvite, getPeerId, freshStart
│   ├── ui.ts       -- openSidebar, switchChannel, switchTab, messageAction
│   └── touch.ts    -- longPress, tap, swipe
├── test-hooks.ts   -- Peer wrapper, ClientEvent type, Snapshot type
└── ...spec.ts
```

Three patterns purged:

1. `while (await locator.isVisible()) { click; waitForTimeout(300) }` loops at `helpers.ts:178`, `:358`, `:471` → recursive `clickBack` with bounded depth gating on `expect(backBtn).toBeHidden()`.
2. Bare `waitForTimeout` after navigation (e.g. `helpers.ts:118` after Settings nav) → assert on a landing-page locator instead.
3. `{ timeout: 30_000 }` overrides on `toBeVisible` for cross-peer assertions (23 occurrences) → `await peer.waitUntilConverged(other)` before the assertion, then default 5s timeout.

**Targeted exception rule.** A small number of waits may have no event-based equivalent (e.g. a CSS transition on a third-party component without `transitionend`). These require a `// time-wait: <reason>` comment plus a per-line `eslint-disable-next-line` referencing the rule and the tracking issue. No blanket allowlist.

## Pilot conversions

Two pilots ship in the same PR as the infra so the API is exercised under real load:

1. **`e2e/helpers.ts` → `e2e/helpers/{peers,ui,touch}.ts`.** Highest leverage: every spec re-imports through the new modules. Validates the `data-state` pattern (UI helpers) and `page.clock` (touch helpers) end-to-end.
2. **`e2e/multi-peer-sync.spec.ts`.** Worst gossip-pad offender: 30s timeout overrides on every cross-peer assertion plus two `waitForTimeout(2000)` calls. Validates `Peer.nextEvent` and `Peer.waitUntilConverged` in their dominant use case.

Acceptance for the pilots: `just test-e2e-flake N=10` over both must pass with zero failures. Wall-clock for `multi-peer-sync.spec.ts` should drop measurably (current ~45s; target <20s once gossip-pads are removed, but the hard requirement is correctness, not speed).

## Tracking issue

A GitHub issue `e2e: migrate remaining specs to event-based waits` is opened concurrently with this spec. Body lists each remaining file as a checklist:

- [ ] `e2e/permissions.spec.ts`
- [ ] `e2e/mobile.spec.ts`
- [ ] `e2e/mobile-actions.spec.ts`
- [ ] `e2e/multi-peer-mobile.spec.ts`
- [ ] `e2e/cross-browser-sync.spec.ts`
- [ ] `e2e/join-links.spec.ts`
- [ ] `e2e/worker-nodes.spec.ts`

Each file gets its own small PR; each PR removes that file's entry from the ESLint allowlist. The tracking issue is referenced by every `eslint-disable-next-line` comment in un-migrated files.

## CI gate

**Build verification.** `just check-all` adds two steps:

1. `trunk build --release` (no features) → grep the resulting `dist/*.wasm` for `WillowTestHooks`. Must not find. Fails CI if leaked. Catches accidental `default = ["test-hooks"]` regressions.
2. `trunk build --features test-hooks` → grep for `WillowTestHooks`. Must find. Sanity check the gating actually compiles.

**Lint.** New `e2e/.eslintrc.cjs` (or extension of root config) adds:

```js
{
  rules: {
    'no-restricted-syntax': ['error', {
      selector: "CallExpression[callee.property.name='waitForTimeout']",
      message: 'Use event-based waits. See docs/specs/2026-04-27-event-based-waits-design.md.',
    }],
  },
}
```

Un-migrated specs receive `/* eslint-disable no-restricted-syntax -- tracked: <issue-url> */` at file top. As each file migrates, the disable comment is removed in the same PR.

**Ratchet.** A small `scripts/check-wait-timeout-count.sh` counts `waitForTimeout` occurrences in `e2e/`, compares to a baseline file `e2e/.wait-timeout-baseline`, and fails CI if the count increases. Decreases update the baseline. Prevents regressions while migration is in flight.

**Flake harness.** New just recipe:

```
test-e2e-flake N=5:
    for i in $(seq 1 {{N}}); do \
        just test-e2e-ui || exit 1; \
    done
```

Migrated specs must pass `N=10` in CI on every PR that modifies `e2e/`.

## Risks and tradeoffs

| Risk | Mitigation |
|---|---|
| Push queue grows unbounded if a test forgets to drain | Bounded ring buffer (capacity 1024) inside the WASM dispatcher. Oldest dropped on overflow with `console.warn`. |
| `exposeBinding` registered after the app boots → first events dropped | `addInitScript` installs a `window.__willowEvent` placeholder that buffers into `__willowEventBuffer`. Real binding drains the buffer on first call. |
| `page.clock` interferes with iroh's WASM timers (gossip heartbeats, retry backoff) | Clock install is opt-in per test; default e2e tests run with real time. Tests that install the clock are explicit about which timers they advance. |
| `data-state` attributes proliferate across markup | Only on elements tests gate on (drawer, modal, dropdown, tab, action sheet — finite, listed above). Documented in the `e2e/README.md`. |
| Cargo feature drift: e2e build differs from prod build behaviour | `#[cfg]` only adds inert mounting code; no behaviour change. Validated by running existing `just test-browser` once with `--features test-hooks` and asserting the same pass set. |
| Migration stalls; ESLint allowlist becomes permanent | Tracking issue + ratchet script (count must monotonically decrease). Renewed on every file migration PR. |
| `ClientEvent` schema drifts between Rust and TS `Peer` wrapper | TS type kept in `e2e/test-hooks.ts` with a `// keep in sync with crates/client/src/lib.rs ClientEvent` marker. A small Rust integration test serializes every `ClientEvent` variant and asserts the TS type covers it (codegen check is a follow-up if drift becomes painful). |

**Runner-up rejected — DOM-only (`data-testid` + attribute polling, no WASM API).** Rejected because gossip convergence is non-DOM: peer B can apply role/permission events that change no UI, but downstream assertions still depend on those events being applied. Forcing a DOM proxy for every state fact bloats markup and still does not cover the heads-equality check that `waitUntilConverged` relies on.

**Runner-up rejected — always-on `window.__willow` API (no cargo feature).** Rejected on privacy: third-party JS in production could read DAG heads and event counts. Cost of the feature gate is one cargo feature, one CI line, and one symbol-leak grep — small and one-time.

## Testing the test infrastructure

**WASM-side.** New tests in `crates/web/tests/browser.rs` (gated `#[cfg(feature = "test-hooks")]`) construct a `ClientHandle`, apply known events, and assert `WillowTestHooks::snapshot()` and `heads()` return the expected shape and values. Pure WASM, no Playwright, fast.

**Playwright-side.** New `e2e/test-hooks.spec.ts` smoke-tests the `Peer` wrapper:

- `peer.snapshot()` returns the expected fields after a fresh start.
- `peer.eventCount()` equals 1 after `CreateServer`.
- `peer.nextEvent(e => e.kind === 'SyncCompleted')` resolves on the first heartbeat.
- `peer.waitUntilConverged(self)` is a no-op (single peer trivially converges).

**Pilot acceptance.** `multi-peer-sync.spec.ts` is run 10× via `just test-e2e-flake N=10` before the migration (baseline) and 10× after. After must be ≤ baseline failures, target zero.

## Open questions

- Do we need a `Peer.events()` accessor that returns the full applied-event log (filtered by predicate)? Probably yes for permissions tests; defer until those migrate.
- Should the `data-state` attribute pattern be lifted into a shared Leptos helper component to enforce consistency? Defer; first apply the pattern manually to the listed components and refactor if duplication accumulates.
- WebSocket-frame-level waits via `page.waitForEvent('websocket')` are powerful for relay tests but couple to wire format. Out of scope for this spec; revisit if relay-specific flakes appear.
- `ClientEvent` enum is currently internal to `willow-client`. Exposing all variants over `wasm_bindgen` may pull in serialization for variants that should remain internal. The `test-hooks` module re-serializes through a stable JSON shape rather than `serde_wasm_bindgen` on the raw enum, so internal variants can be filtered or remapped.
