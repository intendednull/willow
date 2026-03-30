//! Actor-based concurrency system for worker nodes.
//!
//! Four actors communicate via tokio channels:
//! - Network actor: streams from gossip topic events
//! - State actor: owns WorkerRole + mutable state
//! - Heartbeat actor: periodic announcements via TopicHandle
//! - Sync actor: periodic state sync via TopicHandle

pub mod heartbeat;
pub mod network;
pub mod state;
pub mod sync;

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
