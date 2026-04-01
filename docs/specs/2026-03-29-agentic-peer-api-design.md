# Agentic Peer API Design Spec

**Date**: 2026-03-29
**Status**: Draft

## Overview

Expose the `ClientHandle` API to external agents (AI assistants, bots,
automation scripts) via an MCP (Model Context Protocol) server embedded
in a new `willow-agent` binary. Agents connect over stdio, SSE, or
Streamable HTTP and interact with Willow through MCP tools, resources,
and notifications. The agent binary is a first-class Willow peer — same
identity, same permissions, same gossipsub participation.

## Why MCP

MCP is JSON-RPC 2.0 with conventions purpose-built for AI agent
integration. Choosing MCP over raw JSON-RPC, REST, or gRPC gives us:

1. **Zero-integration AI access** — Any MCP-compatible client (Claude
   Code, Claude Desktop, Cursor, Windsurf, ChatGPT, Gemini, etc.) can
   connect directly. The agent declares its tools and the AI discovers
   them at runtime via `tools/list`. No custom SDK required.
2. **JSON-RPC 2.0 superset** — Non-AI consumers (Python scripts, CLI
   tools, bots) still work via plain JSON-RPC. Nothing is lost.
3. **Built-in schema discovery** — `tools/list` returns every operation
   with typed JSON Schema parameters. Agents know exactly what they can
   call without external documentation.
4. **Resources for state** — Server members, channel lists, message
   history map naturally to MCP resources that AI agents can read.
5. **Server-sent notifications** — MCP supports server→client
   notifications, mapping directly to `ClientEvent` streaming.
6. **Standardized auth** — MCP defines OAuth 2.1 for remote servers
   and simpler bearer token auth for local transports.

## Motivation

Willow's `ClientHandle` already provides a rich, UI-agnostic API for
every operation: messaging, channels, roles, permissions, invites, voice
signaling, file sharing, and state verification. Today only the Bevy
desktop app and Leptos web app consume it. Opening this API to external
processes enables:

- **AI chat agents** that participate in conversations, answer questions,
  summarize threads, or moderate content
- **CI/CD bots** that post build status, deploy notifications, or PR
  links into channels
- **Webhook bridges** that relay events from GitHub, Sentry, PagerDuty
  into Willow channels
- **Custom automation** — scheduled messages, on-call rotations,
  standup bots, poll bots
- **CLI tooling** — scriptable Willow access for power users
- **Multi-agent workflows** — AI agents collaborating across channels

## Design Principles

1. **Peer, not proxy**: The agent binary is a real peer with its own
   Ed25519 identity. It participates in gossipsub, signs messages, and
   is subject to the same permission model as any user.
2. **No new wire protocol**: Agents don't need a new P2P protocol. They
   talk to the local `willow-agent` process over MCP; that process
   handles all networking via the existing `ClientHandle`.
3. **Minimal surface**: MCP tools map 1:1 to `ClientHandle` methods.
   MCP resources map to state accessors. No new abstractions — if the
   client can do it, the agent can do it.
4. **Event streaming**: Agents receive `ClientEvent`s as MCP
   notifications, enabling reactive behavior without polling.
5. **Local-only by default**: The MCP server uses stdio (spawned by AI
   client) or binds to `127.0.0.1`. No remote access without explicit
   configuration.

## Architecture

```
┌─────────────┐     MCP        ┌──────────────────┐   gossipsub   ┌───────────┐
│  AI Agent   │───────────────▶│  willow-agent    │◀────────────▶│  Willow   │
│  (Claude,   │  stdio / SSE   │                  │   iroh QUIC   │  Network  │
│   scripts)  │◀───────────────│  ClientHandle<N> │               │           │
│             │  notifications │  + MCP server    │               │           │
└─────────────┘                └──────────────────┘               └───────────┘
```

### Internal Architecture

The `willow-agent` binary owns a `ClientHandle<IrohNetwork>` backed by
the `willow-actor` system. The client uses a **multi-actor reactive
architecture** with three layers:

**Layer 1 — Domain State Actors** (6 `StateActor<S>` instances):

| Actor | State Type | Fields |
|---|---|---|
| Event State | `ServerState` | Event-sourced channels, roles, members, messages, permissions |
| Server Registry | `ServerRegistry` | `servers: HashMap<String, ServerEntry>`, `active_server: Option<String>` — each entry holds server, topic map, channel keys, unread counts |
| Chat Meta | `ChatMeta` | `current_channel: String`, `peers: Vec<EndpointId>`, `seen_message_ids: HashSet<String>` |
| Profiles | `ProfileState` | `names: HashMap<EndpointId, String>` |
| Network Meta | `NetworkMeta` | `connected: bool`, `typing_peers: HashMap<EndpointId, (String, u64)>`, `last_typing_sent_ms`, `state_verification_results` |
| Voice State | `VoiceState` | `participants: HashMap<String, HashSet<EndpointId>>`, `active_channel`, `muted`, `deafened` |

Each actor holds its state as `Arc<S>` with copy-on-write mutations.
Subscribers are notified only when state actually changes (`PartialEq`).

**Layer 2 — Derived Views** (`DerivedActor` instances):
Reactive computed views that subscribe to layer 1 actors and recompute
automatically. Each is a pure function of its sources:

| View | Sources | Produces |
|---|---|---|
| `MessagesView` | EventState, ServerRegistry, ChatMeta, ProfileState | `Vec<DisplayMessage>` for current channel |
| `ChannelsView` | EventState, ServerRegistry | `Vec<ChannelInfo>` with name + kind |
| `MembersView` | EventState, ChatMeta, ProfileState | `Vec<MemberInfo>` with online status |
| `UnreadView` | ServerRegistry | `HashMap<String, usize>` per channel |
| `RolesView` | EventState | `Vec<RoleEntry>` with permissions |
| `ConnectionView` | NetworkMeta, ChatMeta | `connected`, `peer_count`, `typing_peers` |

These only recompute when their sources change, and only notify
downstream if the computed value differs (`PartialEq`).

**Layer 3 — Composite Views**:
`ChatViews`, `SocialViews`, and a terminal `ClientView` that groups
everything into a single snapshot.

**Access Surfaces**:
- **`client.views()`** → `ClientViewHandle` with `StateRef<T>` handles:
  - Terminal: `view` (`ClientView` — everything in one snapshot)
  - Layer 2: `messages`, `members`, `channels`, `unread`, `roles`,
    `connection` (individual derived views)
  - Layer 1: `event_state`, `server_registry`, `chat_meta`, `profiles`,
    `network`, `voice` (raw source state)
- **`client.mutations()`** → `ClientMutations<N>` typed mutation
  interface for event-sourced operations
- **`ClientHandle` methods** → higher-level actions that coordinate
  multiple actors (e.g., `kick_member`, `create_voice_channel`,
  `set_permission`, `assign_role`, `share_file_inline`)
- **`Broker<ClientEvent>`** → pub/sub event distribution
- **`PersistenceActor`** → fire-and-forget database writes (owns
  non-Send rusqlite handles, single-threaded by actor guarantee)

This maps naturally to MCP: `StateRef` subscriptions power resource
change notifications, `ClientMutations` methods become tools, and
`Broker<ClientEvent>` feeds MCP notifications.

Peer identities use `EndpointId` (an Ed25519 public key from iroh),
which displays as a 64-character hex string. All tool parameters and
resource fields that reference peers use this hex format.

### Components

**1. `willow-agent` binary** (`crates/agent/`)
- Owns a `ClientHandle<IrohNetwork>` with its actor system
- Runs an MCP server supporting two transports:
  - **stdio** (default) — AI clients spawn the binary directly
  - **Streamable HTTP** — `http://127.0.0.1:9100/mcp` for network
    clients (supports both SSE streaming and stateless HTTP via rmcp's
    `transport-streamable-http-server` feature)
- Exposes `ClientHandle` methods as MCP tools
- Exposes state accessors as MCP resources
- Streams `ClientEvent`s as MCP notifications
- Authenticates SSE/HTTP connections with a bearer token

**2. Permission scoping** (bearer token + server-side filtering)
- Each bearer token is scoped to a set of allowed tool categories
- Default token: full access (matches the peer's permissions)
- Restricted tokens: read-only, messaging-only, admin-only
- Tools that exceed the token scope return MCP error responses

## MCP Server Capabilities

### Transports

| Transport | Use Case | Auth |
|---|---|---|
| **stdio** | AI client spawns `willow-agent` as subprocess | Implicit (process isolation) |
| **Streamable HTTP** | Scripts/bots, stateless or long-lived SSE | Bearer token header |

### Tools

Every mutating method on `ClientHandle<N>` maps to an MCP tool.
Internally, `ClientHandle` delegates to `ClientMutations<N>` for
event-sourced operations and directly to domain actors for operations
that span multiple actors (e.g., `kick_member`, `create_voice_channel`,
`set_permission`, `assign_role`). Tools are discoverable via
`tools/list` and include full JSON Schema for params.

#### Server Management

```json
{
  "name": "create_server",
  "description": "Create a new server with the given name. Returns the server ID.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "name": { "type": "string", "description": "Server display name" }
    },
    "required": ["name"]
  }
}
```

Other server tools: `switch_server`, `leave_server`, `rename_server`,
`set_server_description`, `authorize_workers`.

#### Messaging

| Tool | Parameters | Description |
|---|---|---|
| `send_message` | `channel`, `body` | Send a text message |
| `send_reply` | `channel`, `parent_id`, `body` | Reply to a message |
| `share_file_inline` | `channel`, `filename`, `data` | Share file (base64, max 256KB) |
| `edit_message` | `channel`, `message_id`, `new_body` | Edit a message |
| `delete_message` | `channel`, `message_id` | Delete a message |
| `react` | `channel`, `message_id`, `emoji` | Add emoji reaction |
| `pin_message` | `channel`, `message_id` | Pin a message |
| `unpin_message` | `channel`, `message_id` | Unpin a message |

#### Channels

| Tool | Parameters | Description |
|---|---|---|
| `create_channel` | `name` | Create a text channel |
| `create_voice_channel` | `name` | Create a voice channel |
| `delete_channel` | `name` | Delete a channel |
| `switch_channel` | `name` | Set active channel |

#### Permissions & Members

All `peer_id` parameters accept an `EndpointId` as a 64-character hex
string (the Ed25519 public key of the target peer).

| Tool | Parameters | Description |
|---|---|---|
| `trust_peer` | `peer_id` | Grant Administrator permission |
| `untrust_peer` | `peer_id` | Revoke Administrator permission |
| `kick_member` | `peer_id` | Remove member, rotate keys |
| `create_role` | `name` | Create a permission role |
| `delete_role` | `role_id` | Delete a role |
| `set_permission` | `role_id`, `permission`, `granted` | Set role permission |
| `assign_role` | `peer_id`, `role_id` | Assign role to peer |
| `authorize_workers` | `worker_peer_ids` | Grant SyncProvider to workers |

Valid `permission` values: `SyncProvider`, `ManageChannels`,
`ManageRoles`, `KickMembers`, `SendMessages`, `CreateInvite`,
`Administrator`.

#### Identity

| Tool | Parameters | Description |
|---|---|---|
| `set_display_name` | `name` | Set agent's display name |
| `set_server_display_name` | `name` | Set server-scoped name |
| `send_typing` | | Broadcast typing indicator |

#### Invites

| Tool | Parameters | Description |
|---|---|---|
| `generate_invite` | `recipient_peer_id` | Create encrypted invite |
| `accept_invite` | `code` | Accept invite, join server |
| `create_join_link` | `max_uses`, `expires_at?` | Create shareable link |
| `delete_join_link` | `link_id` | Delete a join link |

#### Voice

| Tool | Parameters | Description |
|---|---|---|
| `join_voice` | `channel_id` | Join a voice channel |
| `leave_voice` | | Leave current voice channel |
| `toggle_mute` | | Toggle mute state, returns new state |
| `toggle_deafen` | | Toggle deafen state, returns new state |

#### State

| Tool | Parameters | Description |
|---|---|---|
| `verify_state` | | Broadcast state hash for verification |

### Resources

Read-only state is exposed as MCP resources via `client.views()`.
Each resource maps to a `StateRef<T>` from the reactive view system.
AI agents read resources via `resources/read`; the MCP server reads
the underlying `StateRef` snapshot (cheap `Arc` clone, no computation
on read).

All `peer_id` and `author` fields in resource responses are
`EndpointId` values — 64-character hex strings representing Ed25519
public keys.

#### Static Resources (always available)

| URI | Backed By | Returns |
|---|---|---|
| `willow://identity` | `Identity` + `ProfileState` | `{ peer_id, display_name }` |
| `willow://connection` | `StateRef<ConnectionView>` | `{ connected, peer_count, typing_peers: [{ peer_id, channel }] }` |
| `willow://servers` | `StateRef<ServerRegistry>` | `[{ id, name }]` |

#### Dynamic Resources (per active server)

| URI Template | Backed By | Returns |
|---|---|---|
| `willow://server/current` | `StateRef<ServerRegistry>` | `{ id, name, owner, description, display_name }` |
| `willow://server/channels` | `StateRef<ChannelsView>` | `[{ name, kind }]` |
| `willow://server/members` | `StateRef<MembersView>` | `[{ peer_id, display_name, is_online }]` |
| `willow://server/roles` | `StateRef<RolesView>` | `[{ id, name, permissions }]` |
| `willow://server/unread` | `StateRef<UnreadView>` | `{ channel: count }` |
| `willow://server/join-links` | `join_links` accessor | `[{ id, max_uses, uses }]` |
| `willow://server/state-agreement` | `NetworkMeta` | `{ agreeing, total }` |
| `willow://channel/{name}/messages` | `StateRef<MessagesView>` (filtered) | `[{ id, author, body, timestamp, edited, reply_to, reactions }]` |
| `willow://channel/{name}/pins` | `event_state` accessor | `[{ id, author, body }]` |
| `willow://channel/{name}/typing` | `NetworkMeta` + accessor | `[{ peer_id, display_name }]` |
| `willow://voice/status` | `StateRef<VoiceState>` | `{ active_channel, muted, deafened }` |
| `willow://voice/{channel}/participants` | `StateRef<VoiceState>` | `[{ peer_id }]` |

Resources support MCP's `resources/subscribe` for change notifications.
Under the hood, the MCP server calls `StateRef<T>::subscribe()` on the
backing view actor. When the `DerivedActor` recomputes and the value
actually changes (`PartialEq` check), it sends a `Notify` message to
the MCP server, which emits `notifications/resources/updated` to the
agent. This means:

- **No polling** — changes push from state actors through derived views
  to the MCP transport automatically
- **No spurious updates** — `PartialEq` at every layer ensures agents
  only see real changes
- **Granular subscriptions** — agents can subscribe to individual
  resources (just messages, just members) rather than getting firehosed

The `Backed By` column in the resource tables above shows the exact
`StateRef<T>` or accessor that powers each resource. Resources backed
by a `StateRef` (derived views or layer 1 actors) support reactive
subscriptions. Resources backed by plain accessors (e.g., join links,
pinned messages) are polled on read.

### Notifications (Server → Client)

`ClientEvent`s are distributed via `Broker<ClientEvent>`. The MCP
server subscribes to the broker and forwards each event as an MCP
notification. Agents receive these automatically on stdio/SSE
transports. Dead subscriptions are auto-pruned by the broker.

```json
{
  "jsonrpc": "2.0",
  "method": "notifications/willow/event",
  "params": {
    "type": "MessageReceived",
    "channel": "general",
    "message_id": "msg-uuid-123",
    "is_local": false
  }
}
```

All `ClientEvent` variants are forwarded:

| Notification Type | Key Fields |
|---|---|
| `MessageReceived` | `channel`, `message_id`, `is_local` |
| `MessageEdited` | `channel`, `message_id`, `new_body` |
| `MessageDeleted` | `channel`, `message_id` |
| `ReactionAdded` | `channel`, `message_id`, `emoji`, `author` |
| `PeerConnected` | `peer_id` |
| `PeerDisconnected` | `peer_id` |
| `ChannelCreated` | `name` |
| `ChannelDeleted` | `name` |
| `MemberKicked` | `peer_id` |
| `PeerTrusted` | `peer_id` |
| `PeerUntrusted` | `peer_id` |
| `ProfileUpdated` | `peer_id`, `display_name` |
| `ServerRenamed` | `new_name` |
| `SyncCompleted` | `ops_applied` |
| `RoleCreated` | `name`, `role_id` |
| `RoleDeleted` | `role_id` |
| `StateHashMismatch` | `peer_id`, `our_hash`, `their_hash` |
| `MessagePinned` | `channel`, `message_id` |
| `MessageUnpinned` | `channel`, `message_id` |
| `ServerDescriptionChanged` | `description` |
| `FileAnnounced` | `channel`, `filename`, `size`, `from` |
| `Listening` | `address` (iroh node address string) |
| `VoiceJoined` | `channel_id`, `peer_id` |
| `VoiceLeft` | `channel_id`, `peer_id` |
| `VoiceSignal` | `channel_id`, `from_peer`, `signal` |
| `JoinLinkResponse` | `invite_data` |
| `JoinLinkDenied` | `reason` |

## `willow-agent` Binary

### CLI Interface

```
willow-agent [OPTIONS]

Options:
  --relay <MULTIADDR>       Relay address (required)
  --name <NAME>             Display name [default: "Agent"]
  --server <ID>             Auto-join server by ID
  --invite <CODE>           Accept invite on startup
  --transport <MODE>        MCP transport: stdio | http [default: stdio]
  --bind <ADDR>             HTTP bind address [default: 127.0.0.1:9100]
  --token <TOKEN>           Fixed bearer token (generated if omitted)
  --token-file <PATH>       Write token to file for other processes
  --identity <PATH>         Identity key path [default: ~/.willow/agent-identity]
  --persist                 Enable persistent storage
  --log-level <LEVEL>       Log verbosity [default: info]
```

### Startup Flow

1. Load or generate Ed25519 identity
2. Start `willow-actor` system
3. Create `ClientHandle<IrohNetwork>` with config — spawns all 6
   domain state actors, derived view actors, persistence actor, and
   event broker
4. Call `client.connect(network)` — starts iroh node, subscribes to
   gossipsub topics, spawns topic listener tasks
5. If `--invite`, accept it; if `--server`, switch to it
6. Subscribe MCP server to `Broker<ClientEvent>` for notifications
7. Subscribe MCP server to relevant `StateRef<T>` views for resource
   change detection
8. Start MCP server on the selected transport:
   - **stdio**: read JSON-RPC from stdin, write to stdout (default)
   - **http**: generate bearer token, start Streamable HTTP endpoint
     at `/mcp` (supports SSE streaming and stateless request/response)
9. Block until stdin closes (stdio) or SIGTERM/SIGINT (http)

### AI Client Configuration

AI agents configure `willow-agent` as an MCP server in their config:

**Claude Code (`~/.claude/claude_code_config.json`):**
```json
{
  "mcpServers": {
    "willow": {
      "command": "willow-agent",
      "args": [
        "--relay", "/ip4/relay.example.com/tcp/9091/ws",
        "--name", "Claude",
        "--invite", "wlw_invite_code..."
      ]
    }
  }
}
```

**Claude Desktop (`claude_desktop_config.json`):**
```json
{
  "mcpServers": {
    "willow": {
      "command": "willow-agent",
      "args": ["--relay", "/ip4/1.2.3.4/tcp/9091/ws", "--name", "Assistant"]
    }
  }
}
```

The AI client spawns the process, communicates over stdio, and
discovers all tools/resources automatically via `initialize`.

### HTTP Mode for Scripts/Bots

```
$ willow-agent --relay /ip4/1.2.3.4/tcp/9091/ws --name "BuildBot" --transport http
Agent endpoint ID: a1b2c3d4e5f6...  (64-char hex)
MCP server listening on: http://127.0.0.1:9100
Bearer token: wlw_a1b2c3d4e5f6...

# Call a tool via JSON-RPC over HTTP:
curl -X POST http://127.0.0.1:9100/mcp \
  -H "Authorization: Bearer wlw_a1b2c3d4e5f6..." \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "tools/call",
    "params": {
      "name": "send_message",
      "arguments": { "channel": "general", "body": "Build #42 passed" }
    },
    "id": 1
  }'
```

## Permission Model

Agents are regular peers. Their capabilities are determined by:

1. **Network-level permissions**: What the server owner grants to the
   agent's peer ID (via `trust_peer`, `assign_role`, `set_permission`)
2. **Token-level scoping**: The bearer token can further restrict what
   tools the agent process exposes through MCP

### Token Scopes

```rust
enum TokenScope {
    /// Full access — all tools and resources
    Full,
    /// Read-only — resources only, no tools
    ReadOnly,
    /// Messaging — send/edit/delete messages, reactions, typing
    Messaging,
    /// Admin — full access including permission management
    Admin,
    /// Custom — explicit allowlist of tool names
    Custom(HashSet<String>),
}
```

Token scopes filter which tools appear in `tools/list` and which
resources appear in `resources/list`. A `ReadOnly` token hides all
mutating tools entirely — the AI agent never even sees them. Scopes
cannot grant more than the peer's network permissions — they can only
restrict.

### Trust Setup

Server owners trust an agent the same way they trust any peer:

1. Agent starts and connects to the network
2. Owner sees agent's peer ID in the member list
3. Owner runs `trust_peer` or assigns a role with specific permissions
4. Agent can now perform operations matching its permissions

No special trust model. No backdoors. The agent is just a peer.

## Event-Driven Agent Pattern

### Python (using any MCP client library)

```python
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

async def main():
    server = StdioServerParameters(
        command="willow-agent",
        args=["--relay", "/ip4/1.2.3.4/tcp/9091/ws", "--name", "Bot"],
    )

    async with stdio_client(server) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()

            # Discover available tools
            tools = await session.list_tools()
            print(f"Available: {[t.name for t in tools.tools]}")

            # Read channel messages
            messages = await session.read_resource("willow://channel/general/messages")
            print(messages)

            # Send a message
            result = await session.call_tool("send_message", {
                "channel": "general",
                "body": "Hello from Python!"
            })
            print(result)
```

### Rust (using `willow-agent-sdk`)

```rust
use willow_agent_sdk::AgentClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = AgentClient::connect_sse(
        "http://127.0.0.1:9100",
        "wlw_a1b2c3d4e5f6...",
    ).await?;

    // Read current members
    let members = client.read_resource("willow://server/members").await?;

    // Subscribe to message events
    let mut events = client.notifications().await?;

    while let Some(event) = events.recv().await {
        if event.event_type == "MessageReceived" && !event.is_local {
            let messages = client
                .read_resource(&format!(
                    "willow://channel/{}/messages", event.channel
                ))
                .await?;

            if let Some(latest) = messages.last() {
                if latest.body.to_lowercase().contains("hello") {
                    client.call_tool("send_message", serde_json::json!({
                        "channel": event.channel,
                        "body": format!("Hey {}!", latest.author_name),
                    })).await?;
                }
            }
        }
    }
    Ok(())
}
```

### Claude Code (automatic via MCP config)

Once configured, Claude Code can use Willow tools directly:

> "Send a message in #general saying the deploy is complete"

Claude Code sees the `send_message` tool via MCP discovery and calls
it with `{ "channel": "general", "body": "Deploy complete." }`.

> "Summarize the last 20 messages in #dev"

Claude Code reads the `willow://channel/dev/messages` resource and
synthesizes a summary.

## Relationship to Worker Nodes

Workers and agents serve different purposes:

| | Worker Nodes | Agent Peers |
|---|---|---|
| **Purpose** | Infrastructure (sync, storage) | User-facing automation |
| **Protocol** | `WorkerRole` trait, bincode gossipsub | MCP over stdio/SSE/HTTP |
| **Identity** | Dedicated worker identity | Dedicated agent identity |
| **Consumers** | Other peers (automatic) | External processes (AI, scripts) |
| **Discovery** | `_willow_workers` heartbeats | MCP `tools/list` + `resources/list` |
| **API** | `WorkerRequest`/`WorkerResponse` | `ClientMutations` + `ClientViewHandle` via MCP |
| **Scaling** | Multiple per role | One agent process per identity |

An agent process could optionally also register as a worker (e.g., a
bot that provides search capabilities), but this is not required.

## Crate Structure

```
crates/agent/
├── Cargo.toml
├── tests/
│   └── e2e.rs         — 24 E2E integration tests
└── src/
    ├── main.rs        — CLI parsing, startup, shutdown
    ├── lib.rs         — Public module re-exports for tests
    ├── server.rs      — MCP server setup, stdio + HTTP transports
    ├── tools.rs       — 37 MCP tool definitions (ClientHandle methods)
    ├── resources.rs   — 15 MCP resource definitions (state accessors)
    ├── auth.rs        — Bearer token generation and validation
    ├── notifications.rs — 27 ClientEvent → MCP notification bridge
    └── scopes.rs      — Token scope definitions and enforcement

crates/agent-sdk/
├── Cargo.toml
└── src/
    ├── lib.rs         — AgentClient, connection management
    ├── tools.rs       — Typed tool call wrappers
    ├── resources.rs   — Typed resource read wrappers
    └── events.rs      — Notification stream types
```

### Dependencies

```toml
# crates/agent/Cargo.toml
[dependencies]
willow-client = { path = "../client" }
willow-identity = { path = "../identity" }
willow-network = { path = "../network" }
willow-actor = { path = "../actor" }
rmcp = { version = "1.3", features = ["server", "transport-io", "transport-streamable-http-server"] }
schemars = "1.0"
axum = "0.8"
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1"
rand = "0.9"
dirs = "6"
```

## Implementation Plan

### Phase 1: Core MCP Server (stdio)
- [ ] Create `crates/agent/` with CLI skeleton
- [ ] Implement MCP server with stdio transport
- [ ] Define MCP resources for read-only state (channels, members,
      messages, identity, connection status)
- [ ] Bearer token generation for non-stdio transports
- [ ] Basic integration test: spawn agent, call `tools/list`

### Phase 2: Tools + Notifications + E2E Harness
- [ ] Define MCP tools for all mutating `ClientHandle` methods
- [ ] Wire `ClientEvent`s to MCP notifications
- [ ] Resource subscription support (`resources/subscribe`)
- [ ] Token scoping (Full, ReadOnly, Messaging)
- [ ] Build `AgentTestHarness` — spawn relay + N agents for tests
- [ ] First MCP E2E test: multi-peer message delivery
- [ ] Integration test: call `send_message` tool, verify delivery

### Phase 3: SSE Transport + SDK + E2E Suite
- [ ] Add SSE transport alongside stdio
- [ ] Add Streamable HTTP transport
- [ ] Create `crates/agent-sdk/` with typed Rust client
- [ ] `--invite` and `--server` auto-join on startup
- [ ] Graceful shutdown (drain connections, save state)
- [ ] `just build-agent` / `just agent` commands in justfile
- [ ] `just test-agent-e2e` command for MCP E2E tests
- [ ] Add to `just dev` stack as optional participant
- [ ] Port key Playwright scenarios to MCP E2E tests (permissions,
      multi-peer sync, kick/rejoin, invite flows)
- [ ] Documentation with examples in Python, TypeScript, Rust

### Phase 4: Advanced Features (future)
- [ ] Webhook ingress (HTTP endpoint that maps webhooks → messages)
- [ ] Rate limiting per token
- [ ] Audit logging of all tool calls
- [ ] Multi-server support (switch_server via tool)
- [ ] File upload via tool (base64-encoded)
- [ ] Custom token scopes via config file
- [ ] MCP prompts for common workflows (summarize channel, onboard
      new member, review permissions)

## Security Considerations

1. **Local-only by default**: stdio requires no network listener. SSE
   and HTTP default to `127.0.0.1`. Exposing to `0.0.0.0` requires
   explicit `--bind 0.0.0.0:9100` and is strongly discouraged without
   TLS.
2. **Bearer tokens**: Generated with 256 bits of entropy via
   `rand::OsRng`. Prefixed with `wlw_` for easy identification in
   logs/configs. Only required for SSE/HTTP — stdio relies on process
   isolation.
3. **No privilege escalation**: Token scopes filter which tools and
   resources are visible. They can only restrict, never expand beyond
   the peer's network permissions.
4. **Identity isolation**: Agent uses its own identity key, separate
   from the user's main identity. Compromising the agent token doesn't
   compromise the user's identity.
5. **Rate limiting**: Phase 4 adds per-token rate limits to prevent
   abuse from compromised tokens.
6. **Token rotation**: Restarting the agent generates a new token
   (unless `--token` is pinned). Token files are created with 0600
   permissions.
7. **Tool visibility**: `ReadOnly` scoped tokens hide mutating tools
   from `tools/list` entirely. The AI agent cannot call what it cannot
   discover.

## Testing Strategy

### Unit & Integration Tests

| What | Type | Command |
|---|---|---|
| Tool definitions + schemas | Unit tests | `cargo test -p willow-agent` |
| Resource serialization | Unit tests | `cargo test -p willow-agent` |
| Token auth + scopes | Unit tests | `cargo test -p willow-agent` |
| Agent ↔ network (stdio) | Integration | `cargo test -p willow-agent --test integration` |
| SDK client methods | Unit tests | `cargo test -p willow-agent-sdk` |
| End-to-end agent | Integration | Start agent + relay, script calls tools |

### E2E Testing via MCP (UI-Free)

One of the biggest wins of the agent API is that it enables full
end-to-end testing of multi-peer behavior without a browser, DOM, or
UI framework. Today's Playwright E2E tests must navigate the Leptos web
UI to perform every action — clicking buttons, filling inputs, waiting
for DOM updates. This makes tests slow, brittle (selector changes break
them), and unable to test scenarios that aren't exposed in the UI.

The MCP API gives us a typed, deterministic interface to drive real
peers over the actual network. Tests become:

- **Faster** — no browser startup, no WASM compilation, no DOM rendering
- **More reliable** — no CSS selectors to break, no timing hacks
- **More expressive** — test permission edge cases, concurrent mutations,
  state divergence, and recovery scenarios that are hard to trigger via UI
- **Parallel** — spin up N agent processes cheaply vs. N browser contexts

#### Test Harness: `AgentTestHarness`

Two complementary approaches:

**1. In-process harness (fastest, for most tests)**

Uses `ClientHandle<MemNetwork>` directly — no child processes, no
real networking. The `MemNetwork` test double (already in
`willow-network`) simulates gossipsub in memory. Tests exercise the
full client stack (actors, views, mutations, persistence) without
process or network overhead.

```rust
/// In-process test peers using MemNetwork.
struct AgentTestHarness {
    peers: Vec<TestPeer>,
    system: SystemHandle,
}

struct TestPeer {
    client: ClientHandle<MemNetwork>,
    endpoint_id: EndpointId,
    /// Subscribe to the view system for assertions.
    views: ClientViewHandle,
    /// Drive mutations.
    mutations: ClientMutations<MemNetwork>,
}

impl AgentTestHarness {
    /// Create N in-process peers on a shared MemNetwork.
    /// First peer creates the server and invites the rest.
    async fn start(n: usize) -> Self { ... }

    async fn teardown(self) { ... }
}
```

**2. Process-spawning harness (for MCP protocol + real network tests)**

Spawns actual `willow-agent` binaries connected over iroh, and drives
them via MCP over stdio. Tests the full MCP serialization path and
real networking.

```rust
/// Spawns `willow-agent` processes and provides typed MCP clients.
struct McpTestHarness {
    relay: RelayHandle,
    agents: Vec<McpAgentHandle>,
}

struct McpAgentHandle {
    /// Typed MCP client (from willow-agent-sdk) connected over stdio.
    client: AgentClient,
    endpoint_id: EndpointId,
    process: Child,
}
```

Most tests should use the in-process harness (runs in ~5ms vs ~1-2s).
The MCP process harness is for integration tests that specifically
validate the MCP transport layer and real iroh networking.

#### Example: Multi-Peer Message Delivery (in-process)

```rust
#[tokio::test]
async fn messages_delivered_to_all_peers() {
    let harness = AgentTestHarness::start(3).await;
    let [alice, bob, carol] = &harness.peers[..] else { panic!() };

    // Alice sends a message via the mutations interface.
    alice.mutations.send_message("general", "hello everyone").await.unwrap();

    // Wait for gossipsub delivery via MemNetwork.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify via reactive views — no polling needed.
    let bob_msgs = bob.views.messages.get().await;
    assert!(bob_msgs.messages.iter().any(|m| m.body == "hello everyone"));

    let carol_msgs = carol.views.messages.get().await;
    assert!(carol_msgs.messages.iter().any(|m| m.body == "hello everyone"));

    harness.teardown().await;
}
```

#### Example: Permission Enforcement (in-process)

```rust
#[tokio::test]
async fn unprivileged_peer_cannot_create_channel() {
    let harness = AgentTestHarness::start(2).await;
    let [owner, guest] = &harness.peers[..] else { panic!() };

    // Guest (not trusted) tries to create a channel.
    let result = guest.mutations.create_channel("secret").await;

    // Should fail — guest lacks ManageChannels permission.
    assert!(result.is_err());

    // Verify channel was not created via owner's view.
    let channels = owner.views.channels.get().await;
    assert!(!channels.channels.iter().any(|c| c.name == "secret"));

    harness.teardown().await;
}
```

#### Example: State Convergence (in-process)

```rust
#[tokio::test]
async fn state_converges_after_concurrent_writes() {
    let harness = AgentTestHarness::start(2).await;
    let [alice, bob] = &harness.peers[..] else { panic!() };

    // Both peers send messages concurrently.
    let (a, b) = tokio::join!(
        alice.mutations.send_message("general", "from alice"),
        bob.mutations.send_message("general", "from bob"),
    );
    a.unwrap();
    b.unwrap();

    // Wait for sync to settle.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Both peers should see both messages via their views.
    let alice_msgs = alice.views.messages.get().await;
    let bob_msgs = bob.views.messages.get().await;

    assert_eq!(alice_msgs.messages.len(), bob_msgs.messages.len());
    assert!(alice_msgs.messages.iter().any(|m| m.body == "from bob"));
    assert!(bob_msgs.messages.iter().any(|m| m.body == "from alice"));

    // Verify state hashes agree.
    alice.mutations.verify_state().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (agreeing, total) = alice.client.state_hash_agreement().await;
    assert_eq!(agreeing, total);

    harness.teardown().await;
}
```

#### Example: MCP Protocol Test (process-spawning)

```rust
#[tokio::test]
async fn mcp_send_message_round_trip() {
    let harness = McpTestHarness::start(2).await;
    let [alice, bob] = &harness.agents[..] else { panic!() };

    // Drive via MCP tool calls — validates full serialization path.
    alice.client.call_tool("send_message", json!({
        "channel": "general",
        "body": "hello via MCP",
    })).await.unwrap();

    bob.client.wait_for_notification(|n| {
        n.event_type == "MessageReceived" && !n.is_local
    }).await;

    let bob_msgs = bob.client.read_resource(
        "willow://channel/general/messages"
    ).await.unwrap();
    assert_eq!(bob_msgs.last().unwrap().body, "hello via MCP");

    harness.teardown().await;
}
```

#### Scenarios Enabled by MCP E2E Tests

These are hard or impossible to test via UI but straightforward with
the agent API:

| Scenario | Why it's hard via UI |
|---|---|
| 3-way merge convergence | Need 3 browsers, precise timing |
| Permission escalation/de-escalation | Many UI clicks, hard to verify state |
| Kick + key rotation + rejoin | Multi-step flow across peers |
| Concurrent channel creation | Race conditions masked by UI debounce |
| 10+ peer message flood | 10 browser contexts is expensive |
| Offline peer recovery via relay | Can't simulate disconnect in browser |
| State hash mismatch detection | No UI surface for this at all |
| Role/permission matrix exhaustive | Combinatorial explosion of UI paths |
| Invite flow edge cases (expired, max uses) | Timing-sensitive, multi-peer |
| Worker authorization + sync | Workers have no UI |

#### Integration with Existing Test Tiers

MCP E2E tests sit between the existing client integration tests and
Playwright E2E tests:

| Tier | What it tests | Speed | Needs Network | Needs UI |
|---|---|---|---|---|
| State tests | Pure event logic | ~1ms/test | No | No |
| Client tests | Client API methods | ~5ms/test | No | No |
| **In-process E2E** | **Multi-peer via MemNetwork** | **~5-50ms/test** | **No (MemNetwork)** | **No** |
| **MCP E2E tests** | **MCP protocol + real iroh** | **~1-2s/test** | **Yes (localhost)** | **No** |
| Playwright E2E | Full UI + network | ~10-30s/test | Yes | Yes (browser) |

The in-process harness should be the default for most multi-peer tests.
It exercises the full actor stack (all 6 domain actors, derived views,
mutations, persistence, event broker) without process spawning or real
networking. MCP E2E tests validate the MCP serialization layer and
real iroh transport. Playwright tests focus purely on UI rendering and
interaction (click targets, responsive layout, visual state).

#### Justfile Commands

```
just test-agent       # unit + integration tests for crates/agent
just test-agent-e2e   # MCP-based multi-peer E2E tests
just test-all         # includes test-agent and test-agent-e2e
```

## Open Questions

1. **Should the agent binary support running multiple agent identities
   in one process?** Current design: one identity per process. Multiple
   agents = multiple processes. Simpler, better isolation.

2. **Should agents be able to impersonate the user's identity (act on
   behalf of) instead of having their own?** Current design: agents
   always have their own identity. This is safer and more auditable.
   "Delegate" mode could be explored later with explicit consent.

3. **MCP Prompts**: Should we expose canned prompt templates (e.g.,
   "summarize channel", "review permissions", "draft announcement")?
   These are optional MCP primitives that guide AI behavior. Worth
   adding in Phase 4 once we see real usage patterns.

4. **MCP Sampling**: MCP supports servers requesting LLM completions
   from the client (`sampling/createMessage`). This could let the
   Willow agent ask the AI for help (e.g., auto-moderate by asking the
   AI to evaluate a message). Defer until clear use case emerges.
