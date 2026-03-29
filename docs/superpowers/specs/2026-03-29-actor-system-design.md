# Actor System Design Spec

**Date**: 2026-03-29
**Status**: Draft

## Prior Art

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

1. **Hard `Send` bound on `Actor`**: The `Actor` trait requires
   `Send + 'static`. On WASM, all types are trivially `Send` (single
   thread), but this forces `Send` constraints to propagate through the
   entire Willow type graph. Many Willow types use `Rc<RefCell<>>` in
   WASM paths (e.g., `ClientHandle.shared`), which are not `Send`.
   Switching to `Arc<Mutex<>>` everywhere adds overhead on WASM for no
   benefit.
2. **No supervision**: No restart policies or supervisor trees. Actors
   that panic are simply gone.
3. **No `Recipient<M>` / type-erased message targets**: xtra has
   `MessageChannel` but it's less ergonomic than a standalone
   `Recipient<M>` type for pub-sub patterns.
4. **No interval support**: No built-in periodic tick mechanism. The
   heartbeat/sync actors would still need manual timer loops.
5. **No `MaybeSend` pattern**: The `Send` bound is unconditional. Our
   design needs conditional `Send` to avoid unnecessary synchronization
   on WASM.
6. **Low activity**: Last release Feb 2024, limited maintenance signal.

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
flags for alternative runtimes. Would need forking to add WASM.

### Recommendation: build `willow-actor`

No existing crate satisfies all requirements (dual-target with
conditional Send, supervision, intervals, stream handlers, per-message
handlers). The design below combines:

- **xtra/kameo's `Handler<M>` pattern** — per-message-type trait impls
  with typed returns, not a single enum
- **ractor's `concurrency` module approach** — platform-abstracted
  spawn/channel/timer with cfg-switched backends
- **New: `MaybeSend`** — conditional `Send` bounds dropped on WASM
- **New: supervision, intervals, `Recipient<M>`** — features missing
  from xtra

---

## Overview

Willow has five different channel/concurrency patterns across its crates:

| Layer | Channels | Target |
|-------|----------|--------|
| Bevy bridge | `std::sync::mpsc` | native |
| libp2p node | `tokio::sync::mpsc` (native) / `futures::channel::mpsc` (WASM) | both |
| Client lib | `futures::channel::mpsc` | both |
| Worker actors | `tokio::sync::mpsc` + `oneshot` + `watch` | native only |
| Web UI | `futures::channel::mpsc` + `spawn_local` | WASM only |

The worker crate already uses an actor pattern (state, network, heartbeat,
sync actors communicating via channels), but it's hand-rolled, tokio-only,
and not reusable. Every other crate reinvents the same pattern: spawn a
task, create channels, loop on `select!`, handle shutdown.

`willow-actor` formalizes this into a single crate that works on both
native and WASM, eliminating the per-crate boilerplate while preserving
the existing architecture's strengths.

## Goals

1. **Dual-target**: native (tokio) + WASM (wasm-bindgen-futures), single API
2. **Typed mailboxes**: each actor defines its message type, no `Box<dyn Any>`
3. **Request-reply**: first-class `ask()` with typed responses, no manual oneshot wiring
4. **Supervision**: restart policies for crashed actors (native), error propagation (WASM)
5. **Lightweight**: no `Arc<Mutex<>>` in the hot path, no dynamic dispatch on send
6. **Incremental adoption**: existing crates can migrate one actor at a time

## Non-Goals

- Distributed actors / remote messaging (libp2p handles that)
- Actor persistence / event sourcing (willow-state handles that)
- Replacing Bevy's ECS (the bridge stays, but becomes thinner)

## Core Types

### Message Trait

```rust
/// Marker trait for actor messages. Must be Send on native.
/// On WASM, Send is not required since everything is single-threaded.
pub trait Message: 'static + MaybeSend {
    /// The response type for request-reply. Use `()` for fire-and-forget.
    type Result: 'static + MaybeSend;
}
```

`MaybeSend` is a conditional trait alias:

```rust
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}
```

### Actor Trait

```rust
/// An actor processes messages sequentially in its own task.
#[async_trait(?Send)]  // ?Send for WASM compat
pub trait Actor: 'static + MaybeSend + Sized {
    /// Called once when the actor starts, before processing messages.
    async fn started(&mut self, ctx: &mut Context<Self>) {}

    /// Called when the actor is stopping (mailbox closed or explicit stop).
    async fn stopped(&mut self) {}
}
```

### Handler Trait

```rust
/// Implement Handler<M> for each message type an actor accepts.
#[async_trait(?Send)]
pub trait Handler<M: Message>: Actor {
    async fn handle(&mut self, msg: M, ctx: &mut Context<Self>) -> M::Result;
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
    stop_flag: bool,
}

impl<A: Actor> Context<A> {
    /// Get this actor's own address (for self-sends or passing to children).
    pub fn address(&self) -> Addr<A> { ... }

    /// Spawn a child actor supervised by this actor.
    pub fn spawn<C: Actor>(&self, child: C) -> Addr<C> { ... }

    /// Request a graceful stop after the current message finishes.
    pub fn stop(&mut self) { ... }

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

    /// Spawn with a specific mailbox capacity (default: 256).
    pub fn spawn_with_capacity<A: Actor>(&self, actor: A, capacity: usize) -> Addr<A> { ... }

    /// Get a handle that can be passed to other contexts.
    pub fn handle(&self) -> SystemHandle { ... }

    /// Shut down all actors gracefully.
    pub async fn shutdown(self) { ... }
}
```

## Platform Abstraction

The crate uses a thin `runtime` module to abstract over native vs WASM:

```rust
// crate::runtime (internal)

/// Spawn a future as a background task.
pub fn spawn<F: Future<Output = ()> + MaybeSend + 'static>(fut: F) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::task::spawn(fut);

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_futures::spawn_local(fut);
}

/// One-shot channel (platform-specific).
pub fn oneshot<T: MaybeSend + 'static>() -> (OneshotTx<T>, OneshotRx<T>) {
    #[cfg(not(target_arch = "wasm32"))]
    { /* tokio::sync::oneshot */ }

    #[cfg(target_arch = "wasm32")]
    { /* futures::channel::oneshot */ }
}

/// Bounded MPSC channel.
pub fn channel<T: MaybeSend + 'static>(cap: usize) -> (Sender<T>, Receiver<T>) {
    #[cfg(not(target_arch = "wasm32"))]
    { /* tokio::sync::mpsc */ }

    #[cfg(target_arch = "wasm32")]
    { /* futures::channel::mpsc */ }
}

/// Sleep for a duration (native: tokio::time::sleep, WASM: gloo_timers).
pub async fn sleep(duration: Duration) { ... }
```

## Mailbox Internals

Each actor gets a mailbox backed by a bounded MPSC channel. Messages are
type-erased inside the mailbox using a closure-based envelope pattern:

```rust
// Internal — not part of the public API.

type BoxEnvelope<A> = Box<dyn FnOnce(&mut A, &mut Context<A>) -> BoxFuture<'_, ()> + MaybeSend>;

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

On WASM, `RestartPolicy::OnFailure` and `Backoff` still work but panics
are caught via `std::panic::catch_unwind` only if the actor is
`UnwindSafe`. Otherwise, `Never` is the only safe option on WASM.

## Streams

Actors can subscribe to external event streams (e.g., network events,
timers) that feed into their mailbox:

```rust
#[async_trait(?Send)]
pub trait StreamHandler<S: 'static + MaybeSend>: Actor {
    async fn handle_stream_item(&mut self, item: S, ctx: &mut Context<Self>);

    /// Called when the stream ends.
    async fn stream_finished(&mut self, _ctx: &mut Context<Self>) {}
}

impl<A: Actor> Context<A> {
    /// Attach a stream to this actor. Items are delivered as messages.
    pub fn add_stream<S, St>(&mut self, stream: St)
    where
        A: StreamHandler<S>,
        S: 'static + MaybeSend,
        St: Stream<Item = S> + MaybeSend + 'static,
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
        msg_factory: impl Fn() -> M + MaybeSend + 'static,
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
    #[error("actor mailbox is full")]
    Full(M),
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
    ├── actor.rs        — Actor, Handler, StreamHandler traits
    ├── addr.rs         — Addr<A>, AnyAddr, Recipient<M>
    ├── context.rs      — Context<A>, interval, stream attachment
    ├── envelope.rs     — BoxEnvelope, type-erased message dispatch
    ├── mailbox.rs      — bounded channel wrapper, recv loop
    ├── message.rs      — Message trait, MaybeSend
    ├── runtime.rs      — platform abstraction (spawn, channel, sleep)
    ├── supervisor.rs   — RestartPolicy, supervised spawn
    ├── system.rs       — System, SystemHandle
    └── error.rs        — SendError, AskError
```

## Dependency Graph

```
willow-actor (new)
├── futures-core        (Stream trait)
├── async-trait
├── thiserror
├── tracing
├── cfg-if
├── [native] tokio      (spawn, mpsc, oneshot, sleep)
└── [wasm]   wasm-bindgen-futures, futures-channel, gloo-timers
```

`willow-actor` has **no dependency on any other willow crate**. It is a
pure infrastructure crate.

## Migration Path

### Phase 1: New crate, worker migration

Create `crates/actor/` with the core types. Migrate the worker crate's
four actors to use `willow-actor`:

**Before** (current `crates/worker/src/actors/state.rs`):
```rust
pub async fn run(mut role: Box<dyn WorkerRole>, mut rx: mpsc::Receiver<StateMsg>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            StateMsg::Event(event) => role.on_event(&event),
            StateMsg::Request { req, reply } => {
                let response = role.handle_request(req);
                let _ = reply.send(response);
            }
            StateMsg::Shutdown => break,
        }
    }
}
```

**After**:
```rust
pub struct StateActor {
    role: Box<dyn WorkerRole>,
}

impl Actor for StateActor {}

impl Handler<EventMsg> for StateActor {
    async fn handle(&mut self, msg: EventMsg, _ctx: &mut Context<Self>) {
        self.role.on_event(&msg.0);
    }
}

impl Handler<RequestMsg> for StateActor {
    async fn handle(&mut self, msg: RequestMsg, _ctx: &mut Context<Self>) -> WorkerResponse {
        self.role.handle_request(msg.0)
    }
}
```

**Before** (current `crates/worker/src/runtime.rs`):
```rust
let (state_tx, state_rx) = mpsc::channel::<StateMsg>(256);
let (network_tx, network_rx) = mpsc::channel::<NetworkOutMsg>(256);
let (shutdown_tx, shutdown_rx) = watch::channel(false);

let state_handle = tokio::spawn(state::run(role, state_rx));
let heartbeat_handle = tokio::spawn(heartbeat::run(..., shutdown_rx.clone()));
// ... manual join + shutdown
```

**After**:
```rust
let system = System::new();
let state_addr = system.spawn(StateActor { role });
let network_addr = system.spawn(NetworkActor::new(node, events, state_addr.clone()));
let _heartbeat = system.spawn(HeartbeatActor::new(peer_id, state_addr.clone(), network_addr.clone()));
let _sync = system.spawn(SyncActor::new(peer_id, state_addr, network_addr));

tokio::signal::ctrl_c().await?;
system.shutdown().await;
```

### Phase 2: Client library

Replace `ClientHandle`'s `futures::channel::mpsc` pair with actor addresses.
The `ClientEventLoop` becomes an actor with `StreamHandler<NetworkEvent>`.

### Phase 3: Network bridge

The Bevy bridge becomes a thin adapter: a Bevy system polls a `Receiver`
that an actor feeds. The bridge actor replaces `run_network()`.

### Phase 4: Web UI

The Leptos event loop (`spawn_local` + `futures::channel::mpsc`) becomes
a `StreamHandler` on a UI actor. Signal updates happen in the handler.

## Open Questions

1. **Backpressure policy**: When a mailbox is full, should `send()` drop
   the message (lossy), block (native only), or return an error? Current
   design returns `SendError::Full`. An `async fn send_async()` that
   awaits capacity could be added for native.

2. **Priority messages**: Should shutdown/stop bypass the queue? Current
   design: no, messages are FIFO. Shutdown is just another message. The
   `Context::stop()` flag is checked between messages.

3. **Actor state snapshots**: Should there be a way to query an actor's
   internal state for debugging/metrics? Could add an optional
   `Inspect` trait that serializes state, but this risks breaking
   encapsulation.

4. **Bounded vs unbounded mailboxes**: The current network layers use
   unbounded channels to avoid dropping gossipsub messages. Should
   `System::spawn_unbounded()` be offered? Probably yes, with a lint
   warning in docs.
