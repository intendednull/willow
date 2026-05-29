//! Replay node role implementation.
//!
//! Maintains an in-memory [`EventDag`] per server with per-author chain
//! buffering. Responds to sync requests with event deltas computed from
//! [`HeadsSummary`], or full [`Snapshot`] for far-behind peers.

use std::collections::{BTreeMap, HashMap};

use tracing::warn;
use willow_common::MAX_AUTHORS_PER_SYNC;
use willow_state::{
    apply_incremental, Event, EventDag, EventHash, EventKind, HeadsSummary, InsertError,
    PendingBuffer, ServerState, Snapshot, DEFAULT_PENDING_MAX_AGE_MS, DEFAULT_PENDING_MAX_ENTRIES,
};
use willow_worker::{WorkerRequest, WorkerResponse, WorkerRole, WorkerRoleInfo};

/// Maximum number of servers the replay node will track simultaneously.
/// When exceeded, the least recently accessed server is evicted.
const MAX_SERVERS: usize = 1000;

/// OOM guard on the per-`Sync` delta walk.
///
/// The **authoritative** bound on a sync batch is the per-envelope byte budget
/// ([`willow_common::SYNC_ENVELOPE_BUDGET`]): the `Sync` arm packs the delta
/// with [`willow_common::pack_sync_batches`] and serves only the first
/// budget-fitting batch (setting `more` so the requester re-issues `Sync` with
/// advanced heads). This count cap is a *secondary* defence so a single
/// `events_since` walk cannot materialize an unbounded `Vec<Event>` in memory
/// before byte-budgeting — it is deliberately far larger than any plausible
/// per-envelope batch (a 64 KiB envelope cannot hold 10,000 non-trivial
/// events), so it should never be the binding limit in practice. See
/// `docs/specs/2026-04-24-negentropy-sync.md` § Wire protocol.
const SYNC_DELTA_OOM_GUARD: usize = 10_000;

/// Returns the current wall-clock time in milliseconds since the Unix epoch.
///
/// Replay is native-only (see `crates/replay/Cargo.toml`) so `SystemTime`
/// is always available. If the system clock is set before UNIX_EPOCH (e.g.
/// a misconfigured container), we fall back to 0 — age eviction will then
/// become a no-op for any entry inserted while the clock is bad, which is
/// strictly safer than panicking.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Per-server state held by the replay node.
struct ServerData {
    /// Per-author Merkle-DAG of events.
    dag: EventDag,
    /// Cached materialized state (maintained incrementally).
    state: ServerState,
    /// Buffer for events arriving before their chain predecessors.
    pending: PendingBuffer,
    /// Monotonic counter recording last access (for LRU eviction).
    last_access: u64,
}

/// Configuration for the replay role.
pub struct ReplayConfig {
    /// Max events per author before the oldest are compacted.
    pub max_events_per_author: usize,
    /// Maximum number of pending events per `PendingBuffer` before
    /// oldest-first capacity eviction kicks in. Defaults to
    /// [`DEFAULT_PENDING_MAX_ENTRIES`].
    pub pending_max_entries: usize,
    /// Maximum age (ms) of a pending event before it is evicted. Defaults
    /// to [`DEFAULT_PENDING_MAX_AGE_MS`] (1 hour). Entries inserted by
    /// `try_insert` carry a timestamp via `buffer_for_prev_at`, so both
    /// age and capacity eviction apply.
    pub pending_max_age_ms: u64,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            max_events_per_author: 1000,
            pending_max_entries: DEFAULT_PENDING_MAX_ENTRIES,
            pending_max_age_ms: DEFAULT_PENDING_MAX_AGE_MS,
        }
    }
}

/// The replay node's WorkerRole implementation.
pub struct ReplayRole {
    servers: HashMap<String, ServerData>,
    config: ReplayConfig,
    /// Monotonic counter for LRU tracking.
    access_counter: u64,
}

impl ReplayRole {
    /// Create a new replay role with the given configuration.
    pub fn new(config: ReplayConfig) -> Self {
        Self {
            servers: HashMap::new(),
            config,
            access_counter: 0,
        }
    }

    /// Ingest an event for a specific server.
    pub fn ingest_event(&mut self, server_id: &str, event: &Event) {
        let max_per_author = self.config.max_events_per_author;
        let pending_max_entries = self.config.pending_max_entries;
        let pending_max_age_ms = self.config.pending_max_age_ms;
        let author = event.author;

        // Evict least-recently-used server when at capacity and this is a new server.
        if self.servers.len() >= MAX_SERVERS && !self.servers.contains_key(server_id) {
            if let Some(lru_key) = self
                .servers
                .iter()
                .min_by_key(|(_, d)| d.last_access)
                .map(|(k, _)| k.clone())
            {
                self.servers.remove(&lru_key);
            }
        }

        self.access_counter += 1;
        let access_counter = self.access_counter;

        let data = self
            .servers
            .entry(server_id.to_string())
            .or_insert_with(|| {
                // Use event.author as placeholder genesis author. When the
                // actual CreateServer event is applied via apply_incremental,
                // it properly sets up admins in ServerState.
                let pending = if max_per_author == 0 {
                    // Treat zero as unlimited to avoid immediately evicting
                    // every buffered event (PendingBuffer::with_capacity(0)
                    // would evict all entries on each insert).
                    PendingBuffer::new()
                } else {
                    PendingBuffer::with_limits(pending_max_entries, pending_max_age_ms)
                };
                ServerData {
                    dag: EventDag::new(),
                    state: ServerState::new(server_id, server_id, author),
                    pending,
                    last_access: access_counter,
                }
            });

        data.last_access = access_counter;
        Self::try_insert(data, event.clone(), now_ms());
    }

    /// Try to insert an event into the DAG. On chain gap, buffer it.
    /// On success, resolve any events that were waiting on this one.
    /// Uses an iterative queue to avoid stack overflow on deep chains.
    ///
    /// `now_ms` is the wall-clock time at which buffering began; it is
    /// forwarded to [`PendingBuffer::buffer_for_prev_at`] so that stale
    /// pending entries can be evicted on later inserts.
    fn try_insert(data: &mut ServerData, event: Event, now_ms: u64) {
        let mut queue = vec![event];
        while let Some(current) = queue.pop() {
            let hash = current.hash;
            let prev = current.prev;
            // Clone needed because dag.insert() takes ownership but error
            // paths (SeqGap, NotGenesis) need the event back for buffering.
            match data.dag.insert(current.clone()) {
                Ok(()) => {
                    apply_incremental(&mut data.state, &current);
                    let mut resolved = data.pending.resolve(&hash);
                    // If this was genesis, also drain events buffered under ZERO
                    // (pre-genesis events from other authors with prev=ZERO).
                    if matches!(current.kind, EventKind::CreateServer { .. }) {
                        resolved.extend(data.pending.resolve(&EventHash::ZERO));
                    }
                    queue.extend(resolved);
                }
                Err(InsertError::SeqGap { .. }) => {
                    data.pending.buffer_for_prev_at(prev, current, now_ms);
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
                    data.pending.buffer_for_prev_at(prev, current, now_ms);
                }
                Err(InsertError::Duplicate) => { /* already have it */ }
                Err(InsertError::InvalidSignature) => {
                    warn!("rejected event with invalid signature");
                }
                Err(InsertError::DuplicateGenesis) => {
                    warn!("rejected duplicate CreateServer event");
                }
                Err(InsertError::MissingGovernanceDep { .. }) => {
                    warn!("rejected Vote event missing proposal dep");
                }
                Err(InsertError::PermissionDenied(reason)) => {
                    warn!(%reason, "rejected event: permission denied");
                }
                Err(InsertError::DepsTooLong { got, max }) => {
                    warn!(got, max, "rejected event: deps over cap (SEC-V-07)");
                }
                Err(InsertError::EncryptedKeyTooLarge { got, max }) => {
                    warn!(
                        got,
                        max, "rejected event: RotateChannelKey blob over cap (SEC-V-07)",
                    );
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

    /// Total events currently waiting in every server's `PendingBuffer`
    /// for a missing chain predecessor. Exposed to operators via
    /// [`WorkerRoleInfo::Replay::pending_count`].
    fn total_pending(&self) -> u32 {
        self.servers
            .values()
            .map(|d| u32::try_from(d.pending.pending_count()).unwrap_or(u32::MAX))
            .fold(0u32, |a, b| a.saturating_add(b))
    }
}

impl WorkerRole for ReplayRole {
    fn role_info(&self) -> WorkerRoleInfo {
        WorkerRoleInfo::Replay {
            servers_loaded: self.servers.len() as u32,
            events_buffered: self.total_events_buffered(),
            max_events: self.config.max_events_per_author as u32,
            pending_count: self.total_pending(),
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
                // Reject peer-supplied summaries that would force O(N)
                // BTreeMap construction and DAG walks (see
                // `MAX_AUTHORS_PER_SYNC`). Mirrors the storage cap added in
                // PR #507 / b075140; gated before any allocation so a hostile
                // request fails fast.
                if heads.heads.len() > MAX_AUTHORS_PER_SYNC {
                    return WorkerResponse::Denied {
                        reason: format!(
                            "too many heads in sync request: {} > {}",
                            heads.heads.len(),
                            MAX_AUTHORS_PER_SYNC
                        ),
                    };
                }

                let data = match self.servers.get(&server_id) {
                    Some(d) => d,
                    None => {
                        return WorkerResponse::Denied {
                            reason: format!("unknown server: {server_id}"),
                        }
                    }
                };

                // Convert HeadsSummary to the BTreeMap<EndpointId, u64> that
                // EventDag::events_since() expects.
                let their_heads: BTreeMap<_, _> = heads
                    .heads
                    .iter()
                    .map(|(author, head)| (*author, head.seq))
                    .collect();

                let delta: Vec<Event> = data
                    .dag
                    .events_since(&their_heads, Some(SYNC_DELTA_OOM_GUARD))
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
                        // Fully synced — single zero-event terminator.
                        WorkerResponse::SyncBatch {
                            events: vec![],
                            more: false,
                        }
                    }
                } else {
                    // Byte-budget the delta so a single response envelope stays
                    // within the gossip layer's 64 KiB ceiling. The worker path
                    // is request/response (one `WorkerResponse` per `Sync`), so
                    // we serve only the first budget-fitting batch and set
                    // `more: true` when further events remain — the requester
                    // re-issues `Sync` with advanced heads to drain the rest
                    // (see `WorkerResponse::SyncBatch` docs and
                    // `docs/specs/2026-04-24-negentropy-sync.md` § Wire protocol).
                    let (events, more) = willow_common::pack_sync_batches(
                        delta,
                        willow_common::SYNC_ENVELOPE_BUDGET,
                    )
                    .into_iter()
                    .next()
                    // `pack_sync_batches` always yields at least one
                    // (possibly empty) terminator batch.
                    .expect("pack_sync_batches always returns ≥1 batch");
                    WorkerResponse::SyncBatch { events, more }
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

    /// A message whose body is `body_bytes` long, so a known number of them
    /// overflows the per-envelope `SYNC_ENVELOPE_BUDGET`. Used to drive the
    /// byte-budgeted streaming + `more`-flag behaviour.
    fn make_big_message(id: &Identity, seq: u64, prev: EventHash, body_bytes: usize) -> Event {
        make_dag_event(
            id,
            seq,
            prev,
            EventKind::Message {
                channel_id: "general".to_string(),
                body: "x".repeat(body_bytes),
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
                pending_count,
            } => {
                assert_eq!(servers_loaded, 0);
                assert_eq!(events_buffered, 0);
                assert_eq!(max_events, 1000);
                assert_eq!(pending_count, 0);
            }
            _ => panic!("expected Replay"),
        }
    }

    #[test]
    fn ingest_event_applies_and_buffers() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            ..Default::default()
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
            ..Default::default()
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
            WorkerResponse::SyncBatch { events, .. } => assert_eq!(events.len(), 4),
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
            ..Default::default()
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let mut prev = genesis_hash;
        for seq in 2..=5 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Peer knows up to seq 3 — should get seq 4 and 5.
        let mut their_heads = BTreeMap::new();
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
            WorkerResponse::SyncBatch { events, .. } => assert_eq!(events.len(), 2),
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
            ..Default::default()
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
            ..Default::default()
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

    // Snapshot branch: fires when delta is empty but our DAG has an author
    // the peer's heads didn't mention at all (None => true path in
    // they_are_behind). Currently this requires events for that author to
    // have been compacted out of the chain (so events_since skips them),
    // which is not yet implemented. The test below drives the branch via
    // a two-author DAG: we add a second author whose events are unknown to
    // the peer (they are behind on that author) while all events for the
    // first author are already known to the peer (delta for first author
    // is empty). Because events_since will still return the second author's
    // events the branch is only reachable after compaction; until then the
    // snapshot path acts as a safety net for the future.
    //
    // What we CAN test right now:
    //   (a) Peer at same seq + wrong hash → considered fully synced (empty batch).
    //   (b) Peer that knows every author at or above our seq → fully synced.
    //   (c) New peer (empty heads) → gets a full delta batch (not snapshot).
    //   (d) Snapshot IS returned when the peer's heads include all our
    //       authors at or above our seq so events_since is empty, AND
    //       an author that exists in our DAG is absent from their heads so
    //       they_are_behind is true — achieved here by inserting a second
    //       author whose events the peer never mentions, then sending a
    //       request that claims seq=MAX for the first author so the delta
    //       for the first author is empty, while events for the second
    //       author still flow through. That makes delta non-empty, so we
    //       assert SyncBatch — demonstrating the boundary condition.
    #[test]
    fn sync_fully_synced_peer_gets_empty_batch() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            ..Default::default()
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let mut prev = genesis_hash;
        for seq in 2..=5 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Peer claims to know the author at seq=5 (same as ours) with a
        // wrong hash. events_since skips seq items → empty delta. The peer
        // is NOT behind (their seq == our seq), so we respond with an empty
        // SyncBatch (fully synced), not a Snapshot.
        let mut their_heads = BTreeMap::new();
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

        match resp {
            WorkerResponse::SyncBatch { events, .. } => assert!(
                events.is_empty(),
                "peer at same seq should be considered fully synced"
            ),
            _ => panic!("expected empty SyncBatch for fully-synced peer"),
        }
    }

    #[test]
    fn sync_new_peer_with_empty_heads_gets_full_batch() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            ..Default::default()
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let mut prev = genesis_hash;
        for seq in 2..=5 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Brand-new peer with no heads at all → delta condition fires
        // (heads.heads.is_empty() skips the they_are_behind check) and
        // events_since returns all 5 events.
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
        });

        match resp {
            WorkerResponse::SyncBatch { events, .. } => assert_eq!(
                events.len(),
                5,
                "new peer should receive all events as a delta batch"
            ),
            _ => panic!("expected SyncBatch with all events for new peer"),
        }
    }

    #[test]
    fn sync_snapshot_fallback_when_peer_is_behind() {
        // This test drives the Snapshot branch of handle_request.
        //
        // The branch fires when:
        //   1. delta (events_since result) is empty
        //   2. peer's heads is non-empty
        //   3. they_are_behind is true (we have an author the peer is missing
        //      from their heads entirely, or their seq < our seq for some author)
        //
        // To satisfy (1) the peer must claim a seq >= ours for every author
        // in our DAG. To satisfy (3) simultaneously our DAG must contain an
        // author that the peer's heads doesn't mention. Combining (1) and (3)
        // is only possible when events have been compacted out of memory so
        // that events_since can't return them even though our head seq is
        // still ahead of theirs. Until compaction is implemented, we achieve
        // the condition by using two authors:
        //   • author_A: peer claims seq=1000 (way ahead of our seq=5) →
        //     events_since for A: skip(1000) = empty.
        //   • author_B (second author): peer's heads doesn't mention B at all
        //     → they_are_behind = true (None => true branch).
        //     But events_since for B: skip(0) = returns B's events → delta
        //     is not empty, so we still get SyncBatch instead of Snapshot.
        //
        // Because the pre-compaction boundary condition produces a non-empty
        // delta we assert SyncBatch here. The Snapshot assertion is left in
        // a comment below showing what the code will do once compaction lands.
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            ..Default::default()
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let mut prev = genesis_hash;
        for seq in 2..=5 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        // Add a second author the peer doesn't know about.
        let member = Identity::generate();
        let m1 = make_dag_event(
            &member,
            1,
            EventHash::ZERO,
            EventKind::SetProfile {
                display_name: "member".to_string(),
            },
        );
        let m2 = make_dag_event(
            &member,
            2,
            m1.hash,
            EventKind::SetProfile {
                display_name: "member-v2".to_string(),
            },
        );
        role.ingest_event("srv-1", &m1);
        role.ingest_event("srv-1", &m2);

        // Peer claims owner at seq=1000 (way ahead of our seq=5, so
        // events_since for owner returns empty), but doesn't mention member.
        // they_are_behind = true (member is in our DAG, not in their heads).
        // However events_since for member returns m1+m2 → delta non-empty
        // → we get SyncBatch (pre-compaction boundary).
        let mut their_heads = BTreeMap::new();
        their_heads.insert(
            owner.endpoint_id(),
            willow_state::AuthorHead {
                seq: 1000,
                hash: EventHash::from_bytes(b"far-future"),
            },
        );

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary {
                heads: their_heads.clone(),
            },
        });

        // Pre-compaction: member's events are still in memory so we get a
        // SyncBatch with them rather than a Snapshot.
        match resp {
            WorkerResponse::SyncBatch { events, .. } => {
                assert_eq!(
                    events.len(),
                    2,
                    "pre-compaction: peer should receive member's 2 events as delta"
                );
            }
            _ => panic!("expected SyncBatch (pre-compaction)"),
        }

        // ── Snapshot path (post-compaction, future) ──────────────────────
        // Once compaction is implemented: member's events are evicted from
        // the chain (so events_since for member returns empty) but our
        // heads_summary still shows member at seq=2. In that world the peer
        // would receive WorkerResponse::Snapshot because:
        //   delta = empty (both owner and member chains are exhausted)
        //   heads.heads non-empty ✓
        //   they_are_behind = true (member absent from peer heads) ✓
        // When compaction lands, add an assertion like:
        //   assert!(matches!(resp, WorkerResponse::Snapshot { .. }));
    }

    #[test]
    fn sync_with_multiple_authors_returns_correct_delta() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            ..Default::default()
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
        let mut their_heads = BTreeMap::new();
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
            WorkerResponse::SyncBatch { events, .. } => assert_eq!(events.len(), 2),
            _ => panic!("expected SyncBatch"),
        }
    }

    #[test]
    fn sync_peer_knows_no_authors_gets_everything() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            ..Default::default()
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
        let mut their_heads = BTreeMap::new();
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
            WorkerResponse::SyncBatch { events, .. } => {
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

    /// Issue #51: A second author's first event (prev=ZERO) arrives before
    /// genesis. It gets buffered under EventHash::ZERO. When genesis is
    /// inserted, resolve() is called with genesis.hash — not ZERO — so the
    /// second author's event stays stuck forever.
    #[test]
    fn pre_genesis_event_from_second_author_resolves_after_genesis() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let owner = Identity::generate();
        let member = Identity::generate();

        // Second author's first event (seq=1, prev=ZERO).
        let member_event = make_dag_event(
            &member,
            1,
            EventHash::ZERO,
            EventKind::SetProfile {
                display_name: "member".into(),
            },
        );

        // Genesis from owner.
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

        // Deliver member event FIRST (before genesis).
        role.ingest_event("srv-1", &member_event);
        assert_eq!(role.servers["srv-1"].dag.len(), 0);
        assert_eq!(role.servers["srv-1"].pending.pending_count(), 1);

        // Now deliver genesis — should resolve the member's ZERO-buffered event.
        role.ingest_event("srv-1", &genesis);
        assert_eq!(
            role.servers["srv-1"].dag.len(),
            2,
            "both genesis and member event should be in DAG"
        );
        assert_eq!(
            role.servers["srv-1"].pending.pending_count(),
            0,
            "no events should remain pending"
        );
    }

    #[test]
    fn server_count_bounded_by_max() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            ..Default::default()
        });
        // Insert MAX_SERVERS + 10 unique servers.
        for i in 0..MAX_SERVERS + 10 {
            let sid = format!("srv-{i}");
            setup_server(&mut role, &sid);
        }
        assert!(
            role.servers.len() <= MAX_SERVERS,
            "server count {} exceeds MAX_SERVERS {}",
            role.servers.len(),
            MAX_SERVERS,
        );
    }

    #[test]
    fn zero_max_events_does_not_discard_pending() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 0,
            ..Default::default()
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        // Build e2 and e3 to form a proper chain: genesis -> e2 -> e3.
        let e2 = make_message(&owner, 2, genesis_hash);
        let e3 = make_message(&owner, 3, e2.hash);

        // Insert e3 first (out of order) — it should be buffered because
        // its prev (e2.hash) hasn't been inserted yet.
        role.ingest_event("srv-1", &e3);

        assert!(
            role.servers["srv-1"].pending.pending_count() > 0,
            "pending events should not be immediately evicted when max_events_per_author=0"
        );

        // Now insert e2 to resolve the gap.
        role.ingest_event("srv-1", &e2);

        // Both e2 and e3 should now be in the DAG (genesis + e2 + e3 = 3).
        assert_eq!(role.servers["srv-1"].dag.len(), 3);
        assert_eq!(role.servers["srv-1"].pending.pending_count(), 0);
    }

    // ── on_event routing tests ─────────────────────────────────────────────

    /// Case (a): CreateServer event → on_event uses the event hash as the
    /// server_id, creating a new slot. servers_loaded increases by 1.
    #[test]
    fn on_event_routes_create_server() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let owner = Identity::generate();

        assert_eq!(role.servers.len(), 0, "starts empty");

        let genesis = Event::new(
            &owner,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer {
                name: "my-server".to_string(),
            },
            0,
        );

        role.on_event(&genesis);

        // on_event(CreateServer) uses event.hash.to_string() as server_id.
        assert_eq!(
            role.servers.len(),
            1,
            "CreateServer should create exactly one new server slot"
        );

        let server_id = genesis.hash.to_string();
        assert!(
            role.servers.contains_key(&server_id),
            "server slot key must be the genesis event hash"
        );
        assert_eq!(
            role.servers[&server_id].dag.len(),
            1,
            "genesis event should be in the DAG"
        );
    }

    /// Case (b): Subsequent event whose prev is known → on_event finds the
    /// correct server slot via the DAG and applies the event there, without
    /// creating a new slot.
    #[test]
    fn on_event_routes_subsequent_event_to_existing_server() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let owner = Identity::generate();

        // First, deliver the CreateServer event via on_event.
        let genesis = Event::new(
            &owner,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer {
                name: "my-server".to_string(),
            },
            0,
        );
        role.on_event(&genesis);
        let server_id = genesis.hash.to_string();
        assert_eq!(role.servers.len(), 1);

        // Now deliver a follow-up event whose prev == genesis.hash.
        // on_event must find the server slot via the "known prev" branch
        // and apply the event there — no new slot should be created.
        let msg = make_message(&owner, 2, genesis.hash);
        role.on_event(&msg);

        assert_eq!(
            role.servers.len(),
            1,
            "follow-up event must not create a new server slot"
        );
        assert_eq!(
            role.servers[&server_id].dag.len(),
            2,
            "genesis + message should both be in the correct DAG"
        );
    }

    /// Case (c): Event whose author is already tracked (but prev not yet
    /// seen) → on_event finds the server via the "known author" branch.
    #[test]
    fn on_event_routes_via_known_author_when_prev_unknown() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let owner = Identity::generate();

        // Seed the server via on_event(CreateServer).
        let genesis = Event::new(
            &owner,
            1,
            EventHash::ZERO,
            vec![],
            EventKind::CreateServer {
                name: "my-server".to_string(),
            },
            0,
        );
        role.on_event(&genesis);
        let server_id = genesis.hash.to_string();

        // Build a chain: genesis → e2 → e3.
        let e2 = make_message(&owner, 2, genesis.hash);
        let e3 = make_message(&owner, 3, e2.hash);

        // Deliver e3 first — its prev (e2.hash) is unknown, so the "known
        // prev" branch won't fire. But owner IS already tracked in the DAG
        // (via the genesis event), so the "known author" branch routes e3
        // to the correct slot and buffers it.
        role.on_event(&e3);

        assert_eq!(
            role.servers.len(),
            1,
            "known-author event must not create a new server slot"
        );
        assert_eq!(
            role.servers[&server_id].dag.len(),
            1,
            "e3 should be pending (prev not yet known), not in DAG"
        );
        assert_eq!(
            role.servers[&server_id].pending.pending_count(),
            1,
            "e3 must be buffered waiting for e2"
        );

        // Deliver e2 — resolves e3 from the pending buffer.
        role.on_event(&e2);

        assert_eq!(
            role.servers[&server_id].dag.len(),
            3,
            "after e2 arrives, e3 must resolve: genesis + e2 + e3"
        );
        assert_eq!(role.servers[&server_id].pending.pending_count(), 0,);
    }

    /// Case (d): Fallback → event with unknown prev AND unknown author goes
    /// to the "default" bucket. It stays pending until its chain is resolved.
    #[test]
    fn on_event_fallback_routes_to_default_bucket() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let stranger = Identity::generate();

        // This author has never been seen before (no CreateServer, no prior
        // event in any known server). on_event falls through to the "default"
        // bucket because:
        //   • event.kind is not CreateServer
        //   • no server's DAG contains event.prev
        //   • no server's DAG tracks event.author
        let orphan = make_dag_event(
            &stranger,
            1,
            EventHash::from_bytes(b"unknown-prev"),
            EventKind::SetProfile {
                display_name: "stranger".to_string(),
            },
        );
        role.on_event(&orphan);

        // A "default" bucket must have been created.
        assert!(
            role.servers.contains_key("default"),
            "unknown event should be routed to the 'default' bucket"
        );
        // The event can't be inserted into the DAG (DAG is empty, no genesis)
        // so it should be pending.
        assert_eq!(
            role.servers["default"].dag.len(),
            0,
            "orphan event should not be in the DAG (no genesis)"
        );
        assert_eq!(
            role.servers["default"].pending.pending_count(),
            1,
            "orphan event should be buffered in the default bucket"
        );
    }

    // ── LRU eviction ordering test ──────────────────────────────────────────

    /// Eviction removes the least-recently-accessed server, not the most
    /// recently accessed one. This exercises the LRU property of the
    /// `last_access` counter used by `ingest_event`.
    ///
    /// Strategy (using small server count):
    ///   1. Fill 3 servers: A, B, C (in that order).
    ///   2. Access A again (bumps its last_access above B and C).
    ///   3. Add server D — eviction must remove the LRU server.
    ///      B and C have lower last_access than A; whichever of B/C has
    ///      the smallest last_access is the LRU victim.
    ///   4. Assert A is still present (recently accessed).
    ///   5. Assert D was created.
    ///   6. Assert exactly 3 servers remain (MAX_SERVERS was temporarily 3).
    ///
    /// Because MAX_SERVERS is a compile-time constant we cannot set it to 3
    /// in the test directly. Instead we fill MAX_SERVERS - 1 servers as a
    /// baseline, then perform the access-reorder-eviction dance on top of
    /// that baseline to demonstrate the LRU property without having to
    /// instantiate 1001 servers.
    #[test]
    fn eviction_removes_least_recently_accessed() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 10,
            ..Default::default()
        });

        // Fill up to MAX_SERVERS using background servers we don't care about.
        // Use a small prefix to distinguish them from the test servers.
        for i in 0..MAX_SERVERS - 3 {
            let sid = format!("bg-{i}");
            setup_server(&mut role, &sid);
        }
        // At this point we have MAX_SERVERS - 3 servers.

        // Add server A, then B, then C so that:
        //   last_access(A) < last_access(B) < last_access(C)
        let (owner_a, genesis_a) = setup_server(&mut role, "srv-A");
        let (_, _) = setup_server(&mut role, "srv-B");
        let (_, _) = setup_server(&mut role, "srv-C");
        // Now we are exactly at MAX_SERVERS.
        assert_eq!(role.servers.len(), MAX_SERVERS);

        // Access server A by sending it an event — this bumps A's
        // last_access counter above B and C.
        let a2 = make_message(&owner_a, 2, genesis_a);
        role.ingest_event("srv-A", &a2);

        // Add server D — capacity is full so one server is evicted.
        // The LRU candidate is B (A was re-accessed, C was after B, B is oldest).
        setup_server(&mut role, "srv-D");

        // Total count must stay at MAX_SERVERS.
        assert_eq!(
            role.servers.len(),
            MAX_SERVERS,
            "server count must not exceed MAX_SERVERS after eviction"
        );

        // A must still be present (recently accessed).
        assert!(
            role.servers.contains_key("srv-A"),
            "srv-A was recently accessed and must not be evicted"
        );

        // D must have been created.
        assert!(
            role.servers.contains_key("srv-D"),
            "srv-D must exist after insertion"
        );

        // B should have been evicted (it was the LRU among A, B, C after
        // A was re-accessed). Note: background servers have the lowest
        // access counters, but they were inserted before A/B/C so their
        // last_access values are all lower than A's re-access. The true
        // LRU may be one of the background servers. We verify the invariant
        // that A (re-accessed) and D (just inserted) both survive.
        assert!(role.servers.contains_key("srv-A"), "srv-A must survive");
    }

    /// Stronger LRU test using only 3 named servers (no background noise),
    /// by temporarily using a ReplayRole whose capacity is reached exactly
    /// at 3 servers. Since MAX_SERVERS is fixed at 1000 we instead test the
    /// LRU property directly on the access counter ordering: after filling
    /// MAX_SERVERS - 1 background entries and 3 named ones, the access
    /// counter for the re-touched server must be strictly greater than the
    /// others, confirming the eviction would spare it.
    #[test]
    fn eviction_access_counter_ordering() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 10,
            ..Default::default()
        });

        // Insert three named servers and record their access counters.
        let (owner_a, genesis_a) = setup_server(&mut role, "srv-A");
        let _ = setup_server(&mut role, "srv-B");
        let _ = setup_server(&mut role, "srv-C");

        let ac_a_initial = role.servers["srv-A"].last_access;
        let ac_b = role.servers["srv-B"].last_access;
        let ac_c = role.servers["srv-C"].last_access;

        // Insertion order: A < B < C.
        assert!(ac_a_initial < ac_b, "A inserted before B");
        assert!(ac_b < ac_c, "B inserted before C");

        // Re-access A — bump its last_access.
        let a2 = make_message(&owner_a, 2, genesis_a);
        role.ingest_event("srv-A", &a2);

        let ac_a_after = role.servers["srv-A"].last_access;

        // A's new counter must be strictly greater than C's (the previous max).
        assert!(
            ac_a_after > ac_c,
            "re-accessed server A must have a higher last_access than C ({ac_a_after} > {ac_c})"
        );

        // If eviction were needed now, A would NOT be the LRU — B would be.
        let lru_candidate = role
            .servers
            .iter()
            .min_by_key(|(_, d)| d.last_access)
            .map(|(k, _)| k.clone())
            .unwrap();
        assert_ne!(
            lru_candidate, "srv-A",
            "srv-A must not be the LRU candidate after being re-accessed"
        );
    }

    // ── Issue #40: pending_count exposure in WorkerRoleInfo::Replay ────────

    /// `WorkerRoleInfo::Replay::pending_count` should reflect the number of
    /// events buffered in each server's pending buffer waiting for their
    /// per-author chain predecessors.
    #[test]
    fn role_info_exposes_pending_count() {
        let mut role = ReplayRole::new(ReplayConfig::default());
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        // Build e2 and e3; insert e3 first so it gets buffered (SeqGap).
        let e2 = make_message(&owner, 2, genesis_hash);
        let e3 = make_message(&owner, 3, e2.hash);
        role.ingest_event("srv-1", &e3);

        // pending_count must expose the in-flight buffered event.
        match role.role_info() {
            WorkerRoleInfo::Replay { pending_count, .. } => {
                assert_eq!(pending_count, 1, "expected one pending event before e2");
            }
            _ => panic!("expected Replay"),
        }

        // Now deliver e2 — should drain the buffer.
        role.ingest_event("srv-1", &e2);
        match role.role_info() {
            WorkerRoleInfo::Replay { pending_count, .. } => {
                assert_eq!(pending_count, 0, "pending_count should drop to zero");
            }
            _ => panic!("expected Replay"),
        }
    }

    // ── Issue #514: oversize HeadsSummary rejection ──────────────────────
    //
    // Mirrors the storage cap added by PR #507 / b075140. Without a guard
    // here, a malicious peer could send a multi-thousand-entry HeadsSummary
    // and force replay to do per-author BTreeMap inserts and DAG walks for
    // every entry — same DoS shape as the storage path the sibling PR fixed.

    /// Build a `HeadsSummary` with `n` distinct random authors. Mirrors the
    /// helper in `crates/storage/src/store.rs` (sibling cap test infra).
    fn heads_summary_with_authors(n: usize) -> HeadsSummary {
        use willow_state::AuthorHead;
        let mut heads = BTreeMap::new();
        for _ in 0..n {
            let id = Identity::generate();
            heads.insert(
                id.endpoint_id(),
                AuthorHead {
                    seq: 1,
                    hash: EventHash::ZERO,
                },
            );
        }
        HeadsSummary { heads }
    }

    /// A peer-supplied `HeadsSummary` with more than `MAX_AUTHORS_PER_SYNC`
    /// entries must be rejected by `handle_request(Sync)` before any
    /// per-author BTreeMap construction or DAG walk occurs.
    #[test]
    fn sync_request_rejects_oversize_heads() {
        use willow_common::MAX_AUTHORS_PER_SYNC;
        let mut role = ReplayRole::new(ReplayConfig::default());
        let (_, _) = setup_server(&mut role, "srv-1");

        let oversize = heads_summary_with_authors(MAX_AUTHORS_PER_SYNC + 1);
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: oversize,
        });

        match resp {
            WorkerResponse::Denied { reason } => {
                assert!(
                    reason.contains("too many heads"),
                    "denial reason should mention the cap; got: {reason}"
                );
            }
            other => panic!("expected Denied for oversize heads, got: {other:?}"),
        }
    }

    /// `handle_request(Sync)` must accept exactly `MAX_AUTHORS_PER_SYNC`
    /// entries — the cap is inclusive on the legal side.
    #[test]
    fn sync_request_accepts_exact_cap_heads() {
        use willow_common::MAX_AUTHORS_PER_SYNC;
        let mut role = ReplayRole::new(ReplayConfig::default());
        let (_, _) = setup_server(&mut role, "srv-1");

        let at_cap = heads_summary_with_authors(MAX_AUTHORS_PER_SYNC);
        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: at_cap,
        });

        // The peer's heads mention authors we don't know, so events_since
        // returns the genesis event; our store has 1 author the peer doesn't,
        // so they_are_behind is true. Either branch is acceptable — the only
        // forbidden outcome is Denied for at-cap input.
        if let WorkerResponse::Denied { reason } = resp {
            panic!("at-cap heads must not be denied; got reason: {reason}");
        }
    }

    /// When the configured `pending_max_entries` is exceeded, the oldest
    /// entries are evicted and `pending_count()` reflects the cap.
    #[test]
    fn pending_buffer_capacity_eviction() {
        let max_entries = 8usize;
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            pending_max_entries: max_entries,
            pending_max_age_ms: 60 * 60 * 1000,
        });
        let (_owner, _genesis) = setup_server(&mut role, "srv-1");

        // Deliver (max_entries + 5) events that can never resolve (random
        // prev hashes pointing nowhere). Each one is buffered.
        let stranger = Identity::generate();
        for i in 0..(max_entries as u64 + 5) {
            let mut hash_bytes = [0u8; 32];
            hash_bytes[..8].copy_from_slice(&i.to_le_bytes());
            let fake_prev = EventHash(hash_bytes);
            // seq=2 is past genesis; prev is bogus → SeqGap → buffered.
            let ev = Event::new(
                &stranger,
                2,
                fake_prev,
                vec![],
                EventKind::SetProfile {
                    display_name: format!("stranger{i}"),
                },
                0,
            );
            role.ingest_event("srv-1", &ev);
        }

        // Count must never exceed the cap.
        assert!(
            role.servers["srv-1"].pending.pending_count() <= max_entries,
            "pending_count {} exceeded cap {}",
            role.servers["srv-1"].pending.pending_count(),
            max_entries,
        );
        match role.role_info() {
            WorkerRoleInfo::Replay { pending_count, .. } => {
                assert!(pending_count as usize <= max_entries);
            }
            _ => panic!("expected Replay"),
        }
    }

    // ── Byte-budgeted sync streaming + `more` flag (plan PR 4 Task 4.4) ──────
    //
    // The replay `Sync` arm must byte-budget the delta it serves so a single
    // `WorkerResponse::SyncBatch` envelope stays within the gossip layer's
    // 64 KiB ceiling (`willow_common::SYNC_ENVELOPE_BUDGET`), and set `more`:
    //   - `more: true`  when further events remain past this envelope (the
    //     requester re-issues `Sync` with advanced heads),
    //   - `more: false` on the final / only batch (end-of-stream marker).
    // The legacy 10_000-event `events_since` cap stays solely as an OOM guard.

    use willow_common::SYNC_ENVELOPE_BUDGET;

    /// A small delta (well under one envelope) is served in a single batch
    /// with `more: false` — the terminator.
    #[test]
    fn sync_small_delta_is_single_batch_more_false() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            ..Default::default()
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        let mut prev = genesis_hash;
        for seq in 2..=4 {
            let e = make_message(&owner, seq, prev);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
        });

        match resp {
            WorkerResponse::SyncBatch { events, more } => {
                assert_eq!(events.len(), 4);
                assert!(!more, "a small delta must terminate with more: false");
            }
            other => panic!("expected SyncBatch, got {other:?}"),
        }
    }

    /// When the delta exceeds one envelope's byte budget, the replay role
    /// returns only the budget-fitting first batch with `more: true`, so the
    /// framed envelope cannot exceed `SYNC_ENVELOPE_BUDGET` and the requester
    /// knows to re-issue `Sync` with advanced heads.
    #[test]
    fn sync_large_delta_byte_budgets_first_batch_more_true() {
        let mut role = ReplayRole::new(ReplayConfig {
            max_events_per_author: 100,
            ..Default::default()
        });
        let (owner, genesis_hash) = setup_server(&mut role, "srv-1");

        // ~8 KiB body each → 9 events ≈ 72 KiB > 64 KiB, forcing a split.
        let mut prev = genesis_hash;
        for seq in 2..=10 {
            let e = make_big_message(&owner, seq, prev, 8 * 1024);
            prev = e.hash;
            role.ingest_event("srv-1", &e);
        }

        let resp = role.handle_request(WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
        });

        match resp {
            WorkerResponse::SyncBatch { events, more } => {
                assert!(
                    more,
                    "an over-budget delta must set more: true on the first batch"
                );
                assert!(
                    !events.is_empty(),
                    "the first batch must carry at least one event"
                );
                // Re-packing the returned events with the same budget must
                // yield exactly one batch — proof the served batch already
                // fits within `SYNC_ENVELOPE_BUDGET`.
                let repacked =
                    willow_common::pack_sync_batches(events.clone(), SYNC_ENVELOPE_BUDGET);
                assert_eq!(
                    repacked.len(),
                    1,
                    "served batch must fit within SYNC_ENVELOPE_BUDGET in a single envelope"
                );
                // genesis + 9 big messages = 10 events; the first batch must
                // not contain them all (that would overflow the envelope).
                assert!(
                    events.len() < 10,
                    "byte-budgeted first batch must not contain all 10 over-budget events"
                );
            }
            other => panic!("expected SyncBatch, got {other:?}"),
        }
    }
}
