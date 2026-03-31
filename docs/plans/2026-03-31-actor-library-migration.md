# Actor Library Migration Plan

**Date**: 2026-03-31

## Implementation Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: PersistenceActor | **DONE** | `persistence_actor.rs` — 312 lines, 12 message types |
| Phase 2a: Domain state types | **DONE** | `state_actors.rs` — 6 pure-data types, `SourceState` bundle |
| Phase 2a: Wire actors into ClientHandle | **DONE** | Domain actors spawned alongside legacy actor |
| Phase 2b: Auto-sync + incremental migration | **DONE** | idle() hook syncs legacy→domain; accessors.rs + voice.rs migrated |
| Phase 2c: Delete legacy code | FUTURE | Remove ClientStateActor once all writes are migrated |
| Phase 3: Derived views | FUTURE | DerivedActor chain + ClientViewHandle (depends on Phase 2c) |
| Phase 4: Leptos migration | FUTURE | Replace custom DerivedStateActor with library StateRef (depends on Phase 3) |
| Phase 5: Broker<ClientEvent> | **DONE** | EventReceiver bridge, web crate updated |
| Phase 6: StreamHandler | DEFERRED | TopicEvents trait doesn't impl Stream; needs network crate change |
| Phase 7: Typing Throttle | DEFERRED | Manual impl is 6 lines; Throttle actor adds complexity |

### Key Lessons Learned

1. **Reads and writes must migrate atomically**: Moving accessors to domain
   actors while writes still go to the legacy actor breaks consistency.
   The state decomposition (Phase 2b) must be a single atomic change.

2. **StreamHandler requires `futures::Stream`**: The `TopicEvents` trait uses
   `async fn next()` instead of `Stream`. Migrating to StreamHandler requires
   either adding a Stream impl to `TopicEvents` or creating an adapter. Deferred
   until the network trait evolves.

3. **Throttle vs manual**: The `Throttle<M>` actor is valuable for new features
   but overkill for replacing a 6-line timestamp check. The typing throttle
   works fine as-is.

4. **Auto-sync via idle() beats big-bang**: Instead of rewriting all 60+ call
   sites at once, the `ClientStateActor.idle()` hook automatically syncs
   domain actors after every mutation batch. This enables safe incremental
   migration: files can be migrated one at a time. Reads moved first
   (accessors.rs, voice.rs), writes follow gradually.

## Overview

The `willow-actor` crate now provides `StateActor<S>`, `DerivedActor`, `Broker<T>`,
`StreamHandler`, `Throttle`, and more. This plan migrates `willow-client` to use
these primitives, replacing the monolithic `SharedState` + `ClientStateActor` with
a reactive composition of domain-specific state actors and derived views.

The key architectural change: **decompose the single `SharedState` blob into
independent `StateActor`s per domain, extract persistence into its own actor
(eliminating `unsafe impl Send`), and expose computed view state via
`DerivedActor` chains that the UI subscribes to.**

---

## Architecture: Reactive State Composition

### Layer 1 — Source StateActors (pure data, Send + Sync + Clone)

Each domain owns its own `StateActor<S>`, independently mutated and subscribed to.
New domains are added by defining a type, spawning a `StateActor`, and registering
its `StateRef` in the `SourceState` bundle (see below).

```
StateActor<EventState>       — willow_state::ServerState (messages, channels, roles, members, permissions)
StateActor<ServerRegistry>   — servers map, active_server, topic_maps, keys, unread counts
StateActor<ChatMeta>         — current_channel, peers, hlc, seen_message_ids
StateActor<ProfileState>     — global display name map (EndpointId → String)
StateActor<NetworkMeta>      — connected, typing_peers
StateActor<VoiceState>       — voice_participants, active_voice_channel, muted, deafened
```

All of these are `Send + Sync + Clone` — no rusqlite, no unsafe.

**Extensibility**: To add a new domain (e.g. `NotificationState`), define the
type with `Clone + Send + Sync + PartialEq`, add a `StateRef` field to
`SourceState`, spawn the actor in `ClientHandle::new()`, and wire any derived
views that consume it. No changes needed to the actor library.

### Layer 2 — Derived Views (auto-recompute on source change)

Commonly-accessed view computations become `DerivedActor`s. Each subscribes to
its sources and only notifies downstream when the derived value actually changes
(via `PartialEq`). New views are added by defining a type and calling `derived()`.

```
MessagesView   ← (EventState, ServerRegistry, ChatMeta, ProfileState)
MembersView    ← (EventState, ChatMeta, ProfileState)
ChannelsView   ← (EventState, ServerRegistry)
UnreadView     ← (ServerRegistry,)
RolesView      ← (EventState,)
ConnectionView ← (NetworkMeta, ChatMeta)
```

**Extensibility**: Adding a new view (e.g. `SearchResultsView`) requires only
a new type + a `derived()` call wired to the relevant sources. Register its
`StateRef` in the appropriate view group (see Layer 3).

### Layer 3 — Terminal ClientView (grouped sources, Bevy-style nesting)

`DeriveSource` supports tuples up to arity 6. To stay under the limit while
remaining extensible, Layer 2 views are grouped into intermediate derived
actors (like Bevy's nested `Query` tuples), then composed into the terminal:

```
ChatViews   ← (MessagesView, ChannelsView, UnreadView)     — 3 sources
SocialViews ← (MembersView, RolesView, ConnectionView)     — 3 sources
ClientView  ← (ChatViews, SocialViews, VoiceState)         — 3 sources, room for 3 more groups
```

Each group is itself a `DerivedActor` returning a struct, so adding a new Layer 2
view only requires adding it to the appropriate group (or creating a new group if
all are full). The terminal never needs more than ~6 group sources.

### ClientViewHandle — StateRef access for user code

The user-facing handle exposes both the terminal snapshot and individual
`StateRef`s, so consumers can pick the granularity they need:

```rust
/// All reactive state, accessible at any granularity.
pub struct ClientViewHandle {
    // Terminal — subscribe once, get everything
    pub view: StateRef<ClientView>,

    // Layer 2 — subscribe to specific views
    pub messages: StateRef<MessagesView>,
    pub members: StateRef<MembersView>,
    pub channels: StateRef<ChannelsView>,
    pub unread: StateRef<UnreadView>,
    pub roles: StateRef<RolesView>,
    pub connection: StateRef<ConnectionView>,

    // Layer 1 — subscribe to raw source state
    pub event_state: StateRef<EventState>,
    pub server_registry: StateRef<ServerRegistry>,
    pub chat_meta: StateRef<ChatMeta>,
    pub profiles: StateRef<ProfileState>,
    pub network: StateRef<NetworkMeta>,
    pub voice: StateRef<VoiceState>,
}
```

Users access fine-grained subscriptions through `client.views().messages` etc.
The terminal `client.views().view` provides a single subscription for everything.

### Persistence Actor (owns all rusqlite, runs on dedicated thread)

```
PersistenceActor  — event_store (SqliteEventStore), message_db
```

This actor owns the `!Send` resources. It receives fire-and-forget messages:
`PersistEvent`, `SaveServerState`, `SaveServerConfig`, `SaveProfile`, etc.
No `unsafe impl Send` needed anywhere — the actor system guarantees
single-threaded execution for this actor.

### Architecture Diagram

```
┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  EventState  │  │ServerRegistry│  │   ChatMeta   │  │ ProfileState │
│  StateActor  │  │  StateActor  │  │  StateActor  │  │  StateActor  │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       │                 │                 │                 │
       │    ┌────────────┼─────────────────┼─────────────────┘
       │    │            │                 │
       ▼    ▼            ▼                 ▼
  ┌─────────────┐  ┌───────────┐  ┌──────────────┐
  │MessagesView │  │ChannelsV. │  │ MembersView  │    ... more derived
  │ DerivedActor│  │DerivedAct.│  │ DerivedActor │
  └──────┬──────┘  └─────┬─────┘  └──────┬───────┘
         │               │               │
         └───────────────┼───────────────┘
                         ▼
                  ┌─────────────┐
                  │ ClientView  │  ← UI subscribes here
                  │ DerivedActor│
                  └─────────────┘

  ┌──────────────────┐
  │ PersistenceActor │  ← fire-and-forget, owns rusqlite
  └──────────────────┘

  ┌────────────────────┐
  │ Broker<ClientEvent>│  ← event fan-out for listeners/UI
  └────────────────────┘
```

---

## New Types

### Source State Types (all Clone + Send + Sync + PartialEq)

```rust
/// Event-sourced server state. Wraps willow_state::ServerState
/// which is already pure data (Serialize + Deserialize, no I/O).
pub type EventState = willow_state::ServerState;

/// Registry of all servers and their metadata.
#[derive(Clone, PartialEq)]
pub struct ServerRegistry {
    pub servers: HashMap<String, ServerEntry>,
    pub active_server: Option<String>,
}

#[derive(Clone, PartialEq)]
pub struct ServerEntry {
    pub name: String,
    pub topic_map: HashMap<String, (String, willow_channel::ChannelId)>,
    pub keys: HashMap<String, ChannelKey>,
    pub unread: HashMap<String, usize>,
}

/// Chat session metadata.
#[derive(Clone, PartialEq)]
pub struct ChatMeta {
    pub current_channel: String,
    pub peers: Vec<EndpointId>,
    pub seen_message_ids: HashSet<String>,
    // HLC is internal to mutation logic, not needed in reactive state
}

/// Global profile display names.
#[derive(Clone, PartialEq)]
pub struct ProfileState {
    pub names: HashMap<EndpointId, String>,
}

/// Network connection metadata.
#[derive(Clone, PartialEq)]
pub struct NetworkMeta {
    pub connected: bool,
    pub typing_peers: HashMap<EndpointId, (String, u64)>,
}

/// Voice call state.
#[derive(Clone, PartialEq)]
pub struct VoiceState {
    pub participants: HashMap<String, HashSet<EndpointId>>,
    pub active_channel: Option<String>,
    pub muted: bool,
    pub deafened: bool,
}
```

### Derived View Types (all PartialEq for change detection)

```rust
/// Precomputed message list for the current channel.
#[derive(Clone, PartialEq)]
pub struct MessagesView {
    pub messages: Vec<DisplayMessage>,
    pub channel: String,
}

/// Channel list for the active server.
#[derive(Clone, PartialEq)]
pub struct ChannelsView {
    pub channels: Vec<ChannelInfo>,
}

#[derive(Clone, PartialEq)]
pub struct ChannelInfo {
    pub name: String,
    pub kind: String, // "text" or "voice"
}

/// Member list with online status.
#[derive(Clone, PartialEq)]
pub struct MembersView {
    pub members: Vec<MemberInfo>,
}

#[derive(Clone, PartialEq)]
pub struct MemberInfo {
    pub peer_id: EndpointId,
    pub display_name: String,
    pub is_online: bool,
}

/// Unread badge counts per channel.
#[derive(Clone, PartialEq)]
pub struct UnreadView {
    pub counts: HashMap<String, usize>,
}

/// Role definitions with permissions.
#[derive(Clone, PartialEq)]
pub struct RolesView {
    pub roles: Vec<(String, String, Vec<String>)>, // (id, name, perms)
}

/// Connection and presence info.
#[derive(Clone, PartialEq)]
pub struct ConnectionView {
    pub connected: bool,
    pub peer_count: usize,
    pub typing_peers: Vec<(EndpointId, String)>, // (id, channel)
}

/// Grouped views — Bevy-style nesting keeps terminal under 6-source limit.
#[derive(Clone, PartialEq)]
pub struct ChatViews {
    pub messages: MessagesView,
    pub channels: ChannelsView,
    pub unread: UnreadView,
}

#[derive(Clone, PartialEq)]
pub struct SocialViews {
    pub members: MembersView,
    pub roles: RolesView,
    pub connection: ConnectionView,
}

/// Terminal composite — everything the UI needs in one snapshot.
/// Composed from grouped views so the terminal DerivedActor only
/// needs 3 sources (room for 3 more groups as features grow).
#[derive(Clone, PartialEq)]
pub struct ClientView {
    pub chat: ChatViews,
    pub social: SocialViews,
    pub voice: VoiceState,
    pub server_name: Option<String>,
    pub server_owner: Option<EndpointId>,
    pub current_channel: String,
}
```

### Persistence Messages

```rust
pub struct PersistEvent(pub willow_state::Event);
pub struct PersistServerState { pub server_id: String, pub state: willow_state::ServerState }
pub struct PersistServerConfig { pub server_id: String, pub server: Server, pub keys: HashMap<String, ChannelKey> }
pub struct PersistProfile { pub display_name: String }
pub struct PersistJoinLinks(pub Vec<ops::JoinLink>);
pub struct PersistServerList(pub Vec<String>);
pub struct LoadEvents { pub server_id: String }  // ask() → Vec<Event>
pub struct LoadServerState { pub server_id: String }  // ask() → Option<ServerState>
```

---

## Phase 1: PersistenceActor — Extract rusqlite

**Goal**: Move all `!Send` resources (rusqlite connections) into a dedicated actor.
Eliminates `unsafe impl Send for ClientStateActor`.

### Files to create
- `crates/client/src/persistence_actor.rs`

### Files to modify
- `crates/client/src/lib.rs` — add `PersistenceActor` addr to `ClientHandle`
- `crates/client/src/state.rs` — remove `event_store` and `message_db` from `ClientState`
- `crates/client/src/client_actor.rs` — remove `unsafe impl Send`

### What changes

1. **Create `PersistenceActor`** owning `PersistentEventStore` and `MessageDb`:
   ```rust
   pub struct PersistenceActor {
       event_store: PersistentEventStore,
       message_db: Option<MessageDb>,
       server_id: Option<String>,
   }
   // No unsafe needed — actor system guarantees single-threaded mailbox
   ```

2. **Fire-and-forget persistence**: All current `storage::save_*` calls inside
   `mutate_state` closures become `persistence_addr.do_send(PersistServerState { .. })`.
   Mutations no longer block on I/O.

3. **Ask-based loading**: `LoadEvents` and `LoadServerState` use `ask()` for
   startup/sync scenarios that need the data.

4. **Remove `event_store` and `message_db`** from `ClientState`. The event store
   is now only accessed through the `PersistenceActor`.

5. **WASM**: On WASM, `PersistenceActor` holds the `LocalStorageEventStore`
   (which is already `Send`). Same API, different backend.

### Verification
- `just test-client` — all 93 tests pass
- `just check-wasm` — no `unsafe impl Send` anywhere
- Grep for `unsafe impl Send` — should be zero

---

## Phase 2: Decompose SharedState into Domain StateActors

**Goal**: Replace the monolithic `SharedState` with independent `StateActor<S>` per domain.

### Files to create
- `crates/client/src/state_actors.rs` — defines all source state types and spawning logic

### Files to modify
- `crates/client/src/lib.rs` — `ClientHandle` holds `StateRef<T>` handles instead of one `Addr<ClientStateActor>`
- `crates/client/src/client_actor.rs` — **delete entirely**
- `crates/client/src/state.rs` — remove `SharedState`, `ClientState`; keep `DisplayMessage`, `ServerContext`, `ProfileStore`

### What changes

1. **Define source state types** in `state_actors.rs` (see types above).

2. **Spawn actors** in `ClientHandle::new()`:
   ```rust
   let event_state = system.spawn(StateActor::new(initial_event_state));
   let server_registry = system.spawn(StateActor::new(ServerRegistry::default()));
   let chat_meta = system.spawn(StateActor::new(ChatMeta::default()));
   let profile_state = system.spawn(StateActor::new(ProfileState::default()));
   let network_meta = system.spawn(StateActor::new(NetworkMeta::default()));
   let voice_state = system.spawn(StateActor::new(VoiceState::default()));
   ```

3. **Update `ClientHandle`** to hold `StateRef<T>` handles:
   ```rust
   pub struct ClientHandle<N: Network> {
       pub event_state: StateRef<EventState>,
       pub server_registry: StateRef<ServerRegistry>,
       pub chat_meta: StateRef<ChatMeta>,
       pub profiles: StateRef<ProfileState>,
       pub network: StateRef<NetworkMeta>,
       pub voice: StateRef<VoiceState>,
       pub persistence: Addr<PersistenceActor>,
       pub event_broker: Addr<Broker<ClientEvent>>,
       pub system: SystemHandle,
       pub identity: Identity,
       pub network_backend: Option<Arc<N>>,
       pub topics: Arc<RwLock<HashMap<String, N::Topic>>>,
       // HLC stays here — it's a mutation-time concern, not reactive state
       pub hlc: Arc<Mutex<HLC>>,
   }
   ```

4. **Migrate all `mutate_state` calls** to target the specific `StateActor`:
   ```rust
   // Before:
   mutate_state(&self.state_addr, |s| { s.state.chat.peers.push(id); }).await;

   // After:
   willow_actor::state::mutate(&self.chat_meta_addr, |chat| { chat.peers.push(id); }).await;
   ```

5. **Migrate all `read_state` calls** to `willow_actor::state::select`:
   ```rust
   // Before:
   read_state(&self.state_addr, |s| s.connected).await

   // After:
   willow_actor::state::select(&self.network_addr, |n| n.connected).await
   ```

6. **Delete `client_actor.rs`** — no longer needed. All its functionality is
   provided by the library's `StateActor`, `Select`, `Mutate`, and `Subscribe`.

### Key migration mappings

| Old call | New call |
|---|---|
| `mutate_state(&addr, \|s\| { s.state.chat.X = .. })` | `state::mutate(&chat_meta_addr, \|c\| { c.X = .. })` |
| `mutate_state(&addr, \|s\| { s.state.event_state = .. })` | `state::mutate(&event_state_addr, \|e\| { .. })` |
| `mutate_state(&addr, \|s\| { s.connected = .. })` | `state::mutate(&network_addr, \|n\| { n.connected = .. })` |
| `mutate_state(&addr, \|s\| { s.voice_participants.. })` | `state::mutate(&voice_addr, \|v\| { v.participants.. })` |
| `read_state(&addr, \|s\| s.state.event_state.owner)` | `state::select(&event_state_addr, \|e\| e.owner)` |
| `storage::save_server_state(..)` inside mutate | `persistence.do_send(PersistServerState { .. })` |

### Verification
- `just test-client` — all tests pass
- No `SharedState` or `ClientStateActor` references remain
- `just check-wasm` — compiles clean

---

## Phase 3: Derived View Actors

**Goal**: Replace expensive accessor computations with reactive `DerivedActor`s
that cache results and only recompute when sources change.

### Files to create
- `crates/client/src/views.rs` — derived view types and spawning

### Files to modify
- `crates/client/src/accessors.rs` — rewrite to read from derived views
- `crates/client/src/lib.rs` — spawn derived actors, expose `StateRef<ClientView>`

### What changes

1. **Spawn derived views** in `ClientHandle::new()`:
   ```rust
   let event_ref = StateRef::from(&event_state);
   let registry_ref = StateRef::from(&server_registry);
   let chat_ref = StateRef::from(&chat_meta);
   let profile_ref = StateRef::from(&profile_state);
   let network_ref = StateRef::from(&network_meta);

   let messages_view: StateRef<MessagesView> = derived(
       &system,
       (event_ref.clone(), registry_ref.clone(), chat_ref.clone(), profile_ref.clone()),
       |(events, registry, chat, profiles)| {
           compute_messages_view(events, registry, chat, profiles)
       },
   );

   let members_view: StateRef<MembersView> = derived(
       &system,
       (event_ref.clone(), chat_ref.clone(), profile_ref.clone()),
       |(events, chat, profiles)| {
           compute_members_view(events, chat, profiles)
       },
   );

   let channels_view: StateRef<ChannelsView> = derived(
       &system,
       (event_ref.clone(), registry_ref.clone()),
       |(events, registry)| compute_channels_view(events, registry),
   );

   // ... more derived views ...

   // Group Layer 2 views into intermediate derived actors (Bevy-style nesting)
   let chat_views: StateRef<ChatViews> = derived(
       &system,
       (messages_view.clone(), channels_view.clone(), unread_view.clone()),
       |(msgs, channels, unread)| ChatViews {
           messages: (**msgs).clone(), channels: (**channels).clone(), unread: (**unread).clone(),
       },
   );
   let social_views: StateRef<SocialViews> = derived(
       &system,
       (members_view.clone(), roles_view.clone(), connection_view.clone()),
       |(members, roles, conn)| SocialViews {
           members: (**members).clone(), roles: (**roles).clone(), connection: (**conn).clone(),
       },
   );

   // Terminal: only 3 sources, room for 3 more groups as features grow
   let voice_ref = StateRef::from(&voice_state);
   let client_view: StateRef<ClientView> = derived(
       &system,
       (chat_views.clone(), social_views.clone(), voice_ref.clone()),
       |(chat, social, voice)| ClientView {
           chat: (**chat).clone(), social: (**social).clone(), voice: (**voice).clone(), ...
       },
   );

   // Bundle all StateRefs for user-code access at any granularity
   let view_handle = ClientViewHandle {
       view: client_view,
       messages: messages_view, members: members_view, channels: channels_view,
       unread: unread_view, roles: roles_view, connection: connection_view,
       event_state: event_ref, server_registry: registry_ref, chat_meta: chat_ref,
       profiles: profile_ref, network: network_ref, voice: voice_ref,
   };
   ```

2. **Move computation logic** from `accessors.rs` closures into pure functions:
   ```rust
   fn compute_messages_view(
       events: &Arc<EventState>,
       registry: &Arc<ServerRegistry>,
       chat: &Arc<ChatMeta>,
       profiles: &Arc<ProfileState>,
   ) -> MessagesView { ... }
   ```
   These are the same algorithms currently in the `read_state` closures, just
   extracted into named functions operating on the decomposed state.

3. **Simplify accessors** to cheap reads from derived state:
   ```rust
   pub async fn messages(&self, _channel: &str) -> Vec<DisplayMessage> {
       let view = self.messages_view.get().await;
       view.messages.clone()
   }

   pub async fn server_members(&self) -> Vec<(EndpointId, String, bool)> {
       let view = self.members_view.get().await;
       view.members.iter().map(|m| (m.peer_id, m.display_name.clone(), m.is_online)).collect()
   }
   ```

4. **Expose `ClientViewHandle`** on `ClientHandle`:
   ```rust
   /// All reactive state at any granularity — terminal, views, or raw sources.
   pub fn views(&self) -> &ClientViewHandle {
       &self.view_handle
   }
   ```
   Users pick their subscription level:
   - `client.views().view` — single terminal, everything
   - `client.views().messages` — just messages view
   - `client.views().event_state` — raw event-sourced state

### Performance characteristics
- `messages()` currently recomputes on every call (O(n) messages × O(m) profile lookups)
- After migration: recomputes only when `EventState`, `ChatMeta`, or `ProfileState` changes
- `PartialEq` on `MessagesView` prevents spurious downstream notifications
- `Arc<T>` snapshots make reads zero-copy

### Verification
- `just test-client` — all tests pass
- `just test-browser` — Leptos UI still renders correctly
- Benchmark: `messages()` accessor latency before/after

---

## Phase 4: Leptos Derived State Migration

**Goal**: Replace the custom `DerivedStateActor<T>` in `crates/web/src/derived.rs`
with the library's `StateRef<T>` subscription, and replace the 20+ manual
`derived_signal()` + `Effect` wiring calls in `crates/web/src/state.rs` with
direct subscriptions to the `ClientViewHandle` from Phase 3.

### Current state (what gets replaced)

The Leptos web UI currently has:
- **`crates/web/src/derived.rs`**: Custom `DerivedStateActor<T>` (72 lines) that
  subscribes to `ClientStateActor`, runs a selector on `&SharedState`, compares
  with cached value, and updates a Leptos `WriteSignal<T>`. Has its own
  `unsafe impl Send`. This is a bespoke reimplementation of exactly what
  `DerivedActor` + `StateRef` provides.
- **`crates/web/src/state.rs`** `wire_derived_signals()` (260 lines): 20+
  `derived_signal()` calls, each spawning a separate `DerivedStateActor` with
  a selector closure over monolithic `SharedState`, wired to a `WriteSignal`
  via a Leptos `Effect`.
- **`crates/web/src/state.rs`** `create_signals()` (170 lines): Creates 40+
  `signal()` pairs for read/write halves. Many of these duplicate view state
  that will now live in `ClientViewHandle`.

### What changes

1. **Delete `crates/web/src/derived.rs`** entirely — the custom
   `DerivedStateActor<T>` and `derived_signal()` are replaced by the library's
   `StateRef<T>` + `Notify` subscription.

2. **Create a thin Leptos bridge** (`crates/web/src/state_bridge.rs`):
   ```rust
   /// Bridge a StateRef<T> into a Leptos ReadSignal<T>.
   /// Subscribes to Notify, fetches snapshot, updates signal on change.
   pub fn use_state_ref<T: Clone + Send + Sync + 'static>(
       state_ref: &StateRef<T>,
       system: &SystemHandle,
   ) -> ReadSignal<T> { ... }
   ```
   This is a generic, ~30-line function replacing the entire 72-line custom actor.

3. **Simplify `wire_derived_signals()`** — instead of 20+ selector closures
   operating on monolithic `SharedState`, wire directly from `ClientViewHandle`:
   ```rust
   pub fn wire_signals(views: &ClientViewHandle, system: &SystemHandle) {
       // Each signal is a direct subscription to an already-computed view
       let messages = use_state_ref(&views.messages, system);
       let channels = use_state_ref(&views.channels, system);
       let members = use_state_ref(&views.members, system);
       let unread = use_state_ref(&views.unread, system);
       // ... etc
   }
   ```
   The expensive computation (message formatting, member resolution, etc.)
   already happened in the `DerivedActor`s from Phase 3. The Leptos layer
   just subscribes to pre-computed snapshots.

4. **Reduce signal count**: Signals that were derived from `SharedState` (messages,
   channels, peers, roles, unread, connection_status, etc.) are now just
   `ReadSignal<T>` views of the corresponding `StateRef<T>`. Only purely-local
   UI state (show_settings, show_sidebar, editing, replying_to, etc.) remains
   as Leptos-managed signals.

5. **Simplify `event_processing.rs`**: Side-effect events (VoiceJoined/Left,
   VoiceSignal, JoinLinkResponse) remain. But `PeerConnected → set_loading(false)`
   and `Listening → set_connection_status` are now handled reactively via
   `ConnectionView`, eliminating those event handlers.

6. **Remove typing indicator polling loop** (`app.rs` lines 259-286): The
   2-second `gloo_timers` polling loop that calls `handle.typing_in()` is
   replaced by subscribing to `StateRef<NetworkMeta>` which includes
   `typing_peers` — updates arrive via `Notify` push instead of polling.

### Files to modify
- `crates/web/src/derived.rs` — **delete entirely**
- `crates/web/src/state.rs` — simplify `wire_derived_signals()`, reduce `create_signals()`
- `crates/web/src/app.rs` — use `ClientViewHandle` instead of raw state_addr; remove typing poll loop
- `crates/web/src/event_processing.rs` — remove reactively-handled event arms

### Files to create
- `crates/web/src/state_bridge.rs` — generic `use_state_ref()` bridge function

### Verification
- `just test-browser` — all 39 Leptos browser tests pass
- `just check-wasm` — compiles clean
- Grep for `DerivedStateActor` — should be zero (only library's `DerivedActor` remains)
- Manual: verify messages, channels, members, unread badges update reactively

---

## Phase 5: Broker<ClientEvent> for Event Broadcasting

**Priority**: HIGH — self-contained, no dependencies on other phases. Can be done
in parallel with Phases 1-3.

### Files to modify
- `crates/client/src/events.rs` — add `impl Message for ClientEvent`
- `crates/client/src/lib.rs` — replace `event_tx` with `Addr<Broker<ClientEvent>>`
- `crates/client/src/listeners.rs` — `event_tx.unbounded_send(e)` → `broker.do_send(Publish(e))`
- `crates/client/src/connect.rs` — pass broker addr to listeners
- `crates/client/src/actions.rs` — replace event_tx usage
- `crates/client/src/joining.rs` — replace event_tx usage
- `crates/client/src/voice.rs` — replace event_tx usage

### What changes

1. **Make ClientEvent a Message**:
   ```rust
   impl willow_actor::Message for ClientEvent { type Result = (); }
   ```

2. **Replace channel with broker**:
   - Remove `event_tx: futures_mpsc::UnboundedSender<ClientEvent>` from `ClientHandle`
   - Add `event_broker: Addr<Broker<ClientEvent>>`
   - In `new()`: `let event_broker = system.spawn(Broker::<ClientEvent>::new());`

3. **Mechanical replacement** (~14 call sites):
   - `event_tx.unbounded_send(event)` → `event_broker.do_send(Publish(event))`

4. **Test helper**: Create `EventCollector` actor subscribing to the broker,
   queryable via `ask()` to drain collected events for assertions.

### Verification
- `just test-client` — all tests pass
- `just check-wasm` — compiles
- Zero remaining `event_tx` references

---

## Phase 6: StreamHandler for Topic Listeners

**Priority**: MEDIUM — depends on Phases 2+4 (uses decomposed state addrs + broker).

### Files to modify
- `crates/client/src/listeners.rs` — rewrite as `TopicListenerActor`
- `crates/worker/src/actors/network.rs` — replace spawn workaround with `StreamHandler`

### What changes

1. **Client TopicListenerActor**:
   ```rust
   pub struct TopicListenerActor<T: TopicHandle, E: TopicEvents> {
       topic: T,
       event_state: Addr<StateActor<EventState>>,
       chat_meta: Addr<StateActor<ChatMeta>>,
       profiles: Addr<StateActor<ProfileState>>,
       persistence: Addr<PersistenceActor>,
       event_broker: Addr<Broker<ClientEvent>>,
       events: Option<E>,
   }

   impl<T, E> Actor for TopicListenerActor<T, E> { ... }

   // Attach stream in started():
   fn started(&mut self, ctx: &mut Context<Self>) {
       if let Some(events) = self.events.take() {
           ctx.add_stream(events);
       }
   }

   impl<T, E> StreamHandler<Result<GossipEvent, ...>> for TopicListenerActor<T, E> { ... }
   ```

2. **Worker NetworkActor** — same pattern, replace `started()` spawn with
   `ctx.add_stream()` + `StreamHandler` impl. Remove `GossipEventMsg` wrapper.

### Verification
- `just test-client` + `just test-crate willow-worker`

---

## Phase 7: Typing Indicator Throttle

**Priority**: MEDIUM — independent of other phases.

### Files to modify
- `crates/client/src/joining.rs` — simplify `send_typing()`
- `crates/client/src/lib.rs` — add throttle addr to `ClientHandle`

### What changes

1. **Define `SendTyping` message** + `TypingBroadcastActor`
2. **Wrap with `Throttle`** (3-second interval)
3. **Replace manual timestamp logic** in `send_typing()`:
   `self.typing_throttle.do_send(Enqueue(SendTyping(channel)))`
4. **Remove `last_typing_sent_ms`** — no longer needed

### Verification
- `just test-client` — typing tests pass

---

## Phase Ordering & Dependencies

```
Phase 1 (PersistenceActor) ──→ Phase 2 (Decompose State) ──→ Phase 3 (Derived Views)
                                                                     │
                                                               Phase 4 (Leptos Migration)
                                                                     │
Phase 5 (Broker) ─── independent, can run in parallel ───────────────┤
                                                                     │
                                                               Phase 6 (StreamHandler)
                                                                     │
Phase 7 (Throttle) ─── independent ─────────────────────────────────┘
```

- **Phases 1→2→3→4** are sequential: each builds on the prior
- **Phase 5** (Broker) is fully independent — do it anytime
- **Phase 6** (StreamHandler) depends on Phase 2 (decomposed addrs) and Phase 5 (broker)
- **Phase 7** (Throttle) is independent

---

## Migration Safety

Each phase produces a compilable, testable codebase:
- Phase 1: same behavior, persistence is async (slightly better perf)
- Phase 2: same behavior, state split into actors (same access patterns, just routed differently)
- Phase 3: same behavior, accessors read from cached derived state (faster reads)
- Phase 4: same behavior, Leptos signals driven by library StateRef instead of custom actors
- Phase 5: same behavior, event delivery via Broker instead of channel
- Phase 6: same behavior, listeners are proper actors
- Phase 7: same behavior, typing throttle is explicit

### End-to-end verification after all phases
```bash
just check          # fmt + clippy + test + WASM — zero warnings
just test-client    # all client tests
just test-app       # headless UI + integration
just test-browser   # Leptos browser tests
just test-relay     # relay tests (unchanged)
just test-state     # state tests (unchanged)
```
