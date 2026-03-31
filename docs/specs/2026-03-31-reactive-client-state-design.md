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
2. **`PersistenceActor`** that owns all `!Send` database resources
3. **`DerivedActor` views** that reactively cache computed state
4. **`Broker<ClientEvent>`** for event distribution
5. A **`ClientViewHandle`** that exposes `StateRef<T>` at every granularity

No legacy code. No `SharedState`. No `ClientStateActor`. No `unsafe impl Send`.
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

```rust
struct PersistenceActor {
    event_store: PersistentEventStore,   // SqliteEventStore or LocalStorageEventStore
    server_id: Option<String>,
    persistence_enabled: bool,
}
```

**Write messages** (fire-and-forget via `do_send`):
- `PersistEvent { event, new_hash }` — append event + update hash
- `PersistServerState { server_id, state }` — save full state snapshot
- `PersistServerConfig { server, keys }` — save server config
- `PersistServerList { ids }` — save server ID list
- `PersistProfile { display_name }` — save local profile
- `PersistJoinLinks { server_id, links }` — save join links
- `OpenEventStore { server_id }` — open/switch event store

**Read messages** (ask-based, used at startup/sync):
- `LoadAllEvents` → `Vec<Event>`
- `GetLatestHash` → `StateHash`
- `LoadEventsSince { hash }` → `Vec<Event>`

Mutations that currently call `storage::save_*()` inline become
`persistence_addr.do_send(PersistServerState { .. })`. The mutation
no longer blocks on I/O.

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
└──────────────────┘  └────────────────────┘  └──────────────┘  └──────────────┘
```

## Mutation Patterns

### Simple single-domain mutation
```rust
// Toggle voice mute
pub async fn toggle_mute(&self) -> bool {
    willow_actor::state::mutate(&self.voice_state_addr, |v| {
        v.muted = !v.muted;
        v.muted
    }).await
}
```

### Cross-domain mutation
```rust
// Apply an event: touches EventState + PersistenceActor
pub async fn apply_and_persist(&self, event: &Event) {
    // 1. Apply to event-sourced state
    willow_actor::state::mutate(&self.event_state_addr, |es| {
        willow_state::apply_lenient(es, event);
    }).await;

    // 2. Persist (fire-and-forget, non-blocking)
    let hash = willow_actor::state::select(&self.event_state_addr, |es| es.hash()).await;
    self.persistence_addr.do_send(PersistEvent {
        event: event.clone(),
        new_hash: hash,
    });
}
```

### Channel creation (touches ServerRegistry + EventState)
```rust
pub async fn create_channel(&self, name: &str) -> Result<()> {
    let name = name.to_string();
    let peer_id = self.identity.endpoint_id();

    // 1. Mutate server registry (creates channel in Server object)
    let (event, topic) = willow_actor::state::mutate(&self.server_registry_addr, |reg| {
        let entry = reg.active_mut().ok_or(anyhow!("no active server"))?;
        let ch_id = entry.server.create_channel(&name, ChannelKind::Text)?;
        let topic = make_topic(&entry.server, &name);
        entry.topic_map.insert(topic.clone(), (name.clone(), ch_id));
        // ... build event
        Ok((event, topic))
    }).await?;

    // 2. Apply event to event-sourced state
    self.apply_and_persist(&event).await;

    // 3. Update current channel
    willow_actor::state::mutate(&self.chat_meta_addr, |c| {
        c.current_channel = name;
    }).await;

    // 4. Persist server config
    let (server, keys) = willow_actor::state::select(&self.server_registry_addr, |reg| {
        let e = reg.active().unwrap();
        (e.server.clone(), e.keys.clone())
    }).await;
    self.persistence_addr.do_send(PersistServerConfig { server, keys });

    // 5. Broadcast
    self.broadcast_event(&event);
    Ok(())
}
```

## Scope

### In scope
- Delete `SharedState`, `ClientState`, `ClientStateActor`, `client_actor.rs`
- Delete `ServerContext` (replaced by `ServerEntry` in `ServerRegistry`)
- Rewrite all `read_state`/`mutate_state` calls across:
  accessors.rs, actions.rs, connect.rs, joining.rs, listeners.rs,
  servers.rs, voice.rs
- Create `PersistenceActor` with all persistence messages
- Create domain state types in `state_actors.rs`
- Create derived view types and `ClientViewHandle`
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
