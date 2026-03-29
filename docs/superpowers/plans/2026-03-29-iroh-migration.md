# Iroh Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace libp2p with iroh as Willow's networking layer. Iroh-shaped trait abstraction (`Network`, `TopicHandle`, `TopicEvents`, `BlobStore`) with `IrohNetwork` for production and `MemNetwork` for tests. `EndpointId` replaces `String` for peer identity throughout. Clean break — no backward compatibility.

**Tech Stack:** Rust, iroh (0.97), iroh-gossip (0.97), iroh-blobs (0.99), iroh-relay (0.97), tokio, blake3

**Spec:** `docs/superpowers/specs/2026-03-29-iroh-migration-design.md`

---

## Phase 1: Foundation (identity + network + state + supporting crates)

Two parallel tracks: (A) willow-network rewrite, (B) String → EndpointId across crates. Both depend on willow-identity being done first.

### File Map

#### Modified Crates

```
crates/identity/
├── Cargo.toml             — Replace libp2p dep with iroh-base
└── src/lib.rs             — Rewrite: wrap iroh SecretKey, expose EndpointId

crates/network/
├── Cargo.toml             — Replace libp2p deps with iroh, iroh-gossip, iroh-blobs
└── src/
    ├── lib.rs             — Module exports, re-exports
    ├── traits.rs          — NEW: Network, TopicHandle, TopicEvents, BlobStore,
    │                        ConnectionEvent traits
    ├── iroh.rs            — NEW: IrohNetwork, Config, IrohTopicHandle,
    │                        IrohTopicEvents, IrohBlobStore impls
    ├── mem.rs             — NEW: MemNetwork, MemHub, MemTopicHandle,
    │                        MemTopicEvents, MemBlobStore (test-utils feature)
    ├── topics.rs          — NEW: TopicId registry (topic_id(), system consts,
    │                        channel_topic(), voice_topic())
    ├── behaviour.rs       — DELETE
    ├── node.rs            — DELETE
    ├── config.rs          — DELETE
    └── file_transfer.rs   — DELETE

crates/state/
├── Cargo.toml             — Add iroh-base dep (for EndpointId)
└── src/
    ├── lib.rs             — Event.author: String → EndpointId,
    │                        apply()/apply_lenient() updated
    ├── server.rs          — ServerState: owner, members, peer_permissions,
    │                        profiles keys → EndpointId
    ├── hash.rs            — StateHash computation updated for EndpointId
    ├── merge.rs           — Merge author comparisons → EndpointId
    ├── store.rs           — EventStore trait unchanged (events carry EndpointId)
    ├── types.rs           — Channel, ChatMessage, Member, Role, Profile
    │                        peer fields → EndpointId
    └── tests.rs           — Update all 63 tests: string authors → EndpointId

crates/channel/
├── Cargo.toml             — Add iroh-base dep
└── src/lib.rs             — Server.owner, Member.peer_id, role assignments
                             → EndpointId

crates/messaging/
├── Cargo.toml             — Add iroh-base dep
└── src/
    ├── lib.rs             — Message.author → EndpointId
    └── hlc.rs             — HLC node id → EndpointId

crates/crypto/
├── Cargo.toml             — Replace libp2p identity dep with iroh-base
└── src/lib.rs             — X25519 derivation from iroh SecretKey

crates/transport/
├── Cargo.toml             — Remove any libp2p imports
└── src/lib.rs             — Envelope unchanged, remove libp2p type refs

crates/common/
├── Cargo.toml             — Update identity dep
└── src/wire.rs            — pack_wire/unpack_wire: sign/verify with iroh types,
                             peer id extraction → EndpointId
```

#### Workspace Root

```
Cargo.toml                 — Add iroh workspace deps, keep libp2p for now
                             (client/app/worker still depend until Phase 2-3)
```

---

### Steps

#### 1.1 — willow-identity rewrite

- [ ] Update `crates/identity/Cargo.toml`: replace `libp2p` dep with `iroh-base`
- [ ] Rewrite `crates/identity/src/lib.rs`:
  - `Identity` wraps `iroh_base::SecretKey`
  - `generate()` → `SecretKey::generate()`
  - `from_bytes()` / `to_bytes()` → SecretKey serialization
  - `endpoint_id()` → `EndpointId` (= `PublicKey`)
  - `secret_key()` → `&SecretKey`
  - `public_key()` → `PublicKey`
  - `sign()` → `SecretKey::sign()`
  - Standalone `verify()` function → `PublicKey::verify()`
  - Re-export `EndpointId`, `PublicKey`, `SecretKey`, `Signature`
  - Remove `peer_id() -> String` (breaking change, intentional)
- [ ] Update identity tests: sign/verify round-trip, serialization round-trip, generate produces unique keys
- [ ] Verify: `cargo test -p willow-identity`

#### 1.2 — willow-network traits (parallel with 1.3+)

- [ ] Update `crates/network/Cargo.toml`: add `iroh`, `iroh-base`, `iroh-gossip`, `iroh-blobs`, `bytes`, `async-trait`, `blake3` deps. Add `test-utils` feature flag. Keep old deps temporarily (removed in 1.5).
- [ ] Create `crates/network/src/traits.rs`:
  - `TopicHandle` trait: `broadcast()`, `broadcast_neighbors()`, `neighbors()`
  - `GossipMessage` struct: `content: Bytes`, `sender: EndpointId`
  - `GossipEvent` enum: `Received`, `NeighborUp`, `NeighborDown`
  - `TopicEvents` trait: `next()`, `joined()`
  - `BlobStore` trait: `add()`, `get()`, `has()`, `remove()`, `store_size()`
  - `ConnectionEvent` enum: `RelayConnected`, `RelayDisconnected`, `DirectConnected`, `DirectDisconnected`
  - `Network` trait: `id()`, `subscribe()`, `unsubscribe()`, `blobs()`, `connection_events()`, `shutdown()`
- [ ] Create `crates/network/src/topics.rs`:
  - `topic_id(name: &str) -> TopicId` using blake3
  - `SERVER_OPS_TOPIC`, `WORKERS_TOPIC`, `PROFILES_TOPIC` constants
  - `channel_topic(server_id, channel_id) -> TopicId`
  - `voice_topic(server_id, channel_id) -> TopicId`
- [ ] Update `crates/network/src/lib.rs`: export traits, topics, feature-gate test-utils
- [ ] Verify: `cargo check -p willow-network` (traits compile)

#### 1.3 — MemNetwork test double

- [ ] Create `crates/network/src/mem.rs` (behind `test-utils` feature):
  - `MemHub`: `Arc<Mutex<HashMap<TopicId, broadcast::Sender<...>>>>`, `new() -> Arc<Self>`
  - `MemNetwork`: `id: EndpointId`, `hub: Arc<MemHub>`, `blobs: MemBlobStore`
  - `MemNetwork` impl `Network`: subscribe registers with hub channel, unsubscribe drops
  - `MemTopicHandle` impl `TopicHandle`: broadcast sends `(id, data)` on channel
  - `MemTopicEvents` impl `TopicEvents`: receives from broadcast channel, filters self, synthesizes NeighborUp/Down
  - `MemBlobStore` impl `BlobStore`: `HashMap<Hash, Bytes>`, `remove()` deletes, `store_size()` sums lengths
  - `MemNetwork::connection_events()` returns a stream that never yields (always connected)
- [ ] Add tests in `crates/network/src/mem.rs`:
  - Two MemNetworks on same hub: broadcast delivers to other, not self
  - Topic isolation: message on topic A not seen on topic B
  - NeighborUp fires when second peer subscribes
  - NeighborDown fires when MemNetwork drops
  - BlobStore add/get/has/remove round-trip
  - store_size() returns correct byte count
- [ ] Verify: `cargo test -p willow-network --features test-utils`

#### 1.4 — IrohNetwork implementation

- [ ] Create `crates/network/src/iroh.rs`:
  - `Config` struct: `secret_key`, `relay_url`, `bootstrap_peers`, `mdns`
  - `IrohNetwork::new(config)`: build `Endpoint` (with preset, secret key, relay, mdns), create `Gossip` (max message size 64 KiB), create `BlobsProtocol` (MemStore for now), build `Router` with gossip + blobs ALPNs, spawn router
  - `IrohNetwork` impl `Network`: subscribe delegates to `gossip.subscribe()`, unsubscribe tracks and drops topic handles
  - `IrohTopicHandle` wraps `iroh_gossip::GossipSender`, impl `TopicHandle`
  - `IrohTopicEvents` wraps `iroh_gossip::GossipReceiver` stream, impl `TopicEvents` by mapping `iroh_gossip::Event` → `GossipEvent`
  - `IrohBlobStore` wraps `iroh_blobs::Store`, impl `BlobStore`
  - `connection_events()`: monitor endpoint relay status, emit ConnectionEvents
- [ ] Add integration tests in `crates/network/tests/integration.rs`:
  - Two IrohNetwork nodes on localhost: gossip round-trip
  - Topic isolation
  - Blob add on node A, get on node B
  - NeighborDown on node disconnect
  - Multiple topics on same endpoint
- [ ] Verify: `cargo test -p willow-network` (all tests including integration)

#### 1.5 — Delete old network code

- [ ] Delete `crates/network/src/behaviour.rs`
- [ ] Delete `crates/network/src/node.rs`
- [ ] Delete `crates/network/src/config.rs`
- [ ] Delete `crates/network/src/file_transfer.rs`
- [ ] Remove old libp2p deps from `crates/network/Cargo.toml`
- [ ] Verify: `cargo check -p willow-network`

#### 1.6 — willow-state: String → EndpointId

- [ ] Update `crates/state/Cargo.toml`: add `iroh-base` dep
- [ ] Update `crates/state/src/types.rs`: all peer ID fields `String` → `EndpointId`
  - `ChatMessage.author`, `Member.peer_id`, `Profile` keys, `Reaction` author
- [ ] Update `crates/state/src/server.rs`:
  - `ServerState.owner: String` → `EndpointId`
  - `ServerState.members: HashMap<String, _>` → `HashMap<EndpointId, _>`
  - `ServerState.peer_permissions` → `HashMap<EndpointId, _>`
  - `ServerState.profiles` → `HashMap<EndpointId, _>`
  - `has_permission()`, `is_sync_provider()`, `is_trusted()` → `EndpointId` params
- [ ] Update `crates/state/src/lib.rs`:
  - `Event.author: String` → `EndpointId`
  - `apply_inner()`: all author checks use `EndpointId`
- [ ] Update `crates/state/src/hash.rs`: StateHash computation uses EndpointId serialization (32 bytes)
- [ ] Update `crates/state/src/merge.rs`: author comparisons → `EndpointId`
- [ ] Update `crates/state/src/store.rs`: EventStore trait unchanged (events carry EndpointId internally)
- [ ] Update `crates/state/src/tests.rs` (63 tests):
  - `test_state()` generates Identity, returns `(ServerState, EndpointId)` for owner
  - `event()` / `event_with()` helpers take `EndpointId` for author
  - All string literal authors (`"owner"`, `"alice"`, `"bob"`) → `Identity::generate().endpoint_id()`
  - Assertions compare `EndpointId` values
- [ ] Verify: `cargo test -p willow-state`

#### 1.7 — Supporting crates: channel, messaging, crypto, transport, common

- [ ] Update `crates/channel/Cargo.toml`: add `iroh-base`
- [ ] Update `crates/channel/src/lib.rs`: `Server.owner`, `Member.peer_id`, role assignment peer fields → `EndpointId`
- [ ] Update channel tests
- [ ] Update `crates/messaging/Cargo.toml`: add `iroh-base`
- [ ] Update `crates/messaging/src/lib.rs`: `Message.author` → `EndpointId`
- [ ] Update `crates/messaging/src/hlc.rs`: HLC node ID → `EndpointId`
- [ ] Update messaging tests
- [ ] Update `crates/crypto/Cargo.toml`: replace libp2p identity dep with `iroh-base`
- [ ] Update `crates/crypto/src/lib.rs`: X25519 derivation from `iroh_base::SecretKey::to_bytes()`
- [ ] Update crypto tests
- [ ] Update `crates/transport/Cargo.toml`: remove any libp2p deps
- [ ] Update `crates/transport/src/lib.rs`: remove any libp2p type references
- [ ] Update `crates/common/Cargo.toml`: update identity dep
- [ ] Update `crates/common/src/wire.rs`: `pack_wire()` / `unpack_wire()` sign/verify with iroh types, extract `EndpointId` from signature
- [ ] Update common tests
- [ ] Verify all:
  ```
  cargo test -p willow-channel
  cargo test -p willow-messaging
  cargo test -p willow-crypto
  cargo test -p willow-transport
  cargo test -p willow-common
  ```

#### 1.8 — Phase 1 validation gate

- [ ] `cargo test -p willow-identity` — all pass
- [ ] `cargo test -p willow-state` — 63 tests pass with `EndpointId`
- [ ] `cargo test -p willow-network --features test-utils` — MemNetwork tests pass
- [ ] `cargo test -p willow-network` — IrohNetwork integration tests pass (two localhost nodes exchange gossip)
- [ ] `cargo test -p willow-channel && cargo test -p willow-messaging && cargo test -p willow-crypto && cargo test -p willow-transport && cargo test -p willow-common` — all supporting crates pass
- [ ] `cargo check -p willow-network --target wasm32-unknown-unknown` — WASM compiles

---

## Phase 2: Client + Web UI

Make `willow-client` generic over `Network`. Drop the `NetworkCommand`/`NetworkEvent` enum indirection. The client calls `Network` trait methods directly and spawns per-topic listener tasks. Port all 93 client tests to `MemNetwork`. Wire the Leptos web UI to the restructured client.

### File Map

#### Modified Crates

```
crates/client/
├── Cargo.toml                 — Add willow-network dep, remove libp2p/futures-mpsc
└── src/
    ├── lib.rs                 — ClientHandle<N: Network>, ClientEventLoop removed,
    │                            connect() creates Network + subscribes topics,
    │                            send_message/create_channel etc. call TopicHandle directly
    ├── network.rs             — DELETE (NetworkCommand, NetworkEvent, spawn_network gone)
    ├── listeners.rs           — NEW: spawn_topic_listener(), process_gossip_event(),
    │                            reconnection_task()
    ├── state.rs               — SharedState uses Arc<RwLock<>> instead of Rc<RefCell<>>,
    │                            ServerContext.topic_map keys → TopicId,
    │                            PersistentEventStore unchanged
    ├── events.rs              — ClientEvent peer fields: String → EndpointId
    ├── ops.rs                 — Remove re-exports of old wire types,
    │                            use willow_network::topics for TopicId constants
    ├── files.rs               — File sharing via BlobStore trait instead of
    │                            NetworkCommand::ShareFile
    ├── storage.rs             — Unchanged (persistence backends)
    └── worker_cache.rs        — Worker peer fields: String → EndpointId

crates/web/
└── src/
    ├── app.rs                 — Create IrohNetwork, pass to ClientHandle::connect(),
    │                            event loop consumes ClientEvent stream (same pattern,
    │                            new generic type)
    ├── state.rs               — Signal peer_id fields → EndpointId display format
    ├── event_processing.rs    — process_event_batch: EndpointId for peer fields
    └── components/*.rs        — Peer ID display: fmt_short() instead of truncated PeerId
```

### Steps

#### 2.1 — Restructure ClientHandle as generic

- [ ] Update `crates/client/Cargo.toml`: add `willow-network` dep (with `test-utils` as dev feature), remove `libp2p`, remove `futures` channel deps
- [ ] Rewrite `ClientHandle` in `crates/client/src/lib.rs`:
  - `pub struct ClientHandle<N: Network>` with `network: Arc<N>`, `topics: HashMap<TopicId, N::Topic>`, `state: Arc<RwLock<SharedState>>`, `identity: Identity`, `event_tx: UnboundedSender<ClientEvent>`
  - `connect(network: N, identity: Identity, config: ClientConfig) -> Result<(Self, UnboundedReceiver<ClientEvent>)>` — subscribes to system topics, spawns listeners
  - All command methods (`send_message`, `create_channel`, `trust_peer`, etc.) call `self.topics[&topic].broadcast()` directly instead of `cmd_tx.send(NetworkCommand::...)`
  - `subscribe_channel(topic_id)` / `unsubscribe_channel(topic_id)` — manage per-channel topic subscriptions
- [ ] Update `SharedState` to use `Arc<RwLock<>>` instead of `Rc<RefCell<>>`
- [ ] Update `ServerContext.topic_map` keys from `String` to `TopicId`
- [ ] Verify: `cargo check -p willow-client` (compiles with new generics)

#### 2.2 — Topic listener system

- [ ] Create `crates/client/src/listeners.rs`:
  - `spawn_topic_listener<E: TopicEvents>(events, state, event_tx, identity)` — spawns async task that:
    - Calls `events.next()` in loop
    - On `GossipEvent::Received`: calls `unpack_wire()`, routes by `WireMessage` variant:
      - `Event(e)` → `apply_event()` on state, emit `ClientEvent::MessageReceived` etc.
      - `SyncRequest` → build and broadcast `SyncBatch` response
      - `SyncBatch` → apply events, emit `ClientEvent::SyncCompleted`
      - `TypingIndicator` → emit typing event
      - `VoiceJoin/Leave/Signal` → emit corresponding `ClientEvent`
      - `JoinRequest/Response/Denied` → emit corresponding `ClientEvent`
    - On `GossipEvent::NeighborUp` → emit `ClientEvent::PeerConnected`
    - On `GossipEvent::NeighborDown` → emit `ClientEvent::PeerDisconnected`
  - `spawn_reconnection_task(network, topics, state)` — watches `connection_events()`, re-subscribes on relay reconnect
- [ ] Verify: `cargo check -p willow-client`

#### 2.3 — File sharing via BlobStore

- [ ] Update `crates/client/src/files.rs`:
  - `share_file(network, topic_sender, filename, mime_type, data)`:
    - `network.blobs().add(data)` → get `Hash`
    - Broadcast file announcement (hash, filename, mime_type, size, endpoint_id) over gossip
  - `download_file(network, hash)`:
    - `network.blobs().get(hash)` → return bytes
  - Remove `FileManager` and chunk-based logic (replaced by iroh-blobs)
- [ ] Update `ClientHandle::share_file()` to call new file module
- [ ] Verify: `cargo check -p willow-client`

#### 2.4 — Delete old network module

- [ ] Delete `crates/client/src/network.rs` (NetworkCommand, NetworkEvent, spawn_network)
- [ ] Remove all `NetworkCommand` / `NetworkEvent` references from lib.rs
- [ ] Update ops.rs: remove old wire type re-exports, use `willow_network::topics` for constants
- [ ] Update events.rs: `ClientEvent` peer fields from `String` to `EndpointId`
- [ ] Update worker_cache.rs: peer fields from `String` to `EndpointId`
- [ ] Verify: `cargo check -p willow-client`

#### 2.5 — Port client tests to MemNetwork

- [ ] Update `test_client()` helper:
  - Returns `ClientHandle<MemNetwork>` with `MemHub`
  - Creates server, subscribes to channel topics via MemHub
  - Returns `(handle, event_rx)` for asserting events
- [ ] Add `test_client_pair()` helper:
  - Two `ClientHandle<MemNetwork>` on same `MemHub`
  - Both joined to same server with "general" channel
- [ ] Port all 93 existing tests:
  - Tests that previously asserted on `NetworkCommand` variants now assert on `ClientEvent` arrival at the other client or on local state mutation
  - `send_message` → verify message arrives at other client via MemHub
  - `create_channel` → verify channel creation event propagates
  - `trust/untrust` → verify permission events broadcast
  - `edit/delete/react` → verify state mutations
  - `reply` → verify reply preview
  - Profile/display name → verify profile broadcasts
- [ ] Verify: `cargo test -p willow-client`

#### 2.6 — Wire Leptos web UI

- [ ] Update `crates/web/src/app.rs`:
  - Create `IrohNetwork` with `Config` (relay URL, identity)
  - `ClientHandle::connect(network, identity, config)` — typed as `ClientHandle<IrohNetwork>`
  - Event loop: `spawn_local` reads from `event_rx` (same pattern, new generic type)
  - Remove old deferred channel construction
- [ ] Update `crates/web/src/state.rs`:
  - `peer_id` signal: use `EndpointId` display format
  - `peers` signal: peer tuples use `EndpointId`
- [ ] Update `crates/web/src/event_processing.rs`:
  - `process_event_batch()`: handle `EndpointId` in peer fields
  - Connection status: derive from `ConnectionEvent` stream instead of counting peers
- [ ] Update component files (`components/*.rs`):
  - Peer display: use `fmt_short()` for compact peer ID display
  - Any `PeerId` string comparisons → `EndpointId` comparison
- [ ] Verify: `cargo check -p willow-web --target wasm32-unknown-unknown`

#### 2.7 — Browser tests

- [ ] Update `crates/web/tests/browser.rs`:
  - `DisplayMessage.author_peer_id` → `EndpointId` display string
  - `make_msg()` helper uses `EndpointId` for author
  - All 39 tests pass with updated types
- [ ] Verify: `just test-browser` (requires Firefox + geckodriver)

#### 2.8 — Phase 2 validation gate

- [ ] `cargo test -p willow-client` — all 93 tests pass with `MemNetwork`
- [ ] `cargo check -p willow-web --target wasm32-unknown-unknown` — WASM compiles
- [ ] `just test-browser` — 39 browser tests pass

Note: `just dev` end-to-end smoke test is deferred to Phase 3. The client
now uses `IrohNetwork` but the relay is still libp2p until Phase 3 — these
are incompatible transports. All Phase 2 validation is via `MemNetwork`
tests and WASM compile checks.

---

## Phase 3: Relay + Workers

Replace the custom relay with an iroh-relay wrapper + bootstrap gossip node. Make workers generic over `Network`. Port worker and scaling tests.

### File Map

```
crates/relay/
├── Cargo.toml             — Replace libp2p deps with iroh, iroh-relay, iroh-gossip,
│                            willow-network
└── src/
    ├── lib.rs             — DELETE old RelayBehaviour, Relay struct
    ├── main.rs            — Rewrite: iroh-relay server + bootstrap gossip node
    └── config.rs          — NEW: RelayConfig (relay_addr, bootstrap identity,
                             tls cert/key paths, system topics)

crates/worker/
├── Cargo.toml             — Replace old willow-network dep with new (trait-based)
└── src/
    ├── lib.rs             — WorkerRole trait unchanged, run() becomes generic
    ├── runtime.rs         — run<N: Network>(): create actors with TopicHandle/Events
    ├── config.rs          — WorkerConfig: relay_addr → relay_url: RelayUrl
    ├── identity.rs        — iroh SecretKey, print EndpointId hex
    ├── types.rs           — Unchanged (re-exports willow_common types)
    └── actors/
        ├── network.rs     — Stream from TopicEvents instead of polling libp2p swarm
        ├── state.rs       — Unchanged (no network dependency)
        ├── heartbeat.rs   — Send via TopicHandle instead of NetworkOutMsg
        └── sync.rs        — Send via TopicHandle instead of NetworkOutMsg

crates/replay/src/main.rs — Use IrohNetwork, --relay-url CLI flag
crates/storage/src/main.rs — Use IrohNetwork, --relay-url CLI flag
```

### Steps

#### 3.1 — Relay rewrite

- [ ] Update `crates/relay/Cargo.toml`: replace libp2p deps with `iroh`, `iroh-relay`, `iroh-gossip`, `willow-network`, `willow-identity`
- [ ] Delete `crates/relay/src/lib.rs` (old `Relay` + `RelayBehaviour`)
- [ ] Create `crates/relay/src/config.rs`:
  - `RelayConfig`: `relay_bind_addr`, `bootstrap_identity_path`, `tls_cert_path`, `tls_key_path` (all optional for dev)
  - CLI parsing via clap
- [ ] Rewrite `crates/relay/src/main.rs`:
  - Start `iroh_relay::Server` with configured bind address
  - Create `IrohNetwork` with bootstrap identity (load or generate)
  - Subscribe to system topics: `SERVER_OPS_TOPIC`, `WORKERS_TOPIC`, `PROFILES_TOPIC`
  - `tokio::signal::ctrl_c()` for graceful shutdown
  - Print bootstrap node `EndpointId` on startup (for client config)
- [ ] Verify: `cargo build -p willow-relay`

#### 3.2 — Worker runtime generic over Network

- [ ] Update `crates/worker/Cargo.toml`: replace old willow-network dep with new
- [ ] Update `crates/worker/src/runtime.rs`:
  - `pub async fn run<N: Network>(role, config, network: N)` signature
  - Subscribe to `WORKERS_TOPIC` and `SERVER_OPS_TOPIC` via `network.subscribe()`
  - Pass `TopicHandle` to heartbeat and sync actors
  - Pass `TopicEvents` to network actor
- [ ] Update `crates/worker/src/identity.rs`:
  - `load_or_generate()` → iroh `SecretKey` from/to file
  - `print_peer_id()` → print `EndpointId` hex
- [ ] Update `crates/worker/src/config.rs`:
  - `relay_addr: String` → `relay_url: Option<RelayUrl>`

#### 3.3 — Worker actor rewrites

- [ ] Rewrite `crates/worker/src/actors/network.rs`:
  - `network_actor<E: TopicEvents>(events, state_tx, shutdown_rx)`:
    - Stream from `TopicEvents` instead of polling `NetworkNode`
    - On `GossipEvent::Received` → parse worker/server messages, send to state actor
    - Keep existing `parse_worker_message()` and `parse_server_message()` (pure functions, unchanged)
- [ ] Update `crates/worker/src/actors/heartbeat.rs`:
  - Accept `TopicHandle` instead of `mpsc::Sender<NetworkOutMsg>`
  - `sender.broadcast(packed_announcement)` instead of channel send
- [ ] Update `crates/worker/src/actors/sync.rs`:
  - Accept `TopicHandle` instead of `mpsc::Sender<NetworkOutMsg>`
  - `sender.broadcast(packed_sync_request)` instead of channel send
- [ ] `crates/worker/src/actors/state.rs` — unchanged (no network dependency)
- [ ] Verify: `cargo check -p willow-worker`

#### 3.4 — Replay and storage binaries

- [ ] Update `crates/replay/src/main.rs`:
  - Create `IrohNetwork` with worker identity + relay URL
  - Call `willow_worker::run(role, config, network)`
  - CLI: `--relay` → `--relay-url`
- [ ] Update `crates/storage/src/main.rs`: same pattern
- [ ] Role files (`role.rs`, `store.rs`) — unchanged (pure state logic)
- [ ] Verify: `cargo build -p willow-replay && cargo build -p willow-storage`

#### 3.5 — Port worker tests to MemNetwork

- [ ] Update `crates/worker/tests/integration.rs`:
  - Create `MemHub` + `MemNetwork` instead of mock channels
  - State actor tests: unchanged (no network dependency)
  - Heartbeat tests: pass `MemTopicHandle`, assert broadcasts arrive on hub
  - Sync tests: pass `MemTopicHandle`, assert sync requests broadcast
  - Full orchestration test: wire all actors with `MemNetwork`
  - Graceful shutdown test: verify departure broadcast on hub
- [ ] Verify: `cargo test -p willow-worker`

#### 3.6 — Port scaling tests

- [ ] Create `crates/network/tests/scaling.rs`:
  - `scale_5/10/20_peers_connect()` — IrohNetwork nodes, star topology
  - `scale_5/10_peers_message_flood()` — broadcast and verify delivery
  - Adjust timeout thresholds for iroh QUIC
- [ ] Verify: `cargo test -p willow-network --test scaling`

#### 3.7 — Update `just dev` flow

- [ ] Update `justfile`:
  - `relay` recipe: build and run new `willow-relay` binary
  - `dev` recipe: start iroh relay wrapper → workers → trunk serve
  - Print bootstrap node `EndpointId` for client config
- [ ] Test: `just dev` starts full stack

#### 3.8 — Phase 3 validation gate

- [ ] `cargo build -p willow-relay` — relay builds
- [ ] `cargo test -p willow-worker` — worker tests pass with MemNetwork
- [ ] `cargo build -p willow-replay && cargo build -p willow-storage` — binaries build
- [ ] `just dev` — full stack starts, web UI connects, messages deliver
- [ ] Scaling tests pass

---

## Phase 4: Cleanup

Remove all libp2p vestiges. Delete replaced crates. Update docs and deployment.

### Steps

#### 4.1 — Remove libp2p dependencies

- [ ] Audit all `Cargo.toml` files for remaining libp2p deps
- [ ] Remove `libp2p` from workspace `[dependencies]` in root `Cargo.toml`
- [ ] `cargo check --workspace` — verify no libp2p imports remain

#### 4.2 — Delete replaced crates and files

- [ ] Delete `crates/files/` entirely (replaced by iroh-blobs)
- [ ] Remove `willow-files` from workspace members
- [ ] Delete old test files from `crates/app/tests/` (integration.rs, peer_scale.rs)
- [ ] Clean up any dead code referencing old network types

#### 4.3 — Remove WASM transport branching

- [ ] Search for `#[cfg(target_arch = "wasm32")]` in network-related code
- [ ] Remove platform-specific transport code (iroh handles internally)
- [ ] Keep legitimate WASM cfg gates (blob store selection, storage backend)
- [ ] `cargo check --target wasm32-unknown-unknown -p willow-network -p willow-client`

#### 4.4 — Update E2E state convergence tests

- [ ] Update `crates/app/tests/e2e_flow.rs`: authors → `EndpointId`
- [ ] Verify: `cargo test -p willow-app --test e2e_flow`

#### 4.5 — Update Playwright E2E tests

- [ ] Update `e2e/helpers.ts`: relay startup → new binary
- [ ] Run all Playwright test suites

#### 4.6 — Update Docker deployment

- [ ] Update Dockerfiles for relay, replay, storage
- [ ] Update `docker-compose.yml` for new CLI flags
- [ ] Test: `just docker-build && just docker-up`

#### 4.7 — Update CLAUDE.md

- [ ] Architecture notes: iroh replaces libp2p
- [ ] Dependency graph update
- [ ] Message flow update
- [ ] Network protocol table update
- [ ] Remove "Adding a new libp2p protocol" section
- [ ] Add "Adding a new iroh protocol" section
- [ ] Update `just dev` instructions

#### 4.8 — Phase 4 validation gate

- [ ] `just check` — fmt + clippy + test + WASM, zero warnings
- [ ] `just test-browser` — browser tests pass
- [ ] `just dev` → manual smoke test
- [ ] `grep -r "libp2p" crates/` — zero matches
- [ ] `cargo tree | grep libp2p` — not in dependency tree
