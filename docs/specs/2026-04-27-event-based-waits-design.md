# Event-Based Waits in Playwright Suite — Design

**Date:** 2026-04-27
**Status:** landed — `test-hooks` cargo feature + WASM API, push dispatcher, `data-state` lifecycle, `Peer` wrapper, ESLint rule, ratchet script, flake harness all shipped across PRs 1–4. Two CI gates from §CI gate (symbol-leak check + flake harness) are wired to local `just check-all` only; CI workflow wiring is a follow-up tracked in the *Realised state* note below.
**Implementation plan:** [`docs/plans/2026-04-27-event-based-waits-pr1-test-hooks-foundation.md`](../plans/2026-04-27-event-based-waits-pr1-test-hooks-foundation.md), [`docs/plans/2026-04-28-event-based-waits-pr1-errata.md`](../plans/2026-04-28-event-based-waits-pr1-errata.md), [`docs/plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md`](../plans/2026-04-29-event-based-waits-pr2-peer-wrapper.md), [`docs/plans/2026-04-30-event-based-waits-pr3-data-state-lifecycle.md`](../plans/2026-04-30-event-based-waits-pr3-data-state-lifecycle.md), [`docs/plans/2026-04-30-event-based-waits-pr4-ratchet-flake-harness.md`](../plans/2026-04-30-event-based-waits-pr4-ratchet-flake-harness.md)
**Branch:** `claude/event-based-waits-RNFZ9`

> **2026-04-28 erratum.** Investigation during PR-1 execution found that
> several API assumptions below are wrong. `ClientHandle` has no
> synchronous DAG read path (everything is actor-mediated), `MemNetwork`
> won't compile on `wasm32`, and the `test-hooks` feature must span
> `willow-client` + `willow-web`. The `WillowTestHooks` pull API methods
> therefore return `js_sys::Promise`, not synchronous values. The
> sections below have been updated; the old shape is preserved in
> `docs/plans/2026-04-28-event-based-waits-pr1-errata.md`.

> **Realised state (post-2026-05 audit).** The four PRs landed and the
> system works as designed for the in-scope flows, but the body below
> drifts from the realised implementation in several places. Diffs:
>
> - **`tab_bar.rs` was excluded** from the `data-state` lifecycle because
>   neither of its candidate elements has a CSS transition; recorded in
>   commit 0f79399 ("docs(web): record tab_bar lifecycle decision"). The
>   lifecycle now applies to four components + the action-sheet overlay,
>   not five. §Scope, §`data-state` attribute pattern Components-
>   receiving-the-lifecycle, and the driving-property list are stale on
>   this point.
> - **Driving-property table fix.** `bottom_sheet.rs` drives on
>   `transform` (its listener accepts both `transform` and `opacity`
>   with `opacity` as the reduced-motion fallback); `grove_drawer.rs`
>   similarly accepts both; the action-sheet markup in `message.rs`
>   drives on `transform`. The body's list is incomplete on these points.
> - **`std::mem::forget` instead of `StoredValue`** for the push
>   dispatcher in `app.rs`. `wasm32`'s process *is* the app, so leaking
>   the dispatcher handle is the simpler correct shape. The `StoredValue`
>   lifecycle paragraph in §Push dispatcher is superseded.
> - **`EventReceiver::subscribe_now`** is used instead of
>   `ClientHandle::subscribe_events()` — `subscribe_now` is the race-free
>   path that avoids the boot-window where async-subscribe would drop
>   boot-time events. Comment at `app.rs:167-169` documents the rationale;
>   the spec's §Push dispatcher phrasing should defer to that.
> - **`clock.ts` exists** as a fourth helper module at
>   `e2e/helpers/clock.ts` (wraps `page.clock.install`/`runFor`). The
>   §Helpers redesign directory listing omits it.
> - **`longPress` did NOT migrate to `page.clock`** — the original
>   `longPress` still uses `waitForTimeout`; an opt-in
>   `longPressWithClock` exists but has no callers. §`page.clock for
>   real durations` step 1 is stale; the migration is deferred.
> - **`waitUntilHeadsEqual` default timeout is 90s, not 30s** (per
>   `e2e/test-hooks.ts:193`). Justification recorded in code: iroh-gossip
>   cold-start cost — the relay log shows ~30s of dial timeouts before
>   the first peer-pair handshake completes. §Wait-until-heads-equal
>   sketch and §Three-patterns-purged item 3 need this reality.
> - **`crates/web/src/test_hooks/` is a directory**, not a file. Four
>   submodules: `mod.rs`, `dispatcher.rs`, `snapshot.rs`, `wire.rs`. The
>   spec's references to `crates/web/src/test_hooks.rs` should be a
>   directory path.
> - **Comprehensive WASM self-tests live in `crates/web/tests/test_hooks_browser.rs`**,
>   not `crates/web/tests/browser.rs` (which only carries the
>   `data-state` lifecycle tests + a single mount sanity test). §PR 1
>   references the wrong file for the bulk of the test set.
> - **`dev-quick` did not get the `FEATURES=""` parameter** that `dev`
>   carries. Spec §`just dev` paragraph implies symmetric
>   parameterisation; not realised. Minor follow-up.
> - **CI gaps.** `scripts/check-no-test-hooks-in-prod.sh` exists and is
>   called from `just check-all`, but no GitHub Actions workflow runs
>   `check-all` or that script directly. Similarly, no path-filtered
>   workflow runs `just test-e2e-flake N=10` on PRs that modify `e2e/`.
>   Both gates exist as local-only today. §CI gate and the PR-4
>   `Migrated specs must pass N=10 in CI` claim describe intent that
>   has not been wired to CI yet. Tracked as follow-up.
>
> The body below is preserved as the original design (corrected only by
> the earlier 2026-04-28 erratum). The *Realised state* list above is
> authoritative for current implementation shape; do not edit the body
> in place to match it — that would lose the design rationale that drove
> the four PRs.

## Problem

The Playwright suite leans on time-based waits as flake compensation. Audit of `e2e/` (8 spec files, 1814 LOC):

- **53** `waitForTimeout(ms)` calls in helpers and specs (200ms–2000ms each).
- **71** `{ timeout: <ms> }` overrides on `expect`/`locator` assertions, including 23 occurrences of `30_000ms`, 8 of `60_000ms`, and 8 of `120_000ms`.
- **3** polling loops that sleep 300ms between iterations, gating on UI visibility rather than driving on a real signal.
- **0** uses of `waitForFunction`, `expect.poll`, `waitForResponse`, or any app-emitted event.

Two consequences: arbitrary sleeps mask race conditions instead of fixing them (per Playwright's own guidance, replacing `waitForTimeout` removes ~45% of flake), and the suite's wall-clock is dominated by sleeps that succeed long before they expire.

The underlying cause is that the web crate exposes nothing for tests to synchronise on — no `#[wasm_bindgen] pub` exports, no `data-testid` attributes, no readiness events. So tests guess at delays. The Willow client *does* already publish a `ClientEvent::SyncCompleted { ops_applied }` after every applied `SyncBatch` (`crates/client/src/listeners.rs:290`); it is not currently visible to JS.

## Goal

Every wait in the Playwright suite gates on a real signal: a DOM state, an applied `ClientEvent`, or a deterministic fake-clock advance. No wait is a guess.

## Scope

**In scope:**
- New cargo feature `test-hooks` on `willow-web`, off in production builds.
- WASM-exported `WillowTestHooks` API (snapshot, heads, event count, last event).
- Push-side instrumentation: WASM dispatches every `ClientEvent` to a Playwright `exposeBinding('__willowEvent', …)` callback.
- TypeScript wrapper `Peer` in `e2e/test-hooks.ts` providing `nextEvent(predicate)`, `snapshot()`, `heads()`, `eventCount()`, `waitUntilHeadsEqual(other)`.
- `data-state="<phase>"` attribute pattern on five animated UI elements (mobile drawer, grove drawer, confirm dialog, bottom sheet, tab bar) plus the action-sheet overlay in `message.rs`, tied to CSS `transitionend` with reduced-motion fallback.
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

The feature spans two crates so the test-only `dag_addr_clone()` accessor can be gated cleanly on `willow-client` (which is depended on by other consumers like `willow-agent` that must NOT see test-hooks symbols):

```toml
# crates/client/Cargo.toml
[features]
test-hooks = []           # NEW — distinct from existing test-utils
```

```toml
# crates/web/Cargo.toml
[features]
default = []
test-hooks = ["dep:serde-wasm-bindgen", "willow-client/test-hooks"]
```

`serde-wasm-bindgen` is gated as an optional dep so the prod build pays no cost. The `test-hooks` feature is intentionally distinct from the existing workspace `test-utils` feature: `test-utils` transitively pulls `MemNetwork`, which uses `tokio::sync::broadcast` and **will not compile on wasm32** (verified `crates/network/src/mem.rs:35`). `test-hooks` is narrow read-only instrumentation that builds clean on both targets.

All new instrumentation code lives behind `#[cfg(feature = "test-hooks")]`. Production `trunk build --release` is unchanged: no exported symbols, no event subscription, no `window.__willow`.

The e2e build switches to `trunk build --features test-hooks`. Just recipes affected: `setup-e2e`, `test-e2e-ui`, `test-e2e-sync`, `test-e2e-perms`, `test-e2e-full`, `test-e2e-flake` (new), `check-all`.

**`just dev`** stays on the default feature set (no test-hooks). Developers who want to poke at `window.__willow` from devtools run `just dev FEATURES=test-hooks`. This requires modifying the `dev`, `setup-e2e`, `test-e2e-*`, and `check-all` recipes to accept an optional `FEATURES` variable that is forwarded to `trunk build`. The justfile changes are scoped explicitly into PR 1 alongside the cargo feature flag — the spec assumes no pre-existing `FEATURES` parameterisation. Default value is empty (prod build); e2e recipes hardcode `FEATURES=test-hooks` internally.

## WASM API surface

New module `crates/web/src/test_hooks/`, gated:

```rust
#![cfg(feature = "test-hooks")]

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use willow_actor::Addr;
use willow_actor::StateActor;
use willow_client::state_actors::DagState;
use willow_client::ClientHandle;
use willow_state::ServerState;

/// Read-only test instrumentation handle. Stores the DAG and ServerState
/// actor addresses (cheaply cloneable). All methods are async because
/// the underlying actor read path is async (`willow_actor::state::select`).
#[wasm_bindgen]
pub struct WillowTestHooks {
    dag_addr: Addr<StateActor<DagState>>,
    state_addr: Addr<StateActor<ServerState>>,
}

impl WillowTestHooks {
    /// Construct from any `ClientHandle<N>`. Captures the actor addresses
    /// so the wasm_bindgen-exposed methods stay monomorphic.
    pub fn new<N: willow_network::Network>(handle: &ClientHandle<N>) -> Self {
        Self {
            dag_addr: handle.dag_addr_clone(),         // gated test-hooks
            state_addr: handle.event_state_addr_clone(),// gated test-hooks
        }
    }
}

#[wasm_bindgen]
impl WillowTestHooks {
    /// Aggregated state snapshot for `expect.poll` matchers.
    /// Resolves to: { eventCount, heads: { authorIdHex: { seq, hash }, ... },
    ///                 lastEvent: string | null, channels: [{ name, kind }] }
    pub fn snapshot(&self) -> js_sys::Promise;

    /// Per-author DAG heads. Resolves to Record<authorIdHex, { seq, hash }>.
    pub fn heads(&self) -> js_sys::Promise;

    /// Total events applied to local DAG. Resolves to a number.
    pub fn event_count(&self) -> js_sys::Promise;

    /// Hex hash of the most recently applied event (Display-formatted, 64
    /// chars), or null. Resolves to string | null.
    pub fn last_event(&self) -> js_sys::Promise;
}
```

All read methods return `js_sys::Promise`. The actor-ask round-trip in WASM is sub-millisecond mailbox dispatch — fine for `expect.poll` tick rates. JS callers `await` the promise:

```ts
const count = await window.__willow.event_count();   // number
const snap  = await window.__willow.snapshot();      // Snapshot
```

The TypeScript `Peer` wrapper hides the await from test authors.

`ClientHandle` exposes two new sync getters under `#[cfg(feature = "test-hooks")]`:

```rust
// crates/client/src/accessors.rs
#[cfg(feature = "test-hooks")]
impl<N: Network> ClientHandle<N> {
    pub fn dag_addr_clone(&self)         -> Addr<StateActor<DagState>>     { self.dag_addr.clone() }
    pub fn event_state_addr_clone(&self) -> Addr<StateActor<ServerState>> { self.event_state_addr.clone() }
}
```

These are the only client-side surface additions. Both are sync (just clone an `Addr`) and behind the same feature gate. They are NOT visible to non-test consumers of `willow-client`.

Push side, in the same module:

```rust
/// Subscribes to `client.subscribe_events()` and dispatches every
/// wire-visible `ClientEvent` to `window.__willowEvent` (a Playwright
/// binding). If the binding is not yet wired, events accumulate in a
/// page-local buffer (`window.__willowEventBuffer`) and are drained by
/// the binding on first call.
///
/// Capacity: 65_536 (per-page; never shared across peers). On overflow
/// the dispatcher calls `window.__willowOverflow(droppedCount)` if
/// defined and emits an error to the console. Test fixtures install
/// `__willowOverflow` and fail the test on any call — overflow is
/// always a correctness bug, not backpressure.
///
/// Lifecycle: returns a `DispatcherHandle` that aborts the spawned
/// loop on drop. `app.rs` stores it in a Leptos `StoredValue` keyed
/// to the same lifetime as the `ClientHandle`. Re-init replaces the
/// handle (drop aborts the previous loop). No leak on hot reload.
pub fn install_push_dispatcher(client: ClientHandle) -> DispatcherHandle;
```

Mounted from `app.rs` immediately **after** the `with_trust_store` clone (so the same handle the UI uses is captured), behind the same `cfg`:

```rust
#[cfg(feature = "test-hooks")]
{
    let hooks = test_hooks::WillowTestHooks::new(client_handle.clone());
    js_sys::Reflect::set(&window, &"__willow".into(), &hooks.into()).unwrap();
    let dispatcher = test_hooks::install_push_dispatcher(client_handle.clone());
    // Bind the handle so it lives for the component's scope. `StoredValue`
    // does not drop until its owning Leptos scope is disposed; binding
    // its return value (rather than discarding it) ensures the dispatcher
    // loop runs for the app's lifetime. Without the binding, the value
    // would be dropped at end of expression and the loop would abort.
    let _dispatcher_handle = leptos::StoredValue::new(dispatcher);
}
```

The pull API does **not** serialize `HeadsSummary` directly. Instead, `test_hooks` defines a web-only DTO `SnapshotDto` that reads from the materialised `ServerState` (held by the client) and the DAG `HeadsSummary` (`crates/state/src/sync.rs:22`, with `AuthorHead { seq, hash }` per author at `:28`). The DTO is `#[serde(rename_all = "camelCase")]` so the JS-side shape matches the TypeScript `Snapshot` interface without modifying the state crate.

The push dispatcher reuses `ClientHandle::subscribe_events()` (`crates/client/src/accessors.rs:10`), which returns an `EventReceiver` (`crates/client/src/lib.rs:120`) — a custom actor-broker forwarder, not a `futures::Stream`. The dispatcher spawns a `wasm_bindgen_futures::spawn_local` task that loops on `rx.recv().await`, converts each `ClientEvent` to the stable JSON wire shape (see below), and dispatches via `window.__willowEvent`. No new emit points are added inside `willow-client` or `willow-state`.

### Stable JSON wire shape for `ClientEvent`

The Rust `ClientEvent` enum (`crates/client/src/events.rs:19`) has 30+ variants mixing tuple-style (`PeerConnected(EndpointId)`, `ChannelCreated(String)`) and struct-style (`SyncCompleted { ops_applied }`). Default serde would produce inconsistent JSON shapes per variant.

`test_hooks` defines a stable wire shape `{ kind: <PascalCaseName>, ...flattened fields in camelCase }` and hand-writes the conversion. The `kind` discriminator stays PascalCase (matches Rust variant names); all other field names are camelCase (matches TypeScript convention). Tests see only the variants the suite cares about today; new variants are added explicitly. Initial wire-visible set:

- `{ kind: "SyncCompleted", opsApplied: number }`
- `{ kind: "MessageReceived", channel: string, messageId: string, isLocal: boolean }`
- `{ kind: "PeerConnected", peerId: string }`
- `{ kind: "PeerDisconnected", peerId: string }`
- `{ kind: "ChannelCreated", name: string }`
- `{ kind: "ChannelDeleted", name: string }`
- `{ kind: "PeerTrusted", peerId: string }`
- `{ kind: "PeerUntrusted", peerId: string }`
- `{ kind: "ProfileUpdated", peerId: string, displayName: string }`
- `{ kind: "RoleCreated", roleId: string, name: string }`

Internal-only variants (`QueueChanged`, `VoiceSignal`, etc.) are **not** dispatched to the test side — the dispatcher filters them out. This keeps the test surface narrow and stable across internal client changes.

## Playwright wrapper — `Peer`

New file `e2e/test-hooks.ts`:

```ts
// Mirror of the wire-visible subset of willow-client's ClientEvent.
// Hand-maintained against test_hooks' conversion table; codegen is a
// follow-up if drift becomes painful.
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

export interface AuthorHead {
  seq: number;
  hash: string;
}

export interface Snapshot {
  eventCount: number;
  /// Per-author heads. Keys are EndpointId hex strings.
  heads: Record<string, AuthorHead>;
  lastEvent: string | null;
  /// Channels in the materialised ServerState.
  channels: Array<{ name: string; kind: string }>;
}

export class Peer {
  constructor(public readonly page: Page, public readonly label: string);

  /** Drain the next event matching `predicate` from the per-page push queue. */
  async nextEvent(
    predicate: (e: ClientEvent) => boolean,
    opts?: { timeout?: number },
  ): Promise<ClientEvent>;

  async snapshot(): Promise<Snapshot>;
  async heads(): Promise<Record<string, AuthorHead>>;
  async eventCount(): Promise<number>;

  /** Wait until this peer's heads equal `other`'s heads. Uses expect.poll. */
  async waitUntilHeadsEqual(
    other: Peer,
    opts?: { timeout?: number },
  ): Promise<void>;

  /** Wait until this peer's heads equal each peer in `others`. */
  async waitUntilAllHeadsEqual(
    others: Peer[],
    opts?: { timeout?: number },
  ): Promise<void>;
}
```

Per-page push queue is set up in a Playwright fixture. The order is critical:

1. `context.exposeBinding('__willowEvent', cb)` registers the binding.
2. `context.exposeBinding('__willowOverflow', cb)` registers an overflow hook that fails the test on call.
3. `page.addInitScript(...)` installs the JS-side buffer that the WASM dispatcher writes into when the binding is not yet present (a defensive guard for the narrow window before the page's `__willowEvent` proxy is bound).
4. `page.goto(...)` — only after all three.

```ts
test.beforeEach(async ({ page, context }) => {
  const queue: ClientEvent[] = [];
  await context.exposeBinding('__willowEvent', (_src, ev: ClientEvent) => {
    // Drain on read side too: pulls anything the WASM dispatcher pushed
    // into the buffer while the binding was momentarily absent (hot
    // reload, dispatcher restart, etc.). Drain on read covers the case
    // where no further event arrives to trigger a write-side drain.
    queue.push(ev);
  });
  await context.exposeBinding('__willowOverflow', (_src, dropped: number) => {
    throw new Error(`__willow event queue overflow: ${dropped} events dropped`);
  });
  await page.addInitScript(() => {
    // Buffer for events the WASM dispatcher emits before `__willowEvent`
    // is callable. Defence-in-depth: under normal Playwright ordering
    // (exposeBinding before goto) the buffer is empty; under restart /
    // hot-reload it covers the gap.
    (window as any).__willowEventBuffer = [];
  });
  // Peer construction stores `queue` for nextEvent to drain.
});
```

The WASM dispatcher (`test_hooks::install_push_dispatcher`) implements the drain on **two** edges:

```rust
// On dispatcher init: drain anything left from a prior dispatcher.
if let Some(fn_) = window.get("__willowEvent") {
    if let Some(buf) = window.get("__willowEventBuffer") {
        for buffered in drain_array(buf) { fn_.call(buffered); }
    }
}

// Per-event:
let event_js = serialize_event(&event);
match window.get("__willowEvent") {
    Some(fn_) => {
        // Drain buffer first (covers the case where binding became
        // available between events).
        if let Some(buf) = window.get("__willowEventBuffer") {
            for buffered in drain_array(buf) { fn_.call(buffered); }
        }
        fn_.call(event_js);
    }
    None => push_to_buffer(event_js),
}
```

The combination — **drain on dispatcher init, drain on every dispatch, plus the Playwright fixture's read-side drain on each binding invocation** — closes the stale-buffer hazard in all three failure modes (dispatcher restart, binding-present-but-no-new-events, hot reload).

`waitUntilHeadsEqual` uses `expect.poll` with default intervals `[100, 250, 500, 1000]` and a 30s timeout. Heads are serialized with sorted keys so equality is engine-independent:

```ts
function canonical(heads: Record<string, AuthorHead>): string {
  return JSON.stringify(
    Object.keys(heads).sort().map(k => [k, heads[k].seq, heads[k].hash]),
  );
}

async waitUntilHeadsEqual(other: Peer, opts?: { timeout?: number }) {
  const target = canonical(await other.heads());
  await expect.poll(
    async () => canonical(await this.heads()),
    {
      timeout: opts?.timeout ?? 30_000,
      message: `${this.label} converge with ${other.label}`,
    },
  ).toBe(target);
}

async waitUntilAllHeadsEqual(others: Peer[], opts?) {
  for (const other of others) await this.waitUntilHeadsEqual(other, opts);
}
```

**Naming caveat.** `waitUntilHeadsEqual` is named for what it verifies, not the abstract concept of "convergence." Two peers can be heads-equal yet both still missing an event from a third peer C — the assertion does not protect against that. Tests that need true N-peer convergence call `waitUntilAllHeadsEqual([peerA, peerB, peerC])` to verify every peer has reached the same head set, which is the standard CRDT-suite check (head-set equality across all observed peers). Single-peer tests, or two-peer tests where the peer pair is the entire universe, can use `waitUntilHeadsEqual` directly.

**Partial-equality footgun.** If peer A's head set and peer C's head set name *different* author keys (A has `{x, y}`, C has `{x, y, z}`), `waitUntilHeadsEqual` will hang until timeout because canonical equality requires identical key sets. The timeout error message includes a structured author-key diff (`A missing authors: [z]; C missing authors: []`) so the failure is debuggable without a manual `console.log` round-trip. Required for the helper to be usable in mixed-membership tests.

## `data-state` attribute pattern

For animated UI elements with a settling phase, the component sets a `data-state` attribute reflecting the transition phase and tests gate on the attribute rather than sleeping. This is the canonical replacement for every `waitForTimeout` after a UI transition.

**Lifecycle states.** `closed`, `opening`, `open`, `closing`. The `opening`/`closing` phases are set imperatively when the transition starts; `open`/`closed` are flipped on `transitionend`.

**Three failure modes that must be handled** to prevent false flake:

1. **`prefers-reduced-motion: reduce`** — CSS transition is zeroed; `transitionend` may not fire. The component reads `getComputedStyle(el).transitionDuration`; if `0s`, the terminal state is set synchronously without waiting for the event.
2. **Component unmount mid-transition** — Leptos cleanup must abort the pending state. The signal is owned by the component and is dropped with it; test simply observes the locator detach.
3. **Overlapping transitions** — only the last `transitionend` fires reliably for a given property. The component listens for the *specific* property (`transitionend` filtered by `event.propertyName === <component's driving property>`). Each of the five components documents its driving property in a comment at the listener site (`mobile_shell.rs` and `grove_drawer.rs` use `transform`; `confirm_dialog.rs` and `bottom_sheet.rs` use `opacity`; `tab_bar.rs` uses `transform`). A browser test asserts the chosen property is what advances `data-state` — guards against future CSS edits silently breaking the lifecycle.

```rust
// crates/web/src/components/grove_drawer.rs (illustrative)
let state = RwSignal::new("closed");

let advance = move || {
    state.set(match state.get_untracked() {
        "opening" => "open",
        "closing" => "closed",
        other => other,
    });
};

let on_transition_end = move |ev: web_sys::TransitionEvent| {
    if ev.property_name() == "transform" {
        advance();
    }
};

let trigger_open = move |_| {
    state.set("opening");
    // Reduced-motion shortcut: if the element has zero-duration
    // transition, skip the wait and snap to terminal state.
    if computed_transition_duration_is_zero(&drawer_ref) {
        advance();
    }
};
```

Tests:

```ts
await openSidebarBtn.click();
await expect(drawer).toHaveAttribute('data-state', 'open');
```

**Components receiving the lifecycle.** `mobile_shell.rs` (mobile drawer), `grove_drawer.rs`, `confirm_dialog.rs`, `bottom_sheet.rs`, `tab_bar.rs` (active-tab transition). The mobile action sheet markup lives in `message.rs`; the `data-state` attribute is added to the existing `mobile-action-sheet-overlay` div there. Five physical edits.

**Existing `data-state` usages on `status_dot.rs` and `grove_rail.rs` are NOT brought into the four-phase lifecycle.** They use `data-state` for orthogonal categorical states (`online`/`offline`, etc.). The lifecycle contract applies only to the listed five animated components. The shared attribute name is a convention; tests that gate on it must know which component they target. `e2e/README.md` documents this distinction.

## `page.clock` for real durations

Three legitimate real-duration waits exist today and are migrated to `page.clock`:

1. **`longPress(locator, duration)`** in `e2e/helpers.ts:264`. Today: `mouse.down()` + `waitForTimeout(duration)` + `mouse.up()`. New: `clock.runFor(duration)` between down and up. Test must `await page.clock.install()` in the relevant `beforeEach`.
2. **Debounce timers in the input box** (typing-indicator throttle, message-edit autosave). Tests that exercise these flows install the clock and `runFor` past the debounce window.
3. **HLC drift simulation** (no current test, but a likely future need): `clock.setFixedTime(date)` per peer.

**API correction.** Playwright's clock is `page.clock.install()`, not `context.clock.install()`. The clock is per-page (which means per-`BrowserContext` in the common one-page-per-context Playwright fixture, but the call goes through the page).

**Multi-peer caveat.** `page.clock` is per-page. Two peers in the same Playwright test run in two separate `BrowserContext`s (and therefore two separate pages), so each peer can install an independent clock and the drift simulation works. Tests that share a context across peers (rare in this suite but possible) cannot use independent clocks; the spec's two-peer fixture (`setupTwoPeers`) creates two contexts and is unaffected.

**WASM coverage.** `page.clock` patches the JS `Date`/`setTimeout`/`setInterval`/`requestAnimationFrame` globals. It does **not** patch `performance.now()`. Willow's WASM HLC reads time via `js_sys::Date::now()` (`crates/messaging/src/hlc.rs:96`), which calls the patched global, so HLC behaviour is controlled by the fake clock when installed. **Native** HLC (`hlc.rs:87`) uses `SystemTime::now()` directly and is unaffected — and is exercised in Rust tests, not Playwright, so this is a non-issue for the e2e suite.

**iroh timer verification (PR 1 acceptance gate).** Before merging PR 1, a one-off audit checks whether iroh's WASM transport uses `performance.now()` for retry backoff or gossip heartbeats. Method: `git grep -n 'performance\|Performance::now\|web_sys::window().*performance' iroh*`-vendored sources used by `willow-network`. If iroh uses `performance.now`, installing `page.clock` would freeze UI/HLC time but iroh would keep running on real time — silent divergence. In that case `page.clock` install is constrained to scopes where iroh activity is irrelevant (longPress within a single peer, debounce within a stable connection). The audit result and the resulting constraint are recorded in PR 1's description and inlined back into this section once known.

**Opt-in.** Clock install is per test (or `describe` block), not global. Default e2e tests run with real time so iroh background timers (gossip heartbeats, retry backoff) are unaffected.

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
3. `{ timeout: 30_000 }` overrides on `toBeVisible` for cross-peer assertions (23 occurrences) → `await peer.waitUntilHeadsEqual(other)` before the assertion, then default 5s timeout.

**Targeted exception rule.** A small number of waits may have no event-based equivalent (e.g. a CSS transition on a third-party component without `transitionend`). These require a `// time-wait: <reason>` comment plus a per-line `eslint-disable-next-line` referencing the rule and the tracking issue. No blanket allowlist.

## Implementation phasing

The work is decomposed into four sequential PRs against the `claude/event-based-waits-RNFZ9` branch (or successor branch). Each PR is independently reviewable and ships green CI.

**PR 1 — `test-hooks` feature + WASM API + push dispatcher.**
- `crates/web/Cargo.toml` feature flag.
- `crates/web/src/test_hooks.rs` (`WillowTestHooks`, `SnapshotDto`, `install_push_dispatcher`).
- Mount in `app.rs` post-`with_trust_store`.
- `crates/web/tests/browser.rs` self-tests for `snapshot()`, `heads()`, `event_count()`, `last_event()`.
- `just check-all` symbol-leak grep additions.
- Smoke test: nothing in e2e converted yet; just verifies `window.__willow` exists in the e2e build and not in the prod build.

**PR 2 — Playwright `Peer` wrapper + helpers split + first pilot.**
- `e2e/test-hooks.ts` with `Peer`, `Snapshot`, `ClientEvent` types and the fixture.
- `e2e/helpers.ts` → `e2e/helpers/{peers,ui,touch}.ts`. Magic numbers stripped where the equivalent locator-or-event wait suffices.
- `e2e/test-hooks.spec.ts` smoke tests for the wrapper.
- Pilot: `e2e/multi-peer-sync.spec.ts` converted.
- Other 7 specs continue to use the legacy import paths through a shim (`e2e/helpers.ts` becomes a re-export barrel). No semantic change to un-migrated specs. PR 2 includes an exhaustive enumeration of legacy exports (every name imported by any spec under `e2e/*.spec.ts`) verified by a TypeScript test that imports each one through the barrel; missing exports fail the build.

**PR 3 — `data-state` lifecycle on the five animated components.**
- Lifecycle on `mobile_shell.rs`, `grove_drawer.rs`, `confirm_dialog.rs`, `bottom_sheet.rs`, `tab_bar.rs` and the action-sheet markup in `message.rs`.
- Reduced-motion fallback per the spec section.
- Browser tests in `crates/web/tests/browser.rs` covering reduced-motion and unmount cases.

**Lint window note.** The ESLint rule lands in PR 4, leaving PRs 1–3 unprotected against new `waitForTimeout` calls. To narrow the window: PR 1 also adds the ESLint rule and a `// eslint-disable` header on every existing spec referencing the tracking issue. The full ratchet script + baseline file ship with PR 4 since the baseline value depends on the post-pilot count. This way the rule blocks new offences from PR 1 onward while the count-based ratchet starts when there's a stable count to ratchet against.

**PR 4 — Ratchet script + flake harness + cleanup.**
- `scripts/check-wait-timeout-count.sh` + `e2e/.wait-timeout-baseline` (initial value: post-pilot count, computed at PR-4 land time; script also enforces sunset cutoff).
- `just test-e2e-flake N=5` recipe.
- Removal of any `eslint-disable` headers from specs migrated in PRs 2–3.

The tracking issue is opened **before PR 1** so the URL is stable for `eslint-disable` references in PR 4. The issue's body lists the 7 un-migrated specs, the sunset date (2026-09-30), and links to this spec.

## Pilot conversions

Two pilots ship in PR 2 so the API is exercised under real load:

1. **`e2e/helpers.ts` → `e2e/helpers/{peers,ui,touch}.ts`.** Highest leverage: every spec re-imports through the new modules. Validates the `data-state` pattern (UI helpers) and `page.clock` (touch helpers) end-to-end.
2. **`e2e/multi-peer-sync.spec.ts`.** Worst gossip-pad offender: 30s timeout overrides on every cross-peer assertion plus two `waitForTimeout(2000)` calls. Validates `Peer.nextEvent` and `Peer.waitUntilHeadsEqual` in their dominant use case.

Acceptance for **both** pilots: `just test-e2e-flake N=10` must pass with zero failures, measured in CI. Wall-clock for `multi-peer-sync.spec.ts` should drop measurably (current ~45s wall-clock per local run, sampled best-of-3; target <20s, sampled the same way once gossip-pads are removed). The hard requirement is zero flake; speed is a follow indicator.

## Tracking issue

A GitHub issue `e2e: migrate remaining specs to event-based waits` is opened **before PR 1 lands**, so its URL is stable and can be cited from `eslint-disable` headers added in PR 4. Body lists each remaining file as a checklist plus the 2026-09-30 sunset date:

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

1. `trunk build --release` (no features) → grep the resulting `dist/*.js` (the wasm-bindgen-emitted JS shim, which retains class names regardless of `wasm-opt` symbol stripping in the `.wasm`) for `WillowTestHooks`. Must not find. Fails CI if leaked. Catches accidental `default = ["test-hooks"]` regressions. As a defence-in-depth check, also runs `wasm-objdump --section=name dist/*.wasm | grep -q WillowTestHooks` and asserts no match.
2. `trunk build --features test-hooks` → grep `dist/*.js` for `WillowTestHooks`. Must find. Sanity check the gating actually compiles.

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

**Ratchet.** A small `scripts/check-wait-timeout-count.sh` counts `waitForTimeout` occurrences in `e2e/`, **excluding** lines tagged with the `// time-wait:` exemption marker, compares to a baseline file `e2e/.wait-timeout-baseline`, and fails CI if the count increases. Decreases update the baseline (manually committed; not auto-rewritten). The exemption-marker carve-out is the escape hatch for legitimate real-duration waits that no event can replace; each exemption requires a justification comment and survives review.

**Sunset.** The ESLint allowlist and ratchet baseline are scheduled for removal by **2026-Q3** (concretely: by 2026-09-30). The ratchet script enforces this: after the cutoff date it requires the baseline to be `0` and exits non-zero otherwise, regardless of whether the baseline file still exists. On 2026-09-30 the rule flips to hard-fail at 0, the baseline file is deleted, and any remaining `// time-wait:` exemptions stand on their justification alone. The sunset date is recorded in the tracking issue. If migration is incomplete by then, a brief written extension (PR amending this spec and updating the script's date constant) is required.

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
| Push queue grows unbounded if a test forgets to drain | Bounded buffer (capacity 65 536) inside the WASM dispatcher. Overflow is a hard error: dispatcher calls `window.__willowOverflow(droppedCount)`, which the test fixture wires to fail the test immediately. Overflow is treated as a correctness bug, not backpressure. |
| `exposeBinding` registered after the app boots → first events dropped | `exposeBinding` is registered in `beforeEach` *before* `goto`, so the binding is normally present when WASM dispatches. As defence-in-depth, `addInitScript` declares an empty `__willowEventBuffer`; the dispatcher checks for `__willowEvent` on the window and falls back to pushing into the buffer if absent. The first `__willowEvent` call drains the buffer. |
| `page.clock` interferes with iroh's WASM timers (gossip heartbeats, retry backoff) | Clock install is opt-in per test; default e2e tests run with real time. Tests that install the clock are explicit about which timers they advance. |
| `data-state` attributes proliferate across markup | Only on elements tests gate on (drawer, modal, dropdown, tab, action sheet — finite, listed above). Documented in the `e2e/README.md`. |
| Cargo feature drift: e2e build differs from prod build behaviour | `#[cfg]` only adds inert mounting code; no behaviour change. Validated by running existing `just test-browser` once with `--features test-hooks` and asserting the same pass set. |
| Migration stalls; ESLint allowlist becomes permanent | Tracking issue + ratchet script (count must monotonically decrease). Renewed on every file migration PR. |
| `ClientEvent` schema drifts between Rust and TS `Peer` wrapper | TS type kept in `e2e/test-hooks.ts` with a `// keep in sync with crates/client/src/lib.rs ClientEvent` marker. A small Rust integration test serializes every `ClientEvent` variant and asserts the TS type covers it (codegen check is a follow-up if drift becomes painful). |

**Runner-up rejected — DOM-only (`data-testid` + attribute polling, no WASM API).** Rejected for the *state-convergence* bucket because gossip convergence is non-DOM: peer B can apply role/permission events that change no UI, but downstream assertions still depend on those events being applied. The `data-state` lifecycle pattern is essentially the DOM-only approach for the *DOM-settle* bucket — that's intentional and not a contradiction; the rejection is scoped to the convergence bucket only.

**Runner-up rejected — always-on `window.__willow` API (no cargo feature).** Rejected on privacy: third-party JS in production could read DAG heads and event counts. Cost of the feature gate is one cargo feature, one CI line, and one symbol-leak grep — small and one-time.

**Runner-up rejected — keep `waitForTimeout` and just lengthen.** Rejected because longer sleeps mask the race rather than fix it; the suite still spends real wall-clock waiting after the system has settled, and any new flake source raises the timeout further. The flake is an information-theoretic problem (no signal) and time can't substitute.

**Runner-up rejected — Rust-driven test harness in place of Playwright.** Rejected because the e2e tier exists specifically to validate real iroh + browser behaviour, and that fidelity is exactly what the lower tiers cannot cover. The `MemNetwork` already serves the Rust-driven multi-peer testing role at the `client` tier (per `docs/specs/2026-04-21-e2e-test-architecture-design.md`).

**Runner-up rejected — synchronous iroh mock in tests.** Rejected for the same reason: `MemNetwork` already exists for the Rust tier; e2e exists because the network-effect path matters.

## Testing the test infrastructure

**WASM-side.** New tests in `crates/web/tests/browser.rs` (gated `#[cfg(feature = "test-hooks")]`) construct a `ClientHandle`, apply known events, and assert:
- `snapshot()`, `heads()`, `event_count()`, `last_event()` return the expected shape and values.
- Bounded-buffer overflow path: synthetic stress that pushes 65 537 events triggers the overflow callback exactly once.
- Buffer drain on first binding call: events queued before binding wiring are delivered in order on first call.
- `data-state` lifecycle: reduced-motion shortcut sets terminal state synchronously; component unmount mid-transition does not leak state.

**Playwright-side.** New `e2e/test-hooks.spec.ts` smoke-tests the `Peer` wrapper:
- `peer.snapshot()` returns the expected fields after a fresh start.
- `peer.eventCount()` equals 1 after `CreateServer`.
- `peer.nextEvent(e => e.kind === 'SyncCompleted')` resolves on the first heartbeat, and rejects with a clear error after `opts.timeout` if no matching event arrives.
- `peer.waitUntilHeadsEqual(self)` is a no-op (single peer trivially converges).
- Three-peer test: `waitUntilAllHeadsEqual([peerB, peerC])` blocks until *all* three peers reach the same head set.

**Pilot acceptance.** `multi-peer-sync.spec.ts` is run 10× via `just test-e2e-flake N=10` and `helpers/*` exercised through every spec it underpins. Both must pass with zero failures in CI. The "before" baseline (legacy 30s-timeout version) is captured in the same PR via a one-time CI run on the parent commit; numbers recorded in the PR description for review.

## Cross-references

- [`docs/specs/2026-04-21-e2e-test-architecture-design.md`](./2026-04-21-e2e-test-architecture-design.md) — three-tier test pyramid; this spec slots into Tier 3 (Playwright) and references the "rewrite trigger" rule for migrating tests downwards when selectors drift.

## Open questions

- Do we need a `Peer.events()` accessor that returns the full applied-event log (filtered by predicate)? Probably yes for permissions tests; defer until those migrate.
- Should the `data-state` attribute pattern be lifted into a shared Leptos helper component to enforce consistency? Defer; first apply the pattern manually to the listed components and refactor if duplication accumulates.
- WebSocket-frame-level waits via `page.waitForEvent('websocket')` are powerful for relay tests but couple to wire format. Out of scope for this spec; revisit if relay-specific flakes appear.
- Auto-generation of the TS `ClientEvent` mirror from the Rust enum (e.g. via `ts-rs`). Cheap to add later; not scoped here.
