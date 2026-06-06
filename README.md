# Willow

A peer-to-peer Discord replacement built with Rust. No central servers, no
accounts, no middlemen. End-to-end encrypted by default.

## Features

- **Text chat** with channels, threads, reactions, pins, and emoji
- **End-to-end encryption** — ChaCha20-Poly1305 with X25519 key exchange
- **Peer-to-peer** — iroh networking with QUIC transport and gossip protocol
- **File sharing** — content-addressed chunking, transferred peer-to-peer
- **Servers & permissions** — roles, fine-grained permissions, invites
- **Event-sourced state** — deterministic, mergeable, offline-friendly
- **Runs in the browser** — Leptos + WASM web UI

## Architecture

```
            Leptos Web UI (willow-web)
                       │
              Client Library (willow-client)
                       │
              ┌────────┴─────────┐
              │                  │
       State Machine       Network Layer
    (willow-state, pure)  (willow-network, iroh)
              │                  │
       ┌──────┴──────┐     ┌─────┴─────┐
       │  Channels   │     │   Relay   │
       │  Messaging  │     │  Workers  │
       │  Crypto     │     │ (replay,  │
       │  Identity   │     │  storage) │
       └─────────────┘     └───────────┘
                                 │
                           ┌─────┴─────┐
                           │   Actor   │
                           │ Framework │
                           └───────────┘
```

**Crates:**

| Crate | Purpose |
|-------|---------|
| `willow-state` | Pure event-sourced state machine (zero I/O) |
| `willow-client` | UI-agnostic client library wrapping state + networking |
| `willow-transport` | Binary serialization & protocol framing |
| `willow-identity` | Ed25519 identity, message signing, profiles |
| `willow-messaging` | Chat messages, HLC ordering, message store |
| `willow-crypto` | E2E encryption (ChaCha20-Poly1305, X25519) |
| `willow-network` | iroh-based P2P networking (native + WASM) |
| `willow-actor` | Lightweight actor framework (dual-target native + WASM) |
| `willow-common` | Shared wire protocol types (WireMessage, worker types) |
| `willow-worker` | Shared worker library (roles, actor runtime, peer lifecycle) |
| `willow-relay` | Relay server bridging TCP and WebSocket peers |
| `willow-replay` | Bounded-memory state sync worker (in-memory event buffer) |
| `willow-storage` | Archival disk-backed history worker (SQLite) |
| `willow-agent` | MCP server exposing ClientHandle to AI agents |
| `willow-web` | Leptos web UI |

### State Management

All shared state is **event-sourced** — derived deterministically from a
per-author Merkle-DAG of signed events. There is no mutable database; the
event log *is* the data.

```
  Author A          Author B          Author C
     │                  │                  │
  [e1]─→[e2]─→[e3]  [e1]─→[e2]        [e1]
     │                  │                  │
     └──────────────────┴──────────────────┘
                        │
              topological sort + replay
                        │
                   ServerState
            (channels, roles, members,
             messages, profiles, keys)
```

**How it works:**

- **Events** are content-addressed (SHA-256), signed by their author, and
  linked into a DAG via sequence numbers and dependency pointers.
- **`ServerState`** is the materialized view — rebuilt deterministically by
  topologically sorting all events and replaying them through `materialize()`.
  Peers converge to identical state when they have the same DAG.
- **`EventKind` variants** cover server structure (create/delete/rename
  channels and roles), chat (messages, edits, deletes, reactions, pins),
  permissions (grant/revoke), identity (profiles), encryption (key rotation),
  and governance (proposals and votes).
- **Sync protocol** uses compact `HeadsSummary` (author → seq + hash) so peers
  can efficiently request only missing events. Full `Snapshot` bootstraps
  far-behind peers.

**Trust & permissions:**

- Admin status is granted only through governance votes (`Propose` +
  `Vote`), never by direct event — protecting against single-actor escalation.
- Non-admin permissions (ManageChannels, ManageRoles, SendMessages,
  CreateInvite, SyncProvider) can be granted directly by admins.
- All permission checks are enforced deterministically during event replay.

**Client layer** (`willow-client`): `ClientHandle` wraps the pure state
machine with actor-based state management, reactive UI views, a mutations
API, and persistence — bridging the deterministic core with async networking
and pub/sub event distribution.

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
| Relay | `localhost:3340` | iroh relay HTTP + bootstrap |
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
```

### Deployment

Production deployment is owned by the **`infra`** repo (a shared NixOS deploy flake).
Willow is a `runtime = "multi"` app there — web UI + relay + replay + storage, fronted by
the edge Caddy at `https://willow.intendednull.com` (web) and `https://relay.willow.intendednull.com`
(`wss://` relay). Willow's only deploy artifact is the **flake** (`flake.nix` + `nix/module.nix`),
which infra consumes as an input — there is no willow-side deploy script.

Deploy/update from an `infra` checkout: `nix flake update willow && just deploy web`. See
`infra/ONBOARDING.md` and `docs/plans/2026-06-05-infra-deployment-migration.md`.

## Testing

Willow has a multi-tier test suite:

```bash
just check          # fmt + clippy + test + WASM check (run before committing)
just test           # all cargo tests
just test-state     # pure state machine (instant)
just test-client    # client library
just test-relay     # relay history sync
just test-browser   # in-browser Leptos tests (requires Firefox + geckodriver)
```

## License

See [LICENSE](LICENSE) for details.
