//! # Theme
//!
//! Discord-inspired dark color palette for the Willow UI.

use bevy::prelude::Color;

// ───── Background Colors ────────────────────────────────────────────────────

/// Sidebar background (#1e1f22).
pub const SIDEBAR_BG: Color = Color::srgb(0.118, 0.122, 0.133);

/// Main content area background (#313338).
pub const MAIN_BG: Color = Color::srgb(0.192, 0.200, 0.220);

/// Input area / header background (#2b2d31).
pub const INPUT_BG: Color = Color::srgb(0.169, 0.176, 0.192);

/// Input field background (#383a40).
pub const INPUT_FIELD_BG: Color = Color::srgb(0.220, 0.227, 0.251);

/// Channel header divider (#27292d).
pub const DIVIDER: Color = Color::srgb(0.153, 0.161, 0.176);

/// Hover / active channel background (#35373c).
pub const CHANNEL_ACTIVE_BG: Color = Color::srgb(0.208, 0.216, 0.235);

/// Button hover (#3e4046).
pub const BUTTON_HOVER: Color = Color::srgb(0.243, 0.251, 0.275);

/// User area background (#232428).
pub const USER_AREA_BG: Color = Color::srgb(0.137, 0.141, 0.157);

// ───── Text Colors ──────────────────────────────────────────────────────────

/// Primary text (white-ish, #f2f3f5).
pub const TEXT_PRIMARY: Color = Color::srgb(0.949, 0.953, 0.961);

/// Secondary text (dimmer, #b5bac1).
pub const TEXT_SECONDARY: Color = Color::srgb(0.710, 0.729, 0.757);

/// Muted text (channel names, timestamps, #949ba4).
pub const TEXT_MUTED: Color = Color::srgb(0.580, 0.608, 0.643);

/// Placeholder text (#5c6068).
pub const TEXT_PLACEHOLDER: Color = Color::srgb(0.361, 0.376, 0.408);

/// Section header text (CHANNELS label, #96989d).
pub const TEXT_HEADER: Color = Color::srgb(0.588, 0.596, 0.616);

/// Local message author (blue-ish, #5865f2).
pub const AUTHOR_LOCAL: Color = Color::srgb(0.345, 0.396, 0.949);

/// Remote message author (warm, #f0b232).
pub const AUTHOR_REMOTE: Color = Color::srgb(0.941, 0.698, 0.196);

/// Unread channel highlight (#f0b232).
pub const UNREAD_HIGHLIGHT: Color = Color::srgb(0.941, 0.698, 0.196);

/// Online / connected indicator (#23a55a).
pub const STATUS_ONLINE: Color = Color::srgb(0.137, 0.647, 0.353);

// ───── Accent Colors ────────────────────────────────────────────────────────

/// Primary accent / blurple (#5865f2).
pub const ACCENT: Color = Color::srgb(0.345, 0.396, 0.949);

/// Accent hover (#4752c4).
pub const ACCENT_HOVER: Color = Color::srgb(0.278, 0.322, 0.769);

/// Danger / error (#ed4245).
pub const DANGER: Color = Color::srgb(0.929, 0.259, 0.271);
