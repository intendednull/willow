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
crates/web/src/app.rs                     — Spawn actors instead of manual event loop

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

- [ ] **Step 6: Implement `mailbox.rs`.** `async fn run_mailbox<A: Actor>(mut actor: A, rx: Receiver<BoxEnvelope<A>>, stop: Arc<AtomicBool>)`: actor is moved in and mutated via `&mut` for its lifetime. Calls `actor.started(&mut ctx)`, loops on `rx.recv()`, executes each envelope passing `&mut actor` and `&mut ctx`, checks stop flag between messages, calls `actor.stopped()` on exit (either channel closed or stop flag set).

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

- [ ] **Step 4: Implement `System` / `SystemHandle`.** `System::new()` creates the handle. `SystemHandle` is `Clone` and holds a list of `AnyAddr`s (for shutdown). `spawn()` creates a channel, builds `Context`, spawns `run_mailbox` via `runtime::spawn`, returns `Addr<A>`. `shutdown()` drops all tracked addresses and waits for mailboxes to drain.

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

- [ ] **Step 3: Rewrite `network.rs` as `NetworkActor`.** Struct holds `Addr<StateActor>`, `EndpointId`, and `Option<E>` where `E: TopicEvents` (taken in `started()`). `TopicEvents` is not a `Stream` trait — it has an async `next()` method. Write a thin adapter (`TopicEventStream`) that wraps a `TopicEvents` impl into a `futures::Stream<Item = GossipEvent>` (filtering errors with a warning log). Implement `StreamHandler<GossipEvent>` — the `handle_stream_item` replaces the `while let` loop. Keep `parse_worker_message()` and `parse_server_message()` as pure functions. In `started()`, call `self.events.take().unwrap()` to extract the topic events, wrap in `TopicEventStream`, and attach via `ctx.add_stream()`. Remove the manual `run()` function.

- [ ] **Step 4: Rewrite `heartbeat.rs` as `HeartbeatActor`.** Struct holds `EndpointId`, `Addr<StateActor>`, and the `TopicHandle` (owned, not borrowed — actor owns it for its lifetime). Define `HeartbeatTick` message. Implement `Handler<HeartbeatTick>` — queries state actor via `state_addr.ask(GetRoleInfoMsg)`, broadcasts announcement. In `started()`, call `ctx.run_interval(duration, || HeartbeatTick)`. Implement `stopped()` to broadcast departure message via `self.topic.broadcast()` — the topic handle is still valid because the actor owns it. Remove `shutdown: watch::Receiver` — the actor stops when its address is dropped.

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

Replace `Arc<RwLock<SharedState>>` and `futures::channel::mpsc` with actors.

- [ ] **Step 1: Define `ClientStateActor`.** Holds `SharedState` directly (no Arc, no RwLock). Define message types for mutations: `ApplyEvent`, `SetConnected`, `UpdateTyping`, `JoinVoice`, `LeaveVoice`, etc. Define query messages: `GetState` (returns `ClientState` clone or specific fields), `GetPeers`, `IsConnected`, etc.

- [ ] **Step 2: Define `TopicListenerActor`.** Replaces `spawn_topic_listener`. Implements `StreamHandler` for gossip events. Holds `Addr<ClientStateActor>` and `TopicHandle`. In `handle_stream_item`, sends mutations to the state actor and emits `ClientEvent`s.

- [ ] **Step 3: Refactor `ClientHandle<N>`.** Replace `shared: Arc<RwLock<SharedState>>` with `state: Addr<ClientStateActor>`. Keep `event_tx: futures_mpsc::UnboundedSender<ClientEvent>` as a plain channel — `ClientEvent`s flow to the UI layer which is not an actor. **Note:** this makes previously-synchronous state accessors async (`shared.read()` → `state.ask().await`). All callers in `lib.rs`, `ops.rs`, `invite.rs`, `files.rs`, and `worker_cache.rs` must be updated to `.await` state queries. Methods that were `fn` become `async fn`.

- [ ] **Step 4: Update `listeners.rs`.** Replace `spawn_topic_listener()` with spawning a `TopicListenerActor` on the system. Remove the manual `topic_listener_loop`.

- [ ] **Step 5: Update client tests.** Tests using `test_client()` helper need updating — shared state access changes from lock-based to ask-based. Verify all `just test-client` tests pass.

- [ ] **Step 6: Run `just test-client` and `just clippy`.** All 93+ client tests must pass with zero warnings.

---

## Task 7: Web UI Migration

**Files:**
- Modify: `crates/web/Cargo.toml`
- Modify: `crates/web/src/app.rs`
- Modify: `crates/web/src/event_processing.rs`

Validates WASM target correctness.

- [ ] **Step 1: Update `app.rs` initialization.** Create a `System`, spawn the `ClientStateActor` and listener actors. Pass `Addr`s into Leptos context instead of `Arc<RwLock<>>`.

- [ ] **Step 2: Update signal updates.** Components that read state via `Arc<RwLock<>>` switch to `ask()` on the state actor address. Leptos signals can be updated from the actor's event stream.

- [ ] **Step 3: Verify WASM compilation.** `just check-wasm` must pass.

- [ ] **Step 4: Run `just test-browser`.** All 39+ browser tests must pass.

---

## Task 8: Final Validation

- [ ] **Step 1: Run `just check`.** Full suite: fmt + clippy + test + WASM. Zero warnings.

- [ ] **Step 2: Run `just test-scale`.** Verify no performance regression in event throughput or merge benchmarks.

- [ ] **Step 3: Run `just test-all`.** All 420+ tests pass.

- [ ] **Step 4: Update `CLAUDE.md`.** Add `crates/actor/` to the repository structure. Update the architecture notes to describe the actor system.
