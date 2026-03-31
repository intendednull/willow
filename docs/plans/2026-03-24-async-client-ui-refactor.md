# Async Client + UI Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate polling from the Willow web UI by replacing `std::sync::mpsc` with async channels, splitting the monolithic `Client` into `ClientHandle` + `ClientEventLoop`, and restructuring the Leptos UI with context-based state management.

**Architecture:** The `willow-client` crate's `Client` struct splits into `SharedState` (wrapped in `Rc<RefCell<>>`), `ClientHandle` (cloneable command interface), and `ClientEventLoop` (async event processor). The `willow-web` crate replaces its 50ms poll loop with `spawn_local` tasks that await events, and replaces 30 prop-drilled signals with a context-provided `AppState` struct.

**Tech Stack:** Rust, Leptos 0.7 (CSR), futures 0.3, wasm-bindgen-futures, gloo-timers, futures::channel::mpsc

**Spec:** `docs/specs/2026-03-24-async-client-ui-refactor-design.md`

---

## File Map

### Client crate (`crates/client/src/`)

| File | Action | Responsibility |
|------|--------|----------------|
| `lib.rs` (3630 lines) | **Major rewrite** | Remove `Client` struct. Add `SharedState`, `ClientHandle`, `ClientEventLoop`, constructor `new()`, and `test_handle()` helper. Move 48 pub methods from `Client` to `ClientHandle`. Move `poll()` logic to `ClientEventLoop::run()`. |
| `events.rs` (108 lines) | **Modify** | Remove `ClientNotification` enum. |
| `network.rs` (592 lines) | **Modify** | Change `spawn_network()` signatures from `std::sync::mpsc` to `futures::channel::mpsc`. Remove 16ms tick timer in WASM `run_network_wasm()`. Cfg-gate native `run_network()` and `spawn_network()` out (native is out of scope and allowed to break). |
| `state.rs` (270 lines) | **No change** | `ClientState`, `ChatState`, `ServerContext` stay as-is. |

### Client crate config

| File | Action | Responsibility |
|------|--------|----------------|
| `Cargo.toml` | **Modify** | Add `futures` as a dependency (needed for `futures::channel::mpsc`). |

### Web crate (`crates/web/src/`)

| File | Action | Responsibility |
|------|--------|----------------|
| `app.rs` (802 lines) | **Major rewrite** | Slim to ~50 lines: create handle + event loop, build `AppState`/`AppWriteSignals`, `provide_context`, spawn async tasks, render layout shell. |
| `state.rs` | **Create** | `AppState`, `AppWriteSignals`, all sub-structs (`ChatState`, `NetworkState`, `ServerState`, `UiState`, `VoiceState`, `ChannelViewState`, and write counterparts). Constructor `fn create_signals() -> (AppState, AppWriteSignals)`. |
| `event_processing.rs` | **Create** | `process_event_batch()`, `refresh_all_signals()`, `extract_roles()`. |
| `handlers.rs` | **Create** | `make_send_handler()`, `make_edit_handler()`, `make_delete_handler()`, `make_react_handler()`, `make_channel_click_handler()`, `make_server_click_handler()`, `make_pin_handler()`. |
| `voice.rs` (320 lines) | **Minor modify** | Add `handle_voice_event()` helper called from `process_event_batch`. |
| `components/sidebar.rs` (308 lines) | **Modify** | Replace `ClientHandle` prop with `use_context`. |
| `components/member_list.rs` (107 lines) | **Modify** | Replace `ClientHandle` prop with `use_context`. |
| `components/welcome.rs` (56 lines) | **Modify** | Replace `ClientHandle` prop with `use_context`. |
| `components/add_server.rs` (168 lines) | **Modify** | Replace `ClientHandle` prop with `use_context`. |
| `components/server_settings.rs` (131 lines) | **Modify** | Replace `ClientHandle` prop with `use_context`. |
| `components/settings.rs` (92 lines) | **Modify** | Replace `ClientHandle` prop with `use_context`. |
| `components/file_share.rs` (154 lines) | **Modify** | Replace `ClientHandle` prop with `use_context`. |
| `components/roles.rs` | **Modify** | Replace `ClientHandle` prop with `use_context`. |
| `components/chat.rs` | **Modify** | Replace signal props with `use_context`. |
| `components/input.rs` | **Modify** | Minor: callbacks stay as props, no ClientHandle. |
| `components/message.rs` | **Modify** | Minor: callbacks stay as props, no ClientHandle. |
| `components/mod.rs` (29 lines) | **Modify** | Add exports for new modules. |

### Web crate config

| File | Action | Responsibility |
|------|--------|----------------|
| `Cargo.toml` | **Modify** | Add `futures` dep. Move `gloo-timers` from dev-deps to deps. |

---

## Task Ordering

The plan is structured so each task produces a compiling, testable state:

1. **Tasks 1-3:** Client crate refactor (async channels, SharedState + ClientHandle, EventLoop)
2. **Task 4:** Fix client tests
3. **Task 5:** Web crate dependencies + state module
4. **Task 6:** Event processing + handlers modules
5. **Task 7:** Rewrite App component
6. **Task 8:** Migrate components to context
7. **Task 9:** Verify everything compiles and tests pass

---

### Task 1: Replace `std::sync::mpsc` with `futures::channel::mpsc` in client crate

**Files:**
- Modify: `crates/client/Cargo.toml`
- Modify: `crates/client/src/lib.rs` (channel creation in `new()` and `test_client()`)
- Modify: `crates/client/src/network.rs` (function signatures, remove WASM tick timer)

This task changes the channel types throughout the client crate while keeping the `Client` struct intact. The `poll()` method switches from `try_recv()` to a loop that drains via the futures `try_next()` method.

- [ ] **Step 1: Add `futures` dependency to client Cargo.toml**

In `crates/client/Cargo.toml`, add `futures` to the `[dependencies]` section:

```toml
futures = "0.3"
```

- [ ] **Step 2: Update `DeferredPair` type and channel creation in `lib.rs`**

In `crates/client/src/lib.rs`:

Replace the `std::sync::mpsc as std_mpsc` import and `DeferredPair` type:

```rust
// Before:
use std::sync::mpsc as std_mpsc;

type DeferredPair = (
    std_mpsc::Sender<network::NetworkEvent>,
    std_mpsc::Receiver<network::NetworkCommand>,
);

// After:
use futures::channel::mpsc as futures_mpsc;

type DeferredPair = (
    futures_mpsc::UnboundedSender<network::NetworkEvent>,
    futures_mpsc::UnboundedReceiver<network::NetworkCommand>,
);
```

Update `Client` struct fields:

```rust
// Before:
pub(crate) cmd_tx: std_mpsc::Sender<network::NetworkCommand>,
pub(crate) event_rx: std_mpsc::Receiver<network::NetworkEvent>,

// After:
pub(crate) cmd_tx: futures_mpsc::UnboundedSender<network::NetworkCommand>,
pub(crate) event_rx: futures_mpsc::UnboundedReceiver<network::NetworkEvent>,
```

Update `Client::new()` channel creation:

```rust
// Before:
let (cmd_tx, cmd_rx) = std_mpsc::channel();
let (event_tx, event_rx) = std_mpsc::channel();

// After:
let (cmd_tx, cmd_rx) = futures_mpsc::unbounded();
let (event_tx, event_rx) = futures_mpsc::unbounded();
```

Update `Client::connect()` to pass the new types to `spawn_network`.

Update `cmd_tx.send(...)` calls throughout `lib.rs` to `cmd_tx.unbounded_send(...)`. There are many call sites — use find-and-replace: `self.cmd_tx.send(` → `self.cmd_tx.unbounded_send(`.

Update `poll()` to use `futures::stream::StreamExt::try_next`:

```rust
// Before:
while let Ok(net_event) = self.event_rx.try_recv() {

// After:
use futures::StreamExt;
while let Ok(Some(net_event)) = self.event_rx.try_next() {
```

Remove `notification_tx` field and all `self.notify(...)` calls. Remove the `notify()` method. Remove `with_notifications()` method.

- [ ] **Step 3: Update `spawn_network()` signatures in `network.rs`**

In `crates/client/src/network.rs`:

Update both `spawn_network` function signatures (native and WASM):

```rust
// Before:
pub fn spawn_network(
    identity: willow_identity::Identity,
    event_tx: std::sync::mpsc::Sender<NetworkEvent>,
    cmd_rx: std::sync::mpsc::Receiver<NetworkCommand>,
    config: willow_network::NetworkConfig,
)

// After:
pub fn spawn_network(
    identity: willow_identity::Identity,
    event_tx: futures::channel::mpsc::UnboundedSender<NetworkEvent>,
    cmd_rx: futures::channel::mpsc::UnboundedReceiver<NetworkCommand>,
    config: willow_network::NetworkConfig,
)
```

Update both `run_network` / `run_network_wasm` signatures to match.

Update `event_tx.send(...)` to `event_tx.unbounded_send(...)` throughout `network.rs`.

- [ ] **Step 4: Remove 16ms tick timer in WASM network loop**

In `crates/client/src/network.rs`, in `run_network_wasm()`:

Remove the tick timer:

```rust
// DELETE these lines:
let mut tick = Box::pin(futures::stream::unfold((), |_| async {
    gloo_timers::future::TimeoutFuture::new(16).await;
    Some(((), ()))
}))
.fuse();
```

Add `cmd_rx.next()` as a select arm instead:

```rust
use futures::StreamExt;

// In the select! loop:
futures::select! {
    event = events.next() => {
        // ... existing event handling ...
    }
    cmd = cmd_rx.next() => {
        if let Some(cmd) = cmd {
            handle_network_command(&cmd, &node, &mut file_mgr)?;
        }
    }
    complete => break,
}
```

Remove the trailing `while let Ok(cmd) = cmd_rx.try_recv()` block after the select since commands are now handled inside the select.

- [ ] **Step 5: Cfg-gate native network functions**

The native `run_network()` and `spawn_network()` use `tokio::select!` which does not natively support `futures::channel::mpsc`. Since native is out of scope and allowed to break, cfg-gate the entire native block:

In `crates/client/src/network.rs`, the native `spawn_network` (line ~167) and `run_network` (line ~182) are already behind `#[cfg(not(target_arch = "wasm32"))]`. Leave them as-is — they will fail to compile on native due to the type change, which is acceptable. If clippy warns about dead code, add `#[allow(dead_code)]` to the native functions.

Alternatively, if the native functions cause compile errors even when targeting WASM (unlikely since they're cfg-gated), wrap them in `#[cfg(not(target_arch = "wasm32"))]` blocks that still use `std::sync::mpsc` internally and add a TODO comment.

- [ ] **Step 6: Remove `ClientNotification` from events.rs**

In `crates/client/src/events.rs`, delete the `ClientNotification` enum and its doc comment (lines 94-108).

In `crates/client/src/lib.rs`, remove the re-export:

```rust
// Delete:
pub use events::{ClientEvent, ClientNotification};
// Replace with:
pub use events::ClientEvent;
```

- [ ] **Step 7: Update `test_client()` to use new channel types**

In `crates/client/src/lib.rs`, update `test_client()`:

```rust
// Before:
let (cmd_tx, cmd_rx) = std_mpsc::channel();
let (_event_tx, event_rx) = std_mpsc::channel();

// After:
let (cmd_tx, cmd_rx) = futures_mpsc::unbounded();
let (_event_tx, event_rx) = futures_mpsc::unbounded();
```

Return type changes from `(Client, std::sync::mpsc::Receiver<NetworkCommand>)` to `(Client, futures_mpsc::UnboundedReceiver<NetworkCommand>)`.

- [ ] **Step 8: Verify compilation**

Run: `cargo check -p willow-client`

Expected: Compiles with zero errors. Warnings about unused `cmd_rx` in tests are OK.

- [ ] **Step 9: Run client tests**

Run: `cargo test -p willow-client`

Expected: All 93 tests pass. (Tests don't exercise the async event path — they use `test_client()` which doesn't connect.)

- [ ] **Step 10: Commit**

```bash
git add crates/client/
git commit -m "refactor: replace std::sync::mpsc with futures::channel::mpsc in client crate

Eliminates the 16ms command polling timer in the WASM network loop.
Commands are now awaited directly via futures::select!.
Removes ClientNotification enum (replaced by event channel later)."
```

---

### Task 2: Extract `SharedState` and `ClientHandle` from `Client`

**Files:**
- Modify: `crates/client/src/lib.rs`

This task introduces `SharedState` and `ClientHandle` alongside the existing `Client`. At the end, `Client` is removed and `ClientHandle` takes over.

- [ ] **Step 1: Define `SharedState` struct**

Add at the top of `crates/client/src/lib.rs` (after the imports):

```rust
use std::cell::RefCell;
use std::rc::Rc;

/// All mutable state shared between ClientHandle and ClientEventLoop.
pub struct SharedState {
    pub state: ClientState,
    pub identity: Identity,
    pub config: ClientConfig,
    pub connected: bool,
    pub connected_subscribed: bool,
    pub typing_peers: HashMap<String, (String, u64)>,
    pub voice_participants: HashMap<String, std::collections::HashSet<String>>,
    pub active_voice_channel: Option<String>,
    pub voice_muted: bool,
    pub voice_deafened: bool,
    pub state_verification_results: HashMap<String, willow_state::StateHash>,
    pub last_typing_sent_ms: u64,
}
```

- [ ] **Step 2: Define `ClientHandle` struct**

```rust
/// Cloneable command interface for UI components.
///
/// Wraps shared state and a network command sender. All mutation methods
/// update local state immediately (optimistic) and send commands to the
/// network. All read accessors return data from the shared state.
#[derive(Clone)]
pub struct ClientHandle {
    pub(crate) shared: Rc<RefCell<SharedState>>,
    pub(crate) cmd_tx: futures_mpsc::UnboundedSender<network::NetworkCommand>,
    /// Holds deferred channel halves until connect() consumes them.
    pub(crate) deferred_channels: Option<Rc<RefCell<Option<DeferredPair>>>>,
}
```

- [ ] **Step 3: Move read-only accessors from `Client` to `ClientHandle`**

Move these methods to `impl ClientHandle`, adapting `self.field` to `self.shared.borrow().field`:

- **Remove `state()` and `state_mut()`** — these returned `&ClientState` / `&mut ClientState` which cannot work through `RefCell`. Instead, callers that accessed `client.state().event_state` or `client.state().chat` should use specific accessors. Add any missing targeted accessors as needed (e.g., `event_state_roles()`, `active_server_context()`). Most downstream code already uses the specific accessors like `messages()`, `channels()`, `server_members()`.
- For `extract_roles()` in the web crate, which accesses `client.state().event_state.roles`, add a dedicated `pub fn roles_data(&self) -> Vec<(String, String, Vec<String>)>` accessor on `ClientHandle` that does the borrow internally.
- `peer_id()` → `self.shared.borrow().identity.peer_id().to_string()`
- `display_name()`, `peer_display_name()`, `server_display_name()`
- `messages()`, `channels()`, `channel_kinds()`, `peers()`, `server_members()`
- `is_connected()`, `has_servers()`, `server_list()`, `active_server_name()`, `active_server_id()`
- `pinned_message_ids()`, `pinned_messages()`, `is_pinned()`
- `voice_participants()`, `active_voice_channel()`, `is_voice_muted()`, `is_voice_deafened()`
- `state_hash_agreement()`
- `event_messages()`
- `typing_in()` — note this needs `borrow_mut()` since it prunes stale entries

For methods that return borrowed data (like `peers()` returning `&[String]`), change to return owned data (e.g., `Vec<String>`).

- [ ] **Step 4: Move mutation methods from `Client` to `ClientHandle`**

Move these methods, adapting `self.field` to `self.shared.borrow_mut().field` and `self.cmd_tx.send(...)` to `self.cmd_tx.unbounded_send(...)`:

- `connect()`, `send_message()`, `send_reply()`, `share_file_inline()`
- `edit_message()`, `delete_message()`, `react()`
- `pin_message()`, `unpin_message()`
- `create_channel()`, `create_voice_channel()`, `delete_channel()`, `switch_channel()`
- `trust_peer()`, `untrust_peer()`, `kick_member()`
- `create_role()`, `delete_role()`, `set_permission()`, `assign_role()`
- `create_server()`, `switch_server()`, `accept_invite()`
- `set_display_name()`, `set_server_display_name()`
- `join_voice()`, `leave_voice()`, `toggle_mute()`, `toggle_deafen()`, `send_voice_signal()`
- `send_typing()`, `verify_state()`, `rename_server()`, `set_server_description()`
- `generate_invite()`

Also move private helpers: `init_event_state_for_server()`, `reconcile_topic_map()`, `apply_event()`, `broadcast_event()`.

**Important: `on_connected()` stays on `ClientEventLoop`**, not `ClientHandle`. It is called during event processing (inside `process_batch`) while `SharedState` is already borrowed. If it were on `ClientHandle`, it would try to re-borrow `SharedState`, causing a runtime panic. Instead, `on_connected()` takes `&mut SharedState` and `&UnboundedSender<NetworkCommand>` as parameters, avoiding any re-borrow. It lives on `ClientEventLoop` which owns `cmd_tx`.

Pattern for each method:

```rust
// Before (on Client):
pub fn send_message(&mut self, channel: &str, body: &str) -> anyhow::Result<()> {
    // uses self.state, self.identity, self.cmd_tx
}

// After (on ClientHandle):
pub fn send_message(&self, channel: &str, body: &str) -> anyhow::Result<()> {
    let mut shared = self.shared.borrow_mut();
    // uses shared.state, shared.identity, self.cmd_tx
}
```

Note: methods that previously took `&mut self` can now take `&self` since mutation happens through `RefCell::borrow_mut()`.

- [ ] **Step 5: Delete the `Client` struct**

Remove the `Client` struct definition and its `impl` block. All methods now live on `ClientHandle`.

- [ ] **Step 6: Create the `new()` constructor**

```rust
/// Create a new client. Returns a handle for UI interaction and an event
/// loop to run in `spawn_local`.
///
/// Does **not** connect to the network — call [`ClientHandle::connect()`].
pub fn new(config: ClientConfig) -> (ClientHandle, ClientEventLoop) {
    let identity = load_identity();

    let (cmd_tx, cmd_rx) = futures_mpsc::unbounded();
    let (event_tx, event_rx) = futures_mpsc::unbounded();

    let deferred = Rc::new(RefCell::new(Some((event_tx, cmd_rx))));

    let mut state = ClientState::default();
    // ... existing state initialization from Client::new() ...

    let shared = Rc::new(RefCell::new(SharedState {
        state,
        identity,
        config,
        connected: false,
        connected_subscribed: false,
        typing_peers: HashMap::new(),
        voice_participants: HashMap::new(),
        active_voice_channel: None,
        voice_muted: false,
        voice_deafened: false,
        state_verification_results: HashMap::new(),
        last_typing_sent_ms: 0,
    }));

    let handle = ClientHandle {
        shared: shared.clone(),
        cmd_tx,
        deferred_channels: Some(deferred),
    };

    let event_loop = ClientEventLoop {
        shared,
        event_rx,
    };

    (handle, event_loop)
}
```

The body of this function is the existing `Client::new()` logic, restructured to populate `SharedState` instead of `Client` fields.

- [ ] **Step 7: Verify compilation**

Run: `cargo check -p willow-client`

Expected: Compiles. Tests will fail at this point (next task fixes them).

- [ ] **Step 8: Commit**

```bash
git add crates/client/src/lib.rs
git commit -m "refactor: split Client into SharedState + ClientHandle

Introduces SharedState (Rc<RefCell<>>) and ClientHandle (cloneable).
All 48 public methods moved from Client to ClientHandle.
Client struct removed."
```

---

### Task 3: Implement `ClientEventLoop`

**Files:**
- Modify: `crates/client/src/lib.rs`

- [ ] **Step 1: Define `ClientEventLoop` struct and `run()` method**

```rust
/// Async event processing loop. Owns the network event receiver.
/// Not cloneable — run exactly one instance via `spawn_local`.
pub struct ClientEventLoop {
    pub(crate) shared: Rc<RefCell<SharedState>>,
    pub(crate) event_rx: futures_mpsc::UnboundedReceiver<network::NetworkEvent>,
}

impl ClientEventLoop {
    /// Run the event processing loop.
    ///
    /// Awaits network events, applies them to shared state, and sends
    /// [`ClientEvent`]s to the provided sender. Returns when the network
    /// event channel closes or the sender is dropped.
    pub async fn run(mut self, tx: futures_mpsc::UnboundedSender<ClientEvent>) {
        use futures::StreamExt;

        // Profile re-broadcast schedule: 3s, 6s, 10s, 20s after connect.
        let profile_delays = [3000u32, 3000, 4000, 10000];
        let mut profile_idx = 0;
        let mut profile_timer = Box::pin(async {
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(profile_delays[0]).await;
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_millis(profile_delays[0] as u64)).await;
        })
        .fuse();

        loop {
            futures::select! {
                net_event = self.event_rx.next() => {
                    let Some(net_event) = net_event else {
                        // Network channel closed — shut down gracefully.
                        break;
                    };

                    // Drain any additional ready events for batching.
                    let mut batch = vec![net_event];
                    while let Ok(Some(more)) = self.event_rx.try_next() {
                        batch.push(more);
                    }

                    // Process the batch.
                    let client_events = self.process_batch(batch);
                    for event in client_events {
                        if tx.unbounded_send(event).is_err() {
                            // Receiver dropped — shut down.
                            return;
                        }
                    }
                }
                _ = profile_timer => {
                    // Broadcast profile at this interval.
                    self.broadcast_profile();
                    profile_idx += 1;
                    if profile_idx < profile_delays.len() {
                        profile_timer = Box::pin(async move {
                            #[cfg(target_arch = "wasm32")]
                            gloo_timers::future::TimeoutFuture::new(
                                profile_delays[profile_idx]
                            ).await;
                            #[cfg(not(target_arch = "wasm32"))]
                            tokio::time::sleep(std::time::Duration::from_millis(
                                profile_delays[profile_idx] as u64
                            )).await;
                        }).fuse();
                    } else {
                        // All re-broadcasts done. Replace with a future that never resolves.
                        profile_timer = Box::pin(futures::future::pending()).fuse();
                    }
                }
                complete => break,
            }
        }
    }
}
```

- [ ] **Step 2: Move `poll()` logic into `process_batch()`**

Move the body of the old `Client::poll()` into `ClientEventLoop::process_batch()`:

```rust
impl ClientEventLoop {
    fn process_batch(
        &self,
        net_events: Vec<network::NetworkEvent>,
    ) -> Vec<ClientEvent> {
        let mut shared = self.shared.borrow_mut();
        let mut events = Vec::new();

        for net_event in net_events {
            match net_event {
                // ... exact same match arms as the old poll(), but using
                // `shared.state`, `shared.identity`, etc. instead of `self.state`
            }
        }

        events
    }
}
```

Also move `emit_client_events_for()` to `ClientEventLoop` (it's only called during event processing).

- [ ] **Step 3: Add `broadcast_profile()` helper**

```rust
impl ClientEventLoop {
    fn broadcast_profile(&self) {
        let shared = self.shared.borrow();
        if !shared.connected_subscribed {
            return;
        }
        let saved = storage::load_profile().unwrap_or_default();
        if !saved.display_name.is_empty() {
            // ClientEventLoop doesn't own cmd_tx, so we need it.
            // Option A: store cmd_tx clone in the event loop.
            // Option B: send profile through shared state.
        }
    }
}
```

Note: The event loop needs to send network commands for profile re-broadcast. Add a `cmd_tx: futures_mpsc::UnboundedSender<NetworkCommand>` field to `ClientEventLoop`. The constructor clones it from the same sender used by `ClientHandle`.

Update `ClientEventLoop` struct:

```rust
pub struct ClientEventLoop {
    pub(crate) shared: Rc<RefCell<SharedState>>,
    pub(crate) event_rx: futures_mpsc::UnboundedReceiver<network::NetworkEvent>,
    pub(crate) cmd_tx: futures_mpsc::UnboundedSender<network::NetworkCommand>,
}
```

Update `new()` to clone `cmd_tx` for the event loop.

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p willow-client`

Expected: Compiles with zero errors.

- [ ] **Step 5: Commit**

```bash
git add crates/client/src/lib.rs
git commit -m "feat: implement ClientEventLoop with async event processing

Replaces the synchronous poll() method with an async run() loop.
Events are awaited via futures::select!, processed in batches,
and forwarded as ClientEvents. Profile re-broadcast uses real
timers instead of tick counting."
```

---

### Task 4: Fix client tests

**Files:**
- Modify: `crates/client/src/lib.rs` (test module)

- [ ] **Step 1: Rewrite `test_client()` as `test_handle()`**

```rust
pub(crate) fn test_handle() -> (ClientHandle, futures_mpsc::UnboundedReceiver<network::NetworkCommand>) {
    let identity = Identity::generate();
    let (cmd_tx, cmd_rx) = futures_mpsc::unbounded();
    let (_event_tx, event_rx) = futures_mpsc::unbounded();

    let mut state = ClientState::default();

    // Create a minimal server (same as before).
    let mut server = willow_channel::Server::new("Test Server", identity.peer_id());
    let ch_id = server
        .create_channel("general", willow_channel::ChannelKind::Text)
        .unwrap();
    let topic = util::make_topic(&server, "general");

    let server_id = server.id.to_string();
    let mut topic_map = HashMap::new();
    let mut keys = HashMap::new();

    if let Some(key) = server.channel_key(&ch_id) {
        keys.insert(topic.clone(), key.clone());
    }
    topic_map.insert(topic, ("general".to_string(), ch_id));

    let ctx = ServerContext {
        server,
        topic_map,
        keys,
        unread: HashMap::new(),
    };
    state.servers.insert(server_id.clone(), ctx);
    state.active_server = Some(server_id.clone());
    state.chat.current_channel = "general".to_string();

    // Initialize event state.
    let owner = identity.peer_id().to_string();
    state.event_state = willow_state::ServerState::new(
        server_id, "Test Server".to_string(), owner,
    );

    let shared = Rc::new(RefCell::new(SharedState {
        state,
        identity,
        config: ClientConfig { persistence: false, ..Default::default() },
        connected: false,
        connected_subscribed: false,
        typing_peers: HashMap::new(),
        voice_participants: HashMap::new(),
        active_voice_channel: None,
        voice_muted: false,
        voice_deafened: false,
        state_verification_results: HashMap::new(),
        last_typing_sent_ms: 0,
    }));

    let handle = ClientHandle {
        shared,
        cmd_tx,
        deferred_channels: None,
    };

    (handle, cmd_rx)
}
```

- [ ] **Step 2: Find-and-replace `test_client()` with `test_handle()` in all tests**

There are ~70 call sites. Replace:
- `let (mut client, cmd_rx) = test_client();` → `let (handle, cmd_rx) = test_handle();`
- `let (mut client, _) = test_client();` → `let (handle, _) = test_handle();`
- `let (client, _) = test_client();` → `let (handle, _) = test_handle();`

Then replace `client.method()` with `handle.method()` in each test. Since `ClientHandle` methods take `&self` instead of `&mut self`, remove `mut` from handle bindings where it was only needed for `&mut self`.

**Important:** Some tests directly access struct fields like `client.typing_peers.insert(...)`, `client.state.chat.current_channel`, `client.identity`, etc. These cannot be mechanically replaced — they need `handle.shared.borrow_mut().typing_peers.insert(...)` or equivalent. Search for all `client.` accesses that are NOT method calls (no parentheses) and adapt them to borrow through `shared`.

- [ ] **Step 3: Run all client tests**

Run: `cargo test -p willow-client`

Expected: All 93 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/client/src/lib.rs
git commit -m "test: migrate client tests to ClientHandle API

Replace test_client() with test_handle(). All 93 tests pass
with the new ClientHandle interface."
```

---

### Task 5: Create web crate state module and update dependencies

**Files:**
- Modify: `crates/web/Cargo.toml`
- Create: `crates/web/src/state.rs`
- Modify: `crates/web/src/app.rs` (add `mod state;`)

- [ ] **Step 1: Update web crate Cargo.toml**

Add `futures` to `[dependencies]`:

```toml
futures = "0.3"
```

Move `gloo-timers` from `[dev-dependencies]` to `[dependencies]`:

```toml
gloo-timers = { version = "0.3", features = ["futures"] }
```

- [ ] **Step 2: Create `state.rs` with all AppState sub-structs**

Create `crates/web/src/state.rs`:

```rust
use std::collections::HashMap;

use leptos::prelude::*;
use willow_client::DisplayMessage;

/// Per-channel UI state. Extensible for future needs (drafts, scroll pos).
#[derive(Clone, Default, PartialEq)]
pub struct ChannelViewState {
    pub typing: Vec<String>,
}

// ── Read signals (provided via context) ──────────────────────────────

#[derive(Clone, Copy)]
pub struct AppState {
    pub chat: ChatState,
    pub network: NetworkState,
    pub server: ServerState,
    pub ui: UiState,
    pub voice: VoiceState,
}

#[derive(Clone, Copy)]
pub struct ChatState {
    pub messages: ReadSignal<Vec<DisplayMessage>>,
    pub current_channel: ReadSignal<String>,
    pub channels: ReadSignal<Vec<String>>,
    pub replying_to: ReadSignal<Option<DisplayMessage>>,
    pub editing: ReadSignal<Option<DisplayMessage>>,
    pub pinned_messages: ReadSignal<Vec<DisplayMessage>>,
    pub pin_labels: ReadSignal<HashMap<String, String>>,
    pub channel_views: ReadSignal<HashMap<String, ChannelViewState>>,
}

#[derive(Clone, Copy)]
pub struct NetworkState {
    pub peers: ReadSignal<Vec<(String, String, bool)>>,
    pub peer_count: ReadSignal<usize>,
    pub peer_id: ReadSignal<String>,
    pub connection_status: ReadSignal<String>,
    pub loading: ReadSignal<bool>,
}

#[derive(Clone, Copy)]
pub struct ServerState {
    pub servers: ReadSignal<Vec<(String, String)>>,
    pub active_server_id: ReadSignal<String>,
    pub active_server_name: ReadSignal<String>,
    pub unread: ReadSignal<HashMap<String, usize>>,
    pub roles: ReadSignal<Vec<(String, String, Vec<String>)>>,
    pub display_name: ReadSignal<String>,
}

#[derive(Clone, Copy)]
pub struct UiState {
    pub show_settings: ReadSignal<bool>,
    pub show_server_settings: ReadSignal<bool>,
    pub show_sidebar: ReadSignal<bool>,
    pub show_members: ReadSignal<bool>,
    pub show_add_server: ReadSignal<bool>,
    pub show_pinned: ReadSignal<bool>,
}

#[derive(Clone, Copy)]
pub struct VoiceState {
    pub voice_channel: ReadSignal<Option<String>>,
    pub voice_muted: ReadSignal<bool>,
    pub voice_deafened: ReadSignal<bool>,
    pub voice_participants_map: ReadSignal<HashMap<String, Vec<String>>>,
    pub voice_channel_name: ReadSignal<String>,
}

// ── Write signals (NOT in context — held by event processing) ────────

#[derive(Clone, Copy)]
pub struct AppWriteSignals {
    pub chat: ChatWriteSignals,
    pub network: NetworkWriteSignals,
    pub server: ServerWriteSignals,
    pub ui: UiWriteSignals,
    pub voice: VoiceWriteSignals,
}

#[derive(Clone, Copy)]
pub struct ChatWriteSignals {
    pub set_messages: WriteSignal<Vec<DisplayMessage>>,
    pub set_current_channel: WriteSignal<String>,
    pub set_channels: WriteSignal<Vec<String>>,
    pub set_replying_to: WriteSignal<Option<DisplayMessage>>,
    pub set_editing: WriteSignal<Option<DisplayMessage>>,
    pub set_pinned_messages: WriteSignal<Vec<DisplayMessage>>,
    pub set_pin_labels: WriteSignal<HashMap<String, String>>,
    pub set_channel_views: WriteSignal<HashMap<String, ChannelViewState>>,
}

#[derive(Clone, Copy)]
pub struct NetworkWriteSignals {
    pub set_peers: WriteSignal<Vec<(String, String, bool)>>,
    pub set_peer_count: WriteSignal<usize>,
    pub set_peer_id: WriteSignal<String>,
    pub set_connection_status: WriteSignal<String>,
    pub set_loading: WriteSignal<bool>,
}

#[derive(Clone, Copy)]
pub struct ServerWriteSignals {
    pub set_servers: WriteSignal<Vec<(String, String)>>,
    pub set_active_server_id: WriteSignal<String>,
    pub set_active_server_name: WriteSignal<String>,
    pub set_unread: WriteSignal<HashMap<String, usize>>,
    pub set_roles: WriteSignal<Vec<(String, String, Vec<String>)>>,
    pub set_display_name: WriteSignal<String>,
}

#[derive(Clone, Copy)]
pub struct UiWriteSignals {
    pub set_show_settings: WriteSignal<bool>,
    pub set_show_server_settings: WriteSignal<bool>,
    pub set_show_sidebar: WriteSignal<bool>,
    pub set_show_members: WriteSignal<bool>,
    pub set_show_add_server: WriteSignal<bool>,
    pub set_show_pinned: WriteSignal<bool>,
}

#[derive(Clone, Copy)]
pub struct VoiceWriteSignals {
    pub set_voice_channel: WriteSignal<Option<String>>,
    pub set_voice_muted: WriteSignal<bool>,
    pub set_voice_deafened: WriteSignal<bool>,
    pub set_voice_participants_map: WriteSignal<HashMap<String, Vec<String>>>,
    pub set_voice_channel_name: WriteSignal<String>,
}

/// Create all signal pairs and return the read/write halves.
pub fn create_signals() -> (AppState, AppWriteSignals) {
    let (messages, set_messages) = signal(Vec::<DisplayMessage>::new());
    let (current_channel, set_current_channel) = signal(String::from("general"));
    let (channels, set_channels) = signal(Vec::<String>::new());
    let (replying_to, set_replying_to) = signal(Option::<DisplayMessage>::None);
    let (editing, set_editing) = signal(Option::<DisplayMessage>::None);
    let (pinned_messages, set_pinned_messages) = signal(Vec::<DisplayMessage>::new());
    let (pin_labels, set_pin_labels) = signal(HashMap::<String, String>::new());
    let (channel_views, set_channel_views) = signal(HashMap::<String, ChannelViewState>::new());

    let (peers, set_peers) = signal(Vec::<(String, String, bool)>::new());
    let (peer_count, set_peer_count) = signal(0usize);
    let (peer_id, set_peer_id) = signal(String::new());
    let (connection_status, set_connection_status) = signal("connecting".to_string());
    let (loading, set_loading) = signal(true);

    let (servers, set_servers) = signal(Vec::<(String, String)>::new());
    let (active_server_id, set_active_server_id) = signal(String::new());
    let (active_server_name, set_active_server_name) = signal(String::new());
    let (unread, set_unread) = signal(HashMap::<String, usize>::new());
    let (roles, set_roles) = signal(Vec::<(String, String, Vec<String>)>::new());
    let (display_name, set_display_name) = signal(String::new());

    let (show_settings, set_show_settings) = signal(false);
    let (show_server_settings, set_show_server_settings) = signal(false);
    let (show_sidebar, set_show_sidebar) = signal(false);
    let (show_members, set_show_members) = signal(false);
    let (show_add_server, set_show_add_server) = signal(false);
    let (show_pinned, set_show_pinned) = signal(false);

    let (voice_channel, set_voice_channel) = signal(Option::<String>::None);
    let (voice_muted, set_voice_muted) = signal(false);
    let (voice_deafened, set_voice_deafened) = signal(false);
    let (voice_participants_map, set_voice_participants_map) =
        signal(HashMap::<String, Vec<String>>::new());
    let (voice_channel_name, set_voice_channel_name) = signal(String::new());

    let app_state = AppState {
        chat: ChatState {
            messages, current_channel, channels, replying_to, editing,
            pinned_messages, pin_labels, channel_views,
        },
        network: NetworkState {
            peers, peer_count, peer_id, connection_status, loading,
        },
        server: ServerState {
            servers, active_server_id, active_server_name, unread, roles, display_name,
        },
        ui: UiState {
            show_settings, show_server_settings, show_sidebar, show_members,
            show_add_server, show_pinned,
        },
        voice: VoiceState {
            voice_channel, voice_muted, voice_deafened, voice_participants_map,
            voice_channel_name,
        },
    };

    let write_signals = AppWriteSignals {
        chat: ChatWriteSignals {
            set_messages, set_current_channel, set_channels, set_replying_to,
            set_editing, set_pinned_messages, set_pin_labels, set_channel_views,
        },
        network: NetworkWriteSignals {
            set_peers, set_peer_count, set_peer_id, set_connection_status, set_loading,
        },
        server: ServerWriteSignals {
            set_servers, set_active_server_id, set_active_server_name, set_unread,
            set_roles, set_display_name,
        },
        ui: UiWriteSignals {
            set_show_settings, set_show_server_settings, set_show_sidebar,
            set_show_members, set_show_add_server, set_show_pinned,
        },
        voice: VoiceWriteSignals {
            set_voice_channel, set_voice_muted, set_voice_deafened,
            set_voice_participants_map, set_voice_channel_name,
        },
    };

    (app_state, write_signals)
}
```

- [ ] **Step 3: Add `mod state;` to main.rs and update the type alias**

In `crates/web/src/main.rs`, add `mod state;` to the module declarations.

In `crates/web/src/app.rs`, update the `ClientHandle` type alias:

```rust
// Before:
pub type ClientHandle = SendWrapper<Rc<RefCell<Client>>>;

// After:
/// Wrapper around `ClientHandle` that is `Send` for single-threaded WASM.
pub type WebClientHandle = SendWrapper<willow_client::ClientHandle>;
```

Keep `VoiceManagerHandle` as-is.

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p willow-web`

Expected: Compiles (state.rs is defined but not yet used — that's fine).

- [ ] **Step 5: Commit**

```bash
git add crates/web/
git commit -m "feat: add web state module with AppState and AppWriteSignals

Defines structured signal containers for context-based state management.
create_signals() produces all 30 signal pairs grouped into sub-structs."
```

---

### Task 6: Create event processing and handlers modules

**Files:**
- Create: `crates/web/src/event_processing.rs`
- Create: `crates/web/src/handlers.rs`
- Modify: `crates/web/src/voice.rs`
- Modify: `crates/web/src/app.rs` (add mod declarations)

- [ ] **Step 1: Create `event_processing.rs`**

Create `crates/web/src/event_processing.rs`. Extract the body of the current poll loop (app.rs lines 155-324) into `process_event_batch()`:

```rust
use std::collections::HashMap;

use willow_client::{ClientEvent, VoiceSignalPayload};

use crate::app::{WebClientHandle, VoiceManagerHandle};
use crate::state::AppWriteSignals;
use crate::voice::handle_voice_event;

/// Process a batch of ClientEvents and update signals.
///
/// Uses the same flag-based batching as the original poll loop:
/// collect flags from all events, then do one signal update pass.
pub fn process_event_batch(
    events: &[ClientEvent],
    handle: &WebClientHandle,
    state: &AppState,
    write: &AppWriteSignals,
    voice_manager: &VoiceManagerHandle,
) {
    let mut needs_msg_refresh = false;
    let mut needs_peer_refresh = false;
    let mut needs_channel_refresh = false;

    for event in events {
        match event {
            ClientEvent::MessageReceived { .. } => {
                needs_msg_refresh = true;
            }
            ClientEvent::MessageEdited { .. }
            | ClientEvent::MessageDeleted { .. }
            | ClientEvent::ReactionAdded { .. }
            | ClientEvent::SyncCompleted { .. } => {
                needs_msg_refresh = true;
            }
            ClientEvent::PeerConnected(_) => {
                needs_peer_refresh = true;
                write.network.set_connection_status.set("connected".to_string());
                write.network.set_loading.set(false);
            }
            ClientEvent::PeerDisconnected(_) => {
                needs_peer_refresh = true;
            }
            ClientEvent::Listening(_) => {}
            ClientEvent::ChannelCreated(_) | ClientEvent::ChannelDeleted(_) => {
                needs_channel_refresh = true;
            }
            ClientEvent::ProfileUpdated { .. } => {
                let c = handle.borrow();
                write.server.set_display_name.set(c.display_name());
                needs_peer_refresh = true;
            }
            ClientEvent::VoiceJoined { .. }
            | ClientEvent::VoiceLeft { .. }
            | ClientEvent::VoiceSignal { .. } => {
                handle_voice_event(event, handle, state, write, voice_manager);
            }
            _ => {}
        }
    }

    let current_channel = state.chat.current_channel.get_untracked();
    let c = handle.borrow();
    if needs_msg_refresh {
        // Same smart-diff logic as the original poll loop.
        let new_msgs = c.messages(&current_channel);
        let old_msgs = state.chat.messages.get_untracked();
        // ... change detection ...
        write.chat.set_messages.set(new_msgs);

        // Refresh pinned, pin labels, unread.
        write.chat.set_pinned_messages.set(c.pinned_messages(&current_channel));
        // ... pin labels ...
        // ... unread map ...
    }
    if needs_peer_refresh {
        let peer_list = c.server_members();
        let count = peer_list.iter().filter(|(_, _, online)| *online).count();
        write.network.set_peers.set(peer_list);
        write.network.set_peer_count.set(count);
        if count > 0 {
            write.network.set_connection_status.set("connected".to_string());
        } else {
            write.network.set_connection_status.set("connecting".to_string());
        }
    }
    if needs_channel_refresh {
        write.chat.set_channels.set(c.channels());
        write.server.set_roles.set(extract_roles(&c));
    }
    if needs_msg_refresh || needs_peer_refresh {
        write.server.set_roles.set(extract_roles(&c));
    }
}

/// Refresh all signals from client state. Used after server create/join/switch.
pub fn refresh_all_signals(handle: &WebClientHandle, write: &AppWriteSignals) {
    let c = handle.borrow();
    write.server.set_servers.set(c.server_list());
    write.chat.set_channels.set(c.channels());
    write.network.set_peer_id.set(c.peer_id());
    write.server.set_display_name.set(c.display_name());
    write.server.set_roles.set(extract_roles(&c));
    if let Some(id) = c.active_server_id() {
        write.server.set_active_server_id.set(id.to_string());
    }
    write.server.set_active_server_name.set(c.active_server_name());
    let ch = c.channels().first().cloned().unwrap_or("general".to_string());
    write.chat.set_current_channel.set(ch.clone());
    write.chat.set_messages.set(c.messages(&ch));
    write.ui.set_show_settings.set(false);
    write.ui.set_show_server_settings.set(false);
    write.ui.set_show_add_server.set(false);
}

/// Extract roles from client state.
/// Uses the `roles_data()` accessor on ClientHandle which borrows internally.
pub fn extract_roles(handle: &WebClientHandle) -> Vec<(String, String, Vec<String>)> {
    handle.borrow().roles_data()
}
```

Note: The exact logic inside `process_event_batch` is lifted directly from the current poll loop in `app.rs:155-324`. The code above shows the structure — the full implementation copies each branch verbatim from the existing code, adapting signal names (`set_messages` → `write.chat.set_messages`, etc.).

- [ ] **Step 2: Add voice event helper to `voice.rs`**

Add to `crates/web/src/voice.rs`:

```rust
use willow_client::ClientEvent;
use crate::app::{WebClientHandle, VoiceManagerHandle};
use crate::state::AppWriteSignals;

/// Handle voice-related ClientEvents from the event processing batch.
pub fn handle_voice_event(
    event: &ClientEvent,
    _handle: &WebClientHandle,
    state: &AppState,
    write: &AppWriteSignals,
    voice_manager: &VoiceManagerHandle,
) {
    match event {
        ClientEvent::VoiceJoined { channel_id, peer_id } => {
            write.voice.set_voice_participants_map.update(|m| {
                let participants = m.entry(channel_id.clone()).or_default();
                if !participants.contains(peer_id) {
                    participants.push(peer_id.clone());
                }
            });
            // If we're in this channel, create offer to new peer.
            let current_vc = state.voice.voice_channel.get_untracked();
            if current_vc.as_deref() == Some(channel_id) {
                let vm = voice_manager.clone();
                let pid = peer_id.clone();
                wasm_bindgen_futures::spawn_local(
                    crate::voice::create_offer(vm, pid)
                );
            }
        }
        ClientEvent::VoiceLeft { channel_id, peer_id } => {
            write.voice.set_voice_participants_map.update(|m| {
                if let Some(v) = m.get_mut(channel_id) {
                    v.retain(|p| p != peer_id);
                }
            });
            voice_manager.borrow_mut().close_connection(peer_id);
        }
        ClientEvent::VoiceSignal { from_peer, signal, .. } => {
            // Same spawn_local pattern as current app.rs
            let vm = voice_manager.clone();
            let from = from_peer.clone();
            match signal {
                willow_client::VoiceSignalPayload::Offer(sdp) => {
                    wasm_bindgen_futures::spawn_local(
                        crate::voice::handle_offer(vm, from, sdp.clone())
                    );
                }
                willow_client::VoiceSignalPayload::Answer(sdp) => {
                    wasm_bindgen_futures::spawn_local(
                        crate::voice::handle_answer(vm, from, sdp.clone())
                    );
                }
                willow_client::VoiceSignalPayload::IceCandidate(json) => {
                    let _ = vm.borrow().handle_ice_candidate(&from, json);
                }
            }
        }
        _ => {}
    }
}
```

Rename the existing `handle_voice_create_offer`, `handle_voice_offer`, `handle_voice_answer` functions in `app.rs` to `create_offer`, `handle_offer`, `handle_answer` and move to `voice.rs` (or make them `pub` so the new module can call them).

- [ ] **Step 3: Create `handlers.rs`**

Create `crates/web/src/handlers.rs`:

```rust
use std::collections::HashMap;

use leptos::prelude::*;
use willow_client::DisplayMessage;

use crate::app::WebClientHandle;
use crate::event_processing::{extract_roles, refresh_all_signals};
use crate::state::{AppState, AppWriteSignals};

// **Important:** In Leptos 0.7, `WriteSignal<T>` does NOT have `get_untracked()`.
// Only `ReadSignal<T>` does. All handlers that need to read current values must
// use the `AppState` (read signals), not `AppWriteSignals` (write signals).
// Handler constructors take BOTH `AppState` and `AppWriteSignals`.

/// Send message or reply handler.
pub fn make_send_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn(String) + Clone {
    move |body: String| {
        let ch = state.chat.current_channel.get_untracked();
        let c = handle.borrow();
        let replying = state.chat.replying_to.get_untracked();
        if let Some(reply_msg) = replying {
            let _ = c.send_reply(&ch, &reply_msg.id, &body);
            write.chat.set_replying_to.set(None);
        } else {
            let _ = c.send_message(&ch, &body);
        }
        write.chat.set_messages.set(c.messages(&ch));
    }
}

/// Edit message handler.
pub fn make_edit_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn((String, String)) + Clone {
    move |(message_id, new_body): (String, String)| {
        let ch = state.chat.current_channel.get_untracked();
        let c = handle.borrow();
        let _ = c.edit_message(&ch, &message_id, &new_body);
        write.chat.set_editing.set(None);
        write.chat.set_messages.set(c.messages(&ch));
    }
}

/// Delete message handler.
pub fn make_delete_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn(DisplayMessage) + Clone {
    move |msg: DisplayMessage| {
        let ch = state.chat.current_channel.get_untracked();
        let c = handle.borrow();
        let _ = c.delete_message(&ch, &msg.id);
        write.chat.set_messages.set(c.messages(&ch));
    }
}

/// React handler.
pub fn make_react_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn((DisplayMessage, String)) + Clone {
    move |(msg, emoji): (DisplayMessage, String)| {
        let ch = state.chat.current_channel.get_untracked();
        let c = handle.borrow();
        let _ = c.react(&ch, &msg.id, &emoji);
        write.chat.set_messages.set(c.messages(&ch));
    }
}

/// Channel switch handler.
pub fn make_channel_click_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn(String) + Clone {
    move |name: String| {
        write.chat.set_current_channel.set(name.clone());
        write.ui.set_show_sidebar.set(false);
        write.ui.set_show_pinned.set(false);
        {
            let c = handle.borrow();
            write.chat.set_messages.set(c.messages(&name));
            write.chat.set_pinned_messages.set(c.pinned_messages(&name));
            let mut labels = HashMap::new();
            for msg in c.messages(&name) {
                let label = if c.is_pinned(&name, &msg.id) { "Unpin" } else { "Pin" };
                labels.insert(msg.id.clone(), label.to_string());
            }
            write.chat.set_pin_labels.set(labels);
        }
        handle.borrow().switch_channel(&name);
        write.server.set_unread.update(|m| { m.remove(&name); });
    }
}

/// Server switch handler.
pub fn make_server_click_handler(
    handle: WebClientHandle,
    _state: AppState,
    write: AppWriteSignals,
) -> impl Fn(String) + Clone {
    move |id: String| {
        handle.borrow().switch_server(&id);
        refresh_all_signals(&handle, &write);
    }
}

/// Pin/unpin handler.
pub fn make_pin_handler(
    handle: WebClientHandle,
    state: AppState,
    write: AppWriteSignals,
) -> impl Fn(DisplayMessage) + Clone {
    move |msg: DisplayMessage| {
        let ch = state.chat.current_channel.get_untracked();
        let c = handle.borrow();
        if c.is_pinned(&ch, &msg.id) {
            let _ = c.unpin_message(&ch, &msg.id);
        } else {
            let _ = c.pin_message(&ch, &msg.id);
        }
        write.chat.set_pinned_messages.set(c.pinned_messages(&ch));
        let mut labels = HashMap::new();
        for m in c.messages(&ch) {
            let label = if c.is_pinned(&ch, &m.id) { "Unpin" } else { "Pin" };
            labels.insert(m.id.clone(), label.to_string());
        }
        write.chat.set_pin_labels.set(labels);
    }
}
```

- [ ] **Step 4: Add mod declarations to `main.rs`**

The web crate root is `crates/web/src/main.rs`. Add the new modules there (not in `app.rs`):

```rust
mod app;
mod components;
mod event_processing;
mod handlers;
mod state;
pub(crate) mod util;
pub mod voice;
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p willow-web`

Expected: Compiles. The new modules are defined. The old App component still exists and works for now.

- [ ] **Step 6: Commit**

```bash
git add crates/web/src/
git commit -m "feat: add event_processing and handlers modules

Extract poll loop logic into process_event_batch().
Extract action handlers into named constructor functions.
Add voice event handling helper to voice.rs."
```

---

### Task 7: Rewrite App component

**Files:**
- Modify: `crates/web/src/app.rs`

This is the biggest single change. The 800-line `App` function is gutted and rebuilt using the new modules.

- [ ] **Step 1: Rewrite App component**

Replace the body of the `App` component in `crates/web/src/app.rs`. The new version:

1. Creates `ClientHandle` + `ClientEventLoop` via `willow_client::new()`
2. Calls `handle.connect()`
3. Creates signals via `state::create_signals()`
4. Populates initial state via `event_processing::refresh_all_signals()`
5. Creates `VoiceManager`
6. Provides context: `AppState`, `WebClientHandle`, `VoiceManagerHandle`
7. Spawns event loop task
8. Spawns signal updater task
9. Spawns typing expiry timer
10. Sets loading timeout
11. Creates handlers via `handlers::make_*`
12. Renders the layout (same view tree as before, but with context instead of props)

The `init_theme()`, `toggle_theme()`, `LOADING_TIMEOUT_MS`, and `DEFAULT_RELAY` constants stay in `app.rs`.

Remove the old `ClientHandle` type alias. Add `WebClientHandle`:

```rust
pub type WebClientHandle = SendWrapper<willow_client::ClientHandle>;
```

The view template stays structurally the same but components lose most props (they pull from context). Components that still need callbacks receive them as `Callback` props.

- [ ] **Step 2: Spawn the event loop and signal updater**

Inside `App`, after `provide_context`:

```rust
// Spawn the client event loop.
let (client_event_tx, client_event_rx) = futures::channel::mpsc::unbounded();
wasm_bindgen_futures::spawn_local(event_loop.run(client_event_tx));

// Spawn the signal updater.
let updater_handle = handle.clone();
let updater_state = app_state;  // AppState is Copy (all fields are Copy signals)
let updater_write = write_signals;
let updater_vm = voice_manager.clone();
wasm_bindgen_futures::spawn_local(async move {
    use futures::StreamExt;
    let mut rx = client_event_rx;
    while let Some(event) = rx.next().await {
        let mut batch = vec![event];
        while let Ok(Some(more)) = rx.try_next() {
            batch.push(more);
        }
        event_processing::process_event_batch(
            &batch, &updater_handle, &updater_state, &updater_write, &updater_vm,
        );
    }
});
```

- [ ] **Step 3: Spawn typing expiry timer**

```rust
// Typing indicator expiry timer (~2s).
let typing_handle = handle.clone();
let typing_write = write_signals;
wasm_bindgen_futures::spawn_local(async move {
    use futures::StreamExt;
    let mut interval = gloo_timers::future::IntervalStream::new(2_000);
    while interval.next().await.is_some() {
        let c = typing_handle.borrow();  // Note: typing_in() needs mut, so use borrow_mut() if it prunes stale entries
        let mut views = typing_write.chat.set_channel_views.get_untracked();  // Read current value
        let mut changed = false;
        for ch_name in c.channels() {
            let typers = c.typing_in(&ch_name);
            let current = views.get(&ch_name).map(|v| &v.typing);
            if current != Some(&typers) {
                views.insert(ch_name, crate::state::ChannelViewState { typing: typers });
                changed = true;
            }
        }
        if changed {
            typing_write.chat.set_channel_views.set(views);
        }
    }
});
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p willow-web`

Expected: Compiles. May have warnings about unused old code — that's fine, it will be cleaned up next.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/app.rs
git commit -m "refactor: rewrite App component with context-based state

Replace 50ms poll loop with spawn_local event processing.
Replace 30 prop-drilled signals with AppState context.
App is now ~80 lines of setup + layout."
```

---

### Task 8: Migrate components to context

**Files:**
- Modify: `crates/web/src/components/sidebar.rs`
- Modify: `crates/web/src/components/member_list.rs`
- Modify: `crates/web/src/components/welcome.rs`
- Modify: `crates/web/src/components/add_server.rs`
- Modify: `crates/web/src/components/server_settings.rs`
- Modify: `crates/web/src/components/settings.rs`
- Modify: `crates/web/src/components/file_share.rs`
- Modify: `crates/web/src/components/roles.rs`
- Modify: `crates/web/src/components/chat.rs`
- Modify: `crates/web/src/components/mod.rs`

All 8 components that import `crate::app::ClientHandle` switch to `use_context`.

- [ ] **Step 1: Update component imports**

In each of the 8 component files, replace:

```rust
// Before:
use crate::app::ClientHandle;

// After:
use crate::app::WebClientHandle;
use crate::state::AppState;
```

- [ ] **Step 2: Remove `client: ClientHandle` from component props**

For each component, remove the `client` prop and instead pull from context:

```rust
// Before:
#[component]
pub fn Sidebar(client: ClientHandle, /* many other props */) -> impl IntoView {

// After:
#[component]
pub fn Sidebar(/* only callback props that can't come from context */) -> impl IntoView {
    let handle = use_context::<WebClientHandle>().unwrap();
    let state = use_context::<AppState>().unwrap();
```

Replace `client.borrow()` with `handle.borrow()` and signal prop access with `state.chat.channels.get()` etc.

- [ ] **Step 3: Remove signal props that now come from context**

For example, `Sidebar` currently takes `channels`, `current_channel`, `unread`, `connection_status`, `peer_count`, `server_name`, etc. as props. Remove all of these. The component reads them from `state`:

```rust
// Before (prop):
let chs = channels.get();

// After (context):
let chs = state.chat.channels.get();
```

Keep callback props that are specific to the component's parent context (like `on_channel_click`, `on_voice_join`).

- [ ] **Step 4: Update `chat.rs` (ChannelHeader, MessageList)**

These components mostly receive signals as props. Replace with context reads. `MessageList` keeps its callback props (`on_message_click`, `on_edit`, `on_delete`, `on_react`, `on_pin`).

- [ ] **Step 5: Update `app.rs` view template**

Remove props from component invocations that are now provided via context. For example:

```rust
// Before:
<Sidebar
    channels=channels
    current_channel=current_channel
    open=show_sidebar
    unread=unread
    connection_status=connection_status
    peer_count=peer_count
    server_name=active_server_name
    client=sbc
    on_channel_click=ch_click
    // ... 10 more props
/>

// After:
<Sidebar
    on_channel_click=ch_click
    on_voice_join=voice_join_handler
/>
```

- [ ] **Step 6: Update browser test helpers to provide context**

Browser tests use `mount_test(|| view! { ... })` to render components in isolation. Components that now use `use_context::<AppState>()` will panic if no context is provided. Update the test helper or individual tests to wrap component rendering with `provide_context`:

```rust
fn mount_test_with_context(f: impl FnOnce() -> impl IntoView + 'static) {
    // Create mock AppState with default signals
    let (app_state, _write) = crate::state::create_signals();
    let handle = /* create a mock WebClientHandle with no-op channels */;
    mount_test(move || {
        provide_context(app_state);
        provide_context(handle);
        f()
    });
}
```

Not all 39 browser tests will need this — only tests that render components which use `use_context`. Tests that render pure components (like `MessageView` which only takes props) are unaffected. Update tests incrementally: run `just test-browser`, fix failures one at a time.

- [ ] **Step 7: Clean up unused old code**

Remove any remaining dead code from `app.rs`:
- Old `ClientHandle` type alias (replaced by `WebClientHandle`)
- Old `handle_voice_create_offer`, `handle_voice_offer`, `handle_voice_answer` functions (moved to `voice.rs`)
- Old `extract_roles` function (moved to `event_processing.rs`)

- [ ] **Step 8: Verify compilation**

Run: `cargo check -p willow-web`

Expected: Compiles with zero errors and zero warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/web/
git commit -m "refactor: migrate all components to context-based state

8 components switch from ClientHandle prop to use_context.
Signal props replaced with AppState context reads.
Components are simpler with 1-2 callback props instead of 15+."
```

---

### Task 9: Full verification

**Files:** None (testing only)

- [ ] **Step 1: Run client tests**

Run: `cargo test -p willow-client`

Expected: All 93 tests pass.

- [ ] **Step 2: Check WASM compilation**

Run: `just check-wasm`

Expected: Compiles for `wasm32-unknown-unknown` with zero errors.

- [ ] **Step 3: Run clippy**

Run: `just clippy`

Expected: Zero warnings.

- [ ] **Step 4: Run formatter**

Run: `just fmt`

Expected: No changes (or fix any formatting issues).

- [ ] **Step 5: Run full check**

Run: `just check`

Expected: All checks pass (fmt + clippy + test + WASM).

- [ ] **Step 6: Run browser tests (if Firefox + geckodriver available)**

Run: `just test-browser`

Expected: All 39 browser tests pass.

- [ ] **Step 7: Final commit if any fixes were needed**

```bash
git add -A
git commit -m "fix: address lint and test issues from refactor"
```
