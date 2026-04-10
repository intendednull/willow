# CLAUDE.md — Willow Development Guide

## Project Overview

Willow is a P2P Discord replacement built in Rust. It uses iroh for
networking, Leptos for the web UI, and Ed25519 cryptography for identity.

## Repository Structure

```
docs/
├── plans/       — Implementation plans for features
└── specs/       — Design specs and technical specifications
crates/
├── state/       — Pure event-sourced state machine, zero I/O (willow-state)
├── client/      — UI-agnostic client library wrapping state + networking (willow-client)
├── transport/   — Binary serialization & protocol framing (willow-transport)
├── identity/    — Ed25519 identity, message signing, profiles (willow-identity)
├── messaging/   — Chat messages, HLC ordering, message store (willow-messaging)
├── crypto/      — E2E encryption: ChaCha20-Poly1305, X25519 key exchange (willow-crypto)
├── channel/     — Servers, channels, roles, permissions (willow-channel)
├── network/     — iroh-based P2P networking (willow-network)
│   └── src/
│       ├── lib.rs      — Module exports, re-exports
│       ├── traits.rs   — Network, TopicHandle, TopicEvents, BlobStore traits
│       ├── iroh.rs     — IrohNetwork production implementation
│       ├── mem.rs      — MemNetwork test double (test-utils feature)
│       └── topics.rs   — TopicId registry (blake3 hashing)
├── relay/       — Relay server for bridging TCP and WebSocket peers (willow-relay)
└── web/         — Leptos web UI application (willow-web)

Note: the Bevy desktop app (`crates/app`) has been removed. It may be
re-added later but is not part of the current workspace.

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
just test-relay     # test relay history sync
just test-crate X   # test a specific crate
just check-wasm     # verify WASM compilation
just build-web      # build Leptos web app (crates/web via trunk)
just serve-web      # serve Leptos web app locally
just build-relay    # build relay server (release)
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
- **Network**: iroh handles WASM transport differences internally, so most
  `#[cfg(target_arch = "wasm32")]` gates for networking are no longer needed
- Use `#[cfg(target_arch = "wasm32")]` / `#[cfg(not(target_arch = "wasm32"))]`
  for platform-specific code paths (storage backends, timers, etc.)

### Testing Strategy

Willow uses a multi-tier testing strategy:

**1. Pure state machine tests** (`just test-state`):
- Determinism, idempotency, permission enforcement
- Event replay from genesis, merge convergence
- Stress: 1000 messages, 100-event replay, 3-way merge
- No I/O, no networking — tests run instantly

**2. Client library tests** (`just test-client`):
- Client API methods (send, create channel, trust, kick)
- Event store persistence, bridge conversion
- State accessors, display name resolution

**3. Relay history tests** (`just test-relay`):
- Relay stores events, serves to new peers
- Multi-peer history aggregation
- Offline peer recovery via relay

**4. In-browser Leptos tests** (`just test-browser`):
- Real DOM rendering in headless Firefox via wasm-pack
- Signal reactivity, event handling, Effects
- All components: sidebar, messages, input, channels,
  settings, member list, server list, connection status
- Requires: Firefox + geckodriver + wasm-pack

**5. Playwright E2E tests** (`just test-e2e-ui`, `just test-e2e-sync`):
- Multi-peer sync, permissions, mobile UI
- Real browser interaction against the Leptos web app

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

### Adding Tests

**State machine test** (fastest):
1. Add to `crates/state/src/tests.rs`
2. Use `make_event(state, author, kind)` helper
3. `cargo test -p willow-state`

**Client API test**:
1. Add to `crates/client/src/lib.rs` test module
2. Use `test_client()` helper — creates ClientHandle without networking
3. `cargo test -p willow-client`

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
- **Specs & Plans**: Design specs go in `docs/specs/`, implementation plans go in `docs/plans/`. Name files with date prefix: `YYYY-MM-DD-<feature-name>.md`.

## Architecture Notes

### Dependency Graph

```
willow-web → willow-client  → willow-state
                            → willow-network (iroh, iroh-gossip, iroh-blobs)
           → willow-crypto  → willow-identity → willow-transport
           → willow-channel → willow-crypto
                            → willow-identity
           → willow-messaging → willow-identity
                              (defines SealedContent used by willow-crypto)
```

### Async Model

- **Network layer**: Fully async using iroh's QUIC transport with gossip protocol.
  Runs on a background thread (native) or via spawn_local (WASM).
- **Client library**: Async API. Consumers drive it from their own runtime
  (tokio on native, wasm-bindgen futures in the browser).
- **Deferred startup**: Network doesn't start until the client explicitly
  connects, allowing the UI to configure relay addresses first.

### Message Flow

1. User types in the UI → `Message::text()` creates cleartext message
2. If channel key exists → `seal_content()` encrypts Content → `Content::Encrypted`
3. `pack_wire()` signs with Ed25519 → `TopicHandle::broadcast()` sends to gossip
4. iroh gossip delivers to subscribed peers
5. Listener task receives `GossipEvent::Received` → `unpack_wire()` verifies
6. Client forwards to the UI via its event stream
7. `identity::unpack()` verifies signature → `unpack_envelope()`
8. If `Content::Encrypted`, `open_content()` decrypts
9. Message rendered in the UI

### Event-Based Server State Sync

Server mutations (channels, roles, permissions, kicks) are synchronized
via the event-sourced `willow-state` machine over iroh gossip. Events
are broadcast and received through the `Network` trait and applied
deterministically via `apply()`.

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
4. Handle the new variant in the web UI's message rendering

### Adding a new permission

1. Add a variant to `Permission` in `crates/channel/src/lib.rs`
2. Check it in the relevant server methods
3. Add tests

### Adding a new iroh protocol

1. Define the protocol in `crates/network/src/traits.rs` if needed
2. Implement it in `crates/network/src/iroh.rs` using iroh's ALPN routing
3. Add a test double in `crates/network/src/mem.rs`
4. Use the trait in client/worker code

### Adding a new EventKind

1. Add a variant to `EventKind` in `crates/state/src/lib.rs`
2. Handle it in `apply()` with the appropriate permission checks
3. Expose a method on `Client` in `crates/client/src/lib.rs` to emit it
4. Add state-machine tests for dedup, permission rejection, and application
