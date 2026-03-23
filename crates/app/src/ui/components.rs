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

/// Container for the role list in settings.
#[derive(Component)]
pub struct RoleList;

/// "Create Role" button.
#[derive(Component)]
pub struct CreateRoleButton;

/// Text input for new role name.
#[derive(Component)]
pub struct RoleNameInput;

/// Permission toggle button. Stores (RoleId string, Permission variant name).
#[derive(Component)]
pub struct TogglePermButton(pub String, pub String);

/// Button to assign a role to a member. Stores (peer_id, role_id).
#[derive(Component)]
pub struct AssignRoleButton(pub String, pub String);

/// Delete role button. Stores the RoleId string.
#[derive(Component)]
pub struct DeleteRoleButton(pub String);

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

/// Trust/Untrust button for a peer. Stores the peer ID string.
#[derive(Component)]
pub struct TrustMemberButton(pub String);

/// The clickable chat input area container.
#[derive(Component)]
pub struct ChatInputArea;

// ───── Reusable Input Field ─────────────────────────────────────────────────

/// Marker for the text child inside an input field.
#[derive(Component)]
pub struct InputFieldText;

/// Configuration bundle for spawning a clickable input field.
/// Use [`spawn_input_field`] to create one with consistent styling.
pub struct InputFieldConfig<'a> {
    /// Initial display text.
    pub value: &'a str,
    /// Placeholder shown when empty.
    pub placeholder: &'a str,
    /// Font size (default 13.0).
    pub font_size: f32,
    /// Whether the field fills available width vs grows.
    pub full_width: bool,
}

impl<'a> Default for InputFieldConfig<'a> {
    fn default() -> Self {
        Self {
            value: "",
            placeholder: "",
            font_size: 13.0,
            full_width: true,
        }
    }
}

/// Spawn a clickable input field with consistent styling.
///
/// The field gets `Button` (for click detection), styled `Node`, `BackgroundColor`,
/// `BorderColor`, and a text child. Pass additional marker components via the
/// returned `EntityCommands`.
///
/// ```ignore
/// spawn_input_field(parent, &config)
///     .insert(MyMarkerComponent)
///     .with_children(|_| {});
/// ```
/// Spawn a clickable input field. Returns `EntityCommands` for the container
/// so callers can `.insert(MyMarker)`. Pass `text_marker` to add an extra
/// component to the text child (e.g. `InputText` for the chat input).
pub fn spawn_input_field<'a>(
    parent: &'a mut bevy::prelude::ChildSpawnerCommands,
    config: &InputFieldConfig,
    text_marker: Option<impl bevy::prelude::Component>,
) -> bevy::prelude::EntityCommands<'a> {
    use crate::theme;
    use bevy::prelude::*;

    let (display, color) = if config.value.is_empty() {
        (config.placeholder, theme::TEXT_PLACEHOLDER)
    } else {
        (config.value, theme::TEXT_PRIMARY)
    };
    let font_size = config.font_size;

    let mut entity = parent.spawn((
        Button,
        Node {
            width: if config.full_width {
                Val::Percent(100.0)
            } else {
                Val::Auto
            },
            flex_grow: if config.full_width { 0.0 } else { 1.0 },
            min_height: Val::Px(36.0),
            padding: UiRect::horizontal(Val::Px(12.0)),
            align_items: AlignItems::Center,
            margin: UiRect::vertical(Val::Px(4.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(theme::INPUT_FIELD_BG),
        BorderColor::all(Color::NONE),
    ));
    entity.with_children(move |field| {
        let mut text_entity = field.spawn((
            Text::new(display),
            TextFont::from_font_size(font_size),
            TextColor(color),
            InputFieldText,
        ));
        if let Some(marker) = text_marker {
            text_entity.insert(marker);
        }
    });
    entity
}
