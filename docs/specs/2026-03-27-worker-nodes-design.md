# Worker Nodes Design Spec

**Date**: 2026-03-27
**Status**: Approved

## Overview

Willow's relay currently handles both network plumbing (TCP↔WS bridging,
NAT traversal) and state storage/replay (SQLite event store, SyncRequest
handling). This design separates those concerns by introducing **worker
nodes** — specialized peers that behave like any other peer in the network
but perform specific infrastructure jobs.

Worker nodes use the same protocols, identity system, and permission model
as regular peers. They require no special allowances — server owners
authorize them via `GrantPermission` like any other peer.

## Goals

- Always-available state replay for peers coming online
- Deep archival history beyond what any single peer holds
- Fault isolation: one worker crashing doesn't affect others
- No special trust model — workers are just peers with jobs
- Scalable architecture with stubs for per-server management

## Non-Goals (for initial version)

- Per-server worker allocation UI (stubs only)
- Dynamic worker scaling
- Worker health monitoring dashboard
- Future worker types (see below) — not implemented, but the architecture
  must accommodate them

## Prior Art

This design's central bet — always-on infrastructure nodes that are *just peers*, earning trust through the normal permission system rather than holding wire-level authority — has direct precedent across P2P systems. Each row notes what Willow's worker model borrows or where it diverges.

| System | Relevance to worker nodes |
|---|---|
| **Nostr relays** (NIP-01) | Always-on, untrusted event stores: relays verify signatures and replay events to subscribers but hold no authority over content. Closely mirrors Willow workers — but Nostr relays *replace* P2P (clients only ever talk to relays), whereas Willow workers are optional accelerators layered over real peer-to-peer gossip. |
| **Negentropy / range-based set reconciliation** (hoytech, NIP-77; underlying RBSR algorithm: Aljoscha Meyer, arXiv:2212.13567, Dec 2022 / SRDS 2023) | The catch-up problem Willow solves with `HeadsSummary` + event-replay. Negentropy reconciles two event sets via recursive range fingerprints. Willow deliberately reuses its existing gossip + heads-summary sync path for worker convergence instead of adding a dedicated reconciliation wire protocol — same goal (find the missing events), lighter mechanism. |
| **Secure Scuttlebutt pubs** (ssbc) | Always-online peers in a signed-append-log gossip network that follow/replicate feeds for availability and NAT traversal — the archetype for "an always-on box that is still just a peer." (SSB *rooms*, by contrast, only broker tunneled connections without replicating feeds.) Willow's replay node is the pub analogue (re-serves state); a pub gains no special trust, exactly like a worker pre-`GrantPermission`. |
| **Matrix homeservers** (spec.matrix.org, server-server federation) | The privileged-server model Willow explicitly rejects. A homeserver is authoritative for its users and is part of the trust boundary; losing it loses the account. Willow workers carry no authority — owners grant `SyncProvider` to a worker the same way they would any peer, and revoke it the same way. Contrast, not adoption. |
| **IPFS pinning services** (Pinata, Infura) | Durable hosting of otherwise-ephemeral content-addressed data: dedicated always-on nodes pin content so the network doesn't garbage-collect it. Directly analogous to Willow's storage node, which archives every event indefinitely so history survives beyond any single peer's local cache. |
| **Hypercore / Dat** (holepunchto / Dat Foundation) | Signed append-only logs replicated peer-to-peer, with always-on "pinning" peers that re-host data for availability. Matches Willow's per-author signed event chains plus storage-node re-hosting; differs in that Willow merges many authors' chains into one materialized `ServerState` rather than syncing independent logs. |
| **Dynamo** (DeCandia et al., SOSP 2007) | Hinted handoff + anti-entropy (Merkle-tree) replica reconciliation: the lineage for "durable replicas that actively converge rather than passively receive." Willow's active-sync loop (workers broadcast `SyncRequest` to backfill anything gossip dropped) is the same idea — periodic anti-entropy on top of best-effort delivery — at a smaller scale. |

## Future Worker Types

The `WorkerRole` trait and `RoleType` enum are designed to be extended.
These are anticipated worker types beyond the initial Replay and Storage
nodes:

- **File Node** — Handles large file sharing and content-addressed chunk
  storage/retrieval (building on `willow-files`). Peers upload chunks to
  file nodes instead of flooding gossipsub. File nodes serve chunks on
  demand, acting as always-available seeders.
- **Stream Node** — Optimizes voice/video call routing. Instead of
  full-mesh WebRTC between N peers, a stream node acts as an SFU
  (Selective Forwarding Unit), receiving one stream per sender and
  redistributing to receivers. Reduces per-peer bandwidth from O(N) to
  O(1).
- **Bot Node** — Runs user-defined automation: custom commands, webhook
  integrations, moderation bots, notification bridges (e.g., GitHub →
  channel). Bots are just workers with a scripting/plugin interface.
  They receive events like any peer and can emit messages/reactions.
- **Search Node** — Full-text indexing of message history. Receives
  events from gossipsub, builds a search index (e.g., tantivy), and
  responds to search queries with ranked results.
- **Bridge Node** — Bridges messages between Willow servers and external
  platforms (Matrix, IRC, Slack). Translates protocols bidirectionally.

Each future type would be a new binary (`willow-files-worker`,
`willow-stream`, `willow-bot`, etc.) implementing `WorkerRole`. The
discovery protocol, permission model, and deployment infrastructure
remain unchanged — they're just peers with different jobs.

## Relay Scaling

Since this design strips the relay back to pure network plumbing, it
becomes lightweight enough to run multiple instances for redundancy and
geographic distribution.

### Current State

- `NetworkConfig.bootstrap_peers` is already a `Vec` — the network
  layer can dial multiple relays today
- `with_relay()` is chainable — adding relays to config is trivial
- **Gap**: The client UI (`settings.relay_addr`) and storage layer
  (`NetworkSettings.relay_addr`) only support a single relay address

### Multi-Relay Architecture (Future)

Multiple relays form a mesh of network plumbing. Each relay is
independent — no coordination between them is needed. Peers connect to
whichever relay(s) they can reach; gossipsub handles message propagation
across the mesh.

```
┌──────────┐     gossipsub     ┌──────────┐
│ Relay A  │◄────────────────►│ Relay B  │
│ (US-East)│                   │ (EU-West)│
└────┬─────┘                   └────┬─────┘
     │                              │
  clients                        clients
```

**Benefits:**
- **Redundancy** — If one relay goes down, peers reconnect to another
- **Geographic distribution** — Lower latency for global users
- **Load distribution** — Peers spread across relays naturally

### Changes Needed for Multi-Relay

These are scoped as future work but the initial implementation should
not make them harder:

1. **Config layer** — `NetworkSettings.relay_addr` becomes
   `relay_addrs: Vec<String>`. The UI settings field accepts
   comma-separated addresses or a list input.
2. **Client startup** — Call `with_relay()` for each address in the
   list. The network layer already handles this.
3. **Reconnection failover** — On disconnect, try the next relay in the
   list before backing off. The WASM reconnection loop already has
   backoff; it just needs relay rotation added.
4. **Relay discovery** — Relays could advertise each other's addresses
   via Kademlia, so clients that connect to one relay automatically
   learn about others.

### What This Means for the Current Work

When stripping state storage from the relay:
- Keep the relay's network code clean and stateless so it's trivial to
  run N instances
- Don't introduce any relay-local state that would break with multiple
  relays (the event store removal already achieves this)
- Workers connect to relay(s) the same way clients do — via
  `bootstrap_peers` config — so multi-relay works for workers too

## Architecture

### Crate Structure

```
crates/
├── worker/    — Shared library: peer lifecycle, heartbeat, WorkerRole trait
├── replay/    — Binary: fast bounded-memory state replay
├── storage/   — Binary: archival disk-backed history
├── relay/     — Stripped back to pure network plumbing
```

### Relay Changes

The relay loses its event storage and sync handling responsibilities:

**Removed from relay:**
- SQLite `EventStore` and all event persistence
- `SyncRequest` / `SyncBatch` handling
- `events_for_topic_since_hash()` query logic

**Relay retains:**
- TCP and WebSocket listeners
- libp2p relay protocol (NAT traversal)
- GossipSub message routing (pass-through)
- Kademlia peer discovery
- Identify protocol

The relay becomes stateless network infrastructure.

## Worker Library (`willow-worker`)

### WorkerRole Trait

```rust
pub trait WorkerRole: Send + 'static {
    /// Returns combined role identity and capacity info for heartbeats.
    fn role_info(&self) -> WorkerRoleInfo;

    /// Called when an event is received from gossipsub.
    /// The state actor calls this sequentially — safe to use &mut self.
    fn on_event(&mut self, event: &Event);

    /// Handle an incoming request from a client peer.
    /// Called by the state actor — has exclusive access to state.
    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse;
}

pub enum RoleType {
    Replay,
    Storage,
    // Future: File, Stream, Bot, Search, Bridge
    // Adding a variant here + a new binary is all that's needed
    // to introduce a new worker type.
}
```

### Concurrency Model

Workers are **multi-threaded** and use an **actor system** for
cross-thread synchronization. No shared locks — all state access goes
through message passing.

Each worker runs four actor loops on the tokio multi-threaded runtime:

1. **Network actor** — Owns the libp2p swarm. Receives gossipsub
   events, dispatches them to the state actor. Receives outbound
   messages (heartbeats, responses, sync requests) from other actors
   and publishes them to gossipsub. Runs on a single task (libp2p
   swarm is single-owner).

2. **State actor** — Owns all mutable state (in-memory `ServerState`
   for replay nodes, SQLite connection for storage nodes). Receives
   events and requests via `tokio::sync::mpsc` channels. Processes
   them sequentially — no contention, no locks. Sends responses back
   via a reply channel included in each request message.

3. **Heartbeat actor** — Simple timer loop. Queries the state actor
   for capacity info, sends announcements through the network actor.

4. **Sync actor** — Periodic timer loop. Queries the state actor for
   current state hashes, broadcasts `SyncRequest` through the network
   actor. Ensures the worker actively converges with other peers.

```
┌───────────┐  events   ┌───────────┐  response  ┌──────────┐
│  Network  │──────────►│   State   │───────────►│ Network  │
│   Actor   │  request  │   Actor   │            │  (send)  │
│           │──────────►│           │            │          │
└───────────┘           └─────┬─────┘            └──────────┘
                              │ ▲                      ▲
                    capacity  │ │ state hash    sync    │
                    query     │ │ query        request  │
                        ┌─────┘ └──────┐               │
                        ▼              ▼               │
                   ┌───────────┐ ┌───────────┐         │
                   │ Heartbeat │ │   Sync    │─────────┘
                   │   Actor   │ │   Actor   │
                   └───────────┘ └───────────┘
```

**Request flow:**
1. Network actor receives `WorkerRequest` from gossipsub
2. Sends `(request, oneshot::Sender<WorkerResponse>)` to state actor
3. State actor processes request, sends response on the oneshot
4. Network actor receives response, publishes to gossipsub

Multiple requests are in-flight concurrently — each gets its own
oneshot reply channel. The state actor processes them sequentially
but individual request handling is fast (memory lookups for replay,
indexed queries for storage).

**Why actors over locks:**
- No deadlocks — impossible with message passing
- No contention — state is owned by a single task
- Clean shutdown — drain channels, process remaining messages
- Easy to reason about — each actor has a clear, single responsibility

### Active Sync

Workers that hold server state (replay nodes, and any future stateful
worker types) participate in the same sync protocol as regular peers.
They are not passive recipients — they actively maintain state
consistency by syncing with other peers and workers.

The worker library runs a **sync loop** as a fourth actor alongside
network, state, and heartbeat:

- **Periodic sync**: Every N seconds (configurable, default 30s), the
  sync actor broadcasts a `SyncRequest` with the worker's current state
  hash for each server it serves. This is identical to what clients do
  when they come online.
- **Peer-to-peer convergence**: Other replay nodes, storage nodes, and
  regular peers respond with any events the worker is missing. The
  worker applies them via `on_event()` through the state actor.
- **Incoming sync requests**: Workers also *respond* to `SyncRequest`
  from other workers and peers — they're full participants in the
  protocol, not just consumers.

This creates a **continuous convergence loop** across the network.
Workers don't just wait for events to arrive via gossipsub — they
actively pull from peers to catch up on anything missed. Benefits:

- **Resilience to gossipsub gaps**: If a gossipsub message is dropped
  (network partition, temporary disconnect), the next sync cycle
  catches it
- **Worker-to-worker consistency**: Multiple replay nodes converge to
  the same state, so clients get consistent responses regardless of
  which worker they hit
- **Stale data mitigation**: State is never more than one sync interval
  behind, even if gossipsub delivery is delayed

Storage nodes participate in sync differently — they don't maintain
`ServerState` but they do broadcast `SyncRequest` to discover events
they may have missed, ensuring their archive is complete.

### Peer Lifecycle

The library handles all common peer behavior:

1. **Identity** — Generates or loads a persistent Ed25519 identity per
   worker instance from a configured path.
2. **Network bootstrap** — Connects to the relay, subscribes to both
   `_willow_workers` (discovery protocol) and `_willow_server_ops`
   (server events and permissions) gossipsub topics, plus per-channel
   topics for assigned servers.
3. **Heartbeat loop** — Broadcasts `WorkerAnnouncement` every 10 seconds
   on the `_willow_workers` gossipsub topic.
4. **Permission awareness** — Listens for `GrantPermission` events. Only
   begins serving requests for a server after receiving `SyncProvider`
   permission.
5. **Graceful shutdown** — On SIGTERM/SIGINT, broadcasts a departure
   message so clients evict the worker from their cache immediately.

### Binary Entry Point

Each worker binary is minimal:

```rust
fn main() {
    let config = WorkerConfig::from_args();
    let role = ReplayRole::new(&config);
    willow_worker::run(role, config);
}
```

## Capability Discovery Protocol

### Gossipsub Topic

All worker discovery happens on `_willow_workers`. This topic carries
only worker protocol messages, not server state.

### Heartbeat Message

```rust
WorkerAnnouncement {
    peer_id: String,
    role: WorkerRoleInfo,       // role type + capacity in one enum
    servers: Vec<String>,       // server IDs this worker serves
    timestamp: u64,
}

/// Combined role identity and capacity. The role type is implicit in
/// the variant — impossible to have a Replay role with Storage capacity.
enum WorkerRoleInfo {
    Replay {
        servers_loaded: u32,
        events_buffered: u32,
        max_events: u32,
    },
    Storage {
        servers_tracked: u32,
        total_events_stored: u64,
        disk_used_bytes: u64,
    },
    // Future: File { ... }, Stream { ... }, Bot { ... }
}
```

**Interval**: Every 10 seconds.

### Client-Side Cache

Clients maintain a local `HashMap<(RoleType, ServerId), Vec<WorkerInfo>>`
populated from heartbeats. Entries are evicted after 30 seconds without
a heartbeat (3 missed heartbeats).

When a client needs a service:
1. Look up workers for the role + server in the local cache
2. Pick one (round-robin or random for load distribution)
3. Send a `WorkerRequest` via gossipsub
4. If no workers are cached, fall back to direct peer-to-peer sync

### Failure Detection

- 3 missed heartbeats (30s) → client removes worker from cache
- Graceful shutdown → immediate eviction via departure message
- Client-side timeout on requests (5s) → try next available worker

## Replay Node (`willow-replay`)

### Purpose

Fast, bounded-memory state sync. When a peer comes online, the replay
node gets them caught up as quickly as possible.

### Behavior

- Subscribes to all assigned servers' gossipsub topics
- Applies incoming events to an in-memory `ServerState` per server using
  `apply_lenient()`
- Responds to `SyncRequest` messages by computing a diff against the
  requester's state hash and sending a `SyncBatch`

### Bounded Memory Strategy

- Configurable max events per server (default: 1000)
- When the buffer is full, oldest events are evicted
- The computed `ServerState` snapshot is always retained
- If a client's state hash predates the oldest buffered event, the replay
  node sends a `StateSnapshot` (full computed state) instead of a diff

### New Wire Message

```rust
StateSnapshot {
    server_id: String,
    state: ServerState,
}
```

This enables fast catch-up for peers that are very far behind — instead
of replaying hundreds of events, they receive the final computed state
directly.

### Configuration

```
--max-events-per-server 1000    # Event buffer size
--sync-interval 30              # Active sync interval in seconds
--relay <multiaddr>             # Relay to connect through
--identity-path <path>          # Ed25519 keypair location
```

## Storage Node (`willow-storage`)

### Purpose

Archival history. Persists every event to disk indefinitely. Serves
paginated history queries for older messages.

### Behavior

- Subscribes to all assigned servers' gossipsub topics
- Persists every event to SQLite (migrated from the relay's current
  `EventStore` implementation)
- Does NOT maintain an in-memory `ServerState` — stores raw events only
- Responds to `WorkerRequest::History` with paginated results

### Request/Response Types

```rust
WorkerRequest::History {
    server_id: String,
    channel: String,
    before_timestamp: Option<u64>,  // pagination cursor
    limit: u32,                      // default 50
}

WorkerResponse::HistoryPage {
    events: Vec<Event>,
    has_more: bool,
}
```

### Client Integration

- When a user scrolls up past locally cached messages, the client sends
  a `History` request to a known storage node
- Results are cached locally so the same page isn't re-requested
- UI shows "Loading older messages..." while waiting

### Configuration

```
--db-path <path>                # SQLite database location
--sync-interval 60              # Active sync interval in seconds
--relay <multiaddr>             # Relay to connect through
--identity-path <path>          # Ed25519 keypair location
```

## Wire Protocol Additions

New messages on the `_willow_workers` gossipsub topic:

```rust
enum WorkerWireMessage {
    /// Periodic heartbeat from a worker.
    Announcement(WorkerAnnouncement),

    /// Client requesting a service from a worker.
    Request {
        request_id: String,
        target_peer: String,     // specific worker peer ID
        payload: WorkerRequest,
    },

    /// Worker responding to a client request.
    Response {
        request_id: String,
        target_peer: String,     // requesting client's peer ID
        payload: WorkerResponse,
    },
}

enum WorkerRequest {
    /// Request state sync (handled by replay nodes).
    Sync {
        server_id: String,
        state_hash: Vec<u8>,
    },

    /// Request paginated history (handled by storage nodes).
    History {
        server_id: String,
        channel: String,
        before_timestamp: Option<u64>,
        limit: u32,
    },
}

enum WorkerResponse {
    /// Batch of events for sync catch-up.
    SyncBatch { events: Vec<Event> },

    /// Full state snapshot for far-behind peers.
    Snapshot { state: ServerState },

    /// Paginated history results.
    HistoryPage {
        events: Vec<Event>,
        has_more: bool,
    },

    /// Request denied (no permission, unknown server, etc).
    Denied { reason: String },
}
```

## Authorization & Server Creation

### Permission Model

Workers use the existing `SyncProvider` permission. No new permission
types are needed. A worker without `SyncProvider` for a server can
listen to events (building state) but cannot respond to requests.

### Server Creation Flow

When a user creates a server, the creation flow includes a new
"Worker Nodes" step:

1. User enters server name (existing step)
2. **New: "Worker Nodes" step** — checklist of available platform workers
   grouped by role:
   - **Replay Nodes**: list of known replay worker peer IDs, all checked
     by default
   - **Storage Nodes**: list of known storage worker peer IDs, all checked
     by default
   - "Select All / Deselect All" per group
   - Brief descriptions: "Replay nodes keep your server available when
     you're offline", "Storage nodes preserve your full message history"
3. User can uncheck any workers they don't want
4. On submit, `GrantPermission { peer_id, permission: SyncProvider }`
   events are emitted for each checked worker as part of the genesis
   event sequence

### Known Worker Discovery

The client needs to know which platform workers exist to populate the
checklist. For the initial version:

1. The operator generates persistent Ed25519 identities for each worker
   (e.g., `willow-replay --generate-identity --identity-path /etc/willow/replay.key`)
2. The resulting peer IDs are hardcoded in the client at build time as a
   `PLATFORM_WORKERS` constant
3. A discovery-based approach (querying the `_willow_workers` topic
   before server creation) can replace this later

### Server Settings (Stub)

A "Worker Nodes" section in server settings where owners can:
- See currently authorized workers and their roles
- Revoke worker access
- Authorize new workers

This is a stub for the initial version — the UI exists but the
management API behind it is minimal.

## Auto-Allocation

### Initial Strategy

All workers serve all servers. When a worker starts:

1. Connects to relay
2. Discovers servers via `_willow_server_ops` gossipsub traffic
3. Auto-joins every server it discovers
4. Begins heartbeats listing all servers
5. Waits for `SyncProvider` permission before serving requests

### Future Stubs

```rust
pub enum AllocationStrategy {
    /// Serve all discovered servers (initial implementation).
    Global,
    /// Serve only specific servers.
    PerServer(Vec<String>),
    /// Dynamic allocation based on load (future).
    Dynamic,
}
```

Configuration fields for:
- `allocation_strategy: AllocationStrategy`
- `max_servers: Option<usize>`
- Placeholder for management API (add/remove server assignments at
  runtime)

## Deployment

> **Superseded (2026-06-05):** The Docker Compose + sshpass/Linode systemd deployment described
> below is retired. Production deployment now lives in the shared `infra` NixOS flake, where Willow
> is a single `runtime = "multi"` app (web + relay + replay + storage). The `docker/` directory,
> `docker-compose.yml`, and the CI deploy workflow were removed. See
> [`plans/2026-06-05-infra-deployment-migration.md`](../plans/2026-06-05-infra-deployment-migration.md).
> This section is kept as the historical record of the original worker-node deployment topology.

### Docker

All infrastructure is containerized for reproducible deployment. A
single `docker-compose.yml` at the repo root defines the full stack.

#### Dockerfiles

```
docker/
├── relay.Dockerfile      — Builds willow-relay binary
├── replay.Dockerfile     — Builds willow-replay binary
├── storage.Dockerfile    — Builds willow-storage binary
└── web.Dockerfile        — Builds web app via trunk, serves with nginx
```

All worker Dockerfiles share a common multi-stage pattern:
1. **Builder stage** — `rust:latest`, builds the release binary
2. **Runtime stage** — `debian:bookworm-slim`, copies just the binary
   and runtime deps (libssl, ca-certificates)

#### Docker Compose

```yaml
services:
  relay:
    build:
      dockerfile: docker/relay.Dockerfile
    ports:
      - "9090:9090"   # TCP
      - "9091:9091"   # WebSocket
    volumes:
      - relay-identity:/etc/willow

  replay-1:
    build:
      dockerfile: docker/replay.Dockerfile
    volumes:
      - replay-1-identity:/etc/willow
    environment:
      - RELAY_ADDR=/dns4/relay/tcp/9091/ws/p2p/${RELAY_PEER_ID}
      - MAX_EVENTS_PER_SERVER=1000
      - SYNC_INTERVAL=30

  replay-2:
    build:
      dockerfile: docker/replay.Dockerfile
    volumes:
      - replay-2-identity:/etc/willow
    environment:
      - RELAY_ADDR=/dns4/relay/tcp/9091/ws/p2p/${RELAY_PEER_ID}

  storage-1:
    build:
      dockerfile: docker/storage.Dockerfile
    volumes:
      - storage-1-identity:/etc/willow
      - storage-1-data:/var/lib/willow
    environment:
      - RELAY_ADDR=/dns4/relay/tcp/9091/ws/p2p/${RELAY_PEER_ID}
      - SYNC_INTERVAL=60

  storage-2:
    build:
      dockerfile: docker/storage.Dockerfile
    volumes:
      - storage-2-identity:/etc/willow
      - storage-2-data:/var/lib/willow
    environment:
      - RELAY_ADDR=/dns4/relay/tcp/9091/ws/p2p/${RELAY_PEER_ID}

  web:
    build:
      dockerfile: docker/web.Dockerfile
    ports:
      - "80:80"
      - "443:443"

volumes:
  relay-identity:
  replay-1-identity:
  replay-2-identity:
  storage-1-identity:
  storage-1-data:
  storage-2-identity:
  storage-2-data:
```

#### Identity Bootstrap

On first run, each container generates its Ed25519 identity if one
doesn't exist in its volume. A bootstrap script collects the peer IDs
and outputs them for hardcoding into the client build:

```bash
# After first `docker compose up`, extract worker peer IDs
docker compose exec replay-1 willow-replay --print-peer-id
docker compose exec replay-2 willow-replay --print-peer-id
docker compose exec storage-1 willow-storage --print-peer-id
docker compose exec storage-2 willow-storage --print-peer-id
```

These peer IDs go into `PLATFORM_WORKERS` in the client. A future
improvement replaces this with dynamic discovery.

#### Scaling

Adding more workers is a compose scale or a new service entry:

```bash
# Quick scale (shares config, generates new identity)
docker compose up -d --scale replay=4

# Or add a named service for persistent identity
```

#### Just Commands

```bash
just docker-build     # Build all images
just docker-up        # Start full stack
just docker-down      # Stop full stack
just docker-logs      # Tail all logs
just docker-ids       # Print all worker peer IDs
```

### Alongside Relay (Bare Metal)

Workers are deployed as separate processes alongside the relay on the
same server (or different servers). They connect to the relay like any
browser or native peer.

**Multiple instances per role** are deployed for redundancy. Each
instance has its own identity and runs as an independent peer. If one
crashes, others continue serving. Clients automatically failover via
the worker cache — the downed worker's heartbeats stop, it gets evicted,
and requests route to surviving instances.

```
┌──────────────────────────────────────┐
│  Production Server                   │
│                                      │
│  willow-relay       (ports 9090/1)   │
│  willow-replay-1    (peer)           │
│  willow-replay-2    (peer)           │
│  willow-storage-1   (peer)           │
│  willow-storage-2   (peer)           │
│                                      │
└──────────────────────────────────────┘
```

Each worker instance has its own:
- Ed25519 identity (persistent keypair file, unique per instance)
- Network connection to the relay
- systemd service unit for process management
- Data directory (for storage nodes: separate SQLite databases)

### Systemd Services

```
willow-relay.service       — existing
willow-replay@.service     — template unit, instantiated as
                             willow-replay@1.service,
                             willow-replay@2.service, etc.
willow-storage@.service    — template unit, instantiated as
                             willow-storage@1.service,
                             willow-storage@2.service, etc.
```

Template units use `%i` for instance-specific paths:
```ini
[Service]
ExecStart=/usr/local/bin/willow-replay \
    --identity-path /etc/willow/replay-%i.key \
    --relay /ip4/127.0.0.1/tcp/9091/ws/p2p/<relay-peer-id>
```

### Scaling

To add capacity, the operator deploys additional instances:
```bash
# Generate identity for new instance
willow-replay --generate-identity --identity-path /etc/willow/replay-3.key

# Enable and start
systemctl enable --now willow-replay@3.service
```

The new instance joins the network, begins heartbeating, and clients
discover it automatically. Server owners need to grant it `SyncProvider`
permission (or it gets authorized automatically if the peer ID is added
to `PLATFORM_WORKERS` and new servers are created).

## Client Changes Summary

1. **Worker cache** — new module maintaining discovered workers from
   heartbeats, with TTL-based eviction
2. **Sync routing** — prefer replay nodes for `SyncRequest` instead of
   broadcasting to all peers
3. **History loading** — new "load older messages" flow that queries
   storage nodes on scroll-up
4. **Server creation** — new "Worker Nodes" step with authorization
   checklist
5. **Server settings** — stub "Worker Nodes" section for managing
   authorized workers
