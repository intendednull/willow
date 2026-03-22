//! # Willow Channel
//!
//! Servers, channels, roles, and permissions for the Willow P2P network.
//!
//! ## Data model
//!
//! Willow borrows Discord's organisational hierarchy:
//!
//! - A **[`Server`]** is a named community owned by a peer. It contains
//!   channels and members.
//! - A **[`Channel`]** is a named conversation space inside a server. Channels
//!   can be text or voice.
//! - A **[`Role`]** groups a set of [`Permission`]s under a name.
//! - **[`Member`]** tracks a peer's membership in a server, including which
//!   roles they hold.
//!
//! ## Invite system
//!
//! Servers are private by default. New members join via an [`Invite`] — a
//! signed token that grants the bearer permission to join. Invites can be
//! one-time-use or have an expiration date.
//!
//! ## Examples
//!
//! ```
//! use willow_channel::{Server, ChannelKind};
//! use willow_identity::Identity;
//!
//! let owner = Identity::generate();
//! let mut server = Server::new("My Server", owner.peer_id());
//!
//! server.create_channel("general", ChannelKind::Text).unwrap();
//! server.create_channel("voice-chat", ChannelKind::Voice).unwrap();
//!
//! assert_eq!(server.channels().len(), 2);
//! ```

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use willow_identity::PeerId;

// ───── IDs ───────────────────────────────────────────────────────────────────

/// Unique identifier for a server.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerId(pub Uuid);

impl ServerId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ServerId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ServerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a channel within a server.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub Uuid);

impl ChannelId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ChannelId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a role.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoleId(pub Uuid);

impl RoleId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RoleId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RoleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for an invite.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InviteId(pub Uuid);

impl InviteId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for InviteId {
    fn default() -> Self {
        Self::new()
    }
}

// ───── Errors ────────────────────────────────────────────────────────────────

/// Errors produced by channel operations.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// The caller does not have the required permission.
    #[error("permission denied: {0} requires {1:?}")]
    PermissionDenied(PeerId, Permission),

    /// A channel with this name already exists in the server.
    #[error("duplicate channel name: {0}")]
    DuplicateChannelName(String),

    /// The referenced channel was not found.
    #[error("channel not found: {0}")]
    ChannelNotFound(ChannelId),

    /// The referenced role was not found.
    #[error("role not found: {0}")]
    RoleNotFound(RoleId),

    /// The peer is not a member of this server.
    #[error("not a member: {0}")]
    NotAMember(PeerId),

    /// The invite has expired or already been used.
    #[error("invite expired or invalid")]
    InvalidInvite,
}

// ───── Permissions ───────────────────────────────────────────────────────────

/// Individual permissions that can be granted to roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Send messages in text channels.
    SendMessages,
    /// Read message history.
    ReadMessages,
    /// Delete other people's messages.
    ManageMessages,
    /// Create, rename, or delete channels.
    ManageChannels,
    /// Create, edit, or delete roles.
    ManageRoles,
    /// Kick members from the server.
    KickMembers,
    /// Ban members from the server.
    BanMembers,
    /// Create invite links.
    CreateInvite,
    /// Connect to voice channels.
    VoiceConnect,
    /// Speak in voice channels.
    VoiceSpeak,
    /// Share screen in voice channels.
    ScreenShare,
    /// Upload files.
    AttachFiles,
    /// Full administrative access — implies all other permissions.
    Administrator,
}

// ───── Role ──────────────────────────────────────────────────────────────────

/// A named bundle of [`Permission`]s that can be assigned to members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    /// Unique ID.
    pub id: RoleId,
    /// Human-readable name (e.g. "Moderator").
    pub name: String,
    /// The set of permissions this role grants.
    pub permissions: HashSet<Permission>,
    /// Display color as a hex string (e.g. "#FF5733").
    pub color: Option<String>,
}

impl Role {
    /// Create a new role with no permissions.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: RoleId::new(),
            name: name.into(),
            permissions: HashSet::new(),
            color: None,
        }
    }

    /// Check whether this role grants a specific permission.
    pub fn has_permission(&self, perm: Permission) -> bool {
        self.permissions.contains(&Permission::Administrator) || self.permissions.contains(&perm)
    }
}

// ───── Channel ───────────────────────────────────────────────────────────────

/// Whether a channel carries text or voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelKind {
    /// A text chat channel.
    Text,
    /// A voice (and optionally video/screenshare) channel.
    Voice,
}

/// A named conversation space inside a [`Server`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// Unique ID.
    pub id: ChannelId,
    /// Display name (e.g. "general").
    pub name: String,
    /// Optional description / topic.
    pub topic: Option<String>,
    /// Text or voice.
    pub kind: ChannelKind,
    /// When this channel was created.
    pub created_at: DateTime<Utc>,
}

impl Channel {
    /// Create a new channel.
    pub fn new(name: impl Into<String>, kind: ChannelKind) -> Self {
        Self {
            id: ChannelId::new(),
            name: name.into(),
            topic: None,
            kind,
            created_at: Utc::now(),
        }
    }
}

// ───── Member ────────────────────────────────────────────────────────────────

/// A peer's membership record within a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// The peer.
    pub peer_id: PeerId,
    /// Roles assigned to this member.
    pub roles: HashSet<RoleId>,
    /// When they joined.
    pub joined_at: DateTime<Utc>,
}

impl Member {
    /// Create a new member with no roles.
    pub fn new(peer_id: PeerId) -> Self {
        Self {
            peer_id,
            roles: HashSet::new(),
            joined_at: Utc::now(),
        }
    }
}

// ───── Invite ────────────────────────────────────────────────────────────────

/// A signed token granting the bearer permission to join a server.
///
/// When created with [`Server::create_invite_for`], the invite includes
/// encrypted channel keys so the new member can decrypt messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    /// Unique ID.
    pub id: InviteId,
    /// The server this invite is for.
    pub server_id: ServerId,
    /// Who created the invite.
    pub created_by: PeerId,
    /// When the invite was created.
    pub created_at: DateTime<Utc>,
    /// When the invite expires (if ever).
    pub expires_at: Option<DateTime<Utc>>,
    /// Maximum number of uses (None = unlimited).
    pub max_uses: Option<u32>,
    /// How many times this invite has been used.
    pub uses: u32,
    /// Encrypted channel keys for the recipient. Each entry wraps a
    /// channel's symmetric key using the recipient's X25519 public key.
    #[serde(default)]
    pub encrypted_keys: Vec<(ChannelId, willow_crypto::EncryptedChannelKey)>,
}

impl Invite {
    /// Check whether this invite is still valid.
    pub fn is_valid(&self) -> bool {
        if let Some(expires) = self.expires_at {
            if Utc::now() > expires {
                return false;
            }
        }
        if let Some(max) = self.max_uses {
            if self.uses >= max {
                return false;
            }
        }
        true
    }
}

// ───── Server ────────────────────────────────────────────────────────────────

/// A named community containing channels and members.
///
/// The server owner has implicit [`Permission::Administrator`] access to
/// everything. Other members' access is determined by their roles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    /// Unique ID.
    pub id: ServerId,
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// The peer who created and owns this server.
    pub owner: PeerId,
    /// When the server was created.
    pub created_at: DateTime<Utc>,

    channels: HashMap<ChannelId, Channel>,
    roles: HashMap<RoleId, Role>,
    members: HashMap<PeerId, Member>,
    invites: HashMap<InviteId, Invite>,

    /// Per-channel symmetric encryption keys. Not serialized — keys are
    /// distributed via invites and stored locally.
    #[serde(skip)]
    channel_keys: HashMap<ChannelId, willow_crypto::ChannelKey>,
}

impl Server {
    /// Create a new server. The owner is automatically added as a member.
    pub fn new(name: impl Into<String>, owner: PeerId) -> Self {
        let mut members = HashMap::new();
        members.insert(owner.clone(), Member::new(owner.clone()));

        Self {
            id: ServerId::new(),
            name: name.into(),
            description: None,
            owner,
            created_at: Utc::now(),
            channels: HashMap::new(),
            roles: HashMap::new(),
            members,
            invites: HashMap::new(),
            channel_keys: HashMap::new(),
        }
    }

    // ── Queries ──────────────────────────────────────────────────────────

    /// All channels in this server.
    pub fn channels(&self) -> Vec<&Channel> {
        self.channels.values().collect()
    }

    /// All roles defined in this server.
    pub fn roles(&self) -> Vec<&Role> {
        self.roles.values().collect()
    }

    /// All current members.
    pub fn members(&self) -> Vec<&Member> {
        self.members.values().collect()
    }

    /// Get a channel by ID.
    pub fn channel(&self, id: &ChannelId) -> Option<&Channel> {
        self.channels.get(id)
    }

    /// Check whether a peer is a member of this server.
    pub fn is_member(&self, peer: &PeerId) -> bool {
        self.members.contains_key(peer)
    }

    /// Check whether a peer has a specific permission.
    ///
    /// The server owner always has all permissions.
    pub fn has_permission(&self, peer: &PeerId, perm: Permission) -> bool {
        if *peer == self.owner {
            return true;
        }

        let member = match self.members.get(peer) {
            Some(m) => m,
            None => return false,
        };

        member.roles.iter().any(|role_id| {
            self.roles
                .get(role_id)
                .map(|r| r.has_permission(perm))
                .unwrap_or(false)
        })
    }

    // ── Mutations ────────────────────────────────────────────────────────

    /// Create a new channel in this server. Generates and stores a
    /// symmetric encryption key for the channel.
    pub fn create_channel(
        &mut self,
        name: impl Into<String>,
        kind: ChannelKind,
    ) -> Result<ChannelId, ChannelError> {
        let name = name.into();

        if self.channels.values().any(|ch| ch.name == name) {
            return Err(ChannelError::DuplicateChannelName(name));
        }

        let channel = Channel::new(name, kind);
        let id = channel.id.clone();
        self.channel_keys
            .insert(id.clone(), willow_crypto::generate_channel_key());
        self.channels.insert(id.clone(), channel);
        Ok(id)
    }

    /// Get the encryption key for a channel.
    pub fn channel_key(&self, id: &ChannelId) -> Option<&willow_crypto::ChannelKey> {
        self.channel_keys.get(id)
    }

    /// Store a channel key (e.g. after decrypting from an invite).
    pub fn set_channel_key(&mut self, id: ChannelId, key: willow_crypto::ChannelKey) {
        self.channel_keys.insert(id, key);
    }

    /// Delete a channel by ID.
    pub fn delete_channel(&mut self, id: &ChannelId) -> Result<(), ChannelError> {
        self.channels
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| ChannelError::ChannelNotFound(id.clone()))
    }

    /// Create a new role.
    pub fn create_role(&mut self, role: Role) -> RoleId {
        let id = role.id.clone();
        self.roles.insert(id.clone(), role);
        id
    }

    /// Assign a role to a member.
    pub fn assign_role(&mut self, peer: &PeerId, role_id: &RoleId) -> Result<(), ChannelError> {
        if !self.roles.contains_key(role_id) {
            return Err(ChannelError::RoleNotFound(role_id.clone()));
        }

        let member = self
            .members
            .get_mut(peer)
            .ok_or_else(|| ChannelError::NotAMember(peer.clone()))?;

        member.roles.insert(role_id.clone());
        Ok(())
    }

    /// Add a new member to the server.
    pub fn add_member(&mut self, peer: PeerId) {
        self.members
            .entry(peer.clone())
            .or_insert_with(|| Member::new(peer));
    }

    /// Remove a member from the server and rotate all channel keys.
    ///
    /// Cannot remove the owner. Returns the new channel keys so the app
    /// can distribute them to remaining members.
    pub fn remove_member(
        &mut self,
        peer: &PeerId,
    ) -> Result<HashMap<ChannelId, willow_crypto::ChannelKey>, ChannelError> {
        if *peer == self.owner {
            return Err(ChannelError::PermissionDenied(
                peer.clone(),
                Permission::Administrator,
            ));
        }

        self.members
            .remove(peer)
            .ok_or_else(|| ChannelError::NotAMember(peer.clone()))?;

        // Rotate all channel keys so the removed member can't read future messages.
        let mut new_keys = HashMap::new();
        for channel_id in self.channels.keys() {
            let new_key = willow_crypto::generate_channel_key();
            self.channel_keys
                .insert(channel_id.clone(), new_key.clone());
            new_keys.insert(channel_id.clone(), new_key);
        }

        Ok(new_keys)
    }

    /// Create an invite without encrypted keys (for backwards compat / tests).
    pub fn create_invite(
        &mut self,
        created_by: PeerId,
        expires_at: Option<DateTime<Utc>>,
        max_uses: Option<u32>,
    ) -> Result<InviteId, ChannelError> {
        if !self.is_member(&created_by) {
            return Err(ChannelError::NotAMember(created_by));
        }

        let invite = Invite {
            id: InviteId::new(),
            server_id: self.id.clone(),
            created_by,
            created_at: Utc::now(),
            expires_at,
            max_uses,
            uses: 0,
            encrypted_keys: Vec::new(),
        };

        let id = invite.id.clone();
        self.invites.insert(id.clone(), invite);
        Ok(id)
    }

    /// Create an invite with encrypted channel keys for a specific recipient.
    ///
    /// The `recipient_ed25519_public` is the recipient's 32-byte Ed25519
    /// public key. Each channel's symmetric key is encrypted using ephemeral
    /// X25519 DH so only the intended recipient can decrypt.
    pub fn create_invite_for(
        &mut self,
        created_by: PeerId,
        recipient_ed25519_public: &[u8; 32],
        expires_at: Option<DateTime<Utc>>,
        max_uses: Option<u32>,
    ) -> Result<InviteId, ChannelError> {
        if !self.is_member(&created_by) {
            return Err(ChannelError::NotAMember(created_by));
        }

        let mut encrypted_keys = Vec::new();
        for (channel_id, key) in &self.channel_keys {
            if let Ok(enc) = willow_crypto::encrypt_channel_key_for(key, recipient_ed25519_public) {
                encrypted_keys.push((channel_id.clone(), enc));
            }
        }

        let invite = Invite {
            id: InviteId::new(),
            server_id: self.id.clone(),
            created_by,
            created_at: Utc::now(),
            expires_at,
            max_uses,
            uses: 0,
            encrypted_keys,
        };

        let id = invite.id.clone();
        self.invites.insert(id.clone(), invite);
        Ok(id)
    }

    /// Get an invite by ID.
    pub fn invite(&self, id: &InviteId) -> Option<&Invite> {
        self.invites.get(id)
    }

    /// Use an invite to add a new member.
    pub fn use_invite(&mut self, invite_id: &InviteId, peer: PeerId) -> Result<(), ChannelError> {
        let invite = self
            .invites
            .get_mut(invite_id)
            .ok_or(ChannelError::InvalidInvite)?;

        if !invite.is_valid() {
            return Err(ChannelError::InvalidInvite);
        }

        invite.uses += 1;
        self.add_member(peer);
        Ok(())
    }
}

// ───── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use willow_identity::Identity;

    fn owner_and_server() -> (PeerId, Server) {
        let owner = Identity::generate().peer_id();
        let server = Server::new("Test Server", owner.clone());
        (owner, server)
    }

    #[test]
    fn new_server_has_owner_as_member() {
        let (owner, server) = owner_and_server();
        assert!(server.is_member(&owner));
        assert_eq!(server.members().len(), 1);
    }

    #[test]
    fn owner_has_all_permissions() {
        let (owner, server) = owner_and_server();
        assert!(server.has_permission(&owner, Permission::Administrator));
        assert!(server.has_permission(&owner, Permission::ManageChannels));
        assert!(server.has_permission(&owner, Permission::KickMembers));
    }

    #[test]
    fn non_member_has_no_permissions() {
        let (_, server) = owner_and_server();
        let stranger = Identity::generate().peer_id();
        assert!(!server.has_permission(&stranger, Permission::ReadMessages));
    }

    #[test]
    fn create_channels() {
        let (_, mut server) = owner_and_server();

        let text_id = server.create_channel("general", ChannelKind::Text).unwrap();
        let voice_id = server
            .create_channel("voice-chat", ChannelKind::Voice)
            .unwrap();

        assert_eq!(server.channels().len(), 2);
        assert_eq!(server.channel(&text_id).unwrap().kind, ChannelKind::Text);
        assert_eq!(server.channel(&voice_id).unwrap().kind, ChannelKind::Voice);
    }

    #[test]
    fn duplicate_channel_name_rejected() {
        let (_, mut server) = owner_and_server();
        server.create_channel("general", ChannelKind::Text).unwrap();

        let result = server.create_channel("general", ChannelKind::Text);
        assert!(matches!(result, Err(ChannelError::DuplicateChannelName(_))));
    }

    #[test]
    fn delete_channel() {
        let (_, mut server) = owner_and_server();
        let id = server.create_channel("temp", ChannelKind::Text).unwrap();

        server.delete_channel(&id).unwrap();
        assert!(server.channel(&id).is_none());
    }

    #[test]
    fn delete_nonexistent_channel_fails() {
        let (_, mut server) = owner_and_server();
        let fake_id = ChannelId::new();
        assert!(matches!(
            server.delete_channel(&fake_id),
            Err(ChannelError::ChannelNotFound(_))
        ));
    }

    #[test]
    fn roles_and_permissions() {
        let (_, mut server) = owner_and_server();
        let alice = Identity::generate().peer_id();
        server.add_member(alice.clone());

        // Alice starts with no permissions.
        assert!(!server.has_permission(&alice, Permission::ManageMessages));

        // Create a moderator role.
        let mut mod_role = Role::new("Moderator");
        mod_role.permissions.insert(Permission::ManageMessages);
        mod_role.permissions.insert(Permission::KickMembers);
        let role_id = server.create_role(mod_role);

        // Assign it to Alice.
        server.assign_role(&alice, &role_id).unwrap();

        assert!(server.has_permission(&alice, Permission::ManageMessages));
        assert!(server.has_permission(&alice, Permission::KickMembers));
        assert!(!server.has_permission(&alice, Permission::ManageRoles));
    }

    #[test]
    fn administrator_role_grants_everything() {
        let (_, mut server) = owner_and_server();
        let bob = Identity::generate().peer_id();
        server.add_member(bob.clone());

        let mut admin_role = Role::new("Admin");
        admin_role.permissions.insert(Permission::Administrator);
        let role_id = server.create_role(admin_role);
        server.assign_role(&bob, &role_id).unwrap();

        // Administrator implies every other permission.
        assert!(server.has_permission(&bob, Permission::ManageChannels));
        assert!(server.has_permission(&bob, Permission::BanMembers));
        assert!(server.has_permission(&bob, Permission::ScreenShare));
    }

    #[test]
    fn assign_role_to_non_member_fails() {
        let (_, mut server) = owner_and_server();
        let role = Role::new("Test");
        let role_id = server.create_role(role);

        let stranger = Identity::generate().peer_id();
        assert!(matches!(
            server.assign_role(&stranger, &role_id),
            Err(ChannelError::NotAMember(_))
        ));
    }

    #[test]
    fn remove_member() {
        let (_, mut server) = owner_and_server();
        let alice = Identity::generate().peer_id();
        server.add_member(alice.clone());

        server.remove_member(&alice).unwrap();
        assert!(!server.is_member(&alice));
    }

    #[test]
    fn cannot_remove_owner() {
        let (owner, mut server) = owner_and_server();
        assert!(server.remove_member(&owner).is_err());
    }

    #[test]
    fn invite_flow() {
        let (owner, mut server) = owner_and_server();

        // Owner creates an invite.
        let invite_id = server.create_invite(owner, None, Some(1)).unwrap();

        // A new peer uses it.
        let newcomer = Identity::generate().peer_id();
        server.use_invite(&invite_id, newcomer.clone()).unwrap();
        assert!(server.is_member(&newcomer));

        // Second use fails (max_uses = 1).
        let another = Identity::generate().peer_id();
        assert!(matches!(
            server.use_invite(&invite_id, another),
            Err(ChannelError::InvalidInvite)
        ));
    }

    #[test]
    fn expired_invite_rejected() {
        let (owner, mut server) = owner_and_server();

        // Create an invite that already expired.
        let past = Utc::now() - chrono::Duration::hours(1);
        let invite_id = server.create_invite(owner, Some(past), None).unwrap();

        let peer = Identity::generate().peer_id();
        assert!(matches!(
            server.use_invite(&invite_id, peer),
            Err(ChannelError::InvalidInvite)
        ));
    }

    #[test]
    fn server_serde_round_trip() {
        let (_, mut server) = owner_and_server();
        server.create_channel("general", ChannelKind::Text).unwrap();

        let bytes = willow_transport::pack(&server).unwrap();
        let decoded: Server = willow_transport::unpack(&bytes).unwrap();

        assert_eq!(decoded.name, server.name);
        assert_eq!(decoded.channels().len(), 1);
    }

    #[test]
    fn create_channel_generates_key() {
        let (_, mut server) = owner_and_server();
        let ch_id = server.create_channel("secret", ChannelKind::Text).unwrap();
        assert!(server.channel_key(&ch_id).is_some());
    }

    #[test]
    fn invite_with_encrypted_keys_round_trip() {
        let (owner, mut server) = owner_and_server();
        let ch_id = server.create_channel("general", ChannelKind::Text).unwrap();

        let newcomer = Identity::generate();
        let ed_kp = newcomer.keypair().clone().try_into_ed25519().unwrap();
        let full = ed_kp.to_bytes();
        let mut pub_bytes = [0u8; 32];
        pub_bytes.copy_from_slice(&full[32..]);

        let invite_id = server
            .create_invite_for(owner, &pub_bytes, None, Some(1))
            .unwrap();

        let invite = server.invite(&invite_id).unwrap();
        assert_eq!(invite.encrypted_keys.len(), 1);
        assert_eq!(invite.encrypted_keys[0].0, ch_id);

        // Newcomer can decrypt the channel key.
        let decrypted =
            willow_crypto::decrypt_channel_key(&invite.encrypted_keys[0].1, &newcomer).unwrap();

        // Verify it matches the original.
        let original = server.channel_key(&ch_id).unwrap();
        assert_eq!(decrypted.as_bytes(), original.as_bytes());
    }

    #[test]
    fn remove_member_rotates_channel_keys() {
        let (_, mut server) = owner_and_server();
        let ch_id = server.create_channel("general", ChannelKind::Text).unwrap();
        let original_key = server.channel_key(&ch_id).unwrap().as_bytes().to_owned();

        let alice = Identity::generate().peer_id();
        server.add_member(alice.clone());

        let new_keys = server.remove_member(&alice).unwrap();

        // Key should have changed.
        let rotated_key = server.channel_key(&ch_id).unwrap().as_bytes().to_owned();
        assert_ne!(original_key, rotated_key);

        // Returned keys should match what's stored.
        assert!(new_keys.contains_key(&ch_id));
        assert_eq!(new_keys[&ch_id].as_bytes(), &rotated_key);
    }

    #[test]
    fn remove_member_rotates_all_channels() {
        let (_, mut server) = owner_and_server();
        let ch1 = server.create_channel("general", ChannelKind::Text).unwrap();
        let ch2 = server.create_channel("random", ChannelKind::Text).unwrap();

        let alice = Identity::generate().peer_id();
        server.add_member(alice.clone());

        let new_keys = server.remove_member(&alice).unwrap();
        assert_eq!(new_keys.len(), 2);
        assert!(new_keys.contains_key(&ch1));
        assert!(new_keys.contains_key(&ch2));
    }
}
