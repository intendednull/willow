//! # Token Scoping
//!
//! Defines `TokenScope` variants that control which MCP tools and resources
//! a bearer token grants access to.

use std::collections::HashSet;

/// Scope of a bearer token, controlling tool and resource access.
#[derive(Debug, Clone, Default)]
pub enum TokenScope {
    /// All tools, all resources.
    #[default]
    Full,
    /// No tools, all resources.
    ReadOnly,
    /// Messaging tools only, all resources.
    Messaging,
    /// All tools, all resources (semantically distinct from Full for audit).
    Admin,
    /// Explicit allowlist of tool names.
    Custom(HashSet<String>),
}

/// Tools available in the Messaging scope.
const MESSAGING_TOOLS: &[&str] = &[
    "send_message",
    "send_reply",
    "edit_message",
    "delete_message",
    "react",
    "pin_message",
    "unpin_message",
    "send_typing",
];

impl TokenScope {
    /// Returns true if the given tool name is allowed by this scope.
    pub fn allows_tool(&self, tool_name: &str) -> bool {
        match self {
            Self::Full | Self::Admin => true,
            Self::ReadOnly => false,
            Self::Messaging => MESSAGING_TOOLS.contains(&tool_name),
            Self::Custom(set) => set.contains(tool_name),
        }
    }

    /// Returns true if the given resource URI is allowed.
    pub fn allows_resource(&self, _uri: &str) -> bool {
        // All scopes allow all resources.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_allows_everything() {
        let scope = TokenScope::Full;
        assert!(scope.allows_tool("send_message"));
        assert!(scope.allows_tool("create_channel"));
        assert!(scope.allows_tool("kick_member"));
        assert!(scope.allows_resource("willow://identity"));
    }

    #[test]
    fn readonly_blocks_all_tools() {
        let scope = TokenScope::ReadOnly;
        assert!(!scope.allows_tool("send_message"));
        assert!(!scope.allows_tool("create_channel"));
        assert!(scope.allows_resource("willow://identity"));
        assert!(scope.allows_resource("willow://server/channels"));
    }

    #[test]
    fn messaging_allows_only_messaging_tools() {
        let scope = TokenScope::Messaging;
        assert!(scope.allows_tool("send_message"));
        assert!(scope.allows_tool("send_reply"));
        assert!(scope.allows_tool("edit_message"));
        assert!(scope.allows_tool("delete_message"));
        assert!(scope.allows_tool("react"));
        assert!(scope.allows_tool("pin_message"));
        assert!(scope.allows_tool("unpin_message"));
        assert!(scope.allows_tool("send_typing"));
        assert!(!scope.allows_tool("create_channel"));
        assert!(!scope.allows_tool("kick_member"));
        assert!(!scope.allows_tool("create_server"));
        assert!(scope.allows_resource("willow://identity"));
    }

    #[test]
    fn custom_allows_only_listed_tools() {
        let mut set = HashSet::new();
        set.insert("send_message".to_string());
        set.insert("react".to_string());
        let scope = TokenScope::Custom(set);
        assert!(scope.allows_tool("send_message"));
        assert!(scope.allows_tool("react"));
        assert!(!scope.allows_tool("create_channel"));
        assert!(!scope.allows_tool("kick_member"));
        assert!(scope.allows_resource("willow://identity"));
    }

    #[test]
    fn admin_allows_everything() {
        let scope = TokenScope::Admin;
        assert!(scope.allows_tool("send_message"));
        assert!(scope.allows_tool("create_channel"));
        assert!(scope.allows_tool("kick_member"));
        assert!(scope.allows_resource("willow://identity"));
    }

    #[test]
    fn default_is_full() {
        let scope = TokenScope::default();
        assert!(scope.allows_tool("anything"));
    }
}
