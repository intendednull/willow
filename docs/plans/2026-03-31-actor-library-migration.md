# Actor Library Migration Plan

**Date**: 2026-03-31

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

Each domain owns its own `StateActor<S>`, independently mutated and subscribed to:

```
StateActor<EventState>       — willow_state::ServerState (messages, channels, roles, members, permissions)
StateActor<ServerRegistry>   — servers map, active_server, topic_maps, keys, unread counts
StateActor<ChatMeta>         — current_channel, peers, hlc, seen_message_ids
StateActor<ProfileState>     — global display name map (EndpointId → String)
StateActor<NetworkMeta>      — connected, typing_peers
StateActor<VoiceState>       — voice_participants, active_voice_channel, muted, deafened
```

All of these are `Send + Sync + Clone` — no rusqlite, no unsafe.

### Layer 2 — Derived Views (auto-recompute on source change)

Commonly-accessed view computations become `DerivedActor`s. Each subscribes to
its sources and only notifies downstream when the derived value actually changes
(via `PartialEq`):

```
MessagesView   ← (EventState, ServerRegistry, ChatMeta, ProfileState)
MembersView    ← (EventState, ChatMeta, ProfileState)
ChannelsView   ← (EventState, ServerRegistry)
UnreadView     ← (ServerRegistry,)
RolesView      ← (EventState,)
ConnectionView ← (NetworkMeta, ChatMeta)
```

### Layer 3 — Terminal ClientView

A single `DerivedActor` composing all Layer 2 views into one UI-facing snapshot:

```
ClientView ← (MessagesView, MembersView, ChannelsView, UnreadView, RolesView, ConnectionView)
```

The UI subscribes to `StateRef<ClientView>`. One subscription, one notification
when anything changes. Individual `StateRef<MessagesView>` etc. are also
available for fine-grained subscriptions.

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

/// Terminal composite — everything the UI needs in one snapshot.
#[derive(Clone, PartialEq)]
pub struct ClientView {
    pub messages: MessagesView,
    pub members: MembersView,
    pub channels: ChannelsView,
    pub unread: UnreadView,
    pub roles: RolesView,
    pub connection: ConnectionView,
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

   let client_view: StateRef<ClientView> = derived(
       &system,
       (messages_view.clone(), members_view.clone(), channels_view.clone(),
        unread_view.clone(), roles_view.clone(), connection_view.clone()),
       |(msgs, members, channels, unread, roles, conn)| {
           ClientView { messages: (**msgs).clone(), members: (**members).clone(), ... }
       },
   );
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

4. **Expose the terminal** `StateRef<ClientView>` on `ClientHandle`:
   ```rust
   /// Subscribe to all client state changes via a single reactive handle.
   pub fn view(&self) -> &StateRef<ClientView> {
       &self.client_view
   }
   ```

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

## Phase 4: Broker<ClientEvent> for Event Broadcasting

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

## Phase 5: StreamHandler for Topic Listeners

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

## Phase 6: Typing Indicator Throttle

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
Phase 4 (Broker) ─── independent, can run in parallel ───────────────┤
                                                                     │
                                                               Phase 5 (StreamHandler)
                                                                     │
Phase 6 (Throttle) ─── independent ─────────────────────────────────┘
```

- **Phases 1→2→3** are sequential: each builds on the prior decomposition
- **Phase 4** (Broker) is fully independent — do it anytime
- **Phase 5** (StreamHandler) depends on Phase 2 (needs decomposed addrs) and Phase 4 (uses broker)
- **Phase 6** (Throttle) is independent

---

## Migration Safety

Each phase produces a compilable, testable codebase:
- Phase 1: same behavior, persistence is async (slightly better perf)
- Phase 2: same behavior, state split into actors (same access patterns, just routed differently)
- Phase 3: same behavior, accessors read from cached derived state (faster reads)
- Phase 4: same behavior, event delivery via Broker instead of channel
- Phase 5: same behavior, listeners are proper actors
- Phase 6: same behavior, typing throttle is explicit

### End-to-end verification after all phases
```bash
just check          # fmt + clippy + test + WASM — zero warnings
just test-client    # all client tests
just test-app       # headless UI + integration
just test-browser   # Leptos browser tests
just test-relay     # relay tests (unchanged)
just test-state     # state tests (unchanged)
```
