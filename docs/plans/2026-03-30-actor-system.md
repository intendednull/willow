# Actor System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace hand-rolled channel/task patterns across worker, client, and web crates with a formalized actor system crate (`willow-actor`) that works on both native and WASM targets, eliminating all locks from the shared state path.

**Architecture:** A pure infrastructure crate (`willow-actor`) provides `Actor`, `Handler<M>`, `StreamHandler`, `Addr<A>`, `System`, and a thin platform abstraction (tokio on native, futures-channel + gloo-timers on WASM). Worker actors, client state, and web UI event loops are migrated to use these primitives.

**Tech Stack:** Rust, tokio (native), futures-channel (WASM), wasm-bindgen-futures, gloo-timers, RPITIT (Rust 1.75+)

**Spec:** `docs/specs/2026-03-29-actor-system-design.md`

---

## File Map

### New Crate

```
crates/actor/
├── Cargo.toml
└── src/
    ├── lib.rs          — Public API re-exports
    ├── actor.rs        — Actor, Handler, StreamHandler, Message traits
    ├── addr.rs         — Addr<A>, AnyAddr, Recipient<M>
    ├── context.rs      — Context<A>, interval, stream attachment
    ├── envelope.rs     — BoxEnvelope, type-erased message dispatch
    ├── mailbox.rs      — Unbounded channel recv loop
    ├── runtime.rs      — Platform abstraction (spawn, channels, timers)
    ├── supervisor.rs   — RestartPolicy, supervised spawn
    ├── system.rs       — System, SystemHandle
    └── error.rs        — SendError, AskError
```

### Modified Files

```
crates/worker/Cargo.toml                  — Add willow-actor dependency
crates/worker/src/actors/mod.rs           — Remove StateMsg enum (replaced by per-message types)
crates/worker/src/actors/state.rs         — StateActor with Handler<EventMsg>, Handler<RequestMsg>, etc.
crates/worker/src/actors/network.rs       — NetworkActor with StreamHandler<GossipEvent>
crates/worker/src/actors/heartbeat.rs     — HeartbeatActor with ctx.run_interval()
crates/worker/src/actors/sync.rs          — SyncActor with ctx.run_interval()
crates/worker/src/runtime.rs              — Replace manual channel/spawn/join with System

crates/client/Cargo.toml                  — Add willow-actor dependency
crates/client/src/lib.rs                  — Replace Arc<RwLock<SharedState>> with state actor
crates/client/src/listeners.rs            — Replace spawn_topic_listener with StreamHandler actor
crates/client/src/state.rs                — State accessor methods become ask() calls

crates/web/Cargo.toml                     — Add willow-actor dependency
crates/web/src/app.rs                     — Update event loop for async handle methods
crates/web/src/event_processing.rs        — process_event_batch becomes async
crates/web/src/components/*.rs            — Wrap sync handle calls in spawn_local

Cargo.toml                                — Add actor to workspace members
```

---

## Task 1: Runtime Abstraction Module

**Files:**
- Create: `crates/actor/Cargo.toml`
- Create: `crates/actor/src/lib.rs`
- Create: `crates/actor/src/runtime.rs`
- Create: `crates/actor/src/error.rs`
- Modify: `Cargo.toml` (workspace members)

The platform abstraction is the foundation everything else builds on. It must compile on both native and `wasm32-unknown-unknown` before any actor types are defined.

- [ ] **Step 1: Create crate skeleton.** `Cargo.toml` with workspace edition/version, dependencies split by target: `tokio` (native), `wasm-bindgen-futures` + `futures-channel` + `gloo-timers` (WASM). Shared deps: `futures-core`, `thiserror`, `tracing`. Add `actor` to workspace `Cargo.toml` members.

- [ ] **Step 2: Implement `runtime.rs`.** Four functions: `spawn()` (tokio::spawn vs spawn_local), `unbounded_channel()` (tokio mpsc vs futures mpsc), `oneshot()` (tokio vs futures), `sleep()` (tokio vs gloo-timers). Define `Sender<T>` / `Receiver<T>` / `OneshotTx<T>` / `OneshotRx<T>` wrapper types that unify the two backends behind a common API. The `Receiver` must implement `async fn recv() -> Option<T>`.

- [ ] **Step 3: Implement `error.rs`.** `SendError<M>` with `Closed(M)` variant. `AskError` with `Closed` and `NoResponse` variants. Both derive `Debug`, `thiserror::Error`.

- [ ] **Step 4: Verify dual-target compilation.** Run `cargo check -p willow-actor` (native) and `cargo check -p willow-actor --target wasm32-unknown-unknown` (WASM). Both must pass with zero warnings.

---

## Task 2: Core Actor Traits and Envelope

**Files:**
- Create: `crates/actor/src/actor.rs`
- Create: `crates/actor/src/envelope.rs`
- Create: `crates/actor/src/mailbox.rs`

Defines the trait hierarchy and the type-erased message dispatch mechanism.

- [ ] **Step 1: Define `Message` trait.** `Send + 'static` with `type Result: Send + 'static`.

- [ ] **Step 2: Define `Actor` trait.** `Send + 'static + Sized` with `started()` and `stopped()` lifecycle hooks using RPITIT (not async_trait). Both have default no-op impls.

- [ ] **Step 3: Define `Handler<M>` trait.** `fn handle(&mut self, msg: M, ctx: &mut Context<Self>) -> impl Future<Output = M::Result> + Send`. Supertrait is `Actor`.

- [ ] **Step 4: Define `StreamHandler<S>` trait.** `handle_stream_item()` and `stream_finished()` with RPITIT. Supertrait is `Actor`.

- [ ] **Step 5: Implement `envelope.rs`.** `BoxEnvelope<A>` type alias: `Box<dyn FnOnce(&mut A, &mut Context<A>) -> BoxFuture<'_, ()> + Send>`. Two factory functions: `envelope_send<A, M>(msg) -> BoxEnvelope<A>` (fire-and-forget) and `envelope_ask<A, M>(msg, reply_tx) -> BoxEnvelope<A>` (captures oneshot sender). Both wrap the handler call in a closure.

- [ ] **Step 6: Implement `mailbox.rs`.** `async fn run_mailbox<A: Actor>(mut actor: A, ctx: Context<A>, rx: Receiver<BoxEnvelope<A>>, stop: Arc<AtomicBool>, done: OneshotTx<()>)`: actor is moved in and mutated via `&mut` for its lifetime. The `Context` and `done` oneshot are provided by the caller (System in Task 3, test harness here). Calls `actor.started(&mut ctx)`, loops on `rx.recv()`, executes each envelope passing `&mut actor` and `&mut ctx`, checks stop flag between messages, calls `actor.stopped()` on exit, then signals `done`. For Task 2 tests, construct a minimal `Context` with a dummy `SystemHandle`.

- [ ] **Step 7: Write mailbox-level tests.** Test `run_mailbox` directly by creating a channel, sending `BoxEnvelope`s manually, and verifying the actor processes them. Test that the mailbox loop exits when the sender is dropped. Full `Addr`/`System`-level tests come in Task 3.

---

## Task 3: Addr, Context, and System

**Files:**
- Create: `crates/actor/src/addr.rs`
- Create: `crates/actor/src/context.rs`
- Create: `crates/actor/src/system.rs`
- Update: `crates/actor/src/lib.rs`

Wires everything together into the public API.

- [ ] **Step 1: Implement `Addr<A>`.** Wraps `runtime::Sender<BoxEnvelope<A>>`. Methods: `send()` wraps msg in `envelope_send`, sends on channel. `ask()` creates a oneshot, wraps msg in `envelope_ask`, sends on channel, awaits oneshot receiver. `is_alive()` checks if channel is open. `Clone` impl.

- [ ] **Step 2: Implement `AnyAddr`.** Type-erased address that can only signal stop (drops a held sender) and check liveness. `From<Addr<A>>` impl.

- [ ] **Step 3: Implement `Context<A>`.** Fields: `addr: Addr<A>`, `system: SystemHandle`, `stop: Arc<AtomicBool>` (shared with the mailbox loop). Methods: `address()`, `stop()` (sets the `AtomicBool` — mailbox checks it between messages), `system()`, `spawn()` (delegates to system), `add_stream()` (spawns a task that polls the stream and forwards items as `StreamEnvelope`s — a separate envelope variant that calls `StreamHandler::handle_stream_item` instead of `Handler::handle`; stream end sends a `StreamFinishedEnvelope`), `run_interval()` (spawns a task that sleeps + sends a message on each tick, returns `IntervalHandle` with cancel).

- [ ] **Step 4: Implement `System` / `SystemHandle`.** `System::new()` creates the handle. `SystemHandle` is `Clone` and holds a list of `AnyAddr`s (for shutdown) plus a list of `OneshotRx<()>` (one per actor, signaled when `run_mailbox` exits). `spawn()` creates a channel, a done oneshot, builds `Context`, spawns `run_mailbox` via `runtime::spawn`, stores the `OneshotRx`, returns `Addr<A>`. `shutdown()` drops all tracked addresses (closing mailboxes) then awaits all done oneshots to confirm actors have stopped.

- [ ] **Step 5: Implement `Recipient<M>`.** Internal `RecipientSender<M>` trait with `send()` and `ask()`. `Addr<A>` implements it for any `A: Handler<M>`. `Recipient<M>` wraps `Box<dyn RecipientSender<M>>`. `From<Addr<A>>` impl.

- [ ] **Step 6: Wire up `lib.rs` re-exports.** Public API: `Actor`, `Handler`, `StreamHandler`, `Message`, `Addr`, `AnyAddr`, `Recipient`, `Context`, `System`, `SystemHandle`, `SendError`, `AskError`, `IntervalHandle`. (`RestartPolicy` added in Task 4.)

- [ ] **Step 7: Write integration tests.** Multi-actor test: spawn two actors, actor A sends to actor B, B replies. Test `StreamHandler` with a `futures::stream::iter`. Test `run_interval` fires expected number of times. Test shutdown stops all actors.

- [ ] **Step 8: Verify WASM compilation.** `cargo check -p willow-actor --target wasm32-unknown-unknown`. Add to `just check-wasm`.

---

## Task 4: Supervision

**Files:**
- Create: `crates/actor/src/supervisor.rs`
- Update: `crates/actor/src/context.rs`

- [ ] **Step 1: Define `RestartPolicy`.** Enum: `Never`, `OnFailure { max: u32 }`, `Backoff { initial: Duration, max_delay: Duration, max_retries: u32 }`.

- [ ] **Step 2: Implement `Context::spawn_supervised()`.** Takes `child: C` where `C: Actor + Clone` and `policy: RestartPolicy`. The wrapper task owns both the channel receiver and the actor clone. On restart, it clones the original actor, creates a fresh stop flag, and re-enters `run_mailbox` — but reuses the same channel. The `Addr<C>` returned to callers points at this stable channel, so it remains valid across restarts. Panics are caught via `std::panic::catch_unwind` (requires `AssertUnwindSafe` wrapper). The wrapper respects `RestartPolicy` limits and backoff delays.

- [ ] **Step 3: Write tests.** Test: actor that panics after N messages gets restarted up to `max` times. Test: `Never` policy does not restart. Test: `Backoff` delays between restarts.

---

## Task 5: Worker Migration

**Files:**
- Modify: `crates/worker/Cargo.toml`
- Modify: `crates/worker/src/actors/mod.rs`
- Modify: `crates/worker/src/actors/state.rs`
- Modify: `crates/worker/src/actors/network.rs`
- Modify: `crates/worker/src/actors/heartbeat.rs`
- Modify: `crates/worker/src/actors/sync.rs`
- Modify: `crates/worker/src/runtime.rs`

Migrate the four hand-rolled worker actors to use `willow-actor`. This is the first real consumer and validates the API.

- [ ] **Step 1: Define message types in `actors/mod.rs`.** Replace `StateMsg` enum with individual message structs: `EventMsg(Event)`, `RequestMsg { req, reply: ... }` → becomes ask-pattern `WorkerRequestMsg(WorkerRequest)` with `type Result = WorkerResponse`, `GetRoleInfoMsg` with `type Result = WorkerRoleInfo`, `GetStateHashesMsg` with `type Result = Vec<(String, StateHash)>`, `ServerDiscoveredMsg { server_id }`. Remove `NetworkOutMsg` (network actor no longer needs a channel — it holds TopicHandle directly). Remove `StateMsg::Shutdown` (handled by mailbox close).

- [ ] **Step 2: Rewrite `state.rs` as `StateActor`.** Struct holds `Box<dyn WorkerRole>`. Implement `Actor` (no lifecycle hooks needed). Implement `Handler<EventMsg>`, `Handler<WorkerRequestMsg>`, `Handler<GetRoleInfoMsg>`, `Handler<GetStateHashesMsg>`, `Handler<ServerDiscoveredMsg>`. Each handler is 1-3 lines — delegates to `self.role`. Remove the manual `run()` function and its `while let` loop.

- [ ] **Step 3: Rewrite `network.rs` as `NetworkActor`.** Struct holds `Addr<StateActor>`, `EndpointId`, and `Option<E>` where `E: TopicEvents + 'static` (the `'static` bound is required because `Actor: 'static`; taken in `started()`). `TopicEvents` is not a `Stream` trait — it has an async `next()` method. Write a thin adapter (`TopicEventStream`) that wraps a `TopicEvents` impl into a `futures::Stream<Item = GossipEvent>` (filtering errors with a warning log). Implement `StreamHandler<GossipEvent>` — the `handle_stream_item` replaces the `while let` loop. Keep `parse_worker_message()` and `parse_server_message()` as pure functions. In `started()`, call `self.events.take().unwrap()` to extract the topic events, wrap in `TopicEventStream`, and attach via `ctx.add_stream()`. Remove the manual `run()` function.

- [ ] **Step 4: Rewrite `heartbeat.rs` as `HeartbeatActor`.** Struct holds `EndpointId`, `Addr<StateActor>`, and the `TopicHandle` (owned, not borrowed — actor owns it for its lifetime). Define `HeartbeatTick` message. Implement `Handler<HeartbeatTick>` — queries state actor via `state_addr.ask(GetRoleInfoMsg)`, broadcasts announcement. In `started()`, call `ctx.run_interval(duration, || HeartbeatTick)`. Implement `stopped()` to broadcast departure message via `self.topic.broadcast()` — best-effort (may fail silently if network is already shut down). Remove `shutdown: watch::Receiver` — the actor stops when its address is dropped.

- [ ] **Step 5: Rewrite `sync.rs` as `SyncActor`.** Same pattern as heartbeat. Define `SyncTick` message. `Handler<SyncTick>` queries state hashes and broadcasts sync requests. `started()` calls `ctx.run_interval()`. Remove watch-based shutdown.

- [ ] **Step 6: Rewrite `runtime.rs`.** Replace manual channel creation, `tokio::spawn`, watch channel, and `tokio::join!` with: create `System`, spawn all four actors, `ctrl_c().await`, `system.shutdown().await`. The function stays generic over `N: Network`.

- [ ] **Step 7: Update existing tests.** Adapt tests in `state.rs`, `network.rs`, `heartbeat.rs` to use the actor system. Tests that sent `StateMsg` directly now use `Addr<StateActor>::send()` / `ask()`. Tests that checked `watch::Receiver` for shutdown now verify the actor stops when the system shuts down.

- [ ] **Step 8: Run `just test-crate worker`.** All existing worker tests must pass. Run `just clippy` — zero warnings.

---

## Task 6: Client Library Migration

**Files:**
- Modify: `crates/client/Cargo.toml`
- Modify: `crates/client/src/lib.rs`
- Modify: `crates/client/src/listeners.rs`
- Modify: `crates/client/src/state.rs`
- Modify: `crates/client/src/events.rs`
- Modify: `crates/client/src/ops.rs`
- Modify: `crates/client/src/invite.rs`
- Modify: `crates/client/src/files.rs`
- Modify: `crates/client/src/worker_cache.rs`

Replace `Arc<RwLock<SharedState>>` and `futures::channel::mpsc` with actors. The state actor becomes the single source of truth, with a subscriber notification mechanism for derived state.

- [ ] **Step 1: Define `ClientStateActor`.** Holds `SharedState` directly (no Arc, no RwLock) plus a `Vec<Recipient<StateChanged>>` subscriber list. Audit all `shared.write()` and `shared.read()` call sites in the client crate to discover the full message set — expect ~10-15 mutation messages and ~5-10 query messages. Define message structs for each. After every mutation handler completes, the actor sends `StateChanged` to all subscribers. Also implement a `ReadState` message with a selector callback that returns a boxed value (for derived state queries). Also implement `Subscribe(Recipient<StateChanged>)` to register new watchers.

- [ ] **Step 2: Define `TopicListenerActor`.** Replaces `spawn_topic_listener`. Implements `StreamHandler` for gossip events. Holds `Addr<ClientStateActor>` and `TopicHandle`. In `handle_stream_item`, sends mutations to the state actor. No longer emits `ClientEvent`s — the state actor's subscriber notification replaces the event channel for state-derived signals. Ephemeral events (typing indicators, connection status changes) that aren't part of `SharedState` can still use a lightweight channel or a separate notification actor.

- [ ] **Step 3: Refactor `ClientHandle<N>`.** Replace `shared: Arc<RwLock<SharedState>>` with `state: Addr<ClientStateActor>`. Also move `topics: Arc<RwLock<HashMap<String, N::Topic>>>` into the state actor (or a dedicated topic actor). Remove `event_tx` for state synchronization — derived state actors replace it. Keep a small ephemeral event channel for non-state notifications (typing, connection). **Note:** this makes previously-synchronous state accessors async (`shared.read()` → `state.ask().await`). All callers in `lib.rs`, `ops.rs`, `invite.rs`, `files.rs`, and `worker_cache.rs` must be updated to `.await` state queries. Methods that were `fn` become `async fn`.

- [ ] **Step 4: Update `listeners.rs`.** Replace `spawn_topic_listener()` with spawning a `TopicListenerActor` on the system. Remove the manual `topic_listener_loop`.

- [ ] **Step 5: Update client tests.** Tests using `test_client()` helper need updating — shared state access changes from lock-based to ask-based. Verify all `just test-client` tests pass.

- [ ] **Step 6: Run `just test-client` and `just clippy`.** All 93+ client tests must pass with zero warnings.

---

## Task 7: Web UI Migration — Derived State Signals

**Files:**
- Modify: `crates/web/Cargo.toml`
- Modify: `crates/web/src/app.rs`
- Delete: `crates/web/src/event_processing.rs`
- Rewrite: `crates/web/src/state.rs`
- Modify: `crates/web/src/components/*.rs` (remove direct handle state reads)

Replaces the `ClientEvent` → `process_event_batch` → signal update pipeline with selector-based derived state actors.

### Actor ↔ Leptos Bridge

**Current flow** (event-driven, pull-based):
```
TopicEvents → spawn_local loop → ClientEvent channel → process_event_batch() → WriteSignal::set()
                                                         ↓ reads handle.peers(), handle.messages(), etc.
                                                         (sync reads via Arc<RwLock<SharedState>>)
```

**New flow** (push-based, selector-driven):
```
Network → TopicListenerActor → mutations → ClientStateActor
                                                ↓ StateChanged notification
                                          DerivedStateActors (one per signal)
                                                ↓ selector(state) → compare → signal.set() if changed
                                          Leptos reactive signals
```

The `ClientEvent` channel and `process_event_batch` are eliminated for state-derived signals. Each Leptos signal is backed by a `DerivedStateActor` that watches a specific slice of `SharedState` via a selector function.

- [ ] **Step 1: Implement `DerivedStateActor<T>`.** Generic actor parameterized by `T: PartialEq + Clone + Send + 'static`. Fields: `state_addr: Addr<ClientStateActor>`, `selector: Box<dyn Fn(&SharedState) -> T + Send>`, `cached: Option<T>`, `write: WriteSignalSender<T>` (a callback or channel that sets the Leptos signal — must be `Send` on WASM via `SendWrapper`). Implements `Handler<StateChanged>`: asks state actor for current derived value via `ReadState`, compares with cached, updates signal if different. Subscribes to state actor in `started()`.

- [ ] **Step 2: Implement `derived_signal` helper.** A function in the web crate (not in willow-actor — it depends on Leptos): `fn derived_signal<T>(state_addr, system, selector) -> ReadSignal<T>`. Creates a Leptos signal pair, spawns a `DerivedStateActor`, returns the read half. This is the primary API for connecting actor state to Leptos.

- [ ] **Step 3: Rewrite `state.rs`.** Replace the current `create_signals()` function (which creates ~30 independent signals) with calls to `derived_signal()`. Each signal maps to a selector:
  - `messages` → `|s| s.state.messages_for(&s.state.current_channel)`
  - `channels` → `|s| s.state.channels.clone()`
  - `peers` → `|s| s.state.chat.peers.clone()`
  - `display_name` → `|s| s.state.display_name.clone()`
  - etc.

  Signals that don't derive from `SharedState` (e.g., `show_settings`, `show_palette`, `current_tab`) remain as regular Leptos signals — they are pure UI state.

- [ ] **Step 4: Update `app.rs`.** Remove the `spawn_local` event loop that drained `ClientEvent`s and called `process_event_batch`. Remove `refresh_all_signals`. The `ClientHandle` connection still happens in a `spawn_local` (network setup is async), but signal updates are now automatic via derived state actors. Handle ephemeral events (typing indicators, connection status) via a small separate channel or dedicated actors.

- [ ] **Step 5: Delete `event_processing.rs`.** The entire module is replaced by derived state actors. The `process_event_batch` function, `needs_*_refresh` flags, and event-to-signal mapping are all gone.

- [ ] **Step 6: Update components.** Components that called `handle.peers()`, `handle.messages()`, etc. directly now read from their corresponding derived signal instead. Components that perform actions (send message, create channel) still call `handle.send_message()` etc., which sends a mutation message to the state actor. Grep for `handle.` in components and verify each call is either an action (keep) or a state read (replace with signal).

- [ ] **Step 7: Handle ephemeral events.** Typing indicators, connection status changes, and voice signals are transient — they aren't part of `SharedState` and don't need derived actors. Options: (a) add them to `SharedState` and let selectors handle them, (b) keep a small `futures::channel::mpsc` for ephemeral events with a dedicated `spawn_local` consumer, (c) use dedicated actors with their own signals. Choose (a) if the events map cleanly to state fields; (b) for truly transient notifications.

- [ ] **Step 8: Verify WASM compilation.** `just check-wasm` must pass.

- [ ] **Step 9: Run `just test-browser`.** All 39+ browser tests must pass.

---

## Task 8: Final Validation

- [ ] **Step 1: Run `just check`.** Full suite: fmt + clippy + test + WASM. Zero warnings.

- [ ] **Step 2: Run `just test-scale`.** Verify no performance regression in event throughput or merge benchmarks.

- [ ] **Step 3: Run `just test-all`.** All 420+ tests pass.

- [ ] **Step 4: Update `CLAUDE.md`.** Add `crates/actor/` to the repository structure. Update the architecture notes to describe the actor system.
