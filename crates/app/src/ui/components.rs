//! Bevy marker components for the Willow UI.

use bevy::prelude::*;

#[derive(Component)]
pub struct MessageList;

#[derive(Component)]
pub struct ChannelHeader;

#[derive(Component)]
pub struct PeerCount;

#[derive(Component)]
pub struct InputText;

/// Sidebar channel button. Stores the channel *name*.
#[derive(Component)]
pub struct ChannelButton(pub String);

/// Container for the channel button list so we can rebuild it dynamically.
#[derive(Component)]
pub struct ChannelList;

/// The main content area (chat + input OR settings panel).
#[derive(Component)]
pub struct MainContent;

/// Settings panel root.
#[derive(Component)]
pub struct SettingsPanel;

/// Chat panel root (channel header + messages + input).
#[derive(Component)]
pub struct ChatPanel;

/// Settings relay address text display.
#[derive(Component)]
pub struct SettingsRelayText;

/// Settings button in the sidebar.
#[derive(Component)]
pub struct SettingsButton;

/// Save button in settings.
#[derive(Component)]
pub struct SaveSettingsButton;

/// Display name text field in settings.
#[derive(Component)]
pub struct SettingsNameText;

/// The sidebar display of the local user's name.
#[derive(Component)]
pub struct LocalUserDisplay;

/// "Share File" button in the input area.
#[derive(Component)]
pub struct ShareFileButton;

/// Container for a settings input field. Stores which field it wraps.
#[derive(Component)]
pub struct SettingsFieldContainer(pub super::resources::SettingsField);

/// Delete channel button. Stores the channel name.
#[derive(Component)]
pub struct DeleteChannelButton(pub String);

/// Container for the member list in settings.
#[derive(Component)]
pub struct MemberList;

/// Kick button for a specific peer. Stores the peer ID string.
#[derive(Component)]
pub struct KickMemberButton(pub String);

/// "Copy PeerId" button in the user area.
#[derive(Component)]
pub struct CopyPeerIdButton;

/// "Copy" button for the invite code.
#[derive(Component)]
pub struct CopyInviteButton;

/// "+" button to create a new channel.
#[derive(Component)]
pub struct CreateChannelButton;

/// Text display for the new channel name input.
#[derive(Component)]
pub struct NewChannelInput;

/// "Generate Invite" button.
#[derive(Component)]
pub struct GenerateInviteButton;

/// Text display showing the generated invite code.
#[derive(Component)]
pub struct InviteCodeDisplay;

/// "Join Server" button (processes the join_code).
#[derive(Component)]
pub struct JoinServerButton;

/// Text input for pasting an invite code.
#[derive(Component)]
pub struct JoinCodeInput;
