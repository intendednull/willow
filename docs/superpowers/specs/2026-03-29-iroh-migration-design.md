# Iroh Migration Design Spec

**Date**: 2026-03-29
**Status**: Approved

## Overview

Replace libp2p with iroh as Willow's networking layer. Iroh provides
QUIC-based peer-to-peer connections dialed by public key, with built-in
NAT traversal, relay fallback, and native WASM support. This migration
simplifies the networking stack while gaining better performance (QUIC
multiplexing, 0-RTT), simpler NAT traversal (hole punching + relay
built-in), and a cleaner protocol composition model.

## Why Iroh

**Problems with libp2p today:**
- Complex composite `NetworkBehaviour` with 6 sub-behaviours
- Separate TCP and WebSocket transports require a dedicated relay to
  bridge native and browser peers
- NAT traversal requires explicit relay protocol configuration
- GossipSub mesh maintenance adds overhead for small networks
- Large dependency tree (~150 transitive deps for networking alone)

**What iroh provides:**
- Single `Endpoint` type handles all connections (QUIC-native)
- Built-in hole punching with automatic relay fallback
- Ed25519 public key IS the peer address (no separate PeerId mapping)
- `iroh-gossip` uses HyParView+PlumTree — self-optimizing for latency,
  lower overhead for small networks (active view of 5 peers)
- ALPN-based protocol routing via `Router` replaces behaviour composition
- Native WASM support without transport adapters
- Content-addressed blob transfer (BLAKE3) replaces custom chunk protocol

## Non-Goals

- Changing the event-sourced state model (willow-state is untouched)
- Changing the wire message format (WireMessage/pack_wire/unpack_wire)
- Changing the client's public API semantics (send_message, create_server, etc.)
- Changing the Leptos web UI components
- Changing the Bevy desktop app (out of scope — focus on web UI only)
- Migrating in a single atomic step (phased approach)
- Preserving backward compatibility with old libp2p data (clean break)

## Architecture Mapping

### Identity

| Current (libp2p) | Iroh |
|---|---|
| `libp2p::PeerId` (multihash of public key) | `iroh::EndpointId` (= Ed25519 `PublicKey`, 32 bytes) |
| `libp2p::identity::Keypair` | `iroh_base::SecretKey` |
| `willow_identity::Identity` wraps libp2p keypair | `willow_identity::Identity` wraps `iroh_base::SecretKey` |

**Key change**: iroh's `EndpointId` is the raw Ed25519 public key (32
bytes), not a multihash. All peer ID strings throughout the codebase
change format. This affects:
- `ServerState.owner`, `ServerState.members` keys
- `Event.author` field
- Stored profiles, permissions, channel keys
- Wire protocol peer identification

**Approach**: Build the entire networking stack around iroh's model
natively. Don't shim iroh into libp2p-shaped abstractions anywhere:

- `Identity` wraps `iroh_base::SecretKey` and exposes `EndpointId` directly
- Drop the `peer_id() -> String` indirection — consumers use `EndpointId`
  as the native peer identifier type throughout the codebase
- `ServerState.owner`, `Event.author`, permission maps, profile keys all
  change from `String` to `EndpointId` (or its serialized form)
- Network layer uses `Endpoint` + `Router` + `ProtocolHandler` natively,
  not wrapped behind libp2p-shaped `NetworkNode` / `NetworkEvent` enums
- Gossip uses `GossipTopic` / `GossipSender` / `GossipReceiver` directly,
  not wrapped behind publish/subscribe command channels
- File transfer uses `iroh-blobs` `Hash` / `BlobTicket` directly, not
  mapped through `FileManifest` / `ChunkRequest` abstractions
- No backward compatibility with libp2p data — clean break, fresh state

### Transport

| Current | Iroh |
|---|---|
| TCP + Noise + Yamux (native) | QUIC via `noq` (native) |
| WebSocket via `websocket_websys` (WASM) | QUIC over relay (WASM) |
| Manual relay protocol for NAT traversal | Built-in hole punching + relay fallback |
| Separate TCP/WS listeners on relay | Single relay server, handles both |

**Key change**: No more separate transport stacks for native vs WASM.
Both use the same `Endpoint` — native gets direct QUIC + relay fallback,
WASM gets relay-only (same as today but transparent). The relay is an
iroh relay server instead of a custom libp2p relay.

### Protocol Composition

| Current | Iroh |
|---|---|
| `WillowBehaviour` (6 sub-behaviours) | `Router` with ALPN handlers |
| GossipSub for pub/sub | `iroh-gossip` (HyParView + PlumTree) |
| Kademlia for DHT | DNS + pkarr address lookup |
| mDNS for LAN discovery | `address-lookup-mdns` feature |
| `identify::Behaviour` | Automatic (QUIC TLS includes identity) |
| Request-Response for file chunks | `iroh-blobs` (BLAKE3 verified streaming) |

### Gossip

| Current (GossipSub) | Iroh Gossip |
|---|---|
| String topic names | `TopicId` (32-byte hash) |
| `node.subscribe(topic)` | `gossip.subscribe(topic_id, bootstrap_peers)` |
| `node.publish(topic, data)` | `sender.broadcast(data)` |
| Mesh-based with heartbeat | Epidemic broadcast tree (self-optimizing) |
| Max message size: configurable | Max message size: 4096 default, configurable |

**Topic mapping**: Current string topics (channel names, `_willow_server_ops`,
`_willow_workers`, `_willow_profiles`) become `TopicId` values derived by
hashing the string: `TopicId::from(blake3::hash(topic_string.as_bytes()))`.

**Bootstrap**: iroh-gossip requires bootstrap peers when subscribing to a
topic. The relay/worker nodes serve as bootstrap peers — their `EndpointId`s
are known at build time (same as current `PLATFORM_WORKERS`).

### File Transfer

| Current | Iroh |
|---|---|
| Custom `ChunkRequest`/`ChunkResponse` | `iroh-blobs` with BLAKE3 hashes |
| Manual request-response protocol | Built-in verified streaming |
| `willow-files` content-addressed chunks | Map to `iroh-blobs` `Hash` + `HashSeq` |
| `FileManifest` over gossipsub | `BlobTicket` shared over gossip |

**Key change**: Replace the custom `/willow/chunks/1` request-response
protocol with `iroh-blobs`. Files are added to the local blob store,
a `BlobTicket` (containing hash + provider address) is broadcast over
gossip, and receivers download directly via the blobs protocol.

### Relay

| Current | Iroh |
|---|---|
| Custom `willow-relay` binary | `iroh-relay` server |
| TCP + WebSocket dual listeners | Single relay endpoint |
| GossipSub pass-through | Encrypted packet forwarding |
| Kademlia + Identify protocols | Not needed (DNS-based lookup) |
| Stateless (after worker extraction) | Stateless by design |

**Key change**: The relay becomes an off-the-shelf iroh relay server.
It only forwards encrypted QUIC packets — it cannot read message content.
This is a security improvement over the current relay which participates
in GossipSub and can read unencrypted gossip traffic.

## Crate Changes

### `willow-identity` (rewritten)

Thin wrapper around iroh's native identity types. No libp2p vestiges.

```rust
use iroh_base::{SecretKey, PublicKey, Signature};
pub use iroh::EndpointId; // re-export, = PublicKey

pub struct Identity {
    secret_key: SecretKey,
}

impl Identity {
    pub fn generate() -> Self;
    pub fn from_bytes(bytes: &[u8]) -> Result<Self>;
    pub fn to_bytes(&self) -> Vec<u8>;
    pub fn endpoint_id(&self) -> EndpointId;
    pub fn secret_key(&self) -> &SecretKey;
    pub fn sign(&self, data: &[u8]) -> Signature;
    pub fn public_key(&self) -> PublicKey;
}

// Standalone verification — no Identity needed
pub fn verify(key: &PublicKey, data: &[u8], sig: &Signature) -> bool;
```

No `peer_id() -> String`. Consumers use `EndpointId` (= `PublicKey`)
directly. Display formatting uses iroh's `fmt_short()` for UIs.

### `willow-network` (rewritten)

The entire crate is replaced. Current contents (behaviour.rs, node.rs,
config.rs, file_transfer.rs) are removed. The new crate is a thin
setup layer — it does NOT wrap iroh types behind Willow-specific
abstractions. Consumers use iroh types directly.

```rust
use iroh::{Endpoint, Router, EndpointId};
use iroh_base::{SecretKey, RelayUrl, EndpointAddr};
use iroh_gossip::{Gossip, TopicId};
use iroh_blobs::BlobsProtocol;

/// Configuration for creating a Willow network endpoint.
pub struct Config {
    pub secret_key: SecretKey,
    pub relay_url: Option<RelayUrl>,
    pub bootstrap_peers: Vec<EndpointAddr>,
    pub mdns: bool,
}

/// Assembled iroh stack, ready to use. Fields are public —
/// consumers interact with iroh types directly.
pub struct Network {
    pub endpoint: Endpoint,
    pub gossip: Gossip,
    pub blobs: BlobsProtocol,
    pub router: Router,
}

impl Network {
    /// Build and spawn the iroh endpoint, router, gossip, and blobs.
    pub async fn new(config: Config) -> Result<Self>;

    /// Convenience: this node's EndpointId.
    pub fn id(&self) -> EndpointId;

    /// Graceful shutdown.
    pub async fn shutdown(self) -> Result<()>;
}
```

Consumers call `network.gossip.subscribe(topic, peers)` directly to
get a `GossipTopic`, then call `.split()` for sender/receiver. No
Willow-specific `subscribe()` / `publish()` wrappers. Same for blobs —
call `network.blobs` methods directly.

**No more native/WASM split**: iroh's `Endpoint` handles platform
differences internally. The same code compiles for both targets.

### `willow-transport` (minimal changes)

The `Envelope` and `pack`/`unpack` functions are unchanged — they operate
on `Vec<u8>` and don't depend on the transport layer. The only change is
removing any libp2p type imports if present.

### `willow-relay` (replaced)

The custom relay binary is replaced by an iroh relay server deployment.
The `crates/relay/` directory can either:

1. **Wrap iroh-relay** with Willow-specific configuration (recommended):
   ```rust
   fn main() {
       let config = RelayConfig::from_args();
       iroh_relay::Server::new(config)
           .tls(cert, key)
           .bind(addr)
           .run()
           .await;
   }
   ```

2. **Use iroh-relay directly** as an external binary, configured via
   environment variables.

Option 1 is preferred for consistency with the existing deployment model.

### `willow-client` (restructured)

Drop the `NetworkCommand` / `NetworkEvent` enum indirection. The client
holds iroh handles directly and calls them inline.

```rust
use willow_network::Network;
use iroh_gossip::{GossipSender, GossipReceiver, TopicId};
use iroh_blobs::Hash;

pub struct ClientHandle {
    network: Network,
    /// Active gossip subscriptions, keyed by TopicId.
    topics: HashMap<TopicId, GossipSender>,
    state: Rc<RefCell<SharedState>>,
}

impl ClientHandle {
    pub async fn connect(config: willow_network::Config) -> Result<Self> {
        let network = Network::new(config).await?;
        // Subscribe to system topics directly
        let ops_topic = network.gossip
            .subscribe(SERVER_OPS_TOPIC, bootstrap_peers)
            .await?;
        // ...
        Ok(Self { network, topics, state })
    }

    pub async fn send_message(&self, channel: &str, body: &str) -> Result<()> {
        let event = /* create Event */;
        let data = pack_wire(&WireMessage::Event(event), &self.identity)?;
        let topic_id = channel_topic(server_id, channel_id);
        self.topics[&topic_id].broadcast(data.into()).await?;
        Ok(())
    }

    pub async fn share_file(&self, topic: TopicId, data: Vec<u8>) -> Result<Hash> {
        let hash = self.network.blobs.add_slice(&data).await?.hash;
        let ticket = BlobTicket::new(self.network.endpoint.addr(), hash, BlobFormat::Raw);
        // Broadcast ticket over gossip
        self.topics[&topic].broadcast(ticket_bytes.into()).await?;
        Ok(hash)
    }
}
```

**No more command channels**: The old architecture used `mpsc` channels
to bridge async networking into sync Bevy ECS. Since we're focusing on
the Leptos web UI (which is async-native), the client calls iroh
directly. No `NetworkCommand` enum, no `NetworkEvent` enum, no bridge.

The `ClientEventLoop` is replaced by spawned tasks that stream from
`GossipReceiver` and update `SharedState` directly:

```rust
// Spawned per-topic listener
async fn listen_topic(
    mut receiver: GossipReceiver,
    state: Rc<RefCell<SharedState>>,
    event_tx: UnboundedSender<ClientEvent>,
) {
    while let Some(event) = receiver.next().await {
        if let Ok(Event::Received(msg)) = event {
            let (wire_msg, from) = unpack_wire(&msg.content)?;
            match wire_msg {
                WireMessage::Event(e) => {
                    apply_event(&mut state.borrow_mut(), e);
                    event_tx.send(ClientEvent::MessageReceived { .. });
                }
                // ...
            }
        }
    }
}
```

### `willow-app` (out of scope)

The Bevy desktop app is not part of this migration. Focus is on the
Leptos web UI (`crates/web/`), which consumes `willow-client` directly.
The Bevy app can be migrated later using the same updated client library.

### `willow-worker` (restructured)

Workers hold `Network` directly and use iroh handles natively. The
actor model (network, state, heartbeat, sync) remains, but the network
actor uses `GossipReceiver` streams instead of polling a libp2p swarm:

```rust
pub struct WorkerNode {
    network: Network,
    role: Box<dyn WorkerRole>,
    state_tx: mpsc::Sender<StateMsg>,
}

// Network actor: stream gossip events directly
async fn network_actor(
    mut receiver: GossipReceiver,
    state_tx: mpsc::Sender<StateMsg>,
    sender: GossipSender,
) {
    while let Some(event) = receiver.next().await {
        if let Ok(Event::Received(msg)) = event {
            let (wire_msg, from) = unpack_wire(&msg.content)?;
            state_tx.send(StateMsg::from(wire_msg)).await?;
        }
    }
}
```

No `NetworkEvent` / `NetworkCommand` enums — the actor reads from
`GossipReceiver` and writes to `GossipSender` directly.

## Topic ID Registry

All gossipsub string topics become deterministic `TopicId` values:

```rust
fn topic_id(name: &str) -> TopicId {
    TopicId::from(blake3::hash(name.as_bytes()).as_bytes())
}

// System topics
const SERVER_OPS_TOPIC: TopicId = topic_id("_willow_server_ops");
const WORKERS_TOPIC: TopicId = topic_id("_willow_workers");
const PROFILES_TOPIC: TopicId = topic_id("_willow_profiles");

// Per-channel topics
fn channel_topic(server_id: &str, channel_id: &str) -> TopicId {
    topic_id(&format!("{server_id}/{channel_id}"))
}
```

## Gossip Bootstrap Strategy

iroh-gossip requires bootstrap peers when subscribing to a topic (unlike
GossipSub which discovers peers via the mesh). Strategy:

1. **Relay as bootstrap**: The relay's `EndpointId` is known. All peers
   bootstrap gossip topics through the relay. The relay subscribes to
   all system topics and acts as a rendezvous point.

2. **Worker nodes as bootstrap**: Known worker `EndpointId`s (from
   `PLATFORM_WORKERS`) serve as additional bootstrap peers.

3. **Peer exchange**: Once connected to a topic, iroh-gossip's HyParView
   protocol automatically maintains the peer set. New peers are
   discovered through the gossip protocol itself.

4. **LAN discovery**: With `address-lookup-mdns` enabled, peers on the
   same LAN discover each other without relay. They bootstrap gossip
   topics with each other directly.

## Migration Phases

### Phase 1: Foundation (identity + network + transport)

Rewrite `willow-identity` and `willow-network` against iroh. Update
`willow-state` to use `EndpointId` instead of `String` for peer
identifiers. Update `willow-transport` to remove any libp2p imports.

- `willow-identity`: `SecretKey` / `PublicKey` / `EndpointId` native
- `willow-network`: `Network` struct exposing iroh handles directly
- `willow-state`: `Event.author` becomes `EndpointId`, `ServerState`
  member/permission maps key on `EndpointId`

**Test**: Identity sign/verify, state apply/merge, network endpoint
creation on localhost.
**Risk**: Medium — touches state types, but it's a clean break so no
compatibility concerns.

### Phase 2: Client + Web UI

Restructure `willow-client` to hold `Network` directly. Drop
`NetworkCommand` / `NetworkEvent` enums and the bridge layer. Wire
the Leptos web UI to the new async-native client.

**Test**: Client tests, web UI integration, gossip round-trips.
**Risk**: Medium — largest behavioral change, but simpler code.

### Phase 3: Relay + Workers

Replace `willow-relay` with iroh relay wrapper. Restructure worker
network actor to use `GossipReceiver` / `GossipSender` directly.

**Test**: Relay tests, worker tests, scaling tests.
**Risk**: Low — relay is stateless, workers follow same pattern as
client.

### Phase 4: Cleanup

- Remove all libp2p dependencies from `Cargo.toml` workspace
- Remove `#[cfg(target_arch = "wasm32")]` transport branching
- Update CLAUDE.md architecture docs
- Update Docker deployment configs

## Dependency Changes

### Removed
```toml
# All libp2p crates
libp2p = { version = "0.54", features = [...] }
# Plus transitive: libp2p-gossipsub, libp2p-kad, libp2p-mdns,
# libp2p-identify, libp2p-relay, libp2p-request-response,
# libp2p-noise, libp2p-yamux, libp2p-tcp, libp2p-websocket-websys
```

### Added
```toml
iroh = { version = "0.97", features = ["address-lookup-mdns"] }
iroh-base = "0.97"
iroh-gossip = "0.97"
iroh-blobs = "0.99"
iroh-relay = "0.97"  # relay binary only
```

## WASM Considerations

Iroh handles WASM internally, but these constraints remain:

- **No direct QUIC in browsers**: WASM peers connect via relay only
  (same as current WebSocket-only model). Once WebTransport is widely
  available, iroh can use it for direct browser-to-browser connections.
- **No filesystem blob store**: WASM uses `MemStore` for blobs.
  Persistent blob caching on WASM would need IndexedDB integration
  (future work).
- **Address lookup**: WASM uses `PkarrResolver` (HTTPS-based) instead
  of DNS queries. Configure with `PkarrResolver::n0_dns()`.

**Improvement over current**: No more separate `node.rs` native/WASM
modules. The `Endpoint` builder accepts platform-appropriate config
and handles the rest. The bridge event loop is unified.

## Security Implications

**Improvements:**
- Relay cannot read gossip traffic (forwards encrypted QUIC packets).
  Current relay participates in GossipSub and sees plaintext envelopes.
- QUIC provides transport encryption by default (TLS 1.3). Current
  Noise protocol achieves similar but with more configuration.
- Identity is bound to transport (Ed25519 key in TLS cert). Current
  system has separate libp2p identity and message signing.

**Unchanged:**
- End-to-end encryption (ChaCha20-Poly1305) remains the same.
- Message signing with Ed25519 remains the same (different key wrapper).
- Trust model and permission enforcement in `willow-state` unchanged.

## Performance Expectations

- **Connection establishment**: Faster (QUIC 0-RTT vs TCP+Noise+Yamux
  handshake)
- **Multiplexing**: Better (QUIC streams vs Yamux, no head-of-line
  blocking)
- **Gossip overhead**: Lower for small networks (HyParView active view
  of 5 vs GossipSub mesh degree)
- **File transfer**: Better (BLAKE3 verified streaming vs manual chunk
  request-response)
- **Binary size**: Likely smaller (one transport stack vs two)

## Decisions

### Relay: Self-Hosted by Default

Self-host iroh relays for both development and production. The relay
binary in `crates/relay/` wraps `iroh-relay` with Willow-specific
defaults (ports, TLS, logging). n0's public relay infrastructure can
be used as an additional fallback.

For local dev (`just dev`), the relay runs without TLS on localhost.
For production, the relay runs on the Linode server behind the existing
Caddy/nginx TLS termination — no separate cert management needed.
The production relay is deployed the same way as today (systemd service,
persistent identity volume).

### Gossip Max Message Size

Increase from 4096 to **64 KiB** via `Builder::max_message_size(65536)`.

**Implications**: iroh-gossip uses epidemic broadcast trees (PlumTree).
Messages above the `max_message_size` are rejected at the sender. The
PlumTree protocol sends full messages eagerly to peers in the eager set,
and only sends `IHave` (hash) notifications to peers in the lazy set.
Larger messages mean:

- **More bandwidth per eager push**: Each message is forwarded in full
  to ~5 active-view peers. At 64 KiB, a single broadcast costs ~320 KiB
  of outbound traffic. At 4 KiB, it's ~20 KiB. For Willow's traffic
  patterns (chat messages, sync batches, file manifests), 64 KiB is
  well within reason.
- **Lazy repair cost**: When a lazy peer sends `IHave` and the receiver
  needs the message, it sends `Graft` + the full message is forwarded.
  Larger messages make this repair more expensive, but it only happens
  on tree restructuring (rare).
- **Memory**: Each peer buffers recent message hashes for dedup. Message
  *content* is not retained by the gossip layer after delivery, so the
  max size doesn't affect memory proportionally.
- **No fragmentation risk**: QUIC handles packet-level fragmentation
  transparently. Unlike UDP-based gossip, there's no MTU concern.

64 KiB covers all current message types comfortably. The largest messages
are `SyncBatch` (hundreds of events) which can be split into multiple
batches if they approach the limit. Chat messages and file manifests are
well under 4 KiB.

### Bootstrap Cold Start

This is an infrastructure concern, not an application-level problem.
The relay must be running and reachable for gossip to work — same as
today. The relay's `EndpointId` is baked into the client build config.

For `just dev`, the relay starts first and workers/web connect after.
For production, the relay is a long-running systemd service. If the
relay goes down, peers already connected to each other via HyParView
continue to gossip directly — only new topic joins fail.

Worker nodes provide additional bootstrap redundancy. If the relay is
unreachable but a worker is, peers can bootstrap through the worker.

## Open Questions

1. **iroh stability**: iroh is pre-1.0 (v0.97). API may change between
   minor versions. Pin exact versions and budget for update maintenance.

2. **Blob garbage collection**: iroh-blobs retains all received blobs
   in its store indefinitely. Without GC, disk usage grows unbounded.

   **On clients (browser)**: WASM uses `MemStore` — blobs are lost on
   page close. No GC needed. Native clients could use `MemStore` too
   since files are saved to the filesystem separately after download.

   **On worker nodes**: Storage workers archive events in SQLite, not
   blobs. File workers (future) would need blob GC. Options:
   - **TTL-based**: Evict blobs not accessed in N days. Simple, but
     risks evicting content still needed by peers who haven't downloaded.
   - **Reference counting**: Track which servers/channels reference a
     blob. Evict when no references remain. More complex, but precise.
   - **Size cap**: Evict oldest blobs when store exceeds N GB. Simple
     and predictable. Works well for file workers with bounded disk.

   Recommendation: Start with **size cap** for workers (configurable
   `--max-blob-store-size`). Use `MemStore` for clients. Revisit with
   reference counting when file workers are implemented.

   iroh-blobs' `FsStore` (backed by `redb`) supports deletion via its
   API, so implementing any GC strategy is straightforward once the
   policy is decided.
