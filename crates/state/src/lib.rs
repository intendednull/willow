//! # Willow State
//!
//! Pure, deterministic event-sourced state machine for the Willow P2P chat
//! network. All state is derived from a per-author Merkle-DAG of signed
//! events via the [`materialize`] function. This crate has zero I/O, zero
//! networking — just DAG operations and deterministic state projection.

#[cfg(test)]
#[path = "tests/dag.rs"]
mod tests_dag;

#[cfg(test)]
#[path = "tests/materialize.rs"]
mod tests_materialize;

#[cfg(test)]
#[path = "tests/permissions.rs"]
mod tests_permissions;

#[cfg(test)]
#[path = "tests/stress.rs"]
mod tests_stress;

#[cfg(test)]
#[path = "tests/sync.rs"]
mod tests_sync;

#[cfg(test)]
#[path = "tests/voting.rs"]
mod tests_voting;

pub mod dag;
pub mod ephemeral;
pub mod event;
pub mod hash;
pub mod managed;
pub mod materialize;
pub mod server;
pub mod sync;
pub mod types;

// Re-exports for convenience.
pub use dag::{EventDag, InsertError};
pub use ephemeral::{
    derive_ephemeral_state, EphemeralConfig, EphemeralKind, EphemeralState,
    DEFAULT_CHANNEL_THRESHOLD_MS, DEFAULT_THREAD_THRESHOLD_MS, DEFAULT_WHISPER_THRESHOLD_MS,
    IDLE_THRESHOLD_MAX_MS, IDLE_THRESHOLD_MIN_MS,
};
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
    Channel, ChannelKind, ChatMessage, CrestPattern, FileAttachment, Member, MuteState,
    PinMetadata, PinnedFragment, PinnedKind, Profile, ProfileDelta, Role,
};
