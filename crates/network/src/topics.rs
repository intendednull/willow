//! # Topic ID Registry
//!
//! Deterministic topic ID generation using BLAKE3 hashing.
//! All gossip topics are identified by a 32-byte [`TopicId`] derived from
//! a human-readable string name.

use iroh_gossip::TopicId;

/// Derive a deterministic [`TopicId`] from a topic name string.
///
/// Uses BLAKE3 to hash the name into a 32-byte identifier.
pub fn topic_id(name: &str) -> TopicId {
    let hash = blake3::hash(name.as_bytes());
    TopicId::from_bytes(*hash.as_bytes())
}

/// Topic for server state operations (channels, roles, permissions, kicks).
pub fn server_ops_topic() -> TopicId {
    topic_id("_willow_server_ops")
}

/// Topic for worker announcements and coordination.
pub fn workers_topic() -> TopicId {
    topic_id("_willow_workers")
}

/// Topic for user profile broadcasts.
pub fn profiles_topic() -> TopicId {
    topic_id("_willow_profiles")
}

use std::sync::LazyLock;

/// Topic for server state operations.
pub static SERVER_OPS_TOPIC: LazyLock<TopicId> = LazyLock::new(server_ops_topic);
/// Topic for worker announcements.
pub static WORKERS_TOPIC: LazyLock<TopicId> = LazyLock::new(workers_topic);
/// Topic for user profile broadcasts.
pub static PROFILES_TOPIC: LazyLock<TopicId> = LazyLock::new(profiles_topic);

/// Derive the gossip topic for a specific channel in a server.
pub fn channel_topic(server_id: &str, channel_id: &str) -> TopicId {
    topic_id(&format!("{server_id}/{channel_id}"))
}

/// Derive the gossip topic for voice signaling in a channel.
pub fn voice_topic(server_id: &str, channel_id: &str) -> TopicId {
    topic_id(&format!("{server_id}/{channel_id}/voice"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_id_is_deterministic() {
        let a = topic_id("test-topic");
        let b = topic_id("test-topic");
        assert_eq!(a, b);
    }

    #[test]
    fn different_names_produce_different_ids() {
        let a = topic_id("topic-a");
        let b = topic_id("topic-b");
        assert_ne!(a, b);
    }

    #[test]
    fn system_topics_are_distinct() {
        assert_ne!(*SERVER_OPS_TOPIC, *WORKERS_TOPIC);
        assert_ne!(*SERVER_OPS_TOPIC, *PROFILES_TOPIC);
        assert_ne!(*WORKERS_TOPIC, *PROFILES_TOPIC);
    }

    #[test]
    fn channel_and_voice_topics_differ() {
        let ch = channel_topic("server1", "general");
        let vc = voice_topic("server1", "general");
        assert_ne!(ch, vc);
    }

    #[test]
    fn static_topic_matches_runtime() {
        assert_eq!(*SERVER_OPS_TOPIC, topic_id("_willow_server_ops"));
        assert_eq!(*WORKERS_TOPIC, topic_id("_willow_workers"));
        assert_eq!(*PROFILES_TOPIC, topic_id("_willow_profiles"));
    }
}
