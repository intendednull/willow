# Willow — P2P Chat Platform

A decentralized Discord replacement for friend groups. No central servers, no
accounts, no middlemen. Built with Rust, libp2p, and Bevy.

## Vision

A native desktop app where you and your friends can:
- Text chat with channels, threads, reactions, and emoji
- Voice call, video call, and screen share
- Share files peer-to-peer
- Create servers and channels with role-based permissions
- Discover each other on the local network or via bootstrap nodes
- Own your data — everything is end-to-end encrypted

## Architecture

```
┌─────────────────────────────────────────────────┐
│              Bevy App (willow-app)               │
│   Sidebar │ Chat │ Settings │ Files │ Emoji      │
│   ui/ (10 modules), invite, emoji, clipboard     │
├─────────────────────────────────────────────────┤
│              Application Layer                   │
│   willow-channel │ willow-messaging │ willow-files│
├─────────────────────────────────────────────────┤
│              Crypto Layer                        │
│   willow-crypto (ChaCha20-Poly1305, X25519)      │
│   Content encryption │ Key ratchet │ Key exchange │
├─────────────────────────────────────────────────┤
│              Network Layer                       │
│   willow-network (libp2p + tokio / wasm-bindgen) │
│   GossipSub │ Kademlia │ mDNS │ Relay │ Chunks   │
├─────────────────────────────────────────────────┤
│              Foundation                          │
│   willow-identity │ willow-transport             │
│   Ed25519 signing │ Protocol framing             │
└─────────────────────────────────────────────────┘
```

## Crates

| Crate | Purpose | Status |
|-------|---------|--------|
| `willow-transport` | Binary serialization, protocol versioning, message framing | Done |
| `willow-identity` | Ed25519 keypairs, signed messages, user profiles, PeerId extraction | Done |
| `willow-messaging` | Chat messages, HLC ordering, message store, SealedContent | Done |
| `willow-crypto` | E2E encryption, key ratchet, X25519 key exchange | Done |
| `willow-channel` | Servers, channels, roles, permissions, invites, key rotation | Done |
| `willow-files` | Content-addressed file chunking, SHA-256 hashing, ChunkStore | Done |
| `willow-network` | libp2p networking, chunk transfer protocol (native + WASM) | Done |
| `willow-app` | Bevy 0.18 desktop/web UI with full feature set | Done |
| `willow-relay` | Relay server bridging native TCP and browser WebSocket peers | Done |

## Roadmap

### Phase 1 — Foundation (COMPLETE)
- [x] Transport layer with versioned envelopes
- [x] Cryptographic identity with Ed25519 signing
- [x] Message model with Hybrid Logical Clock ordering
- [x] Channel/server/role/permission model
- [x] libp2p networking with GossipSub, Kademlia, mDNS

### Phase 2 — Working Chat (COMPLETE)
- [x] Wire up network ↔ messaging over GossipSub
- [x] Text input in Bevy UI (keyboard capture)
- [x] Message rendering with timestamps and author names
- [x] Channel switching in the sidebar
- [x] Bevy 0.18 upgrade with modern component-based API
- [x] Headless UI testing with MinimalPlugins
- [x] Full integration tests with real libp2p network nodes

### Phase 3 — E2E Encryption (COMPLETE)
- [x] ChaCha20-Poly1305 content encryption (seal/open)
- [x] Per-channel symmetric keys with ChannelKey
- [x] X25519 key exchange (Ed25519 → X25519 conversion)
- [x] Encrypted key distribution via invite system
- [x] Message-level Ed25519 signatures
- [x] Content::Encrypted(SealedContent) with key_epoch
- [x] Transparent encrypt-on-send, decrypt-on-receive

### Phase 4 — Forward Secrecy & Key Rotation (COMPLETE)
- [x] KeyRatchet: per-message key derivation via HKDF
- [x] Key rotation on member removal (re-key all channels)
- [x] SealedContent.ratchet_counter for receiver-side derivation
- [x] Backwards compatibility (counter=0 uses raw key)

### Phase 5 — Persistence & Profiles (COMPLETE)
- [x] Identity persistence (Ed25519 keypair saved to disk / localStorage)
- [x] Server state persistence (bincode to file / localStorage)
- [x] Channel key persistence
- [x] Message persistence (SQLite on native, localStorage on WASM)
- [x] Profile editing (display name in settings)
- [x] Profile broadcasting over gossipsub

### Phase 6 — File Sharing (COMPLETE)
- [x] willow-files crate: content-addressed chunking (SHA-256, 256 KiB)
- [x] ChunkStore for tracking partial downloads and reassembly
- [x] libp2p request-response protocol for chunk transfer (CBOR)
- [x] FileManager: share files, serve chunks, track downloads
- [x] Share button with native file picker (rfd)
- [x] File announcements displayed in chat
- [x] Received chunks seeded to other peers

### Phase 7 — Server State Sync (COMPLETE)
- [x] StampedOp wrapper: UUID dedup, HLC ordering, author verification
- [x] OpLog resource with persistence (save/load to disk)
- [x] Trust model: owner + TrustPeer/UntrustPeer ops
- [x] Trust management UI (Trust/Untrust buttons in member list)
- [x] Idempotent ops: SetPermission, deterministic channel/role IDs
- [x] SyncMessage protocol: Op, SyncRequest, SyncBatch
- [x] Catch-up on connect: exchange missing ops via HLC comparison
- [x] Key rotation distribution on kick (encrypted per-member)
- [x] Author verification in network bridge (stamped_op.author == signer)
- [x] Integration tests: server sync, 3-node propagation, file chunks

### Phase 8 — Event-Sourced State Machine (COMPLETE)
- [x] willow-state crate: pure deterministic state machine (zero I/O)
- [x] Event + EventKind with 17 mutation variants
- [x] StateHash (SHA-256) for divergence detection
- [x] apply() with permission enforcement and dedup
- [x] Fine-grained Permission model (replaces binary trust)
- [x] EventStore trait + InMemoryStore
- [x] merge() for divergent history resolution
- [x] 50 tests covering determinism, permissions, merge convergence
- [x] Invite payload carries owner + sync provider hints (unverified)
- [x] willow-client integrates willow-state (event_state + event_store)
- [x] Full migration: Client actions create Events, apply to event_state
- [x] Bridge layer (Event ↔ Op) for wire format backward compat
- [x] Push-based notification (mpsc channel via ClientNotification)
- [x] SQLite EventStore (native) + localStorage EventStore (WASM)
- [x] PersistentEventStore enum with platform dispatch
- [x] Event replay on startup to rebuild state from stored events
- [x] Invite carries owner + sync provider hints (unverified suggestions)
- [x] Full wire format migration: WireMessage(Event) replaces SyncMessage(Op)
- [x] Dual-format network layer (new Event + legacy Op) for interop
- [x] Bevy app uses legacy format via bridge (deprecated but functional)
- [x] Relay stores events in SQLite as they pass through gossipsub
- [x] Multi-peer state verification (StateVerification event + hash tracking)

### Phase 9 — Voice & Video (FUTURE)
- [ ] WebRTC-like media transport
- [ ] Voice channels with Opus audio
- [ ] Video with VP8/VP9
- [ ] Screen sharing

### Phase 10 — Polish (COMPLETE)
- [x] Discord-style dark theme (theme.rs color palette)
- [x] Message timestamps (HH:MM)
- [x] Unread channel indicators with count badges
- [x] Message search (Ctrl+F)
- [x] Emoji reactions on messages (badge rendering)
- [x] Custom emoji shortcodes (100+ built-in + server-defined)
- [x] Channel creation from UI ("+" button)
- [x] Channel deletion from UI ("×" button)
- [x] Secure per-recipient invite system (X25519 encrypted)
- [x] Copy PeerId / Copy invite code (clipboard)
- [x] Settings UI (display name, relay address, invites)
- [x] Focus indicators on settings fields
- [x] Escape key exits settings
- [x] Message memory pruning (1000 cap)
- [x] Peer count grammar ("1 peer" vs "3 peers")
- [x] UI module split (10 focused files)
- [x] OS notifications (notify-rust native, Notification API WASM)
- [x] Member list with online indicators and kick button
- [x] Role management UI (create, delete, assign, permission toggles)
- [x] Message editing and deletion
- [x] Reply threads with parent preview
- [x] Trust badges in member list ([owner], [trusted])
- [x] Server name/description editing (owner-only)
- [x] Notification sounds (Web Audio API, plays when tab hidden)
- [x] Dark/light theme toggle (CSS variables, localStorage persistence)
- [x] File sharing UI (inline file attachment with download cards)
- [x] Role management in Leptos web app (CRUD + permission toggles)
- [x] PWA manifest + service worker for installable web app
- [x] E2E integration tests (9 tests covering full invite→chat→sync flow)

### Infrastructure (COMPLETE)
- [x] WASM dual-target support (all crates compile for wasm32)
- [x] Relay server (TCP + WebSocket, auto-subscribes to topics)
- [x] Deferred network startup with ConnectCommand
- [x] Cross-platform storage (filesystem / localStorage)
- [x] Cross-platform clipboard (arboard / navigator.clipboard)
- [x] justfile with all build/test/serve commands
- [x] Web entry point (index.html)

## Key Technical Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | Memory safety, performance, async ecosystem |
| UI Framework | Bevy 0.18 | GPU-accelerated, ECS, native + WASM |
| Networking | libp2p 0.53 | Battle-tested P2P, NAT traversal |
| Encryption | ChaCha20-Poly1305 + X25519 | AEAD + DH key exchange |
| Forward Secrecy | HKDF key ratchet | Per-message keys, epoch-based rotation |
| Key Distribution | X25519 per-recipient invites | No central key server |
| Signing | Ed25519 | Fast, small keys, message authenticity |
| Serialization | bincode | Fast, compact binary format |
| Message Ordering | Hybrid Logical Clocks | No central coordination needed |
| File Sharing | SHA-256 content-addressed chunks | Integrity verification, dedup |
| Persistence | SQLite (native) / localStorage (WASM) | Cross-platform |

## Test Coverage

451+ tests across 8 tiers:

| Tier | Tests | Scope |
|------|-------|-------|
| Pure state machine | 77 | Determinism, permissions, merges, stress, replay, verification |
| Client library | 101 | API methods, event store, bridge, state accessors, file sharing |
| Bevy headless UI | 99 | Keyboard, chat, settings, invites, permissions |
| Network integration | 14 | Real libp2p, 3-node topology, file chunks |
| Scaling | 7 | 5/10/20 peers, message flood, 532k events/sec |
| E2E flow | 9 | State machine + network: invite, chat, sync, merge, permissions |
| Relay history | 3 | Store events, serve to new peers, multi-peer |
| Browser (Leptos) | 39+ | Real DOM, signals, events, all components |
| Other crates | ~100 | Transport, identity, crypto, channel, files |

Run: `just test` (cargo), `just test-browser` (headless Firefox),
`just test-all` (both), `just test-scale` (with output),
`just test-e2e` (end-to-end flow)

## Honest Tradeoffs

- **NAT traversal is hard** — most home networks need relay nodes
- **Offline delivery** — pure P2P means no one receives your message if
  they're offline; the relay node can help bridge
- **Bevy UI** — powerful but immature for traditional GUI; text input,
  scroll views, and accessibility need custom work
- **No forward secrecy per-message in practice** — the key ratchet
  infrastructure is built but not yet wired into the send/receive pipeline
  (messages still use the raw channel key with counter=0)
