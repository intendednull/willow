# UX Navigation Improvements Design

## Problem

The Willow web UI has several navigation and UX friction points:

1. **Settings navigation is confusing** — Two separate panels (User Settings, Server Settings) accessed from different places. Sidebar has both a "Settings" button and a gear icon going to different panels. Users must click through Settings → link to reach Server Settings.
2. **No confirmation on destructive actions** — Channel delete, kick member, message delete, and role delete all happen on single click with no safety net.
3. **No navigation hierarchy** — Settings panels lack breadcrumbs or location context. Users click "Back" with no sense of where they are.
4. **Server management is limited** — No way to leave a server. No context menu on server icons for quick actions.
5. **Invite flow requires too many steps** — Copying your peer ID (needed for invites) requires navigating to settings.
6. **No quick navigation** — No keyboard shortcut to jump between channels, servers, or find members.

## Scope

- **In scope:** Unified settings tabs, confirmation dialogs, breadcrumbs, server context menu, quick peer ID copy, command palette. Changes primarily in `willow-web` crate plus a `leave_server()` addition to `willow-client`.
- **Out of scope:** Changes to the P2P protocol, invite code format, relay behavior, or `willow-state` event kinds.

## Design

### 1. Unified Settings Panel with Tabs

Replace the three separate panels (`SettingsPanel`, `ServerSettingsPanel`, `RoleManager` as top-level) with a single `SettingsPanel` component with tab navigation.

**Tabs:**
- **Profile** — display name, peer ID copy (current SettingsPanel content)
- **Server** — server name, description, invite generation (current ServerSettingsPanel content minus roles)
- **Roles** — role CRUD, permission toggles, assignment (current RoleManager content)

**State changes:**
- Remove `show_server_settings` signal entirely from `UiState`/`UiWriteSignals`.
- `show_settings` opens the unified panel.
- Add `settings_tab` signal using a `SettingsTab` enum:

```rust
#[derive(Clone, Copy, PartialEq, Default)]
pub enum SettingsTab {
    #[default]
    Profile,
    Server,
    Roles,
}
```

- Sidebar gear icon opens settings with `settings_tab = Server`. Sidebar "Settings" button opens with `settings_tab = Profile`.
- `show_add_server` continues to work as a separate panel. When `show_add_server` opens, it clears `show_settings`. When `show_settings` opens, it clears `show_add_server`. Panel priority: `show_add_server` > `show_settings` > chat view.

**Access control:** "Roles" tab only visible to server owner. "Server" tab shows invite generation for everyone; server rename/description only for owner.

**Files:**
- Delete: `components/server_settings.rs` (merged into settings)
- Modify: `components/settings.rs` (becomes tabbed panel)
- Modify: `components/roles.rs` (stays as sub-component rendered in Roles tab)
- Modify: `components/mod.rs` (remove `mod server_settings`, add new component modules)
- Modify: `app.rs` (remove `show_server_settings` signal/handler, simplify panel routing)
- Modify: `state.rs` (remove `show_server_settings`, add `settings_tab` with `SettingsTab` enum, add `show_palette`)
- Modify: `handlers.rs` (if delete handlers need confirmation dialog integration)
- Modify: `event_processing.rs` (remove `show_server_settings` references)
- Modify: `style.css` (tab styles)

### 2. Confirmation Dialogs

Reusable `ConfirmDialog` component for destructive actions.

**Component props:**
- `visible: ReadSignal<bool>` — controls visibility
- `title: &str` — e.g., "Delete Channel"
- `message: &str` — e.g., "Are you sure you want to delete #random?"
- `confirm_text: &str` — e.g., "Delete"
- `danger: bool` — red confirm button when true
- `on_confirm: Callback<()>` — fires on confirm
- `on_cancel: Callback<()>` — fires on cancel or Escape

**Visual:** Modal overlay with backdrop blur, centered card, cancel + confirm buttons. Escape key dismisses.

**Wired into:**
- Channel delete (sidebar channel item "x" button)
- Kick member (member list "Kick" button)
- Message delete (message dropdown/sheet "Delete" action)
- Role delete (role manager "x" button)

**Mobile message delete flow:** When the user taps "Delete" in the mobile action sheet, the sheet closes first, then the confirmation dialog opens. No stacking of two overlays.

**State per usage site:** Each site holds `(show_confirm, set_show_confirm)` and `pending_item` signals. The destructive button sets these instead of acting directly. `on_confirm` performs the action and clears the signals.

**File:** New `components/confirm_dialog.rs`.

### 3. Breadcrumb Navigation in Settings

Replace the "← Back" button pattern with a breadcrumb-style header.

**Format:** `← Settings / Profile` where `←` is a clickable back-to-chat button and the tab name shows current location. Driven by the `settings_tab` signal.

Clicking "Settings" in the breadcrumb could switch to first tab. Current tab name is static (you're already there). Lightweight — just styled spans in the settings header.

### 4. Server Context Menu

Right-click (desktop) or long-press (mobile) on a server icon shows a popup menu.

**Menu items:**
- **Server Settings** — opens settings panel to "server" tab
- **Invite** — opens settings panel to "server" tab
- **Leave Server** — with confirmation dialog (destructive)

**Implementation:**
- New `components/context_menu.rs` — reusable positioned popup rendering at cursor/touch position.
- Context menu state is local to the `ServerList` component (not in global `UiState`), since it doesn't need to be shared. Uses local signals: `show_context_menu`, `menu_position` (x/y), `context_server_id`.
- Desktop: `on:contextmenu` on `.server-icon`, prevent default, show menu at mouse position.
- Mobile: reuse the same long-press pattern from `message.rs` (500ms timer, touchstart/touchend, haptic feedback) for consistency. Extract the long-press detection into a shared utility if practical, or duplicate the pattern with a comment noting the shared approach.
- Click outside or Escape dismisses.

**`leave_server()` in client:** This is a **local-only** operation (no P2P event, no state machine change). It removes the server from `SharedState.state.servers`, removes associated data from persistence (`storage::save_server_list`, `storage::save_server_by_id`), and switches to the next available server or clears `active_server` (returning to welcome if no servers remain). The event history for the left server is retained in storage (not purged).

**Files:** Modify `crates/client/src/lib.rs` to add `leave_server()` on `ClientHandle`.

### 5. Quick Peer ID Copy

Add a copy icon button in the sidebar user area (next to display name) that copies the user's peer ID to clipboard with a brief "Copied!" tooltip.

Both sides of the invite flow need to share peer IDs. Having it one click away in the always-visible sidebar eliminates navigating to settings just to copy your ID.

**Implementation:** Small copy icon button in the sidebar user area. `on:click` copies `handle.peer_id()` to clipboard via `navigator.clipboard.writeText()`. Shows "Copied!" tooltip briefly (CSS animation, auto-hides after 1.5s). The `peer_id` is available via `ClientHandle` from context — no new props needed on `Sidebar`.

### 6. Command Palette (Ctrl+K)

Search overlay for fast navigation across channels, servers, and members.

**Trigger:** `Ctrl+K` / `Cmd+K`. Mobile: search icon button added to channel header (in `chat.rs`).

**Global keydown registration:** Use `web_sys::window().add_event_listener_with_callback("keydown", ...)` registered in the `App` component setup. The handler checks for `Ctrl+K` / `Cmd+K`, calls `prevent_default()`, and sets `show_palette = true`. When the palette is open, it captures Escape to close. When the chat input is focused, `Ctrl+K` still opens the palette (the global listener fires before element-scoped handlers).

**UI:** Full-width overlay with backdrop blur, centered search input auto-focused, results list below. Arrow keys navigate, Enter selects, Escape closes.

**Search scope:**
- Channels — current server channels, prefixed with `#` or volume icon
- Servers — all servers, shown with server name
- Members — known peers, shown with display name + truncated peer ID

**Result format:** Icon + name + category label (right-aligned, muted). Simple substring match, case-insensitive, all client-side.

**Actions on select:**
- Channel → switch to that channel
- Server → switch to that server (and first channel)
- Member → open member list

**State:** `show_palette` signal in `UiState`/`UiWriteSignals`.

**Files:**
- New `components/command_palette.rs`
- Modify `components/chat.rs` (add search icon for mobile trigger)
- Modify `components/mod.rs` (add module)
- Modify `app.rs` (register global keydown, render palette)

## Testing

- Existing browser tests (`just test-browser`) must continue to pass.
- WASM compilation (`just check-wasm`) must pass.
- E2E helpers (`e2e/helpers.ts`) must be updated: `openServerSettings` now opens the unified settings panel to the server tab. `generateInvite` flow changes accordingly.
- E2E tests (`e2e/permissions.spec.ts`) must be updated for the new settings structure.
- New E2E tests: confirmation dialog appears on destructive actions, command palette opens/closes with Ctrl+K, server context menu appears on right-click.
