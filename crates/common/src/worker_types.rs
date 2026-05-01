//! Wire protocol types for the worker node system.
//!
//! All types are serializable for gossipsub transport on the
//! `_willow_workers` topic. Defined here so both `willow-client`
//! (including WASM) and `willow-worker` (native-only) can use them.

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;
use willow_state::{Event, HeadsSummary, Snapshot};

/// Gossipsub topic for worker discovery and request/response.
pub const WORKERS_TOPIC: &str = "_willow_workers";

/// Gossipsub topic for server state operations (shared with clients).
pub const SERVER_OPS_TOPIC: &str = "_willow_server_ops";

/// Combined role identity and capacity info. The variant determines
/// the role — impossible to mismatch role type and capacity data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum WorkerRoleInfo {
    Replay {
        servers_loaded: u32,
        events_buffered: u32,
        max_events: u32,
        /// Total events currently buffered in each server's `PendingBuffer`
        /// waiting for chain predecessors. Useful for monitoring partition
        /// / offline-peer backpressure.
        pending_count: u32,
    },
    Storage {
        servers_tracked: u32,
        total_events_stored: u64,
        disk_used_bytes: u64,
    },
    Feedback {
        reports_accepted: u64,
        reports_rejected: u64,
        /// Gauge: peers currently throttled by the per-peer bucket.
        currently_rate_limited: u32,
        /// Gauge: true if the worker is hot-tripped on the global cap.
        global_rate_limited: bool,
    },
    // Future: File { ... }, Stream { ... }, Bot { ... }
}

impl WorkerRoleInfo {
    /// Returns the role name as a string for display/logging.
    pub fn role_name(&self) -> &'static str {
        match self {
            WorkerRoleInfo::Replay { .. } => "replay",
            WorkerRoleInfo::Storage { .. } => "storage",
            WorkerRoleInfo::Feedback { .. } => "feedback",
        }
    }
}

/// Periodic heartbeat broadcast by workers on `_willow_workers`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkerAnnouncement {
    pub peer_id: EndpointId,
    pub role: WorkerRoleInfo,
    pub servers: Vec<String>,
    pub timestamp: u64,
}

/// Top-level wire message for the `_willow_workers` gossipsub topic.
///
/// **Security note:** These messages are signed with Ed25519. All messages
/// are wrapped in a [`crate::WireMessage::Worker`] variant and signed via
/// [`crate::pack_wire`] before broadcast. Recipients verify signatures via
/// [`crate::unpack_wire`], which returns an error if the signature is invalid.
/// Unsigned, tampered, or wrong-variant messages are rejected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerWireMessage {
    /// Periodic heartbeat.
    Announcement(WorkerAnnouncement),

    /// Graceful departure notification.
    Departure { peer_id: EndpointId },

    /// Client requesting a service from a specific worker.
    Request {
        request_id: String,
        target_peer: EndpointId,
        payload: WorkerRequest,
    },

    /// Worker responding to a client request.
    Response {
        request_id: String,
        target_peer: EndpointId,
        payload: Box<WorkerResponse>,
    },
}

/// Request payloads sent by clients to workers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum WorkerRequest {
    /// State sync request (handled by replay nodes).
    /// Sends our heads so the worker can compute a delta.
    Sync {
        server_id: String,
        heads: HeadsSummary,
    },

    /// Paginated history request (handled by storage nodes).
    /// Channel is optional (None = all channels). Cursor is a
    /// HeadsSummary representing the point to paginate before.
    History {
        server_id: String,
        channel: Option<String>,
        before: Option<HeadsSummary>,
        limit: u32,
    },

    /// Submit a user feedback report (handled by feedback nodes).
    Feedback {
        /// 16-byte client-generated dedup key. Worker maintains an LRU
        /// cache of (signer, dedup_id) → issue_url so retries return
        /// the original URL.
        dedup_id: [u8; 16],
        /// 1..=200 chars (worker-validated).
        title: String,
        category: FeedbackCategory,
        /// 1..=8000 chars (worker-validated). Worker wraps this
        /// verbatim in a fenced markdown code block on GitHub.
        body: String,
        diagnostics: Option<FeedbackDiagnostics>,
    },
}

/// Response payloads sent by workers back to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WorkerResponse {
    /// Batch of events for sync catch-up.
    SyncBatch { events: Vec<Event> },

    /// Full DAG snapshot for far-behind peers.
    Snapshot {
        snapshot: Box<Snapshot>,
        post_snapshot_events: Vec<Event>,
    },

    /// Paginated history results.
    HistoryPage { events: Vec<Event>, has_more: bool },

    /// Request denied.
    Denied { reason: String },

    /// Feedback report accepted; GitHub issue created or dedup hit.
    FeedbackOk { issue_url: String },

    /// Feedback report rejected.
    FeedbackErr { reason: FeedbackErrReason },
}

impl PartialEq for WorkerResponse {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (WorkerResponse::Denied { reason: a }, WorkerResponse::Denied { reason: b }) => a == b,
            (
                WorkerResponse::HistoryPage {
                    has_more: a_more, ..
                },
                WorkerResponse::HistoryPage {
                    has_more: b_more, ..
                },
            ) => a_more == b_more,
            (
                WorkerResponse::FeedbackOk { issue_url: a },
                WorkerResponse::FeedbackOk { issue_url: b },
            ) => a == b,
            (
                WorkerResponse::FeedbackErr { reason: a },
                WorkerResponse::FeedbackErr { reason: b },
            ) => a == b,
            (WorkerResponse::SyncBatch { events: a }, WorkerResponse::SyncBatch { events: b }) => {
                // `Event` does not derive `PartialEq`; compare by the
                // canonical content hash, which IS each event's identity.
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.hash == y.hash)
            }
            (
                WorkerResponse::Snapshot {
                    snapshot: a,
                    post_snapshot_events: ae,
                },
                WorkerResponse::Snapshot {
                    snapshot: b,
                    post_snapshot_events: be,
                },
            ) => {
                // Compare snapshot by its canonical SHA-256 hash (documented
                // as the identity of the snapshot) and post-snapshot events
                // by their content hashes.
                a.hash == b.hash
                    && ae.len() == be.len()
                    && ae.iter().zip(be.iter()).all(|(x, y)| x.hash == y.hash)
            }
            _ => false,
        }
    }
}

/// The trait that each worker binary implements.
///
/// The state actor owns the implementor exclusively — `&mut self` is
/// safe because no other task can access it concurrently.
#[async_trait::async_trait]
pub trait WorkerRole: Send + 'static {
    /// Returns combined role identity and capacity info for heartbeats.
    fn role_info(&self) -> WorkerRoleInfo;

    /// Called when an event is received from gossipsub.
    fn on_event(&mut self, event: &Event);

    /// Handle an inbound request from a client. `signer` is the
    /// verified Ed25519 signer of the inbound `WireMessage`; roles
    /// that don't need it (replay, storage) ignore the parameter.
    async fn handle_request(
        &mut self,
        signer: willow_identity::EndpointId,
        req: WorkerRequest,
    ) -> WorkerResponse;

    /// Returns heads summaries for all tracked servers.
    /// Used by the sync actor to broadcast current DAG state.
    /// Default returns empty — override in roles that track server state.
    fn heads_summaries(&self) -> Vec<(String, HeadsSummary)> {
        vec![]
    }
}

/// Allocation strategy for which servers a worker serves.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum AllocationStrategy {
    /// Serve all discovered servers (initial implementation).
    #[default]
    Global,
    /// Serve only specific servers (future).
    PerServer(Vec<String>),
    /// Dynamic allocation based on load (future).
    Dynamic,
}

/// Top-level category for a feedback report. Surfaced as a label and
/// title prefix on the GitHub issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum FeedbackCategory {
    Bug,
    Suggestion,
    /// Free-form category. `detail` is a short subcategory string the
    /// user types (e.g. "performance", "docs"); shown in the issue
    /// title prefix as `[Other:<detail>]`.
    Other {
        /// Optional, <= 60 chars. Validated by the worker.
        detail: Option<String>,
    },
}

/// The submitting client's platform — coarse-grained on purpose so
/// the issue body cannot include a fingerprintable full UA string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum ClientPlatform {
    /// Browser submission. `ua_family` is `"<browser>/<major>"`,
    /// e.g. `"firefox/138"`. <= 40 chars.
    Web { ua_family: String },
    /// Native submission. `"linux"` / `"macos"` / `"windows"` and
    /// e.g. `"x86_64"` / `"aarch64"`.
    Native { os: String, arch: String },
}

/// Optional diagnostic info attached to a feedback report. Only
/// included when the user opts in via the UI checkbox; the disclosure
/// renders the *exact* value that will be sent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct FeedbackDiagnostics {
    /// `CARGO_PKG_VERSION` of the submitting client.
    pub app_version: String,
    /// Short git SHA from `option_env!("WILLOW_BUILD_SHA")` injected
    /// by `build.rs`. None in dev builds.
    pub build_hash: Option<String>,
    /// IETF BCP 47 locale tag (e.g. `"en-US"`).
    pub locale: Option<String>,
    /// Platform the submission originated from.
    pub client: ClientPlatform,
}

/// Reason a feedback request was rejected. Units are MILLISECONDS to
/// align with the broader `WireRejectReason` design
/// ([`docs/specs/2026-04-24-error-prefixes.md`]); consolidating the
/// two enums is a follow-up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum FeedbackErrReason {
    RateLimited {
        retry_after_ms: u64,
    },
    /// `field` <= 64 chars; `message` <= 200 chars (worker-enforced
    /// before constructing the reply, client-enforced on receipt).
    InvalidInput {
        field: String,
        message: String,
    },
    GithubFailure {
        status: u16,
        /// GitHub's `message` field, truncated to 200 chars.
        message: Option<String>,
    },
    /// Worker has no PAT configured, or PAT was revoked (401).
    Unconfigured,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pack_wire, unpack_wire, WireMessage};
    use willow_identity::Identity;

    fn gen_id() -> EndpointId {
        Identity::generate().endpoint_id()
    }

    /// Round-trip a `WorkerWireMessage` through the actual production wire
    /// path: wrapped in `WireMessage::Worker`, signed via `pack_wire`, and
    /// verified+decoded via `unpack_wire`.
    fn worker_wire_round_trip(msg: WorkerWireMessage, identity: &Identity) -> WorkerWireMessage {
        let wire = WireMessage::Worker(msg);
        let bytes = pack_wire(&wire, identity).expect("pack_wire should succeed");
        let (decoded, signer) = unpack_wire(&bytes).expect("unpack_wire should succeed");
        assert_eq!(signer, identity.endpoint_id(), "signer mismatch");
        match decoded {
            WireMessage::Worker(inner) => inner,
            _ => panic!("expected WireMessage::Worker"),
        }
    }

    #[test]
    fn worker_role_info_replay_round_trip() {
        let info = WorkerRoleInfo::Replay {
            servers_loaded: 3,
            events_buffered: 500,
            max_events: 1000,
            pending_count: 0,
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
            peer_id: gen_id(),
            role: WorkerRoleInfo::Replay {
                servers_loaded: 1,
                events_buffered: 100,
                max_events: 1000,
                pending_count: 0,
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
        let id = Identity::generate();
        let pid = id.endpoint_id();
        let msg = WorkerWireMessage::Announcement(WorkerAnnouncement {
            peer_id: pid,
            role: WorkerRoleInfo::Storage {
                servers_tracked: 2,
                total_events_stored: 5000,
                disk_used_bytes: 1_000_000,
            },
            servers: vec!["s1".to_string()],
            timestamp: 999,
        });
        let decoded = worker_wire_round_trip(msg, &id);
        match decoded {
            WorkerWireMessage::Announcement(a) => {
                assert_eq!(a.peer_id, pid);
                assert_eq!(a.servers, vec!["s1".to_string()]);
                assert_eq!(a.timestamp, 999);
                match a.role {
                    WorkerRoleInfo::Storage {
                        servers_tracked,
                        total_events_stored,
                        disk_used_bytes,
                    } => {
                        assert_eq!(servers_tracked, 2);
                        assert_eq!(total_events_stored, 5000);
                        assert_eq!(disk_used_bytes, 1_000_000);
                    }
                    _ => panic!("expected Storage role"),
                }
            }
            _ => panic!("expected Announcement"),
        }
    }

    #[test]
    fn worker_wire_message_departure_round_trip() {
        let id = Identity::generate();
        let pid = id.endpoint_id();
        let msg = WorkerWireMessage::Departure { peer_id: pid };
        let decoded = worker_wire_round_trip(msg, &id);
        match decoded {
            WorkerWireMessage::Departure { peer_id } => {
                assert_eq!(peer_id, pid);
            }
            _ => panic!("expected Departure"),
        }
    }

    #[test]
    fn worker_request_sync_round_trip() {
        let req = WorkerRequest::Sync {
            server_id: "srv-1".to_string(),
            heads: HeadsSummary::default(),
        };
        let bytes = bincode::serialize(&req).unwrap();
        let decoded: WorkerRequest = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerRequest::Sync { server_id, heads } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(heads, HeadsSummary::default());
            }
            _ => panic!("expected Sync"),
        }
    }

    #[test]
    fn worker_request_history_round_trip() {
        use std::collections::BTreeMap;
        use willow_state::{AuthorHead, EventHash};

        let mut heads_map = BTreeMap::new();
        heads_map.insert(
            gen_id(),
            AuthorHead {
                seq: 5,
                hash: EventHash::from_bytes(b"test"),
            },
        );
        let cursor = HeadsSummary { heads: heads_map };

        let req = WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: Some("general".to_string()),
            before: Some(cursor.clone()),
            limit: 50,
        };
        let bytes = bincode::serialize(&req).unwrap();
        let decoded: WorkerRequest = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerRequest::History {
                server_id,
                channel,
                before,
                limit,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(channel, Some("general".to_string()));
                assert_eq!(before, Some(cursor));
                assert_eq!(limit, 50);
            }
            _ => panic!("expected History"),
        }
    }

    #[test]
    fn worker_request_history_no_cursor_round_trip() {
        let req = WorkerRequest::History {
            server_id: "srv-1".to_string(),
            channel: None,
            before: None,
            limit: 25,
        };
        let bytes = bincode::serialize(&req).unwrap();
        let decoded: WorkerRequest = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerRequest::History {
                server_id,
                channel,
                before,
                limit,
            } => {
                assert_eq!(server_id, "srv-1");
                assert_eq!(channel, None);
                assert_eq!(before, None);
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
    fn worker_response_snapshot_round_trip() {
        use willow_state::{HeadsSummary, ServerState, Snapshot};

        let id = Identity::generate();
        let state = ServerState::new("srv-snap", "Snap Server", id.endpoint_id());
        let heads = HeadsSummary::default();
        let snapshot = Snapshot::new(state, heads);
        let snapshot_hash = snapshot.hash;

        let resp = WorkerResponse::Snapshot {
            snapshot: Box::new(snapshot),
            post_snapshot_events: vec![],
        };
        let bytes = bincode::serialize(&resp).unwrap();
        let decoded: WorkerResponse = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerResponse::Snapshot {
                snapshot,
                post_snapshot_events,
            } => {
                assert_eq!(snapshot.hash, snapshot_hash);
                assert_eq!(snapshot.state.server_id, "srv-snap");
                assert_eq!(snapshot.state.server_name, "Snap Server");
                assert!(post_snapshot_events.is_empty());
            }
            _ => panic!("expected Snapshot"),
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
        let id = Identity::generate();
        let pid = id.endpoint_id();
        let msg = WorkerWireMessage::Request {
            request_id: "req-123".to_string(),
            target_peer: pid,
            payload: WorkerRequest::Sync {
                server_id: "srv".to_string(),
                heads: HeadsSummary::default(),
            },
        };
        let decoded = worker_wire_round_trip(msg, &id);
        match decoded {
            WorkerWireMessage::Request {
                request_id,
                target_peer,
                payload,
            } => {
                assert_eq!(request_id, "req-123");
                assert_eq!(target_peer, pid);
                match payload {
                    WorkerRequest::Sync { server_id, heads } => {
                        assert_eq!(server_id, "srv");
                        assert_eq!(heads, HeadsSummary::default());
                    }
                    _ => panic!("expected Sync payload"),
                }
            }
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn worker_wire_message_response_round_trip() {
        let id = Identity::generate();
        let pid = id.endpoint_id();
        let msg = WorkerWireMessage::Response {
            request_id: "req-456".to_string(),
            target_peer: pid,
            payload: Box::new(WorkerResponse::Denied {
                reason: "unknown server".to_string(),
            }),
        };
        let decoded = worker_wire_round_trip(msg, &id);
        match decoded {
            WorkerWireMessage::Response {
                request_id,
                target_peer,
                payload,
            } => {
                assert_eq!(request_id, "req-456");
                assert_eq!(target_peer, pid);
                match *payload {
                    WorkerResponse::Denied { reason } => {
                        assert_eq!(reason, "unknown server");
                    }
                    _ => panic!("expected Denied payload"),
                }
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

    #[test]
    fn feedback_category_round_trips() {
        for cat in [
            FeedbackCategory::Bug,
            FeedbackCategory::Suggestion,
            FeedbackCategory::Other { detail: None },
            FeedbackCategory::Other {
                detail: Some("performance".to_string()),
            },
        ] {
            let bytes = bincode::serialize(&cat).unwrap();
            let decoded: FeedbackCategory = bincode::deserialize(&bytes).unwrap();
            assert_eq!(cat, decoded);
        }
    }

    #[test]
    fn client_platform_round_trips() {
        for cp in [
            ClientPlatform::Web {
                ua_family: "firefox/138".to_string(),
            },
            ClientPlatform::Native {
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
            },
        ] {
            let bytes = bincode::serialize(&cp).unwrap();
            let decoded: ClientPlatform = bincode::deserialize(&bytes).unwrap();
            assert_eq!(cp, decoded);
        }
    }

    #[test]
    fn feedback_diagnostics_round_trips() {
        let diag = FeedbackDiagnostics {
            app_version: "0.1.0".to_string(),
            build_hash: Some("abc1234".to_string()),
            locale: Some("en-US".to_string()),
            client: ClientPlatform::Web {
                ua_family: "firefox/138".to_string(),
            },
        };
        let bytes = bincode::serialize(&diag).unwrap();
        let decoded: FeedbackDiagnostics = bincode::deserialize(&bytes).unwrap();
        assert_eq!(diag, decoded);
    }

    #[test]
    fn feedback_err_reason_variants_round_trip() {
        for r in [
            FeedbackErrReason::RateLimited {
                retry_after_ms: 12_345,
            },
            FeedbackErrReason::InvalidInput {
                field: "title".to_string(),
                message: "too long".to_string(),
            },
            FeedbackErrReason::GithubFailure {
                status: 422,
                message: Some("Validation Failed".to_string()),
            },
            FeedbackErrReason::GithubFailure {
                status: 0,
                message: None,
            },
            FeedbackErrReason::Unconfigured,
        ] {
            let bytes = bincode::serialize(&r).unwrap();
            let decoded: FeedbackErrReason = bincode::deserialize(&bytes).unwrap();
            assert_eq!(r, decoded);
        }
    }

    #[test]
    fn worker_request_feedback_round_trip() {
        let id = Identity::generate();
        let req = WorkerRequest::Feedback {
            dedup_id: [7u8; 16],
            title: "It crashes".to_string(),
            category: FeedbackCategory::Bug,
            body: "Steps:\n1. open the app\n2. it crashes".to_string(),
            diagnostics: Some(FeedbackDiagnostics {
                app_version: "0.1.0".to_string(),
                build_hash: Some("abc1234".to_string()),
                locale: Some("en-US".to_string()),
                client: ClientPlatform::Web {
                    ua_family: "firefox/138".to_string(),
                },
            }),
        };
        let msg = WorkerWireMessage::Request {
            request_id: "rid-1".to_string(),
            target_peer: id.endpoint_id(),
            payload: req.clone(),
        };
        let decoded = worker_wire_round_trip(msg, &id);
        match decoded {
            WorkerWireMessage::Request { payload, .. } => assert_eq!(payload, req),
            _ => panic!("expected Request"),
        }
    }

    #[test]
    fn worker_response_feedback_round_trip() {
        let id = Identity::generate();
        for resp in [
            WorkerResponse::FeedbackOk {
                issue_url: "https://github.com/x/y/issues/42".to_string(),
            },
            WorkerResponse::FeedbackErr {
                reason: FeedbackErrReason::RateLimited {
                    retry_after_ms: 60_000,
                },
            },
        ] {
            let msg = WorkerWireMessage::Response {
                request_id: "rid-1".to_string(),
                target_peer: id.endpoint_id(),
                payload: Box::new(resp.clone()),
            };
            let decoded = worker_wire_round_trip(msg, &id);
            match decoded {
                WorkerWireMessage::Response { payload, .. } => assert_eq!(*payload, resp),
                _ => panic!("expected Response"),
            }
        }
    }

    #[test]
    fn worker_response_sync_batch_self_equal() {
        // Previously SyncBatch fell through to `_ => false`, making
        // `assert_eq!(resp, resp.clone())` silently fail.
        let resp = WorkerResponse::SyncBatch { events: Vec::new() };
        assert_eq!(resp, resp.clone());
    }

    #[test]
    fn worker_response_snapshot_self_equal() {
        use willow_state::{HeadsSummary, ServerState, Snapshot};

        // Previously Snapshot fell through to `_ => false`; ensure the
        // new arm matches and compares by canonical hash.
        let id = Identity::generate();
        let state = ServerState::new("srv-eq", "Eq Server", id.endpoint_id());
        let heads = HeadsSummary::default();
        let snapshot = Snapshot::new(state, heads);

        let resp = WorkerResponse::Snapshot {
            snapshot: Box::new(snapshot),
            post_snapshot_events: Vec::new(),
        };
        assert_eq!(resp, resp.clone());
    }

    #[test]
    fn worker_role_info_feedback_round_trip_and_name() {
        let info = WorkerRoleInfo::Feedback {
            reports_accepted: 17,
            reports_rejected: 4,
            currently_rate_limited: 2,
            global_rate_limited: false,
        };
        let bytes = bincode::serialize(&info).unwrap();
        let decoded: WorkerRoleInfo = bincode::deserialize(&bytes).unwrap();
        assert_eq!(info, decoded);
        assert_eq!(info.role_name(), "feedback");
    }
}
