//! Wire protocol types for the worker node system.
//!
//! All types are serializable for gossipsub transport on the
//! `_willow_workers` topic.

use serde::{Deserialize, Serialize};
use willow_state::{Event, ServerState, StateHash};

/// Gossipsub topic for worker discovery and request/response.
pub const WORKERS_TOPIC: &str = "_willow_workers";

/// Gossipsub topic for server state operations (shared with clients).
pub const SERVER_OPS_TOPIC: &str = "_willow_server_ops";

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

#[cfg(test)]
mod tests {
    use super::*;

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
