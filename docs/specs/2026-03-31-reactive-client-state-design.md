# Reactive Client State — Domain Actor Decomposition

**Date**: 2026-03-31
**Status**: Draft

## Problem

The `willow-client` crate manages all runtime state through a single monolithic
`SharedState` struct wrapped in a custom `ClientStateActor`. This creates several
problems:

1. **`unsafe impl Send`**: `SharedState` contains `rusqlite::Connection` (via
   `PersistentEventStore` and `MessageDb`), which is `!Send`. The current code
   uses `unsafe impl Send for ClientStateActor` to work around this.

2. **Monolithic mutations**: Every mutation — whether it touches voice state,
   chat metadata, or event-sourced state — goes through a single actor's
   mailbox via type-erased closures (`mutate_state(&addr, |s| { ... })`).
   No concurrency between independent domains.

3. **Recomputation on every read**: Accessor methods like `messages()` and
   `server_members()` recompute their entire result on every call by running
   a closure over `SharedState`. Nothing is cached.

4. **Duplicated patterns**: `ClientStateActor` is a hand-rolled reimplementation
   of the library's `StateActor<S>` (dirty flag, subscriber list, idle batching,
   type-erased selectors/mutators). The Leptos web UI has its own
   `DerivedStateActor<T>` that reimplements `DerivedActor`.

5. **No reactive composition**: UI frameworks must poll or manually wire
   selectors to detect changes. There is no push-based notification when a
   specific slice of state changes.

### What exists today

```
SharedState (monolithic, !Send)
├── state: ClientState
│   ├── chat: ChatState           (current_channel, peers, hlc, seen_ids)
│   ├── servers: HashMap<String, ServerContext>
│   ├── active_server: Option<String>
│   ├── profiles: ProfileStore
│   ├── emoji: EmojiRegistry
│   ├── message_db: Option<Arc<Mutex<MessageDb>>>     ← !Send (rusqlite)
│   ├── event_state: willow_state::ServerState
│   └── event_store: PersistentEventStore              ← !Send (rusqlite)
├── identity: Identity
├── config: ClientConfig
├── connected: bool
├── typing_peers: HashMap
├── voice_participants: HashMap
├── active_voice_channel: Option<String>
├── voice_muted: bool
├── voice_deafened: bool
├── join_links: Vec<JoinLink>
└── last_typing_sent_ms: u64
```

All reads and writes go through `ClientStateActor` via `read_state`/`mutate_state`.

## Goal

Replace the monolithic `SharedState` + `ClientStateActor` with:

1. **Domain-specific `StateActor<S>`** for each independent state domain
2. **`PersistenceActor`** that auto-persists via `Notify` subscriptions
3. **`DerivedActor` views** that reactively cache computed state
4. **`Broker<ClientEvent>`** for event distribution
5. **`ClientViewHandle`** that exposes `StateRef<T>` at every granularity
6. **`ClientMutations`** — typed mutation interface routing to domain actors

No legacy code. No `SharedState`. No `ClientStateActor`. No `unsafe impl Send`.
User code interacts via `client.views()` (reads) and `client.mutations()` (writes).
The new architecture uses only library primitives from `willow-actor`.

## Constraints

- All code must compile for both native and `wasm32-unknown-unknown`
- All 420+ existing tests must continue to pass
- `willow-actor` library primitives only — no hand-rolled actor patterns
- `StateActor<S>` requires `S: Clone + Send + Sync + 'static`
- `DerivedActor` change detection requires `S: PartialEq`
- `DeriveSource` tuple max arity is 6

## Design

### 1. Layer 1 — Source StateActors

Each domain gets its own `StateActor<S>` holding pure data. All types are
`Clone + Send + Sync + PartialEq`. No I/O, no database handles, no unsafe.

```rust
// Event-sourced server state — the single source of truth for shared state.
// willow_state::ServerState already derives Clone + Serialize + Deserialize.
type EventState = willow_state::ServerState;

// Server registry — all servers and their metadata.
struct ServerRegistry {
    servers: HashMap<String, ServerEntry>,
    active_server: Option<String>,
}

struct ServerEntry {
    server: willow_channel::Server,   // has create_channel/delete_channel methods
    name: String,
    topic_map: HashMap<String, (String, ChannelId)>,
    keys: HashMap<String, ChannelKey>,
    unread: HashMap<String, usize>,
}

// Chat session metadata.
struct ChatMeta {
    current_channel: String,
    peers: Vec<EndpointId>,
    seen_message_ids: HashSet<String>,
}

// Global profile display names.
struct ProfileState {
    names: HashMap<EndpointId, String>,
}

// Network connection metadata.
struct NetworkMeta {
    connected: bool,
    typing_peers: HashMap<EndpointId, (String, u64)>,
    last_typing_sent_ms: u64,
}

// Voice call state.
struct VoiceState {
    participants: HashMap<String, HashSet<EndpointId>>,
    active_channel: Option<String>,
    muted: bool,
    deafened: bool,
}
```

Each is spawned as `system.spawn(StateActor::new(initial_value))`.

Mutation example (actions that currently use `mutate_state`):
```rust
// Before (monolithic):
mutate_state(&self.state_addr, |s| {
    s.state.chat.peers.push(peer_id);
    s.connected = true;
}).await;

// After (domain-specific):
willow_actor::state::mutate(&self.chat_meta_addr, |c| c.peers.push(peer_id)).await;
willow_actor::state::mutate(&self.network_meta_addr, |n| n.connected = true).await;
```

Read example:
```rust
// Before:
read_state(&self.state_addr, |s| s.state.chat.current_channel.clone()).await

// After:
willow_actor::state::select(&self.chat_meta_addr, |c| c.current_channel.clone()).await
```

Fields that don't belong to any domain actor live directly on `ClientHandle`:
- `identity: Identity` (already there)
- `persistence_enabled: bool` (from config, immutable after construction)
- `join_links: Arc<Mutex<Vec<JoinLink>>>` (rarely modified)
- `hlc: Arc<Mutex<HLC>>` (mutation-time clock, not reactive state)

### 2. PersistenceActor

Owns all `!Send` resources. Runs on a single-threaded mailbox — no
`unsafe impl Send` needed.

**Subscribes to state changes and auto-persists.** Callers never send
explicit persist messages — the actor watches relevant `StateRef`s via
`Notify` subscriptions and writes to disk whenever state changes.

```rust
struct PersistenceActor {
    event_store: PersistentEventStore,   // SqliteEventStore or LocalStorageEventStore
    server_id: Option<String>,
    persistence_enabled: bool,
    // StateRef handles for subscription
    event_state: StateRef<EventState>,
    server_registry: StateRef<ServerRegistry>,
    profiles: StateRef<ProfileState>,
}
```

On startup, the actor subscribes to `Notify` on each source `StateRef`.
When notified, it fetches the current snapshot via `state_ref.get()` and
persists it. This means:

- **Event state changes** → auto-saves `ServerState` + appends to event store
- **Server registry changes** → auto-saves server config, keys, server list
- **Profile changes** → auto-saves local profile

Debouncing via the actor's `idle()` hook batches rapid mutations into a
single persist operation.

**Read messages** (ask-based, used at startup/sync):
- `LoadAllEvents` → `Vec<Event>`
- `GetLatestHash` → `StateHash`
- `LoadEventsSince { hash }` → `Vec<Event>`
- `OpenEventStore { server_id }` — open/switch event store

No write messages needed — persistence is fully reactive.

### 3. Layer 2 — Derived Views

Commonly-accessed view computations become `DerivedActor`s. Each subscribes
to its source `StateRef`s and only recomputes when sources change.
`PartialEq` prevents spurious downstream notifications.

```rust
MessagesView   ← (EventState, ServerRegistry, ChatMeta, ProfileState)
MembersView    ← (EventState, ChatMeta, ProfileState)
ChannelsView   ← (EventState, ServerRegistry)
UnreadView     ← (ServerRegistry,)
RolesView      ← (EventState,)
ConnectionView ← (NetworkMeta, ChatMeta)
```

Each returns a `StateRef<T>` that can be subscribed to or used as a source
for further derivation.

Example:
```rust
let messages_view: StateRef<MessagesView> = derived(
    &system,
    (event_ref.clone(), registry_ref.clone(), chat_ref.clone(), profile_ref.clone()),
    |(events, registry, chat, profiles)| {
        compute_messages_view(&events, &registry, &chat, &profiles)
    },
);
```

The `compute_messages_view` function is the same algorithm currently in
`accessors.rs::messages()`, extracted into a pure function.

Adding a new view: define a type, call `derived()`, register in the
appropriate view group.

### 4. Layer 3 — Terminal ClientView

Layer 2 views are grouped into intermediate derived actors (Bevy-style
nesting) to stay under the 6-source `DeriveSource` limit:

```rust
ChatViews   ← (MessagesView, ChannelsView, UnreadView)       // 3 sources
SocialViews ← (MembersView, RolesView, ConnectionView)       // 3 sources
ClientView  ← (ChatViews, SocialViews, VoiceState)           // 3 sources
```

Room for 3 more groups as features grow.

```rust
struct ClientView {
    chat: ChatViews,
    social: SocialViews,
    voice: VoiceState,
    server_name: Option<String>,
    server_owner: Option<EndpointId>,
    current_channel: String,
}
```

### 5. ClientViewHandle

Exposes `StateRef<T>` at every granularity so consumers pick their
subscription level:

```rust
struct ClientViewHandle {
    // Terminal — subscribe once, get everything
    view: StateRef<ClientView>,

    // Layer 2 — subscribe to specific views
    messages: StateRef<MessagesView>,
    members: StateRef<MembersView>,
    channels: StateRef<ChannelsView>,
    unread: StateRef<UnreadView>,
    roles: StateRef<RolesView>,
    connection: StateRef<ConnectionView>,

    // Layer 1 — subscribe to raw source state
    event_state: StateRef<EventState>,
    server_registry: StateRef<ServerRegistry>,
    chat_meta: StateRef<ChatMeta>,
    profiles: StateRef<ProfileState>,
    network: StateRef<NetworkMeta>,
    voice: StateRef<VoiceState>,
}
```

Accessed via `client.views()`. UI code subscribes at whatever level
makes sense: `client.views().messages` for just messages, or
`client.views().view` for everything.

### 6. Broker<ClientEvent>

Replaces `futures::channel::mpsc::UnboundedSender<ClientEvent>` with the
library's `Broker<T>`. Listeners publish via `do_send(Publish(event))`,
consumers subscribe via `BrokerSubscribe`. Dead subscribers are
auto-pruned.

An `EventReceiver` bridge provides `recv()`/`try_recv()` for consumers
that need a channel-like API.

### 7. Leptos Integration

The current `DerivedStateActor<T>` in `crates/web/src/derived.rs` is
deleted entirely. Replaced by a generic bridge:

```rust
fn use_state_ref<T: Clone + Send + Sync + 'static>(
    state_ref: &StateRef<T>,
    system: &SystemHandle,
) -> ReadSignal<T>
```

This subscribes to a `StateRef<T>`, receives `Notify` on change, fetches
the snapshot, and updates a Leptos signal. ~30 lines replacing the 72-line
custom actor.

The 260-line `wire_derived_signals()` function becomes:
```rust
fn wire_signals(views: &ClientViewHandle, system: &SystemHandle) {
    let messages = use_state_ref(&views.messages, system);
    let channels = use_state_ref(&views.channels, system);
    let members = use_state_ref(&views.members, system);
    // ... etc
}
```

No selectors, no closures over monolithic state. Each signal is a direct
subscription to a pre-computed view.

## Architecture Diagram

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  EventState  │  │ServerRegistry│  │   ChatMeta   │  │ ProfileState │
│  StateActor  │  │  StateActor  │  │  StateActor  │  │  StateActor  │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                 │                 │                 │
  ┌────┴─────────────────┼─────────────────┼─────────────────┘
  │                      │                 │
  ▼                      ▼                 ▼
┌─────────────┐  ┌───────────┐  ┌──────────────┐
│MessagesView │  │ChannelsV. │  │ MembersView  │
│ DerivedActor│  │DerivedAct.│  │ DerivedActor │
└──────┬──────┘  └─────┬─────┘  └──────┬───────┘
       │               │               │
       ▼               ▼               ▼
  ┌──────────┐    ┌──────────┐
  │ChatViews │    │SocialV.  │
  │ Derived  │    │ Derived  │
  └────┬─────┘    └────┬─────┘
       │               │
       └───────┬───────┘
               ▼
        ┌─────────────┐
        │ ClientView  │  ← ClientViewHandle.view
        │ DerivedActor│
        └─────────────┘

┌──────────────────┐  ┌────────────────────┐  ┌──────────────┐  ┌──────────────┐
│ PersistenceActor │  │ Broker<ClientEvent>│  │ NetworkMeta  │  │  VoiceState  │
│ (owns rusqlite)  │  │ (event fan-out)    │  │  StateActor  │  │  StateActor  │
│ subscribes to:   │  └────────────────────┘  └──────────────┘  └──────────────┘
│  EventState      │
│  ServerRegistry  │              ┌───────────────────┐
│  ProfileState    │              │  ClientMutations   │ ← user code calls this
└──────────────────┘              │  (typed interface)  │
                                  └───────────────────┘
```

## Mutation Handle

User code should not interact with domain actors directly. Instead, a
`ClientMutations` handle provides a typed interface that routes each
operation to the correct domain actors, builds events, and broadcasts.
Callers never see `StateActor`, `mutate()`, or actor addresses.

```rust
/// Typed mutation interface. Routes operations to domain actors.
///
/// Cloneable — hand to UI handlers, listener tasks, etc.
pub struct ClientMutations {
    event_state: Addr<StateActor<EventState>>,
    server_registry: Addr<StateActor<ServerRegistry>>,
    chat_meta: Addr<StateActor<ChatMeta>>,
    profiles: Addr<StateActor<ProfileState>>,
    network: Addr<StateActor<NetworkMeta>>,
    voice: Addr<StateActor<VoiceState>>,
    persistence: Addr<PersistenceActor>,
    event_broker: Addr<Broker<ClientEvent>>,
    identity: Identity,
    hlc: Arc<Mutex<HLC>>,
    join_links: Arc<Mutex<Vec<JoinLink>>>,
    topics: Arc<RwLock<HashMap<String, TopicHandle>>>,
    persistence_enabled: bool,
}
```

### Chat mutations

```rust
impl ClientMutations {
    /// Send a text message to a channel.
    pub async fn send_message(&self, channel: &str, body: &str) -> Result<()> {
        let event = self.build_event(EventKind::Message {
            channel_id: self.resolve_channel_id(channel).await?,
            body: body.to_string(),
            reply_to: None,
        }).await;
        self.apply_event(&event).await;
        self.broadcast_event(&event);
        Ok(())
    }

    /// Edit a message.
    pub async fn edit_message(&self, message_id: &str, new_body: &str) -> Result<()> { ... }

    /// Delete a message.
    pub async fn delete_message(&self, message_id: &str) -> Result<()> { ... }

    /// React to a message.
    pub async fn react(&self, message_id: &str, emoji: &str) -> Result<()> { ... }

    /// Pin/unpin a message.
    pub async fn pin_message(&self, channel: &str, message_id: &str) -> Result<()> { ... }
    pub async fn unpin_message(&self, channel: &str, message_id: &str) -> Result<()> { ... }

    /// Switch current channel.
    pub async fn switch_channel(&self, channel: &str) {
        let ch = channel.to_string();
        state::mutate(&self.chat_meta, move |c| c.current_channel = ch).await;
    }
}
```

### Server mutations

```rust
impl ClientMutations {
    /// Create a new channel.
    pub async fn create_channel(&self, name: &str) -> Result<()> {
        let name = name.to_string();

        // Mutate registry (creates channel in Server object + topic map)
        let event = state::mutate(&self.server_registry, |reg| {
            let entry = reg.active_mut().ok_or(anyhow!("no active server"))?;
            let ch_id = entry.server.create_channel(&name, ChannelKind::Text)?;
            let topic = make_topic(&entry.server, &name);
            entry.topic_map.insert(topic, (name.clone(), ch_id.clone()));
            // ... return built event
        }).await?;

        // Apply to event-sourced state (persistence auto-triggers)
        self.apply_event(&event).await;

        // Update current channel
        state::mutate(&self.chat_meta, |c| c.current_channel = name).await;

        self.broadcast_event(&event);
        Ok(())
    }

    /// Delete a channel.
    pub async fn delete_channel(&self, name: &str) -> Result<()> { ... }

    /// Create/delete roles.
    pub async fn create_role(&self, name: &str) -> Result<()> { ... }
    pub async fn delete_role(&self, role_id: &str) -> Result<()> { ... }

    /// Trust/untrust/kick members.
    pub async fn trust_peer(&self, peer_id: EndpointId) { ... }
    pub async fn untrust_peer(&self, peer_id: EndpointId) { ... }
    pub async fn kick_member(&self, peer_id: EndpointId) -> Result<()> { ... }

    /// Create/switch/leave servers.
    pub async fn create_server(&self, name: &str) -> Result<String> { ... }
    pub async fn switch_server(&self, server_id: &str) { ... }
    pub async fn leave_server(&self, server_id: &str) { ... }
}
```

### Voice mutations

```rust
impl ClientMutations {
    pub async fn join_voice(&self, channel_id: &str) { ... }
    pub async fn leave_voice(&self) { ... }
    pub async fn toggle_mute(&self) -> bool {
        state::mutate(&self.voice, |v| { v.muted = !v.muted; v.muted }).await
    }
    pub async fn toggle_deafen(&self) -> bool { ... }
}
```

### Network mutations (called by listeners)

```rust
impl ClientMutations {
    /// Apply an incoming event from a peer.
    pub async fn apply_event(&self, event: &Event) {
        state::mutate(&self.event_state, |es| {
            willow_state::apply_lenient(es, event);
        }).await;
        // PersistenceActor auto-persists via subscription — no manual call.
    }

    /// Track a peer as online.
    pub async fn peer_connected(&self, peer_id: EndpointId) {
        state::mutate(&self.chat_meta, move |c| {
            if !c.peers.contains(&peer_id) {
                c.peers.push(peer_id);
            }
        }).await;
        self.event_broker.do_send(Publish(ClientEvent::PeerConnected(peer_id)));
    }

    /// Track a peer as offline.
    pub async fn peer_disconnected(&self, peer_id: EndpointId) { ... }

    /// Update a peer's profile.
    pub async fn update_profile(&self, peer_id: EndpointId, name: String) { ... }

    /// Record a typing indicator.
    pub async fn record_typing(&self, peer_id: EndpointId, channel: String) { ... }
}
```

### Internal helpers

```rust
impl ClientMutations {
    /// Build an event with the next HLC timestamp and current state hash.
    async fn build_event(&self, kind: EventKind) -> Event {
        let parent_hash = state::select(&self.event_state, |es| es.hash()).await;
        let ts = self.hlc.lock().unwrap().now().as_u64();
        Event {
            id: Uuid::new_v4().to_string(),
            parent_hash,
            author: self.identity.endpoint_id(),
            timestamp_ms: ts,
            kind,
        }
    }

    /// Resolve channel name → channel ID via event state.
    async fn resolve_channel_id(&self, channel: &str) -> Result<String> { ... }

    /// Broadcast a signed event to peers.
    fn broadcast_event(&self, event: &Event) { ... }
}
```

### ClientHandle composition

`ClientHandle` composes views (read) and mutations (write) into one
user-facing type:

```rust
pub struct ClientHandle<N: Network> {
    /// Read state at any granularity.
    views: ClientViewHandle,
    /// Typed mutation interface.
    mutations: ClientMutations,
    /// The network backend.
    network: Option<Arc<N>>,
    /// Local identity.
    identity: Identity,
}

impl<N: Network> ClientHandle<N> {
    /// Access reactive state views.
    pub fn views(&self) -> &ClientViewHandle { &self.views }

    /// Access mutation interface.
    pub fn mutations(&self) -> &ClientMutations { &self.mutations }

    /// Convenience: delegate common operations to mutations.
    pub async fn send_message(&self, channel: &str, body: &str) -> Result<()> {
        self.mutations.send_message(channel, body).await
    }
    // ... more delegations for backward compat
}
```

User code becomes:
```rust
// Read:
let msgs = client.views().messages.get().await;
let channels = client.views().channels.get().await;

// Write:
client.mutations().send_message("general", "hello").await?;
client.mutations().create_channel("dev").await?;
client.mutations().toggle_mute().await;

// Subscribe:
let msgs_ref = &client.views().messages;
state::subscribe(msgs_ref, my_notification_recipient);
```

## Scope

### In scope
- Delete `SharedState`, `ClientState`, `ClientStateActor`, `client_actor.rs`
- Delete `ServerContext` (replaced by `ServerEntry` in `ServerRegistry`)
- Create `ClientMutations` handle routing all operations to domain actors
- Create `ClientViewHandle` exposing `StateRef<T>` at every granularity
- Rewrite all `read_state`/`mutate_state` calls across:
  accessors.rs, actions.rs, connect.rs, joining.rs, listeners.rs,
  servers.rs, voice.rs
- Create `PersistenceActor` that auto-persists via `Notify` subscriptions
- Create domain state types in `state_actors.rs`
- Create derived view types and compute functions in `views.rs`
- Replace `event_tx` channel with `Broker<ClientEvent>`
- Delete `crates/web/src/derived.rs`, create `use_state_ref` bridge
- Simplify `wire_derived_signals()`
- Update `test_client()` helper

### Out of scope
- Migrating `crates/app/` (Bevy app) network bridge — architecturally different
- `StreamHandler` for topic listeners — requires `TopicEvents` to impl `Stream`
- `Throttle<SendTyping>` for typing indicator — manual impl is simpler
- Worker crate changes — already uses actor library well
- Changing the `willow-state` event-sourced model
- Changing the network protocol or wire format
