//! # Token Scoping
//!
//! Defines `TokenScope` variants that control which MCP tools and resources
//! a bearer token grants access to.

use std::collections::HashSet;

/// URI of the join-links resource. The `link_id` values surfaced through this
/// resource are single-step bearer credentials for `WireMessage::JoinRequest`,
/// so any scope below `Full`/`Admin` MUST be denied access. See issue #436
/// (audit finding AUD-2).
const JOIN_LINKS_URI: &str = "willow://server/join-links";

/// Scope of a bearer token, controlling tool and resource access.
///
/// Resource access is **not** uniform across all scopes: `willow://server/join-links`
/// exposes bearer credentials usable by `WireMessage::JoinRequest`, so it is
/// restricted to the privileged scopes (`Full`, `Admin`) and to `Custom` token
/// allowlists that explicitly include the URI.
#[derive(Debug, Clone, Default)]
pub enum TokenScope {
    /// All tools, all resources.
    #[default]
    Full,
    /// No tools, all resources except `willow://server/join-links`.
    ReadOnly,
    /// Messaging tools only, all resources except `willow://server/join-links`.
    Messaging,
    /// All tools, all resources (semantically distinct from Full for audit).
    Admin,
    /// Explicit allowlist of tool *and* resource names. A name in the set
    /// authorises either a tool call or a resource read with that exact
    /// identifier.
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
    ///
    /// `willow://server/join-links` exposes single-step bearer credentials and
    /// is therefore gated to `Full`/`Admin` (or a `Custom` set that lists it).
    /// All other URIs are allowed under `ReadOnly` and `Messaging`.
    pub fn allows_resource(&self, uri: &str) -> bool {
        match self {
            Self::Full | Self::Admin => true,
            Self::ReadOnly | Self::Messaging => !uri.starts_with(JOIN_LINKS_URI),
            Self::Custom(set) => set.contains(uri),
        }
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
    fn readonly_scope_denies_join_links() {
        let scope = TokenScope::ReadOnly;
        assert!(
            !scope.allows_resource("willow://server/join-links"),
            "ReadOnly scope must NOT expose join-links: link_id is a bearer credential (issue #436)"
        );
    }

    #[test]
    fn messaging_scope_denies_join_links() {
        let scope = TokenScope::Messaging;
        assert!(
            !scope.allows_resource("willow://server/join-links"),
            "Messaging scope must NOT expose join-links: link_id is a bearer credential (issue #436)"
        );
    }

    #[test]
    fn full_and_admin_allow_join_links() {
        assert!(TokenScope::Full.allows_resource("willow://server/join-links"));
        assert!(TokenScope::Admin.allows_resource("willow://server/join-links"));
    }

    #[test]
    fn custom_resource_allowlist_gates_join_links() {
        // Custom set without the URI denies it.
        let mut set = HashSet::new();
        set.insert("send_message".to_string());
        let scope = TokenScope::Custom(set);
        assert!(!scope.allows_resource("willow://server/join-links"));

        // Custom set including the URI grants it.
        let mut set = HashSet::new();
        set.insert("willow://server/join-links".to_string());
        let scope = TokenScope::Custom(set);
        assert!(scope.allows_resource("willow://server/join-links"));
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
        assert!(scope.allows_resource("willow://server/channels"));
    }

    #[test]
    fn custom_allows_only_listed_tools() {
        let mut set = HashSet::new();
        set.insert("send_message".to_string());
        set.insert("react".to_string());
        // Custom is also an explicit resource allowlist now (issue #436),
        // so include the identity URI to keep its access in this test.
        set.insert("willow://identity".to_string());
        let scope = TokenScope::Custom(set);
        assert!(scope.allows_tool("send_message"));
        assert!(scope.allows_tool("react"));
        assert!(!scope.allows_tool("create_channel"));
        assert!(!scope.allows_tool("kick_member"));
        assert!(scope.allows_resource("willow://identity"));
        // URI not in the set is denied.
        assert!(!scope.allows_resource("willow://server/channels"));
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
