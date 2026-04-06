//! Replay node role implementation.
//!
//! Maintains an in-memory [`EventDag`] per server with per-author chain
//! buffering. Responds to sync requests with event deltas computed from
//! [`HeadsSummary`], or full [`Snapshot`] for far-behind peers.

use std::collections::HashMap;

use tracing::warn;
use willow_state::{
    apply_incremental, Event, EventDag, EventKind, HeadsSummary, InsertError, PendingBuffer,
    ServerState, Snapshot,
};
use willow_worker::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};

/// Per-server state held by the replay node.
struct ServerData {
    /// Per-author Merkle-DAG of events.
    dag: EventDag,
    /// Cached materialized state (maintained incrementally).
    state: ServerState,
    /// Buffer for events arriving before their chain predecessors.
    pending: PendingBuffer,
    /// Max events per author before compaction (reserved for future use).
    #[allow(dead_code)]
    max_events_per_author: usize,
}

/// Configuration for the replay role.
pub struct ReplayConfig {
    /// Max events per author before the oldest are compacted.
    pub max_events_per_author: usize,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            max_events_per_author: 1000,
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
        let max_per_author = self.config.max_events_per_author;
        let author = event.author;
        let data = self
            .servers
            .entry(server_id.to_string())
            .or_insert_with(|| {
                // Use event.author as placeholder genesis author. When the
                // actual CreateServer event is applied via apply_incremental,
                // it properly sets up admins in ServerState.
                ServerData {
                    dag: EventDag::new(),
                    state: ServerState::new(server_id, server_id, author),
                    pending: PendingBuffer::new(),
                    max_events_per_author: max_per_author,
                }
            });

        Self::try_insert(data, event.clone());

        // Evict pending events if the buffer grows too large.
        // Cap at 10x the per-author limit as a reasonable upper bound.
        let max_pending = max_per_author * 10;
        data.pending.evict_to(max_pending);
    }

    /// Try to insert an event into the DAG. On chain gap, buffer it.
    /// On success, resolve any events that were waiting on this one.
    /// Uses an iterative queue to avoid stack overflow on deep chains.
    fn try_insert(data: &mut ServerData, event: Event) {
        let mut queue = vec![event];
        while let Some(current) = queue.pop() {
            let hash = current.hash;
            let prev = current.prev;
            // Clone needed because dag.insert() takes ownership but error
            // paths (SeqGap, NotGenesis) need the event back for buffering.
            match data.dag.insert(current.clone()) {
                Ok(()) => {
                    apply_incremental(&mut data.state, &current);
                    let resolved = data.pending.resolve(&hash);
                    queue.extend(resolved);
                }
                Err(InsertError::SeqGap { .. }) => {
                    data.pending.buffer_for_prev(prev, current);
                }
                Err(InsertError::PrevMismatch {
                    author,
                    expected,
                    got,
                }) => {
                    warn!(
                        %author, %expected, %got,
                        "PrevMismatch: equivocation or conflicting chain — dropping event"
                    );
                }
                Err(InsertError::NotGenesis) => {
                    data.pending.buffer_for_prev(prev, current);
                }
                Err(InsertError::Duplicate) => { /* already have it */ }
                Err(InsertError::InvalidSignature) => {
                    warn!("rejected event with invalid signature");
                }
            }
        }
    }

    /// Get the heads summaries for all tracked servers.
    fn compute_heads_summaries(&self) -> Vec<(String, HeadsSummary)> {
        self.servers
            .iter()
            .map(|(id, data)| (id.clone(), data.dag.heads_summary()))
            .collect()
    }

    /// Total events buffered across all servers.
    fn total_events_buffered(&self) -> u32 {
        self.servers
            .values()
            .map(|d| u32::try_from(d.dag.len()).unwrap_or(u32::MAX))
            .fold(0u32, |a, b| a.saturating_add(b))
    }
}

impl WorkerRole for ReplayRole {
    fn role_info(&self) -> WorkerRoleInfo {
        WorkerRoleInfo::Replay {
            servers_loaded: self.servers.len() as u32,
            events_buffered: self.total_events_buffered(),
            max_events: self.config.max_events_per_author as u32,
        }
    }

    fn on_event(&mut self, event: &Event) {
        // Derive server_id from the event's DAG context:
        // 1. CreateServer → use the event hash as server_id.
        // 2. Known prev hash → find the server whose DAG contains the
        //    predecessor. This is the most precise check (prev is unique
        //    to one DAG) and handles both existing and new authors.
        // 3. Known author → find the server whose DAG tracks this author.
        //    Less precise (author could be in multiple servers) but
        //    catches events whose prev we haven't seen yet.
        // 4. Fallback → "default" bucket (event will be buffered until
        //    its predecessor chain connects it to a known server).
        let server_id = if let EventKind::CreateServer { .. } = &event.kind {
            event.hash.to_string()
        } else if let Some((id, _)) = self
            .servers
            .iter()
            .find(|(_, data)| data.dag.get(&event.prev).is_some())
        {
            id.clone()
        } else if let Some((id, _)) = self
            .servers
            .iter()
            .find(|(_, data)| data.dag.latest_seq(&event.author) > 0)
        {
            id.clone()
        } else {
            "default".to_string()
        };
        self.ingest_event(&server_id, event);
    }

    fn handle_request(&mut self, req: WorkerRequest) -> WorkerResponse {
        match req {
            WorkerRequest::Sync { server_id, heads } => {
                let data = match self.servers.get(&server_id) {
                    Some(d) => d,
                    None => {
                        return WorkerResponse::Denied {
                            reason: format!("unknown server: {server_id}"),
                        }
                    }
                };

                // Convert HeadsSummary to the HashMap<EndpointId, u64> that
                // EventDag::events_since() expects.
                let their_heads: HashMap<_, _> = heads
                    .heads
                    .iter()
                    .map(|(author, head)| (*author, head.seq))
                    .collect();

                let delta: Vec<Event> = data
                    .dag
                    .events_since(&their_heads)
                    .into_iter()
                    .cloned()
                    .collect();

                if delta.is_empty() && !heads.heads.is_empty() {
                    // They may be too far behind (events compacted), or synced.
                    // Check if they know authors we don't — if so, they're synced.
                    // Otherwise send a snapshot.
                    let our_heads = data.dag.heads_summary();
                    let they_are_behind = our_heads.heads.iter().any(|(author, our_head)| {
                        match heads.heads.get(author) {
                            Some(their_head) => their_head.seq < our_head.seq,
                            None => true,
                        }
                    });

                    if they_are_behind {
                        let snapshot = Snapshot::new(data.state.clone(), our_heads);
                        WorkerResponse::Snapshot {
                            snapshot: Box::new(snapshot),
                            post_snapshot_events: vec![],
                        }
                    } else {
                        // Fully synced.
                        WorkerResponse::SyncBatch { events: vec![] }
                    }
                } else {
                    WorkerResponse::SyncBatch { events: delta }
                }
            }
            WorkerRequest::History { .. } => WorkerResponse::Denied {
                reason: "replay nodes do not serve history".to_string(),
            },
        }
    }

    fn heads_summaries(&self) -> Vec<(String, HeadsSummary)> {
        self.compute_heads_summaries()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;
    use willow_state::{EventHash, EventKind};

    fn make_dag_event(id: &Identity, seq: u64, prev: EventHash, kind: EventKind) -> Event {
        Event::new(id, seq, prev, vec![], kind, seq * 1000)
    }

    fn make_message(id: &Identity, seq: u64, prev: EventHash) -> Event {
        make_dag_event(
            id,
            seq,
            prev,
            EventKind::Message {
                channel_id: "general".to_string(),
                body: format!("message seq={seq}"),
                reply_to: None,
            },
        )
    }

    /// Helper to build a server with a genesis event and return the identity + genesis hash.
    fn setup_server(role: &mut ReplayRole, server_id: &str) -> (Identity, EventHash) {
        let owner = Identity::generate();
        let genesis = Event::new(
            &owner,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer {
                name: server_id.to_string(),
            },
            0,
        );
        role.ingest_event(server_id, &genesis);
        (owner, genesis.hash)
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
            max_events_per_author: 100,
        });
        let (_, _) = setup_server(&mut role, "srv-1");

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
    fn sync_request_returns_events_since_empty_heads() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        // Add some messages.
        let mut prev = genesis_hash;
        for seq in 2..=4 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Empty heads = new peer, should get all events.
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
        });

        match resp {
            WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 4),
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn sync_request_unknown_server_denied() {
        let mut role = ReplayRole::new(ReplayConfig::default());

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "nonexistent".to_string(),
            heads: HeadsSummary::default(),
        });

        match resp {
            WorkerResponse::Denied { reason } => assert!(reason.contains("unknown server")),
            _ => panic!("expected Denied"),
        }
    }

    #[test]
    fn sync_request_returns_delta() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let mut prev = genesis_hash;
        for seq in 2..=5 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Peer knows up to seq 3 — should get seq 4 and 5.
        let mut their_heads = HashMap::new();
        their_heads.insert(
            owner.endpoint_id(),
            willow_state::AuthorHead {
                seq: 3,
                hash: EventHash::from_bytes(b"doesnt-matter-for-delta"),
            },
        );
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary { heads: their_heads },
        });

        match resp {
            WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 2),
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn history_request_denied_by_replay_node() {
        let mut role = ReplayRole::new(ReplayConfig::default());

        let resp = role.handle_request(WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: Some("general".to_string()),
            before: None,
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
            max_events_per_author: 100,
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let mut prev = genesis_hash;
        for seq in 2..=5 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
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
    fn heads_summaries_returns_per_server() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let (_, _) = setup_server(&mut role, "srv-1");
        let (_, _) = setup_server(&mut role, "srv-2");

        let summaries = role.heads_summaries();
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn multiple_servers_tracked_independently() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
        });
        let (owner1, genesis1) = setup_server(&mut role, "srv-1");
        let (owner2, genesis2) = setup_server(&mut role, "srv-2");

        // Add 3 messages to srv-1.
        let mut prev = genesis1;
        for seq in 2..=4 {
            let e = make_message(&owner1, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Add 5 messages to srv-2.
        let mut prev = genesis2;
        for seq in 2..=6 {
            let e = make_message(&owner2, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-2", &e);
        }

        assert_eq!(role.servers["srv-1"].dag.len(), 4);
        assert_eq!(role.servers["srv-2"].dag.len(), 6);

        match role.role_info() {
            WorkerRoleInfo::Replay {
                servers_loaded,
                events_buffered,
                ..
            } => {
                assert_eq!(servers_loaded, 2);
                assert_eq!(events_buffered, 10);
            }
            _ => panic!("expected Replay"),
        }
    }

    #[test]
    fn duplicate_events_are_deduplicated() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let e = make_message(&owner, 2, genesis_hash);
        role.ingest_event("srv-1", &e);
        role.ingest_event("srv-1", &e); // duplicate

        assert_eq!(role.servers["srv-1"].dag.len(), 2); // genesis + 1
    }

    #[test]
    fn out_of_order_events_buffered_and_resolved() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        // Create a chain: genesis → e2 → e3
        let e2 = make_message(&owner, 2, genesis_hash);
        let e3 = make_message(&owner, 3, e2.hash);

        // Deliver e3 FIRST (out of order) — should be buffered.
        role.ingest_event("srv-1", &e3);
        assert_eq!(role.servers["srv-1"].dag.len(), 1); // only genesis
        assert_eq!(role.servers["srv-1"].pending.pending_count(), 1);

        // Now deliver e2 — should resolve e3 from the buffer.
        role.ingest_event("srv-1", &e2);
        assert_eq!(role.servers["srv-1"].dag.len(), 3); // genesis + e2 + e3
        assert_eq!(role.servers["srv-1"].pending.pending_count(), 0);
    }

    #[test]
    fn deeply_out_of_order_chain_resolves() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        // Create chain: genesis → e2 → e3 → e4
        let e2 = make_message(&owner, 2, genesis_hash);
        let e3 = make_message(&owner, 3, e2.hash);
        let e4 = make_message(&owner, 4, e3.hash);

        // Deliver in reverse order: e4, e3, e2
        role.ingest_event("srv-1", &e4);
        role.ingest_event("srv-1", &e3);
        assert_eq!(role.servers["srv-1"].dag.len(), 1); // only genesis
        assert_eq!(role.servers["srv-1"].pending.pending_count(), 2);

        // Deliver e2 — should cascade: e2 resolves e3, e3 resolves e4.
        role.ingest_event("srv-1", &e2);
        assert_eq!(role.servers["srv-1"].dag.len(), 4);
        assert_eq!(role.servers["srv-1"].pending.pending_count(), 0);
    }

    #[test]
    fn sync_snapshot_fallback_when_peer_is_behind() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        // Add messages so the replay node has state.
        let mut prev = genesis_hash;
        for seq in 2..=5 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Peer claims to know the author at seq=5 with a WRONG hash.
        // events_since will return empty (seqs match), but the peer is
        // "behind" because the hash doesn't match. This should trigger
        // snapshot fallback.
        let mut their_heads = HashMap::new();
        their_heads.insert(
            owner.endpoint_id(),
            willow_state::AuthorHead {
                seq: 5,
                hash: EventHash::from_bytes(b"wrong-hash"),
            },
        );

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary { heads: their_heads },
        });

        // Peer is at same seq so they are NOT behind — should get empty batch.
        match resp {
            WorkerResponse::SyncBatch { events } => assert!(events.is_empty()),
            _ => panic!("expected SyncBatch (synced)"),
        }

        // Now test a peer that is truly behind (lower seq).
        let mut behind_heads = HashMap::new();
        behind_heads.insert(
            owner.endpoint_id(),
            willow_state::AuthorHead {
                seq: 100, // higher than ours — they won't appear behind
                hash: EventHash::from_bytes(b"ahead"),
            },
        );
        // Add a second author that the peer doesn't know about at all.
        let author2 = Identity::generate();
        let a2_msg = make_dag_event(
            &author2,
            1,
            EventHash::ZERO,
            EventKind::CreateServer {
                name: "srv-1".to_string(),
            },
        );
        // Can't insert a second CreateServer into the same DAG, so just
        // test the "we have author, they don't" path.
        // Our heads include owner; their heads include owner at seq=100 but
        // NOT author2 (if we had one). Let's use a simpler scenario:

        // Peer has empty heads — gets all events as delta (not snapshot).
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
        });
        match resp {
            WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 5),
            _ => panic!("expected SyncBatch for new peer"),
        }
    }

    #[test]
    fn sync_with_multiple_authors_returns_correct_delta() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        // Owner sends 3 messages (seq 2, 3, 4).
        let mut prev = genesis_hash;
        for seq in 2..=4 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Second author joins and sends messages.
        let author2 = Identity::generate();
        let a2_e1 = make_dag_event(
            &author2,
            1,
            EventHash::ZERO,
            EventKind::Message {
                channel_id: "general".to_string(),
                body: "author2 msg1".to_string(),
                reply_to: None,
            },
        );
        let a2_e2 = make_dag_event(
            &author2,
            2,
            a2_e1.hash,
            EventKind::Message {
                channel_id: "general".to_string(),
                body: "author2 msg2".to_string(),
                reply_to: None,
            },
        );
        role.ingest_event("srv-1", &a2_e1);
        role.ingest_event("srv-1", &a2_e2);

        // Total: 4 (owner) + 2 (author2) = 6 events.
        assert_eq!(role.servers["srv-1"].dag.len(), 6);

        // Peer knows owner at seq 3 and author2 at seq 1.
        // Should get: owner seq 4, author2 seq 2 = 2 events.
        let mut their_heads = HashMap::new();
        their_heads.insert(
            owner.endpoint_id(),
            willow_state::AuthorHead {
                seq: 3,
                hash: EventHash::from_bytes(b"irrelevant"),
            },
        );
        their_heads.insert(
            author2.endpoint_id(),
            willow_state::AuthorHead {
                seq: 1,
                hash: EventHash::from_bytes(b"irrelevant"),
            },
        );

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary { heads: their_heads },
        });

        match resp {
            WorkerResponse::SyncBatch { events } => assert_eq!(events.len(), 2),
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn sync_peer_knows_no_authors_gets_everything() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let mut prev = genesis_hash;
        for seq in 2..=3 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Peer has heads for a completely different author — doesn't know owner.
        let unknown = Identity::generate();
        let mut their_heads = HashMap::new();
        their_heads.insert(
            unknown.endpoint_id(),
            willow_state::AuthorHead {
                seq: 10,
                hash: EventHash::from_bytes(b"unknown"),
            },
        );

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary { heads: their_heads },
        });

        match resp {
            WorkerResponse::SyncBatch { events } => {
                // Should get all 3 events (genesis + 2 messages).
                assert_eq!(events.len(), 3);
            }
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn heads_summaries_content_is_correct() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let e2 = make_message(&owner, 2, genesis_hash);
        role.ingest_event("srv-1", &e2);

        let summaries = role.heads_summaries();
        assert_eq!(summaries.len(), 1);
        let (_, heads) = &summaries[0];
        let head = heads.heads.get(&owner.endpoint_id()).unwrap();
        assert_eq!(head.seq, 2);
        assert_eq!(head.hash, e2.hash);
    }

    #[test]
    fn non_genesis_event_buffered_until_genesis_arrives() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let owner = Identity::generate();

        // Create genesis and a follow-up message.
        let genesis = Event::new(
            &owner,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer {
                name: "srv-1".to_string(),
            },
            0,
        );
        let msg = make_message(&owner, 2, genesis.hash);

        // Deliver message FIRST (before genesis) to a brand new server.
        role.ingest_event("srv-1", &msg);

        // ServerData was created, but the event should be pending (NotGenesis).
        assert_eq!(role.servers["srv-1"].dag.len(), 0);
        assert_eq!(role.servers["srv-1"].pending.pending_count(), 1);

        // Now deliver genesis — should resolve the buffered message.
        role.ingest_event("srv-1", &genesis);
        assert_eq!(role.servers["srv-1"].dag.len(), 2); // genesis + msg
        assert_eq!(role.servers["srv-1"].pending.pending_count(), 0);
    }
}
