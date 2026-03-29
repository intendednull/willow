# Willow

A peer-to-peer Discord replacement built with Rust. No central servers, no
accounts, no middlemen. End-to-end encrypted by default.

## Features

- **Text chat** with channels, threads, reactions, pins, and emoji
- **End-to-end encryption** — ChaCha20-Poly1305 with X25519 key exchange
- **Peer-to-peer** — libp2p networking with GossipSub, Kademlia, and mDNS
- **File sharing** — content-addressed chunking, transferred peer-to-peer
- **Servers & permissions** — roles, fine-grained permissions, invites
- **Event-sourced state** — deterministic, mergeable, offline-friendly
- **Runs everywhere** — native desktop (Bevy) and web browser (Leptos/WASM)

## Architecture

```
Leptos Web UI  or  Bevy Desktop UI
        │                │
        └──── Client Library (willow-client) ────┐
                     │                            │
              State Machine              Network Layer
           (willow-state, pure)       (willow-network, libp2p)
                     │                            │
              ┌──────┴──────┐              ┌──────┴──────┐
              │  Channels   │              │   Relay     │
              │  Messaging  │              │   Workers   │
              │  Crypto     │              │  (replay,   │
              │  Files      │              │   storage)  │
              └─────────────┘              └─────────────┘
```

**Crates:**

| Crate | Purpose |
|-------|---------|
| `willow-state` | Pure event-sourced state machine (zero I/O) |
| `willow-client` | UI-agnostic client library |
| `willow-transport` | Binary serialization & protocol framing |
| `willow-identity` | Ed25519 identity, message signing, profiles |
| `willow-messaging` | Chat messages, HLC ordering |
| `willow-crypto` | E2E encryption (ChaCha20-Poly1305, X25519) |
| `willow-channel` | Servers, channels, roles, permissions |
| `willow-files` | Content-addressed file chunking |
| `willow-network` | libp2p networking layer (native + WASM) |
| `willow-relay` | Relay server bridging TCP and WebSocket peers |
| `willow-web` | Leptos web UI |
| `willow-app` | Bevy desktop UI |

## Getting Started

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- [just](https://github.com/casey/just) (command runner)
- [trunk](https://trunkrs.dev/) (for the web UI)
- `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`

### Local Development

Start the full stack with a single command:

```bash
just dev
```

This launches all services:

| Service | Address | Description |
|---------|---------|-------------|
| Relay | `localhost:9090` (TCP), `localhost:9091` (WS) | Bridges peers |
| Replay node | connects via relay | In-memory state sync (max 1000 events/server) |
| Storage node | connects via relay | Archival SQLite storage |
| Web UI | `http://localhost:8080` | Leptos app via trunk serve |

All service logs are color-coded and interleaved in the terminal. Press
`Ctrl+C` to stop everything.

Identity keys and data persist in `.dev/` so peer IDs stay stable across
restarts. After the first run, use `just dev-quick` to skip the build step.

```bash
just dev-quick   # skip build, start immediately
just dev-clean   # reset all local dev data
```

### Running Individual Services

```bash
just relay          # relay server only
just serve-web      # web UI only (trunk serve)
just run            # native desktop app
```

### Docker

```bash
just docker-build   # build all images
just docker-up      # start full stack
just docker-down    # stop full stack
just docker-logs    # tail all logs
```

## Testing

Willow has 420+ tests across multiple tiers:

```bash
just check          # fmt + clippy + test + WASM check (run before committing)
just test           # all cargo tests
just test-state     # pure state machine (64 tests, instant)
just test-client    # client library (93 tests)
just test-app       # Bevy headless + network integration (113 tests)
just test-relay     # relay history sync (3 tests)
just test-scale     # scaling/performance tests
just test-browser   # in-browser Leptos tests (requires Firefox + geckodriver)
```

## License

See [LICENSE](LICENSE) for details.
