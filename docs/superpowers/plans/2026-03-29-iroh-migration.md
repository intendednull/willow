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

*To be detailed after Phase 1 is implemented.*

**Scope**: Make `willow-client` generic over `Network`. Drop `NetworkCommand`/`NetworkEvent` enums. Port 93 client tests to `MemNetwork`. Wire Leptos web UI to new async-native client.

---

## Phase 3: Relay + Workers

*To be detailed after Phase 2 is implemented.*

**Scope**: Replace `willow-relay` with iroh-relay wrapper + bootstrap node. Make workers generic over `Network`. Port worker tests to `MemNetwork`. Port scaling tests to `IrohNetwork`.

---

## Phase 4: Cleanup

*To be detailed after Phase 3 is implemented.*

**Scope**: Remove all libp2p deps. Delete `willow-files`. Remove WASM transport branching. Update CLAUDE.md. Update Docker configs. Playwright E2E tests.
