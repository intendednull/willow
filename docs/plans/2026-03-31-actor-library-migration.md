# Actor Library Migration Plan

**Date**: 2026-03-31
**Spec**: `docs/specs/2026-03-31-reactive-client-state-design.md`

## Overview

Implements the Reactive Client State spec: replace monolithic
`SharedState` + `ClientStateActor` with domain `StateActor`s,
reactive `DerivedActor` views, auto-persisting `PersistenceActor`,
`Broker<ClientEvent>`, `ClientMutations` handle, and `ClientViewHandle`.

No legacy code preserved. Full paradigm shift.

---

## Phase 1: Foundation — Types, PersistenceActor, Broker

**Goal**: Create all new types and actors without changing existing code.
Everything compiles, existing tests still pass via legacy path.

### 1a. Domain state types (`state_actors.rs`)

Create `Clone + Send + Sync + PartialEq` types:
- `EventState` (type alias for `willow_state::ServerState`)
- `ServerRegistry` + `ServerEntry` (includes `willow_channel::Server`)
- `ChatMeta`, `ProfileState`, `NetworkMeta`, `VoiceState`

### 1b. Derived view types (`views.rs`)

Create `Clone + PartialEq` view types:
- `MessagesView`, `MembersView`, `ChannelsView`
- `UnreadView`, `RolesView`, `ConnectionView`
- `ChatViews`, `SocialViews` (groups)
- `ClientView` (terminal)
- `ClientViewHandle` (bundle of all `StateRef<T>`)

### 1c. PersistenceActor (`persistence_actor.rs`)

Actor owns all `!Send` resources. Subscribes to `Notify` on `EventState`,
`ServerRegistry`, `ProfileState` — auto-persists on change. No write
messages from callers. Debounces via `idle()`.

Read messages for startup: `LoadAllEvents`, `GetLatestHash`,
`LoadEventsSince`, `OpenEventStore`.

### 1d. Broker<ClientEvent>

- `impl Message for ClientEvent`
- `EventReceiver` bridge for stream-like consumption
- Spawn `Broker::new()` in system

### Verification
- All new types compile
- Existing tests pass unchanged (nothing wired yet)

---

## Phase 2: ClientMutations — Typed Mutation Interface

**Goal**: Create the `ClientMutations` handle that routes all operations
to domain actors. This is the write-side API.

### Files to create
- `crates/client/src/mutations.rs`

### What it provides

```rust
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
    // ... topics for broadcasting
}
```

Methods grouped by domain (from spec):
- **Chat**: `send_message`, `edit_message`, `delete_message`, `react`,
  `pin_message`, `unpin_message`, `switch_channel`
- **Server**: `create_channel`, `delete_channel`, `create_role`,
  `delete_role`, `trust_peer`, `untrust_peer`, `kick_member`,
  `create_server`, `switch_server`, `leave_server`
- **Voice**: `join_voice`, `leave_voice`, `toggle_mute`, `toggle_deafen`
- **Network** (for listeners): `apply_event`, `peer_connected`,
  `peer_disconnected`, `update_profile`, `record_typing`
- **Helpers**: `build_event`, `resolve_channel_id`, `broadcast_event`

Each method routes to the correct domain `StateActor` via
`state::mutate()` / `state::select()`. PersistenceActor auto-persists
— no manual persist calls.

### Verification
- `ClientMutations` compiles
- Unit tests for `build_event`, `resolve_channel_id`

---

## Phase 3: Wire Everything — Replace ClientHandle

**Goal**: Rewrite `ClientHandle` to compose `ClientViewHandle` (reads) +
`ClientMutations` (writes). Spawn all actors. Delete all legacy code.

### What changes

1. **Rewrite `ClientHandle`**:
   ```rust
   pub struct ClientHandle<N: Network> {
       views: ClientViewHandle,
       mutations: ClientMutations,
       network: Option<Arc<N>>,
       identity: Identity,
       system: SystemHandle,
   }
   ```

2. **Spawn actor tree** in `ClientHandle::new()`:
   - 6 source `StateActor`s
   - 6 derived view `DerivedActor`s
   - 2 group `DerivedActor`s (ChatViews, SocialViews)
   - 1 terminal `DerivedActor` (ClientView)
   - `PersistenceActor` (subscribes to sources)
   - `Broker<ClientEvent>`
   - Bundle into `ClientViewHandle` + `ClientMutations`

3. **Rewrite all callers**:
   - `accessors.rs` → reads from `self.views()` or domain `StateRef`s
   - `actions.rs` → delegates to `self.mutations()`
   - `voice.rs` → delegates to `self.mutations()`
   - `servers.rs` → delegates to `self.mutations()`
   - `joining.rs` → delegates to `self.mutations()`
   - `connect.rs` → uses `mutations` for peer tracking, profile broadcast
   - `listeners.rs` → receives `ClientMutations` handle, calls
     `mutations.apply_event()`, `mutations.peer_connected()`, etc.

4. **Delete legacy code**:
   - Delete `client_actor.rs` entirely
   - Delete `SharedState` from `lib.rs`
   - Delete `ClientState`, `ServerContext` from `state.rs`
     (keep `DisplayMessage`, `PersistentEventStore`, `ProfileStore`)
   - Remove `unsafe impl Send`

5. **Update `test_client()`**: Spawns the full actor tree, returns
   `(ClientHandle, Addr<Broker<ClientEvent>>)`

### Verification
- `cargo test -p willow-client` — all tests pass
- `cargo clippy -p willow-client` — zero warnings
- Grep for `SharedState`, `ClientStateActor`, `read_state`, `mutate_state`
  — zero matches
- Grep for `unsafe impl Send` — zero matches

---

## Phase 4: Leptos Integration

**Goal**: Replace `DerivedStateActor<T>` with library `StateRef` subscriptions.

### What changes

1. **Delete `crates/web/src/derived.rs`**

2. **Create `crates/web/src/state_bridge.rs`**:
   ```rust
   fn use_state_ref<T>(state_ref: &StateRef<T>, system: &SystemHandle) -> ReadSignal<T>
   ```

3. **Simplify `wire_derived_signals()`**: Direct subscriptions to
   `ClientViewHandle` views instead of selector closures.

4. **Remove typing indicator polling loop**: Subscribe to
   `StateRef<NetworkMeta>` instead.

5. **Update `app.rs`**: Use `client.views()` and `client.mutations()`.

### Verification
- `cargo check -p willow-web` — compiles
- `just test-browser` — all browser tests pass

---

## Phase Ordering

```
Phase 1 (Foundation: types, persistence, broker)
    ↓
Phase 2 (ClientMutations handle)
    ↓
Phase 3 (Wire everything, delete legacy)
    ↓
Phase 4 (Leptos integration)
```

All phases are sequential. Each produces a compilable codebase.

---

## Files Changed (complete list)

### Created
- `crates/client/src/state_actors.rs` — domain state types
- `crates/client/src/views.rs` — derived view types, `ClientViewHandle`
- `crates/client/src/mutations.rs` — `ClientMutations` handle
- `crates/client/src/persistence_actor.rs` — auto-persisting actor
- `crates/client/src/event_receiver.rs` — `Broker` → channel bridge
- `crates/web/src/state_bridge.rs` — `StateRef` → Leptos signal bridge

### Deleted
- `crates/client/src/client_actor.rs` — replaced by library `StateActor`

### Rewritten
- `crates/client/src/lib.rs` — new `ClientHandle`, actor tree spawning
- `crates/client/src/accessors.rs` — reads from `ClientViewHandle`
- `crates/client/src/actions.rs` — delegates to `ClientMutations`
- `crates/client/src/voice.rs` — delegates to `ClientMutations`
- `crates/client/src/servers.rs` — delegates to `ClientMutations`
- `crates/client/src/joining.rs` — delegates to `ClientMutations`
- `crates/client/src/connect.rs` — uses `ClientMutations` for network setup
- `crates/client/src/listeners.rs` — receives `ClientMutations`, calls typed methods
- `crates/client/src/state.rs` — remove `SharedState`, `ClientState`, `ServerContext`
- `crates/client/src/events.rs` — `impl Message for ClientEvent`
- `crates/web/src/derived.rs` — deleted
- `crates/web/src/state.rs` — simplified `wire_derived_signals()`
- `crates/web/src/app.rs` — uses `views()` + `mutations()`

### Verification (end-to-end)
```bash
just check          # fmt + clippy + test + WASM — zero warnings
just test-client    # all client tests
just test-browser   # Leptos browser tests
just test-state     # state tests (unchanged)
just test-relay     # relay tests (unchanged)
```
