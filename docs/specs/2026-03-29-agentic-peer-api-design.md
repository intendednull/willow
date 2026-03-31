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
the `willow-actor` system. `ClientHandle<N>` is generic over the
`Network` trait and communicates with a `ClientStateActor` for all state
reads and mutations. All `ClientHandle` methods are async — they send
messages to the actor and await responses. This maps naturally to the
MCP request/response model.

Peer identities use `EndpointId` (an Ed25519 public key from iroh),
which displays as a 64-character hex string. All tool parameters and
resource fields that reference peers use this hex format.

### Components

**1. `willow-agent` binary** (`crates/agent/`)
- Owns a `ClientHandle<IrohNetwork>` connected to the actor system
- Runs an MCP server supporting all three transports:
  - **stdio** (default) — AI clients spawn the binary directly
  - **SSE** — `http://127.0.0.1:9100/sse` for network clients
  - **Streamable HTTP** — `http://127.0.0.1:9100/mcp` for stateless
    HTTP clients with optional session upgrade
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
| **SSE** | Long-lived connections from scripts/bots | Bearer token header |
| **Streamable HTTP** | Stateless calls, language-agnostic | Bearer token header |

### Tools

Every mutating `ClientHandle` method maps to an MCP tool. Tools are
discoverable via `tools/list` and include full JSON Schema for params.

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

Read-only state accessors are exposed as MCP resources. AI agents can
read these via `resources/read` without needing to call tools.

All `peer_id` and `author` fields in resource responses are
`EndpointId` values — 64-character hex strings representing Ed25519
public keys.

#### Static Resources (always available)

| URI | Description | Returns |
|---|---|---|
| `willow://identity` | Agent's endpoint ID and display name | `{ peer_id, display_name }` |
| `willow://connection` | Network connection status | `{ connected, peer_count, peers }` |
| `willow://servers` | List of joined servers | `[{ id, name }]` |

#### Dynamic Resources (per active server)

| URI Template | Description | Returns |
|---|---|---|
| `willow://server/current` | Active server info | `{ id, name, owner, description }` |
| `willow://server/channels` | All channels with type | `[{ name, kind }]` |
| `willow://server/members` | All members with status | `[{ peer_id, display_name, is_online }]` |
| `willow://server/roles` | Roles with permissions | `[{ role_id, name, permissions }]` |
| `willow://channel/{name}/messages` | Messages in channel | `[{ id, author, body, timestamp, edited, reply_to, reactions }]` |
| `willow://channel/{name}/pins` | Pinned messages | `[{ id, author, body }]` |
| `willow://server/unread` | Unread counts per channel | `{ channel: count }` |
| `willow://server/join-links` | Active join links | `[{ id, max_uses, uses }]` |
| `willow://server/state-agreement` | State hash consensus | `{ agreeing, total }` |
| `willow://voice/status` | Voice channel state | `{ active_channel, muted, deafened }` |
| `willow://voice/{channel}/participants` | Voice participants | `[{ peer_id }]` |

Resources support MCP's `resources/subscribe` for change notifications.
When underlying state changes (new message, member joins, channel
created), the server emits `notifications/resources/updated` so agents
can re-read the resource.

### Notifications (Server → Client)

`ClientEvent`s are forwarded as MCP notifications. Agents receive these
automatically on stdio/SSE transports.

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
| `VoiceJoined` | `channel_id`, `peer_id` |
| `VoiceLeft` | `channel_id`, `peer_id` |
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
  --transport <MODE>        MCP transport: stdio | sse | http [default: stdio]
  --bind <ADDR>             SSE/HTTP bind address [default: 127.0.0.1:9100]
  --token <TOKEN>           Fixed bearer token (generated if omitted)
  --token-file <PATH>       Write token to file for other processes
  --identity <PATH>         Identity key path [default: ~/.willow/agent-identity]
  --persist                 Enable persistent storage
  --log-level <LEVEL>       Log verbosity [default: info]
```

### Startup Flow

1. Load or generate Ed25519 identity
2. Start `willow-actor` system
3. Create `ClientHandle<IrohNetwork>` with config (spawns
   `ClientStateActor`)
4. Call `client.connect(network)` — starts iroh node, subscribes to
   gossipsub topics, spawns topic listener tasks
5. If `--invite`, accept it; if `--server`, switch to it
6. Start MCP server on the selected transport:
   - **stdio**: read JSON-RPC from stdin, write to stdout (default)
   - **sse**: generate bearer token, start HTTP server with SSE endpoint
   - **http**: generate bearer token, start Streamable HTTP endpoint
7. Forward `ClientEvent`s from the event channel as MCP notifications
8. Block until stdin closes (stdio) or SIGTERM/SIGINT (sse/http)

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

### SSE Mode for Scripts/Bots

```
$ willow-agent --relay /ip4/1.2.3.4/tcp/9091/ws --name "BuildBot" --transport sse
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
| **API** | `WorkerRequest`/`WorkerResponse` | Full `ClientHandle` via MCP |
| **Scaling** | Multiple per role | One agent process per identity |

An agent process could optionally also register as a worker (e.g., a
bot that provides search capabilities), but this is not required.

## Crate Structure

```
crates/agent/
├── Cargo.toml
└── src/
    ├── main.rs        — CLI parsing, startup, shutdown
    ├── tools.rs       — MCP tool definitions (ClientHandle methods)
    ├── resources.rs   — MCP resource definitions (state accessors)
    ├── auth.rs        — Bearer token generation and validation
    ├── notifications.rs — ClientEvent → MCP notification bridge
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
rmcp = { version = "0.1", features = ["server", "transport-sse", "transport-io"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1"
rand = "0.8"
```

## Implementation Plan

### Phase 1: Core MCP Server (stdio)
- [ ] Create `crates/agent/` with CLI skeleton
- [ ] Implement MCP server with stdio transport
- [ ] Define MCP resources for read-only state (channels, members,
      messages, identity, connection status)
- [ ] Bearer token generation for non-stdio transports
- [ ] Basic integration test: spawn agent, call `tools/list`

### Phase 2: Tools + Notifications
- [ ] Define MCP tools for all mutating `ClientHandle` methods
- [ ] Wire `ClientEvent`s to MCP notifications
- [ ] Resource subscription support (`resources/subscribe`)
- [ ] Token scoping (Full, ReadOnly, Messaging)
- [ ] Integration test: call `send_message` tool, verify delivery

### Phase 3: SSE Transport + SDK
- [ ] Add SSE transport alongside stdio
- [ ] Add Streamable HTTP transport
- [ ] Create `crates/agent-sdk/` with typed Rust client
- [ ] `--invite` and `--server` auto-join on startup
- [ ] Graceful shutdown (drain connections, save state)
- [ ] `just build-agent` / `just agent` commands in justfile
- [ ] Add to `just dev` stack as optional participant
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

| What | Type | Command |
|---|---|---|
| Tool definitions + schemas | Unit tests | `cargo test -p willow-agent` |
| Resource serialization | Unit tests | `cargo test -p willow-agent` |
| Token auth + scopes | Unit tests | `cargo test -p willow-agent` |
| Agent ↔ network (stdio) | Integration | `cargo test -p willow-agent --test integration` |
| SDK client methods | Unit tests | `cargo test -p willow-agent-sdk` |
| End-to-end agent | Integration | Start agent + relay, script calls tools |

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
