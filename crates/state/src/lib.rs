//! # Willow State
//!
//! Pure, deterministic event-sourced state machine for the Willow P2P chat
//! network. All state is derived from a per-author Merkle-DAG of signed
//! events via the [`materialize`] function. This crate has zero I/O, zero
//! networking — just DAG operations and deterministic state projection.

#[cfg(test)]
mod tests;

pub mod dag;
pub mod event;
pub mod hash;
pub mod managed;
pub mod materialize;
pub mod server;
pub mod sync;
pub mod types;

// Re-exports for convenience.
pub use dag::{EventDag, InsertError};
pub use event::{Event, EventKind, Permission, ProposedAction, VoteThreshold};
pub use hash::EventHash;
pub use managed::ManagedDag;
pub use materialize::{apply_incremental, check_permission, materialize, ApplyResult};
pub use server::{PendingProposal, ServerState};
pub use sync::{
    AuthorHead, AuthorRequest, ChainStatus, HeadsSummary, PendingBuffer, Snapshot, SyncMessage,
    DEFAULT_PENDING_MAX_AGE_MS, DEFAULT_PENDING_MAX_ENTRIES,
};
pub use types::{
    Channel, ChannelKind, ChatMessage, CrestPattern, Member, MuteState, PinnedFragment, PinnedKind,
    Profile, ProfileDelta, Role,
};
