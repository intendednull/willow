# Agentic Peer API Design Spec

**Date**: 2026-03-29
**Status**: Draft

## Overview

Expose the `ClientHandle` API to external agents (AI assistants, bots,
automation scripts) via a local JSON-RPC server embedded in a new
`willow-agent` binary. Agents connect over a Unix socket or TCP
localhost, authenticate with a bearer token, and issue commands against
the full peer client API. The agent binary is a first-class Willow peer
— same identity, same permissions, same gossipsub participation.

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
   talk to the local `willow-agent` process over JSON-RPC; that process
   handles all networking via the existing `ClientHandle`.
3. **Minimal surface**: The JSON-RPC API maps 1:1 to `ClientHandle`
   methods. No new abstractions — if the client can do it, the agent
   can do it.
4. **Event streaming**: Agents subscribe to `ClientEvent`s via a
   persistent JSON-RPC notification stream (WebSocket or SSE), enabling
   reactive behavior.
5. **Local-only by default**: The RPC server binds to `127.0.0.1` or a
   Unix socket. No remote access without explicit configuration.

## Architecture

```
┌─────────────┐   JSON-RPC    ┌──────────────────┐   gossipsub   ┌───────────┐
│  AI Agent   │──────────────▶│  willow-agent    │◀────────────▶│  Willow   │
│  (any lang) │  unix socket  │                  │   libp2p      │  Network  │
│             │◀──────────────│  ClientHandle    │               │           │
│             │  events/SSE   │  + RPC server    │               │           │
└─────────────┘               └──────────────────┘               └───────────┘
```

### Components

**1. `willow-agent` binary** (`crates/agent/`)
- Owns a `ClientHandle` + `ClientEventLoop`
- Runs a JSON-RPC 2.0 server (via `jsonrpsee`)
- Accepts connections on Unix socket (`/tmp/willow-agent.sock`) or
  TCP (`127.0.0.1:9100`)
- Authenticates requests with a bearer token (generated on startup,
  printed to stdout, optionally written to a file)
- Forwards JSON-RPC calls to `ClientHandle` methods
- Streams `ClientEvent`s as JSON-RPC notifications over WebSocket

**2. `willow-agent-sdk`** (`crates/agent-sdk/`)
- Optional Rust client library for connecting to the agent RPC
- Typed wrappers around JSON-RPC calls
- Async event stream via `tokio::sync::broadcast`
- Other languages connect directly via JSON-RPC (Python, TypeScript,
  Go — any language with a JSON-RPC client)

**3. Permission scoping** (bearer token + server-side filtering)
- Each bearer token is scoped to a set of allowed operations
- Default token: full access (matches the peer's permissions)
- Restricted tokens: read-only, messaging-only, admin-only
- Operations that exceed the token scope return JSON-RPC error -32001

## JSON-RPC API

### Transport

- **WebSocket**: `ws://127.0.0.1:9100/ws` or `ws+unix:///tmp/willow-agent.sock`
- **HTTP POST**: `http://127.0.0.1:9100/rpc` (stateless calls only)
- Authentication: `Authorization: Bearer <token>` header

### Methods

Every `ClientHandle` public method maps to a JSON-RPC method. Method
names use snake_case matching the Rust API.

#### Server Management

```json
{"jsonrpc":"2.0","method":"create_server","params":{"name":"My Server"},"id":1}
{"jsonrpc":"2.0","method":"switch_server","params":{"server_id":"..."},"id":2}
{"jsonrpc":"2.0","method":"server_list","id":3}
{"jsonrpc":"2.0","method":"leave_server","params":{"server_id":"..."},"id":4}
{"jsonrpc":"2.0","method":"rename_server","params":{"new_name":"..."},"id":5}
```

#### Messaging

```json
{"jsonrpc":"2.0","method":"send_message","params":{"channel":"general","body":"hello"},"id":1}
{"jsonrpc":"2.0","method":"send_reply","params":{"channel":"general","parent_id":"msg-uuid","body":"reply"},"id":2}
{"jsonrpc":"2.0","method":"edit_message","params":{"channel":"general","message_id":"...","new_body":"edited"},"id":3}
{"jsonrpc":"2.0","method":"delete_message","params":{"channel":"general","message_id":"..."},"id":4}
{"jsonrpc":"2.0","method":"messages","params":{"channel":"general"},"id":5}
{"jsonrpc":"2.0","method":"react","params":{"channel":"general","message_id":"...","emoji":"👍"},"id":6}
{"jsonrpc":"2.0","method":"pin_message","params":{"channel":"general","message_id":"..."},"id":7}
```

#### Channels

```json
{"jsonrpc":"2.0","method":"create_channel","params":{"name":"dev"},"id":1}
{"jsonrpc":"2.0","method":"delete_channel","params":{"name":"dev"},"id":2}
{"jsonrpc":"2.0","method":"channels","id":3}
{"jsonrpc":"2.0","method":"switch_channel","params":{"name":"dev"},"id":4}
```

#### Permissions & Members

```json
{"jsonrpc":"2.0","method":"trust_peer","params":{"peer_id":"12D3KooW..."},"id":1}
{"jsonrpc":"2.0","method":"kick_member","params":{"peer_id":"12D3KooW..."},"id":2}
{"jsonrpc":"2.0","method":"create_role","params":{"name":"moderator"},"id":3}
{"jsonrpc":"2.0","method":"set_permission","params":{"role_id":"...","permission":"KickMembers","granted":true},"id":4}
{"jsonrpc":"2.0","method":"server_members","id":5}
{"jsonrpc":"2.0","method":"has_permission","params":{"peer_id":"...","permission":"ManageChannels"},"id":6}
```

#### State & Identity

```json
{"jsonrpc":"2.0","method":"peer_id","id":1}
{"jsonrpc":"2.0","method":"display_name","id":2}
{"jsonrpc":"2.0","method":"set_display_name","params":{"name":"AgentBot"},"id":3}
{"jsonrpc":"2.0","method":"is_connected","id":4}
{"jsonrpc":"2.0","method":"peers","id":5}
{"jsonrpc":"2.0","method":"verify_state","id":6}
```

#### Invites

```json
{"jsonrpc":"2.0","method":"generate_invite","params":{"recipient_peer_id":"..."},"id":1}
{"jsonrpc":"2.0","method":"accept_invite","params":{"code":"..."},"id":2}
{"jsonrpc":"2.0","method":"create_join_link","params":{"max_uses":10},"id":3}
```

### Event Notifications (WebSocket only)

After connecting via WebSocket, agents receive real-time notifications:

```json
{"jsonrpc":"2.0","method":"event","params":{"type":"MessageReceived","channel":"general","message_id":"...","is_local":false}}
{"jsonrpc":"2.0","method":"event","params":{"type":"PeerConnected","peer_id":"12D3KooW..."}}
{"jsonrpc":"2.0","method":"event","params":{"type":"MemberKicked","peer_id":"12D3KooW..."}}
{"jsonrpc":"2.0","method":"event","params":{"type":"ChannelCreated","name":"announcements"}}
```

Agents can filter events:

```json
{"jsonrpc":"2.0","method":"subscribe","params":{"events":["MessageReceived","PeerConnected"]},"id":1}
{"jsonrpc":"2.0","method":"unsubscribe","params":{"events":["PeerConnected"]},"id":2}
```

## `willow-agent` Binary

### CLI Interface

```
willow-agent [OPTIONS]

Options:
  --relay <MULTIADDR>       Relay address (required)
  --name <NAME>             Display name [default: "Agent"]
  --server <ID>             Auto-join server by ID
  --invite <CODE>           Accept invite on startup
  --bind <ADDR>             RPC bind address [default: 127.0.0.1:9100]
  --socket <PATH>           Unix socket path [default: /tmp/willow-agent.sock]
  --token <TOKEN>           Fixed bearer token (generated if omitted)
  --token-file <PATH>       Write token to file for other processes
  --identity <PATH>         Identity key path [default: ~/.willow/agent-identity]
  --persist                 Enable persistent storage
  --log-level <LEVEL>       Log verbosity [default: info]
```

### Startup Flow

1. Load or generate Ed25519 identity
2. Create `ClientHandle` with config
3. Connect to relay
4. If `--invite`, accept it; if `--server`, switch to it
5. Generate bearer token, print to stdout and optionally write to file
6. Start JSON-RPC server on socket + TCP
7. Spawn `ClientEventLoop`, forward events to WebSocket subscribers
8. Block until SIGTERM/SIGINT

### Token Management

```
$ willow-agent --relay /ip4/1.2.3.4/tcp/9091/ws --name "BuildBot"
Agent peer ID: 12D3KooWAbc...
RPC listening on: 127.0.0.1:9100
RPC socket: /tmp/willow-agent.sock
Bearer token: wlw_a1b2c3d4e5f6...

# Other processes use the token:
curl -X POST http://127.0.0.1:9100/rpc \
  -H "Authorization: Bearer wlw_a1b2c3d4e5f6..." \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"send_message","params":{"channel":"general","body":"Build #42 passed"},"id":1}'
```

## Permission Model

Agents are regular peers. Their capabilities are determined by:

1. **Network-level permissions**: What the server owner grants to the
   agent's peer ID (via `trust_peer`, `assign_role`, `set_permission`)
2. **Token-level scoping**: The bearer token can further restrict what
   the agent process allows through the RPC

### Token Scopes

```rust
enum TokenScope {
    /// Full access — anything the peer's permissions allow
    Full,
    /// Read-only — messages, channels, members, state queries
    ReadOnly,
    /// Messaging — send/edit/delete messages, reactions, typing
    Messaging,
    /// Admin — full access including permission management
    Admin,
    /// Custom — explicit allowlist of method names
    Custom(HashSet<String>),
}
```

Token scopes are enforced server-side in the RPC layer. They cannot
grant more than the peer's network permissions — they can only restrict.

### Trust Setup

Server owners trust an agent the same way they trust any peer:

1. Agent starts and connects to the network
2. Owner sees agent's peer ID in the member list
3. Owner runs `trust_peer` or assigns a role with specific permissions
4. Agent can now perform operations matching its permissions

No special trust model. No backdoors. The agent is just a peer.

## Event-Driven Agent Pattern

The primary use case is reactive agents that respond to events:

```python
# Python example using any JSON-RPC WebSocket client
import asyncio
import websockets
import json

async def main():
    uri = "ws://127.0.0.1:9100/ws"
    headers = {"Authorization": "Bearer wlw_a1b2c3d4e5f6..."}

    async with websockets.connect(uri, extra_headers=headers) as ws:
        # Subscribe to message events
        await ws.send(json.dumps({
            "jsonrpc": "2.0", "method": "subscribe",
            "params": {"events": ["MessageReceived"]}, "id": 1
        }))

        async for raw in ws:
            msg = json.loads(raw)
            if msg.get("method") == "event":
                params = msg["params"]
                if params["type"] == "MessageReceived" and not params["is_local"]:
                    # Fetch the message content
                    await ws.send(json.dumps({
                        "jsonrpc": "2.0", "method": "messages",
                        "params": {"channel": params["channel"]}, "id": 2
                    }))
                    resp = json.loads(await ws.recv())
                    messages = resp["result"]
                    latest = messages[-1]

                    # Auto-respond to greetings
                    if "hello" in latest["body"].lower():
                        await ws.send(json.dumps({
                            "jsonrpc": "2.0", "method": "send_message",
                            "params": {
                                "channel": params["channel"],
                                "body": f"Hey {latest['author_name']}! 👋"
                            }, "id": 3
                        }))

asyncio.run(main())
```

```rust
// Rust example using willow-agent-sdk
use willow_agent_sdk::AgentClient;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = AgentClient::connect_unix(
        "/tmp/willow-agent.sock",
        "wlw_a1b2c3d4e5f6...",
    ).await?;

    let mut events = client.subscribe(vec!["MessageReceived"]).await?;

    while let Some(event) = events.recv().await {
        if let AgentEvent::MessageReceived { channel, message_id, is_local: false } = event {
            let messages = client.messages(&channel).await?;
            if let Some(latest) = messages.last() {
                if latest.body.to_lowercase().contains("hello") {
                    client.send_message(&channel, &format!(
                        "Hey {}! 👋", latest.author_name
                    )).await?;
                }
            }
        }
    }
    Ok(())
}
```

## Relationship to Worker Nodes

Workers and agents serve different purposes:

| | Worker Nodes | Agent Peers |
|---|---|---|
| **Purpose** | Infrastructure (sync, storage) | User-facing automation |
| **Protocol** | `WorkerRole` trait, bincode gossipsub | JSON-RPC over local socket |
| **Identity** | Dedicated worker identity | Dedicated agent identity |
| **Consumers** | Other peers (automatic) | External processes (scripts, AI) |
| **Discovery** | `_willow_workers` heartbeats | Local socket, not discovered |
| **API** | `WorkerRequest`/`WorkerResponse` | Full `ClientHandle` methods |
| **Scaling** | Multiple per role | One agent process per identity |

An agent process could optionally also register as a worker (e.g., a
bot that provides search capabilities), but this is not required.

## Crate Structure

```
crates/agent/
├── Cargo.toml
└── src/
    ├── main.rs        — CLI parsing, startup, shutdown
    ├── rpc.rs         — JSON-RPC method handlers
    ├── auth.rs        — Bearer token generation and validation
    ├── events.rs      — ClientEvent → JSON-RPC notification bridge
    └── scopes.rs      — Token scope definitions and enforcement

crates/agent-sdk/
├── Cargo.toml
└── src/
    ├── lib.rs         — AgentClient, connection management
    ├── methods.rs     — Typed method wrappers
    └── events.rs      — Event stream types
```

### Dependencies

```toml
# crates/agent/Cargo.toml
[dependencies]
willow-client = { path = "../client" }
willow-identity = { path = "../identity" }
jsonrpsee = { version = "0.24", features = ["server", "ws-server"] }
tokio = { version = "1", features = ["full"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
anyhow = "1"
rand = "0.8"
```

## Implementation Plan

### Phase 1: Core RPC Server
- [ ] Create `crates/agent/` with CLI skeleton
- [ ] Implement JSON-RPC server with `jsonrpsee`
- [ ] Map read-only `ClientHandle` methods (messages, channels, members,
      peers, state queries)
- [ ] Bearer token generation and auth middleware
- [ ] Basic integration test: start agent, query via JSON-RPC

### Phase 2: Write Operations + Events
- [ ] Map mutating methods (send_message, create_channel, etc.)
- [ ] WebSocket event streaming (ClientEvent → JSON-RPC notifications)
- [ ] Event subscription filtering (subscribe/unsubscribe)
- [ ] Token scoping (Full, ReadOnly, Messaging)
- [ ] Integration test: send message via RPC, verify receipt

### Phase 3: SDK + Polish
- [ ] Create `crates/agent-sdk/` with typed Rust client
- [ ] Unix socket support alongside TCP
- [ ] `--invite` and `--server` auto-join on startup
- [ ] Graceful shutdown (drain connections, save state)
- [ ] `just build-agent` / `just agent` commands in justfile
- [ ] Add to `just dev` stack as optional participant
- [ ] Documentation with examples in Python, TypeScript, Rust

### Phase 4: Advanced Features (future)
- [ ] Webhook ingress (HTTP endpoint that maps webhooks → messages)
- [ ] Rate limiting per token
- [ ] Audit logging of all RPC calls
- [ ] Multi-server support (switch_server via RPC)
- [ ] File upload via RPC (multipart or base64)
- [ ] Custom token scopes via config file

## Security Considerations

1. **Local-only binding**: Default `127.0.0.1` prevents remote access.
   Exposing to `0.0.0.0` requires explicit `--bind 0.0.0.0:9100` flag
   and is strongly discouraged without TLS.
2. **Bearer tokens**: Generated with 256 bits of entropy via
   `rand::OsRng`. Prefixed with `wlw_` for easy identification in
   logs/configs.
3. **No privilege escalation**: Token scopes can only restrict, never
   expand beyond the peer's network permissions.
4. **Identity isolation**: Agent uses its own identity key, separate
   from the user's main identity. Compromising the agent token doesn't
   compromise the user's identity.
5. **Rate limiting**: Phase 4 adds per-token rate limits to prevent
   abuse from compromised tokens.
6. **Token rotation**: Restarting the agent generates a new token
   (unless `--token` is pinned). Token files are created with 0600
   permissions.

## Testing Strategy

| What | Type | Command |
|---|---|---|
| RPC method mapping | Unit tests | `cargo test -p willow-agent` |
| Token auth + scopes | Unit tests | `cargo test -p willow-agent` |
| Agent ↔ network | Integration | `cargo test -p willow-agent --test integration` |
| SDK client methods | Unit tests | `cargo test -p willow-agent-sdk` |
| End-to-end agent | Integration | Start agent + relay, script sends via RPC |

## Open Questions

1. **Should the agent binary support running multiple agent identities
   in one process?** Current design: one identity per process. Multiple
   agents = multiple processes. Simpler, better isolation.

2. **Should we support SSE as an alternative to WebSocket for event
   streaming?** WebSocket is more capable (bidirectional), but SSE is
   simpler for read-only event consumers. Could add in Phase 4.

3. **Should agents be able to impersonate the user's identity (act on
   behalf of) instead of having their own?** Current design: agents
   always have their own identity. This is safer and more auditable.
   "Delegate" mode could be explored later with explicit consent.

4. **gRPC vs JSON-RPC?** JSON-RPC is simpler, works over plain
   WebSocket, needs no code generation, and is accessible from any
   language with an HTTP client. gRPC has better streaming and types
   but adds complexity. JSON-RPC wins for accessibility.
