//! Client-side worker discovery cache.
//!
//! Populated from [`WorkerAnnouncement`] heartbeats received on the
//! `_willow_workers` gossipsub topic. Entries are evicted after a TTL
//! (default 30s) without a heartbeat.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use willow_common::{WorkerAnnouncement, WorkerRoleInfo};
use willow_identity::EndpointId;

/// Information about a known worker.
#[derive(Debug, Clone)]
pub struct WorkerInfo {
    /// The worker's peer ID.
    pub peer_id: EndpointId,
    /// Role identity and capacity info.
    pub role: WorkerRoleInfo,
    /// Server IDs this worker serves.
    pub servers: Vec<String>,
    /// When we last received a heartbeat.
    pub last_seen: Instant,
}

/// Cache of discovered workers with TTL-based eviction.
pub struct WorkerCache {
    workers: HashMap<EndpointId, WorkerInfo>,
    ttl: Duration,
}

impl WorkerCache {
    /// Create a new cache with the given TTL.
    pub fn new(ttl: Duration) -> Self {
        Self {
            workers: HashMap::new(),
            ttl,
        }
    }

    /// Update the cache from a heartbeat announcement.
    pub fn update(&mut self, announcement: &WorkerAnnouncement) {
        self.workers.insert(
            announcement.peer_id,
            WorkerInfo {
                peer_id: announcement.peer_id,
                role: announcement.role.clone(),
                servers: announcement.servers.clone(),
                last_seen: Instant::now(),
            },
        );
    }

    /// Remove a worker (e.g., on departure message).
    pub fn remove(&mut self, peer_id: &EndpointId) {
        self.workers.remove(peer_id);
    }

    /// Evict workers that haven't sent a heartbeat within the TTL.
    pub fn evict_stale(&mut self) {
        self.evict_stale_at(Instant::now());
    }

    /// Evict workers stale relative to an injected `now`.
    ///
    /// Used by tests to remove timing dependencies; production callers should
    /// use [`Self::evict_stale`].
    pub(crate) fn evict_stale_at(&mut self, now: Instant) {
        let cutoff = now - self.ttl;
        self.workers.retain(|_, info| info.last_seen > cutoff);
    }

    /// Find replay workers for a given server.
    pub fn replay_workers_for_server(&self, server_id: &str) -> Vec<&WorkerInfo> {
        self.workers
            .values()
            .filter(|w| {
                matches!(w.role, WorkerRoleInfo::Replay { .. })
                    && w.servers.contains(&server_id.to_string())
            })
            .collect()
    }

    /// Find storage workers for a given server.
    pub fn storage_workers_for_server(&self, server_id: &str) -> Vec<&WorkerInfo> {
        self.workers
            .values()
            .filter(|w| {
                matches!(w.role, WorkerRoleInfo::Storage { .. })
                    && w.servers.contains(&server_id.to_string())
            })
            .collect()
    }

    /// Pick a replay worker for a server. Returns None if unavailable.
    pub fn pick_replay(&self, server_id: &str) -> Option<&WorkerInfo> {
        self.replay_workers_for_server(server_id).into_iter().next()
    }

    /// Pick a storage worker for a server. Returns None if unavailable.
    pub fn pick_storage(&self, server_id: &str) -> Option<&WorkerInfo> {
        self.storage_workers_for_server(server_id)
            .into_iter()
            .next()
    }

    /// Total number of cached workers.
    pub fn len(&self) -> usize {
        self.workers.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    fn gen_id() -> EndpointId {
        Identity::generate().endpoint_id()
    }

    fn make_replay_announcement(peer_id: EndpointId, servers: Vec<&str>) -> WorkerAnnouncement {
        WorkerAnnouncement {
            peer_id,
            role: WorkerRoleInfo::Replay {
                servers_loaded: servers.len() as u32,
                events_buffered: 100,
                max_events: 1000,
                pending_count: 0,
            },
            servers: servers.into_iter().map(String::from).collect(),
            timestamp: 1000,
        }
    }

    fn make_storage_announcement(peer_id: EndpointId, servers: Vec<&str>) -> WorkerAnnouncement {
        WorkerAnnouncement {
            peer_id,
            role: WorkerRoleInfo::Storage {
                servers_tracked: servers.len() as u32,
                total_events_stored: 5000,
                disk_used_bytes: 1_000_000,
            },
            servers: servers.into_iter().map(String::from).collect(),
            timestamp: 1000,
        }
    }

    #[test]
    fn update_adds_worker() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement(gen_id(), vec!["srv-1"]));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn update_replaces_existing() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        let w1 = gen_id();
        cache.update(&make_replay_announcement(w1, vec!["srv-1"]));
        cache.update(&make_replay_announcement(w1, vec!["srv-1", "srv-2"]));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.replay_workers_for_server("srv-2").len(), 1);
    }

    #[test]
    fn remove_evicts_worker() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        let w1 = gen_id();
        cache.update(&make_replay_announcement(w1, vec!["srv-1"]));
        cache.remove(&w1);
        assert!(cache.is_empty());
    }

    #[test]
    fn evict_stale_removes_expired() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement(gen_id(), vec!["srv-1"]));
        // Simulate "now" being well past the TTL boundary instead of sleeping.
        let future = Instant::now() + Duration::from_secs(60);
        cache.evict_stale_at(future);
        assert!(cache.is_empty());
    }

    #[test]
    fn evict_stale_keeps_fresh() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement(gen_id(), vec!["srv-1"]));
        cache.evict_stale();
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn replay_workers_for_server_filters_by_role_and_server() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        let r1 = gen_id();
        cache.update(&make_replay_announcement(r1, vec!["srv-1"]));
        cache.update(&make_replay_announcement(gen_id(), vec!["srv-2"]));
        cache.update(&make_storage_announcement(gen_id(), vec!["srv-1"]));

        let workers = cache.replay_workers_for_server("srv-1");
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].peer_id, r1);
    }

    #[test]
    fn storage_workers_for_server_filters() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement(gen_id(), vec!["srv-1"]));
        cache.update(&make_storage_announcement(gen_id(), vec!["srv-1"]));
        cache.update(&make_storage_announcement(gen_id(), vec!["srv-1"]));

        assert_eq!(cache.storage_workers_for_server("srv-1").len(), 2);
    }

    #[test]
    fn pick_replay_returns_none_when_empty() {
        let cache = WorkerCache::new(Duration::from_secs(30));
        assert!(cache.pick_replay("srv-1").is_none());
    }

    #[test]
    fn pick_replay_returns_worker() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement(gen_id(), vec!["srv-1"]));
        assert!(cache.pick_replay("srv-1").is_some());
    }

    #[test]
    fn multiple_replay_workers_for_same_server() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement(gen_id(), vec!["srv-1"]));
        cache.update(&make_replay_announcement(gen_id(), vec!["srv-1"]));
        assert_eq!(cache.replay_workers_for_server("srv-1").len(), 2);
    }

    #[test]
    fn worker_serving_multiple_servers() {
        let mut cache = WorkerCache::new(Duration::from_secs(30));
        cache.update(&make_replay_announcement(
            gen_id(),
            vec!["srv-1", "srv-2", "srv-3"],
        ));

        assert_eq!(cache.replay_workers_for_server("srv-1").len(), 1);
        assert_eq!(cache.replay_workers_for_server("srv-2").len(), 1);
        assert_eq!(cache.replay_workers_for_server("srv-3").len(), 1);
        assert_eq!(cache.replay_workers_for_server("srv-4").len(), 0);
    }
}
