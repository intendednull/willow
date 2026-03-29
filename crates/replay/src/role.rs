//! Replay node role implementation.
//!
//! Maintains in-memory [`ServerState`] per server with a bounded event
//! buffer. Responds to sync requests with event diffs or full state
//! snapshots for far-behind peers.

use std::collections::{HashMap, VecDeque};

use willow_state::{Event, ServerState, StateHash};
use willow_worker::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};

/// Per-server state held by the replay node.
struct ServerData {
    /// Current computed state.
    state: ServerState,
    /// Bounded event buffer (oldest at front).
    events: VecDeque<Event>,
    /// Max events to retain.
    max_events: usize,
}

/// Configuration for the replay role.
pub struct ReplayConfig {
    /// Max events per server before eviction.
    pub max_events_per_server: usize,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            max_events_per_server: 1000,
        }
    }
}

/// The replay node's WorkerRole implementation.
pub struct ReplayRole {
    servers: HashMap<String, ServerData>,
    config: ReplayConfig,
}

impl ReplayRole {
    /// Create a new replay role with the given configuration.
    pub fn new(config: ReplayConfig) -> Self {
        Self {
            servers: HashMap::new(),
            config,
        }
    }

    /// Ingest an event for a specific server.
    pub fn ingest_event(&mut self, server_id: &str, event: &Event) {
        let data = self
            .servers
            .entry(server_id.to_string())
            .or_insert_with(|| ServerData {
                state: ServerState::new(server_id, server_id, event.author.clone()),
                events: VecDeque::new(),
                max_events: self.config.max_events_per_server,
            });

        willow_state::apply_lenient(&mut data.state, event);
        data.events.push_back(event.clone());

        while data.events.len() > data.max_events {
            data.events.pop_front();
        }
    }

    /// Find events after the given state hash in a server's buffer.
    fn events_since_hash(&self, server_id: &str, hash: &StateHash) -> Option<Vec<Event>> {
        let data = self.servers.get(server_id)?;

        // If hash is ZERO, send all buffered events.
        if *hash == StateHash::ZERO {
            return Some(data.events.iter().cloned().collect());
        }

        // Find first event whose parent_hash matches.
        for (i, event) in data.events.iter().enumerate() {
            if event.parent_hash == *hash {
                return Some(data.events.iter().skip(i).cloned().collect());
            }
        }

        // Hash not found in buffer — caller is too far behind.
        None
    }

    /// Get the state hashes for all tracked servers (used by sync actor).
    pub fn state_hashes(&self) -> Vec<(String, StateHash)> {
        self.servers
            .iter()
            .map(|(id, data)| (id.clone(), data.state.hash()))
            .collect()
    }
}

impl WorkerRole for ReplayRole {
    fn role_info(&self) -> WorkerRoleInfo {
        let total_events: u32 = self.servers.values().map(|s| s.events.len() as u32).sum();
        WorkerRoleInfo::Replay {
            servers_loaded: self.servers.len() as u32,
            events_buffered: total_events,
            max_events: self.config.max_events_per_server as u32,
        }
    }

    fn on_event(&mut self, event: &Event) {
        // In the full runtime, server_id comes from the gossipsub topic.
        // For now, use a default server for events that arrive without
        // topic context.
        self.ingest_event("default", event);
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::Sync {
                server_id,
                state_hash,
            } => match self.events_since_hash(&server_id, &state_hash) {
                Some(events) => WorkerResponse::SyncBatch { events },
                None => {
                    // Client is too far behind — send full snapshot.
                    match self.servers.get(&server_id) {
                        Some(data) => WorkerResponse::Snapshot {
                            state: Box::new(data.state.clone()),
                        },
                        None => WorkerResponse::Denied {
                            reason: format!("unknown server: {server_id}"),
                        },
                    }
                }
            },
            WorkerRequest::History { .. } => WorkerResponse::Denied {
                reason: "replay nodes do not serve history".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_state::EventKind;

    fn make_message(id: &str, ts: u64) -> Event {
        Event {
            id: id.to_string(),
            parent_hash: StateHash::ZERO,
            author: "peer-1".to_string(),
            timestamp_ms: ts,
            kind: EventKind::Message {
                channel_id: "general".to_string(),
                body: format!("message {id}"),
                reply_to: None,
            },
        }
    }

    #[test]
    fn role_info_starts_empty() {
        let role = ReplayRole::new(ReplayConfig::default());
        match role.role_info() {
            WorkerRoleInfo::Replay {
                servers_loaded,
                events_buffered,
                max_events,
            } => {
                assert_eq!(servers_loaded, 0);
                assert_eq!(events_buffered, 0);
                assert_eq!(max_events, 1000);
            }
            _ => panic!("expected Replay"),
        }
    }

    #[test]
    fn ingest_event_applies_and_buffers() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 100,
        });

        role.ingest_event("srv-1", &make_message("evt-1", 1000));

        match role.role_info() {
            WorkerRoleInfo::Replay {
                servers_loaded,
                events_buffered,
                ..
            } => {
                assert_eq!(servers_loaded, 1);
                assert_eq!(events_buffered, 1);
            }
            _ => panic!("expected Replay"),
        }
    }

    #[test]
    fn bounded_buffer_evicts_oldest() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 3,
        });

        for i in 0..5 {
            role.ingest_event("srv-1", &make_message(&format!("evt-{i}"), (i + 1) * 1000));
        }

        let data = &role.servers["srv-1"];
        assert_eq!(data.events.len(), 3);
        assert_eq!(data.events[0].id, "evt-2");
        assert_eq!(data.events[1].id, "evt-3");
        assert_eq!(data.events[2].id, "evt-4");
    }

    #[test]
    fn sync_request_returns_events_since_zero() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 100,
        });

        for i in 0..3 {
            role.ingest_event("srv-1", &make_message(&format!("evt-{i}"), (i + 1) * 1000));
        }

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: StateHash::ZERO,
        });

        match resp {
            WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 3),
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn sync_request_unknown_server_denied() {
        let mut role = ReplayRole::new(ReplayConfig::default());

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "nonexistent".to_string(),
            state_hash: StateHash::ZERO,
        });

        match resp {
            WorkerResponse::Denied { reason } => assert!(reason.contains("unknown server")),
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn sync_request_far_behind_returns_snapshot() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 2,
        });

        for i in 0..5 {
            role.ingest_event("srv-1", &make_message(&format!("evt-{i}"), (i + 1) * 1000));
        }

        let fake_old_hash = StateHash::from_bytes(b"old state");
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            state_hash: fake_old_hash,
        });

        match resp {
            WorkerResponse::Snapshot { state } => {
                assert_eq!(state.server_id, "srv-1");
            }
            _ => panic!("expected Snapshot"),
        }
    }

    #[test]
    fn history_request_denied_by_replay_node() {
        let mut role = ReplayRole::new(ReplayConfig::default());

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: "general".to_string(),
            before_timestamp: None,
            limit: 50,
        });

        match resp {
            WorkerResponse::Denied { reason } => assert!(reason.contains("history")),
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn role_info_reflects_buffered_events() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 100,
        });

        for i in 0..5 {
            role.ingest_event("srv-1", &make_message(&format!("evt-{i}"), (i + 1) * 1000));
        }

        match role.role_info() {
            WorkerRoleInfo::Replay {
                servers_loaded,
                events_buffered,
                ..
            } => {
                assert_eq!(servers_loaded, 1);
                assert_eq!(events_buffered, 5);
            }
            _ => panic!("expected Replay"),
        }
    }

    #[test]
    fn state_hashes_returns_per_server_hashes() {
        let mut role = ReplayRole::new(ReplayConfig::default());

        role.ingest_event("srv-1", &make_message("e1", 1000));
        role.ingest_event("srv-2", &make_message("e2", 2000));

        let hashes = role.state_hashes();
        assert_eq!(hashes.len(), 2);
    }

    #[test]
    fn multiple_servers_tracked_independently() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_server: 2,
        });

        for i in 0..3 {
            role.ingest_event("srv-1", &make_message(&format!("s1-{i}"), (i + 1) * 1000));
        }
        for i in 0..5 {
            role.ingest_event("srv-2", &make_message(&format!("s2-{i}"), (i + 1) * 1000));
        }

        // srv-1: 2 events (evicted 1), srv-2: 2 events (evicted 3)
        assert_eq!(role.servers["srv-1"].events.len(), 2);
        assert_eq!(role.servers["srv-2"].events.len(), 2);

        match role.role_info() {
            WorkerRoleInfo::Replay {
                servers_loaded,
                events_buffered,
                ..
            } => {
                assert_eq!(servers_loaded, 2);
                assert_eq!(events_buffered, 4);
            }
            _ => panic!("expected Replay"),
        }
    }
}
