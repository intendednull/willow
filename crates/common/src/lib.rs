//! Shared types for the Willow P2P system.
//!
//! Contains wire protocol types used by both `willow-client` and
//! `willow-worker`, including `WireMessage` (the gossipsub wire format)
//! and worker node types. WASM-compatible.

pub mod wire;
pub mod worker_types;

pub use wire::*;
pub use worker_types::*;

/// Maximum number of events allowed in a single sync batch.
///
/// Single source of truth shared by:
/// - `willow-storage` — caps batches produced by `sync_since` / `history`
///   so the SQLite-backed store cannot OOM a peer.
/// - `willow-client` — rejects oversized inbound `SyncBatch` wire messages
///   so a hostile peer cannot OOM us.
///
/// Both sides MUST agree on this value: if production exceeds validation,
/// honest peers reject honest batches; if validation exceeds production,
/// the validation cap is dead code. Keeping the constant here in
/// `willow-common` (already a dep of both crates) guarantees they stay
/// aligned at compile time.
pub const SYNC_BATCH_LIMIT: usize = 10_000;
