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
│   Sidebar │ Chat │ Voice/Video │ Files │ Emoji   │
├─────────────────────────────────────────────────┤
│              Application Layer                   │
│   willow-channel │ willow-messaging              │
├─────────────────────────────────────────────────┤
│              Crypto Layer                        │
│   willow-crypto (ChaCha20-Poly1305, X25519)      │
│   Content encryption │ Key exchange │ Signing     │
├─────────────────────────────────────────────────┤
│              Media Layer (future)                │
│   WebRTC voice/video │ File chunking             │
├─────────────────────────────────────────────────┤
│              Network Layer                       │
│   willow-network (libp2p + tokio)                │
│   GossipSub │ Kademlia │ mDNS │ Relay │ Noise   │
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
| `willow-identity` | Ed25519 keypairs, signed messages, user profiles | Done |
| `willow-messaging` | Chat messages, HLC ordering, message store, SealedContent | Done |
| `willow-crypto` | E2E encryption (ChaCha20-Poly1305), X25519 key exchange | Done |
| `willow-channel` | Servers, channels, roles, permissions, invites with encrypted keys | Done |
| `willow-network` | libp2p networking (GossipSub, Kademlia, mDNS, Relay) | Done |
| `willow-app` | Bevy-based native desktop UI with encrypted chat | Done |

## Roadmap

### Phase 1 — Foundation (COMPLETE)
- [x] Transport layer with versioned envelopes
- [x] Cryptographic identity with Ed25519 signing
- [x] Message model with Hybrid Logical Clock ordering
- [x] Channel/server/role/permission model
- [x] libp2p networking with GossipSub, Kademlia, mDNS

### Phase 2 — Working Chat (COMPLETE)
- [x] Wire up network <> messaging: publish/receive messages over GossipSub
- [x] Text input in Bevy UI (keyboard capture)
- [x] Message rendering (scrollable list, author names)
- [x] Channel switching in the sidebar
- [x] Bevy 0.18 upgrade with modern component-based API
- [x] Headless UI testing with MinimalPlugins
- [x] Full integration tests with real libp2p network nodes

### Phase 3 — E2E Encryption (COMPLETE)
- [x] `willow-crypto` crate: ChaCha20-Poly1305 content encryption
- [x] Per-channel symmetric keys with `ChannelKey` type
- [x] X25519 key exchange (Ed25519 → X25519 conversion)
- [x] Encrypted key distribution via invite system
- [x] Message-level Ed25519 signatures (author verification)
- [x] `Content::Encrypted(SealedContent)` variant with `key_epoch`
- [x] Transparent encrypt-on-send, decrypt-on-receive pipeline
- [x] Backwards compatibility: unsigned/unencrypted messages still accepted
- [x] Integration tests: encrypted+signed round-trip over real network

### Phase 4 — Forward Secrecy & Key Rotation
- [ ] Evaluate p2panda-encryption or Decentralized MLS (DMLS)
- [ ] Key rotation on member removal (re-key channel)
- [ ] Per-message forward secrecy (Double Ratchet or DCGKA)
- [ ] DM encryption (1:1 via X25519 DH)

### Phase 5 — Persistence & Profiles
- [ ] Server creation and invite flow in the UI
- [ ] Persist messages to disk (SQLite or sled)
- [ ] Secure local key storage
- [ ] Profile editing (display name, avatar)
- [ ] Encrypted metadata (topic obfuscation)

### Phase 6 — File Sharing (IN PROGRESS)
- [x] `willow-files` crate — content-addressed chunked file storage
  - SHA-256 content hashing, configurable chunk size (default 256 KiB)
  - split_file / assemble_file with integrity verification
  - ChunkStore for tracking partial downloads and reassembly
  - FileManifest for metadata broadcast over gossipsub
- [ ] File transfer protocol over libp2p request-response
- [ ] File sharing integration in the chat UI
- [ ] Inline image/video previews
- [ ] Peers seed files they've downloaded (BitTorrent-style)

### Phase 7 — Voice & Video
- [ ] `willow-media` crate — WebRTC-like media transport
- [ ] Voice channels with Opus audio
- [ ] Video with VP8/VP9
- [ ] Screen sharing
- [ ] Mesh topology for small groups (≤8 peers)
- [ ] Push-to-talk and voice activity detection

### Phase 8 — Polish
- [ ] Custom emoji (shared as files, referenced by shortcode)
- [ ] Emoji reactions on messages
- [ ] Message search
- [ ] Notification system
- [ ] Themes and UI customization
- [ ] Optional relay node for offline message delivery

## Key Technical Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | Memory safety, performance, async ecosystem |
| UI Framework | Bevy 0.18 | GPU-accelerated, ECS state management, native + WASM |
| Networking | libp2p | Battle-tested P2P stack, NAT traversal, encryption |
| Async Runtime | tokio | Standard for Rust async, full-featured |
| Message Ordering | Hybrid Logical Clocks | Consistent ordering without central coordination |
| Encryption | ChaCha20-Poly1305 + X25519 | AEAD symmetric encryption + DH key exchange |
| Key Distribution | X25519 via invites | No central key server needed |
| Signing | Ed25519 | Fast signing, small keys, message authenticity |
| Serialization | bincode | Fast, compact binary format |

## Honest Tradeoffs

- **NAT traversal is hard** — most home networks need relay nodes. Budget for
  one $5/mo VPS running a Willow relay.
- **Offline delivery** — pure P2P means no one receives your message if they're
  offline. The relay node doubles as a mailbox.
- **Group calls** — mesh topology works for ~5-8 people. Larger groups would
  need an SFU, which reintroduces a server.
- **Bevy UI** — powerful but immature for traditional GUI. Text input, scroll
  views, and accessibility need custom work.
- **No forward secrecy yet** — Phase 3 uses static per-channel keys. A
  compromised key exposes all past messages on that channel. Phase 4 adds
  key ratcheting for per-message forward secrecy.
