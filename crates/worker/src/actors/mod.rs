//! Actor-based concurrency system for worker nodes.
//!
//! Four actors communicate via `willow-actor` typed messages:
//! - Network actor: streams from gossip topic events
//! - State actor: owns WorkerRole + mutable state
//! - Heartbeat actor: periodic announcements via TopicHandle
//! - Sync actor: periodic state sync via TopicHandle

pub mod heartbeat;
pub mod network;
pub mod state;
pub mod sync;

use willow_actor::Message;
use willow_state::{Event, HeadsSummary};

use crate::types::{WorkerRequest, WorkerResponse, WorkerRoleInfo};

/// A new event arrived from gossipsub.
pub struct EventMsg(pub Event);
impl Message for EventMsg {
    type Result = ();
}

/// A client request that needs a response.
pub struct WorkerRequestMsg(pub WorkerRequest);
impl Message for WorkerRequestMsg {
    type Result = WorkerResponse;
}

/// Heartbeat actor asking for current role info.
pub struct GetRoleInfoMsg;
impl Message for GetRoleInfoMsg {
    type Result = WorkerRoleInfo;
}

/// Sync actor asking for current heads summaries per server.
pub struct GetHeadsSummariesMsg;
impl Message for GetHeadsSummariesMsg {
    type Result = Vec<(String, HeadsSummary)>;
}

/// A server was discovered — add it to the set of tracked servers.
pub struct ServerDiscoveredMsg {
    pub server_id: String,
}
impl Message for ServerDiscoveredMsg {
    type Result = ();
}
