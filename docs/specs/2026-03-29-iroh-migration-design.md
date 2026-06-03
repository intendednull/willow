# Iroh Migration Design Spec

**Date**: 2026-03-29
**Status**: Completed 2026-04-18. The Bevy app (`crates/app/`) has been retired
and removed from the workspace; references to `crates/app/` tests below are
historical only.

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
- Changing the WireMessage enum variants or pack/unpack semantics
  (the outer signed envelope naturally changes because the signer's
  public key becomes `EndpointId` instead of `PeerId`, but the
  inner message format is preserved)
- Changing the client's public API semantics (send_message, create_server, etc.)
- Changing the Leptos web UI components
- Changing the Bevy desktop app (out of scope — focus on web UI only)
- Migrating in a single atomic step (phased approach)
- Preserving backward compatibility with old libp2p data (clean break)

## Prior Art

The networking redesign builds on a deep lineage of P2P transport, gossip, and key-addressed-identity systems:

| System | Key idea adopted / how Willow diverges |
|---|---|
| **iroh / n0 stack** (Endpoint, iroh-gossip, iroh-blobs, iroh-relay, iroh-base) | The entire networking layer. QUIC-native `Endpoint` + ALPN `Router`, gossip, content-addressed blobs, and pure-forwarding relay — adopted via a thin trait boundary that speaks iroh's own `TopicId`/`EndpointId` (plus `bytes::Bytes` from the shared Tokio ecosystem). The one Willow-invented wrapper is `BlobHash` (`crates/network/src/traits.rs:14-20`), used because `iroh_blobs` does not compile on WASM; content addressing stays BLAKE3 either way. |
| **libp2p** (GossipSub mesh, Kademlia DHT, Noise/Yamux, DCUtR, circuit relay v2, request-response) | The system being replaced. We abandon its PeerId multihash, GossipSub framing, and Kademlia records wholesale — a clean break with no data backward-compat — citing fragile browser WebRTC/WebTransport, operational weight of three subsystems, and finicky DCUtR hole-punching. |
| **HyParView** (Leitão, Pereira & Rodrigues, DSN 2007) + **Plumtree** / Epidemic Broadcast Trees (Leitão, Pereira & Rodrigues, SRDS 2007) | The gossip algorithm lineage underneath iroh-gossip: HyParView's hybrid active/passive partial-view membership plus Plumtree's eager-push spanning tree with lazy-push gossip repair. We inherit this overlay instead of GossipSub's flood-and-mesh design; topics are `blake3(server_id ‖ channel)` `TopicId`s. |
| **QUIC** (RFC 9000) + **TLS 1.3 for QUIC** (RFC 9001) | UDP-based multiplexed transport with built-in TLS 1.3 encryption, stream multiplexing, connection migration, and low-latency setup. Provides hop-by-hop transport encryption and identity binding; Willow keeps its X25519 + ChaCha20-Poly1305 E2E `Content` sealing independent and on top, so relays forwarding encrypted QUIC never see plaintext. |
| **BLAKE3** (O'Connor, Aumasson, Neves & Wilcox-O'Hearn, 2020) + **Bao** verified-streaming spec | The blob-transfer model behind iroh-blobs: BLAKE3's Merkle-tree hash enables incremental, resumable, content-addressed verified streaming of slices. We delete the custom `willow-files` libp2p request-response protocol and address avatars/attachments by content hash (`BlobHash`). |
| **pkarr** (Public Key Addressable Resource Records) over **Mainline DHT** (BitTorrent, BEP 44) + DNS-over-HTTPS | Key-addressed discovery: publish signed node records keyed by the Ed25519 public key to the Mainline DHT and to n0's DNS server; resolve an `EndpointId` to dialable addresses via `PkarrResolver` / `n0_dns`. Replaces operating a Willow-side Kademlia DHT. |
| **Nostr** (NIP-01) | Comparable public-key-as-identity, relay-mediated propagation model. Willow shares the "raw pubkey is the identity" stance (`EndpointId` = 32-byte Ed25519 key; Nostr uses 32-byte secp256k1/Schnorr per BIP-340 — a curve divergence) but diverges sharply: Willow peers form direct QUIC gossip overlays, with relays as pure encrypted-packet forwarders rather than store-and-forward event databases. |
| **Matrix** (federated homeserver model) | Contrast point for the relay/trust model. Matrix federates trusted homeservers that hold and replicate room state; Willow's relay is a regular, untrusted gossip/forwarding participant (granted SyncProvider only if explicitly authorized) and state lives in the per-author event DAG, not on a server. |

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

**Approach**: Build the entire networking stack around iroh's types and
patterns. Use iroh types (`TopicId`, `EndpointId`, `Bytes`, `Hash`) in
trait interfaces — not libp2p types, not Willow-invented abstractions.
But keep a thin trait boundary so the client and worker code can be
tested without real iroh endpoints:

- `Identity` wraps `iroh_base::SecretKey` and exposes `EndpointId` directly
- Drop the `peer_id() -> String` indirection — consumers use `EndpointId`
  as the native peer identifier type throughout the codebase
- `ServerState.owner`, `Event.author`, permission maps, profile keys all
  change from `String` to `EndpointId` (or its serialized form)
- Network abstraction uses iroh-shaped traits (`TopicHandle`, `BlobStore`)
  that speak iroh types but can be swapped for in-memory test doubles
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
topic. The bootstrap node and worker nodes serve as bootstrap peers —
their `EndpointId`s are known at build time (same as current
`PLATFORM_WORKERS`).

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

**`willow-files` crate**: Currently handles content-addressed chunking
and reassembly. With `iroh-blobs`, chunking is handled by the blob
protocol itself (BLAKE3 verified streaming does incremental
verification). `willow-files` is **deleted** — its responsibilities
are subsumed by `iroh-blobs`. The `BlobStore` trait in `willow-network`
is the new abstraction for file operations.

### Relay

| Current | Iroh |
|---|---|
| Custom `willow-relay` binary | `iroh-relay` server |
| TCP + WebSocket dual listeners | Single relay endpoint |
| GossipSub pass-through | Encrypted packet forwarding |
| Kademlia + Identify protocols | Not needed (DNS-based lookup) |
| Stateless (after worker extraction) | Stateless by design |

**Key change**: The relay splits into two roles:
1. **iroh-relay** — pure packet forwarding for NAT traversal. Cannot
   read message content. This replaces the current libp2p relay.
2. **Bootstrap node** — a lightweight gossip participant that subscribes
   to system topics so new peers have someone to bootstrap against.
   Runs alongside the relay as a separate process (or integrated into
   the relay wrapper binary).

This is a security improvement: the relay's packet-forwarding role
cannot read gossip traffic. The bootstrap node participates in gossip
but only for peer discovery — it doesn't store or process messages
(unlike the current relay which sees all GossipSub traffic in
plaintext).

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
config.rs, file_transfer.rs) are removed.

The new crate provides two things:
1. **Iroh-shaped traits** — abstract over gossip and blob operations
   using iroh's own types. Thin enough that the real implementation is
   trivial, but swappable for test doubles.
2. **Iroh implementation** — assembles `Endpoint` + `Router` + `Gossip`
   + `BlobsProtocol` and implements the traits.

```rust
use bytes::Bytes;
use iroh::EndpointId;
use iroh_gossip::TopicId;
use iroh_blobs::{Hash, BlobFormat};

// ── Traits (iroh-shaped, but mockable) ──────────────────────────

/// A handle to a single gossip topic subscription.
/// Mirrors iroh_gossip::GossipTopic but as a trait.
#[async_trait]
pub trait TopicHandle: Send + Sync {
    async fn broadcast(&self, data: Bytes) -> Result<()>;
    async fn broadcast_neighbors(&self, data: Bytes) -> Result<()>;
    fn neighbors(&self) -> Vec<EndpointId>;
}

/// Incoming gossip message.
pub struct GossipMessage {
    pub content: Bytes,
    pub sender: EndpointId,
}

/// Stream of incoming gossip messages for a topic.
/// Mirrors iroh_gossip::GossipReceiver but as a trait.
#[async_trait]
pub trait TopicEvents: Send {
    async fn next(&mut self) -> Option<Result<GossipEvent>>;
    async fn joined(&mut self) -> Result<()>;
}

pub enum GossipEvent {
    Received(GossipMessage),
    NeighborUp(EndpointId),
    NeighborDown(EndpointId),
}

/// Content-addressed blob operations.
#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn add(&self, data: Bytes) -> Result<Hash>;
    async fn get(&self, hash: Hash) -> Result<Option<Bytes>>;
    async fn has(&self, hash: Hash) -> bool;
}

/// Top-level network handle. Assembled once, passed to client/workers.
#[async_trait]
pub trait Network: Send + Sync {
    type Topic: TopicHandle;
    type Events: TopicEvents;

    fn id(&self) -> EndpointId;

    async fn subscribe(
        &self,
        topic: TopicId,
        bootstrap: Vec<EndpointId>,
    ) -> Result<(Self::Topic, Self::Events)>;

    /// Unsubscribe from a topic. Drops the sender/receiver and leaves
    /// the gossip mesh for this topic.
    async fn unsubscribe(&self, topic: TopicId) -> Result<()>;

    fn blobs(&self) -> &dyn BlobStore;

    /// Stream of connectivity events (relay up/down, peer connects).
    /// Used by client to re-subscribe topics after reconnection.
    async fn connection_events(&self) -> ConnectionEventStream;

    async fn shutdown(&self) -> Result<()>;
}

pub enum ConnectionEvent {
    RelayConnected,
    RelayDisconnected,
    DirectConnected(EndpointId),
    DirectDisconnected(EndpointId),
}

// ── Iroh implementation ─────────────────────────────────────────

pub struct Config {
    pub secret_key: SecretKey,
    pub relay_url: Option<RelayUrl>,
    pub bootstrap_peers: Vec<EndpointAddr>,
    pub mdns: bool,
}

/// Real iroh-backed implementation.
pub struct IrohNetwork { /* Endpoint, Router, Gossip, Blobs */ }

impl IrohNetwork {
    pub async fn new(config: Config) -> Result<Self>;
}

impl Network for IrohNetwork { /* delegates to iroh types */ }

// ── Test double ─────────────────────────────────────────────────

/// In-memory network for tests. No real connections, no network I/O.
/// Needs #[tokio::test] to drive async trait methods, but all
/// delivery happens in-process via broadcast channels.
#[cfg(any(test, feature = "test-utils"))]
pub struct MemNetwork { /* ... */ }

#[cfg(any(test, feature = "test-utils"))]
pub struct MemHub { /* shared broadcast channels per TopicId */ }
```

**Design rationale**: The traits use iroh's types (`TopicId`,
`EndpointId`, `Hash`, `Bytes`) everywhere — no Willow-invented ID
types or message wrappers. The trait surface is small (subscribe,
broadcast, blobs) because iroh's API is already small. The `MemNetwork`
test double lets client and worker tests run without real QUIC
connections and without iroh as a dev-dependency.

**No more native/WASM split**: iroh's `Endpoint` handles platform
differences internally. The same code compiles for both targets.

### `willow-transport` (minimal changes)

The `Envelope` and `pack`/`unpack` functions are unchanged — they operate
on `Vec<u8>` and don't depend on the transport layer. The only change is
removing any libp2p type imports if present.

### `willow-relay` (replaced)

The custom relay binary is replaced. The new `crates/relay/` binary
runs two things:
1. **iroh-relay server** — packet forwarding for NAT traversal
2. **Bootstrap node** — a minimal gossip participant that subscribes
   to system topics so new peers can join the mesh

```rust
#[tokio::main]
async fn main() {
    let config = RelayConfig::from_args();

    // Start iroh relay for NAT traversal
    let relay = iroh_relay::Server::new(config.relay)
        .bind(config.relay_addr)
        .spawn().await?;

    // Start bootstrap gossip node alongside relay
    let bootstrap = IrohNetwork::new(config.bootstrap).await?;
    bootstrap.subscribe(SERVER_OPS_TOPIC, vec![]).await?;
    bootstrap.subscribe(WORKERS_TOPIC, vec![]).await?;
    bootstrap.subscribe(PROFILES_TOPIC, vec![]).await?;

    // Run until shutdown
    tokio::signal::ctrl_c().await?;
}
```

The bootstrap node is lightweight — it joins topics but doesn't
process messages. It exists so new peers have a known `EndpointId`
to bootstrap gossip against.

### `willow-client` (restructured)

The client is generic over `Network`. Production uses `IrohNetwork`,
tests use `MemNetwork`. No `NetworkCommand` / `NetworkEvent` enums —
the client calls trait methods directly.

```rust
use willow_network::{Network, TopicHandle, TopicEvents, GossipEvent};
use iroh_gossip::TopicId;
use iroh_blobs::Hash;

pub struct ClientHandle<N: Network> {
    network: Arc<N>,
    /// Active gossip topic handles, keyed by TopicId.
    topics: HashMap<TopicId, N::Topic>,
    /// Arc<RwLock> instead of Rc<RefCell> — Network: Send + Sync
    /// requires the client to be Send, and spawned topic listener
    /// tasks need shared access across threads/tasks.
    state: Arc<RwLock<SharedState>>,
}

impl<N: Network> ClientHandle<N> {
    pub async fn connect(network: N, identity: Identity) -> Result<Self> {
        let network = Arc::new(network);
        // Subscribe to system topics via trait
        let (ops_sender, ops_events) = network
            .subscribe(SERVER_OPS_TOPIC, bootstrap_peers)
            .await?;

        // Spawn listener task for incoming events
        spawn_topic_listener(ops_events, state.clone(), event_tx.clone());

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
        let hash = self.network.blobs().add(data.into()).await?;
        // Broadcast hash + endpoint ID over gossip
        self.topics[&topic].broadcast(announce_bytes.into()).await?;
        Ok(hash)
    }
}

/// Spawned per-topic: streams GossipEvents, applies state mutations,
/// emits ClientEvents to the UI layer.
async fn spawn_topic_listener<E: TopicEvents>(
    mut events: E,
    state: Arc<RwLock<SharedState>>,
    event_tx: UnboundedSender<ClientEvent>,
) {
    while let Some(Ok(gossip_event)) = events.next().await {
        match gossip_event {
            GossipEvent::Received(msg) => {
                let (wire_msg, from) = unpack_wire(&msg.content)?;
                match wire_msg {
                    WireMessage::Event(e) => {
                        apply_event(&mut state.write().unwrap(), e);
                        event_tx.send(ClientEvent::MessageReceived { .. });
                    }
                    // ...
                }
            }
            GossipEvent::NeighborUp(id) => { /* track peer */ }
            GossipEvent::NeighborDown(id) => { /* remove peer */ }
        }
    }
}
```

**Testing**:
```rust
#[tokio::test]
async fn send_message_broadcasts_to_topic() {
    let hub = MemHub::new();
    let net_a = MemNetwork::new(&hub);
    let net_b = MemNetwork::new(&hub);

    let client_a = ClientHandle::connect(net_a, identity_a).await?;
    let client_b = ClientHandle::connect(net_b, identity_b).await?;

    client_a.send_message("general", "hello").await?;

    // Message arrives at client_b via MemHub in-process broadcast
    let msg = client_b.next_event().await;
    assert_eq!(msg.body, "hello");
}
```

No real QUIC, no ports, no network I/O. Tests use `#[tokio::test]`
to drive the async trait methods, but `MemHub` delivers messages
in-process via broadcast channels — no actual networking happens.

### `willow-app` (out of scope)

The Bevy desktop app is not part of this migration. Focus is on the
Leptos web UI (`crates/web/`), which consumes `willow-client` directly.
The Bevy app can be migrated later using the same updated client library.

### `willow-worker` (restructured)

Workers are also generic over `Network`. The actor model (network,
state, heartbeat, sync) remains, but the network actor streams from
`TopicEvents` and writes via `TopicHandle`:

```rust
pub struct WorkerNode<N: Network> {
    network: Arc<N>,
    role: Box<dyn WorkerRole>,
    state_tx: mpsc::Sender<StateMsg>,
}

// Network actor: stream topic events via trait
async fn network_actor<E: TopicEvents, T: TopicHandle>(
    mut events: E,
    sender: T,
    state_tx: mpsc::Sender<StateMsg>,
) {
    while let Some(Ok(gossip_event)) = events.next().await {
        if let GossipEvent::Received(msg) = gossip_event {
            let (wire_msg, from) = unpack_wire(&msg.content)?;
            state_tx.send(StateMsg::from(wire_msg)).await?;
        }
    }
}
```

Worker tests use `MemNetwork` just like client tests — verify event
application, sync responses, and heartbeat logic without real
connections.

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

1. **Bootstrap node**: A lightweight gossip participant deployed
   alongside the relay. Its `EndpointId` is known at build time. All
   peers bootstrap gossip topics through it. It subscribes to system
   topics and acts as a rendezvous point but does not store or process
   messages — it exists solely so new peers can join the gossip mesh.

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
- `willow-network`: `Network` trait + `IrohNetwork` + `MemNetwork`
- `willow-state`: `Event.author` becomes `EndpointId`, `ServerState`
  member/permission maps key on `EndpointId`
- `willow-channel`: `Server.owner`, `Member.peer_id` → `EndpointId`
- `willow-messaging`: `Message.author` → `EndpointId`
- `willow-crypto`: X25519 derivation from iroh `SecretKey`

The `String` → `EndpointId` changes across state/channel/messaging/crypto
are mechanical and independent from the `willow-network` rewrite.
Work both tracks simultaneously.

**Test**: Identity sign/verify, state apply/merge, `MemNetwork`
round-trips, `IrohNetwork` endpoint creation on localhost.
**Risk**: Medium — touches state types, but it's a clean break so no
compatibility concerns.

### Phase 2: Client + Web UI

Make `willow-client` generic over `Network`. Wire up `IrohNetwork` for
production and `MemNetwork` for tests. Port existing client tests to
use `MemNetwork`. Wire the Leptos web UI to the new async-native client.

**Test**: All existing client tests ported to `MemNetwork`, new gossip
round-trip tests, web UI integration.
**Risk**: Medium — largest behavioral change, but `MemNetwork` lets us
validate everything without real connections.

### Phase 3: Relay + Workers

Replace `willow-relay` with iroh relay wrapper + bootstrap node.
Restructure worker network actor to use `TopicEvents` / `TopicHandle`
traits (same pattern as client).

**Test**: Relay tests, worker tests, scaling tests.
**Risk**: Low — relay is stateless, workers follow same pattern as
client.

### Phase 4: Cleanup

- Remove all libp2p dependencies from `Cargo.toml` workspace
- Delete `willow-files` crate (replaced by `iroh-blobs`)
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
  Persistent blob caching on WASM would need IndexedDB integration.
  Include stubs in Phase 1 so the path is clear:

  ```rust
  /// Platform-aware blob store. Uses MemStore on WASM, FsStore on native.
  #[cfg(target_arch = "wasm32")]
  pub type PlatformBlobStore = MemBlobStore;

  #[cfg(not(target_arch = "wasm32"))]
  pub type PlatformBlobStore = FsBlobStore;

  /// WASM blob store backed by in-memory HashMap.
  /// TODO: Replace with IndexedDB-backed store for persistence across
  /// page reloads. Implementation plan:
  ///
  /// 1. Add `idb` crate dependency (IndexedDB wrapper for wasm-bindgen)
  /// 2. Create `IdbBlobStore` implementing `BlobStore` trait:
  ///    - Object store: "blobs", keyed by Hash (hex string)
  ///    - add(): put blob bytes into object store, return Hash
  ///    - get(): fetch by hash key, return Option<Bytes>
  ///    - has(): key existence check via count()
  /// 3. Add LRU eviction when store exceeds configurable size limit
  ///    (browser storage quota is ~50-100 MB depending on browser)
  /// 4. Wire into PlatformBlobStore via cfg(target_arch = "wasm32")
  /// 5. Add browser test: add blob, reload page, verify blob persists
  pub struct MemBlobStore {
      store: Mutex<HashMap<Hash, Bytes>>,
  }
  ```
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
The bootstrap node must be running and reachable for gossip to work —
same as today. Its `EndpointId` is baked into the client build config.

For `just dev`, the relay starts first and workers/web connect after.
For production, the relay is a long-running systemd service. If the
relay goes down, peers already connected to each other via HyParView
continue to gossip directly — only new topic joins fail.

Worker nodes provide additional bootstrap redundancy. If the relay is
unreachable but a worker is, peers can bootstrap through the worker.

## Testing Strategy

The existing test suite has ~665 tests across 7 tiers. The migration
must preserve coverage at every tier, porting tests to the new
abstractions rather than dropping them.

### Tier 1: State Machine (63 tests — unchanged)

`crates/state/src/tests.rs` — pure event application, merge, permissions.

**Impact**: `Event.author` and `ServerState` member keys change from
`String` to `EndpointId`. Tests update to use `EndpointId` values
instead of string literals like `"owner"` and `"alice"`.

```rust
// Before
let event = event(&state, "e1", "alice", EventKind::Message { .. });

// After
let alice = Identity::generate().endpoint_id();
let event = event(&state, "e1", alice, EventKind::Message { .. });
```

The `test_state()` helper generates an `Identity` for the owner and
returns both the state and the owner's `EndpointId`. Test assertions
use `EndpointId` comparison instead of string comparison.

**No networking involved** — these tests stay fast and deterministic.

### Tier 2: Client API (93 tests — ported to MemNetwork)

`crates/client/src/lib.rs` test module.

**Current**: `test_client()` creates a `ClientHandle` with a captured
`mpsc::Receiver<NetworkCommand>` — no real networking. Tests verify
that calling `send_message()` produces the right `NetworkCommand`.

**After**: `test_client()` creates a `ClientHandle<MemNetwork>` with
a `MemHub`. Tests verify actual behavior — messages sent by client A
arrive at client B through the in-process hub:

```rust
async fn test_client_pair() -> (ClientHandle<MemNetwork>, ClientHandle<MemNetwork>) {
    let hub = MemHub::new();
    let a = ClientHandle::connect(MemNetwork::new(&hub), Identity::generate()).await?;
    let b = ClientHandle::connect(MemNetwork::new(&hub), Identity::generate()).await?;
    (a, b)
}

#[tokio::test]
async fn send_message_delivered() {
    let (alice, bob) = test_client_pair().await;
    alice.send_message("general", "hello").await?;
    let event = bob.next_event().await;
    assert!(matches!(event, ClientEvent::MessageReceived { .. }));
}
```

This is strictly better than the current approach — tests verify
end-to-end behavior through the gossip abstraction, not just that
the right command enum variant was produced.

**What MemHub provides**:
- Deterministic message delivery (no timing, no flakes)
- Multiple isolated hubs per test (no cross-test interference)
- Neighbor tracking (NeighborUp/Down events fire on subscribe)
- Optional: configurable message loss for chaos testing

### Tier 3: Browser / Leptos (39 tests — minimal changes)

`crates/web/tests/browser.rs` — DOM rendering via `wasm_bindgen_test`.

**Impact**: Minimal. These tests render Leptos components with mock
data (`DisplayMessage` structs). They don't touch networking. The only
change is `DisplayMessage.author_peer_id` becomes an `EndpointId`
display string instead of a libp2p `PeerId` string.

### Tier 4: Network Integration (new — replaces libp2p integration)

Currently `crates/app/tests/integration.rs` — real libp2p nodes on
localhost TCP. These are **deleted and rewritten** against iroh.

New location: `crates/network/tests/integration.rs`

```rust
#[tokio::test]
async fn two_nodes_gossip_round_trip() {
    let a = IrohNetwork::new(test_config()).await?;
    let b = IrohNetwork::new(test_config()).await?;

    let topic = topic_id("test-topic");
    let (sender_a, _) = a.subscribe(topic, vec![b.id()]).await?;
    let (_, mut events_b) = b.subscribe(topic, vec![a.id()]).await?;

    events_b.joined().await?;
    sender_a.broadcast("hello".into()).await?;

    match events_b.next().await {
        Some(Ok(GossipEvent::Received(msg))) => {
            assert_eq!(msg.content.as_ref(), b"hello");
            assert_eq!(msg.sender, a.id());
        }
        other => panic!("expected Received, got {:?}", other),
    }
}
```

These tests use **real iroh endpoints on localhost** — they validate
that `IrohNetwork` correctly assembles the iroh stack and that gossip
actually works over QUIC. They replace the libp2p integration tests
1:1.

**Tests to write**:
- Two nodes connect and exchange gossip messages
- Topic isolation (messages on topic A don't appear on topic B)
- Blob add + get round-trip between two nodes
- Node disconnect fires NeighborDown
- Multiple topics on same endpoint
- Relay-mediated connection (requires local iroh-relay in test)

### Tier 5: Scaling (7 tests — ported to iroh)

Currently `crates/app/tests/peer_scale.rs` — N real nodes in star
topology measuring connection time and message delivery.

Ported to use `IrohNetwork` instead of libp2p `NetworkNode`. The test
structure stays the same — create N nodes, dial into a hub, measure
latency. Thresholds may need adjustment since iroh's QUIC connections
have different latency characteristics than libp2p TCP+Noise+Yamux.

```rust
#[tokio::test]
async fn scale_10_peers_connect() {
    let hub = IrohNetwork::new(test_config()).await?;
    let mut peers = vec![];
    for _ in 0..9 {
        let peer = IrohNetwork::new(test_config()).await?;
        // Subscribe to shared topic with hub as bootstrap
        peer.subscribe(topic, vec![hub.id()]).await?;
        peers.push(peer);
    }
    // Verify all peers see each other as neighbors
}
```

### Tier 6: Worker (existing tests — ported to MemNetwork)

`crates/worker/tests/integration.rs` — actor message passing.

Workers become generic over `Network`. Worker tests use `MemNetwork`
to verify:
- State actor ingests events from gossip
- Sync requests produce correct batches
- Heartbeat actor broadcasts announcements
- Concurrent requests resolve correctly

Same test logic, swap `MemNetwork` for the current mock setup.

### Tier 7: E2E State Convergence (existing — unchanged)

`crates/app/tests/e2e_flow.rs` pure state machine tests (lines 100-394).

These create 3 `ServerState` instances and apply events directly to
simulate concurrent peers. No networking. Only change is `String` →
`EndpointId` for author fields. These tests are the most valuable
correctness tests in the codebase and are completely unaffected by the
networking migration.

### MemHub Design

The `MemHub` is the core test primitive. It simulates an in-process
gossip network with deterministic delivery:

```rust
/// Shared in-process gossip mesh for testing.
pub struct MemHub {
    /// Per-topic broadcast channels.
    topics: Mutex<HashMap<TopicId, broadcast::Sender<(EndpointId, Bytes)>>>,
}

impl MemHub {
    pub fn new() -> Arc<Self>;
}

/// Test network backed by MemHub. No real connections.
pub struct MemNetwork {
    id: EndpointId,
    hub: Arc<MemHub>,
    blobs: MemBlobStore,
}

/// In-memory blob store for tests.
pub struct MemBlobStore {
    store: Mutex<HashMap<Hash, Bytes>>,
}
```

**Behavior**:
- `subscribe(topic, _bootstrap)` — registers with the hub's broadcast
  channel for that topic. Bootstrap peers are ignored (everyone is
  already "connected" through the hub).
- `broadcast(data)` — sends `(sender_id, data)` to all subscribers
  on that topic via the broadcast channel.
- `next()` — receives from the broadcast channel. Filters out
  messages from self (same as real gossip).
- `NeighborUp` — fired for all existing subscribers when a new peer
  joins a topic.
- `NeighborDown` — fired when a `MemNetwork` is dropped.
- Blob add/get — simple HashMap insert/lookup.

**Properties**:
- Deterministic: messages arrive in send order, no timing variance
- Isolated: each `MemHub` instance is independent
- Fast: needs `#[tokio::test]` for async trait methods but all I/O is
  in-process channel sends — sub-millisecond per test
- Correct: mirrors the real gossip semantics closely enough that
  tests catching bugs in MemNetwork also catch them in IrohNetwork

### Test Migration Checklist

| Test file | Current count | Migration action |
|---|---|---|
| `crates/state/src/tests.rs` | 63 | Update `String` → `EndpointId` |
| `crates/client/src/lib.rs` | 93 | Port to `ClientHandle<MemNetwork>` |
| `crates/web/tests/browser.rs` | 39 | Minimal — update display types |
| `crates/app/tests/e2e_flow.rs` (state) | 5 | Update `String` → `EndpointId` |
| `crates/app/tests/integration.rs` | 14 | Rewrite against `IrohNetwork` |
| `crates/app/tests/peer_scale.rs` | 7 | Port to `IrohNetwork` |
| `crates/worker/tests/integration.rs` | ~5 | Port to `MemNetwork` |
| `e2e/*.spec.ts` | ~40 | Update relay startup in helpers |
| `crates/app/src/tests.rs` | 99 | Out of scope (Bevy) |

### Tier 8: Playwright E2E (existing — ported to iroh relay)

`e2e/*.spec.ts` — multi-peer sync, permissions, mobile UI tests.

These spin up the full `just dev` stack and test real browser-to-browser
communication. They need the iroh relay running instead of the libp2p
relay. The test helpers (`setupTwoPeers`, `sendMessage`, etc.) are
unchanged — they interact with the Leptos UI, not the network layer
directly.

**Action**: Update `e2e/helpers.ts` to start the iroh relay wrapper
instead of `willow-relay`. Everything else should work as-is since
the tests operate at the UI level.

**Total**: ~266 tests to port/rewrite (excluding Bevy). The Bevy app
tests (99) are out of scope for this migration but remain functional
until the Bevy app is migrated separately.

### Validation Gates

Each migration phase has a gate before proceeding:

- **Phase 1 gate**: `just test-state` passes (63 tests with
  `EndpointId`). `MemNetwork` round-trip tests pass. `IrohNetwork`
  connects two localhost nodes and exchanges a gossip message.

- **Phase 2 gate**: All 93 client tests pass with `MemNetwork`.
  Leptos browser tests pass (39). New multi-client gossip tests pass.

- **Phase 3 gate**: Relay starts and two `IrohNetwork` nodes connect
  through it. Worker tests pass with `MemNetwork`. Scaling tests
  pass with `IrohNetwork` (thresholds adjusted if needed).

- **Phase 4 gate**: `just check` passes with zero warnings. No
  libp2p imports remain. WASM build succeeds (`just check-wasm`).

## Additional Crate Impacts

### `willow-crypto` (modified)

Currently derives X25519 Diffie-Hellman keys from Ed25519 keys via
libp2p's identity types. After migration, derive from iroh's
`SecretKey` instead:

```rust
// Before: libp2p keypair → ed25519 bytes → X25519
// After:  iroh SecretKey → ed25519 bytes → X25519
let ed_bytes = identity.secret_key().to_bytes();
let x25519_secret = x25519_dalek::StaticSecret::from(
    ed25519_to_x25519(&ed_bytes)
);
```

The underlying Ed25519→X25519 conversion is the same algorithm
(clamped SHA-512 of the seed). Only the wrapper type changes. E2E
encryption (ChaCha20-Poly1305) is unaffected.

### `willow-channel` and `willow-messaging` (modified)

Both crates use `String` for peer identifiers internally:
- `willow-channel`: `Server.owner`, `Member.peer_id`, role assignments
- `willow-messaging`: `Message.author`, `HLC` node identifiers

These change to `EndpointId` (or a serializable wrapper). Since these
crates don't depend on libp2p directly, the change is mechanical —
swap `String` fields to `EndpointId`, update constructors and accessors.

### `willow-common` / wire format (modified)

`pack_wire` and `unpack_wire` sign/verify with `willow_identity`. The
signature bytes change format because iroh's `SecretKey::sign()` may
produce a different envelope than libp2p's `Keypair::sign()`. Both use
Ed25519 signatures (64 bytes), but the signed payload structure may
differ.

**Decision**: Keep the existing signed envelope format (hash payload,
sign hash, prepend signature + public key bytes). Just swap the
signing/verification calls to use iroh types. The wire format stays
compatible across versions since it's our own envelope, not libp2p's.

### `EndpointId` serialization

`EndpointId` (= `PublicKey`) needs consistent serialization across
wire protocol, state persistence, and display:

- **Wire / persistence**: 32 raw bytes (compact, used in bincode
  serialization of `Event`, `ServerState`, etc.)
- **Display**: hex string via iroh's `Display` impl (64 chars) for
  logs, UI display names, debug output
- **Short display**: `fmt_short()` (first 5 bytes as hex, 10 chars)
  for UI peer badges

iroh's `PublicKey` already implements `Serialize`/`Deserialize` (raw
32 bytes for binary formats, hex string for human-readable formats).
This works with bincode (wire) and JSON (debug/config) out of the box.

## Voice / WebRTC Signaling

Voice signaling (`VoiceJoin`, `VoiceLeave`, `VoiceSignal`) currently
uses gossipsub topics. This maps directly to iroh-gossip — voice
signals are just gossip messages on a voice-specific `TopicId`:

```rust
fn voice_topic(server_id: &str, channel_id: &str) -> TopicId {
    topic_id(&format!("{server_id}/{channel_id}/voice"))
}
```

No protocol change needed. The signaling messages are small (SDP
offers/answers, ICE candidates) — well within the 64 KiB gossip limit.
WebRTC data channels are established peer-to-peer after signaling and
don't go through iroh.

## Reconnection and Resilience

### Relay Disconnection

iroh's `Endpoint` handles relay reconnection internally. If the relay
drops, the endpoint automatically attempts to re-establish the relay
connection with exponential backoff. Direct peer connections (via hole
punching) are unaffected by relay outages.

### Topic Subscription Recovery

If the underlying connection drops and recovers, gossip topic
subscriptions need to be re-established. The `Network` trait exposes
`connection_events()` (see trait definition above). The client spawns
a reconnection task that watches for `RelayDisconnected` and
re-subscribes to all topics in the `topics` map when `RelayConnected`
fires. HyParView handles re-joining the gossip mesh automatically
once the topic subscription is re-established.

### WASM-Specific

The current WASM reconnection loop (backoff + retry) is replaced by
iroh's built-in relay reconnection. No custom reconnection code needed
in the client — iroh handles it at the transport level.

## `just dev` Changes

The `justfile` dev stack updates:

```
# Before
just dev → relay (willow-relay) + replay worker + storage worker + trunk serve

# After
just dev → relay (iroh-relay wrapper) + replay worker + storage worker + trunk serve
```

The relay binary changes from `willow-relay` to the new iroh-relay
wrapper in `crates/relay/`. Startup script updates to pass iroh-relay
flags instead of libp2p multiaddrs. Worker and web startup remain the
same (they consume `willow-client` which handles the network layer).

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

   Include GC stubs in the `BlobStore` trait from day one:

   ```rust
   #[async_trait]
   pub trait BlobStore: Send + Sync {
       async fn add(&self, data: Bytes) -> Result<Hash>;
       async fn get(&self, hash: Hash) -> Result<Option<Bytes>>;
       async fn has(&self, hash: Hash) -> bool;

       /// Remove a blob from the store. Returns true if it existed.
       /// TODO: Called by GC strategies below. No-op on MemStore.
       async fn remove(&self, hash: Hash) -> Result<bool>;

       /// Current store size in bytes. Returns None if unsupported.
       /// TODO: Used by size-cap GC to decide when to evict.
       async fn store_size(&self) -> Option<u64>;
   }

   /// TODO: Blob GC implementation plan:
   ///
   /// 1. Add `BlobGc` struct that wraps a `BlobStore` + config:
   ///    ```
   ///    pub struct BlobGc<S: BlobStore> {
   ///        store: S,
   ///        max_size: u64,           // e.g. 1 GB
   ///        check_interval: Duration, // e.g. 5 minutes
   ///    }
   ///    ```
   ///
   /// 2. GC loop (spawned as background task on workers):
   ///    - Poll store_size() on interval
   ///    - If over max_size, list blobs by last-access time
   ///    - Remove oldest blobs until under 80% of max_size
   ///    - Log evictions for debugging
   ///
   /// 3. FsStore integration:
   ///    - iroh-blobs FsStore (redb) supports delete via its API
   ///    - Track last-access timestamps in a separate redb table
   ///    - Update timestamp on get(), don't update on has()
   ///
   /// 4. MemStore: remove() deletes from HashMap. store_size()
   ///    returns sum of value byte lengths.
   ///
   /// 5. Worker CLI flag: --max-blob-store-size <bytes>
   ///    Default: 1 GB for replay nodes, 10 GB for file nodes
   ///
   /// 6. Tests:
   ///    - Add blobs until over limit, verify oldest evicted
   ///    - Verify recently-accessed blobs survive GC
   ///    - Verify GC runs on interval without blocking operations
   ///    - Verify remove() returns false for missing hash
   ```
