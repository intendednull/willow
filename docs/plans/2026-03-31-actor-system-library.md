# Actor System Library Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add generic state actors, derived actors, stream output, pub/sub broker, FSM, pool, and debounce/throttle to the `willow-actor` crate, making it a self-contained actor framework with built-in state management patterns.

**Architecture:** All new types build on the existing `Actor`, `Handler<M>`, `Addr<A>` primitives. `StateActor<S>` and `DerivedActor<Src, T>` form a reactive state graph via `StateRef<S>` handles. `StreamOutput<T>` enables actors to produce `futures::Stream`s. `Broker<T>`, `FsmActor<M>`, `Pool<A>`, `Debounce<M>`, and `Throttle<M>` provide common patterns.

**Tech Stack:** Rust, tokio (native), futures (join!), futures-channel (WASM), RPITIT (Rust 1.75+)

**Spec:** `docs/specs/2026-03-31-actor-system-library-design.md`

---

## File Map

### New Files

```
crates/actor/src/
├── state.rs            — StateActor<S>, StateRef<S>, Get, Set, Mutate, Select, Subscribe, Notify
├── derived.rs          — DeriveSource trait, DerivedActor<Src, T>, derived(), tuple macro
├── stream.rs           — StreamOutput<T>, OutputStream<T>, SubscribeStream<T>
├── broker.rs           — Broker<T>, Publish, BrokerSubscribe, BrokerUnsubscribe, SubscriptionId
├── fsm.rs              — StateMachine trait, FsmActor<M>, TransitionResult
├── pool.rs             — Pool<A>
├── debounce.rs         — Debounce<M>, Throttle<M>
crates/actor/tests/
└── performance.rs      — Performance/throughput tests
```

### Modified Files

```
crates/actor/Cargo.toml     — Add `futures` dependency
crates/actor/src/lib.rs     — Add module declarations and re-exports
crates/actor/src/context.rs — Add run_after() and TimerHandle
crates/actor/src/runtime.rs — Add bounded_channel()
justfile                     — Add test-actor and test-actor-perf commands
```

---

## Task 1: Core Extensions

**Files:**
- Modify: `crates/actor/Cargo.toml`
- Modify: `crates/actor/src/runtime.rs`
- Modify: `crates/actor/src/context.rs`

Prerequisites that other tasks depend on: `futures` dependency, bounded channels, and one-shot timers.

- [ ] **Step 1: Add `futures` dependency.** Add `futures = "0.3"` to `[dependencies]` in `Cargo.toml` (not target-gated — needed on both native and WASM for `join!`). Move existing `futures = "0.3"` from `[dev-dependencies]` to `[dependencies]`.

- [ ] **Step 2: Add `bounded_channel` to `runtime.rs`.** New `BoundedSender<T>` and `BoundedReceiver<T>` wrapper types. `BoundedSender::try_send(val) -> Result<(), T>` — returns the value on full or closed. `BoundedReceiver` implements `futures::Stream<Item = T>`. Factory: `fn bounded_channel<T: Send + 'static>(capacity: usize) -> (BoundedSender<T>, BoundedReceiver<T>)`. Native: `tokio::sync::mpsc::channel(capacity)`. WASM: `futures::channel::mpsc::channel(capacity)`. Note: `BoundedReceiver` must implement `Stream` for use with `ctx.add_stream()` and for `OutputStream` — implement via `poll_fn` forwarding to the inner receiver's `poll_recv` (native) or `poll_next` (WASM).

- [ ] **Step 3: Add `TimerHandle` and `run_after` to `context.rs`.** `TimerHandle` is structurally identical to `IntervalHandle` (wraps `Arc<AtomicBool>`). `run_after<M>(&self, delay: Duration, msg: M) -> TimerHandle` spawns a task that sleeps once, checks cancelled flag, then sends the message envelope. Unlike `run_interval`, no loop — task exits after the single send. Drop cancels (same as `IntervalHandle`).

- [ ] **Step 4: Write tests.** Test `run_after` fires after delay. Test `TimerHandle::cancel()` prevents delivery. Test cancel-then-create-new works. Test `bounded_channel` respects capacity (send N+1 items to a capacity-N channel, verify `try_send` fails on the last).

- [ ] **Step 5: Verify WASM compilation.** `cargo check -p willow-actor --target wasm32-unknown-unknown`.

---

## Task 2: StateActor

**Files:**
- Create: `crates/actor/src/state.rs`
- Modify: `crates/actor/src/lib.rs`

The foundational state container that all other stateful actor types build on.

- [ ] **Step 1: Define message types.** In `state.rs`:
  - `Notify` — `Clone`, `Message<Result = ()>`. Sent to subscribers after mutation.
  - `Subscribe(Recipient<Notify>)` — `Message<Result = ()>`.
  - `Get<S>(PhantomData<S>)` — `Message<Result = Arc<S>>` where `S: Send + Sync`. Returns cheaply via `Arc::clone`.
  - `Set<S>(S)` — `Message<Result = ()>` where `S: Send + Sync`.
  - `Select(Box<dyn FnOnce(&dyn Any) -> Box<dyn Any + Send> + Send>)` — `Message<Result = Box<dyn Any + Send>>`.
  - `Mutate(Box<dyn FnOnce(&mut dyn Any) -> Box<dyn Any + Send> + Send>)` — `Message<Result = Box<dyn Any + Send>>`. Note: the handler impl requires `S: Clone` for `Arc::make_mut()`.

- [ ] **Step 2: Implement `StateActor<S>`.** Struct bound: `S: Send + Sync + 'static`. Fields: `state: Arc<S>`, `dirty: bool`, `subscribers: Vec<Recipient<Notify>>`. Constructor: `StateActor::new(initial: S)` wraps in `Arc::new(initial)`. Implement `Actor` with `idle()` that checks `dirty`, sends `Notify` to all subscribers (pruning dead ones where `do_send` fails), resets `dirty`. Implement handlers:
  - `Handler<Get<S>>` — returns `Arc::clone(&self.state)` (pointer bump, no deep clone).
  - `Handler<Set<S>>` — `self.state = Arc::new(msg.0); self.dirty = true;`.
  - `Handler<Select>` — downcast `&*self.state as &dyn Any`, call the closure, return result. Panic on downcast failure (type mismatch is a programming error).
  - `Handler<Mutate>` — requires `S: Clone` on the impl (stricter than the struct bound). Call `Arc::make_mut(&mut self.state)` for copy-on-write (clones only if refcount > 1), downcast the `&mut S` to `&mut dyn Any`, call the closure, set `self.dirty = true`, return result.
  - `Handler<Subscribe>` — push recipient to `self.subscribers`.

- [ ] **Step 3: Implement typed helpers.** Free functions:
  - `pub async fn get<S>(addr) -> Arc<S>` where `S: Send + Sync` — calls `addr.ask(Get(PhantomData))`, returns `Arc<S>`.
  - `pub async fn select<S, T>(addr, f) -> T` where `S: Send + Sync` — wraps `f` in a `Select` closure that downcasts `&dyn Any` to `&S`, calls `addr.ask(Select(...))`, downcasts result `Box<dyn Any>` to `T`.
  - `pub async fn mutate<S, T>(addr, f) -> T` where `S: Clone + Send + Sync` — wraps `f` in a `Mutate` closure. The `StateActor` handler calls `Arc::make_mut()` internally before downcasting to `&mut S`, so the closure receives `&mut S` transparently. The `Clone` bound is only on this helper, not on `get`/`select`.
  - `pub fn subscribe<S, A>(state, subscriber)` where `S: Send + Sync` — converts subscriber addr to `Recipient<Notify>`, sends `Subscribe`.

- [ ] **Step 4: Implement `StateRef<S>`.** Fields: `subscribe: Arc<dyn Fn(Recipient<Notify>) + Send + Sync>`, `select: Arc<dyn Fn(...) -> Pin<Box<...>> + Send + Sync>`, `get: Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Arc<S>> + Send>> + Send + Sync>`, `_phantom: PhantomData<S>`. Methods: `subscribe()`, `get() -> Arc<S>` (no deep clone), `select()`. `from_addr<A>(addr)` — generic constructor for any `A: Handler<Subscribe> + Handler<Select> + Handler<Get<S>>`. `From<&Addr<StateActor<S>>>` impl constructs the closures by capturing a cloned `Addr` — `get` forwards to `ask(Get(PhantomData))`, `select` forwards to `ask(Select(...))`. Derive `Clone`.

- [ ] **Step 5: Write unit tests.** All 13 StateActor tests from the test plan: `state_get_returns_arc`, `state_get_arc_shares_identity`, `state_set_updates`, `state_select_slice`, `state_mutate_modifies`, `state_mutate_returns_value`, `state_mutate_cow_clones_when_held`, `state_mutate_inplace_when_sole`, `state_subscribe_notifies`, `state_no_notify_without_mutation`, `state_batch_notifications`, `state_dead_subscribers_pruned`, `state_multiple_subscribers`. Plus 5 StateRef tests: `state_ref_from_state_actor`, `state_ref_clone`, `state_ref_get`, `state_ref_select`.

- [ ] **Step 6: Add to `lib.rs`.** `pub mod state;` and re-exports: `StateActor`, `StateRef`, `Notify`, `Subscribe`, `Get`, `Set`, `Select`, `Mutate`, `get`, `select`, `mutate`, `subscribe`.

---

## Task 3: DerivedActor

**Files:**
- Create: `crates/actor/src/derived.rs`
- Modify: `crates/actor/src/lib.rs`

Depends on Task 2 (StateActor, StateRef, Notify, Subscribe).

- [ ] **Step 1: Define `DeriveSource` trait.** `Clone + Send + 'static` with associated type `Snapshot: Send + 'static`. Methods: `fn subscribe_all(&self, recipient: Recipient<Notify>)` and `fn snapshot(&self) -> impl Future<Output = Self::Snapshot> + Send`.

- [ ] **Step 2: Implement `DeriveSource` for `StateRef<S>`.** `Snapshot = Arc<S>`. `subscribe_all` calls `self.subscribe(recipient)`. `snapshot` calls `self.get()` which returns `Arc<S>` (pointer bump).

- [ ] **Step 3: Implement tuple macro.** `impl_derive_source_tuple!` macro that generates `DeriveSource` impls for `(StateRef<S0>, StateRef<S1>)` through `(StateRef<S0>, ..., StateRef<S5>)`. `Snapshot` is a tuple of `Arc`s (e.g., `(Arc<S0>, Arc<S1>)`). `subscribe_all` clones the recipient for each element (last one moves). `snapshot` uses `futures::join!` for parallel `get()` — all returns are `Arc` so no deep cloning. Generate invocations for arities 2 through 6.

- [ ] **Step 4: Define internal `UpdateCache<T>` message.** `Message<Result = ()>`. This is private to the module — used by `DerivedActor` to update its cached value after the async snapshot completes.

- [ ] **Step 5: Implement `DerivedActor<Src, T>`.** Struct bound: `T: PartialEq + Send + Sync + 'static` (Sync required because `Arc<T>` is stored and actor must be `Send`). Fields: `sources: Src`, `selector: Arc<dyn Fn(&Src::Snapshot) -> T + Send + Sync>`, `cached: Option<Arc<T>>`, `subscribers: Vec<Recipient<Notify>>`, `dirty: bool`. Implement `Actor::started()` — calls `sources.subscribe_all(ctx.address().into())`, then does an initial snapshot + selector computation and sends `UpdateCache` to self. Implement `Handler<Notify>` — clones `sources` and `selector` into async block, fetches snapshot (returns `Arc`s — cheap), computes value via selector, sends `UpdateCache(value)` to self via `ctx.address()`. Implement `Handler<UpdateCache<T>>` — compares with `cached` via `PartialEq` (deref through `Arc`), if changed: `cached = Some(Arc::new(new_value))`, sets `dirty`. Implement `Actor::idle()` — same pattern as StateActor (notify subscribers if dirty). Implement `Handler<Subscribe>` — push to subscribers list. Implement `Handler<Select>` — read from `cached`, return `Arc::clone`. Implement `Handler<Get<T>>` — return `Arc::clone(&self.cached.unwrap())`.

- [ ] **Step 6: Implement `StateRef<T>` for DerivedActor.** `From<&Addr<DerivedActor<Src, T>>>` impl that constructs `StateRef<T>` by capturing the derived actor's address. The `select` closure sends `Select` to the derived actor. The `subscribe` closure sends `Subscribe`.

- [ ] **Step 7: Implement `derived()` convenience constructor.** `pub fn derived<Src, T>(system: &SystemHandle, sources: Src, selector: impl Fn(&Src::Snapshot) -> T + Send + Sync + 'static) -> StateRef<T>`. Spawns a `DerivedActor`, returns `StateRef<T>` from its address.

- [ ] **Step 8: Write unit tests.** All 9 DerivedActor tests from the test plan: `derived_single_source`, `derived_caches_unchanged`, `derived_multi_source_tuple2`, `derived_multi_source_tuple3`, `derived_chain`, `derived_chain_caches`, `derived_initial_value`, `derived_source_dies`, `derived_update_cache_self_message`. Plus `state_ref_from_derived_actor` from the StateRef tests.

- [ ] **Step 9: Add to `lib.rs`.** `pub mod derived;` and re-exports: `DeriveSource`, `DerivedActor`, `derived`.

---

## Task 4: StreamOutput

**Files:**
- Create: `crates/actor/src/stream.rs`
- Modify: `crates/actor/src/lib.rs`

Depends on Task 1 (bounded_channel).

- [ ] **Step 1: Implement `StreamOutput<T>`.** Fields: `subscribers: Vec<runtime::BoundedSender<T>>`, `default_capacity: usize` (default 64). Methods:
  - `StreamOutput::new() -> Self`
  - `emit(&mut self, value: T)` — iterates subscribers, calls `try_send(value.clone())` on each, prunes closed channels (where `try_send` returns error due to closed, not full). Log warning via `tracing::warn!` when dropping a value due to full buffer.
  - `subscribe(&mut self) -> OutputStream<T>` — creates bounded channel with `default_capacity`, stores sender, returns `OutputStream` wrapping receiver.
  - `subscribe_with_capacity(&mut self, capacity: usize) -> OutputStream<T>` — same with custom capacity.

- [ ] **Step 2: Implement `OutputStream<T>`.** Wraps `runtime::BoundedReceiver<T>`. Implement `futures::Stream<Item = T>` by delegating to the inner receiver's `Stream` impl.

- [ ] **Step 3: Define `SubscribeStream<T>` message.** `pub struct SubscribeStream<T>(PhantomData<T>)` with `Message<Result = OutputStream<T>>`. Provide `Default` impl for ergonomic construction.

- [ ] **Step 4: Write unit tests.** All 6 StreamOutput tests: `stream_output_single_consumer`, `stream_output_multi_consumer`, `stream_output_consumer_drop`, `stream_output_backpressure`, `stream_output_custom_capacity`, `stream_output_subscribe_stream_message`.

- [ ] **Step 5: Add to `lib.rs`.** `pub mod stream;` and re-exports: `StreamOutput`, `OutputStream`, `SubscribeStream`.

---

## Task 5: Broker

**Files:**
- Create: `crates/actor/src/broker.rs`
- Modify: `crates/actor/src/lib.rs`

- [ ] **Step 1: Define types.** `SubscriptionId(u64)` — `Clone, Copy, PartialEq, Eq, Hash`. `Publish<T>(T)` — `Message<Result = ()>`, requires `T: Message<Result = ()> + Clone`. `BrokerSubscribe<T>(Recipient<T>)` — `Message<Result = SubscriptionId>`. `BrokerUnsubscribe(SubscriptionId)` — `Message<Result = ()>`.

- [ ] **Step 2: Implement `Broker<T>`.** Fields: `subscribers: Vec<(SubscriptionId, Recipient<T>)>`, `next_id: u64`. Constructor: `Broker::new()`. Implement `Actor` (no lifecycle hooks). Handlers:
  - `Handler<Publish<T>>` — iterate subscribers, `do_send(msg.0.clone())` to each. Prune dead (where `do_send` returns `Err`).
  - `Handler<BrokerSubscribe<T>>` — assign `next_id`, increment, push `(id, recipient)`, return id.
  - `Handler<BrokerUnsubscribe>` — `retain` entries where id doesn't match.

- [ ] **Step 3: Write unit tests.** All 6 Broker tests: `broker_publish_to_subscribers`, `broker_no_subscribers`, `broker_subscribe_returns_id`, `broker_unsubscribe_by_id`, `broker_dead_subscriber_pruned`, `broker_multiple_publishers`.

- [ ] **Step 4: Add to `lib.rs`.** `pub mod broker;` and re-exports.

---

## Task 6: FsmActor

**Files:**
- Create: `crates/actor/src/fsm.rs`
- Modify: `crates/actor/src/lib.rs`

- [ ] **Step 1: Define `StateMachine` trait.** Associated types: `State: Send + Sync + Clone + 'static` (Sync required for `StateRef` compatibility), `Input: Message<Result = TransitionResult<Self::State>> + Send + 'static`. Methods: `fn transition(&self, state: &Self::State, input: &Self::Input) -> Result<Self::State, String>`, `fn on_enter(&mut self, old: &Self::State, new: &Self::State, ctx: &mut Context<FsmActor<Self>>)` (default no-op). Bound: `Send + 'static`.

- [ ] **Step 2: Define `TransitionResult<S>`.** Enum: `Ok(S)`, `Rejected(String)`.

- [ ] **Step 3: Implement `FsmActor<M>`.** Fields: `machine: M`, `state: M::State`, `subscribers: Vec<Recipient<Notify>>`, `dirty: bool`. Constructor takes `machine` and `initial_state`. Implement `Handler<M::Input>` — calls `machine.transition(&self.state, &msg)`. On `Ok(new)`: calls `machine.on_enter(&old, &new, ctx)`, updates `self.state`, sets `dirty`, returns `TransitionResult::Ok(new)`. On `Err(reason)`: returns `TransitionResult::Rejected(reason)`. Implement `Actor::idle()` — same notify pattern as StateActor. Implement `Handler<Subscribe>` and `Handler<Select>` — Select reads from `self.state`.

- [ ] **Step 4: Implement `StateRef` for FsmActor.** `From<&Addr<FsmActor<M>>>` producing `StateRef<M::State>`.

- [ ] **Step 5: Write unit tests.** All 7 FSM tests: `fsm_valid_transition`, `fsm_rejected_transition`, `fsm_on_enter_called`, `fsm_on_enter_not_called_on_reject`, `fsm_notifies_subscribers`, `fsm_no_notify_on_reject`, `fsm_select_current_state`.

- [ ] **Step 6: Add to `lib.rs`.** `pub mod fsm;` and re-exports.

---

## Task 7: Pool

**Files:**
- Create: `crates/actor/src/pool.rs`
- Modify: `crates/actor/src/lib.rs`

- [ ] **Step 1: Implement `Pool<A>`.** Fields: `workers: Vec<Addr<A>>`, `next: usize`. Constructor: `Pool::new(system: &SystemHandle, actor: A, size: usize) -> Self` where `A: Actor + Clone` — clones the actor `size` times, spawns each on the system, collects addresses. Methods:
  - `send<M>(&mut self, msg) -> Result<(), SendError<M>>` — fire-and-forget to `workers[next % len]`, increment `next`.
  - `ask<M>(&mut self, msg) -> impl Future<Output = Result<M::Result, AskError>>` — same routing, forwards via `ask()`.

- [ ] **Step 2: Write unit tests.** All 5 Pool tests: `pool_round_robin_distribution`, `pool_send_fire_and_forget`, `pool_ask_returns_result`, `pool_worker_dies`, `pool_size_one`.

- [ ] **Step 3: Add to `lib.rs`.** `pub mod pool;` and re-exports.

---

## Task 8: Debounce and Throttle

**Files:**
- Create: `crates/actor/src/debounce.rs`
- Modify: `crates/actor/src/lib.rs`

Depends on Task 1 (`run_after`, `TimerHandle`).

- [ ] **Step 1: Implement `Debounce<M>`.** Fields: `target: Recipient<M>`, `delay: Duration`, `pending: Option<M>`, `timer: Option<TimerHandle>`. Constructor: `Debounce::new(target: Recipient<M>, delay: Duration)`. Define internal `Flush` message (`Message<Result = ()>`). Implement `Actor` (no hooks). Implement `Handler<M>` — store `msg` as `pending`, cancel existing timer (`self.timer.take()`), start new timer via `ctx.run_after(self.delay, Flush)`. Implement `Handler<Flush>` — if `pending.take()` is Some, forward to `target` via `do_send`.

- [ ] **Step 2: Implement `Throttle<M>`.** Fields: `target: Recipient<M>`, `interval: Duration`, `pending: Option<M>`, `cooling_down: bool`. Constructor: `Throttle::new(target: Recipient<M>, interval: Duration)`. Define internal `CooldownExpired` message. Implement `Handler<M>` — if not `cooling_down`, forward immediately via `target.do_send(msg)`, set `cooling_down = true`, start `ctx.run_after(self.interval, CooldownExpired)`. If `cooling_down`, store as `pending`. Implement `Handler<CooldownExpired>` — set `cooling_down = false`. If `pending.take()` is Some, forward it, set `cooling_down = true` again, start new cooldown timer.

- [ ] **Step 3: Write unit tests.** All 8 Debounce/Throttle tests: `debounce_single_message`, `debounce_rapid_messages`, `debounce_timer_reset`, `debounce_separate_bursts`, `throttle_immediate_first`, `throttle_rate_limited`, `throttle_pending_forwarded`, `throttle_only_latest_pending`.

- [ ] **Step 4: Add to `lib.rs`.** `pub mod debounce;` and re-exports.

---

## Task 9: Performance Tests

**Files:**
- Create: `crates/actor/tests/performance.rs`
- Modify: `justfile`

Depends on all previous tasks.

- [ ] **Step 1: Add justfile commands.** Add `test-actor` for unit tests and `test-actor-perf` for performance tests:
  ```
  test-actor:
      cargo test -p willow-actor

  test-actor-perf:
      cargo test -p willow-actor --test performance -- --nocapture
  ```

- [ ] **Step 2: Implement throughput tests.** In `performance.rs`:
  - `perf_state_actor_throughput` — 10k `mutate()` calls, assert >100k ops/sec.
  - `perf_state_actor_get_throughput` — 10k `get()` calls (Arc clone only), assert >500k ops/sec.
  - `perf_state_actor_select_throughput` — 10k `select()` calls, assert >100k ops/sec.
  - `perf_state_actor_cow_vs_clone` — compare `mutate()` throughput with and without outstanding `Arc` refs. Report only (informational).
  - `perf_stream_output_throughput` — 100k emits to single consumer, assert >500k emits/sec.
  - `perf_pool_round_robin_throughput` — 10k `ask()` through 4-worker pool, assert >50k ops/sec.

- [ ] **Step 3: Implement fanout tests.**
  - `perf_state_notify_fanout` — Create StateActor with N subscribers (N = 1, 10, 100, 1000). Mutate once, measure time until all subscribers received `Notify`. Assert <1ms for 100 subscribers.
  - `perf_broker_fanout` — Same pattern with Broker, publish to N subscribers.
  - `perf_stream_output_multi_consumer` — 100k emits with 10 consumers, assert >100k emits/sec.

- [ ] **Step 4: Implement propagation tests.**
  - `perf_derived_propagation_latency` — StateActor → DerivedActor, measure end-to-end from mutation to cache update. Assert <1ms.
  - `perf_derived_chain_depth` — Chain of 10 DerivedActors, measure propagation from root mutation to leaf. Assert <5ms.
  - `perf_derived_multi_source_snapshot` — DerivedActor with 2, 4, 6 sources. Measure snapshot fetch time. Assert <2ms for 6 sources.

- [ ] **Step 5: Implement efficiency tests.**
  - `perf_derived_idle_batching_efficiency` — 1000 rapid mutations to a StateActor with one subscriber. Count how many `Notify` messages the subscriber receives. Assert notification count is <10% of mutation count.
  - `perf_debounce_overhead` — Send a single message through Debounce(50ms), measure actual delivery time. Assert <52ms (delay + <2ms overhead).

- [ ] **Step 6: Implement memory test.**
  - `perf_state_actor_memory` — Create a StateActor with 100 subscribers, report approximate memory usage. No threshold — informational only, printed with `--nocapture`.

---

## Task 10: Final Validation

- [ ] **Step 1: Run `just fmt` and `just clippy`.** Zero warnings.

- [ ] **Step 2: Run `just test-actor`.** All unit tests pass.

- [ ] **Step 3: Run `just test-actor-perf`.** All performance tests pass thresholds.

- [ ] **Step 4: Run `just check-wasm`.** WASM compilation succeeds.

- [ ] **Step 5: Run `just test`.** Full workspace tests — no regressions in other crates.

- [ ] **Step 6: Update `lib.rs` documentation.** Add module-level docs for each new module. Update the top-level `//!` doc comment in `lib.rs` to mention the new actor types.
