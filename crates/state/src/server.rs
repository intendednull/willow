//! Server state — the complete shared state of a server.
//!
//! [`ServerState`] holds all channels, roles, members, messages, trust
//! information, and profiles. It is fully derivable from an ordered sequence
//! of events via [`apply`](crate::apply).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::hash::StateHash;
use crate::types::{Channel, ChatMessage, Member, Permission, Profile, Role};

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
    /// Per-peer permissions. Owner always has all permissions implicitly.
    pub peer_permissions: HashMap<String, HashSet<Permission>>,
    /// Chat messages in event-sequence order.
    pub messages: Vec<ChatMessage>,
    /// Peer profiles keyed by peer ID.
    pub profiles: HashMap<String, Profile>,
    /// Server description (default empty string).
    pub description: String,
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
            description: &'a str,
            channels: Vec<(&'a String, &'a Channel)>,
            roles: Vec<(&'a String, &'a Role)>,
            members: Vec<(&'a String, &'a Member)>,
            peer_permissions: Vec<(&'a String, Vec<&'a Permission>)>,
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

        let mut peer_permissions: Vec<_> = self
            .peer_permissions
            .iter()
            .map(|(k, v)| {
                let mut perms: Vec<_> = v.iter().collect();
                perms.sort_by_key(|p| format!("{p:?}"));
                (k, perms)
            })
            .collect();
        peer_permissions.sort_by_key(|(k, _)| (*k).clone());

        let mut profiles: Vec<_> = self.profiles.iter().collect();
        profiles.sort_by_key(|(k, _)| *k);

        let mut channel_keys: Vec<_> = self.channel_keys.iter().collect();
        channel_keys.sort_by_key(|(k, _)| *k);

        let hashable = Hashable {
            server_id: &self.server_id,
            server_name: &self.server_name,
            owner: &self.owner,
            description: &self.description,
            channels,
            roles,
            members,
            peer_permissions,
            messages: &self.messages,
            profiles,
            channel_keys,
        };

        // Use bincode for canonical serialization — it is deterministic for
        // the same input.
        let bytes = bincode::serialize(&hashable).expect("state serialization should not fail");
        StateHash::from_bytes(&bytes)
    }

    /// Check whether a peer has a specific permission.
    ///
    /// The owner always has all permissions. Non-owner peers with
    /// [`Administrator`](Permission::Administrator) also have all permissions.
    pub fn has_permission(&self, peer_id: &str, perm: &Permission) -> bool {
        if peer_id == self.owner {
            return true; // owner has all permissions
        }
        self.peer_permissions
            .get(peer_id)
            .map(|perms| perms.contains(&Permission::Administrator) || perms.contains(perm))
            .unwrap_or(false)
    }

    /// Check if a peer can provide sync (trusted for history).
    pub fn is_sync_provider(&self, peer_id: &str) -> bool {
        self.has_permission(peer_id, &Permission::SyncProvider)
    }

    /// Check whether a peer is trusted (owner is always trusted).
    ///
    /// A peer is considered trusted if it has any permissions granted.
    /// This is a backward-compatible bridge for code that uses the old
    /// binary trust model.
    pub fn is_trusted(&self, peer_id: &str) -> bool {
        if peer_id == self.owner {
            return true;
        }
        self.peer_permissions
            .get(peer_id)
            .map(|perms| !perms.is_empty())
            .unwrap_or(false)
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

    #[test]
    fn owner_has_all_permissions() {
        let state = ServerState::new("s1", "Test", "owner");
        assert!(state.has_permission("owner", &Permission::ManageChannels));
        assert!(state.has_permission("owner", &Permission::Administrator));
        assert!(state.has_permission("owner", &Permission::SyncProvider));
    }

    #[test]
    fn peer_without_permissions() {
        let state = ServerState::new("s1", "Test", "owner");
        assert!(!state.has_permission("stranger", &Permission::ManageChannels));
        assert!(!state.is_sync_provider("stranger"));
    }

    #[test]
    fn hash_changes_with_permissions() {
        let a = ServerState::new("s1", "Test", "owner");
        let mut b = ServerState::new("s1", "Test", "owner");
        let mut perms = std::collections::HashSet::new();
        perms.insert(Permission::ManageChannels);
        b.peer_permissions.insert("alice".into(), perms);
        assert_ne!(a.hash(), b.hash());
    }
}
