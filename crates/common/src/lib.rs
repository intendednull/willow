//! Shared types for the Willow P2P system.
//!
//! Contains wire protocol types used by both `willow-client` and
//! `willow-worker`, including `WireMessage` (the gossipsub wire format)
//! and worker node types. WASM-compatible.

pub mod relay_info;
pub mod wire;
pub mod worker_types;

pub use relay_info::{
    canonical_json, capability_etag, sign_capability_doc, verify_capability_doc, CapabilityError,
    Limitation, Retention, WillowRelayInfo,
};
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

/// Maximum number of `(author, head)` entries accepted from a peer-supplied
/// [`willow_state::HeadsSummary`] in a single sync / history call.
///
/// Single source of truth shared by:
/// - `willow-storage` — `sync_since` / `history` build SQL by concatenating
///   one `(author = ? AND seq <op> ?)` fragment per entry; an oversize
///   summary would either exceed rusqlite's bind-parameter limit
///   (default 32766) or waste CPU compiling a giant prepared statement.
/// - `willow-replay` — `handle_request(Sync)` iterates per-author into a
///   `BTreeMap` then walks the in-memory DAG; an oversize summary forces
///   O(N) work and is the same DoS shape as the storage path.
///
/// 256 is well above any plausible honest server's distinct-author count
/// while keeping bind-parameter and BTreeMap-construction costs bounded.
/// Both production sites MUST agree on this value, so the canonical
/// definition lives here alongside [`SYNC_BATCH_LIMIT`].
pub const MAX_AUTHORS_PER_SYNC: usize = 256;
