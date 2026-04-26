//! Worker configuration and CLI argument parsing.

use crate::AllocationStrategy;

/// Configuration for a worker node.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Path to the Ed25519 identity keypair file.
    pub identity_path: String,
    /// Relay URL to connect through (iroh relay server).
    pub relay_url: Option<String>,
    /// Sync interval in seconds.
    pub sync_interval_secs: u64,
    /// Allocation strategy.
    pub allocation: AllocationStrategy,
}

#[cfg(any(test, feature = "test-utils"))]
impl WorkerConfig {
    /// Create a config for testing.
    pub fn test_config() -> Self {
        Self {
            identity_path: "/tmp/test-worker.key".to_string(),
            relay_url: None,
            sync_interval_secs: 30,
            allocation: AllocationStrategy::Global,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_has_defaults() {
        let cfg = WorkerConfig::test_config();
        assert_eq!(cfg.sync_interval_secs, 30);
        assert!(cfg.relay_url.is_none());
        assert!(matches!(cfg.allocation, AllocationStrategy::Global));
    }
}
