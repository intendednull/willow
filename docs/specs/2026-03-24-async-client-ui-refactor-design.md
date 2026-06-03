# Async Client + UI Refactor Design

## Problem

The Willow web UI has two layers of polling that add latency and waste CPU:

1. **WASM network loop** polls `cmd_rx.try_recv()` every 16ms via a `gloo_timers` interval to check for outbound commands.
2. **Leptos UI** polls `client.poll()` every 50ms via `set_interval` to drain inbound events.

Both use `std::sync::mpsc` which has no async awareness, forcing timer-based polling.

Additionally, the Leptos `App` component is an 800-line monolith that creates 30 loose signals, threads them as props through components, clones `Rc<RefCell<Client>>` 14 times, and mixes event handling, voice WebRTC, state management, and layout in a single function.

## Scope

- **In scope:** `willow-client` crate (async channels, Client split), `willow-web` crate (UI state context, event-driven updates, App breakup).
- **Out of scope:** Bevy native app (`crates/app/`). The refactored Client API will break the Bevy integration. This is acceptable since Bevy is disabled. It can be re-adapted later if needed.
- **Out of scope:** Yew frontend. This design makes the client framework-agnostic, which will make a future Yew frontend straightforward, but building it is not part of this work.

## Prior Art

This refactor adapts established UI and concurrency architecture patterns to a single-threaded WASM client:

| Prior art | Relevance to this design |
|---|---|
| **The Elm Architecture (TEA)** (Evan Czaplicki, Elm; 2012) | Model/Update/View with strict unidirectional flow: messages drive state, state drives the view. Mirrored here by `NetworkEvent` -> `ClientEventLoop` -> `ClientEvent` -> signal updater -> Leptos view, with no view-to-state back-channel. |
| **Redux / Flux** (Dan Abramov & Andrew Clark, 2015; Facebook Flux, 2014) | A read-only store mutated only via dispatched actions through one processing path. Maps to the read/write signal split: `ReadSignal` halves live in `AppState` context (read-only to components); `WriteSignal` halves (`AppWriteSignals`) are held solely by the event-processing layer. |
| **CQRS** (Greg Young, ~2010; building on Bertrand Meyer's Command-Query Separation, Eiffel) | Separate the command/write model from the read/query model. Mirrored by `ClientHandle` (cloneable command + read interface, optimistic local writes) vs. the single non-cloneable `ClientEventLoop` (exclusive async event processor), and again by the `ReadSignal`/`WriteSignal` split. |
| **Actor handle/inbox pattern** (actix `Addr<A>`; ractor; kameo) | A cheaply-cloneable address that enqueues messages to exactly one owning message loop. Directly parallels the cloneable `ClientHandle` (sends `NetworkCommand`s over an `UnboundedSender`) plus the single owning `ClientEventLoop` that drains the inbox and is the only writer of shared state. |
| **iced / relm4** (Elm-inspired Rust GUIs; iced on `futures`, relm4 on gtk4-rs) | Demonstrate TEA's `Message`/`update`/`view` in Rust; iced's first-class async actions turn `futures` into the message source — the substitute for polling that this design also adopts (`select!` over `futures::channel::mpsc` instead of a 16ms/50ms interval poll). |
| **SolidJS fine-grained signals** (the acknowledged inspiration for Leptos) | `createSignal` returns a `[getter, setter]` tuple, making read/write segregation first-class and enabling targeted DOM updates with no virtual DOM. This design leans on the same split (`ReadSignal`/`WriteSignal`) and propagates the read halves via `provide_context` rather than threading 30 signals as props. |
| **Stream `select!` event loops** (`futures`, `tokio`) | Backpressure-free async channels (`futures::channel::mpsc::unbounded`) consumed via `select!` replace timer-driven `std::sync::mpsc` polling, so commands and events are delivered the instant they are produced. |

Out of scope but informed by the same lineage: **Yew / Seed** (Elm-style Rust/WASM frontends) are named future targets — the framework-agnostic `ClientHandle`/`ClientEventLoop` split is intended to make such a frontend straightforward later.

## Design

### 1. Async Channels

Replace `std::sync::mpsc` with `futures::channel::mpsc::unbounded` for both directions between the network and client.

**Commands (UI -> Network):**

- `ClientHandle` holds `UnboundedSender<NetworkCommand>`.
- WASM network loop `select!`s on `cmd_rx.next()` directly, eliminating the 16ms tick timer.
- Commands are delivered instantly.

**Events (Network -> UI):**

- Network loop sends `NetworkEvent`s on `UnboundedSender<NetworkEvent>`.
- `ClientEventLoop` awaits `event_rx.next()`, eliminating all polling.
- Events arrive the instant the network produces them.

**`spawn_network()` signature change:** Both the WASM and native `spawn_network()` functions change from `std::sync::mpsc` types to `futures::channel::mpsc` types:

```rust
// Before:
pub fn spawn_network(
    identity: Identity,
    event_tx: std::sync::mpsc::Sender<NetworkEvent>,
    cmd_rx: std::sync::mpsc::Receiver<NetworkCommand>,
    config: NetworkConfig,
)

// After:
pub fn spawn_network(
    identity: Identity,
    event_tx: futures::channel::mpsc::UnboundedSender<NetworkEvent>,
    cmd_rx: futures::channel::mpsc::UnboundedReceiver<NetworkCommand>,
    config: NetworkConfig,
)
```

The WASM `run_network_wasm()` replaces the 16ms tick timer with `cmd_rx.next()` in its `futures::select!`. The native `run_network()` replaces the 16ms `tokio::time::sleep` poll with `cmd_rx.next()` in its `tokio::select!` (this requires wrapping the `futures::channel` receiver with `tokio_util::compat` or switching the native side to `tokio::sync::mpsc` behind a cfg gate). Since native is allowed to break, the simplest path is to update both to use `futures::channel::mpsc` and fix the native side minimally or cfg-gate it.

**Deferred channels mechanism:** The current `DeferredPair` type alias (`Arc<Mutex<Option<(Sender<NetworkEvent>, Receiver<NetworkCommand>)>>>`) changes to hold the `futures::channel::mpsc` halves. `ClientHandle::connect()` consumes the deferred pair and passes the network-side halves (`event_tx`, `cmd_rx`) to `spawn_network()`.

**Profile re-broadcast:** Currently tick-counted (fires at ticks 60, 120, 200, 400 of the 50ms poll, roughly 3s/6s/10s/20s). Replace with actual timers inside the event loop via `futures::select!`. On WASM, use `gloo_timers::future::TimeoutFuture`. On native, use `tokio::time::sleep` behind `#[cfg]`. The existing `profile_broadcast_counter` field is removed.

**`notification_tx` removal:** The existing `notification_tx: Option<Sender<ClientNotification>>` push mechanism is removed. The new `ClientEventLoop` -> `UnboundedSender<ClientEvent>` channel replaces it entirely. The `ClientNotification` enum is also removed. Its `EventApplied` and `StateChanged` variants were internal signaling that the event loop now handles implicitly (it processes events and emits `ClientEvent`s directly). No new `ClientEvent` variants are needed.

**New dependencies in `willow-web`:** Add `futures` to `Cargo.toml` for `StreamExt` (`.next()`, `.try_next()`). Move `gloo-timers` from `[dev-dependencies]` to `[dependencies]` for the typing indicator interval timer.

### 2. Client Split

Split the monolithic `Client` struct into three pieces.

**`SharedState`** contains all mutable state, wrapped in `Rc<RefCell<...>>`:

```rust
pub struct SharedState {
    pub state: ClientState,          // servers, event_state, chat, profiles, message_db
    pub identity: Identity,
    pub config: ClientConfig,
    pub connected: bool,
    pub connected_subscribed: bool,
    pub typing_peers: HashMap<String, (String, u64)>,
    pub voice_participants: HashMap<String, HashSet<String>>,
    pub active_voice_channel: Option<String>,
    pub voice_muted: bool,
    pub voice_deafened: bool,
    pub state_verification_results: HashMap<String, StateHash>,
    pub last_typing_sent_ms: u64,
}
```

**`Rc` vs `Arc`:** The CLAUDE.md convention says "Use `Arc` everywhere — all types must be `Send + Sync`." This applies to library crates that must work on both native and WASM. Since this refactor intentionally breaks native and targets WASM only, `SharedState` uses `Rc<RefCell<...>>` (no `Send + Sync` needed on single-threaded WASM). When native support is re-added later, this can be swapped to `Arc<Mutex<...>>` behind a cfg gate or via a type alias.

**`ClientHandle`** is a cloneable command interface for UI components:

- Holds `Rc<RefCell<SharedState>>` + `UnboundedSender<NetworkCommand>`.
- Exposes all mutation methods: `send_message()`, `create_channel()`, `switch_server()`, `join_voice()`, etc.
- Exposes all read accessors: `messages()`, `channels()`, `peer_id()`, `display_name()`, etc.
- Has `connect()` which spawns the network task. The deferred channel halves (`event_tx`, `cmd_rx` for the network side) are stored on `ClientHandle` (not `SharedState`) since they are consumed once during `connect()`.
- Cloneable. Components get their own copy via context.

**Optimistic updates:** `ClientHandle` mutation methods (like `send_message`, `create_channel`) update `SharedState` immediately (applying the event to event-sourced state, persisting) AND send the command/event to the network. This matches the current `Client` behavior where local state updates are synchronous. There is no latency gap between a user action and the UI reflecting it.

**Migration scope:** The existing `Client` has approximately 40+ methods spanning ~1500 lines. These methods are mechanically moved to `ClientHandle`, changing `self.state` references to `self.shared.borrow()` / `self.shared.borrow_mut()` and `self.cmd_tx` references (which remain the same). This is the bulk of the implementation work but is mostly mechanical.

**`ClientEventLoop`** is the exclusive event processor, not cloneable:

- Holds `Rc<RefCell<SharedState>>` + `UnboundedReceiver<NetworkEvent>`.
- `async fn run(self, tx: UnboundedSender<ClientEvent>)` is the main loop.
- Awaits network events, processes them (applies to event-sourced state, persists, dedup), sends `ClientEvent`s out on the tx channel. This includes all event types currently handled in `poll()`: `EventReceived`, `SyncBatchReceived`, `PeerConnected/Disconnected`, `ProfileReceived`, `FileAnnounced`, `TypingReceived`, `VoiceJoinReceived/LeaveReceived/SignalReceived`, etc.
- Handles profile re-broadcast via timer arms in `futures::select!`.
- **Batching at this layer:** When a network event arrives, drains any additional ready events from `event_rx` via `try_next()` before processing. A single `SyncBatchReceived` may emit multiple `ClientEvent`s. All are sent on `tx` individually.

**Error handling / shutdown:** When `event_rx` closes (network shut down), `run()` returns gracefully. When `tx` is closed (UI dropped the receiver), the event loop logs a warning and returns. No panics on channel closure.

**Constructor:**

```rust
pub fn new(config: ClientConfig) -> (ClientHandle, ClientEventLoop)
```

Both share the same `Rc<RefCell<SharedState>>`. The handle borrows briefly for sync operations; the event loop borrows briefly during processing then releases before the next await. No conflicts on single-threaded WASM.

The existing `Client` struct and `poll()` method are removed.

**Naming:** The web crate currently defines `pub type ClientHandle = SendWrapper<Rc<RefCell<Client>>>`. This type alias is removed. The new `willow_client::ClientHandle` replaces it. Since `ClientHandle` contains `Rc` (not `Send`), the web crate wraps it in `SendWrapper` for Leptos context: `pub type WebClientHandle = SendWrapper<willow_client::ClientHandle>`. Components that currently import `crate::app::ClientHandle` switch to `crate::app::WebClientHandle`.

### 3. UI State Context

Replace 30 loose signals threaded as props with a structured `AppState` provided via Leptos `provide_context`.

**`AppState`** is grouped into sub-structs (read-only halves):

```rust
pub struct AppState {
    pub chat: ChatState,
    pub network: NetworkState,
    pub server: ServerState,
    pub ui: UiState,
    pub voice: VoiceState,
}

pub struct ChatState {
    pub messages: ReadSignal<Vec<DisplayMessage>>,
    pub current_channel: ReadSignal<String>,
    pub channels: ReadSignal<Vec<String>>,
    pub replying_to: ReadSignal<Option<DisplayMessage>>,
    pub editing: ReadSignal<Option<DisplayMessage>>,
    pub pinned_messages: ReadSignal<Vec<DisplayMessage>>,
    pub pin_labels: ReadSignal<HashMap<String, String>>,
    /// Per-channel view state (typing indicators, future: drafts, scroll pos).
    pub channel_views: ReadSignal<HashMap<String, ChannelViewState>>,
}

/// Per-channel UI state. Extensible for future per-channel needs
/// (draft text, scroll position, etc.).
#[derive(Clone, Default, PartialEq)]
pub struct ChannelViewState {
    pub typing: Vec<String>,
}

pub struct NetworkState {
    pub peers: ReadSignal<Vec<(String, String, bool)>>,
    pub peer_count: ReadSignal<usize>,
    pub peer_id: ReadSignal<String>,
    pub connection_status: ReadSignal<String>,
    pub loading: ReadSignal<bool>,
}

pub struct ServerState {
    pub servers: ReadSignal<Vec<(String, String)>>,
    pub active_server_id: ReadSignal<String>,
    pub active_server_name: ReadSignal<String>,
    pub unread: ReadSignal<HashMap<String, usize>>,
    pub roles: ReadSignal<Vec<(String, String, Vec<String>)>>,
    pub display_name: ReadSignal<String>,
}

pub struct UiState {
    pub show_settings: ReadSignal<bool>,
    pub show_server_settings: ReadSignal<bool>,
    pub show_sidebar: ReadSignal<bool>,
    pub show_members: ReadSignal<bool>,
    pub show_add_server: ReadSignal<bool>,
    pub show_pinned: ReadSignal<bool>,
}

pub struct VoiceState {
    pub voice_channel: ReadSignal<Option<String>>,
    pub voice_muted: ReadSignal<bool>,
    pub voice_deafened: ReadSignal<bool>,
    pub voice_participants_map: ReadSignal<HashMap<String, Vec<String>>>,
    pub voice_channel_name: ReadSignal<String>,
}
```

**`AppWriteSignals`** is a companion struct holding the `WriteSignal` halves. Not provided as context. Held only by the event processing layer and handler closures that need to write:

```rust
pub struct AppWriteSignals {
    pub chat: ChatWriteSignals,
    pub network: NetworkWriteSignals,
    pub server: ServerWriteSignals,
    pub ui: UiWriteSignals,
    pub voice: VoiceWriteSignals,
}

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

pub struct NetworkWriteSignals {
    pub set_peers: WriteSignal<Vec<(String, String, bool)>>,
    pub set_peer_count: WriteSignal<usize>,
    pub set_peer_id: WriteSignal<String>,
    pub set_connection_status: WriteSignal<String>,
    pub set_loading: WriteSignal<bool>,
}

pub struct ServerWriteSignals {
    pub set_servers: WriteSignal<Vec<(String, String)>>,
    pub set_active_server_id: WriteSignal<String>,
    pub set_active_server_name: WriteSignal<String>,
    pub set_unread: WriteSignal<HashMap<String, usize>>,
    pub set_roles: WriteSignal<Vec<(String, String, Vec<String>)>>,
    pub set_display_name: WriteSignal<String>,
}

pub struct UiWriteSignals {
    pub set_show_settings: WriteSignal<bool>,
    pub set_show_server_settings: WriteSignal<bool>,
    pub set_show_sidebar: WriteSignal<bool>,
    pub set_show_members: WriteSignal<bool>,
    pub set_show_add_server: WriteSignal<bool>,
    pub set_show_pinned: WriteSignal<bool>,
}

pub struct VoiceWriteSignals {
    pub set_voice_channel: WriteSignal<Option<String>>,
    pub set_voice_muted: WriteSignal<bool>,
    pub set_voice_deafened: WriteSignal<bool>,
    pub set_voice_participants_map: WriteSignal<HashMap<String, Vec<String>>>,
    pub set_voice_channel_name: WriteSignal<String>,
}
```

Components access read state via `use_context::<AppState>()` and reach into sub-structs: `state.chat.messages`, `state.ui.show_sidebar`, etc.

Components that need to write get access through `Callback` props or by pulling `WebClientHandle` from context (for mutations that go through the client). UI-only toggles (like `set_show_sidebar`) are provided via `Callback` props from the parent that holds `AppWriteSignals`.

### 4. Event-Driven UI Updates

Replace the `set_interval(50ms)` poll loop with two `spawn_local` tasks.

**Task 1 — Event loop (runs in client crate logic):**

```rust
spawn_local(event_loop.run(client_event_tx));
```

Awaits `NetworkEvent`s, processes them, sends `ClientEvent`s on the channel. Batching at this layer: drains ready `NetworkEvent`s, processes them, emits individual `ClientEvent`s.

**Task 2 — Signal updater (in web crate):**

```rust
spawn_local(async move {
    while let Some(event) = client_event_rx.next().await {
        let mut batch = vec![event];
        while let Ok(Some(more)) = client_event_rx.try_next() {
            batch.push(more);
        }
        process_event_batch(&batch, &handle, &write, &voice_manager);
    }
});
```

Batching at this layer: drains ready `ClientEvent`s so that signal updates happen once per batch, not once per event. The two batching layers are intentionally independent. The event loop batches `NetworkEvent` processing (one lock cycle). The signal updater batches `ClientEvent` consumption (one render cycle). A single `SyncBatchReceived` network event may produce many `ClientEvent`s that the signal updater collects into one batch.

The signal update logic is extracted into a standalone function:

```rust
fn process_event_batch(
    events: &[ClientEvent],
    handle: &WebClientHandle,
    write: &AppWriteSignals,
    voice_manager: &VoiceManagerHandle,
)
```

This uses the same flag-based approach the current poll loop does (`needs_msg_refresh`, `needs_peer_refresh`, `needs_channel_refresh`) but as a named function instead of a 170-line inline closure.

**Voice events inside `process_event_batch`:** Voice events (`VoiceJoined`, `VoiceSignal`) require async WebRTC operations (creating offers, handling answers). The batch processor calls `wasm_bindgen_futures::spawn_local` for these, same as the current poll loop does. The batch processor is called from within a `spawn_local` context so this is valid.

**Typing indicator refresh:** Currently checked every 50ms as a flat list for the active channel. Replace with per-channel tracking via the `ChannelViewState` map. A separate `spawn_local` runs a `gloo_timers::future::IntervalStream` at ~2s to expire stale typing entries and update the `channel_views` signal. Components look up `channel_views.get().get(&current_channel)` to display typing for the active channel. Typing state only needs coarse-grained updates.

### 5. App Component Breakup

The 800-line `App` function splits into focused modules.

**`app.rs` — App component (~50 lines):**

- Creates `ClientHandle` + `ClientEventLoop` via `willow_client::new()`.
- Creates signal pairs, builds `AppState` + `AppWriteSignals`.
- Calls `provide_context` for `AppState`, `WebClientHandle`, `VoiceManagerHandle`.
- Spawns the async tasks (event loop, signal updater, typing timer).
- Renders the top-level layout shell.

**`event_processing.rs` (~100 lines):**

- `fn process_event_batch(...)` — the flag-based batch processor.
- `fn refresh_all_signals(...)` — full refresh used after server create/join/switch.
- `fn extract_roles(...)` — role extraction helper.

**`handlers.rs` (~80 lines):**

Handler constructors that return closures:

- `fn make_send_handler(...)` — send message / reply.
- `fn make_edit_handler(...)` — edit message.
- `fn make_delete_handler(...)` — delete message.
- `fn make_react_handler(...)` — add/toggle reaction.
- `fn make_channel_click_handler(...)` — switch channel.
- `fn make_server_click_handler(...)` — switch server.
- `fn make_pin_handler(...)` — pin/unpin message.

Components receive these as 1-2 `Callback` props instead of many signal props.

**Voice handling** stays in `voice.rs`. Voice event matching (offer/answer/ICE) moves from the inline poll loop to `process_event_batch`, which calls into `voice.rs` helpers via `spawn_local`.

**File structure after refactor:**

```
crates/web/src/
├── app.rs              — App component (setup + layout)
├── event_processing.rs — batch processing + refresh logic
├── handlers.rs         — action handler constructors
├── voice.rs            — VoiceManager (existing, gains event helpers)
├── main.rs             — entry point (unchanged)
├── util.rs             — existing utilities
└── components/         — existing components, props simplified
```

## Testing

- **Browser tests** (`just test-browser`, 39 tests) validate component rendering and do not use `Client` directly. These must continue to pass with the new context-based component props.
- **Client library tests** (`just test-client`, 93 tests) create `Client` via `test_client()`. This helper is replaced with `test_handle()` which creates a `ClientHandle` + `SharedState` without a network connection (no `connect()` call, commands go to a no-op channel). The `test_handle()` function returns `(ClientHandle, UnboundedReceiver<NetworkCommand>)` so tests can assert on commands sent. Test assertions change from `client.method()` to `handle.method()` — largely mechanical.
- **WASM compilation check** (`just check-wasm`) must pass.
