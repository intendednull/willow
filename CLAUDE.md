# CLAUDE.md — Willow Development Guide

## Project Overview

Willow is a P2P Discord replacement built in Rust. It uses iroh for
networking, Leptos for the web UI, and Ed25519 cryptography for identity.

## Dev Guidelines

Quality and longevity beat speed and convenience.

- **Choose the right solution, not the easy one.** Ask: which approach makes most
  sense long-term, causes least future confusion, lasts? Pick that one.
- **No hacky workarounds, no shortcuts.** If the obvious fix is a band-aid,
  keep digging until you find the real fix.
- **Root-cause every bug.** Don't patch symptoms. Don't disable failing tests.
  Don't swallow errors. Find why, fix why.
- **Scope creep is acceptable when warranted, not when speculative.** If doing it
  right means touching more files or refactoring an abstraction — do it. But
  don't add features, abstractions, or error handling the task didn't ask for.
- **When the answer isn't obvious, stop and design.** Two+ reasonable approaches?
  Write a brief note in `docs/specs/YYYY-MM-DD-<name>-design.md` before coding.
  Implementation plan goes in `docs/plans/YYYY-MM-DD-<name>.md`. Cheap up front,
  expensive later. **Specs go in `docs/specs/`, plans in `docs/plans/`.** Do
  not create `docs/superpowers/`.
- **Surface tradeoffs explicitly.** When picking between approaches, name the
  runner-up and why rejected. Commit body or PR description. Future-you needs
  the reasoning, not just the result.
- **Mechanical rigor: `just check` before commit.** Fmt, clippy, tests, WASM.
  Zero warnings.
- **Semantic rigor: verify before claiming done.** Run the actual test, hit the
  actual UI, read the actual output. No "should work" assertions. See
  `superpowers:verification-before-completion`.
- **Process skills before implementation skills.** Brainstorming and debugging
  determine *how* to approach. Don't skip them to feel productive.
- **Tests at the lowest tier that covers the behavior.** State > client >
  browser > Playwright. See `## Which test tier to use`.

## Repository Structure

```
docs/
├── plans/              — Implementation plans for features (YYYY-MM-DD-<name>.md)
├── specs/              — Design specs and technical specifications (YYYY-MM-DD-<name>-design.md)
├── reference-designs/  — Exploratory UI / design references
└── reports/            — Ad-hoc audit and investigation reports
crates/
├── state/       — Pure event-sourced state machine, zero I/O (willow-state)
├── client/      — UI-agnostic client library wrapping state + networking (willow-client)
├── common/      — Shared wire-protocol types used by client + workers (willow-common)
├── transport/   — Binary serialization & protocol framing (willow-transport)
├── identity/    — Ed25519 identity, message signing, profiles (willow-identity)
├── messaging/   — Chat messages, HLC ordering, message store (willow-messaging)
├── crypto/      — E2E encryption: ChaCha20-Poly1305, X25519 key exchange (willow-crypto)
├── network/     — iroh-based P2P networking (willow-network)
│   └── src/
│       ├── lib.rs      — Module exports, re-exports
│       ├── traits.rs   — Network, TopicHandle, TopicEvents, BlobStore traits
│       ├── iroh.rs     — IrohNetwork production implementation
│       ├── mem.rs      — MemNetwork test double (test-utils feature)
│       └── topics.rs   — TopicId registry (blake3 hashing)
├── actor/       — Lightweight dual-target actor framework (willow-actor)
├── worker/      — Shared WorkerRole trait + actor runtime (willow-worker)
├── replay/      — Bounded-memory state-sync worker binary (willow-replay)
├── storage/     — Archival SQLite-backed history worker binary (willow-storage)
├── relay/       — Relay server for bridging TCP and WebSocket peers (willow-relay)
├── agent/       — MCP server exposing ClientHandle to AI agents (willow-agent)
└── web/         — Leptos web UI application (willow-web)
```

## State Management

All shared mutable state in library crates lives inside an actor (see
`crates/actor/`). Default to `StateActor<S>` for new state. Decision tree:

| Situation | Pattern |
|---|---|
| Shared mutable state in a lib crate | `StateActor<S>` or bespoke actor (default) |
| External-callback boundary (iroh) | Lock + `// state: lock-ok — <reason>` |
| Sync trait abstraction over small cache (legacy) | Single `Mutex<Inner>` + `// state: lock-ok` (trait elimination is follow-up work) |
| Pre-existing lock with actor migration deferred | Single `Mutex<_>`/`RwLock<_>` + `// state: lock-ok` citing the spec follow-up entry |
| One-shot static init | `OnceLock<T>` / `LazyLock<T>` |
| Cross-task control flag (stop, cancel) | `AtomicBool` / `AtomicU32` |
| WASM single-threaded interior mutability | `Rc<RefCell<T>>` (web only) |
| Reactive UI state in web | Leptos signal (`RwSignal`, `Resource`) |
| Web state mutated from non-Leptos context | `StateActor<S>` |
| Actor coordination signal (ready, cancel, one-shot) | `tokio::sync::watch` / `oneshot` / `broadcast` / `Notify` (never `tokio::sync::Mutex` for business state) |

**No `Arc<Mutex<T>>` / `Arc<RwLock<T>>` / `parking_lot::*` for business state.**
New locks need a `// state: lock-ok — <reason>` comment with rationale at the
use site. `MemNetwork` (`crates/network/src/mem.rs`) is test infrastructure and
exempt; the iroh layer (`crates/network/src/iroh.rs`) is the only production
exception, justified by iroh's external-callback delivery model.

Full discussion + audit trail: `docs/specs/2026-04-26-state-management-model-design.md`.

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
just test-workers   # test worker + replay + storage + common
just test-agent     # test agent library (MCP server)
just test-agent-e2e # multi-peer E2E via agent harness
just test-crate X   # test a specific crate
just check-wasm     # verify WASM compilation
just check-all      # fmt + clippy + test + wasm-pack browser + Playwright (PR gate)
just build-web      # build Leptos web app (crates/web via trunk)
just serve-web      # serve Leptos web app locally
just build-relay    # build relay server (release)
just build-workers  # build replay + storage binaries (release)
just build-agent    # build agent binary
just agent -- ARGS  # run agent MCP server
just relay          # run the relay server
just dev            # start full local dev stack (relay + workers + web)
just dev-quick      # same as dev, but skip cargo build
just dev-clean      # remove .dev/ (keys, logs, storage DB)
just docker-up      # start full stack via docker compose
just docker-down    # stop docker stack
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

If `vibe-annotations-server` is installed, `just dev` starts it automatically
at `http://127.0.0.1:3846` alongside the other services.

## Agent Tooling

Two tools are configured for agents working on this project:

### agent-browser

CLI browser automation tool for driving the Willow web UI directly. Installed
as a skill at `.agents/skills/agent-browser` (symlinked into `.claude/skills/`).

**One-time setup (human):**
```bash
npm install -g agent-browser
agent-browser install          # downloads Chromium
npx skills add vercel-labs/agent-browser --yes
```

**Usage:**
```bash
agent-browser open http://localhost:8080   # navigate to a URL
agent-browser snapshot                     # get interactive element tree with refs
agent-browser click e3                     # click element by ref
agent-browser fill e4 "some text"          # fill a text input
agent-browser key "Enter"                  # press a key
agent-browser screenshot shot.png          # capture screenshot
agent-browser close                        # close the browser
```

Use agent-browser to verify UI features after changes, test flows end-to-end,
or investigate bugs by navigating the live app. Always run `just dev` first.

### Vibe Annotations

Browser extension + local MCP server for visual UI feedback. Lets you click
elements in Chrome and leave annotated feedback that agents read via MCP.

The MCP server is pre-configured in `.mcp.json` at `http://127.0.0.1:3846/mcp`.

**One-time setup (human):**
1. Install the [Chrome extension](https://chromewebstore.google.com/detail/vibe-annotations-visual-f/gkofobaeeepjopdpahbicefmljcmpeof)
2. `npm install -g vibe-annotations-server`

**Usage:**
- `just dev` starts the server automatically if installed
- Open `http://localhost:8080` in Chrome, click the extension to annotate elements
- Agent reads pending annotations via the `read_annotations` MCP tool
- Agent deletes annotations after implementing fixes via `delete_annotation`

**MCP tools available** (requires restarting Claude Code after first setup):
- `read_annotations` — fetch all pending annotations
- `watch_annotations` — poll for new annotations
- `delete_annotation` — mark an annotation as resolved
- `get_project_context` — infer tech stack from URL

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

## Which test tier to use

Decision tree for every new test:

1. **State-machine logic only?** (event application, permissions, merge, dedup, HLC) → Rust state crate test (`crates/state/src/tests.rs`).
2. **Client API + derivation, no DOM?** (mutations, view signals, ClientHandle methods) → Rust client crate test (`crates/client/src/tests/`).
3. **Multi-peer sync semantics?** → Rust client crate test with `willow_network::mem::MemNetwork` (unless validating real iroh/QUIC behaviour specifically).
4. **DOM rendering or event dispatch?**
   - Single client + single viewport → wasm-pack browser test (`crates/web/tests/browser.rs`). Use `mount_test_with_shell(TestShell::Desktop | Mobile)` for viewport-specific flows.
   - Multi-client or multi-viewport → Playwright (`e2e/*.spec.ts`).
5. **Cross-browser quirk coverage (Firefox vs Chrome behaviour)?** → Playwright.
6. **Touch / gesture / mobile-shell media query behaviour?** → Playwright mobile-chrome.
7. **Service worker, push, or navigator APIs?** → Playwright.

**Default to the lowest tier that can cover the behaviour.**

**Rewrite trigger.** When a Playwright test fails because a selector or helper drifts — not because behaviour broke — that test is at the wrong tier. Migrate it down on the same commit.

Full discussion: `docs/specs/2026-04-21-e2e-test-architecture-design.md`.

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
- **Specs & Plans**: Design specs go in `docs/specs/` named `YYYY-MM-DD-<feature-name>-design.md`. Implementation plans go in `docs/plans/` named `YYYY-MM-DD-<feature-name>.md`.

## Architecture Notes

### Authority Model

See [`docs/specs/2026-04-12-state-authority-and-mutations.md`](docs/specs/2026-04-12-state-authority-and-mutations.md).
All authority checks live in `willow-state::materialize::apply_event`
and the `required_permission()` table. Permissions are checked before
an event is created — rejected events never enter the DAG.

### Dependency Graph

```
willow-web → willow-client  → willow-state
                            → willow-network (iroh, iroh-gossip, iroh-blobs)
           → willow-crypto  → willow-messaging → willow-identity → willow-transport
                              (re-exports Content, SealedContent from willow-messaging)

willow-replay  → willow-worker → willow-actor
willow-storage → willow-worker → willow-actor
willow-agent   → willow-client
willow-client  → willow-common (shared wire types)
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

1. Add a variant to `Permission` in `crates/state/src/event.rs`
2. Add the `EventKind` → `Permission` mapping to `required_permission()` in `crates/state/src/materialize.rs`
3. Add state-machine tests: grant, revoke, rejection without permission

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
