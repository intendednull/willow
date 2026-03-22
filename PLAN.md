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
| `willow-transport` | Binary serialization, protocol versioning, message framing | ✅ Done |
| `willow-identity` | Ed25519 keypairs, signed messages, user profiles | ✅ Done |
| `willow-messaging` | Chat messages, HLC ordering, message store | ✅ Done |
| `willow-channel` | Servers, channels, roles, permissions, invites | ✅ Done |
| `willow-network` | libp2p networking (GossipSub, Kademlia, mDNS, Relay) | ✅ Done |
| `willow-app` | Bevy-based native desktop UI | ✅ Scaffold done |

## Roadmap

### Phase 1 — Foundation (COMPLETE)
- [x] Transport layer with versioned envelopes
- [x] Cryptographic identity with Ed25519 signing
- [x] Message model with Hybrid Logical Clock ordering
- [x] Channel/server/role/permission model
- [x] libp2p networking with GossipSub, Kademlia, mDNS
- [x] Bevy app scaffold with sidebar + chat layout

### Phase 2 — Working Chat
- [ ] Wire up network ↔ messaging: publish/receive messages over GossipSub
- [ ] Text input in Bevy UI (keyboard capture, text editing)
- [ ] Message rendering (scrollable list, timestamps, author names)
- [ ] Channel switching in the sidebar
- [ ] Server creation and invite flow in the UI
- [ ] Persist messages to disk (SQLite or sled)
- [ ] Profile editing (display name, avatar)

### Phase 3 — File Sharing
- [ ] `willow-files` crate — content-addressed chunked file storage
- [ ] File transfer protocol over libp2p streams
- [ ] Drag-and-drop file upload in Bevy
- [ ] Inline image/video previews
- [ ] Peers seed files they've downloaded (BitTorrent-style)

### Phase 4 — Voice & Video
- [ ] `willow-media` crate — WebRTC-like media transport
- [ ] Voice channels with Opus audio
- [ ] Video with VP8/VP9
- [ ] Screen sharing
- [ ] Mesh topology for small groups (≤8 peers)
- [ ] Push-to-talk and voice activity detection

### Phase 5 — Polish
- [ ] Custom emoji (shared as files, referenced by shortcode)
- [ ] Emoji reactions on messages
- [ ] Message search
- [ ] Notification system
- [ ] Themes and UI customization
- [ ] End-to-end encryption with per-channel group keys
- [ ] Optional relay node for offline message delivery

## Key Technical Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | Memory safety, performance, async ecosystem |
| UI Framework | Bevy | GPU-accelerated, ECS state management, native + WASM |
| Networking | libp2p | Battle-tested P2P stack, NAT traversal, encryption |
| Async Runtime | tokio | Standard for Rust async, full-featured |
| Message Ordering | Hybrid Logical Clocks | Consistent ordering without central coordination |
| State Sync | CRDTs (future) | Convergent state across disconnected peers |
| Serialization | bincode | Fast, compact binary format |
| Crypto | Ed25519 | Fast signing, small keys, well-audited |

## Honest Tradeoffs

- **NAT traversal is hard** — most home networks need relay nodes. Budget for
  one $5/mo VPS running a Willow relay.
- **Offline delivery** — pure P2P means no one receives your message if they're
  offline. The relay node doubles as a mailbox.
- **Group calls** — mesh topology works for ~5-8 people. Larger groups would
  need an SFU, which reintroduces a server.
- **Bevy UI** — powerful but immature for traditional GUI. Text input, scroll
  views, and accessibility need custom work.
