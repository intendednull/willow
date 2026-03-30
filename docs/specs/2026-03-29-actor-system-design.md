# Actor System Design Spec

**Date**: 2026-03-29
**Status**: Ready for implementation
**Depends on**: ~~iroh integration~~ (complete — merged to main)

## Existing Solutions

A survey of existing Rust actor crates was conducted to determine whether
an off-the-shelf solution could be adopted. Summary:

| Crate | Version | WASM | Send req | Supervision | Handlers | Status |
|-------|---------|------|----------|-------------|----------|--------|
| **ractor** | 0.15.12 | **Yes** (`tokio_with_wasm`) | Send+Sync | Yes (Erlang) | Single `Msg` enum | Active (2026-03) |
| **kameo** | 0.19.2 | No (tokio) | Send | Yes (OneForOne) | Per-message `Message<M>` | Active (2025-11) |
| **actix** | 0.13.5 | No (tokio) | Unpin (no Send) | Basic | Per-message `Handler<M>` | Passive |
| **xtra** | 0.6.0 | **Yes** (`wasm_bindgen`) | Send | No | Per-message `Handler<M>` | Low (2024-02) |
| **coerce** | 0.8.11 | No (tokio full) | Send+Sync | Yes | Per-message `Handler<M>` | Dormant (2023) |
| **xactor** | 0.7.11 | No | Send | No | Per-message `Handler<M>` | Dead (2020) |
| **xtor** | 0.9.10 | **Yes** (`wasm_bindgen`) | Send | Yes | Per-message handler | Dead (2022) |
| **stakker** | 0.2.14 | Provisional | Not Send | No | Macro-based callbacks | Niche |

### ractor — first-class WASM, Erlang-style

[ractor](https://github.com/slawlor/ractor) (546k downloads, MIT) is
the most actively maintained option and has **first-class WASM support**
with 84 passing browser tests. Key details:

- **WASM runtime**: Uses `tokio_with_wasm` (a shim that provides
  tokio-compatible channels/spawn/timers on `wasm32-unknown-unknown`
  backed by the JS event loop). Platform abstraction lives in a
  `concurrency` module with three backends: `tokio_primitives`,
  `async_std_primitives`, `wasm_browser_primitives`.
- **Erlang-style API**: Each actor declares a single `type Msg` enum.
  The `handle()` method pattern-matches on it. State is separated from
  the handler (`&self` + `&mut State`).
- **Supervision**: `spawn_linked()` establishes parent-child links.
  `SupervisionEvent` notifies parents of child panics/deaths. No
  built-in restart policies — left to the `handle_supervisor_evt` impl
  (like Erlang's custom supervisor).
- **Request-reply**: `RpcReplyPort<T>` for typed replies. `call()` and
  `cast()` for ask/tell patterns.

**Why not adopt ractor directly:**

1. **Single-enum message type**: `type Msg: Message` requires one enum
   per actor for all message types. This means every actor needs a
   hand-written `match` over its message enum in `handle()`, and adding
   a new message type requires modifying the enum + the match arm. With
   per-message `Handler<M>` traits, new message types are additive (just
   implement another trait). For Willow's actors that handle 5-10+
   message types each, the enum approach produces large match blocks.
2. **Separated `&self` + `&mut State`**: The actor handler is immutable;
   mutable state lives in a separate `State` type. This is idiomatic
   Erlang but awkward in Rust — fields that logically belong together
   (e.g., a `WorkerRole` + its config) are split across two types.
3. **Hard `Send + Sync` on `Actor`**: Requires all actor types to be
   `Send + Sync`. This is stricter than necessary — actors are
   single-owner by design, so `Sync` is never needed.
4. **`tokio_with_wasm` dependency**: Pulls in a full tokio-compatible
   shim for WASM. Willow already uses `futures::channel::mpsc` and
   `wasm_bindgen_futures::spawn_local` directly — adding another layer
   of abstraction over tokio's API on WASM is unnecessary indirection.
5. **Heavy dependency tree**: `dashmap`, `bon`, `strum`, `once_cell`,
   plus the full `tokio_with_wasm` crate on WASM. Willow's actor system
   needs only channels, oneshot, and spawn.

### xtra — per-message handlers, lightweight

[xtra](https://github.com/Restioson/xtra) (83k downloads, MPL-2.0) is
the closest match to our desired API shape:

- **Multi-runtime**: tokio, async-std, smol, and `wasm_bindgen` via
  feature flags. WASM spawns use `wasm_bindgen_futures::spawn_local`.
- **Per-message `Handler<M>`**: Each message type gets its own `Handler`
  impl with `type Return`. Request-reply via `Address::send()` returning
  a `SendFuture` that resolves to the handler's return value.
- **Lightweight**: core deps are `catty`, `futures-core`, `event-listener`,
  `spin`. No proc macros required (optional `xtra-macros`).
- **Actor lifecycle**: `started(&mut self, &Mailbox)` and
  `stopped(self) -> Self::Stop`.
- **Address/Mailbox split**: `Address<A>` for sending, `Mailbox<A>` for
  the actor's receive loop.

**Why not adopt xtra directly:**

1. **No supervision**: No restart policies or supervisor trees. Actors
   that panic are simply gone.
2. **No `Recipient<M>` / type-erased message targets**: xtra has
   `MessageChannel` but it's less ergonomic than a standalone
   `Recipient<M>` type for pub-sub patterns.
3. **No interval support**: No built-in periodic tick mechanism. The
   heartbeat/sync actors would still need manual timer loops.
4. **Low activity**: Last release Feb 2024, limited maintenance signal.

### kameo — best API shape, no WASM

[kameo](https://github.com/tqwewe/kameo) (190k downloads, MIT) has the
cleanest API design with per-message `Message<M>` trait impls and
ask/tell naming:

```rust
impl Message<MyMsg> for MyActor {
    type Reply = MyReply;
    async fn handle(&mut self, msg: MyMsg, ctx: &mut Context<..>) -> Self::Reply;
}
```

It has OneForOne supervision, stream attachment, and actor linking. But
it depends on tokio directly with no WASM runtime support and no feature
flags for alternative runtimes.

### kameo fork feasibility

A source audit of kameo's tokio coupling reveals it is **shallow and
concentrated**. All tokio usage falls into 6 primitives across 4 files:

| Primitive | Call sites | WASM replacement |
|-----------|-----------|------------------|
| `tokio::spawn` | 5 | `wasm_bindgen_futures::spawn_local` |
| `tokio::sync::mpsc` (bounded+unbounded) | 1 module (~15 method delegations) | `futures::channel::mpsc` |
| `tokio::sync::Mutex` | 1 | `futures::lock::Mutex` |
| `tokio::sync::SetOnce` | 2 | `OnceCell` or custom |
| `tokio::select!` | 1 | `futures::select!` |
| `tokio::runtime::Handle` | 1 (for `spawn_in_thread`) | `#[cfg(not(wasm32))]` gate |
| `task_local!` | 1 | `thread_local!` (WASM is single-threaded) |

**Total estimated changes**: ~150-200 lines to introduce a `runtime`
abstraction module with `cfg(target_arch = "wasm32")` branches, plus
Cargo.toml feature flag changes. The supervision module has **zero**
production tokio usage. The mailbox module is the densest — it wraps
tokio mpsc types — but it's a clean 1:1 delegation layer that maps
directly to `futures::channel::mpsc`.

**Challenges:**

1. **`Send` bounds everywhere**: kameo requires `Actor: Send + 'static`
   and all futures must be `Send`. On WASM this compiles (everything is
   trivially Send on single-threaded targets) but it forces Willow types
   that currently use `Rc<RefCell<>>` to switch to `Arc<Mutex<>>`. This
   is a Willow-side change, not a kameo fork issue.
2. **`spawn_in_thread()`**: Uses `tokio::runtime::Handle::current()` and
   `std::thread::spawn`. Must be `cfg`-gated out on WASM entirely.
3. **`blocking_send()` / `blocking_recv()`**: These tokio mpsc methods
   have no WASM equivalent. Must be gated or removed on WASM.
4. **Minimum Rust 1.88.0**: kameo requires edition 2024 / Rust 1.88+.
   Willow would need to match this MSRV.
5. **Upstream maintenance**: kameo is actively developed (v0.19.2, last
   commit March 2026). Forking means maintaining divergence or getting
   the runtime abstraction upstreamed.

**Verdict: fork is feasible but not clearly better than writing our own.**

The fork saves ~800 lines of actor machinery (mailbox, supervision,
actor lifecycle) but introduces:
- Ongoing merge burden with an actively evolving upstream
- The `Send` bound issue remains (kameo won't accept a `Send`
  change upstream — it's a fundamental API decision)
- kameo's `remote` feature (libp2p-based distributed actors) would
  conflict with Willow's iroh networking layer
- kameo's dependency on `downcast-rs`, `dyn-clone`, `serde` (with
  derive) adds weight Willow doesn't need

Writing `willow-actor` from scratch is estimated at ~1000-1500 lines
for the core (message, actor, handler, addr, context, mailbox, envelope,
runtime, error modules). This is comparable to the fork effort when
accounting for the abstraction layer + ongoing maintenance cost.

### Iroh integration (complete)

Willow has migrated from libp2p to iroh. The networking layer now uses
trait-based abstractions (`Network`, `TopicHandle`, `TopicEvents`,
`BlobStore`) backed by iroh's QUIC transport and `iroh-gossip` for
broadcast. See `docs/specs/2026-03-29-iroh-migration-design.md`.

**Key facts for the actor system:**

1. **Tokio everywhere.** Iroh depends on tokio for both native and WASM.
   The client already uses `Arc<RwLock<>>` (not `Rc<RefCell<>>`). All
   futures must be `Send`. Channels are `tokio::sync::mpsc` throughout.

2. **`Network` trait is generic.** Workers and client are generic over
   `N: Network`, with `IrohNetwork` for production and `MemNetwork` for
   tests. Actor types should also be generic over `Network` where they
   interact with gossip, following the same pattern.

3. **`TopicEvents` is a stream.** The `TopicEvents` trait has
   `async fn next() -> Option<Result<GossipEvent>>` — a natural fit for
   `StreamHandler`. The network actor currently wraps this in a manual
   `while let` loop; the actor system replaces that.

4. **`TopicHandle` for broadcast.** The heartbeat and sync actors
   currently take `T: TopicHandle` and call `topic.broadcast()`. With
   the actor system, they hold the `TopicHandle` as actor state and
   call it from message handlers.

5. **Shutdown via `watch` channel.** Workers currently use
   `tokio::sync::watch` for shutdown signaling. The actor system
   replaces this with `CancellationToken` (used by iroh internally)
   or simply dropping the `Addr` (closing the mailbox).

### Recommendation: build `willow-actor`

No existing crate satisfies all requirements (dual-target, supervision,
intervals, stream handlers, per-message handlers). The design below
combines:

- **xtra/kameo's `Handler<M>` pattern** — per-message-type trait impls
  with typed returns, not a single enum
- **tokio channels directly** — no runtime abstraction needed since iroh
  already provides tokio on both native and WASM
- **`Send` unconditionally** — matches iroh's requirement, compiles on
  WASM (everything is trivially Send on single-threaded targets)
- **`CancellationToken` for lifecycle** — aligns with iroh's shutdown
  pattern
- **Supervision, intervals, `Recipient<M>`** — features missing from
  xtra

---

## Overview

With the iroh migration complete, Willow's networking is now trait-based
(`Network`, `TopicHandle`, `TopicEvents`) and generic — but the
concurrency patterns above the network layer are still hand-rolled:

| Layer | Current pattern | Problem |
|-------|----------------|---------|
| Worker actors | `tokio::sync::mpsc` + `oneshot` + `watch`, 4 manual loops | Not reusable, manual shutdown via watch channel |
| Client lib | `Arc<RwLock<SharedState>>` + `futures::channel::mpsc` | Shared mutable state behind locks, monolithic event loop |
| Web UI | `futures::channel::mpsc` + `spawn_local` | Duplicates client event loop logic |

The worker crate already uses an actor pattern (state, network, heartbeat,
sync actors communicating via channels), but it's hand-rolled and not
reusable. The client and web crates reinvent the same pattern: spawn a
task, create channels, loop on `select!`, handle shutdown.

`willow-actor` formalizes this into a single crate, building on iroh's
tokio runtime (available on both native and WASM) to eliminate the
per-crate boilerplate.

## Goals

1. **Dual-target**: native (tokio) + WASM (wasm-bindgen-futures), single API
2. **Typed mailboxes**: each actor defines its message type, no `Box<dyn Any>`
3. **Request-reply**: first-class `ask()` with typed responses, no manual oneshot wiring
4. **Supervision**: restart policies for crashed actors (native), error propagation (WASM)
5. **No locks**: shared state lives inside actors, eliminating `Arc<Mutex<>>` / `Arc<RwLock<>>` — access is serialized through message passing
6. **Lightweight**: no dynamic dispatch on send

## Non-Goals

- Distributed actors / remote messaging (iroh gossip handles that)
- Actor persistence / event sourcing (willow-state handles that)
- Bevy desktop app (out of scope for this migration)

## Core Types

### Message Trait

```rust
/// Marker trait for actor messages.
pub trait Message: Send + 'static {
    /// The response type for request-reply. Use `()` for fire-and-forget.
    type Result: Send + 'static;
}
```

`Send` is required unconditionally. On WASM (single-threaded), all types
are trivially `Send`, so this compiles without issue. This matches iroh's
requirement that all futures and channel payloads are `Send`.

### Actor Trait

```rust
/// An actor processes messages sequentially in its own task.
pub trait Actor: Send + 'static + Sized {
    /// Called once when the actor starts, before processing messages.
    fn started(&mut self, ctx: &mut Context<Self>)
        -> impl Future<Output = ()> + Send { async {} }

    /// Called when the actor is stopping (mailbox closed or explicit stop).
    fn stopped(&mut self)
        -> impl Future<Output = ()> + Send { async {} }
}
```

Uses RPITIT (return-position impl trait in trait, stabilized in Rust
1.75) instead of `async_trait` — avoids the proc macro dependency and
Box allocation per handler call.

### Handler Trait

```rust
/// Implement Handler<M> for each message type an actor accepts.
pub trait Handler<M: Message>: Actor {
    fn handle(&mut self, msg: M, ctx: &mut Context<Self>)
        -> impl Future<Output = M::Result> + Send;
}
```

An actor can implement `Handler` for multiple message types. Each handler
is type-checked at compile time.

### Context

```rust
/// Provided to handlers — gives access to the actor's own address and system.
pub struct Context<A: Actor> {
    addr: Addr<A>,
    system: SystemHandle,
    cancel: CancellationToken,
}

impl<A: Actor> Context<A> {
    /// Get this actor's own address (for self-sends or passing to children).
    pub fn address(&self) -> Addr<A> { ... }

    /// Spawn a child actor supervised by this actor.
    pub fn spawn<C: Actor>(&self, child: C) -> Addr<C> { ... }

    /// Request a graceful stop after the current message finishes.
    pub fn stop(&mut self) { ... }

    /// Get the cancellation token (child of the system's root token).
    /// Integrates with iroh's CancellationToken-based shutdown.
    pub fn cancellation_token(&self) -> &CancellationToken { ... }

    /// Access the actor system (for spawning unrelated actors).
    pub fn system(&self) -> &SystemHandle { ... }
}
```

### Addr (Actor Address / Handle)

```rust
/// Type-safe handle for sending messages to an actor.
/// Cheaply cloneable (wraps an Arc'd channel sender).
pub struct Addr<A: Actor> {
    tx: MessageSender,       // platform-specific channel sender
    _phantom: PhantomData<A>,
}

impl<A: Actor> Addr<A> {
    /// Fire-and-forget: send a message, don't wait for a response.
    /// Returns Err if the actor's mailbox is closed.
    pub fn send<M>(&self, msg: M) -> Result<(), SendError<M>>
    where
        A: Handler<M>,
        M: Message<Result = ()>,
    { ... }

    /// Request-reply: send a message and await the response.
    /// Returns a future that resolves to M::Result.
    pub fn ask<M>(&self, msg: M) -> impl Future<Output = Result<M::Result, AskError>>
    where
        A: Handler<M>,
        M: Message,
    { ... }

    /// Check if the actor is still alive.
    pub fn is_alive(&self) -> bool { ... }
}

impl<A: Actor> Clone for Addr<A> { ... }
```

### AnyAddr (Type-Erased Address)

For cases where you need to store addresses of different actor types
together (e.g. a supervisor tracking children):

```rust
/// Type-erased actor address. Can send shutdown signals but not typed messages.
pub struct AnyAddr { ... }

impl AnyAddr {
    pub fn stop(&self) { ... }
    pub fn is_alive(&self) -> bool { ... }
}

impl<A: Actor> From<Addr<A>> for AnyAddr { ... }
```

### Recipient (Multi-Actor Message Target)

For when multiple actor types handle the same message and you want to
abstract over the concrete actor:

```rust
/// Type-erased handle that can send a specific message type.
/// Useful for pub-sub patterns where the sender doesn't know the actor type.
pub struct Recipient<M: Message> {
    tx: Box<dyn RecipientSender<M>>,
}

impl<M: Message> Recipient<M> {
    pub fn send(&self, msg: M) -> Result<(), SendError<M>> { ... }
    pub fn ask(&self, msg: M) -> impl Future<Output = Result<M::Result, AskError>> { ... }
}

impl<A, M> From<Addr<A>> for Recipient<M>
where
    A: Handler<M>,
    M: Message,
{ ... }
```

## Actor System

```rust
/// The actor system — owns the runtime and tracks all top-level actors.
pub struct System {
    handle: SystemHandle,
}

/// Cheap cloneable handle into the system.
#[derive(Clone)]
pub struct SystemHandle { ... }

impl System {
    /// Create a new actor system.
    pub fn new() -> Self { ... }

    /// Spawn a top-level actor and return its address.
    pub fn spawn<A: Actor>(&self, actor: A) -> Addr<A> { ... }

    /// Get a handle that can be passed to other contexts.
    pub fn handle(&self) -> SystemHandle { ... }

    /// Shut down all actors gracefully.
    pub async fn shutdown(self) { ... }
}
```

## Platform Abstraction

Since iroh already depends on tokio for both native and WASM, the
runtime module is minimal — only task spawning differs by platform:

```rust
// crate::runtime (internal)

/// Spawn a future as a background task.
pub fn spawn<F: Future<Output = ()> + Send + 'static>(fut: F) {
    #[cfg(not(target_arch = "wasm32"))]
    { tokio::task::spawn(fut); }

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(fut);
}
```

Channels use `tokio::sync::mpsc` and `tokio::sync::oneshot` on both
targets — iroh's WASM shim makes these available. Timers use
`tokio::time::sleep` on native and `gloo_timers` (or iroh's internal
timer abstraction) on WASM.

## Mailbox Internals

Each actor gets a mailbox backed by an unbounded MPSC channel. Messages are
type-erased inside the mailbox using a closure-based envelope pattern:

```rust
// Internal — not part of the public API.

type BoxEnvelope<A> = Box<dyn FnOnce(&mut A, &mut Context<A>) -> BoxFuture<'_, ()> + Send + 'static>;

// When Addr<A>.send(msg) is called for M where A: Handler<M>:
// 1. msg is wrapped in an envelope closure
// 2. The closure calls A::handle(msg, ctx) when executed
// 3. For ask(), a oneshot sender is captured in the closure
//    and the response is sent back through it
```

This means the channel carries `BoxEnvelope<A>` — one channel per actor,
handling all message types. No dynamic dispatch on the sender side; the
dispatch happens once when the envelope is executed.

## Supervision

```rust
/// Restart policy for supervised actors.
pub enum RestartPolicy {
    /// Never restart (default). Errors are logged.
    Never,
    /// Restart immediately on panic/error, up to `max` times.
    OnFailure { max: u32 },
    /// Restart with exponential backoff.
    Backoff { initial: Duration, max_delay: Duration, max_retries: u32 },
}

impl<A: Actor> Context<A> {
    /// Spawn a supervised child actor.
    pub fn spawn_supervised<C: Actor + Clone>(
        &self,
        child: C,
        policy: RestartPolicy,
    ) -> Addr<C> { ... }
}
```

Panics are caught via `std::panic::catch_unwind`. On WASM, this works
only if the actor is `UnwindSafe`; otherwise `Never` is the only safe
option.

## Streams

Actors can subscribe to external event streams (e.g., network events,
timers) that feed into their mailbox:

```rust
pub trait StreamHandler<S: Send + 'static>: Actor {
    fn handle_stream_item(&mut self, item: S, ctx: &mut Context<Self>)
        -> impl Future<Output = ()> + Send;

    /// Called when the stream ends.
    fn stream_finished(&mut self, _ctx: &mut Context<Self>)
        -> impl Future<Output = ()> + Send { async {} }
}

impl<A: Actor> Context<A> {
    /// Attach a stream to this actor. Items are delivered as messages.
    pub fn add_stream<S, St>(&mut self, stream: St)
    where
        A: StreamHandler<S>,
        S: 'static + Send,
        St: Stream<Item = S> + Send + 'static,
    { ... }
}
```

## Intervals

Built-in support for periodic ticks (replaces the manual
`tokio::select! + sleep` pattern in heartbeat/sync actors):

```rust
impl<A: Actor> Context<A> {
    /// Start a periodic interval. Delivers messages to the actor
    /// on each tick. Returns a handle that can cancel the interval.
    pub fn run_interval<M: Message<Result = ()>>(
        &mut self,
        duration: Duration,
        msg_factory: impl Fn() -> M + Send + 'static,
    ) -> IntervalHandle
    where
        A: Handler<M>,
    { ... }
}

pub struct IntervalHandle { ... }
impl IntervalHandle {
    pub fn cancel(self) { ... }
}
```

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum SendError<M> {
    #[error("actor mailbox is closed")]
    Closed(M),
}

#[derive(Debug, thiserror::Error)]
pub enum AskError {
    #[error("actor mailbox is closed")]
    Closed,
    #[error("actor did not respond (dropped the reply channel)")]
    NoResponse,
}
```

## Crate Structure

```
crates/actor/
├── Cargo.toml
└── src/
    ├── lib.rs          — public API re-exports
    ├── actor.rs        — Actor, Handler, StreamHandler, Message traits
    ├── addr.rs         — Addr<A>, AnyAddr, Recipient<M>
    ├── context.rs      — Context<A>, interval, stream attachment
    ├── envelope.rs     — BoxEnvelope, type-erased message dispatch
    ├── mailbox.rs      — tokio mpsc wrapper, recv loop
    ├── runtime.rs      — spawn abstraction (tokio::spawn vs spawn_local)
    ├── supervisor.rs   — RestartPolicy, supervised spawn
    ├── system.rs       — System, SystemHandle (CancellationToken)
    └── error.rs        — SendError, AskError
```

## Dependency Graph

```
willow-actor (new)
├── tokio               (sync: mpsc, oneshot; time: sleep, interval)
├── tokio-util          (CancellationToken)
├── futures-core        (Stream trait)
├── thiserror
├── tracing
└── [wasm] wasm-bindgen-futures  (spawn_local)
```

No `async-trait` needed — uses RPITIT (Rust 1.75+). `willow-actor` has
**no dependency on any other willow crate**. It is a pure infrastructure
crate. It shares `tokio` with iroh — no additional runtime overhead.

## Migration Path

### Phase 1: Core crate + worker migration

Create `crates/actor/` with the core types. Migrate the worker crate's
four hand-rolled actor loops to use `willow-actor`. This is the smallest
useful scope and the cleanest test case — the workers already have
well-defined actor boundaries.

**Before** (current `crates/worker/src/runtime.rs`):
```rust
pub async fn run<N: Network>(role: Box<dyn WorkerRole>, config: WorkerConfig, network: N) {
    let (state_tx, state_rx) = mpsc::channel::<StateMsg>(256);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let state_handle = tokio::spawn(state::run(role, state_rx));
    let network_handle = tokio::spawn(network::run(workers_events, state_tx.clone(), peer_id));
    let heartbeat_handle = tokio::spawn(heartbeat::run(peer_id, ..., workers_sender.clone(), shutdown_rx.clone()));
    let sync_handle = tokio::spawn(sync::run(peer_id, ..., workers_sender, shutdown_rx));
    tokio::signal::ctrl_c().await?;
    let _ = shutdown_tx.send(true);
    let _ = state_tx.send(StateMsg::Shutdown).await;
    let _ = tokio::join!(state_handle, network_handle, heartbeat_handle, sync_handle);
}
```

**After**:
```rust
pub async fn run<N: Network>(role: Box<dyn WorkerRole>, config: WorkerConfig, network: N) {
    let system = System::new();
    let state_addr = system.spawn(StateActor { role });
    let _network = system.spawn(NetworkActor::new(workers_events, state_addr.clone(), peer_id));
    let _heartbeat = system.spawn(HeartbeatActor::new(peer_id, state_addr.clone(), workers_sender.clone()));
    let _sync = system.spawn(SyncActor::new(peer_id, state_addr, workers_sender));
    tokio::signal::ctrl_c().await?;
    system.shutdown().await;
}
```

The `NetworkActor` uses `StreamHandler<GossipEvent>` to receive from
`TopicEvents`. The `HeartbeatActor` uses `ctx.run_interval()` instead
of a manual `tokio::select! + sleep` loop. Shutdown propagates via
`CancellationToken` — no more `watch` channel.

### Phase 2: Client library

Replace `ClientHandle<N>`'s `Arc<RwLock<SharedState>>` with actors.
Shared state moves into a state actor — no more locks. The client event
loop becomes an actor with `StreamHandler` for `TopicEvents`. External
callers use `Addr` to send commands and `ask()` to query state.

### Phase 3: Web UI

The Leptos event loop becomes a `StreamHandler` on a UI actor. Signal
updates happen in the handler. Validates WASM target correctness and
completes the migration across all active crates.

## Decisions

1. **Mailboxes are unbounded.** `send()` returns `Err` only if the
   actor is dead (mailbox closed). Bounded mailboxes can be added later
   if backpressure becomes necessary.

2. **FIFO, no priority messages.** Shutdown is just another message.
   `CancellationToken` provides an independent out-of-band shutdown
   signal that doesn't go through the mailbox.

3. **Shared state lives in actors.** `Arc<RwLock<SharedState>>` in the
   client library is replaced by a state actor. External code queries
   state via `ask()`. This eliminates all locks from the hot path —
   the actor processes messages sequentially, so no synchronization is
   needed inside the actor.
