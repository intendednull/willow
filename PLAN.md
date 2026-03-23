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
- [x] Headless UI testing with MinimalPlugins (236 tests)
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

### Phase 7 — Voice & Video (FUTURE)
- [ ] WebRTC-like media transport
- [ ] Voice channels with Opus audio
- [ ] Video with VP8/VP9
- [ ] Screen sharing

### Phase 8 — Polish (MOSTLY COMPLETE)
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
- [ ] OS notifications
- [ ] Member list with kick button
- [ ] Role management UI

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

236+ tests across all crates covering:
- Serialization round-trips, encryption/decryption, key exchange
- Key ratcheting, forward secrecy, key rotation
- Network integration (real libp2p nodes)
- Headless Bevy UI (input, sending, receiving, settings)
- File chunking, reassembly, chunk store
- Secure invite generate/accept/tamper-detection
- Full end-to-end invite flow (owner → recipient → decrypt → chat)
- Emoji shortcode expansion
- Profile persistence, message persistence

## Honest Tradeoffs

- **NAT traversal is hard** — most home networks need relay nodes
- **Offline delivery** — pure P2P means no one receives your message if
  they're offline; the relay node can help bridge
- **Bevy UI** — powerful but immature for traditional GUI; text input,
  scroll views, and accessibility need custom work
- **No forward secrecy per-message in practice** — the key ratchet
  infrastructure is built but not yet wired into the send/receive pipeline
  (messages still use the raw channel key with counter=0)
