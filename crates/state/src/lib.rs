//! # Willow State
//!
//! Pure, deterministic event-sourced state machine for the Willow P2P chat
//! network. All state is derived from a per-author Merkle-DAG of signed
//! events via the [`materialize`] function. This crate has zero I/O, zero
//! networking — just DAG operations and deterministic state projection.

#[cfg(test)]
mod tests;

pub mod compat;
pub mod dag;
pub mod event;
pub mod hash;
pub mod materialize;
pub mod server;
pub mod sync;
pub mod types;

// Re-exports for convenience.
pub use dag::{EventDag, InsertError};
pub use event::{Event, EventKind, Permission, ProposedAction, VoteThreshold};
pub use hash::EventHash;
pub use materialize::{apply_incremental, materialize, ApplyResult};
pub use server::{PendingProposal, ServerState};
pub use sync::{
    AuthorHead, AuthorRequest, ChainStatus, HeadsSummary, PendingBuffer, Snapshot, SyncMessage,
};
pub use types::{Channel, ChatMessage, Member, Profile, Role};

// Backward-compatible re-exports for downstream crates during migration.
// Remove these as each downstream crate is updated to the new API.
// Tracked by: https://github.com/intendednull/willow/issues/24
pub use compat::{apply, apply_lenient, EventStore, InMemoryStore, StateHash};
