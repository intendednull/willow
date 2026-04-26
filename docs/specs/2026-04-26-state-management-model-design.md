# State Management Model Design

**Date:** 2026-04-26
**Status:** draft
**Branch:** `audit/lock-actor-alignment`

## Problem

Willow has a clean actor framework (`willow-actor`) and a clear architectural intent: shared mutable state lives inside an actor; everything else is message passing, derived state, or one-shot init. But the codebase has drifted. A focused audit found 6 hotspots in library and web crates where lock-based state (`Arc<RwLock<_>>`, `Arc<Mutex<_>>`, `parking_lot`) protects business state that should sit inside an actor. Several of those hotspots use a multi-lock pattern that hand-rolls atomicity — exactly what `StateActor` was built to avoid.

Concrete examples:

- `crates/client/src/search/handle.rs:29-32` — `SearchIndexHandle` wraps four logically related fields (index, config, recents, build status) in four independent `parking_lot::Mutex`es. No cross-field atomicity. Updating "build finished + recents bumped" needs careful lock ordering or it's racy.
- `crates/client/src/nickname.rs:39-40` — `MemNicknameStore` carries an `RwLock<HashMap>` cache plus a separate `RwLock<u64>` version counter. The version exists to drive change notification. `StateActor` already provides version-bumped `Notify` for free.
- `crates/client/src/trust.rs:149` — `InMemoryState` behind a `std::sync::Mutex`. Same shape as the others.
- `crates/client/src/lib.rs:214,254,558` — `ClientHandle` carries `Arc<RwLock<HashMap<_,Topic>>>` (`topics`), `Arc<parking_lot::Mutex<Vec<JoinLink>>>` (`join_links`), and `Arc<std::sync::Mutex<MessageDb>>` (`message_db`). The first two are mutated from listeners, mutations, and UI; `message_db` is dead weight (PersistenceActor owns persistence).
- `crates/web/src/trust_store.rs`, `crates/web/src/profile/nickname_store.rs`, `crates/web/src/state_bridge.rs` — `Mutex`/`RwLock` for component-local state in a Leptos app where signals are the natural primitive.

Underlying cause: there is no written rule. Contributors (human or agent) face a state-management decision and have to reverse-engineer the pattern from neighbouring code. Neighbours include both the right answer (`StateActor` in `crates/worker/`) and the wrong answer (`Arc<Mutex<_>>` in `crates/client/src/lib.rs`). Without a rule, the wrong answer keeps spreading.

## Goal

Every shared-mutable-state decision in the codebase has an obvious, documented answer. The decision is encoded in CLAUDE.md so every agent session sees it. Existing misalignments are fixed in the same PR. Legitimate exceptions (iroh callback boundary, OnceLock init, single-threaded WASM `RefCell`) are explicitly documented inline, not buried.

## Scope

- **In scope:** Written rule + decision tree in `CLAUDE.md` and this spec. Refactor of all 6 audit hotspots. Inline `// state: lock-ok — <reason>` comments on every legitimate remaining lock. Test coverage at the new actor boundaries (client tier).
- **Out of scope:** Mechanical CI gate (rejected — too many false positives, agent-context awareness preferred). Changes to the actor framework itself. Changes to iroh layer locks (documented as boundary). Changes to native worker runtime. Performance optimisation. New features.

## Architecture

### The rule

> **Shared mutable state in library crates lives inside an actor.** No `Arc<Mutex<T>>` / `Arc<RwLock<T>>` / `parking_lot::*` for business state. The actor owns the data; consumers send messages.

### Decision tree

```
Need shared mutable state?
├─ Default                                  → StateActor<S> or bespoke actor
│                                              (use bespoke when S is non-Clone
│                                               or when domain-rich messages
│                                               read clearer than closure-based
│                                               mutate; see SearchActor for an
│                                               example)
│
├─ External-callback boundary (iroh)?       → Lock OK
│                                              // state: lock-ok — iroh callback
│                                              //   delivers events outside actor loop
│
├─ Sync trait abstraction over small        → Single Mutex<Inner>
│  in-memory cache (legacy)?                   // state: lock-ok — sync trait API,
│                                              //   trait elimination tracked in
│                                              //   spec § Follow-up work
│
├─ One-shot init of static data?            → OnceLock<T> / LazyLock<T>
│                                              (regex compilation, topic IDs)
│
├─ Cross-task control flag (stop/cancel)?   → AtomicBool / AtomicU32
│                                              (mailbox stop, supervisor cancel)
│
├─ Single-threaded WASM interior mut?       → Rc<RefCell<T>>
│                                              (web crate only — voice nodes,
│                                               WebRTC closures, audio analysers)
│
├─ Reactive UI state in web?                → Leptos signal
│                                              (RwSignal, Resource, Memo)
│                                              StateActor only when state mutated
│                                              from non-Leptos context (background
│                                              task, timer spanning components,
│                                              MCP, async coalescing).
│
└─ Coordination signal between actors        → tokio::sync::watch / oneshot /
   (ready, cancel, single-value broadcast)?    broadcast / Notify
                                              (control flow, not shared mutable
                                               state — never `tokio::sync::Mutex`
                                               for business state; same rule as
                                               `std`/`parking_lot` Mutex.)
```

### Why StateActor by default

The `willow-actor` `StateActor<S>` already solves every recurring problem the lock-based stores hand-roll:

- **Atomicity across fields** — one mutation sees a consistent `S`. No lock ordering.
- **Change notification** — `Notify` on every mutation, version-bumped, batched in `idle()`. Subscribers don't poll.
- **Cheap reads** — state held as `Arc<S>`; readers clone the `Arc`, get a snapshot, no contention.
- **Copy-on-write mutations** — `Arc::make_mut` for in-place updates without cloning unmodified state.
- **Single-task ownership** — no concurrent mutator. No data races by construction.
- **Dual-target** — works on native (`tokio::spawn`) and WASM (`spawn_local`) without per-call gates.

A hand-rolled `Arc<Mutex<HashMap>> + Arc<Mutex<Version>>` reproduces the first three points poorly and skips the rest.

### Where actors live

- **State machine** (`crates/state/`) — pure, no I/O, no actors. Events in, state out.
- **Client lib** (`crates/client/`) — actors own all shared mutable state. `ClientHandle` is a thin facade holding actor `Addr`s. No business state on the handle struct itself.
- **Workers** (`crates/worker`, `crates/replay`, `crates/storage`, `crates/relay`, `crates/agent`) — already actor-based via `WorkerRole`. Stay actor-based.
- **Network** (`crates/network/`) — `IrohNetwork` is the iroh boundary. Locks here are forced by iroh's external-callback delivery model. Documented exception, not aspirational target.
- **Web** (`crates/web/`) — Leptos signals are the primitive. `StateActor` allowed only where state mutated from outside reactive scope. Bridge to client actors via existing `state_bridge`/`Resource` pattern.

### Why no hard CI gate

A grep gate (`! rg 'Arc<.*Mutex'`) is bulletproof for new violations but generates false positives the moment we touch the iroh layer or want a legitimate `Arc<AtomicBool>`. Allowlist comments turn into noise. A custom clippy lint costs ongoing maintenance against an unstable plugin API. We picked agent-context awareness instead: the rule is in `CLAUDE.md`, every agent session loads it, every PR review reasons against it. If drift recurs, we revisit.

## Refactor plan

Six hotspots from the audit. All ship in this PR.

### 1. `SearchIndexHandle` → bespoke `SearchActor`

`crates/client/src/search/handle.rs:29-32`

Today: four `Arc<parking_lot::Mutex<_>>` for `index`, `config`, `recents`, `status`.

After: bespoke `SearchActor` (in `crates/client/src/search/actor.rs`) owning all four fields as one unit; handle wraps `Addr<SearchActor>`. Cross-field updates become atomic via mailbox serialization.

```rust
pub struct SearchActor { /* index + config + recents + status + persist */ }
pub struct SearchIndexHandle { addr: Addr<SearchActor> }
```

**Bespoke actor over generic `StateActor<S>`**: `SearchIndex` is intentionally not `Clone` (large inverted index, deep copy is wasteful). `StateActor::Mutate` requires `S: Clone` for copy-on-write, so a generic state actor doesn't fit. Bespoke message types (`Insert`, `Rebuild`, `Query`, `RemoveMessage`, `RemoveChannel`, `RemoveGrove`, `GetConfig`, `SetConfig`, `GetRecents`, `PushRecent`, `ForgetRecent`, `ClearAllRecents`, `GetStatus`, `MessageCount`) also read clearer than closure-based `Mutate`.

API trade-off: read methods (`config`, `recents`, `status`, `message_count`, `query`, `rebuild`) become async. Web call sites in `app.rs` and `search/surface.rs` gain thin `spawn_local` wraps.

Tests: `just test-client`. 74 search tests pass under `tokio::test`.

### 2. `MemNicknameStore` — collapse lock-pair to single `Mutex<Inner>`

`crates/client/src/nickname.rs:39-40`

Today: `RwLock<HashMap>` + `RwLock<u64>` (version).

After: single `Mutex<Inner { map: HashMap<_,_>, version: u64 }>`. Cross-field atomicity (cache write + version bump) is preserved by the single guard. The lock carries a `// state: lock-ok` annotation explaining the constraint.

**Why not bespoke actor**: `MemNicknameStore` implements the public `NicknameStore` trait whose API is sync (`fn get`, `fn set`, `fn clear`, `fn version`, `fn snapshot`). Multiple impls (`MemNicknameStore` in client, `WebNicknameStore` in web) and ~10 sync call sites in the web crate consume the trait. Eliminating the trait in favour of a `NicknameActor` is the right long-term shape but requires async API + web rewrite, which is a separate PR. Tracked in § Follow-up work.

Tests: existing nickname tests stay sync, no API change.

### 3. `trust::InMemoryState` — collapse `Mutex<Inner>` (already single-lock)

`crates/client/src/trust.rs:149`

Today: `std::sync::Mutex<InMemoryState>` — already a single lock; no multi-lock atomicity smell. The `TrustStore` trait API is sync (`fn get`, `fn set`, `fn snapshot`, `fn version`) with the same trait shape as `NicknameStore` and ~15 call sites.

After: keep the single lock; add a `// state: lock-ok` annotation explaining the sync trait constraint. No code change beyond the annotation.

**Why not bespoke actor**: same reasoning as #2. Trait elimination is tracked in § Follow-up work.

Tests: no change.

### 4. `ClientHandle.topics` + `join_links` — annotate, actor migration deferred

`crates/client/src/lib.rs:214,254`

Today: `Arc<RwLock<HashMap<String, N::Topic>>>` (`topics`) and `Arc<parking_lot::Mutex<Vec<ops::JoinLink>>>` (`join_links`) as fields on `ClientHandle`, threaded through `MutationContext` and `ListenerContext`.

After this PR: both stay as locks with `// state: lock-ok` annotations naming the constraint. Each is already a single guard so the multi-lock-atomicity smell does not apply (same shape as #3/§3).

**Why migration is deferred**: ~25 sync `.lock()` / `.read()` call sites across `crates/client/src/{listeners,mutations,joining,lib,connect}.rs`. The `topics` field is also generic over `N::Topic`, so a `StateActor` here would need to be `StateActor<HashMap<String, N::Topic>>` — a generic-typed actor that ripples through `MutationContext` and `ListenerContext`. The migration is mechanical but large enough to warrant its own PR rather than landing inside this one. Tracked as F4 in § Follow-up work.

Tests: no change.

### 5. `ClientHandle.message_db` — delete

`crates/client/src/state.rs:180`

Today: `Option<Arc<std::sync::Mutex<MessageDb>>>` field on `ClientState`.

After: removed. `PersistenceActor` (`crates/client/src/persistence_actor.rs`) already owns persistence and is wired through `ClientHandle.persistence_addr`. The legacy field is dead weight.

Verify with `just check` after deletion (workspace-wide — `cargo check -p willow-client` alone would not catch a downstream consumer in `willow-web` or `willow-agent`). No new tests — pure removal.

### 6. Web layer locks — collapse + annotate, signal conversion deferred

- `crates/web/src/trust_store.rs:46` — `Mutex<Inner>` is already a single lock implementing the sync `TrustStore` trait. Add `// state: lock-ok` annotation. Conversion to a Leptos signal is gated on the trait elimination in § Follow-up work.
- `crates/web/src/profile/nickname_store.rs:23-24` — `RwLock<HashMap> + RwLock<u64>` → collapse to single `Mutex<Inner { map, version }>` for cross-field atomicity. Annotate. Conversion to a Leptos signal is gated on trait elimination.
- `crates/web/src/state_bridge.rs:23` — `Arc<Mutex<Option<U>>>` async result cache. Annotate as `// state: lock-ok` for now; the cache shape is genuinely cross-Effect, and the right replacement (Leptos `Resource` vs eliminate via redesign) needs a deeper look at the bridge architecture. Tracked in § Follow-up work.

**Why not full signal conversion now**: the web stores all sit behind the sync `NicknameStore` / `TrustStore` traits. Replacing the lock with a signal in the impl while leaving the trait sync produces an awkward middle state. The full play is signals + trait elimination, which is § Follow-up work.

Tests: no change.

## Legitimate locks — kept, annotated

Each gets a `// state: lock-ok — <reason>` comment so future readers (human + agent) see the rationale at the use site, not just in this spec.

- `crates/network/src/iroh.rs:107,132` — neighbor list `RwLock<Vec<EndpointId>>`. Mutated from iroh gossip-event callback outside actor loop. Forced by iroh API.
- `crates/network/src/iroh.rs:196,197` — `Mutex<Option<Router>>`, `Mutex<HashMap<TopicId, _>>`. Iroh router + subscription map; iroh's lifecycle API is synchronous.
- `crates/network/src/iroh.rs:215` — relay-status timestamp `Mutex<Option<Instant>>`. Written from the boot online-signal future, read from sync `relay_status()`. Iroh boundary state.
- `crates/network/src/iroh.rs:64` — `IrohBlobStore` `Mutex<HashMap>`. **Not** an iroh-callback boundary — `BlobStore` is Willow's own async trait. Annotated as `lock-ok` because the in-memory store is an interim stub pending a persistent backend; the lock surface goes away with that swap.
- `crates/network/src/iroh.rs:210` — `AtomicBool relay_online_at_boot`. Read-only after init, atomic by choice not necessity but legitimate.
- `crates/client/src/mentions.rs:79` — `OnceLock<Regex>`. One-shot regex compile.
- `crates/network/src/topics.rs:35,37,39` — `LazyLock<TopicId>`. Static topic-ID constants.
- `crates/worker/src/runtime.rs:40` — `tokio::sync::watch::channel` ready signal. Actor coordination primitive, not shared mutable state.
- All `AtomicBool` / `AtomicU32` in `crates/actor/src/*.rs` — mailbox stop, supervisor cancel, test counters. Control flow, not business state.
- All `Rc<RefCell<_>>` / `SendWrapper<RefCell<_>>` in `crates/web/src/voice.rs`, `notifications.rs`, `components/toast.rs`, `components/call_page.rs`, `app.rs` — single-threaded WASM. `RefCell` is the correct primitive.
- `MemNetwork` locks in `crates/network/src/mem.rs` — test infrastructure (`#[cfg(feature = "test-utils")]`). Acceptable; not production code path.
- `crates/agent/src/server.rs:27` — `Arc<tokio::sync::OnceCell<()>>` for one-shot notification bridge init. One-shot, not ongoing mutation.

`MemNicknameStore`/trust in-mem stores get refactored (#2, #3) rather than annotated, because they are production code paths even if currently used only from tests.

## CLAUDE.md addition

Add a `## State Management` section between the existing `## Repository Structure` and `## Build & Test`. Body:

> All shared mutable state in library crates lives inside an actor (see `crates/actor/`). Default to `StateActor<S>` for new state. The decision tree:
>
> | Situation | Pattern |
> |---|---|
> | Shared mutable state in a lib crate | `StateActor<S>` (default) |
> | External-callback boundary (iroh) | Lock + `// state: lock-ok — <reason>` |
> | One-shot static init | `OnceLock` / `LazyLock` |
> | Cross-task control flag (stop, cancel) | `AtomicBool` / `AtomicU32` |
> | WASM single-threaded interior mut | `Rc<RefCell<_>>` (web only) |
> | Reactive UI state in web | Leptos signal (`RwSignal`, `Resource`) |
> | Web state mutated from non-Leptos context | `StateActor<S>` |
>
> No `Arc<Mutex<T>>` / `Arc<RwLock<T>>` / `parking_lot::*` for business state. New locks need a `// state: lock-ok` comment with rationale. Full discussion: `docs/specs/2026-04-26-state-management-model-design.md`.

This block is the source of truth for new code. The spec is the source of truth for the *why* and the audit trail.

## Testing

- `just test-client` for refactored client actors. Each refactored module gets at least one test that exercises the message-passing surface (mutation + read, mutation + subscribe, concurrent mutators if applicable).
- `just test-browser` for web-layer signal conversions.
- Full `just check` (fmt + clippy + test + WASM) green before commit. No warnings.
- `MessageDb` field removal verified by full `just check` (workspace-wide), so any downstream consumer in `willow-web` or `willow-agent` would surface.

No new browser/Playwright tests required — refactors preserve external behaviour.

## Migration order

1. **Doc first.** Spec + CLAUDE.md section. Lands the rule before any refactor commit so reviewers and agents have the contract.
2. **Annotate locks.** Add `// state: lock-ok` comments to all kept locks. Cheap, zero behaviour change, demonstrates the contract.
3. **`SearchIndexHandle`** — proves the bespoke-actor pattern on the highest-value target (4 → 1 lock collapse via single-mailbox ownership; full async API).
4. **`MemNicknameStore`** + **trust `InMemoryState`** — collapse + annotate per § 2/§ 3; trait elimination deferred to F1.
5. **Delete `message_db`** — pure removal, no design risk.
6. **`ClientHandle.topics` + `join_links`** — annotate + defer to F4 per § 4 (full migration is mechanical but large).
7. **Web stores** — collapse `WebNicknameStore` lock pair, annotate the rest per § 6 (signal conversion gated on F1 trait elimination).
8. **`just check`** + commit chain + PR.

Each step in its own commit. Commit body names the runner-up approach where one existed.

## Open questions

None remaining at spec-write time. Audit answered:

- ✅ `PersistenceActor` exists and owns persistence (verified `crates/client/src/persistence_actor.rs`); `message_db` field is dead.
- ✅ Web philosophy locked: signals by default, `StateActor` carve-out for non-Leptos mutators.
- ✅ Enforcement: agent-context awareness via CLAUDE.md, no hard gate.

Decisions deferred to impl time:

- Topic registry shape (Option A fold-in vs Option B new actor) — picked when reading the mutation surface.
- `state_bridge.rs` cache shape (Leptos `Resource` vs `StateActor`) — picked when reading actual call sites.

## Follow-up work

Discovered during impl, deferred to subsequent PRs to keep this one coherent:

### F1. Eliminate `NicknameStore` and `TrustStore` traits in favour of actors

The `NicknameStore` (`crates/client/src/nickname.rs`) and `TrustStore` (`crates/client/src/trust.rs`) traits expose **sync** APIs (`fn get`, `fn set`, `fn version`, `fn snapshot`). Two impls each (`Mem*` in client, `Web*` in web) with ~25 sync call sites across `crates/web/`. The sync API forces lock-based impls; replacing the lock with an actor inside the impl while keeping the trait sync produces a worse hybrid (cache + actor + persistence).

Right shape:

- Single `NicknameActor` and `TrustActor` in client, each owning the in-memory state directly. No trait.
- Persistence becomes a `Persist*` callback / message addressed to a separate persistence-aware actor (e.g. `WebPersistenceActor` for `localStorage`, no-op on native).
- Web call sites adopt async / `spawn_local` the way `SearchIndexHandle` consumers did (#1).
- Spec § 2, 3, and 6 collapse into "the *Store traits are gone".

Estimated cost: ~600 lines across `crates/client/src/{nickname,trust}.rs`, `crates/web/src/{trust_store,profile/nickname_store,components/profile_card,components/add_friend,state}.rs`, and the web tests under `crates/web/tests/browser.rs`.

Trigger: open as a dedicated PR titled `refactor(client+web): eliminate NicknameStore + TrustStore traits in favour of actors`, link back to this spec.

### F2. Re-evaluate `state_bridge.rs` cache shape

The `Arc<Mutex<Option<U>>>` async result cache in `crates/web/src/state_bridge.rs` is annotated `// state: lock-ok` in this PR. Whether the right replacement is a Leptos `Resource`, an explicit derived state actor, or a redesign that eliminates the cache entirely needs a closer look at the bridge's call sites. Touch this when F1 lands, since the trait elimination will simplify the bridge.

### F3. Custom clippy lint or grep gate (optional)

Spec rejected a hard CI gate due to false-positive rate. If drift recurs after F1, revisit. A grep-based check that scans for new `Arc<Mutex<` / `Arc<RwLock<` outside the allowlisted crates and looks for an inline `// state: lock-ok` comment within N lines is the cheapest viable mechanism. A custom clippy lint is the cleaner path but has ongoing maintenance cost against unstable plugin APIs.

### F4. Migrate `ClientHandle.topics` + `join_links` into actors

`Arc<RwLock<HashMap<String, N::Topic>>>` (topics) and `Arc<parking_lot::Mutex<Vec<JoinLink>>>` (join_links) currently live as fields on `ClientHandle` and `MutationContext` / `ListenerContext`. This PR annotates them as `// state: lock-ok` because the migration is mechanical but large: ~25 sync `.lock()` / `.read()` call sites across `listeners.rs`, `mutations.rs`, `joining.rs`, `lib.rs`, `connect.rs`. The `topics` map is generic over `N::Topic`, requiring a generic-typed `StateActor<HashMap<String, N::Topic>>` that threads `N` through any actor it lands in.

Right shape:

- `JoinLinksActor` (concrete, no generics) owning `Vec<JoinLink>`.
- `TopicRegistryActor<N>` owning `HashMap<String, N::Topic>`. Alternatively folded into the existing `server_registry_addr` if `ServerRegistry` becomes generic over `N` (bigger ripple).
- All consumers switch to `Addr.do_send` / `Addr.ask`. `MutationContext` and `ListenerContext` carry `Addr`s instead of `Arc<Lock<_>>`.

Trigger: open as a dedicated PR titled `refactor(client): migrate ClientHandle.topics + join_links into actors`, link back to this spec § 4 + F4.

### F5. SearchActor head-of-line + rebuild-storm fixes

`SearchActor::Rebuild` runs to completion in one mailbox turn. With `SearchIndex` intentionally non-Clone and rebuild-of-N being O(N), a long rebuild blocks every queued `Query` (search-as-you-type latency). Two compounding factors:

1. The rebuild Effect at `crates/web/src/app.rs:367` re-fires on every `messages_sig` change with no debounce or coalescing, so a chatty channel can stack multiple full rebuilds in the mailbox back-to-back.
2. `SearchIndexHandle::insert` uses `do_send`; under sustained burst it silently drops. The Effect's "rebuild on every messages_sig" pattern is the documented recovery path, but it makes the storm worse.

Right shape:

- Wrap the rebuild Effect in a `Debounce<Rebuild>` (the `willow-actor` framework already has `Debounce`), 100–300 ms window.
- Either chunk `Rebuild` to yield between batches (the `Indexing { done, total }` enum variant is already reserved for this) so `Query` can preempt, OR split the read-only `Query` path onto a separate actor with its own `Arc<SearchIndex>` snapshot updated reactively from the writer.
- If the chunked path is chosen, document the now-observable `Indexing` semantics and add a streaming-banner test.

Trigger: open as a dedicated PR titled `perf(client): chunk SearchActor::Rebuild + debounce rebuild Effect`, link back to this spec § 1 + F5.

### F6. Browser-tier coverage for `SearchIndexHandle` consumers

The bespoke-actor migration (§ 1) moved `SearchIndexHandle::query` and `rebuild` from sync to async. Web call sites in `crates/web/src/app.rs` and `crates/web/src/components/search/surface.rs` gained `spawn_local` wrapping. The 74 actor-tier tests under `tokio::test` cover the actor itself; the spawn_local + Effect + signal-write-back path is uncovered at the browser tier.

Per CLAUDE.md "Which test tier to use", DOM rendering + event dispatch in a single client + single viewport → wasm-pack browser test.

Right shape:

- `crates/web/tests/browser.rs` test that mounts `SearchSurface`, fills the input, asserts `set_debouncing` becomes true, then false after results arrive.
- Mount `App` (or a slimmer harness) with two `messages_sig` values and assert `idx.message_count()` reflects each after Effect re-runs.

Trigger: open as a dedicated PR titled `test(web): browser-tier coverage for SearchIndexHandle consumers`, link back to this spec § 1 + F6.

### F7. Sealed `ClientSpawner` to narrow the `system()` API surface

`ClientHandle::system()` returns `&willow_actor::SystemHandle`, which exposes `spawn`, `spawn_supervised`, `spawn_with_capacity`. External consumers (the web `SearchIndexHandle::new`) only need to spawn one specific actor type. Returning the full `SystemHandle` lets a misbehaving consumer spawn arbitrary actors that share the runtime with the client's domain actors.

Right shape:

- Introduce `ClientSpawner` in `crates/client/` that wraps `&SystemHandle` and exposes only the surface external consumers need (e.g. a method per allowed actor type, or a sealed `spawn<A: AllowedActor>` trait).
- Change `ClientHandle::system()` to return `&ClientSpawner`. Document the narrowed surface as the supported extension point.

Trigger: open as a dedicated PR titled `refactor(client): narrow system() to a sealed ClientSpawner`, link back to this spec § 1 + F7.

### F8. Search query debouncing-flicker fix

`crates/web/src/components/search/surface.rs:43-76` debounces the user's keystrokes via `set_timeout_with_handle`. The cleanup cancels the timer but cannot cancel an already-spawned `idx.query().await` future. If the timer already fired and the spawn_local is in flight, the next keystroke's "still loading" indicator can be momentarily stomped to false by the in-flight query's resolution. Functional correctness preserved (FIFO mailbox guarantees ordering); UX flicker only.

Right shape:

- Tag each query with a monotonic generation counter; the spawn_local reads the current generation before writing back to the signal.
- Or migrate to a Leptos `Resource` whose pending state is reactive and managed automatically. (Simpler, but couples the search input to `Resource` semantics — evaluate during impl.)

Trigger: open as a dedicated PR titled `fix(web): generation-tag search-query result write-back`, link back to this spec § 1 + F8.
