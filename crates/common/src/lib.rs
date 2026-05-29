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

/// OOM guard on the event count materialized for a single sync / history
/// query — **not** the authoritative wire bound on a sync batch.
///
/// Since the heads-based delta protocol landed (plan PR 4 /
/// `docs/specs/2026-04-24-negentropy-sync.md` § Wire protocol), the
/// authoritative per-envelope bound on a `SyncBatchV2` /
/// `WorkerResponse::SyncBatch` is the gossip layer's 64 KiB ceiling, applied
/// via [`SYNC_ENVELOPE_BUDGET`] + [`pack_sync_batches`]: responders pack the
/// delta and serve only the first budget-fitting batch, setting `more` so the
/// requester drains the rest. A 64 KiB envelope cannot hold 10,000 non-trivial
/// events, so this count cap is normally never the binding limit.
///
/// It remains useful as a secondary defence shared by:
/// - `willow-storage` — `sync_since` / `history` bound the SQL `LIMIT` so the
///   SQLite-backed store cannot materialize an unbounded row set before
///   byte-budgeting.
/// - `willow-client` — rejects pathological inbound `SyncBatch` /
///   `SyncBatchV2` wire messages (e.g. 10,000+ tiny events crammed into one
///   envelope by a malicious/buggy peer) so a hostile peer cannot OOM us.
///
/// Both sides MUST agree on this value, so the canonical definition lives here
/// in `willow-common` (already a dep of both crates) to keep them aligned at
/// compile time.
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
