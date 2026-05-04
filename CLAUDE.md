# CLAUDE.md — Willow Development Guide

## Project Overview

Willow = P2P Discord replacement, Rust. Uses iroh for networking, Leptos for web UI, Ed25519 for identity.

## Dev Guidelines

Quality + longevity beat speed + convenience.

- **Choose right solution, not easy one.** Ask: which approach makes most sense long-term, causes least future confusion, lasts? Pick that.
- **No hacky workarounds, no shortcuts.** If obvious fix is band-aid, keep digging for real fix.
- **Root-cause every bug.** No patching symptoms. No disabling failing tests. No swallowing errors. Find why, fix why.
- **Scope creep OK when warranted, not speculative.** Doing it right means touching more files / refactoring abstraction — do it. Don't add features, abstractions, error handling task didn't ask for.
- **Answer not obvious? Stop, design.** Two+ reasonable approaches? Brief note in `docs/specs/YYYY-MM-DD-<name>-design.md` before coding. Plan in `docs/plans/YYYY-MM-DD-<name>.md`. Cheap up front, expensive later. **Specs in `docs/specs/`, plans in `docs/plans/`.** No `docs/superpowers/`.
- **Surface tradeoffs explicit.** Picking between approaches, name runner-up + why rejected. Commit body or PR description. Future-you needs reasoning, not just result.
- **Mechanical rigor: `just check` before commit.** Fmt, clippy, tests, WASM. Zero warnings.
- **Semantic rigor: verify before claiming done.** Run actual test, hit actual UI, read actual output. No "should work" assertions. See `superpowers:verification-before-completion`.
- **Process skills before implementation skills.** Brainstorming + debugging determine *how*. Don't skip to feel productive.
- **Tests at lowest tier covering behavior.** State > client > browser > Playwright. See `## Which test tier to use`.

## Repository Structure

```
docs/
├── plans/              — Implementation plans for features (YYYY-MM-DD-<name>.md)
├── specs/              — Design specs and technical specifications (YYYY-MM-DD-<name>-design.md)
├── design/             — Long-form design documents (UX specs, etc.)
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

All shared mutable state in lib crates lives inside actor (see `crates/actor/`). Default `StateActor<S>` for new state. Decision tree:

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

**No `Arc<Mutex<T>>` / `Arc<RwLock<T>>` / `parking_lot::*` for business state.** New locks need `// state: lock-ok — <reason>` comment with rationale at use site. `MemNetwork` (`crates/network/src/mem.rs`) = test infra, exempt; iroh layer (`crates/network/src/iroh.rs`) = only production exception, justified by iroh's external-callback delivery model.

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
just test-actor     # test the actor framework
just test-relay     # test relay history sync
just test-workers   # test worker + replay + storage + common
just test-agent     # test agent library (MCP server)
just test-agent-e2e # multi-peer E2E via agent harness
just test-e2e-full  # setup-e2e + Playwright + teardown (CI-style)
just test-e2e-ui-headed # Playwright headed (debugging)
just test-crate X   # test a specific crate
just check-native   # cargo check (native target)
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
just docker-build   # build all docker images
just docker-up      # start full stack via docker compose
just docker-down    # stop docker stack
just docker-logs    # tail all docker compose logs
just docker-ids     # print all worker peer IDs
just clean          # cargo clean + remove crates/web/dist
```

**All code must pass `just check` (fmt + clippy + test + WASM) zero warnings before commit.** Browser tests (`just test-browser`) need Firefox + geckodriver installed.

### Local Development Stack

`just dev` to start all services locally:

| Service | Address | Description |
|---------|---------|-------------|
| Relay | `localhost:3340` | iroh relay HTTP + bootstrap |
| Replay node | connects via relay | In-memory state sync (max 1000 events/server) |
| Storage node | connects via relay | Archival SQLite storage |
| Web UI | `http://localhost:8080` | Leptos app via `trunk serve` |

Service logs color-coded, interleaved in terminal. `Ctrl+C` stops everything. Identity keys + data persist in `.dev/` across restarts so peer IDs stay stable. `just dev-clean` resets.

After first run, `just dev-quick` skips build, starts services immediately.

If `vibe-annotations-server` installed, `just dev` starts it auto at `http://127.0.0.1:3846` alongside other services.

## Agent Tooling

Two tools configured for agents on this project:

### agent-browser

CLI browser automation tool driving Willow web UI directly. Installed as skill at `.agents/skills/agent-browser` (symlinked into `.claude/skills/`).

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

Use agent-browser to verify UI features after changes, test flows end-to-end, investigate bugs by navigating live app. Always run `just dev` first.

### Vibe Annotations

Browser extension + local MCP server for visual UI feedback. Click elements in Chrome, leave annotated feedback agents read via MCP.

MCP server pre-configured in `.mcp.json` at `http://127.0.0.1:3846/mcp`.

**One-time setup (human):**
1. Install [Chrome extension](https://chromewebstore.google.com/detail/vibe-annotations-visual-f/gkofobaeeepjopdpahbicefmljcmpeof)
2. `npm install -g vibe-annotations-server`

**Usage:**
- `just dev` starts server auto if installed
- Open `http://localhost:8080` in Chrome, click extension to annotate elements
- Agent reads pending annotations via `read_annotations` MCP tool
- Agent deletes annotations after fixes via `delete_annotation`

**MCP tools available** (need restart Claude Code after first setup):
- `read_annotations` — fetch all pending annotations
- `watch_annotations` — poll for new annotations
- `delete_annotation` — mark annotation resolved
- `get_project_context` — infer tech stack from URL

### Dual-Target Support (Native + WASM)

All lib crates must compile both native + `wasm32-unknown-unknown`. Adding new code, ensure WASM compat:

- **No `std::fs`** in lib crates — gate with `#[cfg(not(target_arch = "wasm32"))]`
- **No `std::time::SystemTime`** — use `js_sys::Date::now()` on WASM
- **No `std::thread`** or **tokio** in lib crates — native-only
- **RNG**: `getrandom` needs `js` (v0.2) / `wasm_js` (v0.3) features on WASM
- **UUID**: workspace dep includes `js` feature for WASM v4 generation
- **Network**: iroh handles WASM transport differences internally, so most `#[cfg(target_arch = "wasm32")]` gates for networking no longer needed
- Use `#[cfg(target_arch = "wasm32")]` / `#[cfg(not(target_arch = "wasm32"))]` for platform-specific paths (storage backends, timers, etc.)

### Testing Strategy

Willow uses multi-tier testing:

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
- All components: sidebar, messages, input, channels, settings, member list, server list, connection status
- Requires: Firefox + geckodriver + wasm-pack

**5. Playwright E2E tests** (`just test-e2e-ui`, `just test-e2e-sync`):
- Multi-peer sync, permissions, mobile UI
- Real browser interaction against Leptos web app

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

**Default to lowest tier covering behaviour.**

**Rewrite trigger.** Playwright test fails because selector/helper drifts — not because behaviour broke — test at wrong tier. Migrate down same commit.

Full discussion: `docs/specs/2026-04-21-e2e-test-architecture-design.md`.

### Which Test to Write

**Adding feature or fixing bug, always add test at lowest level covering behavior.** Prefer state > client > browser > Playwright E2E. E2E only for behavior needing real P2P sync or browser interaction.

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
1. Add to `crates/client/src/tests/` (dominant pattern: e.g. `multi_peer_sync.rs`, `trust_flow.rs`) or to a `#[cfg(test)] mod tests_*` block in `lib.rs`
2. Use `test_client()` helper — creates ClientHandle without networking
3. `cargo test -p willow-client`

**Browser test**:
1. Add to `crates/web/tests/browser.rs`
2. Use `mount_test(|| view! { ... })` to render into DOM
3. Use `tick().await` to flush reactive effects
4. `wasm-pack test --headless --firefox crates/web`

**Playwright E2E test**:
1. Add to appropriate `e2e/*.spec.ts` file
2. Use helpers from `e2e/helpers.ts` (`setupTwoPeers`, `sendMessage`, etc.)
3. Multi-peer: use `browser` fixture, not hardcoded `chromium.launch()`
4. `npx playwright test e2e/your-file.spec.ts`

## Code Conventions

- **Crate naming**: `willow-<name>` in Cargo.toml, `willow_<name>` in code
- **Thread safety**: Use `Arc` (not `Rc`) everywhere — all types must be `Send + Sync`
- **Error handling**: `thiserror` for library error types, `anyhow` for application code
- **Documentation**: Every public type + function has doc comment. Module-level `//!` docs explain purpose + provide examples.
- **Testing**: Every crate has unit tests. Use `#[cfg(test)] mod tests` at bottom of each file.
- **Serialization**: All wire types derive `Serialize + Deserialize`. Round-trip tests validate compatibility.
- **Specs & Plans**: Design specs in `docs/specs/` named `YYYY-MM-DD-<feature-name>-design.md`. Plans in `docs/plans/` named `YYYY-MM-DD-<feature-name>.md`.

## Architecture Notes

### Authority Model

See [`docs/specs/2026-04-12-state-authority-and-mutations.md`](docs/specs/2026-04-12-state-authority-and-mutations.md). All authority checks live in `willow-state::materialize::apply_event` + `required_permission()` table. Permissions checked before event created — rejected events never enter DAG.

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

- **Network layer**: Fully async, iroh's QUIC transport + gossip protocol. Runs on background thread (native) or via spawn_local (WASM).
- **Client library**: Async API. Consumers drive from own runtime (tokio native, wasm-bindgen futures browser).
- **Deferred startup**: Network doesn't start until client explicitly connects, lets UI configure relay addresses first.

### Message Flow

1. User types in UI → `Message::text()` creates cleartext message
2. If channel key exists → `seal_content()` encrypts Content → `Content::Encrypted`
3. `pack_wire()` signs with Ed25519 → `TopicHandle::broadcast()` sends to gossip
4. iroh gossip delivers to subscribed peers
5. Listener task receives `GossipEvent::Received` → `unpack_wire()` verifies
6. Client forwards to UI via event stream
7. `identity::unpack()` verifies signature → `unpack_envelope()`
8. If `Content::Encrypted`, `open_content()` decrypts
9. Message rendered in UI

### Event-Based Server State Sync

Server mutations (channels, roles, permissions, kicks) sync via event-sourced `willow-state` machine over iroh gossip. Events broadcast + received through `Network` trait, applied deterministically via `apply()`.

### Event-Sourced State (willow-state)

All shared state derived from per-author Merkle DAG of signed events. `willow-state` crate pure — zero I/O, zero networking. See `docs/specs/2026-04-01-per-author-merkle-dag-state-design.md`.

- **Event** (`crates/state/src/event.rs`): signed by author Ed25519 key. Per-author `prev` hash links into the author's chain; cross-author causal `deps` array references other authors' events. Carries `EventKind` mutation variant.
- **EventKind** (`crates/state/src/event.rs`): variants for server structure, roles, fine-grained permissions, chat, identity, encryption, ephemeral.
- **EventDag** (`crates/state/src/dag.rs`): in-memory store of all known events, indexed by `EventHash`. `EventDag::insert` validates signature, genesis, `prev`/`deps` linkage. No explicit "merge" — DAG converges as events arrive.
- **ServerState** (`crates/state/src/server.rs`): materialized state derived by walking the DAG.
- **`materialize::apply_incremental(state, event)`**: ONLY public mutation entry point. Pure. Internally calls `apply_event` + `required_permission` for authority checks.
- **Permission model**: Owner = root of trust. Fine-grained permissions (SyncProvider, ManageChannels, ManageRoles, SendMessages, CreateInvite) granted via `GrantPermission` events. Admin status is structurally separate — managed exclusively through `ProposedAction` + vote path, never via `GrantPermission`. Kicks are an admin-only `ProposedAction` (no granular "can kick" permission).
- **Sync** (`crates/state/src/sync.rs`): `HeadsSummary` = compact per-author DAG state for efficient sync; `PendingBuffer` holds events arriving before their `prev` chain predecessors.

### Trust Model

- Owner = implicit all-permissions (root of trust chain).
- Permissions granted via `GrantPermission` events from owner/admin.
- Invite trust lists = *suggestions* — joining peers verify state from multiple sources, use majority-agreed state.
- Relay = regular client — trusted only if explicitly granted SyncProvider permission by owner.
- State verification: get state hash from multiple peers, use hash agreed upon by most trusted sources.

### Hybrid Logical Clocks (HLC)

Messages ordered using HLCs (`willow-messaging/src/hlc.rs`). Every node maintains `HLC` instance. Call `hlc.now()` for local events, `hlc.receive(remote_ts)` for remote messages. Ensures consistent ordering despite system clock drift.

## Common Tasks

### Adding a new message type

1. Add variant to `Content` in `crates/messaging/src/lib.rs`
2. Add constructor method on `Message`
3. Add tests
4. Handle new variant in web UI's message rendering

### Adding a new permission

1. Add variant to `Permission` in `crates/state/src/event.rs`
2. Add `EventKind` → `Permission` mapping to `required_permission()` in `crates/state/src/materialize.rs`
3. Add state-machine tests: grant, revoke, rejection without permission

### Adding a new iroh protocol

1. Define protocol in `crates/network/src/traits.rs` if needed
2. Implement in `crates/network/src/iroh.rs` using iroh's ALPN routing
3. Add test double in `crates/network/src/mem.rs`
4. Use trait in client/worker code

### Adding a new EventKind

1. Add variant to `EventKind` in `crates/state/src/event.rs` (re-exported from `lib.rs`)
2. Handle in `apply_event()` in `crates/state/src/materialize.rs` (add to `required_permission()` if it needs authority)
3. Expose method on `Client` in `crates/client/src/lib.rs` to emit it
4. Add state-machine tests for dedup, permission rejection, application
