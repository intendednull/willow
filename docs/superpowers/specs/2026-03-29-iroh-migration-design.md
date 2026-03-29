# Iroh Migration Design Spec

**Date**: 2026-03-29
**Status**: Draft

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
- Changing the client API surface (ClientHandle methods stay the same)
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

**Approach**: Don't shim iroh into the old libp2p-shaped API. Restructure
`willow-identity` around iroh's model natively:

- `Identity` wraps `iroh_base::SecretKey` and exposes `EndpointId` directly
- Drop the `peer_id() -> String` indirection — consumers use `EndpointId`
  as the native peer identifier type throughout the codebase
- `ServerState.owner`, `Event.author`, permission maps, profile keys all
  change from `String` to `EndpointId` (or its serialized form)
- No backward compatibility with libp2p `PeerId` strings — clean break,
  all state starts fresh

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

### `willow-identity` (modified)

```rust
// Before
pub struct Identity {
    keypair: libp2p::identity::Keypair,
}

// After
pub struct Identity {
    secret_key: iroh_base::SecretKey,
}

impl Identity {
    pub fn generate() -> Self;
    pub fn from_bytes(bytes: &[u8]) -> Result<Self>;
    pub fn to_bytes(&self) -> Vec<u8>;
    pub fn peer_id(&self) -> String;           // hex(EndpointId)
    pub fn endpoint_id(&self) -> EndpointId;   // new
    pub fn secret_key(&self) -> &SecretKey;     // new
    pub fn sign(&self, data: &[u8]) -> Signature;
    pub fn verify(public_key: &PublicKey, data: &[u8], sig: &Signature) -> bool;
}
```

Signing and verification use `iroh_base::SecretKey::sign()` and
`iroh_base::PublicKey::verify()` directly — same Ed25519 algorithm,
different wrapper types.

### `willow-network` (rewritten)

The entire crate is replaced. Current contents (behaviour.rs, node.rs,
config.rs, file_transfer.rs) are removed.

```rust
// New public API

pub struct NetworkNode {
    endpoint: iroh::Endpoint,
    gossip: iroh_gossip::Gossip,
    blobs: iroh_blobs::BlobsProtocol,
    router: iroh::Router,
}

pub struct NetworkConfig {
    pub secret_key: SecretKey,
    pub relay_url: Option<RelayUrl>,       // replaces Multiaddr
    pub bootstrap_peers: Vec<EndpointAddr>, // replaces Vec<(PeerId, Multiaddr)>
    pub mdns: bool,                         // enable LAN discovery
}

impl NetworkNode {
    pub async fn new(config: NetworkConfig) -> Result<Self>;
    pub async fn subscribe(&self, topic: TopicId, bootstrap: Vec<EndpointId>)
        -> Result<(GossipSender, GossipReceiver)>;
    pub async fn publish(&self, sender: &GossipSender, data: Vec<u8>) -> Result<()>;
    pub fn endpoint_id(&self) -> EndpointId;
    pub fn endpoint(&self) -> &Endpoint;

    // Blob operations (replaces file_transfer.rs)
    pub async fn add_blob(&self, data: Vec<u8>) -> Result<Hash>;
    pub async fn get_blob(&self, hash: Hash, from: EndpointAddr) -> Result<Vec<u8>>;
    pub fn blob_ticket(&self, hash: Hash) -> BlobTicket;

    pub async fn shutdown(self) -> Result<()>;
}
```

**No more native/WASM split in node.rs**: iroh's `Endpoint` handles
platform differences internally. The same code compiles for both targets.

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

### `willow-client` (modified)

The `network.rs` module is updated to use iroh types:

```rust
// NetworkCommand changes
pub enum NetworkCommand {
    Subscribe(TopicId),                          // was Subscribe(String)
    Publish { topic: TopicId, data: Vec<u8> },   // was String topic
    ShareFile { topic: TopicId, ... },
    BroadcastProfile { display_name: String },
    BroadcastEvent { event: Event, topic: Option<TopicId> },
    RequestSync { state_hash: StateHash, topic: TopicId },
    SendSyncBatch { events: Vec<Event> },
    // Voice, typing unchanged
}
```

The `spawn_network()` function simplifies significantly — no more
separate native/WASM code paths:

```rust
pub async fn spawn_network(
    config: NetworkConfig,
    cmd_rx: UnboundedReceiver<NetworkCommand>,
    event_tx: UnboundedSender<NetworkEvent>,
) {
    let node = NetworkNode::new(config).await.unwrap();

    // Subscribe to topics...
    // Single event loop for both native and WASM
    loop {
        tokio::select! {
            Some(event) = receiver.next() => { /* handle gossip event */ }
            Some(cmd) = cmd_rx.recv() => { /* handle command */ }
        }
    }
}
```

### `willow-app` (out of scope)

The Bevy desktop app is not part of this migration. Focus is on the
Leptos web UI (`crates/web/`), which consumes `willow-client` directly.
The Bevy app can be migrated later using the same updated client library.

### `willow-worker` (modified)

Worker network actor switches from libp2p swarm to iroh endpoint.
The actor model (network, state, heartbeat, sync) stays the same.

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

### Phase 1: Identity Layer

Update `willow-identity` to use `iroh_base::SecretKey` / `PublicKey`.
Keep the same `Identity` API surface. Add conversion utilities for
the peer ID format change.

**Test**: All identity tests pass. Sign/verify round-trips work.
**Risk**: Low — isolated crate with clear API boundary.

### Phase 2: Network Crate

Rewrite `willow-network` against iroh. Implement `NetworkNode` with
gossip and blob support. Delete `behaviour.rs`, `file_transfer.rs`.

**Test**: New integration tests with real iroh endpoints on localhost.
**Risk**: Medium — largest code change, but well-isolated behind
`NetworkNode` API.

### Phase 3: Client Network Layer

Update `willow-client/src/network.rs` to use the new `NetworkNode`.
Collapse the native/WASM code paths into a single implementation.

**Test**: Client tests, web UI integration.
**Risk**: Medium — touches the async boundary.

### Phase 4: Relay + Workers

Replace `willow-relay` with iroh relay wrapper. Update worker network
actor. Deploy new relay alongside old relay for testing.

**Test**: Relay tests, worker tests, scaling tests.
**Risk**: Medium — deployment change, but relay is stateless.

### Phase 5: Cleanup

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
