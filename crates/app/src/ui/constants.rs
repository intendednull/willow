//! Shared UI constants — placeholder strings, default values.
//!
//! Centralizing these prevents duplicate string literals across layout
//! and system code.

/// Default channel name on first launch.
pub const DEFAULT_CHANNEL: &str = "general";

/// Placeholder for the chat input field.
pub const CHAT_PLACEHOLDER: &str = "Type a message...";

/// Placeholder for the display name field in settings.
pub const NAME_PLACEHOLDER: &str = "Enter your name...";

/// Placeholder for the relay address field in settings.
pub const RELAY_PLACEHOLDER: &str = "/ip4/.../tcp/9091/ws/p2p/12D3KooW...";

/// Example relay address shown as hint text.
pub const RELAY_EXAMPLE: &str = "Example: /ip4/1.2.3.4/tcp/9091/ws/p2p/12D3KooW...";
