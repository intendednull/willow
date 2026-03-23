//! Server state — the complete shared state of a server.
//!
//! [`ServerState`] holds all channels, roles, members, messages, trust
//! information, and profiles. It is fully derivable from an ordered sequence
//! of events via [`apply`](crate::apply).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::hash::StateHash;
use crate::types::{Channel, ChatMessage, Member, Profile, Role};

/// The complete shared state of a server, derivable from events.
///
/// All fields except `seen_event_ids` participate in the state hash.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ServerState {
    /// Unique server ID.
    pub server_id: String,
    /// Display name.
    pub server_name: String,
    /// The peer who owns this server (always trusted).
    pub owner: String,
    /// Channels keyed by channel ID.
    pub channels: HashMap<String, Channel>,
    /// Roles keyed by role ID.
    pub roles: HashMap<String, Role>,
    /// Members keyed by peer ID.
    pub members: HashMap<String, Member>,
    /// Explicitly trusted peer IDs (the owner is implicitly trusted).
    pub trusted_peers: HashSet<String>,
    /// Chat messages in event-sequence order.
    pub messages: Vec<ChatMessage>,
    /// Peer profiles keyed by peer ID.
    pub profiles: HashMap<String, Profile>,
    /// Encrypted channel key material (opaque bytes, keyed by channel ID).
    pub channel_keys: HashMap<String, Vec<u8>>,
    /// Set of seen event IDs for deduplication.
    /// Excluded from hash computation (dedup metadata, not state).
    #[serde(skip)]
    pub seen_event_ids: HashSet<String>,
}

impl ServerState {
    /// Create a new server state with the given ID, name, and owner.
    ///
    /// The owner is automatically added as a member.
    pub fn new(id: impl Into<String>, name: impl Into<String>, owner: impl Into<String>) -> Self {
        let owner = owner.into();
        let mut members = HashMap::new();
        members.insert(
            owner.clone(),
            Member {
                peer_id: owner.clone(),
                roles: HashSet::new(),
                display_name: None,
            },
        );

        Self {
            server_id: id.into(),
            server_name: name.into(),
            owner,
            members,
            ..Default::default()
        }
    }

    /// Compute the SHA-256 hash of this state.
    ///
    /// The `seen_event_ids` field is excluded (it is dedup metadata, not
    /// application state). All other fields are serialized canonically with
    /// bincode and then hashed.
    pub fn hash(&self) -> StateHash {
        // Build a hashable view that excludes seen_event_ids.
        // We serialize the meaningful fields in a fixed order.
        #[derive(Serialize)]
        struct Hashable<'a> {
            server_id: &'a str,
            server_name: &'a str,
            owner: &'a str,
            channels: Vec<(&'a String, &'a Channel)>,
            roles: Vec<(&'a String, &'a Role)>,
            members: Vec<(&'a String, &'a Member)>,
            trusted_peers: Vec<&'a String>,
            messages: &'a [ChatMessage],
            profiles: Vec<(&'a String, &'a Profile)>,
            channel_keys: Vec<(&'a String, &'a Vec<u8>)>,
        }

        let mut channels: Vec<_> = self.channels.iter().collect();
        channels.sort_by_key(|(k, _)| *k);

        let mut roles: Vec<_> = self.roles.iter().collect();
        roles.sort_by_key(|(k, _)| *k);

        let mut members: Vec<_> = self.members.iter().collect();
        members.sort_by_key(|(k, _)| *k);

        let mut trusted_peers: Vec<_> = self.trusted_peers.iter().collect();
        trusted_peers.sort();

        let mut profiles: Vec<_> = self.profiles.iter().collect();
        profiles.sort_by_key(|(k, _)| *k);

        let mut channel_keys: Vec<_> = self.channel_keys.iter().collect();
        channel_keys.sort_by_key(|(k, _)| *k);

        let hashable = Hashable {
            server_id: &self.server_id,
            server_name: &self.server_name,
            owner: &self.owner,
            channels,
            roles,
            members,
            trusted_peers,
            messages: &self.messages,
            profiles,
            channel_keys,
        };

        // Use bincode for canonical serialization — it is deterministic for
        // the same input.
        let bytes = bincode::serialize(&hashable).expect("state serialization should not fail");
        StateHash::from_bytes(&bytes)
    }

    /// Check whether a peer is trusted (owner is always trusted).
    pub fn is_trusted(&self, peer_id: &str) -> bool {
        peer_id == self.owner || self.trusted_peers.contains(peer_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_server_has_owner_as_member() {
        let state = ServerState::new("s1", "Test Server", "owner-peer");
        assert!(state.members.contains_key("owner-peer"));
        assert_eq!(state.members.len(), 1);
    }

    #[test]
    fn owner_is_always_trusted() {
        let state = ServerState::new("s1", "Test", "owner");
        assert!(state.is_trusted("owner"));
        assert!(!state.is_trusted("stranger"));
    }

    #[test]
    fn hash_is_deterministic() {
        let a = ServerState::new("s1", "Test", "owner");
        let b = ServerState::new("s1", "Test", "owner");
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn hash_changes_with_state() {
        let a = ServerState::new("s1", "Test", "owner");
        let mut b = ServerState::new("s1", "Test", "owner");
        b.channels.insert(
            "ch1".into(),
            Channel {
                id: "ch1".into(),
                name: "general".into(),
            },
        );
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn seen_event_ids_excluded_from_hash() {
        let mut a = ServerState::new("s1", "Test", "owner");
        let b = ServerState::new("s1", "Test", "owner");
        a.seen_event_ids.insert("evt-1".into());
        assert_eq!(a.hash(), b.hash());
    }
}
