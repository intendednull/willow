# UX Navigation Improvements

## Current Issues

### Settings navigation is confusing
Two separate panels (User Settings, Server Settings) accessed from different places. Sidebar has both a "Settings" button and a gear icon going to different panels. User must click Settings → link at bottom to reach Server Settings. Two paths, neither intuitive.

### No navigation hierarchy
Everything is flat panel toggles. Settings, Server Settings, Add Server, Chat all occupy the same space with no breadcrumbs or back-stack. "Back" button exists but no sense of where you are.

### Add Server is buried
After onboarding, only accessible via tiny "+" on server rail. Not discoverable.

### Invite flow is disconnected
Inviter goes to Server Settings → enter peer ID → generate code. Invitee starts from Welcome/Add Server. The two sides feel unrelated.

### Role management deeply nested
Server Settings → scroll → Role Manager. Too hidden for something as important as permissions.

### No confirmation on destructive actions
Channel delete, kick member, delete message — all single-click, no "are you sure?"

### Mobile sidebar lacks context
No indication of current server beyond highlighted channel item.

## Improvements to Implement

### 1. Unified settings panel with tabs
Replace two separate settings panels with a single tabbed panel:
- **Profile** tab (display name, peer ID)
- **Server** tab (server name, description, invites)
- **Roles** tab (role CRUD, permissions)

### 2. Confirmation dialogs for destructive actions
Add a reusable `<ConfirmDialog>` component. Use for:
- Channel delete
- Kick member
- Delete message

### 3. Breadcrumb in settings header
Show "Settings > Server > Roles" style breadcrumb so user knows where they are and can navigate back to any level.

### 4. Server context menu
Right-click (desktop) or long-press (mobile) on server icon shows:
- Server Settings
- Invite
- Leave Server

### 5. Improved invite UX
Show a "copy invite link" button directly in the sidebar or server header, reducing the steps to share an invite.

### 6. Quick channel switcher
Ctrl+K opens a search overlay for fast channel/server switching.
