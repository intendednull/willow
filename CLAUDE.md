# CLAUDE.md — Willow Development Guide

## Project Overview

Willow is a P2P Discord replacement built in Rust. It uses libp2p for
networking, Bevy for the desktop UI, and Ed25519 cryptography for identity.

## Repository Structure

```
crates/
├── transport/   — Binary serialization & protocol framing (willow-transport)
├── identity/    — Ed25519 identity, message signing, profiles (willow-identity)
├── messaging/   — Chat messages, HLC ordering, message store (willow-messaging)
├── crypto/      — E2E encryption: ChaCha20-Poly1305, X25519 key exchange (willow-crypto)
├── channel/     — Servers, channels, roles, permissions (willow-channel)
├── network/     — libp2p P2P networking layer (willow-network)
└── app/         — Bevy desktop UI application (willow-app)
```

## Build & Test

```bash
# Check everything compiles
cargo check

# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p willow-messaging

# Build the desktop app (requires desktop environment with GPU)
cargo build -p willow-app

# Formatting and linting (must pass with zero warnings)
cargo fmt --check
cargo clippy -- -D warnings
```

**All code must pass `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` with zero warnings before being committed.**

### Headless UI Testing

The Bevy app is tested programmatically using headless `MinimalPlugins` — no
window or GPU required. Tests live in `crates/app/src/tests.rs` and cover
keyboard input, message sending, network event handling, and serialization.

To add a new UI test:

1. Use `test_app()` to get a headless `App` and a network command receiver.
2. Inject input via `send_key()` or write messages directly with
   `app.world_mut().write_message(...)`.
3. Call `app.update()` to run one frame of systems.
4. Assert on `ChatState`, `InputState`, or the network command receiver.

Note: `handle_keyboard_input` and `send_message` run in the same `Update`
schedule but as separate systems. Setting `send_requested = true` takes effect
on the *next* `app.update()` call.

### System Dependencies (for Bevy app)

On a full desktop Linux system, you'll need:
```bash
sudo apt install -y libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev
```

The library crates (transport, identity, messaging, channel, network) compile
without any system dependencies.

## Code Conventions

- **Crate naming**: `willow-<name>` in Cargo.toml, `willow_<name>` in code
- **Thread safety**: Use `Arc` (not `Rc`) everywhere — all types must be `Send + Sync`
- **Error handling**: `thiserror` for library error types, `anyhow` for application code
- **Documentation**: Every public type and function has a doc comment. Module-level `//!` docs explain the purpose and provide examples.
- **Testing**: Every crate has unit tests. Use `#[cfg(test)] mod tests` at the bottom of each file.
- **Serialization**: All wire types derive `Serialize + Deserialize`. Round-trip tests validate compatibility.

## Architecture Notes

### Dependency Graph

```
willow-app → willow-crypto   → willow-identity → willow-transport
           → willow-network  → willow-identity
           → willow-channel  → willow-identity
           → willow-messaging → willow-identity
                              (defines SealedContent used by willow-crypto)
```

### Async / Sync Boundary

- **Network layer**: Fully async (tokio). Runs on a background thread.
- **Bevy app**: Synchronous ECS. Communicates with the network via `std::sync::mpsc` channels.
- **Bridge**: `network_bridge.rs` in the app crate converts between the two worlds.

### Message Flow

1. User types in Bevy UI → `Message::text()` creates cleartext message
2. If channel key exists → `seal_content()` encrypts Content → `Content::Encrypted`
3. `pack_envelope()` serializes → `NetworkBridgeCommand::Publish`
4. Bridge sends to tokio task → `NetworkNode::publish()`
5. libp2p GossipSub floods to subscribed peers
6. Remote peer receives → `NetworkEvent::Message`
7. Bridge forwards to Bevy → `NetworkBridgeEvent::MessageReceived`
8. `unpack_envelope()` → if `Content::Encrypted`, `open_content()` decrypts
9. Bevy system updates `ChatState` resource → UI re-renders

### Hybrid Logical Clocks (HLC)

Messages are ordered using HLCs (`willow-messaging/src/hlc.rs`). Every node
maintains an `HLC` instance. Call `hlc.now()` for local events and
`hlc.receive(remote_ts)` when processing remote messages. This ensures
consistent ordering even when system clocks drift.

## Common Tasks

### Adding a new message type

1. Add a variant to `Content` in `crates/messaging/src/lib.rs`
2. Add a constructor method on `Message`
3. Add tests
4. Handle the new variant in the app's network event handler

### Adding a new permission

1. Add a variant to `Permission` in `crates/channel/src/lib.rs`
2. Check it in the relevant server methods
3. Add tests

### Adding a new libp2p protocol

1. Add the protocol to `WillowBehaviour` in `crates/network/src/behaviour.rs`
2. Handle its events in `run_swarm()` in `crates/network/src/node.rs`
3. Expose relevant commands/events through `NetworkNode` and `NetworkEvent`
