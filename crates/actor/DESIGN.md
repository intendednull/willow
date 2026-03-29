# willow-actor Design Spec

## Problem

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
    /// Start a periodic interval. Delivers `Tick` messages to the actor.
    /// Returns a handle that can cancel the interval.
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

## Crate Structure

```
crates/actor/
├── Cargo.toml
├── DESIGN.md           (this file)
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
