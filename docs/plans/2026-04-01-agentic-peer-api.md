# Agentic Peer API — Implementation Plan

**Date**: 2026-04-01
**Spec**: `docs/specs/2026-03-29-agentic-peer-api-design.md`

## Overview

Build `willow-agent`, an MCP server binary that exposes `ClientHandle`
as tools/resources/notifications to AI agents, bots, and scripts. Also
build a multi-peer E2E test harness that exercises the full client stack
without a UI — this becomes the primary way to test complex multi-peer
scenarios.

Four phases, each producing a compilable, testable codebase.

---

## Phase 1: Crate Skeleton + CLI + Stdio MCP Server

**Goal**: A `willow-agent` binary that starts up, connects to the
network as a real peer, and serves a working MCP server over stdio with
tool discovery (`tools/list`) and resource listing (`resources/list`).
No tools execute yet — just the shell.

### 1a. Create `crates/agent/` crate

Create the crate with binary target:

```
crates/agent/
├── Cargo.toml
└── src/
    ├── main.rs       — CLI parsing (clap), startup, shutdown
    ├── server.rs     — MCP server setup, transport selection
    ├── tools.rs      — Tool definitions (schema only, stubs)
    ├── resources.rs  — Resource definitions (schema only, stubs)
    └── auth.rs       — Bearer token generation
```

**`Cargo.toml` dependencies:**
```toml
[package]
name = "willow-agent"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "willow-agent"
path = "src/main.rs"

[dependencies]
willow-client = { path = "../client" }
willow-identity = { path = "../identity" }
willow-network = { path = "../network" }
willow-actor = { path = "../actor" }
willow-state = { path = "../state" }
rmcp = { version = "0.1", features = ["server", "transport-io"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1"
rand = "0.8"

[dev-dependencies]
willow-network = { path = "../network", features = ["test-utils"] }
tempfile = "3"

# Note: willow-client test-utils feature is enabled for tests via:
# [features]
# test-harness = ["willow-client/test-utils"]
# (added in Phase 3 when the multi-peer harness is built)
```

### 1b. CLI parsing (`main.rs`)

Implement `clap::Parser` struct matching the spec CLI interface:

```rust
#[derive(Parser)]
#[command(name = "willow-agent", about = "Willow MCP agent peer")]
struct Cli {
    #[arg(long)]
    relay: Option<String>,
    #[arg(long, default_value = "Agent")]
    name: String,
    #[arg(long)]
    server: Option<String>,
    #[arg(long)]
    invite: Option<String>,
    #[arg(long, default_value = "stdio")]
    transport: String,           // "stdio" | "sse" | "http"
    #[arg(long, default_value = "127.0.0.1:9100")]
    bind: String,
    #[arg(long)]
    token: Option<String>,
    #[arg(long)]
    token_file: Option<String>,
    #[arg(long)]
    identity: Option<String>,    // defaults to ~/.willow/agent-identity
    #[arg(long)]
    persist: bool,
    #[arg(long, default_value = "info")]
    log_level: String,
    #[arg(long)]
    generate_identity: bool,
    #[arg(long)]
    print_peer_id: bool,
}
```

Follow the worker binary pattern from `crates/replay/src/main.rs`:
identity load/generate, tracing init, tokio runtime.

### 1c. Startup flow (`main.rs`)

1. Init tracing
2. Load or generate Ed25519 identity (reuse `willow_worker::identity`
   helpers, or inline equivalent — `Identity::load`/`Identity::generate`)
3. Create `ClientHandle<IrohNetwork>` via `ClientHandle::new(config)`
4. If `--relay`, create `IrohNetwork` and call `client.connect(network)`
5. If `--invite`, call `client.accept_invite(&code)`
6. If `--server`, call `client.switch_server(&id)`
7. Set display name via `client.set_display_name(&name)`
8. Start MCP server on selected transport (stdio only in Phase 1)
9. Block until stdin closes or SIGTERM

### 1d. MCP server shell (`server.rs`)

Use `rmcp` crate to create an MCP server. Register:
- Server info (name: "willow-agent", version)
- Tool schemas from 1e (schema-only stubs, handlers return
  `NotImplemented` until Phase 2)
- Resource schemas from 1f (placeholder JSON until Phase 2)

For stdio transport: `rmcp::transport::stdio::serve(server)`.

### 1e. Tool definitions — schema only (`tools.rs`)

Define all tool schemas from the spec as `rmcp::Tool` definitions.
Each tool has a name, description, and JSON Schema for `inputSchema`.
Tool handlers return `ToolError::NotImplemented` for now.

Tool groups (from spec):
- **Server**: `create_server`, `switch_server`, `leave_server`,
  `rename_server`, `set_server_description`, `authorize_workers`
- **Messaging**: `send_message`, `send_reply`, `share_file_inline`,
  `edit_message`, `delete_message`, `react`, `pin_message`,
  `unpin_message`
- **Channels**: `create_channel`, `create_voice_channel`,
  `delete_channel`, `switch_channel`
- **Permissions**: `trust_peer`, `untrust_peer`, `kick_member`,
  `create_role`, `delete_role`, `set_permission`, `assign_role`
- **Identity**: `set_display_name`, `set_server_display_name`,
  `send_typing`
- **Invites**: `generate_invite`, `accept_invite`, `create_join_link`,
  `delete_join_link`
- **Voice**: `join_voice`, `leave_voice`, `toggle_mute`, `toggle_deafen`
- **State**: `verify_state`

### 1f. Resource definitions — schema only (`resources.rs`)

Define all MCP resource URIs from the spec. Handlers return placeholder
JSON for now.

Static resources:
- `willow://identity`
- `willow://connection`
- `willow://servers`

Dynamic resources:
- `willow://server/current`
- `willow://server/channels`
- `willow://server/members`
- `willow://server/roles`
- `willow://server/unread`
- `willow://server/join-links`
- `willow://server/state-agreement`
- `willow://channel/{name}/messages`
- `willow://channel/{name}/pins`
- `willow://channel/{name}/typing`
- `willow://voice/status`
- `willow://voice/{channel}/participants`

### 1g. Bearer token generation (`auth.rs`)

- Generate 256-bit random token via `rand::OsRng`
- Prefix with `wlw_` and hex-encode
- `--token` flag overrides auto-generation
- `--token-file` writes token to file with 0600 permissions
- Stdio transport skips token auth (process isolation)

### 1h. Justfile + workspace integration

Add to `justfile`:
```
# Build the agent binary
build-agent:
    cargo build -p willow-agent

# Run the agent
agent *args:
    cargo run -p willow-agent -- {{args}}

# Test the agent crate
test-agent:
    cargo test -p willow-agent
```

No changes needed to `check-wasm` — it explicitly lists
WASM-compatible crates, so `willow-agent` is already excluded.

### 1i. Unit tests

- CLI parsing: verify defaults, required args
- Token generation: format, uniqueness, length
- Tool list: all expected tools present, schemas valid JSON
- Resource list: all expected URIs present

### Verification

```bash
cargo build -p willow-agent           # compiles
cargo test -p willow-agent            # unit tests pass
echo '{"jsonrpc":"2.0","method":"initialize","params":{"capabilities":{}},"id":1}' | cargo run -p willow-agent -- --transport stdio
# Returns MCP initialize response with server capabilities
just clippy                           # zero warnings
```

---

## Phase 2: Tool Implementations

**Goal**: Every MCP tool actually executes against `ClientHandle`.
Calling `send_message` via MCP delivers a real message over gossipsub.

### 2a. Wire `ClientHandle` into MCP server

The MCP server holds an `Arc<ClientHandle<IrohNetwork>>` (or generic
over `N: Network` for testability). Each tool handler receives a
reference to the client handle.

Define a `WillowMcpServer` struct:
```rust
pub struct WillowMcpServer<N: Network> {
    client: Arc<ClientHandle<N>>,
    // token_scope added in Phase 4
}
```

This struct implements `rmcp::ServerHandler` (or equivalent trait from
the rmcp crate).

### 2b. Implement messaging tools

Map each tool's JSON params to `ClientHandle` method calls:

| Tool | Calls |
|---|---|
| `send_message` | `client.send_message(channel, body)` |
| `send_reply` | `client.send_reply(channel, parent_id, body)` |
| `share_file_inline` | `client.share_file_inline(channel, filename, &base64_decode(data))` |
| `edit_message` | `client.edit_message(channel, message_id, new_body)` |
| `delete_message` | `client.delete_message(channel, message_id)` |
| `react` | `client.react(channel, message_id, emoji)` |
| `pin_message` | `client.pin_message(channel, message_id)` |
| `unpin_message` | `client.unpin_message(channel, message_id)` |

Return `{ "success": true }` on Ok, MCP error on Err.

### 2c. Implement channel tools

| Tool | Calls |
|---|---|
| `create_channel` | `client.create_channel(name)` |
| `create_voice_channel` | `client.create_voice_channel(name)` |
| `delete_channel` | `client.delete_channel(name)` |
| `switch_channel` | `client.switch_channel(name)` |

### 2d. Implement permission/member tools

Parse `peer_id` from 64-char hex string to `EndpointId`:

| Tool | Calls |
|---|---|
| `trust_peer` | `client.trust_peer(parse_endpoint_id(peer_id))` |
| `untrust_peer` | `client.untrust_peer(parse_endpoint_id(peer_id))` |
| `kick_member` | `client.kick_member(parse_endpoint_id(peer_id))` |
| `create_role` | `client.create_role(name)` |
| `delete_role` | `client.delete_role(role_id)` |
| `set_permission` | `client.set_permission(role_id, permission, granted)` |
| `assign_role` | `client.assign_role(parse_endpoint_id(peer_id), role_id)` |

### 2e. Implement server management tools

| Tool | Calls |
|---|---|
| `create_server` | `client.create_server(name)` |
| `switch_server` | `client.switch_server(id)` |
| `leave_server` | `client.leave_server()` |
| `rename_server` | `client.rename_server(name)` |
| `set_server_description` | `client.set_server_description(desc)` |
| `authorize_workers` | `client.authorize_workers(&parse_endpoint_ids(worker_peer_ids))` |

### 2f. Implement identity, invite, voice, state tools

**Identity:**
- `set_display_name` → `client.set_display_name(name)`
- `set_server_display_name` → `client.set_server_display_name(name)`
- `send_typing` → `client.send_typing()`

**Invites:**
- `generate_invite` → `client.generate_invite(&parse_endpoint_id(recipient_peer_id))`
- `accept_invite` → `client.accept_invite(code)`
- `create_join_link` → `client.create_join_link(max_uses, expires_at)`
- `delete_join_link` → `client.delete_join_link(link_id)`

**Voice:**
- `join_voice` → `client.join_voice(channel_id)`
- `leave_voice` → `client.leave_voice()`
- `toggle_mute` → `client.toggle_mute()`
- `toggle_deafen` → `client.toggle_deafen()`

**State:**
- `verify_state` → `client.verify_state()`

### 2g. Implement resource handlers

Wire each resource URI to the appropriate `ClientHandle` accessor or
`ClientViewHandle` `StateRef`. Resources return JSON-serialized
snapshots:

**Static:**
- `willow://identity` → `{ peer_id: client.peer_id(), display_name: client.display_name() }`
- `willow://connection` → `client.views().connection.get()` → serialize `ConnectionView`
- `willow://servers` → `client.server_list()` → `[{ id, name }]`

**Dynamic (per active server):**
- `willow://server/current` → `{ id, name, owner, description, display_name }`
  from `server_registry` + accessors
- `willow://server/channels` → `client.views().channels.get()` → serialize
- `willow://server/members` → `client.views().members.get()` → serialize
- `willow://server/roles` → `client.views().roles.get()` → serialize
- `willow://server/unread` → `client.views().unread.get()` → serialize
- `willow://server/join-links` → `client.join_links()`
- `willow://server/state-agreement` → `client.state_hash_agreement()`
- `willow://channel/{name}/messages` → filter `messages` view or
  `client.messages(name)`
- `willow://channel/{name}/pins` → `client.pinned_messages(name)`
- `willow://channel/{name}/typing` → filter `ConnectionView.typing_peers`
- `willow://voice/status` → `voice` state ref
- `willow://voice/{channel}/participants` → `client.voice_participants(channel)`

### 2h. Unit tests for tool dispatch

Create a local `test_mcp_client()` helper in the agent crate that
constructs a `WillowMcpServer<MemNetwork>` with a single-peer
`ClientHandle` (replicate the `test_client()` setup from
`crates/client/src/lib.rs`). This is Phase 2-only — Phase 3d
introduces a proper `test-utils` feature for multi-peer harnesses.

For each tool category:
1. Construct valid JSON params
2. Call the tool handler via the MCP server
3. Verify the state change via the underlying `ClientHandle` accessors

Example: call `send_message` tool, then `client.messages("general")`
should contain the message.

### Verification

```bash
cargo test -p willow-agent            # all tool + resource tests pass
just clippy                           # zero warnings
# Manual: pipe JSON-RPC tool calls via stdio, see real results
```

---

## Phase 3: Notifications + E2E Test Harness

**Goal**: Wire `ClientEvent` notifications to MCP, build the
`AgentTestHarness` for in-process multi-peer E2E testing, and write the
first batch of E2E tests. This phase is where we get the biggest
testing win — multi-peer scenarios without a browser.

### 3a. ClientEvent → MCP notifications (`notifications.rs`)

Create `crates/agent/src/notifications.rs`:

1. Subscribe to `Broker<ClientEvent>` via `client.subscribe_events()`
2. Spawn a task that reads from `EventReceiver`
3. For each `ClientEvent`, serialize to JSON matching the spec's
   notification format:
   ```json
   {
     "jsonrpc": "2.0",
     "method": "notifications/willow/event",
     "params": { "type": "MessageReceived", "channel": "general", ... }
   }
   ```
4. Forward to the MCP server's notification channel

Implement `ClientEvent` → JSON serialization for all 27 variants:
- `MessageReceived`, `MessageEdited`, `MessageDeleted`, `ReactionAdded`
- `PeerConnected`, `PeerDisconnected`
- `ChannelCreated`, `ChannelDeleted`
- `MemberKicked`, `PeerTrusted`, `PeerUntrusted`
- `ProfileUpdated`, `ServerRenamed`, `ServerDescriptionChanged`
- `SyncCompleted`, `RoleCreated`, `RoleDeleted`
- `StateHashMismatch`
- `MessagePinned`, `MessageUnpinned`
- `FileAnnounced`, `Listening`
- `VoiceJoined`, `VoiceLeft`, `VoiceSignal`
- `JoinLinkResponse`, `JoinLinkDenied`

### 3b. Resource subscription support

Wire MCP `resources/subscribe` to `StateRef<T>::subscribe()`:

1. When an MCP client subscribes to a resource URI, look up the
   backing `StateRef` from the resource table
2. Call `state_ref.subscribe()` to get a notification stream
3. Spawn a task that watches for changes and emits
   `notifications/resources/updated` to the MCP client
4. On unsubscribe, drop the subscription handle (auto-cleaned by actor)

Resources backed by `StateRef` (reactive): `connection`, `channels`,
`members`, `roles`, `unread`, `messages`, `voice/status`,
`voice/{channel}/participants`.

Resources backed by plain accessors (polled on read): `identity`,
`servers`, `server/current`, `server/join-links`,
`server/state-agreement`, `channel/{name}/pins`,
`channel/{name}/typing`.

### 3c. `AgentTestHarness` — in-process multi-peer E2E

Create `crates/agent/src/test_harness.rs` (cfg(test) only):

```rust
/// In-process test peers using MemNetwork.
pub(crate) struct AgentTestHarness {
    pub peers: Vec<TestPeer>,
    _system: willow_actor::System,
}

pub(crate) struct TestPeer {
    pub client: ClientHandle<MemNetwork>,
    pub endpoint_id: EndpointId,
    pub views: ClientViewHandle,
}

impl AgentTestHarness {
    /// Create N in-process peers on a shared MemNetwork hub.
    ///
    /// Peer 0 is the "owner" — creates the server and is trusted.
    /// Peers 1..N join via invite (or direct server state seeding).
    pub async fn start(n: usize) -> Self { ... }

    /// Convenience: get peer by index.
    pub fn peer(&self, i: usize) -> &TestPeer { ... }

    /// Wait for gossipsub delivery to settle across all peers.
    pub async fn settle(&self) { ... }

    pub async fn teardown(self) { ... }
}
```

Key design decisions:
- Uses `MemNetwork` (from `willow-network` `test-utils` feature) for
  zero-overhead gossipsub simulation
- Each peer gets its own `ClientHandle<MemNetwork>` with full actor
  tree (6 domain actors, derived views, persistence, broker)
- Peers share a `MemHub` for message delivery
- `settle()` sleeps briefly to let async actor propagation finish
  (start with 100ms, tune down as we learn the actual latency)
- Owner peer creates the server and trusts other peers automatically
  so tests can focus on the scenario, not setup boilerplate
- Exposed as `pub(crate)` — tests in the agent crate use it directly

Implementation notes:
- Adapt `test_client()` from `crates/client/src/lib.rs` — it already
  creates a `ClientHandle<MemNetwork>` with the full actor tree. The
  harness needs to extend this to N peers sharing a `MemHub` and
  connected to the same server.
- `MemNetwork::connect()` / topic subscription will wire peers together.
  Check if `MemNetwork` supports multi-peer hubs (it uses
  `tokio::sync::broadcast`). If not, extend `MemHub` to track multiple
  peers per topic.
- The existing `test_client()` is `pub(crate)` in `willow-client`. We
  need either: (a) a `pub` test helper in `willow-client` behind a
  `test-utils` feature, or (b) replicate the setup in `willow-agent`
  tests. Option (a) is cleaner — add `pub fn test_client_with_hub(hub)`
  to `willow-client` behind `#[cfg(feature = "test-utils")]`.

### 3d. Multi-peer client test utilities in `willow-client`

Add to `crates/client/Cargo.toml`:
```toml
[features]
test-utils = ["willow-network/test-utils"]
```

Add to `crates/client/src/lib.rs` (or a new `test_utils.rs`):
```rust
#[cfg(feature = "test-utils")]
pub mod test_utils {
    /// Create a ClientHandle connected to the given MemHub.
    /// Returns the client, its EndpointId, and the event broker.
    pub fn test_client_on_hub(
        hub: &MemHub,
        server_state: &ServerSeed,
    ) -> (ClientHandle<MemNetwork>, EndpointId, Addr<Broker<ClientEvent>>) { ... }

    /// Seed data for creating a shared server across test peers.
    pub struct ServerSeed { ... }

    /// Create a server seed owned by the given identity.
    pub fn create_server_seed(owner: &Identity) -> ServerSeed { ... }
}
```

This keeps the complex `ClientHandle` construction centralized in the
client crate and lets agent tests (and any future test consumers) just
call `test_client_on_hub()`.

### 3e. First E2E test batch

Create `crates/agent/tests/e2e.rs`:

**Test 1: `messages_delivered_to_all_peers`**
- 3 peers, Alice sends "hello everyone"
- Assert Bob and Carol see it via their views

**Test 2: `edit_message_propagates`**
- Alice sends a message, edits it
- Assert Bob sees the edited body

**Test 3: `delete_message_propagates`**
- Alice sends, then deletes
- Assert Bob no longer sees it

**Test 4: `reactions_propagate`**
- Alice sends, Bob reacts with 👍
- Assert Alice sees the reaction

**Test 5: `create_channel_visible_to_all`**
- Alice creates "dev" channel
- Assert Bob and Carol see it in their channel list

**Test 6: `pin_unpin_propagates`**
- Alice sends, pins the message
- Assert Bob sees it pinned
- Alice unpins, assert Bob sees it unpinned

**Test 7: `concurrent_messages_converge`**
- Alice and Bob send simultaneously
- Assert both peers see both messages (same order)

**Test 8: `events_emitted_on_message_received`**
- Subscribe to Bob's event broker
- Alice sends a message
- Assert Bob's broker emits `MessageReceived`

### 3f. Permission E2E tests

**Test 9: `untrusted_peer_cannot_create_channel`**
- 2 peers, owner doesn't trust guest
- Guest tries `create_channel` → expect error
- Verify channel wasn't created via owner's view

**Test 10: `kick_member_removes_from_server`**
- Owner kicks guest
- Assert guest is no longer in member list
- Assert `MemberKicked` event emitted

**Test 11: `trust_then_untrust_flow`**
- Owner trusts peer, peer creates channel (succeeds)
- Owner untrusts peer, peer tries to create another (fails)

**Test 12: `role_permission_enforcement`**
- Create role with `SendMessages` only
- Assign to guest
- Guest can send messages but cannot create channels

### 3g. State convergence E2E tests

**Test 13: `state_hash_agreement`**
- 3 peers, perform several operations
- Call `verify_state` on all peers
- Assert `state_hash_agreement` returns unanimous

**Test 14: `concurrent_channel_creation`**
- Alice and Bob both create channels simultaneously
- Assert both channels exist on both peers after settling

**Test 15: `10_peer_message_flood`**
- 10 peers, each sends 5 messages to "general"
- Assert all peers see all 50 messages

### 3h. Notification unit tests

- Serialize each `ClientEvent` variant to JSON
- Verify field names match the spec notification table
- Round-trip: serialize → deserialize → compare

### Verification

```bash
cargo test -p willow-agent                          # unit + notification tests
cargo test -p willow-agent --test e2e               # E2E tests
cargo test -p willow-client --features test-utils   # client test-utils
just clippy
```

---

## Phase 4: Token Scoping + SSE Transport + Justfile Integration

**Goal**: Add bearer token scoping, SSE/HTTP transports, integrate into
the dev stack, and port remaining high-value Playwright scenarios.

### 4a. Token scoping (`scopes.rs`)

Create `crates/agent/src/scopes.rs`:

```rust
#[derive(Debug, Clone)]
pub enum TokenScope {
    Full,
    ReadOnly,
    Messaging,
    Admin,
    Custom(HashSet<String>),
}

impl TokenScope {
    /// Returns true if the given tool name is allowed by this scope.
    pub fn allows_tool(&self, tool_name: &str) -> bool { ... }

    /// Returns true if the given resource URI is allowed.
    pub fn allows_resource(&self, uri: &str) -> bool { ... }
}
```

Scope definitions:
- **Full**: all tools, all resources
- **ReadOnly**: no tools, all resources
- **Messaging**: `send_message`, `send_reply`, `edit_message`,
  `delete_message`, `react`, `pin_message`, `unpin_message`,
  `send_typing` + all resources
- **Admin**: all tools + all resources (same as Full, but semantically
  distinct for future per-tool audit logging)
- **Custom**: explicit allowlist of tool names

Wire into `WillowMcpServer`:
- `tools/list` filters by scope
- `tools/call` checks scope before dispatch, returns MCP error if denied
- `resources/list` filters by scope

### 4b. SSE transport (`server.rs`)

Add `rmcp` feature `transport-sse`:
```toml
rmcp = { version = "0.1", features = ["server", "transport-io", "transport-sse"] }
```

When `--transport sse`:
1. Generate bearer token (or use `--token`)
2. Start HTTP server on `--bind` address
3. SSE endpoint at `/sse` for long-lived connections
4. Validate `Authorization: Bearer <token>` header
5. Print token to stderr and optionally to `--token-file`

When `--transport http`:
1. Same as SSE but use Streamable HTTP at `/mcp`
2. Supports stateless request/response and optional session upgrade

### 4c. Justfile updates

```just
# Build the agent binary
build-agent:
    cargo build -p willow-agent

# Build agent (release)
build-agent-release:
    cargo build --release -p willow-agent

# Run the agent
agent *args:
    cargo run -p willow-agent -- {{args}}

# Test agent unit + integration
test-agent:
    cargo test -p willow-agent

# Run E2E multi-peer tests via agent harness
test-agent-e2e:
    cargo test -p willow-agent --test e2e -- --nocapture

# Update test-all to include agent
test-all: test test-browser test-agent-e2e test-e2e-ui
```

Update `scripts/dev.sh` to optionally start an agent process alongside
relay + workers + web UI. Add `--agent` flag to `dev.sh` that starts
`willow-agent --transport sse --relay <local-relay>`.

### 4d. Advanced E2E tests (ported from Playwright scenarios)

These scenarios are currently tested via Playwright with real browsers.
Port the core logic to in-process E2E tests:

**Test 16: `kick_and_rejoin_flow`**
- Owner kicks member
- Verify member removed from all peer views
- Member re-joins via new invite
- Verify member visible again

**Test 17: `invite_max_uses_enforcement`**
- Create join link with max_uses=2
- Two peers join successfully
- Third peer's join attempt is rejected

**Test 18: `server_rename_propagates`**
- Owner renames server
- All peers see `ServerRenamed` event
- Server name updated in all views

**Test 19: `display_name_propagates`**
- Peer sets display name
- All other peers see `ProfileUpdated` event
- Member list shows new name

**Test 20: `voice_join_leave_tracking`**
- Peer joins voice channel
- All peers see `VoiceJoined` event
- Peer leaves, all see `VoiceLeft`
- Voice participants list updates

**Test 21: `offline_peer_catches_up`** (if MemNetwork supports
disconnect/reconnect simulation)
- Peer goes offline
- Other peers send messages
- Peer reconnects
- Assert peer sees all missed messages

### 4e. Scope enforcement tests

**Test 22: `readonly_token_hides_tools`**
- Create MCP server with `ReadOnly` scope
- `tools/list` returns empty
- `resources/list` returns full list
- Calling any tool returns error

**Test 23: `messaging_scope_restricts_tools`**
- Create MCP server with `Messaging` scope
- `tools/list` shows only messaging tools
- `create_channel` call returns error
- `send_message` call succeeds

**Test 24: `custom_scope_allowlist`**
- Create scope with only `["send_message", "react"]`
- Verify only those tools appear and execute

### Verification

```bash
just test-agent                  # unit tests
just test-agent-e2e              # all E2E tests (15+ scenarios)
just clippy                      # zero warnings
just check                       # full check passes
```

---

## Phase Ordering

```
Phase 1 (Crate skeleton, CLI, stdio MCP shell)
    ↓
Phase 2 (Tool + resource implementations)
    ↓
Phase 3 (Notifications, E2E harness, first 15 E2E tests)
    ↓
Phase 4 (Scopes, SSE/HTTP, justfile, 10+ more E2E tests)
```

All phases are sequential. Each produces a compilable codebase that
passes `just check`.

---

## Files Changed (complete list)

### Created

```
crates/agent/Cargo.toml
crates/agent/src/main.rs           — CLI, startup, shutdown
crates/agent/src/server.rs         — MCP server setup, transports
crates/agent/src/tools.rs          — MCP tool definitions + handlers
crates/agent/src/resources.rs      — MCP resource definitions + handlers
crates/agent/src/notifications.rs  — ClientEvent → MCP notifications
crates/agent/src/auth.rs           — Bearer token generation + validation
crates/agent/src/scopes.rs         — TokenScope definitions + enforcement
crates/agent/src/test_harness.rs   — AgentTestHarness (cfg(test))
crates/agent/tests/e2e.rs          — Multi-peer E2E test suite
```

### Modified

```
crates/client/Cargo.toml           — add test-utils feature
crates/client/src/lib.rs           — add pub test_utils module
justfile                           — add agent targets, update test-all
scripts/dev.sh                     — optional --agent flag
```

### E2E Test Inventory

| # | Test | Phase | Harness |
|---|---|---|---|
| 1 | messages_delivered_to_all_peers | 3 | in-process |
| 2 | edit_message_propagates | 3 | in-process |
| 3 | delete_message_propagates | 3 | in-process |
| 4 | reactions_propagate | 3 | in-process |
| 5 | create_channel_visible_to_all | 3 | in-process |
| 6 | pin_unpin_propagates | 3 | in-process |
| 7 | concurrent_messages_converge | 3 | in-process |
| 8 | events_emitted_on_message_received | 3 | in-process |
| 9 | untrusted_peer_cannot_create_channel | 3 | in-process |
| 10 | kick_member_removes_from_server | 3 | in-process |
| 11 | trust_then_untrust_flow | 3 | in-process |
| 12 | role_permission_enforcement | 3 | in-process |
| 13 | state_hash_agreement | 3 | in-process |
| 14 | concurrent_channel_creation | 3 | in-process |
| 15 | 10_peer_message_flood | 3 | in-process |
| 16 | kick_and_rejoin_flow | 4 | in-process |
| 17 | invite_max_uses_enforcement | 4 | in-process |
| 18 | server_rename_propagates | 4 | in-process |
| 19 | display_name_propagates | 4 | in-process |
| 20 | voice_join_leave_tracking | 4 | in-process |
| 21 | offline_peer_catches_up | 4 | in-process |
| 22 | readonly_token_hides_tools | 4 | unit |
| 23 | messaging_scope_restricts_tools | 4 | unit |
| 24 | custom_scope_allowlist | 4 | unit |

---

## Future (post-Phase 4)

- `McpTestHarness` — process-spawning harness with real iroh for MCP
  protocol-level integration tests
- `crates/agent-sdk/` — typed Rust MCP client library
- Webhook ingress endpoint
- Rate limiting per token
- Audit logging
- MCP prompts for common workflows
- Multi-server support via `switch_server` tool
