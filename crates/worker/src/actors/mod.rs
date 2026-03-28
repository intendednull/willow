//! Actor-based concurrency system for worker nodes.
//!
//! Four actors communicate via tokio channels:
//! - Network actor: owns libp2p swarm
//! - State actor: owns WorkerRole + mutable state
//! - Heartbeat actor: periodic announcements
//! - Sync actor: periodic state sync

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

/// Messages sent to the network actor for outbound publishing.
pub enum NetworkOutMsg {
    /// Publish raw bytes on a gossipsub topic.
    Publish { topic: String, data: Vec<u8> },

    /// Subscribe to a gossipsub topic.
    Subscribe(String),
}
