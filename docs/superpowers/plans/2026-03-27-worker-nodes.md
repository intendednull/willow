# Worker Nodes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce specialized worker nodes (replay, storage) that behave as regular peers with specific infrastructure jobs, replacing the relay's state storage responsibilities.

**Architecture:** A shared `willow-worker` library crate provides the peer lifecycle (identity, networking, heartbeat, actor system). Separate binaries (`willow-replay`, `willow-storage`) implement the `WorkerRole` trait. Workers communicate via a `_willow_workers` gossipsub topic with typed messages. The relay is stripped back to pure network plumbing. Docker Compose provides reproducible deployment.

**Tech Stack:** Rust, libp2p (gossipsub, kademlia), tokio (multi-threaded runtime, mpsc/oneshot channels), SQLite (storage node), Docker

**Spec:** `docs/superpowers/specs/2026-03-27-worker-nodes-design.md`

---

## File Map

### New Crates

```
crates/worker/
├── Cargo.toml
├── src/
│   ├── lib.rs           — Public API: WorkerRole trait, run(), WorkerConfig
│   ├── types.rs         — WorkerRoleInfo, WorkerAnnouncement, WorkerWireMessage,
│   │                      WorkerRequest, WorkerResponse, AllocationStrategy
│   ├── actors/
│   │   ├── mod.rs       — Actor message types (StateMsg, NetworkMsg)
│   │   ├── network.rs   — Network actor: owns libp2p swarm, dispatches events
│   │   ├── state.rs     — State actor: owns WorkerRole, processes events/requests
│   │   ├── heartbeat.rs — Heartbeat actor: periodic WorkerAnnouncement
│   │   └── sync.rs      — Sync actor: periodic SyncRequest broadcasts
│   ├── config.rs        — WorkerConfig, CLI arg parsing (clap)
│   └── identity.rs      — --generate-identity and --print-peer-id commands

crates/replay/
├── Cargo.toml
├── src/
│   ├── main.rs          — Entry point: parse args, create ReplayRole, call run()
│   └── role.rs          — ReplayRole: in-memory ServerState, bounded event buffer,
│                          implements WorkerRole trait

crates/storage/
├── Cargo.toml
├── src/
│   ├── main.rs          — Entry point: parse args, create StorageRole, call run()
│   ├── role.rs          — StorageRole: SQLite-backed, implements WorkerRole trait
│   └── store.rs         — StorageEventStore: SQLite schema, queries, pagination
```

### New Files in Existing Crates

```
crates/client/src/worker_cache.rs  — Client-side worker discovery cache with TTL
```

### Modified Files

```
crates/relay/src/lib.rs            — Remove EventStore, SyncRequest/SyncBatch handling
crates/relay/src/main.rs           — Remove --data-dir arg, event store init
crates/relay/src/event_store.rs    — DELETE this file (moves to storage node)
crates/relay/Cargo.toml            — Remove rusqlite dependency

crates/client/src/lib.rs           — Integrate worker cache, route sync to replay nodes
crates/client/src/ops.rs           — Add WorkerWireMessage pack/unpack, WORKERS_TOPIC const
crates/client/src/network.rs       — Subscribe to _willow_workers, handle worker messages

crates/web/src/app.rs              — Wire up server creation worker step
crates/web/src/components/settings.rs — Add worker nodes stub section
crates/web/src/state.rs            — Add worker-related signals

Cargo.toml                         — Add workspace members: worker, replay, storage
justfile                           — Add build/test/docker commands
docker-compose.yml                 — Full stack definition (NEW)
docker/relay.Dockerfile            — (NEW)
docker/replay.Dockerfile           — (NEW)
docker/storage.Dockerfile          — (NEW)
docker/web.Dockerfile              — (NEW)
```

---

## Task 1: Wire Protocol Types

**Files:**
- Create: `crates/worker/Cargo.toml`
- Create: `crates/worker/src/lib.rs`
- Create: `crates/worker/src/types.rs`
- Modify: `Cargo.toml` (workspace members)

This task defines all shared types that every other task depends on. No networking yet — just data structures and serialization.

- [ ] **Step 1: Create the worker crate skeleton**

Create `crates/worker/Cargo.toml`:

```toml
[package]
name = "willow-worker"
version = "0.1.0"
edition = "2021"

[dependencies]
willow-state = { path = "../state" }
willow-identity = { path = "../identity" }
willow-transport = { path = "../transport" }
willow-network = { path = "../network" }
serde = { workspace = true }
bincode = { workspace = true }
tokio = { version = "1", features = ["full"] }
tracing = { workspace = true }
clap = { version = "4", features = ["derive"] }
anyhow = { workspace = true }
```

Create `crates/worker/src/lib.rs`:

```rust
//! Shared worker node library.
//!
//! Provides the [`WorkerRole`] trait, actor-based runtime, and common
//! peer lifecycle (identity, networking, heartbeat, sync) for all
//! worker node binaries.

pub mod types;

pub use types::*;
```

Add `"crates/worker"` to the workspace members list in the root `Cargo.toml`.

- [ ] **Step 2: Define all worker wire types**

Create `crates/worker/src/types.rs`:

```rust
//! Wire protocol types for the worker node system.
//!
//! All types are serializable for gossipsub transport on the
//! `_willow_workers` topic.

use serde::{Deserialize, Serialize};
use willow_state::{Event, ServerState, StateHash};

/// Gossipsub topic for worker discovery and request/response.
pub const WORKERS_TOPIC: &str = "_willow_workers";

/// Combined role identity and capacity info. The variant determines
/// the role — impossible to mismatch role type and capacity data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkerRoleInfo {
    Replay {
        servers_loaded: u32,
        events_buffered: u32,
        max_events: u32,
    },
    Storage {
        servers_tracked: u32,
        total_events_stored: u64,
        disk_used_bytes: u64,
    },
    // Future: File { ... }, Stream { ... }, Bot { ... }
}

impl WorkerRoleInfo {
    /// Returns the role name as a string for display/logging.
    pub fn role_name(&self) -> &'static str {
        match self {
            WorkerRoleInfo::Replay { .. } => "replay",
            WorkerRoleInfo::Storage { .. } => "storage",
        }
    }
}

/// Periodic heartbeat broadcast by workers on `_willow_workers`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerAnnouncement {
    pub peer_id: String,
    pub role: WorkerRoleInfo,
    pub servers: Vec<String>,
    pub timestamp: u64,
}

/// Top-level wire message for the `_willow_workers` gossipsub topic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerWireMessage {
    /// Periodic heartbeat.
    Announcement(WorkerAnnouncement),

    /// Graceful departure notification.
    Departure { peer_id: String },

    /// Client requesting a service from a specific worker.
    Request {
        request_id: String,
        target_peer: String,
        payload: WorkerRequest,
    },

    /// Worker responding to a client request.
    Response {
        request_id: String,
        target_peer: String,
        payload: WorkerResponse,
    },
}

/// Request payloads sent by clients to workers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerRequest {
    /// State sync request (handled by replay nodes).
    Sync {
        server_id: String,
        state_hash: StateHash,
    },

    /// Paginated history request (handled by storage nodes).
    History {
        server_id: String,
        channel: String,
        before_timestamp: Option<u64>,
        limit: u32,
    },
}

/// Response payloads sent by workers back to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerResponse {
    /// Batch of events for sync catch-up.
    SyncBatch { events: Vec<Event> },

    /// Full state snapshot for far-behind peers.
    Snapshot { state: ServerState },

    /// Paginated history results.
    HistoryPage { events: Vec<Event>, has_more: bool },

    /// Request denied.
    Denied { reason: String },
}

/// The trait that each worker binary implements.
///
/// The state actor owns the implementor exclusively — `&mut self` is
/// safe because no other task can access it concurrently.
pub trait WorkerRole: Send + 'static {
    /// Returns combined role identity and capacity info for heartbeats.
    fn role_info(&self) -> WorkerRoleInfo;

    /// Called when an event is received from gossipsub.
    fn on_event(&mut self, event: &Event);

    /// Handle an incoming request from a client peer.
    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse;
}

/// Allocation strategy for which servers a worker serves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AllocationStrategy {
    /// Serve all discovered servers (initial implementation).
    Global,
    /// Serve only specific servers (future).
    PerServer(Vec<String>),
    /// Dynamic allocation based on load (future).
    Dynamic,
}

impl Default for AllocationStrategy {
    fn default() -> Self {
        AllocationStrategy::Global
    }
}
```

- [ ] **Step 3: Write serialization round-trip tests**

Add tests to `crates/worker/src/types.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use willow_state::StateHash;

    #[test]
    fn worker_role_info_replay_round_trip() {
        let info = WorkerRoleInfo::Replay {
            servers_loaded: 3,
            events_buffered: 500,
            max_events: 1000,
        };
        let bytes = bincode::serialize(&info).unwrap();
        let decoded: WorkerRoleInfo = bincode::deserialize(&bytes).unwrap();
        assert_eq!(info, decoded);
        assert_eq!(info.role_name(), "replay");
    }

    #[test]
    fn worker_role_info_storage_round_trip() {
        let info = WorkerRoleInfo::Storage {
            servers_tracked: 5,
            total_events_stored: 100_000,
            disk_used_bytes: 50_000_000,
        };
        let bytes = bincode::serialize(&info).unwrap();
        let decoded: WorkerRoleInfo = bincode::deserialize(&bytes).unwrap();
        assert_eq!(info, decoded);
        assert_eq!(info.role_name(), "storage");
    }

    #[test]
    fn worker_announcement_round_trip() {
        let ann = WorkerAnnouncement {
            peer_id: "12D3KooWTest".to_string(),
            role: WorkerRoleInfo::Replay {
                servers_loaded: 1,
                events_buffered: 100,
                max_events: 1000,
            },
            servers: vec!["server-1".to_string(), "server-2".to_string()],
            timestamp: 1234567890,
        };
        let bytes = bincode::serialize(&ann).unwrap();
        let decoded: WorkerAnnouncement = bincode::deserialize(&bytes).unwrap();
        assert_eq!(ann, decoded);
    }

    #[test]
    fn worker_wire_message_announcement_round_trip() {
        let msg = WorkerWireMessage::Announcement(WorkerAnnouncement {
            peer_id: "peer1".to_string(),
            role: WorkerRoleInfo::Storage {
                servers_tracked: 2,
                total_events_stored: 5000,
                disk_used_bytes: 1_000_000,
            },
            servers: vec!["s1".to_string()],
            timestamp: 999,
        });
        let bytes = bincode::serialize(&msg).unwrap();
        let decoded: WorkerWireMessage = bincode::deserialize(&bytes).unwrap();
        // Verify it's an announcement
        match decoded {
            WorkerWireMessage::Announcement(a) => {
                assert_eq!(a.peer_id, "peer1");
                assert_eq!(a.servers.len(), 1);
            }
            _ => panic!("expected Announcement"),
        }
    }

    #[test]
    fn worker_wire_message_departure_round_trip() {
        let msg = WorkerWireMessage::Departure {
            peer_id: "leaving-peer".to_string(),
        };
        let bytes = bincode::serialize(&msg).unwrap();
        let decoded: WorkerWireMessage = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerWireMessage::Departure { peer_id } => {
                assert_eq!(peer_id, "leaving-peer");
            }
            _ => panic!("expected Departure"),
        }
    }

    #[test]
    fn worker_request_sync_round_trip() {
        let req = WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        };
        let bytes = bincode::serialize(&req).unwrap();
        let decoded: WorkerRequest = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerRequest::Sync {
                server_id,
                state_hash,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(state_hash, StateHash::ZERO);
            }
            _ => panic!("expected Sync"),
        }
    }

    #[test]
    fn worker_request_history_round_trip() {
        let req = WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "general".to_string(),
            before_timestamp: Some(1000),
            limit: 50,
        };
        let bytes = bincode::serialize(&req).unwrap();
        let decoded: WorkerRequest = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerRequest::History {
                server_id,
                channel,
                before_timestamp,
                limit,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(channel, "general");
                assert_eq!(before_timestamp, Some(1000));
                assert_eq!(limit, 50);
            }
            _ => panic!("expected History"),
        }
    }

    #[test]
    fn worker_request_history_no_cursor_round_trip() {
        let req = WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "general".to_string(),
            before_timestamp: None,
            limit: 25,
        };
        let bytes = bincode::serialize(&req).unwrap();
        let decoded: WorkerRequest = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerRequest::History {
                before_timestamp,
                limit,
                ..
            } => {
                assert_eq!(before_timestamp, None);
                assert_eq!(limit, 25);
            }
            _ => panic!("expected History"),
        }
    }

    #[test]
    fn worker_response_sync_batch_round_trip() {
        let resp = WorkerResponse::SyncBatch { events: vec![] };
        let bytes = bincode::serialize(&resp).unwrap();
        let decoded: WorkerResponse = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerResponse::SyncBatch { events } => assert!(events.is_empty()),
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn worker_response_history_page_round_trip() {
        let resp = WorkerResponse::HistoryPage {
            events: vec![],
            has_more: true,
        };
        let bytes = bincode::serialize(&resp).unwrap();
        let decoded: WorkerResponse = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerResponse::HistoryPage { events, has_more } => {
                assert!(events.is_empty());
                assert!(has_more);
            }
            _ => panic!("expected HistoryPage"),
        }
    }

    #[test]
    fn worker_response_denied_round_trip() {
        let resp = WorkerResponse::Denied {
            reason: "no permission".to_string(),
        };
        let bytes = bincode::serialize(&resp).unwrap();
        let decoded: WorkerResponse = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerResponse::Denied { reason } => {
                assert_eq!(reason, "no permission");
            }
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn worker_wire_message_request_round_trip() {
        let msg = WorkerWireMessage::Request {
            request_id: "req-123".to_string(),
            target_peer: "worker-peer".to_string(),
            payload: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                state_hash: StateHash::ZERO,
            },
        };
        let bytes = bincode::serialize(&msg).unwrap();
        let decoded: WorkerWireMessage = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerWireMessage::Request {
                request_id,
                target_peer,
                ..
            } => {
                assert_eq!(request_id, "req-123");
                assert_eq!(target_peer, "worker-peer");
            }
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn worker_wire_message_response_round_trip() {
        let msg = WorkerWireMessage::Response {
            request_id: "req-456".to_string(),
            target_peer: "client-peer".to_string(),
            payload: WorkerResponse::Denied {
                reason: "unknown server".to_string(),
            },
        };
        let bytes = bincode::serialize(&msg).unwrap();
        let decoded: WorkerWireMessage = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerWireMessage::Response {
                request_id,
                target_peer,
                ..
            } => {
                assert_eq!(request_id, "req-456");
                assert_eq!(target_peer, "client-peer");
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn allocation_strategy_default_is_global() {
        match AllocationStrategy::default() {
            AllocationStrategy::Global => {}
            _ => panic!("expected Global"),
        }
    }
}
```

- [ ] **Step 4: Verify tests pass**

Run: `cargo test -p willow-worker`
Expected: All 13 tests pass.

- [ ] **Step 5: Verify WASM compatibility**

Run: `cargo check -p willow-worker --target wasm32-unknown-unknown`
Expected: Compiles without errors. The types crate has no platform-specific code.

- [ ] **Step 6: Commit**

```bash
git add crates/worker/ Cargo.toml
git commit -m "feat: add willow-worker crate with wire protocol types

Defines WorkerRole trait, WorkerRoleInfo, WorkerAnnouncement,
WorkerWireMessage, WorkerRequest, WorkerResponse, and AllocationStrategy.
All types are serializable for gossipsub transport."
```

---

## Task 2: Worker Config and Identity CLI

**Files:**
- Create: `crates/worker/src/config.rs`
- Create: `crates/worker/src/identity.rs`
- Modify: `crates/worker/src/lib.rs`

Workers need to generate persistent identities and parse CLI args. This task builds the config and identity plumbing that both binaries share.

- [ ] **Step 1: Write config tests**

Add to `crates/worker/src/config.rs`:

```rust
//! Worker configuration and CLI argument parsing.

use crate::AllocationStrategy;

/// Configuration for a worker node.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Path to the Ed25519 identity keypair file.
    pub identity_path: String,
    /// Relay multiaddr to connect through.
    pub relay_addr: String,
    /// Sync interval in seconds.
    pub sync_interval_secs: u64,
    /// Allocation strategy.
    pub allocation: AllocationStrategy,
}

impl WorkerConfig {
    /// Create a config for testing.
    pub fn test_config() -> Self {
        Self {
            identity_path: "/tmp/test-worker.key".to_string(),
            relay_addr: "/ip4/127.0.0.1/tcp/9091/ws/p2p/12D3KooWTest".to_string(),
            sync_interval_secs: 30,
            allocation: AllocationStrategy::Global,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_has_defaults() {
        let cfg = WorkerConfig::test_config();
        assert_eq!(cfg.sync_interval_secs, 30);
        assert!(matches!(cfg.allocation, AllocationStrategy::Global));
    }
}
```

- [ ] **Step 2: Write identity generation and loading**

Create `crates/worker/src/identity.rs`:

```rust
//! Identity management for worker nodes.
//!
//! Provides `--generate-identity` and `--print-peer-id` CLI commands.

use anyhow::Result;
use willow_identity::Identity;

/// Generate a new Ed25519 identity and save it to `path`.
///
/// If a file already exists at `path`, returns an error to prevent
/// accidental overwrites.
pub fn generate_identity(path: &str) -> Result<Identity> {
    let p = std::path::Path::new(path);
    if p.exists() {
        anyhow::bail!("identity file already exists: {path}");
    }
    let identity = Identity::load_or_generate(path)?;
    Ok(identity)
}

/// Load an existing identity from `path`, or generate if absent.
pub fn load_or_generate(path: &str) -> Result<Identity> {
    Identity::load_or_generate(path).map_err(|e| anyhow::anyhow!("failed to load identity: {e}"))
}

/// Print the peer ID for an identity file. Used by operators to
/// collect worker peer IDs for `PLATFORM_WORKERS`.
pub fn print_peer_id(path: &str) -> Result<()> {
    let identity = load_or_generate(path)?;
    println!("{}", identity.peer_id());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn generate_creates_new_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.key");
        let path_str = path.to_str().unwrap();

        let id = generate_identity(path_str).unwrap();
        assert!(path.exists());
        // Peer ID should be a non-empty string.
        assert!(!id.peer_id().to_string().is_empty());
    }

    #[test]
    fn generate_refuses_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.key");
        fs::write(&path, b"existing data").unwrap();

        let result = generate_identity(path.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn load_or_generate_creates_if_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.key");
        let path_str = path.to_str().unwrap();

        let id = load_or_generate(path_str).unwrap();
        assert!(path.exists());
        assert!(!id.peer_id().to_string().is_empty());
    }

    #[test]
    fn load_or_generate_reloads_same_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reload.key");
        let path_str = path.to_str().unwrap();

        let id1 = load_or_generate(path_str).unwrap();
        let id2 = load_or_generate(path_str).unwrap();
        assert_eq!(id1.peer_id().to_string(), id2.peer_id().to_string());
    }
}
```

- [ ] **Step 3: Update lib.rs exports**

```rust
//! Shared worker node library.
//!
//! Provides the [`WorkerRole`] trait, actor-based runtime, and common
//! peer lifecycle (identity, networking, heartbeat, sync) for all
//! worker node binaries.

pub mod config;
pub mod identity;
pub mod types;

pub use config::WorkerConfig;
pub use types::*;
```

- [ ] **Step 4: Add tempfile dev-dependency**

Add to `crates/worker/Cargo.toml` under `[dev-dependencies]`:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p willow-worker`
Expected: All tests pass (13 type tests + 1 config test + 4 identity tests = 18).

- [ ] **Step 6: Commit**

```bash
git add crates/worker/
git commit -m "feat: add worker config and identity CLI helpers

Supports --generate-identity and --print-peer-id for operator
workflows. Identity files are persistent Ed25519 keypairs."
```

---

## Task 3: Actor System — Message Types and State Actor

**Files:**
- Create: `crates/worker/src/actors/mod.rs`
- Create: `crates/worker/src/actors/state.rs`
- Modify: `crates/worker/src/lib.rs`

The state actor is the core of the worker runtime. It owns the `WorkerRole` implementor exclusively and processes events and requests sequentially via channels.

- [ ] **Step 1: Define actor message types**

Create `crates/worker/src/actors/mod.rs`:

```rust
//! Actor-based concurrency system for worker nodes.
//!
//! Four actors communicate via tokio channels:
//! - Network actor: owns libp2p swarm
//! - State actor: owns WorkerRole + mutable state
//! - Heartbeat actor: periodic announcements
//! - Sync actor: periodic state sync

pub mod state;

use tokio::sync::oneshot;

use crate::types::{WorkerRequest, WorkerResponse, WorkerRoleInfo};
use willow_state::{Event, StateHash};

/// Messages sent to the state actor.
pub enum StateMsg {
    /// A new event arrived from gossipsub.
    Event(Event),

    /// A client request that needs a response.
    Request {
        req: WorkerRequest,
        reply: oneshot::Sender<WorkerResponse>,
    },

    /// Heartbeat actor asking for current role info.
    GetRoleInfo {
        reply: oneshot::Sender<WorkerRoleInfo>,
    },

    /// Sync actor asking for current state hashes per server.
    GetStateHashes {
        reply: oneshot::Sender<Vec<(String, StateHash)>>,
    },

    /// A server was discovered — add it to the set of tracked servers.
    ServerDiscovered { server_id: String },

    /// Shutdown signal.
    Shutdown,
}

/// Messages sent to the network actor for outbound publishing.
pub enum NetworkOutMsg {
    /// Publish raw bytes on a gossipsub topic.
    Publish { topic: String, data: Vec<u8> },

    /// Subscribe to a gossipsub topic.
    Subscribe(String),
}
```

- [ ] **Step 2: Implement the state actor**

Create `crates/worker/src/actors/state.rs`:

```rust
//! State actor — owns the [`WorkerRole`] and processes messages sequentially.
//!
//! All mutable state access goes through this actor. No locks needed
//! because only this task touches the role.

use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::StateMsg;
use crate::WorkerRole;

/// Run the state actor loop.
///
/// Receives messages on `rx`, dispatches to the `role` implementation.
/// Exits when `rx` is closed (all senders dropped) or Shutdown is received.
pub async fn run(mut role: Box<dyn WorkerRole>, mut rx: mpsc::Receiver<StateMsg>) {
    debug!("state actor started");

    while let Some(msg) = rx.recv().await {
        match msg {
            StateMsg::Event(event) => {
                role.on_event(&event);
            }
            StateMsg::Request { req, reply } => {
                let response = role.handle_request(req);
                if reply.send(response).is_err() {
                    warn!("request reply channel closed");
                }
            }
            StateMsg::GetRoleInfo { reply } => {
                let info = role.role_info();
                let _ = reply.send(info);
            }
            StateMsg::GetStateHashes { reply } => {
                // Default: no state hashes. Override by wrapping the role
                // or extending the trait later. For now replay nodes
                // implement this via a wrapper that tracks server states.
                let _ = reply.send(vec![]);
            }
            StateMsg::ServerDiscovered { server_id } => {
                debug!(%server_id, "server discovered by state actor");
                // Future: allocation strategy filtering would go here.
            }
            StateMsg::Shutdown => {
                debug!("state actor shutting down");
                break;
            }
        }
    }

    debug!("state actor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        WorkerRequest, WorkerResponse, WorkerRoleInfo,
    };
    use tokio::sync::oneshot;
    use willow_state::Event;

    /// A minimal test role that counts events and echoes requests.
    struct TestRole {
        event_count: u32,
    }

    impl TestRole {
        fn new() -> Self {
            Self { event_count: 0 }
        }
    }

    impl WorkerRole for TestRole {
        fn role_info(&self) -> WorkerRoleInfo {
            WorkerRoleInfo::Replay {
                servers_loaded: 1,
                events_buffered: self.event_count,
                max_events: 100,
            }
        }

        fn on_event(&mut self, _event: &Event) {
            self.event_count += 1;
        }

        fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
            match req {
                WorkerRequest::Sync { .. } => WorkerResponse::SyncBatch {
                    events: vec![],
                },
                WorkerRequest::History { .. } => WorkerResponse::Denied {
                    reason: "not a storage node".to_string(),
                },
            }
        }
    }

    fn make_test_event() -> Event {
        Event {
            id: uuid::Uuid::new_v4().to_string(),
            parent_hash: willow_state::StateHash::ZERO,
            author: "test-peer".to_string(),
            timestamp_ms: 1000,
            kind: willow_state::EventKind::Message {
                channel_id: "general".to_string(),
                body: "hello".to_string(),
                reply_to: None,
            },
        }
    }

    #[tokio::test]
    async fn state_actor_processes_events() {
        let (tx, rx) = mpsc::channel(32);
        let role = Box::new(TestRole::new());

        let handle = tokio::spawn(run(role, rx));

        // Send 3 events.
        for _ in 0..3 {
            tx.send(StateMsg::Event(make_test_event())).await.unwrap();
        }

        // Query role info — should show 3 events buffered.
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(StateMsg::GetRoleInfo { reply: reply_tx })
            .await
            .unwrap();
        let info = reply_rx.await.unwrap();
        match info {
            WorkerRoleInfo::Replay {
                events_buffered, ..
            } => assert_eq!(events_buffered, 3),
            _ => panic!("expected Replay"),
        }

        // Shutdown.
        tx.send(StateMsg::Shutdown).await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn state_actor_handles_requests() {
        let (tx, rx) = mpsc::channel(32);
        let role = Box::new(TestRole::new());

        let handle = tokio::spawn(run(role, rx));

        // Send a sync request.
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(StateMsg::Request {
            req: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                state_hash: willow_state::StateHash::ZERO,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();

        let resp = reply_rx.await.unwrap();
        match resp {
            WorkerResponse::SyncBatch { events } => assert!(events.is_empty()),
            _ => panic!("expected SyncBatch"),
        }

        // Send a history request (should be denied by replay role).
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(StateMsg::Request {
            req: WorkerRequest::History {
                server_id: "srv".to_string(),
                channel: "general".to_string(),
                before_timestamp: None,
                limit: 50,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();

        let resp = reply_rx.await.unwrap();
        match resp {
            WorkerResponse::Denied { reason } => {
                assert!(reason.contains("not a storage"));
            }
            _ => panic!("expected Denied"),
        }

        tx.send(StateMsg::Shutdown).await.unwrap();
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn state_actor_exits_on_channel_close() {
        let (tx, rx) = mpsc::channel(32);
        let role = Box::new(TestRole::new());

        let handle = tokio::spawn(run(role, rx));

        // Drop sender — actor should exit cleanly.
        drop(tx);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn state_actor_handles_multiple_concurrent_requests() {
        let (tx, rx) = mpsc::channel(64);
        let role = Box::new(TestRole::new());

        let handle = tokio::spawn(run(role, rx));

        // Send 10 requests without waiting for replies.
        let mut reply_rxs = vec![];
        for _ in 0..10 {
            let (reply_tx, reply_rx) = oneshot::channel();
            tx.send(StateMsg::Request {
                req: WorkerRequest::Sync {
                    server_id: "srv".to_string(),
                    state_hash: willow_state::StateHash::ZERO,
                },
                reply: reply_tx,
            })
            .await
            .unwrap();
            reply_rxs.push(reply_rx);
        }

        // All 10 should resolve.
        for rx in reply_rxs {
            let resp = rx.await.unwrap();
            match resp {
                WorkerResponse::SyncBatch { .. } => {}
                _ => panic!("expected SyncBatch"),
            }
        }

        tx.send(StateMsg::Shutdown).await.unwrap();
        handle.await.unwrap();
    }
}
```

- [ ] **Step 3: Update lib.rs**

```rust
//! Shared worker node library.

pub mod actors;
pub mod config;
pub mod identity;
pub mod types;

pub use config::WorkerConfig;
pub use types::*;
```

- [ ] **Step 4: Add uuid dev-dependency**

Add to `crates/worker/Cargo.toml`:

```toml
[dependencies]
# ... existing ...
uuid = { workspace = true }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p willow-worker`
Expected: All tests pass (18 previous + 4 state actor tests = 22).

- [ ] **Step 6: Commit**

```bash
git add crates/worker/
git commit -m "feat: add state actor with channel-based message passing

State actor owns the WorkerRole exclusively, processes events and
requests sequentially via mpsc channels. No locks needed."
```

---

## Task 4: Heartbeat and Sync Actors

**Files:**
- Create: `crates/worker/src/actors/heartbeat.rs`
- Create: `crates/worker/src/actors/sync.rs`
- Modify: `crates/worker/src/actors/mod.rs`

- [ ] **Step 1: Implement heartbeat actor**

Create `crates/worker/src/actors/heartbeat.rs`:

```rust
//! Heartbeat actor — broadcasts [`WorkerAnnouncement`] periodically.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::debug;

use super::{NetworkOutMsg, StateMsg};
use crate::types::{WorkerAnnouncement, WorkerWireMessage, WORKERS_TOPIC};

/// Run the heartbeat actor loop.
///
/// Every `interval` seconds, queries the state actor for role info
/// and broadcasts an announcement via the network actor.
pub async fn run(
    peer_id: String,
    interval: Duration,
    state_tx: mpsc::Sender<StateMsg>,
    network_tx: mpsc::Sender<NetworkOutMsg>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    debug!("heartbeat actor started (interval: {:?})", interval);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    // Send departure before exiting.
                    let departure = WorkerWireMessage::Departure {
                        peer_id: peer_id.clone(),
                    };
                    if let Ok(bytes) = bincode::serialize(&departure) {
                        let _ = network_tx
                            .send(NetworkOutMsg::Publish {
                                topic: WORKERS_TOPIC.to_string(),
                                data: bytes,
                            })
                            .await;
                    }
                    debug!("heartbeat actor shutting down");
                    return;
                }
            }
        }

        // Query state actor for role info.
        let (reply_tx, reply_rx) = oneshot::channel();
        if state_tx
            .send(StateMsg::GetRoleInfo { reply: reply_tx })
            .await
            .is_err()
        {
            break;
        }

        let role_info = match reply_rx.await {
            Ok(info) => info,
            Err(_) => break,
        };

        let announcement = WorkerAnnouncement {
            peer_id: peer_id.clone(),
            role: role_info,
            servers: vec![], // Populated by state actor in the full runtime
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };

        let msg = WorkerWireMessage::Announcement(announcement);
        if let Ok(bytes) = bincode::serialize(&msg) {
            let _ = network_tx
                .send(NetworkOutMsg::Publish {
                    topic: WORKERS_TOPIC.to_string(),
                    data: bytes,
                })
                .await;
        }
    }

    debug!("heartbeat actor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WorkerRoleInfo;

    /// Minimal role info responder for testing.
    async fn fake_state_actor(mut rx: mpsc::Receiver<StateMsg>) {
        while let Some(msg) = rx.recv().await {
            match msg {
                StateMsg::GetRoleInfo { reply } => {
                    let _ = reply.send(WorkerRoleInfo::Replay {
                        servers_loaded: 1,
                        events_buffered: 42,
                        max_events: 1000,
                    });
                }
                StateMsg::Shutdown => break,
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn heartbeat_sends_announcements() {
        let (state_tx, state_rx) = mpsc::channel(32);
        let (network_tx, mut network_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Spawn fake state actor.
        tokio::spawn(fake_state_actor(state_rx));

        // Spawn heartbeat with very short interval.
        let hb = tokio::spawn(run(
            "test-peer".to_string(),
            Duration::from_millis(50),
            state_tx.clone(),
            network_tx,
            shutdown_rx,
        ));

        // Wait for at least 2 announcements.
        let msg1 = tokio::time::timeout(Duration::from_secs(1), network_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let msg2 = tokio::time::timeout(Duration::from_secs(1), network_rx.recv())
            .await
            .unwrap()
            .unwrap();

        // Verify they're announcements on the correct topic.
        match msg1 {
            NetworkOutMsg::Publish { topic, data } => {
                assert_eq!(topic, WORKERS_TOPIC);
                let decoded: WorkerWireMessage = bincode::deserialize(&data).unwrap();
                match decoded {
                    WorkerWireMessage::Announcement(a) => {
                        assert_eq!(a.peer_id, "test-peer");
                    }
                    _ => panic!("expected Announcement"),
                }
            }
            _ => panic!("expected Publish"),
        }

        // Matches second announcement pattern.
        assert!(matches!(msg2, NetworkOutMsg::Publish { .. }));

        // Shutdown.
        shutdown_tx.send(true).unwrap();
        hb.await.unwrap();

        // Check departure message was sent.
        let departure = tokio::time::timeout(Duration::from_millis(100), network_rx.recv())
            .await
            .unwrap()
            .unwrap();
        match departure {
            NetworkOutMsg::Publish { data, .. } => {
                let decoded: WorkerWireMessage = bincode::deserialize(&data).unwrap();
                assert!(matches!(decoded, WorkerWireMessage::Departure { .. }));
            }
            _ => panic!("expected departure Publish"),
        }
    }
}
```

- [ ] **Step 2: Implement sync actor**

Create `crates/worker/src/actors/sync.rs`:

```rust
//! Sync actor — periodically broadcasts SyncRequests for state convergence.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::debug;

use super::{NetworkOutMsg, StateMsg};
use crate::types::{WorkerRequest, WorkerWireMessage, WORKERS_TOPIC};

/// Run the sync actor loop.
///
/// Every `interval`, queries the state actor for state hashes per server
/// and broadcasts SyncRequests so other peers/workers can send missing events.
pub async fn run(
    peer_id: String,
    interval: Duration,
    state_tx: mpsc::Sender<StateMsg>,
    network_tx: mpsc::Sender<NetworkOutMsg>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    debug!("sync actor started (interval: {:?})", interval);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    debug!("sync actor shutting down");
                    return;
                }
            }
        }

        // Query state actor for current state hashes.
        let (reply_tx, reply_rx) = oneshot::channel();
        if state_tx
            .send(StateMsg::GetStateHashes { reply: reply_tx })
            .await
            .is_err()
        {
            break;
        }

        let hashes = match reply_rx.await {
            Ok(h) => h,
            Err(_) => break,
        };

        // Broadcast a sync request for each server.
        for (server_id, state_hash) in hashes {
            let msg = WorkerWireMessage::Request {
                request_id: uuid::Uuid::new_v4().to_string(),
                target_peer: String::new(), // Broadcast — any replay/peer can respond
                payload: WorkerRequest::Sync {
                    server_id,
                    state_hash,
                },
            };
            if let Ok(bytes) = bincode::serialize(&msg) {
                let _ = network_tx
                    .send(NetworkOutMsg::Publish {
                        topic: WORKERS_TOPIC.to_string(),
                        data: bytes,
                    })
                    .await;
            }
        }
    }

    debug!("sync actor stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_state::StateHash;

    /// Fake state actor that returns known state hashes.
    async fn fake_state_actor(mut rx: mpsc::Receiver<StateMsg>) {
        while let Some(msg) = rx.recv().await {
            match msg {
                StateMsg::GetStateHashes { reply } => {
                    let _ = reply.send(vec![
                        ("server-a".to_string(), StateHash::ZERO),
                        ("server-b".to_string(), StateHash::ZERO),
                    ]);
                }
                StateMsg::Shutdown => break,
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn sync_actor_broadcasts_sync_requests() {
        let (state_tx, state_rx) = mpsc::channel(32);
        let (network_tx, mut network_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        tokio::spawn(fake_state_actor(state_rx));

        let sync = tokio::spawn(run(
            "test-peer".to_string(),
            Duration::from_millis(50),
            state_tx,
            network_tx,
            shutdown_rx,
        ));

        // Should get 2 sync requests (one per server) per interval.
        let msg1 = tokio::time::timeout(Duration::from_secs(1), network_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let msg2 = tokio::time::timeout(Duration::from_secs(1), network_rx.recv())
            .await
            .unwrap()
            .unwrap();

        let mut server_ids = vec![];
        for msg in [msg1, msg2] {
            match msg {
                NetworkOutMsg::Publish { topic, data } => {
                    assert_eq!(topic, WORKERS_TOPIC);
                    let decoded: WorkerWireMessage = bincode::deserialize(&data).unwrap();
                    match decoded {
                        WorkerWireMessage::Request { payload, .. } => match payload {
                            WorkerRequest::Sync { server_id, .. } => {
                                server_ids.push(server_id);
                            }
                            _ => panic!("expected Sync request"),
                        },
                        _ => panic!("expected Request"),
                    }
                }
                _ => panic!("expected Publish"),
            }
        }

        server_ids.sort();
        assert_eq!(server_ids, vec!["server-a", "server-b"]);

        shutdown_tx.send(true).unwrap();
        sync.await.unwrap();
    }

    #[tokio::test]
    async fn sync_actor_exits_on_shutdown() {
        let (state_tx, _state_rx) = mpsc::channel(32);
        let (network_tx, _network_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        let sync = tokio::spawn(run(
            "test-peer".to_string(),
            Duration::from_secs(60), // Long interval — won't fire
            state_tx,
            network_tx,
            shutdown_rx,
        ));

        // Immediate shutdown.
        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), sync)
            .await
            .unwrap()
            .unwrap();
    }
}
```

- [ ] **Step 3: Update actors/mod.rs with new modules**

```rust
//! Actor-based concurrency system for worker nodes.

pub mod heartbeat;
pub mod state;
pub mod sync;

// ... existing message types unchanged ...
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p willow-worker`
Expected: All tests pass (22 previous + 2 heartbeat + 2 sync = 26).

- [ ] **Step 5: Commit**

```bash
git add crates/worker/
git commit -m "feat: add heartbeat and sync actors

Heartbeat broadcasts WorkerAnnouncement every N seconds, sends
Departure on shutdown. Sync broadcasts SyncRequest per server
for active state convergence."
```

---

## Task 5: Replay Node Binary

**Files:**
- Create: `crates/replay/Cargo.toml`
- Create: `crates/replay/src/main.rs`
- Create: `crates/replay/src/role.rs`
- Modify: `Cargo.toml` (workspace members)

The replay node keeps in-memory `ServerState` per server with a bounded event buffer. It responds to sync requests with event diffs or full state snapshots.

- [ ] **Step 1: Create the replay crate**

Create `crates/replay/Cargo.toml`:

```toml
[package]
name = "willow-replay"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "willow-replay"
path = "src/main.rs"

[dependencies]
willow-worker = { path = "../worker" }
willow-state = { path = "../state" }
willow-identity = { path = "../identity" }
clap = { version = "4", features = ["derive"] }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
uuid = { workspace = true }
```

Add `"crates/replay"` to the root `Cargo.toml` workspace members.

- [ ] **Step 2: Write tests for ReplayRole**

Create `crates/replay/src/role.rs`:

```rust
//! Replay node role implementation.
//!
//! Maintains in-memory [`ServerState`] per server with a bounded event
//! buffer. Responds to sync requests with event diffs or full state
//! snapshots for far-behind peers.

use std::collections::{HashMap, VecDeque};

use willow_state::{Event, EventKind, ServerState, StateHash};
use willow_worker::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};

/// Per-server state held by the replay node.
struct ServerData {
    /// Current computed state.
    state: ServerState,
    /// Bounded event buffer (oldest at front).
    events: VecDeque<Event>,
    /// Max events to retain.
    max_events: usize,
}

impl ServerData {
    fn new(server_id: &str, owner: &str, max_events: usize) -> Self {
        Self {
            state: ServerState::new(server_id, server_id, owner.to_string()),
            events: VecDeque::new(),
            max_events,
        }
    }
}

/// Configuration for the replay role.
pub struct ReplayConfig {
    /// Max events per server before eviction.
    pub max_events_per_server: usize,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            max_events_per_server: 1000,
        }
    }
}

/// The replay node's WorkerRole implementation.
pub struct ReplayRole {
    servers: HashMap<String, ServerData>,
    config: ReplayConfig,
}

impl ReplayRole {
    pub fn new(config: ReplayConfig) -> Self {
        Self {
            servers: HashMap::new(),
            config,
        }
    }

    /// Get or create server data for a server ID.
    /// The owner is extracted from the event's author for genesis events.
    fn get_or_create_server(&mut self, server_id: &str, owner: &str) -> &mut ServerData {
        self.servers
            .entry(server_id.to_string())
            .or_insert_with(|| {
                ServerData::new(server_id, owner, self.config.max_events_per_server)
            })
    }

    /// Find events after the given state hash in a server's buffer.
    fn events_since_hash(
        &self,
        server_id: &str,
        hash: &StateHash,
    ) -> Option<Vec<Event>> {
        let data = self.servers.get(server_id)?;

        // If hash is ZERO, send all buffered events.
        if *hash == StateHash::ZERO {
            return Some(data.events.iter().cloned().collect());
        }

        // Find first event whose parent_hash matches.
        for (i, event) in data.events.iter().enumerate() {
            if event.parent_hash == *hash {
                return Some(data.events.iter().skip(i).cloned().collect());
            }
        }

        // Hash not found in buffer — caller is too far behind.
        None
    }
}

impl WorkerRole for ReplayRole {
    fn role_info(&self) -> WorkerRoleInfo {
        let total_events: u32 = self
            .servers
            .values()
            .map(|s| s.events.len() as u32)
            .sum();
        WorkerRoleInfo::Replay {
            servers_loaded: self.servers.len() as u32,
            events_buffered: total_events,
            max_events: self.config.max_events_per_server as u32,
        }
    }

    fn on_event(&mut self, event: &Event) {
        // Determine server_id from the event. For channel events the
        // topic encodes the server, but since we receive events via
        // the state actor (which knows the server), we use a convention:
        // the event is tagged with a server_id extracted from the
        // gossipsub topic by the network actor.
        //
        // For now, use a simple approach: check if we already track a
        // server that contains this event's parent hash, or create a
        // new one from channel events.

        // Try to find existing server by checking all tracked servers.
        let server_id = self
            .servers
            .keys()
            .find(|id| {
                self.servers[*id]
                    .state
                    .seen_event_ids
                    .contains(&event.id)
                    || self.servers[*id].state.hash() == event.parent_hash
            })
            .cloned();

        // If no server found and this looks like a genesis event, create.
        let server_id = match server_id {
            Some(id) => id,
            None => {
                // Use a placeholder — in the full runtime, server_id
                // comes from the gossipsub topic.
                "default".to_string()
            }
        };

        let data = self.get_or_create_server(&server_id, &event.author);

        // Apply event to state.
        willow_state::apply_lenient(&mut data.state, event);

        // Add to event buffer.
        data.events.push_back(event.clone());

        // Evict oldest if over limit.
        while data.events.len() > data.max_events {
            data.events.pop_front();
        }
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::Sync {
                server_id,
                state_hash,
            } => {
                match self.events_since_hash(&server_id, &state_hash) {
                    Some(events) => WorkerResponse::SyncBatch { events },
                    None => {
                        // Client is too far behind — send full snapshot.
                        match self.servers.get(&server_id) {
                            Some(data) => WorkerResponse::Snapshot {
                                state: data.state.clone(),
                            },
                            None => WorkerResponse::Denied {
                                reason: format!("unknown server: {server_id}"),
                            },
                        }
                    }
                }
            }
            WorkerRequest::History { .. } => WorkerResponse::Denied {
                reason: "replay nodes do not serve history".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_state::{EventKind, StateHash};

    fn make_event(id: &str, author: &str, kind: EventKind) -> Event {
        Event {
            id: id.to_string(),
            parent_hash: StateHash::ZERO,
            author: author.to_string(),
            timestamp_ms: 1000,
            kind,
        }
    }

    fn make_message_event(id: &str) -> Event {
        make_event(
            id,
            "peer-1",
            EventKind::Message {
                channel_id: "general".to_string(),
                body: format!("message {id}"),
                reply_to: None,
            },
        )
    }

    #[test]
    fn role_info_starts_empty() {
        let role = ReplayRole::new(ReplayConfig::default());
        let info = role.role_info();
        match info {
            WorkerRoleInfo::Replay {
                servers_loaded,
                events_buffered,
                max_events,
            } => {
                assert_eq!(servers_loaded, 0);
                assert_eq!(events_buffered, 0);
                assert_eq!(max_events, 1000);
            }
            _ => panic!("expected Replay"),
        }
    }

    #[test]
    fn on_event_applies_and_buffers() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 100,
        });

        // Manually create a server so we can control its ID.
        role.get_or_create_server("srv-1", "owner");

        let event = make_message_event("evt-1");
        // Simulate the event being routed to srv-1 by the network layer.
        let data = role.servers.get_mut("srv-1").unwrap();
        willow_state::apply_lenient(&mut data.state, &event);
        data.events.push_back(event);

        assert_eq!(role.servers["srv-1"].events.len(), 1);
    }

    #[test]
    fn bounded_buffer_evicts_oldest() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 3,
        });

        role.get_or_create_server("srv-1", "owner");
        let data = role.servers.get_mut("srv-1").unwrap();

        for i in 0..5 {
            let event = make_message_event(&format!("evt-{i}"));
            willow_state::apply_lenient(&mut data.state, &event);
            data.events.push_back(event);
            while data.events.len() > 3 {
                data.events.pop_front();
            }
        }

        // Only last 3 events should remain.
        assert_eq!(data.events.len(), 3);
        assert_eq!(data.events[0].id, "evt-2");
        assert_eq!(data.events[1].id, "evt-3");
        assert_eq!(data.events[2].id, "evt-4");
    }

    #[test]
    fn sync_request_returns_events_since_zero() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 100,
        });

        role.get_or_create_server("srv-1", "owner");
        let data = role.servers.get_mut("srv-1").unwrap();
        for i in 0..3 {
            let event = make_message_event(&format!("evt-{i}"));
            willow_state::apply_lenient(&mut data.state, &event);
            data.events.push_back(event);
        }

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        });

        match resp {
            WorkerResponse::SyncBatch { events } => {
                assert_eq!(events.len(), 3);
            }
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn sync_request_unknown_server_denied() {
        let mut role = ReplayRole::new(ReplayConfig::default());

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "nonexistent".to_string(),
            state_hash: StateHash::ZERO,
        });

        match resp {
            WorkerResponse::Denied { reason } => {
                assert!(reason.contains("unknown server"));
            }
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn sync_request_far_behind_returns_snapshot() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 2,
        });

        role.get_or_create_server("srv-1", "owner");
        let data = role.servers.get_mut("srv-1").unwrap();
        // Add 5 events, buffer only holds 2.
        for i in 0..5 {
            let event = make_message_event(&format!("evt-{i}"));
            willow_state::apply_lenient(&mut data.state, &event);
            data.events.push_back(event);
            while data.events.len() > 2 {
                data.events.pop_front();
            }
        }

        // Request with a hash that's no longer in the buffer.
        let fake_old_hash = StateHash::from_bytes(b"old state");
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: fake_old_hash,
        });

        match resp {
            WorkerResponse::Snapshot { state } => {
                assert_eq!(state.server_id, "srv-1");
            }
            _ => panic!("expected Snapshot, got {:?}", resp),
        }
    }

    #[test]
    fn history_request_denied_by_replay_node() {
        let mut role = ReplayRole::new(ReplayConfig::default());

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "general".to_string(),
            before_timestamp: None,
            limit: 50,
        });

        match resp {
            WorkerResponse::Denied { reason } => {
                assert!(reason.contains("history"));
            }
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn role_info_reflects_buffered_events() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 100,
        });

        role.get_or_create_server("srv-1", "owner");
        let data = role.servers.get_mut("srv-1").unwrap();
        for i in 0..5 {
            let event = make_message_event(&format!("evt-{i}"));
            data.events.push_back(event);
        }

        match role.role_info() {
            WorkerRoleInfo::Replay {
                servers_loaded,
                events_buffered,
                ..
            } => {
                assert_eq!(servers_loaded, 1);
                assert_eq!(events_buffered, 5);
            }
            _ => panic!("expected Replay"),
        }
    }
}
```

- [ ] **Step 3: Create main.rs entry point**

Create `crates/replay/src/main.rs`:

```rust
//! Willow Replay Node — fast bounded-memory state sync worker.

mod role;

use clap::Parser;

#[derive(Parser)]
#[command(name = "willow-replay", about = "Willow replay worker node")]
struct Cli {
    /// Path to the Ed25519 identity keypair file.
    #[arg(long, default_value = "/etc/willow/replay.key")]
    identity_path: String,

    /// Relay multiaddr to connect through.
    #[arg(long)]
    relay: Option<String>,

    /// Max events per server to buffer in memory.
    #[arg(long, default_value = "1000")]
    max_events_per_server: usize,

    /// Active sync interval in seconds.
    #[arg(long, default_value = "30")]
    sync_interval: u64,

    /// Generate a new identity and exit.
    #[arg(long)]
    generate_identity: bool,

    /// Print the peer ID for the identity file and exit.
    #[arg(long)]
    print_peer_id: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    if cli.generate_identity {
        willow_worker::identity::generate_identity(&cli.identity_path)?;
        tracing::info!("identity generated at {}", cli.identity_path);
        return Ok(());
    }

    if cli.print_peer_id {
        return willow_worker::identity::print_peer_id(&cli.identity_path);
    }

    tracing::info!(
        max_events = cli.max_events_per_server,
        sync_interval = cli.sync_interval,
        "starting replay node"
    );

    // Full runtime integration (network actor, etc.) will be wired
    // in a later task. For now, the role and actors are independently
    // testable.
    tracing::info!("replay node ready (runtime not yet wired)");
    Ok(())
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p willow-replay`
Expected: All 8 replay role tests pass.

- [ ] **Step 5: Build the binary**

Run: `cargo build -p willow-replay`
Expected: Compiles successfully. `--help` shows CLI options.

- [ ] **Step 6: Commit**

```bash
git add crates/replay/ Cargo.toml
git commit -m "feat: add willow-replay binary with ReplayRole

In-memory ServerState per server, bounded event buffer with eviction,
sync request handling with event diff or full snapshot fallback."
```

---

## Task 6: Storage Node Binary

**Files:**
- Create: `crates/storage/Cargo.toml`
- Create: `crates/storage/src/main.rs`
- Create: `crates/storage/src/role.rs`
- Create: `crates/storage/src/store.rs`
- Modify: `Cargo.toml` (workspace members)

The storage node persists events to SQLite and serves paginated history queries.

- [ ] **Step 1: Create the storage crate**

Create `crates/storage/Cargo.toml`:

```toml
[package]
name = "willow-storage"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "willow-storage"
path = "src/main.rs"

[dependencies]
willow-worker = { path = "../worker" }
willow-state = { path = "../state" }
willow-identity = { path = "../identity" }
rusqlite = { version = "0.31", features = ["bundled"] }
clap = { version = "4", features = ["derive"] }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tokio = { version = "1", features = ["full"] }
bincode = { workspace = true }
serde = { workspace = true }

[dev-dependencies]
tempfile = "3"
uuid = { workspace = true }
```

Add `"crates/storage"` to root `Cargo.toml` workspace members.

- [ ] **Step 2: Implement the SQLite store with tests**

Create `crates/storage/src/store.rs`:

```rust
//! SQLite-backed event store for the storage node.
//!
//! Stores all events indefinitely and serves paginated history queries.

use rusqlite::{params, Connection, Result as SqlResult};
use willow_state::Event;

/// SQLite-backed event store.
pub struct StorageEventStore {
    conn: Connection,
}

impl StorageEventStore {
    /// Open or create the database at `path`.
    pub fn open(path: &str) -> anyhow::Result<Self> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(path)?
        };
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                server_id TEXT NOT NULL,
                channel_id TEXT NOT NULL DEFAULT '',
                author TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                event_data BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_server ON events(server_id);
            CREATE INDEX IF NOT EXISTS idx_events_channel_ts ON events(server_id, channel_id, timestamp_ms);
            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp_ms);",
        )?;
        Ok(Self { conn })
    }

    /// Store an event. Deduplicates by event ID.
    pub fn store_event(&self, server_id: &str, event: &Event) -> anyhow::Result<bool> {
        let channel_id = Self::extract_channel_id(event);
        let event_data = bincode::serialize(event)?;
        let rows = self.conn.execute(
            "INSERT OR IGNORE INTO events (id, server_id, channel_id, author, timestamp_ms, event_data)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id,
                server_id,
                channel_id,
                event.author,
                event.timestamp_ms,
                event_data,
            ],
        )?;
        Ok(rows > 0)
    }

    /// Query events for a channel, paginated by timestamp (descending).
    /// Returns events older than `before_timestamp`, limited to `limit`.
    pub fn history(
        &self,
        server_id: &str,
        channel: &str,
        before_timestamp: Option<u64>,
        limit: u32,
    ) -> anyhow::Result<(Vec<Event>, bool)> {
        let before = before_timestamp.unwrap_or(u64::MAX);
        // Fetch limit + 1 to determine if there are more pages.
        let fetch_limit = limit as usize + 1;

        let mut stmt = self.conn.prepare(
            "SELECT event_data FROM events
             WHERE server_id = ?1 AND channel_id = ?2 AND timestamp_ms < ?3
             ORDER BY timestamp_ms DESC
             LIMIT ?4",
        )?;

        let events: Vec<Event> = stmt
            .query_map(params![server_id, channel, before as i64, fetch_limit as i64], |row| {
                let data: Vec<u8> = row.get(0)?;
                Ok(data)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|data| bincode::deserialize(&data).ok())
            .collect();

        let has_more = events.len() > limit as usize;
        let events: Vec<Event> = events.into_iter().take(limit as usize).collect();

        Ok((events, has_more))
    }

    /// Total number of stored events.
    pub fn count(&self) -> anyhow::Result<u64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        Ok(count as u64)
    }

    /// Total disk usage estimate in bytes.
    pub fn disk_usage_bytes(&self) -> anyhow::Result<u64> {
        let pages: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))?;
        let page_size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))?;
        Ok((pages * page_size) as u64)
    }

    /// Number of distinct servers tracked.
    pub fn server_count(&self) -> anyhow::Result<u32> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT server_id) FROM events",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u32)
    }

    /// Extract channel_id from an event, if applicable.
    fn extract_channel_id(event: &Event) -> String {
        use willow_state::EventKind;
        match &event.kind {
            EventKind::Message { channel_id, .. }
            | EventKind::EditMessage { .. }
            | EventKind::DeleteMessage { .. }
            | EventKind::Reaction { .. }
            | EventKind::PinMessage { channel_id, .. }
            | EventKind::UnpinMessage { channel_id, .. } => channel_id.clone(),
            EventKind::CreateChannel { channel_id, .. }
            | EventKind::DeleteChannel { channel_id, .. }
            | EventKind::RenameChannel { channel_id, .. }
            | EventKind::RotateChannelKey { channel_id, .. } => channel_id.clone(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_state::{EventKind, StateHash};

    fn make_message(id: &str, channel: &str, ts: u64) -> Event {
        Event {
            id: id.to_string(),
            parent_hash: StateHash::ZERO,
            author: "peer-1".to_string(),
            timestamp_ms: ts,
            kind: EventKind::Message {
                channel_id: channel.to_string(),
                body: format!("msg {id}"),
                reply_to: None,
            },
        }
    }

    #[test]
    fn store_and_count() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let event = make_message("e1", "general", 1000);
        assert!(store.store_event("srv-1", &event).unwrap());
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn deduplicates_by_event_id() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let event = make_message("e1", "general", 1000);
        assert!(store.store_event("srv-1", &event).unwrap());
        assert!(!store.store_event("srv-1", &event).unwrap()); // Duplicate
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn history_returns_newest_first() {
        let store = StorageEventStore::open(":memory:").unwrap();
        for i in 0..5 {
            let event = make_message(&format!("e{i}"), "general", (i + 1) * 1000);
            store.store_event("srv-1", &event).unwrap();
        }

        let (events, has_more) = store.history("srv-1", "general", None, 3).unwrap();
        assert_eq!(events.len(), 3);
        assert!(has_more);
        // Newest first.
        assert_eq!(events[0].timestamp_ms, 5000);
        assert_eq!(events[1].timestamp_ms, 4000);
        assert_eq!(events[2].timestamp_ms, 3000);
    }

    #[test]
    fn history_pagination_with_cursor() {
        let store = StorageEventStore::open(":memory:").unwrap();
        for i in 0..10 {
            let event = make_message(&format!("e{i}"), "general", (i + 1) * 1000);
            store.store_event("srv-1", &event).unwrap();
        }

        // First page: newest 3.
        let (page1, has_more1) = store.history("srv-1", "general", None, 3).unwrap();
        assert_eq!(page1.len(), 3);
        assert!(has_more1);
        assert_eq!(page1[0].timestamp_ms, 10000);

        // Second page: before oldest of page 1.
        let cursor = page1.last().unwrap().timestamp_ms;
        let (page2, has_more2) = store.history("srv-1", "general", Some(cursor), 3).unwrap();
        assert_eq!(page2.len(), 3);
        assert!(has_more2);
        assert_eq!(page2[0].timestamp_ms, 7000);

        // Continue pagination to the end.
        let cursor = page2.last().unwrap().timestamp_ms;
        let (page3, has_more3) = store.history("srv-1", "general", Some(cursor), 3).unwrap();
        assert_eq!(page3.len(), 3);
        assert!(has_more3); // Still 1 more
        let cursor = page3.last().unwrap().timestamp_ms;
        let (page4, has_more4) = store.history("srv-1", "general", Some(cursor), 3).unwrap();
        assert_eq!(page4.len(), 1);
        assert!(!has_more4);
    }

    #[test]
    fn history_filters_by_channel() {
        let store = StorageEventStore::open(":memory:").unwrap();
        store
            .store_event("srv-1", &make_message("e1", "general", 1000))
            .unwrap();
        store
            .store_event("srv-1", &make_message("e2", "random", 2000))
            .unwrap();
        store
            .store_event("srv-1", &make_message("e3", "general", 3000))
            .unwrap();

        let (events, _) = store.history("srv-1", "general", None, 10).unwrap();
        assert_eq!(events.len(), 2);
        for e in &events {
            match &e.kind {
                EventKind::Message { channel_id, .. } => {
                    assert_eq!(channel_id, "general");
                }
                _ => panic!("expected Message"),
            }
        }
    }

    #[test]
    fn history_filters_by_server() {
        let store = StorageEventStore::open(":memory:").unwrap();
        store
            .store_event("srv-1", &make_message("e1", "general", 1000))
            .unwrap();
        store
            .store_event("srv-2", &make_message("e2", "general", 2000))
            .unwrap();

        let (events, _) = store.history("srv-1", "general", None, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "e1");
    }

    #[test]
    fn server_count_tracks_distinct_servers() {
        let store = StorageEventStore::open(":memory:").unwrap();
        store
            .store_event("srv-1", &make_message("e1", "general", 1000))
            .unwrap();
        store
            .store_event("srv-2", &make_message("e2", "general", 2000))
            .unwrap();
        store
            .store_event("srv-1", &make_message("e3", "general", 3000))
            .unwrap();

        assert_eq!(store.server_count().unwrap(), 2);
    }

    #[test]
    fn disk_usage_returns_nonzero_after_insert() {
        let store = StorageEventStore::open(":memory:").unwrap();
        store
            .store_event("srv-1", &make_message("e1", "general", 1000))
            .unwrap();
        assert!(store.disk_usage_bytes().unwrap() > 0);
    }

    #[test]
    fn empty_history_returns_no_events() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let (events, has_more) = store.history("srv-1", "general", None, 10).unwrap();
        assert!(events.is_empty());
        assert!(!has_more);
    }
}
```

- [ ] **Step 3: Implement StorageRole**

Create `crates/storage/src/role.rs`:

```rust
//! Storage node role implementation.
//!
//! Persists events to SQLite, serves paginated history queries.

use willow_state::Event;
use willow_worker::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};

use crate::store::StorageEventStore;

/// The storage node's WorkerRole implementation.
pub struct StorageRole {
    store: StorageEventStore,
    /// Server ID is determined by the gossipsub topic in the full
    /// runtime. For now, default to a fixed value.
    default_server_id: String,
}

impl StorageRole {
    pub fn new(store: StorageEventStore) -> Self {
        Self {
            store,
            default_server_id: "default".to_string(),
        }
    }

    /// Set the default server ID (used when server can't be inferred).
    pub fn set_default_server(&mut self, id: String) {
        self.default_server_id = id;
    }
}

impl WorkerRole for StorageRole {
    fn role_info(&self) -> WorkerRoleInfo {
        let total = self.store.count().unwrap_or(0);
        let disk = self.store.disk_usage_bytes().unwrap_or(0);
        let servers = self.store.server_count().unwrap_or(0);
        WorkerRoleInfo::Storage {
            servers_tracked: servers,
            total_events_stored: total,
            disk_used_bytes: disk,
        }
    }

    fn on_event(&mut self, event: &Event) {
        let _ = self
            .store
            .store_event(&self.default_server_id, event);
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::History {
                server_id,
                channel,
                before_timestamp,
                limit,
            } => match self.store.history(&server_id, &channel, before_timestamp, limit) {
                Ok((events, has_more)) => WorkerResponse::HistoryPage { events, has_more },
                Err(e) => WorkerResponse::Denied {
                    reason: format!("query failed: {e}"),
                },
            },
            WorkerRequest::Sync { .. } => WorkerResponse::Denied {
                reason: "storage nodes do not serve sync requests".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_state::{EventKind, StateHash};

    fn make_message(id: &str, channel: &str, ts: u64) -> Event {
        Event {
            id: id.to_string(),
            parent_hash: StateHash::ZERO,
            author: "peer-1".to_string(),
            timestamp_ms: ts,
            kind: EventKind::Message {
                channel_id: channel.to_string(),
                body: format!("msg {id}"),
                reply_to: None,
            },
        }
    }

    #[test]
    fn storage_role_stores_and_serves_history() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        // Ingest events.
        for i in 0..5 {
            role.on_event(&make_message(
                &format!("e{i}"),
                "general",
                (i + 1) * 1000,
            ));
        }

        // Query history.
        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "general".to_string(),
            before_timestamp: None,
            limit: 3,
        });

        match resp {
            WorkerResponse::HistoryPage { events, has_more } => {
                assert_eq!(events.len(), 3);
                assert!(has_more);
            }
            _ => panic!("expected HistoryPage"),
        }
    }

    #[test]
    fn storage_role_denies_sync_requests() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        });

        match resp {
            WorkerResponse::Denied { reason } => {
                assert!(reason.contains("sync"));
            }
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn role_info_reflects_stored_data() {
        let store = StorageEventStore::open(":memory:").unwrap();
        let mut role = StorageRole::new(store);
        role.set_default_server("srv-1".to_string());

        role.on_event(&make_message("e1", "general", 1000));

        match role.role_info() {
            WorkerRoleInfo::Storage {
                total_events_stored,
                servers_tracked,
                ..
            } => {
                assert_eq!(total_events_stored, 1);
                assert_eq!(servers_tracked, 1);
            }
            _ => panic!("expected Storage"),
        }
    }
}
```

- [ ] **Step 4: Create main.rs**

Create `crates/storage/src/main.rs`:

```rust
//! Willow Storage Node — archival disk-backed history worker.

mod role;
mod store;

use clap::Parser;

#[derive(Parser)]
#[command(name = "willow-storage", about = "Willow storage worker node")]
struct Cli {
    /// Path to the Ed25519 identity keypair file.
    #[arg(long, default_value = "/etc/willow/storage.key")]
    identity_path: String,

    /// Relay multiaddr to connect through.
    #[arg(long)]
    relay: Option<String>,

    /// Path to SQLite database.
    #[arg(long, default_value = "/var/lib/willow/storage.db")]
    db_path: String,

    /// Active sync interval in seconds.
    #[arg(long, default_value = "60")]
    sync_interval: u64,

    /// Generate a new identity and exit.
    #[arg(long)]
    generate_identity: bool,

    /// Print the peer ID for the identity file and exit.
    #[arg(long)]
    print_peer_id: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    if cli.generate_identity {
        willow_worker::identity::generate_identity(&cli.identity_path)?;
        tracing::info!("identity generated at {}", cli.identity_path);
        return Ok(());
    }

    if cli.print_peer_id {
        return willow_worker::identity::print_peer_id(&cli.identity_path);
    }

    tracing::info!(db_path = %cli.db_path, sync_interval = cli.sync_interval, "starting storage node");

    // Full runtime integration will be wired in a later task.
    tracing::info!("storage node ready (runtime not yet wired)");
    Ok(())
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p willow-storage`
Expected: All 12 tests pass (9 store + 3 role).

- [ ] **Step 6: Build the binary**

Run: `cargo build -p willow-storage`
Expected: Compiles successfully.

- [ ] **Step 7: Commit**

```bash
git add crates/storage/ Cargo.toml
git commit -m "feat: add willow-storage binary with SQLite-backed history

Persists events to SQLite indefinitely, serves paginated history
queries by server/channel with cursor-based pagination."
```

---

## Task 7: Network Actor and Worker Runtime

**Files:**
- Create: `crates/worker/src/actors/network.rs`
- Create: `crates/worker/src/runtime.rs`
- Modify: `crates/worker/src/lib.rs`
- Modify: `crates/worker/src/actors/mod.rs`

This task wires the four actors together into the full worker runtime.

- [ ] **Step 1: Implement the network actor**

Create `crates/worker/src/actors/network.rs`:

```rust
//! Network actor — owns the libp2p swarm, bridges gossipsub to other actors.

use tokio::sync::mpsc;
use tracing::{debug, warn};

use willow_network::{NetworkEvent, NetworkNode};

use super::{NetworkOutMsg, StateMsg};
use crate::types::{WorkerWireMessage, WORKERS_TOPIC};

/// Run the network actor loop.
///
/// Receives gossipsub events from the swarm, dispatches to the state
/// actor. Receives outbound messages from other actors and publishes
/// to gossipsub.
pub async fn run(
    node: NetworkNode,
    mut events: mpsc::UnboundedReceiver<NetworkEvent>,
    state_tx: mpsc::Sender<StateMsg>,
    mut outbound_rx: mpsc::Receiver<NetworkOutMsg>,
    local_peer_id: String,
) {
    debug!("network actor started");

    // Subscribe to the workers topic.
    node.subscribe(WORKERS_TOPIC);
    // Subscribe to server ops topic for events and permissions.
    node.subscribe(willow_worker::types::SERVER_OPS_TOPIC);

    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                match event {
                    NetworkEvent::Message { topic, data, source } => {
                        handle_incoming_message(
                            &topic,
                            &data,
                            source.map(|p| p.to_string()),
                            &state_tx,
                            &local_peer_id,
                        )
                        .await;
                    }
                    NetworkEvent::PeerConnected(peer) => {
                        debug!(%peer, "peer connected");
                    }
                    NetworkEvent::PeerDisconnected(peer) => {
                        debug!(%peer, "peer disconnected");
                    }
                    _ => {}
                }
            }
            msg = outbound_rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    NetworkOutMsg::Publish { topic, data } => {
                        node.publish(&topic, data);
                    }
                    NetworkOutMsg::Subscribe(topic) => {
                        node.subscribe(&topic);
                    }
                }
            }
        }
    }

    debug!("network actor stopped");
}

async fn handle_incoming_message(
    topic: &str,
    data: &[u8],
    _source: Option<String>,
    state_tx: &mpsc::Sender<StateMsg>,
    local_peer_id: &str,
) {
    if topic == WORKERS_TOPIC {
        // Try to decode as WorkerWireMessage.
        if let Ok(msg) = bincode::deserialize::<WorkerWireMessage>(data) {
            match msg {
                WorkerWireMessage::Request {
                    target_peer,
                    payload,
                    request_id,
                } => {
                    // Only handle if targeted at us or broadcast (empty target).
                    if target_peer.is_empty() || target_peer == local_peer_id {
                        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                        let _ = state_tx
                            .send(StateMsg::Request {
                                req: payload,
                                reply: reply_tx,
                            })
                            .await;
                        // Response is sent back via the reply channel and
                        // published by whoever initiated the request flow.
                        // In the full runtime this would publish a Response
                        // message back. For now we log.
                        if let Ok(resp) = reply_rx.await {
                            debug!(?resp, %request_id, "request handled");
                        }
                    }
                }
                WorkerWireMessage::Response { target_peer, .. } => {
                    if target_peer == local_peer_id {
                        // Handle response to our own sync requests.
                        debug!("received response to our request");
                    }
                }
                WorkerWireMessage::Announcement(_) | WorkerWireMessage::Departure { .. } => {
                    // Workers don't need to track other workers' announcements.
                    // Clients handle this via worker_cache.
                }
            }
        }
    } else {
        // Server ops or channel topic — try to decode as Event.
        if let Ok((wire_msg, _sender)) = willow_worker::ops::unpack_wire(data) {
            match wire_msg {
                willow_worker::ops::ServerWireMessage::Event(event) => {
                    let _ = state_tx.send(StateMsg::Event(event)).await;
                }
                willow_worker::ops::ServerWireMessage::SyncBatch { events } => {
                    for event in events {
                        let _ = state_tx.send(StateMsg::Event(event)).await;
                    }
                }
                _ => {}
            }
        }
    }
}
```

Note: The `willow_worker::ops` module references don't exist yet. The actual wire message unpacking will use `willow_identity::unpack` and `willow_transport::unpack_envelope` like the existing relay. The exact integration depends on how the client `ops.rs` is structured. This will be refined during implementation — the key architecture (network actor dispatching to state actor via channels) is what matters.

- [ ] **Step 2: Implement the runtime orchestrator**

Create `crates/worker/src/runtime.rs`:

```rust
//! Worker runtime — orchestrates all four actors.

use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tracing::info;
use willow_identity::Identity;
use willow_network::{NetworkConfig, NetworkNode};

use crate::actors::{heartbeat, state, sync, NetworkOutMsg, StateMsg};
use crate::config::WorkerConfig;
use crate::WorkerRole;

/// Run a worker node with the given role and configuration.
///
/// This is the main entry point called by each binary's `main()`.
/// It starts all four actors and blocks until shutdown.
pub async fn run(role: Box<dyn WorkerRole>, config: WorkerConfig) -> anyhow::Result<()> {
    // Load identity.
    let identity = crate::identity::load_or_generate(&config.identity_path)?;
    let peer_id = identity.peer_id().to_string();
    info!(%peer_id, "worker identity loaded");

    // Build network config.
    let mut net_config = NetworkConfig::default();
    net_config = net_config.with_relay(&config.relay_addr)?;

    // Start the network node.
    let (node, events) = NetworkNode::start(identity, net_config).await?;
    info!("network node started");

    // Create channels.
    let (state_tx, state_rx) = mpsc::channel::<StateMsg>(256);
    let (network_tx, network_rx) = mpsc::channel::<NetworkOutMsg>(256);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn actors.
    let state_handle = tokio::spawn(state::run(role, state_rx));

    let network_handle = tokio::spawn(crate::actors::network::run(
        node,
        events,
        state_tx.clone(),
        network_rx,
        peer_id.clone(),
    ));

    let heartbeat_handle = tokio::spawn(heartbeat::run(
        peer_id.clone(),
        Duration::from_secs(10),
        state_tx.clone(),
        network_tx.clone(),
        shutdown_rx.clone(),
    ));

    let sync_handle = tokio::spawn(sync::run(
        peer_id,
        Duration::from_secs(config.sync_interval_secs),
        state_tx.clone(),
        network_tx,
        shutdown_rx,
    ));

    // Wait for shutdown signal (Ctrl+C).
    tokio::signal::ctrl_c().await?;
    info!("shutdown signal received");

    // Signal all actors to stop.
    let _ = shutdown_tx.send(true);
    let _ = state_tx.send(StateMsg::Shutdown).await;

    // Wait for actors to finish.
    let _ = tokio::join!(state_handle, network_handle, heartbeat_handle, sync_handle);

    info!("worker shut down cleanly");
    Ok(())
}
```

- [ ] **Step 3: Add SERVER_OPS_TOPIC constant and re-export**

Add to `crates/worker/src/types.rs`:

```rust
/// Gossipsub topic for server state operations (shared with clients).
pub const SERVER_OPS_TOPIC: &str = "_willow_server_ops";
```

- [ ] **Step 4: Update lib.rs with runtime and network modules**

```rust
//! Shared worker node library.

pub mod actors;
pub mod config;
pub mod identity;
pub mod runtime;
pub mod types;

pub use config::WorkerConfig;
pub use types::*;
```

- [ ] **Step 5: Update actors/mod.rs**

```rust
pub mod heartbeat;
pub mod network;
pub mod state;
pub mod sync;

// ... existing message types ...
```

- [ ] **Step 6: Run all worker tests**

Run: `cargo test -p willow-worker`
Expected: All existing tests still pass. Network actor and runtime are integration-level — tested via the full binary in Task 9.

- [ ] **Step 7: Commit**

```bash
git add crates/worker/
git commit -m "feat: add network actor and runtime orchestrator

Network actor bridges gossipsub to state actor via channels. Runtime
wires all four actors (network, state, heartbeat, sync) and handles
graceful shutdown on Ctrl+C."
```

---

## Task 8: Strip Relay to Pure Network Plumbing

**Files:**
- Modify: `crates/relay/src/lib.rs`
- Modify: `crates/relay/src/main.rs`
- Delete: `crates/relay/src/event_store.rs`
- Modify: `crates/relay/Cargo.toml`

- [ ] **Step 1: Read current relay files**

Read: `crates/relay/src/lib.rs`, `crates/relay/src/main.rs`, `crates/relay/src/event_store.rs`, `crates/relay/Cargo.toml`

Understand exactly what to remove vs keep. The relay currently:
- Stores events to SQLite (`event_store.rs`)
- Responds to `SyncRequest` with events from the store
- Caches `SyncBatch` events
- All of this needs to be removed. The relay should just pass gossipsub messages through.

- [ ] **Step 2: Remove event store from relay**

Delete `crates/relay/src/event_store.rs`.

Remove `rusqlite` from `crates/relay/Cargo.toml`.

Remove from `crates/relay/src/lib.rs`:
- The `event_store` field from `Relay` struct
- The `pub mod event_store;` declaration
- All `store_event()` calls in `handle_gossipsub_message()`
- The `SyncRequest` handling logic (events_for_topic_since_hash, events_since)
- The `SyncBatch` caching logic
- Any imports related to `RelayEventStore`

The `handle_gossipsub_message()` should become a no-op — just verify signatures and let gossipsub handle propagation. Or simplify to just logging.

Remove from `crates/relay/src/main.rs`:
- The `--data-dir` CLI argument
- Event store initialization (`RelayEventStore::open()`)
- Passing event store to `Relay::start()`

- [ ] **Step 3: Update Relay struct and constructor**

The `Relay` struct should become:

```rust
pub struct Relay {
    pub swarm: libp2p::Swarm<RelayBehaviour>,
    pub identity: willow_identity::Identity,
    pub peer_id: PeerId,
    pub display_name: String,
}
```

`Relay::start()` no longer takes a database path.

- [ ] **Step 4: Run relay tests**

Run: `cargo test -p willow-relay`
Expected: Tests that relied on event store functionality will need to be removed or adapted. Tests for basic relay startup and gossipsub forwarding should still pass.

- [ ] **Step 5: Run full test suite**

Run: `just test`
Expected: All tests pass. If any client tests relied on relay event storage, they need to be adapted to use replay nodes instead.

- [ ] **Step 6: Commit**

```bash
git add crates/relay/
git commit -m "refactor: strip relay to pure network plumbing

Remove SQLite event store, SyncRequest/SyncBatch handling, and all
state storage from the relay. It now only does TCP/WS bridging, NAT
traversal, and gossipsub message routing."
```

---

## Task 9: Client Worker Cache

**Files:**
- Create: `crates/client/src/worker_cache.rs`
- Modify: `crates/client/src/lib.rs`

The client needs to discover workers via heartbeats and route requests to them.

- [ ] **Step 1: Implement worker cache with tests**

Create `crates/client/src/worker_cache.rs`:

```rust
//! Client-side worker discovery cache.
//!
//! Populated from [`WorkerAnnouncement`] heartbeats received on the
//! `_willow_workers` gossipsub topic. Entries are evicted after a TTL
//! (default 30s) without a heartbeat.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use willow_worker::{WorkerAnnouncement, WorkerRoleInfo};

/// Information about a known worker.
#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub peer_id: String,
    pub role: WorkerRoleInfo,
    pub servers: Vec<String>,
    pub last_seen: Instant,
}

/// Cache of discovered workers with TTL-based eviction.
pub struct WorkerCache {
    workers: HashMap<String, WorkerInfo>, // keyed by peer_id
    ttl: Duration,
}

impl WorkerCache {
    /// Create a new cache with the given TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            workers: HashMap::new(),
            ttl,
        }
    }

    /// Update the cache from a heartbeat announcement.
    pub fn update(&mut self, announcement: &WorkerAnnouncement) {
        self.workers.insert(
            announcement.peer_id.clone(),
            WorkerInfo {
                peer_id: announcement.peer_id.clone(),
                role: announcement.role.clone(),
                servers: announcement.servers.clone(),
                last_seen: Instant::now(),
            },
        );
    }

    /// Remove a worker (e.g., on departure message).
    pub fn remove(&mut self, peer_id: &str) {
        self.workers.remove(peer_id);
    }

    /// Evict workers that haven't sent a heartbeat within the TTL.
    pub fn evict_stale(&mut self) {
        let cutoff = Instant::now() - self.ttl;
        self.workers.retain(|_, info| info.last_seen > cutoff);
    }

    /// Find replay workers for a given server.
    pub fn replay_workers_for_server(&self, server_id: &str) -> Vec<&WorkerInfo> {
        self.workers
            .values()
            .filter(|w| {
                matches!(w.role, WorkerRoleInfo::Replay { .. })
                    && w.servers.contains(&server_id.to_string())
            })
            .collect()
    }

    /// Find storage workers for a given server.
    pub fn storage_workers_for_server(&self, server_id: &str) -> Vec<&WorkerInfo> {
        self.workers
            .values()
            .filter(|w| {
                matches!(w.role, WorkerRoleInfo::Storage { .. })
                    && w.servers.contains(&server_id.to_string())
            })
            .collect()
    }

    /// Pick a worker for a role and server (simple round-robin placeholder).
    /// Returns None if no workers are available.
    pub fn pick_replay(&self, server_id: &str) -> Option<&WorkerInfo> {
        self.replay_workers_for_server(server_id).into_iter().next()
    }

    /// Pick a storage worker.
    pub fn pick_storage(&self, server_id: &str) -> Option<&WorkerInfo> {
        self.storage_workers_for_server(server_id)
            .into_iter()
            .next()
    }

    /// Total number of cached workers.
    pub fn len(&self) -> usize {
        self.workers.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_replay_announcement(peer_id: &str, servers: Vec<&str>) -> WorkerAnnouncement {
        WorkerAnnouncement {
            peer_id: peer_id.to_string(),
            role: WorkerRoleInfo::Replay {
                servers_loaded: servers.len() as u32,
                events_buffered: 100,
                max_events: 1000,
            },
            servers: servers.into_iter().map(String::from).collect(),
            timestamp: 1000,
        }
    }

    fn make_storage_announcement(peer_id: &str, servers: Vec<&str>) -> WorkerAnnouncement {
        WorkerAnnouncement {
            peer_id: peer_id.to_string(),
            role: WorkerRoleInfo::Storage {
                servers_tracked: servers.len() as u32,
                total_events_stored: 5000,
                disk_used_bytes: 1_000_000,
            },
            servers: servers.into_iter().map(String::from).collect(),
            timestamp: 1000,
        }
    }

    #[test]
    fn update_adds_worker() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement("w1", vec!["srv-1"]));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn update_replaces_existing() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement("w1", vec!["srv-1"]));
        cache.update(&make_replay_announcement("w1", vec!["srv-1", "srv-2"]));
        assert_eq!(cache.len(), 1);
        let workers = cache.replay_workers_for_server("srv-2");
        assert_eq!(workers.len(), 1);
    }

    #[test]
    fn remove_evicts_worker() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement("w1", vec!["srv-1"]));
        cache.remove("w1");
        assert!(cache.is_empty());
    }

    #[test]
    fn evict_stale_removes_expired() {
        let mut cache = WorkerCache::new(Duration::from_millis(1));
        cache.update(&make_replay_announcement("w1", vec!["srv-1"]));
        std::thread::sleep(Duration::from_millis(10));
        cache.evict_stale();
        assert!(cache.is_empty());
    }

    #[test]
    fn evict_stale_keeps_fresh() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement("w1", vec!["srv-1"]));
        cache.evict_stale();
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn replay_workers_for_server_filters_by_role_and_server() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement("r1", vec!["srv-1"]));
        cache.update(&make_replay_announcement("r2", vec!["srv-2"]));
        cache.update(&make_storage_announcement("s1", vec!["srv-1"]));

        let workers = cache.replay_workers_for_server("srv-1");
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].peer_id, "r1");
    }

    #[test]
    fn storage_workers_for_server_filters() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement("r1", vec!["srv-1"]));
        cache.update(&make_storage_announcement("s1", vec!["srv-1"]));
        cache.update(&make_storage_announcement("s2", vec!["srv-1"]));

        let workers = cache.storage_workers_for_server("srv-1");
        assert_eq!(workers.len(), 2);
    }

    #[test]
    fn pick_replay_returns_none_when_empty() {
        let cache = WorkerCache::new(Duration::from_secs(30));
        assert!(cache.pick_replay("srv-1").is_none());
    }

    #[test]
    fn pick_replay_returns_worker() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement("r1", vec!["srv-1"]));
        let w = cache.pick_replay("srv-1");
        assert!(w.is_some());
        assert_eq!(w.unwrap().peer_id, "r1");
    }

    #[test]
    fn multiple_replay_workers_for_same_server() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement("r1", vec!["srv-1"]));
        cache.update(&make_replay_announcement("r2", vec!["srv-1"]));

        let workers = cache.replay_workers_for_server("srv-1");
        assert_eq!(workers.len(), 2);
    }

    #[test]
    fn worker_serving_multiple_servers() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement("r1", vec!["srv-1", "srv-2", "srv-3"]));

        assert_eq!(cache.replay_workers_for_server("srv-1").len(), 1);
        assert_eq!(cache.replay_workers_for_server("srv-2").len(), 1);
        assert_eq!(cache.replay_workers_for_server("srv-3").len(), 1);
        assert_eq!(cache.replay_workers_for_server("srv-4").len(), 0);
    }
}
```

- [ ] **Step 2: Add worker_cache module to client**

Add `pub mod worker_cache;` to `crates/client/src/lib.rs`.

Add `willow-worker` as a dependency to `crates/client/Cargo.toml`:

```toml
willow-worker = { path = "../worker" }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p willow-client`
Expected: All existing client tests pass + 11 new worker cache tests.

- [ ] **Step 4: Commit**

```bash
git add crates/client/
git commit -m "feat: add client-side worker discovery cache

TTL-based cache populated from WorkerAnnouncement heartbeats.
Filters by role and server for routing sync/history requests."
```

---

## Task 10: Server Creation Worker Authorization

**Files:**
- Modify: `crates/client/src/lib.rs` (add `authorize_workers` method)
- Modify: `crates/web/src/app.rs` (wire up worker step)
- Modify: `crates/web/src/components/settings.rs` (add worker node step to creation flow)
- Modify: `crates/web/src/state.rs` (add platform workers constant)

- [ ] **Step 1: Add PLATFORM_WORKERS constant**

Add to `crates/web/src/state.rs` (or a new `crates/web/src/constants.rs`):

```rust
/// Known platform worker peer IDs. Hardcoded for the initial version.
/// These are populated after first deployment by running
/// `willow-replay --print-peer-id` and `willow-storage --print-peer-id`.
pub const PLATFORM_WORKERS: &[(&str, &str)] = &[
    // ("peer_id", "role_name")
    // Populated after deployment — see docs/superpowers/specs/2026-03-27-worker-nodes-design.md
];
```

- [ ] **Step 2: Add authorize_workers method to client**

Add to `crates/client/src/lib.rs`:

```rust
/// Grant SyncProvider permission to the given worker peer IDs.
/// Called during server creation for each checked worker.
pub fn authorize_workers(&self, worker_peer_ids: &[String]) {
    for peer_id in worker_peer_ids {
        let _ = self.grant_permission(peer_id, willow_state::Permission::SyncProvider);
    }
}
```

- [ ] **Step 3: Add worker selection to server creation UX**

The server creation flow in `crates/web/src/components/settings.rs` currently has a server name input and a create button. Add a worker nodes checklist step between them. All workers are checked by default.

This is a UI component — the exact implementation follows the existing Leptos patterns in the settings component. The key behavior:
- Show grouped checklist (Replay Nodes, Storage Nodes)
- All checked by default
- "Select All / Deselect All" per group
- On submit, call `authorize_workers()` with checked peer IDs

- [ ] **Step 4: Add stub Worker Nodes section to server settings**

In the server settings panel (Server tab), add a "Worker Nodes" section that shows:
- List of authorized workers with their role
- Placeholder for revoke/authorize buttons (non-functional stubs)

- [ ] **Step 5: Run tests**

Run: `cargo test -p willow-client && cargo test -p willow-web`
Expected: All tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/client/ crates/web/
git commit -m "feat: add worker authorization to server creation flow

Workers are granted SyncProvider during server creation. New Worker
Nodes checklist step with all workers checked by default. Stub
Worker Nodes section in server settings."
```

---

## Task 11: Docker Deployment

**Files:**
- Create: `docker/relay.Dockerfile`
- Create: `docker/replay.Dockerfile`
- Create: `docker/storage.Dockerfile`
- Create: `docker/web.Dockerfile`
- Create: `docker-compose.yml`
- Modify: `justfile`

- [ ] **Step 1: Create relay Dockerfile**

Create `docker/relay.Dockerfile`:

```dockerfile
FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-relay

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/willow-relay /usr/local/bin/willow-relay

EXPOSE 9090 9091
ENTRYPOINT ["willow-relay"]
CMD ["--tcp-port", "9090", "--ws-port", "9091", "--identity", "/etc/willow/relay.key"]
```

- [ ] **Step 2: Create replay Dockerfile**

Create `docker/replay.Dockerfile`:

```dockerfile
FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-replay

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/willow-replay /usr/local/bin/willow-replay

ENTRYPOINT ["willow-replay"]
CMD ["--identity-path", "/etc/willow/replay.key"]
```

- [ ] **Step 3: Create storage Dockerfile**

Create `docker/storage.Dockerfile`:

```dockerfile
FROM rust:latest AS builder
WORKDIR /build
COPY . .
RUN cargo build --release -p willow-storage

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/willow-storage /usr/local/bin/willow-storage

ENTRYPOINT ["willow-storage"]
CMD ["--identity-path", "/etc/willow/storage.key", "--db-path", "/var/lib/willow/storage.db"]
```

- [ ] **Step 4: Create web Dockerfile**

Create `docker/web.Dockerfile`:

```dockerfile
FROM rust:latest AS builder
RUN rustup target add wasm32-unknown-unknown
RUN cargo install trunk
WORKDIR /build
COPY . .
RUN cd crates/web && trunk build --release

FROM nginx:alpine
COPY --from=builder /build/crates/web/dist/ /usr/share/nginx/html/
EXPOSE 80
```

- [ ] **Step 5: Create docker-compose.yml**

Create `docker-compose.yml` at repo root matching the spec's service definitions (relay, replay-1, replay-2, storage-1, storage-2, web) with volumes and environment variables.

- [ ] **Step 6: Add just commands**

Add to `justfile`:

```makefile
# Docker commands
docker-build:
    docker compose build

docker-up:
    docker compose up -d

docker-down:
    docker compose down

docker-logs:
    docker compose logs -f

docker-ids:
    docker compose exec replay-1 willow-replay --print-peer-id 2>/dev/null || echo "replay-1: not running"
    docker compose exec replay-2 willow-replay --print-peer-id 2>/dev/null || echo "replay-2: not running"
    docker compose exec storage-1 willow-storage --print-peer-id 2>/dev/null || echo "storage-1: not running"
    docker compose exec storage-2 willow-storage --print-peer-id 2>/dev/null || echo "storage-2: not running"
```

- [ ] **Step 7: Verify Dockerfiles build**

Run: `docker build -f docker/relay.Dockerfile .`
Expected: Builds successfully.

- [ ] **Step 8: Commit**

```bash
git add docker/ docker-compose.yml justfile
git commit -m "feat: add Docker deployment for full worker stack

Dockerfiles for relay, replay, storage, and web. Docker Compose
defines the full stack with persistent identity volumes. Just
commands for build/up/down/logs/ids."
```

---

## Task 12: Integration Tests

**Files:**
- Create: `crates/worker/tests/integration.rs`
- Modify: `justfile` (add test commands)

End-to-end tests that verify the full worker system works together.

- [ ] **Step 1: Write replay role integration test**

Create `crates/worker/tests/integration.rs`:

```rust
//! Integration tests for the worker node system.

use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use willow_state::{Event, EventKind, ServerState, StateHash};
use willow_worker::actors::state;
use willow_worker::actors::StateMsg;
use willow_worker::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};

/// Full replay role that tracks a single server.
struct TestReplayRole {
    state: ServerState,
    events: Vec<Event>,
    max_events: usize,
}

impl TestReplayRole {
    fn new(server_id: &str, owner: &str, max_events: usize) -> Self {
        Self {
            state: ServerState::new(server_id, server_id, owner.to_string()),
            events: Vec::new(),
            max_events,
        }
    }
}

impl WorkerRole for TestReplayRole {
    fn role_info(&self) -> WorkerRoleInfo {
        WorkerRoleInfo::Replay {
            servers_loaded: 1,
            events_buffered: self.events.len() as u32,
            max_events: self.max_events as u32,
        }
    }

    fn on_event(&mut self, event: &Event) {
        willow_state::apply_lenient(&mut self.state, event);
        self.events.push(event.clone());
        while self.events.len() > self.max_events {
            self.events.remove(0);
        }
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::Sync { state_hash, .. } => {
                if state_hash == StateHash::ZERO {
                    WorkerResponse::SyncBatch {
                        events: self.events.clone(),
                    }
                } else {
                    WorkerResponse::Snapshot {
                        state: self.state.clone(),
                    }
                }
            }
            WorkerRequest::History { .. } => WorkerResponse::Denied {
                reason: "not a storage node".to_string(),
            },
        }
    }
}

fn make_message(id: &str, ts: u64) -> Event {
    Event {
        id: id.to_string(),
        parent_hash: StateHash::ZERO,
        author: "peer-1".to_string(),
        timestamp_ms: ts,
        kind: EventKind::Message {
            channel_id: "general".to_string(),
            body: format!("message {id}"),
            reply_to: None,
        },
    }
}

#[tokio::test]
async fn state_actor_with_replay_role_full_flow() {
    let (tx, rx) = mpsc::channel(64);
    let role = Box::new(TestReplayRole::new("srv-1", "owner", 100));

    let handle = tokio::spawn(state::run(role, rx));

    // 1. Ingest 5 events.
    for i in 0..5 {
        tx.send(StateMsg::Event(make_message(&format!("e{i}"), (i + 1) * 1000)))
            .await
            .unwrap();
    }

    // 2. Verify role info shows 5 buffered events.
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(StateMsg::GetRoleInfo { reply: reply_tx })
        .await
        .unwrap();
    let info = reply_rx.await.unwrap();
    match info {
        WorkerRoleInfo::Replay {
            events_buffered, ..
        } => assert_eq!(events_buffered, 5),
        _ => panic!("expected Replay"),
    }

    // 3. Sync request with ZERO hash — should return all events.
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(StateMsg::Request {
        req: WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        },
        reply: reply_tx,
    })
    .await
    .unwrap();
    match reply_rx.await.unwrap() {
        WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 5),
        _ => panic!("expected SyncBatch"),
    }

    // 4. History request — should be denied.
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(StateMsg::Request {
        req: WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "general".to_string(),
            before_timestamp: None,
            limit: 10,
        },
        reply: reply_tx,
    })
    .await
    .unwrap();
    match reply_rx.await.unwrap() {
        WorkerResponse::Denied { .. } => {}
        _ => panic!("expected Denied"),
    }

    tx.send(StateMsg::Shutdown).await.unwrap();
    handle.await.unwrap();
}

#[tokio::test]
async fn heartbeat_and_state_actor_interaction() {
    use willow_worker::actors::{heartbeat, NetworkOutMsg};

    let (state_tx, state_rx) = mpsc::channel(64);
    let (network_tx, mut network_rx) = mpsc::channel(64);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let role = Box::new(TestReplayRole::new("srv-1", "owner", 100));
    let state_handle = tokio::spawn(state::run(role, state_rx));

    let hb_handle = tokio::spawn(heartbeat::run(
        "test-worker".to_string(),
        Duration::from_millis(50),
        state_tx.clone(),
        network_tx,
        shutdown_rx,
    ));

    // Wait for a heartbeat.
    let msg = tokio::time::timeout(Duration::from_secs(1), network_rx.recv())
        .await
        .unwrap()
        .unwrap();

    match msg {
        NetworkOutMsg::Publish { data, .. } => {
            let decoded: willow_worker::WorkerWireMessage =
                bincode::deserialize(&data).unwrap();
            match decoded {
                willow_worker::WorkerWireMessage::Announcement(a) => {
                    assert_eq!(a.peer_id, "test-worker");
                    match a.role {
                        WorkerRoleInfo::Replay {
                            events_buffered, ..
                        } => assert_eq!(events_buffered, 0),
                        _ => panic!("expected Replay"),
                    }
                }
                _ => panic!("expected Announcement"),
            }
        }
        _ => panic!("expected Publish"),
    }

    shutdown_tx.send(true).unwrap();
    let _ = state_tx.send(StateMsg::Shutdown).await;
    let _ = tokio::join!(state_handle, hb_handle);
}

#[tokio::test]
async fn concurrent_requests_all_resolve() {
    let (tx, rx) = mpsc::channel(256);
    let role = Box::new(TestReplayRole::new("srv-1", "owner", 100));

    let handle = tokio::spawn(state::run(role, rx));

    // Fire 50 concurrent requests.
    let mut reply_rxs = vec![];
    for i in 0..50 {
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(StateMsg::Request {
            req: WorkerRequest::Sync {
                server_id: "srv-1".to_string(),
                state_hash: StateHash::ZERO,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();
        reply_rxs.push(reply_rx);
    }

    // All 50 should resolve.
    for rx in reply_rxs {
        let resp = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(resp, WorkerResponse::SyncBatch { .. }));
    }

    tx.send(StateMsg::Shutdown).await.unwrap();
    handle.await.unwrap();
}
```

- [ ] **Step 2: Add just commands for worker tests**

Add to `justfile`:

```makefile
# Worker node tests
test-worker:
    cargo test -p willow-worker

test-replay:
    cargo test -p willow-replay

test-storage:
    cargo test -p willow-storage

test-workers:
    cargo test -p willow-worker -p willow-replay -p willow-storage
```

- [ ] **Step 3: Run all tests**

Run: `just test-workers`
Expected: All tests pass across all 3 crates.

- [ ] **Step 4: Run full project check**

Run: `just check`
Expected: fmt + clippy + test + WASM check all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/worker/tests/ justfile
git commit -m "test: add integration tests for worker actor system

Tests full state actor flow with replay role, heartbeat-state
interaction, and 50 concurrent request resolution."
```

---

## Summary

| Task | What | Tests |
|------|------|-------|
| 1 | Wire protocol types | 13 serialization round-trips |
| 2 | Config and identity CLI | 5 (config + identity gen/load) |
| 3 | State actor | 4 (events, requests, concurrent, shutdown) |
| 4 | Heartbeat + sync actors | 4 (announcements, sync broadcasts, shutdown) |
| 5 | Replay node binary | 8 (role info, buffering, eviction, sync, snapshot) |
| 6 | Storage node binary | 12 (SQLite store + role + pagination) |
| 7 | Network actor + runtime | Structural (tested via integration) |
| 8 | Relay strip-down | Existing relay tests adapted |
| 9 | Client worker cache | 11 (TTL, eviction, filtering, picking) |
| 10 | Server creation auth | Existing + manual verification |
| 11 | Docker deployment | Build verification |
| 12 | Integration tests | 3 (full flow, heartbeat interaction, concurrency) |

**Total new tests: ~60+**
