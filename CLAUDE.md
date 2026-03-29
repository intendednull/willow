# CLAUDE.md — Willow Development Guide

## Project Overview

Willow is a P2P Discord replacement built in Rust. It uses libp2p for
networking, Bevy for the desktop UI, and Ed25519 cryptography for identity.

## Repository Structure

```
crates/
├── state/       — Pure event-sourced state machine, zero I/O (willow-state)
├── client/      — UI-agnostic client library wrapping state + networking (willow-client)
├── transport/   — Binary serialization & protocol framing (willow-transport)
├── identity/    — Ed25519 identity, message signing, profiles (willow-identity)
├── messaging/   — Chat messages, HLC ordering, message store (willow-messaging)
├── crypto/      — E2E encryption: ChaCha20-Poly1305, X25519 key exchange (willow-crypto)
├── channel/     — Servers, channels, roles, permissions (willow-channel)
├── files/       — Content-addressed file chunking and reassembly (willow-files)
├── network/     — libp2p P2P networking layer (willow-network)
├── relay/       — Relay server for bridging TCP and WebSocket peers (willow-relay)
├── web/         — Leptos web UI application (willow-web)
└── app/         — Bevy desktop UI application (willow-app)
    └── src/
        ├── main.rs          — App entry point
        ├── lib.rs           — Module exports
        ├── base64.rs        — Shared base64 encode/decode
        ├── clipboard.rs     — Cross-platform clipboard (arboard / web_sys)
        ├── emoji.rs         — Shortcode registry and expansion
        ├── file_manager.rs  — File sharing and chunk management
        ├── invite.rs        — Secure per-recipient invite codes
        ├── network_bridge.rs — Async/sync bridge (tokio ↔ Bevy)
        ├── server_sync.rs   — Server state sync: StampedOp, SyncMessage, pack/unpack
        ├── storage.rs       — Cross-platform persistence (filesystem / localStorage)
        ├── theme.rs         — Discord-style dark color palette
        ├── tests.rs         — Headless UI tests
        └── ui/
            ├── mod.rs        — Plugin registration, helpers
            ├── resources.rs  — ECS resources (ChatState, etc.)
            ├── components.rs — Marker components
            ├── constants.rs  — Shared placeholder strings
            ├── layout.rs     — Entity spawning
            ├── init.rs       — Server init, channel subscription
            ├── input.rs      — Keyboard handling, message sending
            ├── chat.rs       — Network events, message rendering
            ├── channels.rs   — Channel/role/member management, invites, trust
            ├── settings.rs   — Settings view systems
            └── files.rs      — File picker systems

docs/superpowers/
├── specs/   — Design specs for new features and architecture changes
└── plans/   — Implementation plans referencing the specs
```

## Build & Test

```bash
just check          # run ALL checks (fmt, clippy, test, wasm) — use before committing
just fmt            # format code
just clippy         # lint with warnings as errors (workspace-wide)
just test           # run all cargo tests (unit + integration)
just test-browser   # run in-browser Leptos tests (needs Firefox + geckodriver)
just test-all       # run ALL tests including browser
just test-state     # test the pure state machine
just test-client    # test the client library
just test-app       # test the Bevy app (headless + integration)
just test-relay     # test relay history sync
just test-scale     # run scaling/performance tests with output
just test-crate X   # test a specific crate
just check-wasm     # verify WASM compilation
just build          # build native desktop app
just build-web      # build Leptos web app (crates/web via trunk)
just serve-web      # serve Leptos web app locally
just build-relay    # build relay server (release)
just run            # run native desktop app
just relay          # run the relay server
just dev            # start full local dev stack (relay + workers + web)
just dev-quick      # same as dev, but skip cargo build
just dev-clean      # remove .dev/ (keys, logs, storage DB)
```

**All code must pass `just check` (fmt + clippy + test + WASM) with zero
warnings before being committed.** Browser tests (`just test-browser`)
require Firefox and geckodriver installed.

### Local Development Stack

Run `just dev` to start all services locally:

| Service | Address | Description |
|---------|---------|-------------|
| Relay | `localhost:9090` (TCP), `localhost:9091` (WS) | Bridges peers |
| Replay node | connects via relay | In-memory state sync (max 1000 events/server) |
| Storage node | connects via relay | Archival SQLite storage |
| Web UI | `http://localhost:8080` | Leptos app via `trunk serve` |

All service logs are color-coded and interleaved in the terminal. Press
`Ctrl+C` to stop everything. Identity keys and data persist in `.dev/`
across restarts so peer IDs stay stable. Use `just dev-clean` to reset.

After the first run, use `just dev-quick` to skip the build step and
start services immediately.

### Dual-Target Support (Native + WASM)

All library crates must compile for both native and `wasm32-unknown-unknown`.
When adding new code, ensure WASM compatibility:

- **No `std::fs`** in library crates — gate with `#[cfg(not(target_arch = "wasm32"))]`
- **No `std::time::SystemTime`** — use `js_sys::Date::now()` on WASM
- **No `std::thread`** or **tokio** in library crates — these are native-only
- **RNG**: `getrandom` needs the `js` (v0.2) / `wasm_js` (v0.3) features on WASM
- **UUID**: workspace dep includes the `js` feature for WASM v4 generation
- **Network**: mDNS and TCP are native-only; WASM uses WebSocket via relay
- Use `#[cfg(target_arch = "wasm32")]` / `#[cfg(not(target_arch = "wasm32"))]`
  for platform-specific code paths

### Testing Strategy (420+ tests)

Willow uses a multi-tier testing strategy:

**1. Pure state machine tests** (`just test-state`, 64 tests):
- Determinism, idempotency, permission enforcement
- Event replay from genesis, merge convergence
- Stress: 1000 messages, 100-event replay, 3-way merge
- No I/O, no networking — tests run instantly

**2. Client library tests** (`just test-client`, 93 tests):
- Client API methods (send, create channel, trust, kick)
- Event store persistence, bridge conversion
- State accessors, display name resolution

**3. Bevy headless UI tests** (`just test-app`, 99 tests):
- Keyboard input, message sending, chat state
- Settings, profiles, invites, permissions
- Uses `test_app()` with `MinimalPlugins` — no window/GPU

**4. Network integration tests** (`just test-app`, 14 tests):
- Real libp2p nodes on localhost TCP
- Message round-trips, channel isolation, encryption
- Server op sync, file chunks, 3-node propagation

**5. Scaling tests** (`just test-scale`, 7 tests):
- 5/10/20 peer connections (~150ms/peer)
- Message flood delivery (100% at 10 peers)
- Event throughput (532k events/sec)
- Merge throughput (1000 events in 2.6ms)

**6. Relay history tests** (`just test-relay`, 3 tests):
- Relay stores events, serves to new peers
- Multi-peer history aggregation
- Offline peer recovery via relay

**7. In-browser Leptos tests** (`just test-browser`, 39 tests):
- Real DOM rendering in headless Firefox via wasm-pack
- Signal reactivity, event handling, Effects
- All components: sidebar, messages, input, channels,
  settings, member list, server list, connection status
- Requires: Firefox + geckodriver + wasm-pack

### Which Test to Write

**When adding a feature or fixing a bug, always add a test at the lowest
level that covers the behavior.** Prefer state tests over client tests,
client tests over browser tests, browser tests over Playwright E2E. Use
E2E tests only for behavior that requires real P2P sync or browser
interaction.

| What changed | Test type | Location | Command |
|---|---|---|---|
| State logic (events, permissions, merge) | State tests | `crates/state/src/tests.rs` | `just test-state` |
| Client API (send, create, trust, kick) | Client tests | `crates/client/src/lib.rs` test module | `just test-client` |
| UI components (rendering, signals, effects) | Browser tests | `crates/web/tests/browser.rs` | `just test-browser` |
| Multi-peer behavior (sync, messaging) | Playwright E2E | `e2e/multi-peer-sync.spec.ts` | `just test-e2e-sync` |
| Permissions (trust, kick, roles) | Playwright E2E | `e2e/permissions.spec.ts` | `just test-e2e-perms` |
| Mobile UI (touch, sidebar, action sheet) | Playwright E2E | `e2e/mobile.spec.ts` or `e2e/multi-peer-mobile.spec.ts` | `just test-e2e-ui` |
| Network protocol (libp2p, relay) | Network integration | `crates/app/tests/e2e_flow.rs` | `just test-app` |

### Adding Tests

**State machine test** (fastest):
1. Add to `crates/state/src/tests.rs`
2. Use `make_event(state, author, kind)` helper
3. `cargo test -p willow-state`

**Client API test**:
1. Add to `crates/client/src/lib.rs` test module
2. Use `test_client()` helper — creates ClientHandle without networking
3. `cargo test -p willow-client`

**Bevy headless test**:
1. Add to `crates/app/src/tests.rs`
2. Use `test_app()` for headless App + command receiver
3. `cargo test -p willow-app --lib`

**Browser test**:
1. Add to `crates/web/tests/browser.rs`
2. Use `mount_test(|| view! { ... })` to render into DOM
3. Use `tick().await` to flush reactive effects
4. `wasm-pack test --headless --firefox crates/web`

**Playwright E2E test**:
1. Add to the appropriate `e2e/*.spec.ts` file
2. Use helpers from `e2e/helpers.ts` (`setupTwoPeers`, `sendMessage`, etc.)
3. For multi-peer: use the `browser` fixture, not hardcoded `chromium.launch()`
4. `npx playwright test e2e/your-file.spec.ts`

## Code Conventions

- **Crate naming**: `willow-<name>` in Cargo.toml, `willow_<name>` in code
- **Thread safety**: Use `Arc` (not `Rc`) everywhere — all types must be `Send + Sync`
- **Error handling**: `thiserror` for library error types, `anyhow` for application code
- **Documentation**: Every public type and function has a doc comment. Module-level `//!` docs explain the purpose and provide examples.
- **Testing**: Every crate has unit tests. Use `#[cfg(test)] mod tests` at the bottom of each file.
- **Serialization**: All wire types derive `Serialize + Deserialize`. Round-trip tests validate compatibility.

## Architecture Notes

### Dependency Graph

```
willow-app → willow-crypto   → willow-identity → willow-transport
           → willow-network  → willow-identity
                             → willow-files
           → willow-channel  → willow-crypto
                             → willow-identity
           → willow-messaging → willow-identity
                              (defines SealedContent used by willow-crypto)
           → willow-files
```

### Async / Sync Boundary

- **Network layer**: Fully async (tokio on native, wasm-bindgen-futures on WASM).
  Runs on a background thread (native) or via spawn_local (WASM).
- **Bevy app**: Synchronous ECS. Communicates with the network via `std::sync::mpsc` channels.
- **Bridge**: `network_bridge.rs` converts between the two worlds.
- **Deferred startup**: Network doesn't start until `ConnectCommand` is sent,
  allowing the UI to configure relay addresses first.

### Message Flow

1. User types in Bevy UI → `Message::text()` creates cleartext message
2. If channel key exists → `seal_content()` encrypts Content → `Content::Encrypted`
3. `pack_envelope()` serializes → `identity::pack()` signs with Ed25519
4. `NetworkBridgeCommand::Publish` → bridge sends to network task
5. libp2p GossipSub floods to subscribed peers
6. Remote peer receives → `NetworkEvent::Message`
7. Bridge forwards to Bevy → `NetworkBridgeEvent::MessageReceived`
8. `identity::unpack()` verifies signature → `unpack_envelope()`
9. If `Content::Encrypted`, `open_content()` decrypts
10. Emoji shortcodes expanded → message rendered in UI

### Server State Sync

Server mutations (channels, roles, permissions, kicks) are synchronized
via `StampedOp` over gossipsub topic `_willow_server_ops`.

- **StampedOp**: wraps a `ServerOp` with UUID (dedup), HLC timestamp
  (ordering), and author PeerId (verified against Ed25519 signature).
- **OpLog**: Bevy resource tracking all ops, seen IDs, and trusted peers.
  Persisted to disk via `storage::save_op_log`.
- **Trust model**: server owner is implicitly trusted. Other peers must
  be explicitly trusted via `TrustPeer` ops. Only ops from trusted peers
  are applied; untrusted ops are rejected but their IDs are recorded.
- **SyncMessage**: wire-level enum wrapping `Op(StampedOp)`,
  `SyncRequest { latest_hlc }`, and `SyncBatch { ops }` for catch-up.
- **Catch-up**: on connect, peers broadcast `SyncRequest` with their
  latest HLC. Trusted peers respond with `SyncBatch` of newer ops.
  Ops are sorted by HLC and deduplicated before applying.
- **Idempotent ops**: `SetPermission { granted: bool }` instead of
  toggle. `CreateChannel` and `CreateRole` carry deterministic IDs.
- **Key rotation on kick**: `KickMember` op carries encrypted rotated
  channel keys per remaining member (X25519 DH + ChaCha20-Poly1305).

### Event-Sourced State (willow-state)

All shared state is derived from an ordered sequence of deterministic
events. The `willow-state` crate is pure — zero I/O, zero networking.

- **Event**: carries unique ID, parent state hash, author PeerId,
  timestamp hint, and an `EventKind` mutation variant.
- **EventKind**: 17 variants covering server structure, roles,
  fine-grained permissions, chat, identity, and encryption.
- **ServerState**: complete shared state derivable from event replay.
  Computes a `StateHash` (SHA-256) for divergence detection.
- **apply()**: the ONLY way to mutate state. Pure function.
  Enforces permissions, dedup, and parent hash verification.
- **Permission model**: Owner is root of trust. Fine-grained permissions
  (SyncProvider, ManageChannels, ManageRoles, KickMembers, SendMessages,
  CreateInvite, Administrator) granted via GrantPermission events.
- **merge()**: resolves divergent histories by finding common ancestor,
  sorting divergent events by timestamp, and replaying.
- **EventStore trait**: append-only event log abstraction.

### Trust Model

- Owner has implicit all-permissions (root of trust chain).
- Permissions are granted via `GrantPermission` events from owner/admin.
- Invite trust lists are *suggestions* — joining peers verify state from
  multiple sources and use majority-agreed state.
- The relay is a regular client — trusted only if explicitly granted
  SyncProvider permission by the owner.
- State verification: get state hash from multiple peers, use the hash
  agreed upon by the most trusted sources.

### Hybrid Logical Clocks (HLC)

Messages are ordered using HLCs (`willow-messaging/src/hlc.rs`). Every node
maintains an `HLC` instance. Call `hlc.now()` for local events and
`hlc.receive(remote_ts)` when processing remote messages. This ensures
consistent ordering even when system clocks drift.

## Common Tasks

### Adding a new message type

1. Add a variant to `Content` in `crates/messaging/src/lib.rs`
2. Add a constructor method on `Message`
3. Add tests
4. Handle the new variant in the app's network event handler (`ui/chat.rs`)

### Adding a new permission

1. Add a variant to `Permission` in `crates/channel/src/lib.rs`
2. Check it in the relevant server methods
3. Add tests

### Adding a new libp2p protocol

1. Add the protocol to `WillowBehaviour` in `crates/network/src/behaviour.rs`
2. Handle its events in `run_swarm()` in `crates/network/src/node.rs`
3. Expose relevant commands/events through `NetworkNode` and `NetworkEvent`

### Adding a new UI system

1. Add the system function to the appropriate `ui/` module
2. Register it in `ui/mod.rs` under the correct `add_systems()` group
3. If it needs new resources, add to `ui/resources.rs` and register in `build()`
4. If it needs new components, add to `ui/components.rs`
5. Add a test in `tests.rs` using `test_app()`

### Adding a new server operation

1. Add a variant to `ServerOp` in `crates/app/src/server_sync.rs`
2. Handle it in `handle_server_op()` in `crates/app/src/ui/chat.rs`
3. Broadcast it from the appropriate UI handler in `crates/app/src/ui/channels.rs`
   using `broadcast_op()` to stamp, record, persist, and send
4. Add tests for dedup, trust rejection, and application

### Adding a custom emoji

1. Call `EmojiRegistry::add("shortcode", "replacement")` on the `EmojiRegistryRes`
2. Users type `:shortcode:` in messages — expanded during rendering
3. Built-in emoji: edit `emoji.rs` `builtin()` function
