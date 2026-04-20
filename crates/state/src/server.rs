//! Server state — the materialized view of a server's DAG.
//!
//! [`ServerState`] holds all channels, roles, members, messages, admin set,
//! governance state, and profiles. It is derived from a [`EventDag`](crate::dag::EventDag)
//! via [`materialize`](crate::materialize::materialize).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use willow_identity::EndpointId;

use crate::event::{Permission, ProposedAction, VoteThreshold};
use crate::hash::EventHash;
use crate::types::{Channel, ChatMessage, Member, MuteState, Profile, Role};

/// A proposal awaiting admin votes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingProposal {
    /// The action being proposed.
    pub action: ProposedAction,
    /// Who proposed it.
    pub proposer: EndpointId,
    /// Votes received: voter -> accept/reject.
    pub votes: BTreeMap<EndpointId, bool>,
}

/// The complete materialized state of a server.
///
/// All fields except governance state (`admins`, `vote_threshold`,
/// `pending_proposals`) are standard application state. The governance
/// fields manage admin membership via a vote-based process.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerState {
    /// Unique server ID (hex of genesis event hash).
    pub server_id: String,
    /// Display name (from genesis CreateServer, mutable via RenameServer).
    pub server_name: String,
    /// Channels keyed by channel ID.
    pub channels: BTreeMap<String, Channel>,
    /// Roles keyed by role ID.
    pub roles: BTreeMap<String, Role>,
    /// Members keyed by peer ID.
    pub members: BTreeMap<EndpointId, Member>,
    /// Non-admin permissions per peer (ManageChannels, SendMessages, etc.).
    /// Does not control admin status — that's in `admins`.
    pub peer_permissions: BTreeMap<EndpointId, BTreeSet<Permission>>,
    /// Chat messages in event-sequence order.
    pub messages: Vec<ChatMessage>,
    /// Peer profiles keyed by peer ID.
    pub profiles: BTreeMap<EndpointId, Profile>,
    /// Server description.
    pub description: String,
    /// Encrypted channel key material: channel ID → (recipient → encrypted key).
    /// Each recipient gets their own encrypted copy of the channel key.
    pub channel_keys: BTreeMap<String, BTreeMap<EndpointId, Vec<u8>>>,
    /// Per-identity mute state (phase 1f). Each entry is the *author's
    /// own* notification preference — never advertised to peers. Absent
    /// means default (no mute). Defaulted via `#[serde(default)]` so
    /// pre-1f serialized states load cleanly.
    #[serde(default)]
    pub mute_state: BTreeMap<EndpointId, MuteState>,

    // -- Governance state --
    /// The peer who created the server (author of the genesis event).
    /// This is the permanent server owner and never changes, unlike `admins`
    /// which can grow when admins are promoted via the governance process.
    /// Stored as `Option` for backwards compatibility with serialized state;
    /// always `Some` for servers created with this version.
    #[serde(default)]
    pub genesis_author: Option<EndpointId>,
    /// The set of peers with admin status. Separate from Permission
    /// enum to make the governance boundary structurally enforced.
    pub admins: BTreeSet<EndpointId>,
    /// Current vote threshold for admin actions.
    pub vote_threshold: VoteThreshold,
    /// Pending proposals awaiting votes.
    pub pending_proposals: BTreeMap<EventHash, PendingProposal>,

    // -- Dedup state --
    /// Hashes of events already applied to this state. Used by
    /// [`apply_incremental`](crate::materialize::apply_incremental) to
    /// guarantee idempotency — applying the same event twice is a no-op.
    #[serde(default, skip)]
    pub applied_events: BTreeSet<EventHash>,

    // -- Fast-lookup indexes --
    /// Maps each message's [`EventHash`] to its index in [`messages`].
    ///
    /// Kept in sync with every insertion into `messages` so that
    /// `EditMessage`, `DeleteMessage`, and `Reaction` handlers can find
    /// their target in O(1) instead of O(N).
    ///
    /// This field is excluded from serialization (`#[serde(skip)]`). It is
    /// always empty on a freshly deserialized `ServerState`. To protect
    /// against silent mutation loss when a caller deserializes state and
    /// then applies incremental `EditMessage`/`DeleteMessage`/`Reaction`
    /// events, [`apply_incremental`](crate::materialize::apply_incremental)
    /// lazily rebuilds the index on first use via
    /// [`rebuild_message_index`](Self::rebuild_message_index). Callers that
    /// load state from disk may also invoke `rebuild_message_index` eagerly
    /// to amortize the O(N) cost up front.
    #[serde(default, skip)]
    pub message_index: HashMap<EventHash, usize>,
}

impl ServerState {
    /// Create a new server state.
    ///
    /// The genesis author is added as both a member and the sole admin.
    pub fn new(id: impl Into<String>, name: impl Into<String>, genesis_author: EndpointId) -> Self {
        let mut members = BTreeMap::new();
        members.insert(
            genesis_author,
            Member {
                peer_id: genesis_author,
                roles: BTreeSet::new(),
                display_name: None,
            },
        );

        let mut admins = BTreeSet::new();
        admins.insert(genesis_author);

        Self {
            server_id: id.into(),
            server_name: name.into(),
            genesis_author: Some(genesis_author),
            members,
            admins,
            vote_threshold: VoteThreshold::default(),
            channels: BTreeMap::new(),
            roles: BTreeMap::new(),
            peer_permissions: BTreeMap::new(),
            messages: Vec::new(),
            profiles: BTreeMap::new(),
            description: String::new(),
            channel_keys: BTreeMap::new(),
            mute_state: BTreeMap::new(),
            pending_proposals: BTreeMap::new(),
            applied_events: BTreeSet::new(),
            message_index: HashMap::new(),
        }
    }

    /// Check if a peer is an admin.
    pub fn is_admin(&self, peer_id: &EndpointId) -> bool {
        self.admins.contains(peer_id)
    }

    /// Check if a peer has a specific non-admin permission.
    ///
    /// Admins implicitly have all permissions.
    pub fn has_permission(&self, peer_id: &EndpointId, perm: &Permission) -> bool {
        if self.admins.contains(peer_id) {
            return true;
        }
        self.peer_permissions
            .get(peer_id)
            .map(|perms| perms.contains(perm))
            .unwrap_or(false)
    }

    /// Check if a peer can provide sync (trusted for history).
    pub fn is_sync_provider(&self, peer_id: &EndpointId) -> bool {
        self.has_permission(peer_id, &Permission::SyncProvider)
    }

    /// Rebuild [`message_index`](Self::message_index) from `messages`.
    ///
    /// This is O(N) in the number of messages. It exists because
    /// `message_index` is `#[serde(skip)]` and is therefore empty on a
    /// freshly deserialized `ServerState`. Without rebuilding, the
    /// `EditMessage`, `DeleteMessage`, and `Reaction` handlers would
    /// silently no-op against the stale (empty) index — a silent
    /// data-loss bug on persisted clients.
    ///
    /// Callers that deserialize state from disk should either invoke this
    /// method eagerly after load, or rely on the lazy rebuild performed
    /// automatically by
    /// [`apply_incremental`](crate::materialize::apply_incremental).
    pub fn rebuild_message_index(&mut self) {
        self.message_index = self
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
    }

    /// Check if a yes-vote count meets the current threshold.
    pub fn meets_threshold(&self, yes_count: usize) -> bool {
        let admin_count = self.admins.len();
        if admin_count == 0 {
            return false;
        }
        match self.vote_threshold {
            VoteThreshold::Majority => yes_count > admin_count / 2,
            VoteThreshold::Unanimous => yes_count >= admin_count,
            VoteThreshold::Count(n) => yes_count >= (n as usize).min(admin_count),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    fn gen_id() -> EndpointId {
        Identity::generate().endpoint_id()
    }

    #[test]
    fn new_server_has_genesis_author_as_admin() {
        let author = gen_id();
        let state = ServerState::new("s1", "Test", author);
        assert!(state.admins.contains(&author));
        assert!(state.is_admin(&author));
        assert!(state.members.contains_key(&author));
        assert_eq!(state.admins.len(), 1);
    }

    #[test]
    fn admin_has_all_permissions() {
        let admin = gen_id();
        let state = ServerState::new("s1", "Test", admin);
        assert!(state.has_permission(&admin, &Permission::ManageChannels));
        assert!(state.has_permission(&admin, &Permission::ManageRoles));
        assert!(state.has_permission(&admin, &Permission::SendMessages));
        assert!(state.has_permission(&admin, &Permission::SyncProvider));
        assert!(state.has_permission(&admin, &Permission::CreateInvite));
    }

    #[test]
    fn peer_without_permissions() {
        let admin = gen_id();
        let stranger = gen_id();
        let state = ServerState::new("s1", "Test", admin);
        assert!(!state.is_admin(&stranger));
        assert!(!state.has_permission(&stranger, &Permission::ManageChannels));
        assert!(!state.is_sync_provider(&stranger));
    }

    #[test]
    fn meets_threshold_majority() {
        let admin = gen_id();
        let mut state = ServerState::new("s1", "Test", admin);
        // 1 admin, majority of 1 = need > 0.5 = need 1.
        assert!(state.meets_threshold(1));
        assert!(!state.meets_threshold(0));

        // 3 admins, majority of 3 = need > 1.5 = need 2.
        state.admins.insert(gen_id());
        state.admins.insert(gen_id());
        assert!(!state.meets_threshold(1));
        assert!(state.meets_threshold(2));
        assert!(state.meets_threshold(3));
    }

    #[test]
    fn meets_threshold_unanimous() {
        let admin = gen_id();
        let mut state = ServerState::new("s1", "Test", admin);
        state.vote_threshold = VoteThreshold::Unanimous;
        state.admins.insert(gen_id());
        state.admins.insert(gen_id());
        // 3 admins, unanimous = need 3.
        assert!(!state.meets_threshold(2));
        assert!(state.meets_threshold(3));
    }

    #[test]
    fn meets_threshold_count() {
        let admin = gen_id();
        let mut state = ServerState::new("s1", "Test", admin);
        state.vote_threshold = VoteThreshold::Count(2);
        state.admins.insert(gen_id());
        state.admins.insert(gen_id());
        // 3 admins, Count(2) = need 2.
        assert!(!state.meets_threshold(1));
        assert!(state.meets_threshold(2));

        // Count(10) with 3 admins = capped at 3.
        state.vote_threshold = VoteThreshold::Count(10);
        assert!(!state.meets_threshold(2));
        assert!(state.meets_threshold(3));
    }

    #[test]
    fn meets_threshold_zero_admins() {
        let admin = gen_id();
        let mut state = ServerState::new("s1", "Test", admin);
        state.admins.clear();
        assert!(!state.meets_threshold(0));
        assert!(!state.meets_threshold(1));
    }
}
