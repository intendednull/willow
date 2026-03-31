# Actor System Library — Extended Actor Types

**Date**: 2026-03-31
**Status**: Draft
**Issue**: https://github.com/intendednull/willow/issues/17

## Motivation

The `willow-actor` crate provides a solid foundation: `Actor`, `Handler<M>`,
`StreamHandler<S>`, `Addr<A>`, `Recipient<M>`, supervision, and intervals.
But higher-level patterns like state management, derived state, and output
streaming are currently hand-built in downstream crates (`willow-client`,
`willow-web`). These patterns are general-purpose and should live in the
actor crate itself, making it publishable as a standalone library.

### What exists today

| Pattern | Location | Problem |
|---------|----------|---------|
| `ClientStateActor` | `willow-client/src/client_actor.rs` | Hardcoded to `SharedState`, uses `Box<dyn Any>` downcasting |
| `DerivedStateActor<T>` | `willow-web/src/derived.rs` | Coupled to Leptos `WriteSignal`, can only derive from one source |
| Worker `StateActor` | `willow-worker/src/actors/state.rs` | Hand-rolled, no generic get/set/subscribe interface |
| Stream consumption | `StreamHandler<S>` | Can consume streams, but no way to *produce* a stream from an actor |

### Goal

Extract generic versions of these patterns into `willow-actor` so they can
be reused across the codebase and by external consumers. The actor crate
should remain dependency-free (no Leptos, no willow-specific types).

---

## 1. State Actor

A generic actor that owns a value of type `S` and provides a uniform
interface for reading, mutating, and subscribing to changes.

### Design

```rust
/// An actor that owns state of type `S` with get/set/subscribe semantics.
///
/// Mutations are batched — subscriber notifications fire in `idle()` after
/// all pending messages are drained, so a burst of mutations triggers a
/// single notification round.
pub struct StateActor<S: Send + 'static> {
    state: S,
    dirty: bool,
    subscribers: Vec<Recipient<Notify>>,
}
```

### Messages

```rust
/// Get the current state (clones it out). Requires S: Clone.
pub struct Get<S>(PhantomData<S>);
impl<S: Clone + Send + 'static> Message for Get<S> { type Result = S; }

/// Set the state to a new value. Marks dirty.
pub struct Set<S: Send + 'static>(pub S);
impl<S: Send + 'static> Message for Set<S> { type Result = (); }

/// Mutate state via a type-erased closure. Returns a type-erased result.
/// Use the `mutate()` helper for type-safe access.
pub struct Mutate(pub Box<dyn FnOnce(&mut dyn Any) -> Box<dyn Any + Send> + Send>);
impl Message for Mutate { type Result = Box<dyn Any + Send>; }

/// Read state via a type-erased selector. Returns a type-erased result.
/// Use the `select()` helper for type-safe access.
pub struct Select(pub Box<dyn FnOnce(&dyn Any) -> Box<dyn Any + Send> + Send>);
impl Message for Select { type Result = Box<dyn Any + Send>; }

/// Subscribe to state change notifications.
pub struct Subscribe(pub Recipient<Notify>);
impl Message for Subscribe { type Result = (); }

/// Notification sent to subscribers after mutations.
#[derive(Clone)]
pub struct Notify;
impl Message for Notify { type Result = (); }
```

The `Mutate` and `Select` closures use `&dyn Any`/`&mut dyn Any` so the
message types are not generic over `S` — this keeps them usable with
`Recipient` and avoids monomorphization. The `StateActor<S>` handler
downcasts internally. The typed `select()`/`mutate()` helpers hide this.

### Typed helpers

```rust
/// Type-safe read via selector.
pub async fn select<S, T>(addr: &Addr<StateActor<S>>, f: impl FnOnce(&S) -> T) -> T
where S: Send + 'static, T: Send + 'static { ... }

/// Type-safe mutation with return value.
pub async fn mutate<S, T>(addr: &Addr<StateActor<S>>, f: impl FnOnce(&mut S) -> T) -> T
where S: Send + 'static, T: Send + 'static { ... }

/// Subscribe any actor that handles Notify to state changes.
pub fn subscribe<S, A>(state: &Addr<StateActor<S>>, subscriber: &Addr<A>)
where
    S: Send + 'static,
    A: Handler<Notify>,
{
    let recipient: Recipient<Notify> = subscriber.clone().into();
    let _ = state.do_send(Subscribe(recipient));
}
```

### Lifecycle

- `idle()`: if `dirty`, sends `Notify` to all subscribers, resets flag.
  This batches a burst of `Set`/`Mutate` messages into one notification.
- Subscribers that are dead (send fails) are pruned on each notify cycle.

### Example

```rust
let system = System::new();
let state = system.spawn(StateActor::new(AppState::default()));

// Read a slice
let count = select(&state, |s| s.message_count).await;

// Mutate
mutate(&state, |s| s.message_count += 1).await;

// Subscribe
subscribe(&state, &my_addr);
```

### Why generic over `S`

The current `ClientStateActor` is hardcoded to `SharedState`. The worker
`StateActor` is hardcoded to `Box<dyn WorkerRole>`. Both are the same
pattern: own state, provide read/mutate, notify subscribers. A generic
`StateActor<S>` eliminates this duplication. Downstream crates parameterize
with their specific state type.

---

## 2. Derived Actor

A generic actor that derives its value from one or more source actors.
Subscribes to source notifications, runs a combiner/selector, caches
the result, and only notifies downstream when the value changes.

### Design (tuple-based, Bevy-style)

A single `DerivedActor<Src, T>` is generic over a `DeriveSource` which
can be a single `StateRef<S>` or a tuple of them. A `DeriveSource` trait
is implemented for single sources and tuples of sources (up to arity 6),
similar to how Bevy implements `WorldQuery` for tuples.

```rust
/// A type-erased, cloneable handle to any observable actor.
/// Supports subscribing to notifications and reading state via selector.
///
/// This is the key to composition — both StateActor and DerivedActor
/// produce StateRef handles, so they're interchangeable as sources.
/// Clone is cheap (Arc bumps).
#[derive(Clone)]
pub struct StateRef<S: Send + 'static> {
    /// Subscribe to change notifications.
    subscribe: Arc<dyn Fn(Recipient<Notify>) + Send + Sync>,
    /// Read current state via type-erased selector closure.
    select: Arc<dyn Fn(Box<dyn FnOnce(&dyn Any) -> Box<dyn Any + Send> + Send>) -> Pin<Box<dyn Future<Output = Box<dyn Any + Send>> + Send>> + Send + Sync>,
}

impl<S: Send + 'static> StateRef<S> {
    /// Create from any Addr whose actor handles Subscribe + Select.
    pub fn from_addr<A>(addr: Addr<A>) -> Self
    where A: Handler<Subscribe> + Handler<Select> { ... }

    /// Subscribe a recipient to change notifications.
    pub fn subscribe(&self, recipient: Recipient<Notify>) { ... }

    /// Read the full state (clone). Sugar for `select(|s| s.clone())`.
    /// Requires S: Clone.
    pub async fn get(&self) -> S where S: Clone { ... }

    /// Read a slice of state via selector.
    pub async fn select<T: Send + 'static>(&self, f: impl FnOnce(&S) -> T + Send + 'static) -> T { ... }
}

impl<S: Clone + Send + 'static> From<&Addr<StateActor<S>>> for StateRef<S> { ... }
```

#### Prerequisite: make `Recipient<M>` cloneable

The existing `Recipient<M>` wraps `Box<dyn RecipientSender<M> + Send>`.
Multi-source derived actors need to subscribe the same recipient to
multiple sources. This requires changing the internals from `Box` to
`Arc` so `Recipient` implements `Clone`. This is a backwards-compatible
change — `Arc<dyn Trait>` supports all the same operations.

#### DeriveSource trait

```rust
/// Trait for single sources and tuples of sources.
/// Implemented for StateRef<S> and tuples (StateRef<S1>, StateRef<S2>), etc.
pub trait DeriveSource: Clone + Send + 'static {
    /// The combined snapshot type passed to the selector.
    type Snapshot: Send + 'static;

    /// Subscribe to all sources.
    fn subscribe_all(&self, recipient: Recipient<Notify>);

    /// Fetch current values from all sources.
    fn snapshot(&self) -> impl Future<Output = Self::Snapshot> + Send;
}

// Single source — snapshot is just S itself.
impl<S: Clone + Send + 'static> DeriveSource for StateRef<S> {
    type Snapshot = S;
    fn subscribe_all(&self, recipient: Recipient<Notify>) { ... }
    async fn snapshot(&self) -> S { ... }
}

// Two sources — snapshot is a tuple (S1, S2).
impl<S1, S2> DeriveSource for (StateRef<S1>, StateRef<S2>)
where
    S1: Clone + Send + 'static,
    S2: Clone + Send + 'static,
{
    type Snapshot = (S1, S2);
    fn subscribe_all(&self, recipient: Recipient<Notify>) {
        // Recipient must be Clone — requires changing internals from Box to Arc.
        self.0.subscribe(recipient.clone());
        self.1.subscribe(recipient);
    }
    async fn snapshot(&self) -> (S1, S2) {
        let (a, b) = futures::join!(self.0.get(), self.1.get());
        (a, b)
    }
}

// Three sources — snapshot is (S1, S2, S3). And so on up to arity 6.
// A macro generates these impls.
macro_rules! impl_derive_source_tuple {
    ($($idx:tt: $S:ident),+) => { ... }
}
impl_derive_source_tuple!(0: S1, 1: S2);
impl_derive_source_tuple!(0: S1, 1: S2, 2: S3);
impl_derive_source_tuple!(0: S1, 1: S2, 2: S3, 3: S4);
// ... up to 6
```

#### Unified DerivedActor

```rust
/// Derives a value of type `T` from one or more source actors.
pub struct DerivedActor<Src: DeriveSource, T: PartialEq + Clone + Send + 'static> {
    sources: Src,
    selector: Arc<dyn Fn(&Src::Snapshot) -> T + Send + Sync>,
    cached: Option<T>,
    subscribers: Vec<Recipient<Notify>>,
    dirty: bool,
}
```

On receiving `Notify` from any source:
1. Calls `sources.snapshot()` to fetch current values from all sources
2. Runs `selector(&snapshot)` to compute the derived value
3. Sends the result back to self via an internal `UpdateCache<T>` message

The `UpdateCache<T>` handler (synchronous, no await):
4. Compares new value with `cached` via `PartialEq`
5. If changed: updates `cached`, marks `dirty`
6. In `idle()`: notifies own subscribers (same batching as `StateActor`)

**Why the self-message?** RPITIT handlers return `impl Future + Send` which
cannot borrow `&mut self` across an `.await` point. The `snapshot()` call
is async, so we can't update `self.cached` after it completes. The
self-message pattern splits the work: the async part runs in the Notify
handler's future, then the synchronous cache update happens in a separate
handler invocation with full `&mut self` access.

### Chaining

`DerivedActor` produces a `StateRef<T>` just like `StateActor`, so it can
be used as a source for another `DerivedActor`. This forms a reactive
computation graph:

```
StateActor<AppState>
    ├── DerivedActor (peers list)
    │       └── DerivedActor (online count)
    └── DerivedActor (channels list)
            └── DerivedActor (unread counts)
```

### Convenience constructor

A single `derived()` function handles all arities via the `DeriveSource`
trait:

```rust
/// Create a derived actor. Works with single sources or tuples.
pub fn derived<Src, T>(
    system: &SystemHandle,
    sources: Src,
    selector: impl Fn(&Src::Snapshot) -> T + Send + Sync + 'static,
) -> StateRef<T>
where
    Src: DeriveSource,
    T: PartialEq + Clone + Send + 'static,
```

### Usage

```rust
let app_state: StateRef<AppState> = system.spawn(StateActor::new(AppState::default())).into();
let net_state: StateRef<NetState> = system.spawn(StateActor::new(NetState::default())).into();

// Single source — selector receives &AppState
let peers = derived(&system, app_state.clone(), |s| s.peers.clone());

// Two sources — selector receives &(AppState, NetState)
let dashboard = derived(&system, (app_state.clone(), net_state.clone()), |(app, net)| {
    DashboardData {
        peer_count: app.peers.len(),
        connected: net.is_connected,
    }
});

// Chain: derive from another derived
let online_count = derived(&system, peers.clone(), |peers| {
    peers.iter().filter(|p| p.online).count()
});
```

---

## 3. Stream Actor

An actor whose output can be consumed as a `futures::Stream`. This is the
inverse of `StreamHandler` (which consumes streams). A stream actor
*produces* values that external code can iterate over asynchronously.

### Design

```rust
/// Multi-consumer output stream, held as a field inside any actor.
///
/// Call `emit()` from message handlers to push values to all active
/// stream consumers. Call `subscribe()` to create new consumers.
pub struct StreamOutput<T: Send + Clone + 'static> {
    subscribers: Vec<runtime::Sender<T>>,
}
```

### How it works

Any actor that wants to produce a stream holds a `StreamOutput<T>` and
calls `emit()` to push values:

```rust
impl<T: Send + Clone + 'static> StreamOutput<T> {
    /// Push a value to all active stream consumers.
    /// Dead consumers (closed channels) are pruned.
    pub fn emit(&mut self, value: T) { ... }

    /// Create a new stream consumer. Returns a Stream<Item = T>.
    pub fn subscribe(&mut self) -> OutputStream<T> { ... }
}

/// A stream of values produced by an actor.
/// Implements `futures::Stream<Item = T>`.
pub struct OutputStream<T> {
    rx: runtime::Receiver<T>,
}
```

### Usage pattern

```rust
struct SensorActor {
    output: StreamOutput<f64>,
}

impl Actor for SensorActor {
    fn started(&mut self, ctx: &mut Context<Self>) {
        ctx.run_interval(Duration::from_secs(1), || ReadSensor);
    }
}

impl Handler<ReadSensor> for SensorActor {
    fn handle(&mut self, _msg: ReadSensor, _ctx: &mut Context<Self>) -> impl Future<Output = ()> + Send {
        let value = read_sensor();
        self.output.emit(value);
        async {}
    }
}

// Consumer side:
let sensor = system.spawn(SensorActor { output: StreamOutput::new() });

// Option A: attach to another actor via StreamHandler
let stream = sensor.ask(SubscribeStream::default()).await?;
ctx.add_stream(stream); // items delivered via StreamHandler<f64>

// Option B: consume in plain async code
let mut stream = sensor.ask(SubscribeStream::default()).await?;
while let Some(value) = stream.next().await {
    println!("sensor: {value}");
}
```

### SubscribeStream message

To get a stream subscription from outside, actors expose a standard message:

```rust
pub struct SubscribeStream<T: Send + 'static>(PhantomData<T>);
impl<T: Send + 'static> Message for SubscribeStream<T> {
    type Result = OutputStream<T>;
}
```

The actor's `Handler<SubscribeStream<T>>` calls `self.output.subscribe()`.

### Backpressure

Output streams use bounded channels. If a consumer falls behind, new
values are dropped (via `try_send`) rather than blocking the actor. The
actor logs a warning on drop so slow consumers are visible. The buffer
size is configurable at subscribe time:

```rust
impl<T: Send + Clone + 'static> StreamOutput<T> {
    /// Subscribe with a custom buffer size (default: 64).
    pub fn subscribe_with_capacity(&mut self, capacity: usize) -> OutputStream<T> { ... }
}
```

---

## 4. Pub/Sub Broker

A generic topic-based publish/subscribe actor. Useful for decoupling
producers and consumers that don't need to know about each other.

### Design

```rust
/// Topic-based pub/sub broker.
///
/// Publishers send `Publish<T>` messages. Subscribers register with
/// `BrokerSubscribe<T>` and receive values via `Recipient<T>`.
pub struct Broker<T: Message<Result = ()> + Clone + Send + 'static> {
    subscribers: Vec<(SubscriptionId, Recipient<T>)>,
    next_id: u64,
}
```

### Messages

```rust
/// Opaque subscription ID returned by BrokerSubscribe.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

/// Publish a value to all subscribers.
pub struct Publish<T>(pub T);
impl<T: Message<Result = ()> + Clone> Message for Publish<T> {
    type Result = ();
}

/// Subscribe to receive published values. Returns an ID for unsubscribing.
pub struct BrokerSubscribe<T: Message<Result = ()>>(pub Recipient<T>);
impl<T: Message<Result = ()>> Message for BrokerSubscribe<T> {
    type Result = SubscriptionId;
}

/// Unsubscribe from the broker by ID.
pub struct BrokerUnsubscribe(pub SubscriptionId);
impl Message for BrokerUnsubscribe {
    type Result = ();
}
```

Dead subscribers (closed channels) are also pruned automatically on each
`Publish`, so explicit unsubscribe is optional.

### Why separate from StateActor

`StateActor` notifies subscribers that *something changed* (via `Notify`),
then subscribers pull the new value via `select()`. The broker pushes the
actual values directly. This is better for event-like data (chat messages,
connection events, errors) where there's no persistent state to query —
just a stream of occurrences.

### Example

```rust
let broker: Addr<Broker<ChatMessage>> = system.spawn(Broker::new());

// Subscribe (ask returns SubscriptionId for later unsubscribe)
let recipient: Recipient<ChatMessage> = my_actor.into();
let sub_id = broker.ask(BrokerSubscribe(recipient)).await?;

// Publish
broker.do_send(Publish(ChatMessage { body: "hello".into(), .. }));

// Unsubscribe (optional — dead subscribers are auto-pruned)
broker.do_send(BrokerUnsubscribe(sub_id));
```

---

## 5. Finite State Machine Actor

An actor that enforces typed state transitions. The state determines which
messages are valid, preventing illegal transitions at the type level where
possible and at runtime otherwise.

### Design

```rust
/// A state machine actor with explicit transitions.
pub trait StateMachine: Send + 'static {
    /// The state type (usually an enum).
    type State: Send + Clone + 'static;

    /// The input type (usually an enum of events/commands).
    type Input: Message<Result = TransitionResult<Self::State>> + Send + 'static;

    /// Compute the next state from current state + input.
    /// Returns `Ok(new_state)` or `Err(reason)` if the transition is invalid.
    fn transition(&self, state: &Self::State, input: &Self::Input)
        -> Result<Self::State, String>;

    /// Side effects to run after a successful transition.
    fn on_enter(&mut self, _old: &Self::State, _new: &Self::State, _ctx: &mut Context<FsmActor<Self>>) {}
}

pub struct FsmActor<M: StateMachine> {
    machine: M,
    state: M::State,
    subscribers: Vec<Recipient<Notify>>,
    dirty: bool,
}

pub enum TransitionResult<S> {
    Ok(S),
    Rejected(String),
}
```

### Use cases

- **Connection lifecycle**: `Disconnected -> Connecting -> Connected -> Disconnecting`
- **Voice call state**: `Idle -> Joining -> InCall -> Leaving`
- **File transfer**: `Pending -> Transferring(progress) -> Complete | Failed`

### Example

```rust
#[derive(Clone)]
enum ConnState { Disconnected, Connecting, Connected }

enum ConnInput { Connect, Connected, Disconnect, Error(String) }
impl Message for ConnInput { type Result = TransitionResult<ConnState>; }

struct ConnMachine;

impl StateMachine for ConnMachine {
    type State = ConnState;
    type Input = ConnInput;

    fn transition(&self, state: &ConnState, input: &ConnInput) -> Result<ConnState, String> {
        match (state, input) {
            (ConnState::Disconnected, ConnInput::Connect) => Ok(ConnState::Connecting),
            (ConnState::Connecting, ConnInput::Connected) => Ok(ConnState::Connected),
            (ConnState::Connected, ConnInput::Disconnect) => Ok(ConnState::Disconnected),
            (_, ConnInput::Error(_)) => Ok(ConnState::Disconnected),
            _ => Err(format!("invalid transition from {state:?}")),
        }
    }
}
```

FSM actors support the same `Subscribe`/`Notify`/`Select` interface as
state actors, so they can feed into derived actors and the reactive graph.

---

## 6. Pool

A utility struct (not itself an actor) that distributes work across a
fixed set of identical worker actors using round-robin routing.

### Design

```rust
/// A pool of identical actors that distributes messages round-robin.
/// Not an actor itself — held by the caller and used to forward messages.
pub struct Pool<A: Actor + Clone> {
    workers: Vec<Addr<A>>,
    next: usize,
}

impl<A: Actor + Clone> Pool<A> {
    pub fn new(system: &SystemHandle, actor: A, size: usize) -> Self { ... }
}
```

### Messages

The pool cannot implement `Handler<M>` generically for all `M` (Rust
doesn't support blanket handler impls on concrete types). Instead, it
provides forwarding methods:

```rust
impl<A: Actor + Clone> Pool<A> {
    /// Fire-and-forget: forward to the next worker (round-robin).
    pub fn send<M>(&mut self, msg: M) -> Result<(), SendError<M>>
    where A: Handler<M>, M: Message<Result = ()> {
        let worker = &self.workers[self.next % self.workers.len()];
        self.next += 1;
        worker.do_send(msg)
    }

    /// Request-reply: forward to the next worker and await the response.
    pub async fn ask<M>(&mut self, msg: M) -> Result<M::Result, AskError>
    where A: Handler<M>, M: Message {
        let worker = &self.workers[self.next % self.workers.len()];
        self.next += 1;
        worker.ask(msg).await
    }
}
```

### Use cases

- CPU-intensive message processing (encryption, hashing)
- Parallel I/O operations (file chunk processing)
- Rate-limited external API calls (one connection per worker)

---

## 7. Debounce / Throttle Actor

Rate-limits message forwarding to a target actor. Useful for UI inputs
(typing indicators, search-as-you-type) and network events.

### Design

Requires a new `Context::run_after(duration, msg)` primitive — a one-shot
delayed message send. Unlike `run_interval` (repeating), this fires once
after a delay and returns a cancellable handle. Debounce cancels the
previous timer on each new message and starts a fresh one.

```rust
/// One-shot delayed message. Cancel to prevent delivery.
pub struct TimerHandle { ... }
impl TimerHandle {
    pub fn cancel(self) { ... }
}

impl<A: Actor> Context<A> {
    /// Send a message to self after a delay. Returns a cancellable handle.
    pub fn run_after<M: Message<Result = ()>>(
        &self, delay: Duration, msg: M,
    ) -> TimerHandle
    where A: Handler<M> { ... }
}
```

```rust
/// Debounce: forwards only the last message after a quiet period.
pub struct Debounce<M: Message<Result = ()> + Send + 'static> {
    target: Recipient<M>,
    delay: Duration,
    pending: Option<M>,
    timer: Option<TimerHandle>,
}

/// Throttle: forwards at most one message per interval.
/// Uses `run_after` internally instead of `Instant` (not available on WASM).
pub struct Throttle<M: Message<Result = ()> + Send + 'static> {
    target: Recipient<M>,
    interval: Duration,
    pending: Option<M>,
    cooling_down: bool,
}
```

### Debounce behavior

1. Receive message -> store as `pending`, reset timer
2. Timer fires -> forward `pending` to target, clear
3. New message before timer -> replace `pending`, restart timer

### Throttle behavior

1. Receive message -> if not `cooling_down`, forward immediately, set
   `cooling_down = true`, start `run_after` timer for cooldown period
2. If `cooling_down`, store as `pending` (replacing any previous pending)
3. Timer fires -> set `cooling_down = false`, forward `pending` if present
   (and start a new cooldown if so)

### Example

```rust
let search_actor = system.spawn(SearchActor::new());
let debounced = system.spawn(Debounce::new(
    search_actor.into(),  // Recipient<SearchQuery>
    Duration::from_millis(300),
));

// Typing fast — only the last query within 300ms gets forwarded
debounced.do_send(SearchQuery("h".into()));
debounced.do_send(SearchQuery("he".into()));
debounced.do_send(SearchQuery("hel".into()));
debounced.do_send(SearchQuery("hello".into()));
// → only SearchQuery("hello") reaches SearchActor
```

---

## Summary of Actor Types

| Type | Purpose | Key trait |
|------|---------|-----------|
| `StateActor<S>` | Own state, get/set/subscribe | Produces `StateRef<S>` |
| `DerivedActor<Src,T>` | Reactive derived value from source(s) | Produces `StateRef<T>` |
| `StreamOutput<T>` | Produce an async stream from an actor | — (composable) |
| `Broker<T>` | Topic-based pub/sub, push values directly | — |
| `FsmActor<M>` | Typed state machine with transitions | Produces `StateRef<M::State>` |
| `Pool<A>` | Round-robin work distribution | Utility (not an actor) |
| `Debounce<M>` / `Throttle<M>` | Rate-limit message forwarding | — |

---

## Crate Organization

All new types go into the existing `willow-actor` crate under feature-gated
modules. The core actor primitives (`Actor`, `Handler`, `Addr`, etc.) remain
ungated — they're always available.

```
crates/actor/src/
├── lib.rs              — existing core re-exports
├── actor.rs            — Actor, Handler, StreamHandler, Message (existing)
├── addr.rs             — Addr, AnyAddr, Recipient (existing)
├── context.rs          — Context, IntervalHandle, TimerHandle (existing, extended)
├── envelope.rs         — BoxEnvelope (existing)
├── mailbox.rs          — message loop (existing)
├── runtime.rs          — platform abstraction (existing)
├── supervisor.rs       — RestartPolicy (existing)
├── system.rs           — System, SystemHandle (existing)
├── error.rs            — SendError, AskError (existing)
├── state.rs        NEW — StateActor<S>, Select, Mutate, Get, Set
├── derived.rs      NEW — DeriveSource trait, DerivedActor, derived()
├── stream.rs       NEW — StreamOutput<T>, OutputStream<T>
├── broker.rs       NEW — Broker<T>, Publish, BrokerSubscribe
├── fsm.rs          NEW — StateMachine trait, FsmActor<M>
├── pool.rs         NEW — Pool<A>
└── debounce.rs     NEW — Debounce<M>, Throttle<M>
```

New dependency: `futures` (for `join!` in multi-source snapshots). The
existing `futures-core` dep is insufficient.

The `runtime` module needs a new `bounded_channel<T>(capacity)` function
for `StreamOutput` backpressure (native: `tokio::sync::mpsc::channel`,
WASM: `futures::channel::mpsc::channel`).

---

## Implementation Plan

### Phase 1: Core extension + StateActor + DerivedActor

1. Make `Recipient<M>` cloneable (change `Box` to `Arc` internally)
2. Add `Context::run_after()` and `TimerHandle` to existing context module
3. Implement `StateActor<S>` in `crates/actor/src/state.rs`
4. Implement `DeriveSource` trait + tuple macro in `crates/actor/src/derived.rs`
5. Implement `DerivedActor<Src, T>` with `StateRef<S>` handles
6. Tests: subscribe, notify batching, caching, multi-source, chaining

### Phase 2: StreamOutput + Broker

1. Implement `StreamOutput<T>` and `OutputStream<T>`
2. Implement `Broker<T>` with `Publish`/`BrokerSubscribe`
3. Tests: stream consumption, backpressure, pub/sub fanout

### Phase 3: FSM, Pool, Debounce/Throttle

1. Implement `FsmActor<M>` with `StateMachine` trait
2. Implement `Pool<A>` with round-robin routing
3. Implement `Debounce<M>` and `Throttle<M>`
4. Tests for each

### Phase 4: Publish

1. Ensure zero willow-specific dependencies
2. Add `README.md`, examples, docs
3. Publish to crates.io

---

## Open Questions

1. **`DeriveSource` tuple arity limit** — Macro-generated impls up to 6
   sources should cover all realistic cases. If someone needs more, they
   can nest tuples or combine intermediate derived actors.

2. **`StateRef` overhead** — The type-erased `StateRef<S>` handle uses
   dynamic dispatch for subscribe/select. This is one vtable call per
   notification round — negligible in practice, but worth benchmarking
   if derived chains get deep.

3. **Pool routing strategy** — Start with round-robin. Add least-loaded
   routing later if needed (requires workers to report queue depth).

4. **StreamOutput buffer overflow policy** — Current proposal drops new
   values via `try_send` when buffer is full. Alternative: disconnect
   slow consumers entirely. Some use cases may want guaranteed delivery
   (unbounded), which is opt-in via `subscribe_unbounded()`.

5. **Naming for the published crate** — `willow-actor` is coupled to the
   Willow project name. Consider renaming to something generic (e.g.,
   `microactor`, `tinyact`) before publishing.
