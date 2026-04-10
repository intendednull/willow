# Willow — P2P Chat Platform

A decentralized Discord replacement for friend groups. No central servers, no
accounts, no middlemen. Built with Rust, libp2p, and Leptos.

## Vision

A web app where you and your friends can:
- Text chat with channels, threads, reactions, pins, and emoji
- Voice call, video call, and screen share
- Share files and images peer-to-peer
- Create servers and channels with role-based permissions
- Discover each other on the local network or via relay nodes
- Own your data — everything is end-to-end encrypted

## Architecture

```
┌─────────────────────────────────────────────────┐
│              Leptos Web App (willow-web)         │
│   Sidebar │ Chat │ Settings │ Files │ Servers    │
│   Welcome │ Pinned │ Members │ Roles │ Typing    │
├─────────────────────────────────────────────────┤
│              Client Library (willow-client)      │
│   Event-sourced state │ DisplayMessage view-model│
│   Server management │ Profiles │ Sync │ Typing   │
├─────────────────────────────────────────────────┤
│              State Machine (willow-state)        │
│   Pure apply() │ 21 EventKind variants │ Merge   │
│   StateHash │ Permissions │ Pins │ Reactions      │
├─────────────────────────────────────────────────┤
│              Application Layer                   │
│   willow-channel │ willow-messaging │ willow-files│
├─────────────────────────────────────────────────┤
│              Crypto Layer                        │
│   willow-crypto (ChaCha20-Poly1305, X25519)      │
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
| `willow-state` | Pure event-sourced state machine, 21 EventKind variants | Done |
| `willow-client` | UI-agnostic client library, DisplayMessage, server management | Done |
| `willow-web` | Leptos web UI (primary frontend) | Done |
| `willow-relay` | Relay server bridging TCP and WebSocket peers | Done |

## Roadmap

### Phase 1 — Foundation (COMPLETE)
- [x] Transport layer with versioned envelopes
- [x] Cryptographic identity with Ed25519 signing
- [x] Message model with Hybrid Logical Clock ordering
- [x] Channel/server/role/permission model
- [x] libp2p networking with GossipSub, Kademlia, mDNS

### Phase 2 — Working Chat (COMPLETE)
- [x] Wire up network ↔ messaging over GossipSub
- [x] Text input, message rendering with timestamps and author names
- [x] Channel switching, creation, deletion
- [x] Full integration tests with real libp2p network nodes

### Phase 3 — E2E Encryption (COMPLETE)
- [x] ChaCha20-Poly1305 content encryption (seal/open)
- [x] Per-channel symmetric keys with ChannelKey
- [x] X25519 key exchange (Ed25519 → X25519 conversion)
- [x] Encrypted key distribution via invite system
- [x] Transparent encrypt-on-send, decrypt-on-receive

### Phase 4 — Forward Secrecy & Key Rotation (COMPLETE)
- [x] KeyRatchet: per-message key derivation via HKDF
- [x] Key rotation on member removal (re-key all channels)

### Phase 5 — Persistence & Profiles (COMPLETE)
- [x] Identity persistence (Ed25519 keypair saved to disk / localStorage)
- [x] Full ServerState persistence (save/load entire state, no replay needed)
- [x] Per-server profiles with display names
- [x] Profile broadcasting over gossipsub

### Phase 6 — File Sharing (COMPLETE)
- [x] Content-addressed chunking (SHA-256, 256 KiB)
- [x] Inline file sharing via gossipsub (<256KB)
- [x] Image/GIF embedding (URL detection + uploaded file embeds)
- [x] File cards with download buttons

### Phase 7 — Server State Sync (COMPLETE)
- [x] Event-based wire format (WireMessage with Events)
- [x] Catch-up on connect: exchange missing events via state hash
- [x] Relay stores events in SQLite, serves to new peers
- [x] Multi-peer state verification (StateVerification + hash tracking)

### Phase 8 — Event-Sourced State Machine (COMPLETE)
- [x] willow-state crate: pure deterministic state machine (zero I/O)
- [x] 21 EventKind variants (channels, roles, permissions, messages,
      edits, deletes, reactions, pins, profiles, server metadata)
- [x] StateHash (SHA-256) for divergence detection
- [x] apply() with permission enforcement and dedup
- [x] Fine-grained Permission model
- [x] merge() for divergent history resolution
- [x] DisplayMessage view-model computed from event_state (never stored)
- [x] Single source of truth: UI reads directly from ServerState
- [x] Legacy Op system fully removed — Events only

### Phase 9 — Voice & Video (IN PROGRESS)
- [ ] WebRTC-like media transport
- [ ] Voice channels with Opus audio
- [ ] Video with VP8/VP9
- [ ] Screen sharing

### Phase 10 — Polish & UX (COMPLETE)
- [x] Discord-style dark/light theme with CSS variables
- [x] IBM Plex typography, refined design
- [x] Welcome/onboarding screen (no default server)
- [x] Create server + join server flows
- [x] Per-server profile editing
- [x] Server settings page (invites, roles, separate from profile)
- [x] Message editing and deletion
- [x] Reply threads with parent preview and jump-to-parent
- [x] Mention highlighting (replies to you)
- [x] Emoji reactions (toggle on click, Discord-style)
- [x] Pinned messages with clickable URLs and jump-to-message
- [x] Typing indicators with debounce and auto-expiry
- [x] Single action dropdown menu (Reply, React, Edit, Delete, Pin, Download)
- [x] Time-gap grouping (5min threshold re-shows author/timestamp)
- [x] URL detection and clickable links in messages
- [x] Member list with online/offline status and role badges
- [x] Mobile member list slide-out panel
- [x] Mobile responsive layout with touch targets
- [x] Auto-focus input on reply/edit
- [x] Notification sounds (Web Audio API)
- [x] PWA manifest + service worker
- [x] Message deduplication (msg_id across sessions)
- [x] Clipboard fallback for non-HTTPS

### Infrastructure (COMPLETE)
- [x] WASM dual-target support (all crates compile for wasm32)
- [x] Relay server with display name (TCP + WebSocket)
- [x] Cross-platform storage (filesystem / localStorage)
- [x] Cross-platform clipboard (web_sys + textarea fallback)
- [x] Deploy skill for automated builds and deployment
- [x] justfile with all build/test/serve commands

## Data Flow

```
Action → Create Event → apply_event() → ServerState mutated → save_server_state()
                      → broadcast_event() → peers receive → apply to their state

UI calls client.messages(channel) → computed Vec<DisplayMessage> from event_state
  → display names resolved fresh (never stale)
  → reactions, edits, deletes, pins all from authoritative state
  → no stored intermediate copies, no manual merging
```

## Test Coverage

353 tests across 7 tiers:

| Tier | Tests | Scope |
|------|-------|-------|
| Pure state machine | 82 | Determinism, permissions, merges, pins, reactions, replay |
| Client library | 114 | DisplayMessage, server management, typing, dedup, profiles |
| Network integration | 14 | Real libp2p, 3-node topology, file chunks |
| Scaling | 7 | 5/10/20 peers, message flood, 532k events/sec |
| E2E flow | 9 | State machine + network: invite, chat, sync, merge |
| Relay history | 3 | Store events, serve to new peers, multi-peer |
| Browser (Leptos) | 39 | Real DOM, signals, events, all components |
| Web component units | 11 | URL extraction, image detection, MIME types |
| Other crates | ~74 | Transport, identity, crypto, channel, files |

Run: `just test` (cargo), `just test-browser` (headless Firefox),
`just test-all` (both), `just test-scale` (with output),
`just test-e2e` (end-to-end flow)

## Key Technical Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Language | Rust | Memory safety, performance, async ecosystem |
| Web UI | Leptos 0.7 CSR | Reactive, WASM-native, component-based |
| Networking | libp2p 0.53 | Battle-tested P2P, NAT traversal |
| State | Event-sourced | Deterministic, mergeable, auditable |
| Encryption | ChaCha20-Poly1305 + X25519 | AEAD + DH key exchange |
| Signing | Ed25519 | Fast, small keys, message authenticity |
| Serialization | bincode | Fast, compact binary format |
| Message Ordering | Hybrid Logical Clocks | No central coordination needed |
| Persistence | Full ServerState save | Instant load, no replay needed |

## Deployed

- **Relay**: `172.234.217.219` ports 9090 (TCP) + 9091 (WebSocket)
- **Web app**: `172.234.217.219` via nginx
- **PeerId**: `12D3KooWMBmUF1rHYG5CneKi8JZfKdMAciJd4oCgknTJkbwCUurd`
