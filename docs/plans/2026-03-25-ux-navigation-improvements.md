# UX Navigation Improvements Implementation Plan

**Status:** landed (commit `0ffb33a` confirmation dialogs; broader Ctrl+K palette + context menu + leave-server in subsequent commits) — `crates/web/src/components/{confirm_dialog,command_palette,context_menu}.rs` shipped; `fn leave_server` at `crates/client/src/servers.rs:83`.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 6 UX friction points: unify settings into tabs, add confirmation dialogs for destructive actions, add breadcrumb navigation, server context menu with leave-server, quick peer ID copy, and Ctrl+K command palette.

**Architecture:** The settings panel restructure is the core change (merges 3 panels into 1 tabbed component). Confirmation dialog and command palette are new reusable components. Context menu and peer ID copy are small additions to existing components. All changes are in `willow-web` except `leave_server()` added to `willow-client`.

**Tech Stack:** Rust, Leptos 0.7 (CSR), inline SVG icons (existing `icons.rs`), CSS

**Spec:** `docs/specs/2026-03-25-ux-navigation-improvements-design.md`

---

## File Map

### New files

| File | Responsibility |
|------|----------------|
| `crates/web/src/components/confirm_dialog.rs` | Reusable modal confirmation dialog component |
| `crates/web/src/components/command_palette.rs` | Ctrl+K search overlay for channels/servers/members |
| `crates/web/src/components/context_menu.rs` | Positioned popup menu for server right-click/long-press |

### Modified files

| File | Changes |
|------|---------|
| `crates/client/src/lib.rs` | Add `leave_server()` method to `ClientHandle` |
| `crates/web/src/state.rs` | Add `SettingsTab` enum, `settings_tab` and `show_palette` signals; remove `show_server_settings` |
| `crates/web/src/components/settings.rs` | Rewrite as tabbed panel (Profile/Server/Roles tabs) with breadcrumb |
| `crates/web/src/components/roles.rs` | Minor: add confirmation dialog for role delete |
| `crates/web/src/components/sidebar.rs` | Add peer ID copy button in user area; wire confirm dialog for channel delete |
| `crates/web/src/components/member_list.rs` | Wire confirm dialog for kick |
| `crates/web/src/components/message.rs` | Wire confirm dialog for message delete |
| `crates/web/src/components/server_list.rs` | Add context menu (right-click/long-press) |
| `crates/web/src/components/chat.rs` | Add search icon for mobile command palette trigger |
| `crates/web/src/components/mod.rs` | Remove `server_settings` module, add new component modules |
| `crates/web/src/app.rs` | Remove `show_server_settings` logic, add command palette rendering + global keydown, simplify panel routing |
| `crates/web/src/event_processing.rs` | Remove `show_server_settings` references |
| `crates/web/src/handlers.rs` | Remove `show_server_settings` references |
| `crates/web/src/main.rs` | No changes expected |
| `crates/web/style.css` | Add styles for tabs, confirm dialog, command palette, context menu, copy tooltip |
| `e2e/helpers.ts` | Update `openServerSettings` to use tab-based navigation |

### Deleted files

| File | Reason |
|------|--------|
| `crates/web/src/components/server_settings.rs` | Merged into tabbed `settings.rs` |

---

## Task Ordering

1. **Task 1:** State changes (add SettingsTab enum, signals, remove show_server_settings)
2. **Task 2:** ConfirmDialog component (standalone, reusable)
3. **Task 3:** Unified settings panel with tabs + breadcrumb (depends on state changes)
4. **Task 4:** Wire confirmation dialogs into sidebar, member list, messages, roles
5. **Task 5:** Add leave_server() to client + server context menu
6. **Task 6:** Quick peer ID copy in sidebar
7. **Task 7:** Command palette
8. **Task 8:** Update E2E helpers and tests
9. **Task 9:** CSS polish + full verification

---

### Task 1: State changes

**Files:**
- Modify: `crates/web/src/state.rs`

- [ ] **Step 1: Add SettingsTab enum and update signals**

In `crates/web/src/state.rs`:

Add the enum near the top (after `ChannelViewState`):

```rust
#[derive(Clone, Copy, PartialEq, Default)]
pub enum SettingsTab {
    #[default]
    Profile,
    Server,
    Roles,
}
```

In `UiState`, replace `show_server_settings` with `settings_tab` and add `show_palette`:

```rust
pub struct UiState {
    pub show_settings: ReadSignal<bool>,
    pub settings_tab: ReadSignal<SettingsTab>,
    pub show_sidebar: ReadSignal<bool>,
    pub show_members: ReadSignal<bool>,
    pub show_add_server: ReadSignal<bool>,
    pub show_pinned: ReadSignal<bool>,
    pub show_palette: ReadSignal<bool>,
}
```

In `UiWriteSignals`, make the matching changes:

```rust
pub struct UiWriteSignals {
    pub set_show_settings: WriteSignal<bool>,
    pub set_settings_tab: WriteSignal<SettingsTab>,
    pub set_show_sidebar: WriteSignal<bool>,
    pub set_show_members: WriteSignal<bool>,
    pub set_show_add_server: WriteSignal<bool>,
    pub set_show_pinned: WriteSignal<bool>,
    pub set_show_palette: WriteSignal<bool>,
}
```

In `create_signals()`, replace the `show_server_settings` signal with the new ones:

```rust
let (settings_tab, set_settings_tab) = signal(SettingsTab::default());
let (show_palette, set_show_palette) = signal(false);
```

Wire them into the `UiState` and `UiWriteSignals` structs in `create_signals()`.

- [ ] **Step 2: Update app.rs to remove show_server_settings references**

In `crates/web/src/app.rs`:
- Remove the import of `ServerSettingsPanel` from components.
- Remove the `show_server_settings` panel rendering branch.
- The panel priority chain becomes: `show_add_server` > `show_settings` > chat.
- Update the sidebar `on_settings_click` handler to set `settings_tab = Profile`.
- Update the sidebar `on_server_settings_click` handler to set `settings_tab = Server` and `show_settings = true`.
- Both handlers should clear `show_add_server`.

- [ ] **Step 3: Update event_processing.rs and handlers.rs**

Remove any references to `set_show_server_settings` in both files. In `refresh_all_signals`, remove the line that sets `show_server_settings`.

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p willow-web`

Expected: May fail because settings.rs and server_settings.rs still exist with old props. That's fine — Task 3 fixes them.

If compilation fails due to the removed signal, temporarily comment out the `ServerSettingsPanel` usage in app.rs and the `server_settings` module in mod.rs. We'll fully replace them in Task 3.

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/state.rs crates/web/src/app.rs crates/web/src/event_processing.rs crates/web/src/handlers.rs
git commit -m "refactor: add SettingsTab enum, show_palette signal, remove show_server_settings"
```

---

### Task 2: ConfirmDialog component

**Files:**
- Create: `crates/web/src/components/confirm_dialog.rs`
- Modify: `crates/web/src/components/mod.rs`
- Modify: `crates/web/style.css`

- [ ] **Step 1: Create the component**

Create `crates/web/src/components/confirm_dialog.rs`:

```rust
use leptos::prelude::*;
use crate::icons;

/// Reusable confirmation dialog for destructive actions.
#[component]
pub fn ConfirmDialog(
    visible: ReadSignal<bool>,
    #[prop(into)] title: String,
    #[prop(into)] message: String,
    #[prop(into)] confirm_text: String,
    #[prop(default = false)] danger: bool,
    on_confirm: Callback<()>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    // Close on Escape key.
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Escape" {
            on_cancel.run(());
        }
    };

    let confirm_class = if danger {
        "btn btn-danger"
    } else {
        "btn btn-primary"
    };

    view! {
        {move || {
            if visible.get() {
                let title = title.clone();
                let message = message.clone();
                let confirm_text = confirm_text.clone();
                Some(view! {
                    <div class="confirm-overlay" on:click=move |_| on_cancel.run(()) on:keydown=on_keydown>
                        <div class="confirm-dialog" on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()>
                            <h3>{title}</h3>
                            <p>{message}</p>
                            <div class="confirm-actions">
                                <button class="btn" on:click=move |_| on_cancel.run(())>"Cancel"</button>
                                <button class=confirm_class on:click=move |_| on_confirm.run(())>{confirm_text}</button>
                            </div>
                        </div>
                    </div>
                })
            } else {
                None
            }
        }}
    }
}
```

- [ ] **Step 2: Add module to mod.rs**

In `crates/web/src/components/mod.rs`, add:

```rust
mod confirm_dialog;
pub use confirm_dialog::*;
```

- [ ] **Step 3: Add CSS for confirm dialog**

In `crates/web/style.css`, add:

```css
/* ── Confirm Dialog ─────────────────────────────────────────────── */

.confirm-overlay {
    position: fixed;
    inset: 0;
    background: var(--overlay-bg);
    backdrop-filter: var(--backdrop-blur);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    animation: fade-in 0.15s ease;
}

.confirm-dialog {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 24px;
    max-width: 400px;
    width: 90%;
    box-shadow: var(--shadow-lg);
    animation: scale-in 0.15s ease;
}

.confirm-dialog h3 {
    margin-bottom: 8px;
    font-size: 16px;
    font-weight: 600;
}

.confirm-dialog p {
    color: var(--text-secondary);
    font-size: 14px;
    margin-bottom: 20px;
    line-height: 1.5;
}

.confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
}

@keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }
@keyframes scale-in { from { opacity: 0; transform: scale(0.95); } to { opacity: 1; transform: scale(1); } }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check -p willow-web`

- [ ] **Step 5: Commit**

```bash
git add crates/web/src/components/confirm_dialog.rs crates/web/src/components/mod.rs crates/web/style.css
git commit -m "feat: add reusable ConfirmDialog component"
```

---

### Task 3: Unified settings panel with tabs + breadcrumb

**Files:**
- Rewrite: `crates/web/src/components/settings.rs`
- Delete: `crates/web/src/components/server_settings.rs`
- Modify: `crates/web/src/components/mod.rs`
- Modify: `crates/web/src/app.rs`
- Modify: `crates/web/style.css`

- [ ] **Step 1: Rewrite settings.rs as tabbed panel**

Replace `crates/web/src/components/settings.rs` entirely. The new component:
- Has a breadcrumb header: `← Settings / {tab name}`
- Has tab buttons (Profile, Server, Roles)
- Renders the appropriate content based on `settings_tab`
- Profile tab: current SettingsPanel content (display name, peer ID, save)
- Server tab: current ServerSettingsPanel content (invite generation, peer ID sharing)
- Roles tab: renders the existing `<RoleManager>` component

Read the current `settings.rs` and `server_settings.rs` and combine their logic into the new tabbed panel. The `on_server_settings` prop is removed since it's now a tab. The `on_back` prop becomes a close handler (returns to chat).

The component takes:
```rust
pub fn SettingsPanel(
    peer_id: ReadSignal<String>,
    #[prop(into)] roles: Signal<Vec<RoleEntry>>,
    on_close: impl Fn(()) + Send + Clone + 'static,
) -> impl IntoView
```

It reads `settings_tab` from `use_context::<AppState>()` and writes via `use_context::<AppWriteSignals>()` — wait, `AppWriteSignals` is not provided as context per the spec. Instead, the tab state can be local to the component or passed as a prop. Simplest: read the initial tab from `AppState`, and manage tab switching locally within the component. The `app.rs` handler sets `settings_tab` before opening the panel, and the component reads it once on mount.

Actually, to keep it simple: `SettingsPanel` takes a `default_tab: SettingsTab` prop. The app.rs handler passes the appropriate default when opening.

- [ ] **Step 2: Delete server_settings.rs and update mod.rs**

Delete `crates/web/src/components/server_settings.rs`.

In `crates/web/src/components/mod.rs`:
- Remove `mod server_settings;` and `pub use server_settings::*;`

- [ ] **Step 3: Update app.rs**

- Remove `ServerSettingsPanel` from imports.
- The `show_settings` panel branch now renders `<SettingsPanel peer_id=pid roles=roles default_tab=... on_close=... />`.
- The `default_tab` is read from `app_state.ui.settings_tab.get_untracked()`.
- Update `on_settings_click` handler: set `settings_tab = Profile`, set `show_settings = true`, clear `show_add_server`.
- Update `on_server_settings_click` handler: set `settings_tab = Server`, set `show_settings = true`, clear `show_add_server`.

- [ ] **Step 4: Add CSS for tabs**

```css
/* ── Settings Tabs ──────────────────────────────────────────────── */

.settings-tabs {
    display: flex;
    gap: 2px;
    padding: 0 16px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 16px;
}

.settings-tab {
    padding: 10px 16px;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    font-weight: 500;
    transition: color var(--transition-fast), border-color var(--transition-fast);
}

.settings-tab:hover {
    color: var(--text-secondary);
}

.settings-tab.active {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
}

.settings-breadcrumb {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 16px;
    border-bottom: 1px solid var(--border);
    font-size: 14px;
    color: var(--text-muted);
}

.settings-breadcrumb .back-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 4px;
    display: flex;
    align-items: center;
    transition: color var(--transition-fast);
}

.settings-breadcrumb .back-btn:hover { color: var(--text-primary); }

.settings-breadcrumb .separator { color: var(--text-muted); }

.settings-breadcrumb .current { color: var(--text-primary); font-weight: 500; }
```

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p willow-web`

Expected: Compiles. The `ServerSettingsPanel` is gone, replaced by the tabbed `SettingsPanel`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat: unified settings panel with tabs (Profile/Server/Roles) and breadcrumb

Merges SettingsPanel + ServerSettingsPanel + RoleManager into a single
tabbed component. Sidebar gear opens Server tab, Settings button opens
Profile tab. Breadcrumb shows current location."
```

---

### Task 4: Wire confirmation dialogs

**Files:**
- Modify: `crates/web/src/components/sidebar.rs` (channel delete)
- Modify: `crates/web/src/components/member_list.rs` (kick)
- Modify: `crates/web/src/components/message.rs` (message delete)
- Modify: `crates/web/src/components/roles.rs` (role delete)

For each site, the pattern is:
1. Add local signals: `let (show_confirm, set_show_confirm) = signal(false);` and `let (pending_item, set_pending_item) = signal(Option::<String>::None);`
2. Replace the direct action call with setting these signals.
3. Render `<ConfirmDialog>` with `on_confirm` performing the actual action.

- [ ] **Step 1: Wire channel delete confirmation in sidebar.rs**

In the channel delete button `on:click`, instead of calling `handle.delete_channel(&name)` directly, set `pending_delete_channel` and `show_confirm_channel` signals. Add a `<ConfirmDialog>` at the bottom of the sidebar view.

- [ ] **Step 2: Wire kick confirmation in member_list.rs**

Same pattern: the "Kick" button sets `pending_kick_peer` and shows the confirm dialog.

- [ ] **Step 3: Wire message delete confirmation in message.rs**

For desktop: the "Delete" dropdown item closes the dropdown, sets pending state, shows confirm dialog.
For mobile: the "Delete" sheet item closes the sheet first, then shows confirm dialog.

- [ ] **Step 4: Wire role delete confirmation in roles.rs**

The role delete "x" button shows a confirm dialog before calling `handle.delete_role()`.

- [ ] **Step 5: Verify compilation**

Run: `cargo check -p willow-web`

- [ ] **Step 6: Commit**

```bash
git add crates/web/src/components/
git commit -m "feat: add confirmation dialogs for channel delete, kick, message delete, role delete"
```

---

### Task 5: leave_server() + server context menu

**Files:**
- Modify: `crates/client/src/lib.rs` (add `leave_server()`)
- Create: `crates/web/src/components/context_menu.rs`
- Modify: `crates/web/src/components/server_list.rs`
- Modify: `crates/web/src/components/mod.rs`
- Modify: `crates/web/style.css`

- [ ] **Step 1: Add leave_server() to ClientHandle**

In `crates/client/src/lib.rs`, add to `impl ClientHandle`:

```rust
/// Leave a server (local-only — removes from UI and persistence).
/// Switches to the next available server, or clears active_server
/// if no servers remain.
pub fn leave_server(&self, server_id: &str) {
    let mut shared = self.shared.borrow_mut();
    shared.state.servers.remove(server_id);

    // Update active server.
    if shared.state.active_server.as_deref() == Some(server_id) {
        shared.state.active_server = shared.state.servers.keys().next().cloned();
    }

    // Persist.
    let ids: Vec<String> = shared.state.servers.keys().cloned().collect();
    drop(shared);
    willow_client::storage::save_server_list(&ids);
}
```

Verify exact field names and storage API by reading the code. The `storage::save_server_list` function already exists (used in `Client::new`).

- [ ] **Step 2: Create context_menu.rs**

A generic positioned popup menu. Takes a list of items (label + callback) and renders at a given x/y position. Dismisses on click outside or Escape.

- [ ] **Step 3: Add context menu to server_list.rs**

In `ServerList`, add:
- `on:contextmenu` handler on each `.server-icon` that prevents default and shows the context menu at mouse position.
- Long-press handler (reuse the 500ms timer pattern from `message.rs`) for mobile.
- Context menu items: "Server Settings", "Invite", "Leave Server".
- "Leave Server" shows a `ConfirmDialog` before calling `handle.leave_server()`.

The context menu needs callbacks to open settings and leave server. These can be passed as props or use context.

- [ ] **Step 4: Add mod to mod.rs**

```rust
mod context_menu;
pub use context_menu::*;
```

- [ ] **Step 5: Add CSS for context menu**

```css
.context-menu {
    position: fixed;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: var(--shadow-lg);
    padding: 4px;
    min-width: 160px;
    z-index: 999;
    animation: scale-in 0.1s ease;
}

.context-menu-item {
    display: block;
    width: 100%;
    padding: 8px 12px;
    background: transparent;
    border: none;
    color: var(--text-primary);
    cursor: pointer;
    font-family: inherit;
    font-size: 13px;
    text-align: left;
    border-radius: 4px;
    transition: background var(--transition-fast);
}

.context-menu-item:hover { background: var(--bg-message-hover); }

.context-menu-item.danger { color: var(--danger); }
.context-menu-item.danger:hover { background: var(--danger-glow); }
```

- [ ] **Step 6: Verify**

Run: `cargo check -p willow-client && cargo check -p willow-web`

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: add server context menu with leave-server support

Right-click (desktop) or long-press (mobile) on server icon shows
context menu with Server Settings, Invite, and Leave Server options.
leave_server() is a local-only operation on ClientHandle."
```

---

### Task 6: Quick peer ID copy in sidebar

**Files:**
- Modify: `crates/web/src/components/sidebar.rs`
- Modify: `crates/web/style.css`

- [ ] **Step 1: Add copy button to sidebar user area**

In the sidebar's user area section, add a small copy icon button next to the display name. On click, copy `handle.peer_id()` to clipboard. Show a brief "Copied!" tooltip via a local signal + CSS animation.

```rust
let (show_copied, set_show_copied) = signal(false);
let on_copy_pid = move |_| {
    let id = handle.peer_id();
    crate::util::copy_to_clipboard(&id);
    set_show_copied.set(true);
    // Auto-hide after 1.5s.
    set_timeout(move || set_show_copied.set(false), std::time::Duration::from_millis(1500));
};
```

In the view:
```rust
<button class="copy-pid-btn" title="Copy Peer ID" on:click=on_copy_pid>
    {icons::icon_copy()}
</button>
{move || show_copied.get().then(|| view! {
    <span class="copied-tooltip">"Copied!"</span>
})}
```

- [ ] **Step 2: Add CSS for tooltip**

```css
.copy-pid-btn {
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    padding: 4px;
    font-size: 14px;
    transition: color var(--transition-fast);
}
.copy-pid-btn:hover { color: var(--text-primary); }

.copied-tooltip {
    position: absolute;
    background: var(--accent);
    color: white;
    font-size: 11px;
    padding: 3px 8px;
    border-radius: 4px;
    white-space: nowrap;
    animation: tooltip-fade 1.5s ease forwards;
}

@keyframes tooltip-fade {
    0%, 70% { opacity: 1; }
    100% { opacity: 0; }
}
```

- [ ] **Step 3: Verify and commit**

```bash
cargo check -p willow-web
git add crates/web/src/components/sidebar.rs crates/web/style.css
git commit -m "feat: add quick peer ID copy button in sidebar user area"
```

---

### Task 7: Command palette

**Files:**
- Create: `crates/web/src/components/command_palette.rs`
- Modify: `crates/web/src/components/chat.rs` (mobile search icon)
- Modify: `crates/web/src/components/mod.rs`
- Modify: `crates/web/src/app.rs` (global keydown, render palette)
- Modify: `crates/web/style.css`

- [ ] **Step 1: Create command_palette.rs**

The component:
- Reads channels from `handle.channels()`, servers from `handle.server_list()`, members from `handle.server_members()`
- Has a text input that filters results (case-insensitive substring match)
- Renders results grouped: channels first, then servers, then members
- Each result has an icon (hash/volume for channels, server initial for servers, user icon for members) + name + muted category label
- Arrow up/down navigates, Enter selects, Escape closes
- On select: channels call the channel switch handler, servers call server switch, members open member list

Props:
```rust
pub fn CommandPalette(
    on_close: Callback<()>,
    on_switch_channel: Callback<String>,
    on_switch_server: Callback<String>,
    on_open_members: Callback<()>,
) -> impl IntoView
```

- [ ] **Step 2: Add global keydown listener in app.rs**

In the `App` component setup, register a global keydown listener:

```rust
// Global Ctrl+K handler for command palette.
let write_for_palette = write_signals;
wasm_bindgen_futures::spawn_local(async move {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let closure = Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
        if (ev.ctrl_key() || ev.meta_key()) && ev.key() == "k" {
            ev.prevent_default();
            write_for_palette.ui.set_show_palette.update(|v| *v = !*v);
        }
    });

    if let Some(window) = web_sys::window() {
        let _ = window.add_event_listener_with_callback(
            "keydown",
            closure.as_ref().unchecked_ref(),
        );
    }
    closure.forget(); // Leak — lives for app lifetime.
});
```

Render `<CommandPalette>` when `show_palette` is true, passing the appropriate callbacks.

- [ ] **Step 3: Add search icon to chat.rs for mobile**

In `ChannelHeader`, add a search icon button (visible on all viewports) that toggles `show_palette`. This needs a new prop or writes to context.

- [ ] **Step 4: Add mod to mod.rs**

```rust
mod command_palette;
pub use command_palette::*;
```

- [ ] **Step 5: Add CSS for command palette**

```css
/* ── Command Palette ────────────────────────────────────────────── */

.palette-overlay {
    position: fixed;
    inset: 0;
    background: var(--overlay-bg);
    backdrop-filter: var(--backdrop-blur);
    display: flex;
    justify-content: center;
    padding-top: 15vh;
    z-index: 1000;
    animation: fade-in 0.1s ease;
}

.palette {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow-lg);
    width: 90%;
    max-width: 520px;
    max-height: 400px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: scale-in 0.1s ease;
}

.palette-input {
    padding: 14px 16px;
    background: transparent;
    border: none;
    border-bottom: 1px solid var(--border);
    color: var(--text-primary);
    font-size: 15px;
    font-family: inherit;
    outline: none;
    width: 100%;
}

.palette-results {
    overflow-y: auto;
    flex: 1;
    padding: 4px;
}

.palette-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-radius: 6px;
    cursor: pointer;
    color: var(--text-primary);
    transition: background var(--transition-fast);
}

.palette-item:hover, .palette-item.selected {
    background: var(--bg-message-hover);
}

.palette-item .icon { color: var(--text-muted); }

.palette-item-label { flex: 1; }

.palette-item-category {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}

.palette-empty {
    padding: 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
}
```

- [ ] **Step 6: Verify**

Run: `cargo check -p willow-web && just check-wasm`

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: add command palette (Ctrl+K) for channel/server/member search

Full-width search overlay with keyboard navigation.
Filters channels, servers, and members by substring match.
Mobile: accessible via search icon in channel header."
```

---

### Task 8: Update E2E helpers and tests

**Files:**
- Modify: `e2e/helpers.ts`
- Modify: `e2e/permissions.spec.ts`

- [ ] **Step 1: Update openServerSettings helper**

The old helper clicks `.server-gear-btn` and expects a separate server settings panel. Now it opens the unified settings panel on the Server tab. Update:

```typescript
export async function openServerSettings(page: Page) {
  await openSidebar(page);
  await page.locator('.server-gear-btn').click();
  await page.waitForTimeout(500);
  // Settings panel should now be open on the Server tab.
}
```

The `generateInvite` helper now doesn't need to click "Back" since the invite UI is inside the settings panel's Server tab. The "Back" button at the top of settings returns to chat. Update the flow accordingly.

- [ ] **Step 2: Update E2E tests for new settings structure**

Any test that navigated to `ServerSettingsPanel` via `openServerSettings` should work with the new tabbed panel since the selectors for invite generation (`.invite-code-display`, `input[placeholder*="12D3KooW"]`, "Generate Invite" button) are the same — they're just inside a tab now.

Verify by running: `npx playwright test --project=desktop-chrome`

- [ ] **Step 3: Commit**

```bash
git add e2e/
git commit -m "fix: update E2E helpers for unified settings panel"
```

---

### Task 9: CSS polish + full verification

- [ ] **Step 1: Run full checks**

```bash
just check   # fmt + clippy + test + wasm
```

Fix any warnings or errors.

- [ ] **Step 2: Build and deploy**

```bash
cd crates/web && trunk build --release && cd ../..
sshpass -p 'WillowP2P2026deploy!' scp -o StrictHostKeyChecking=no crates/web/dist/* root@172.234.217.219:/var/www/willow/
sshpass -p 'WillowP2P2026deploy!' ssh -o StrictHostKeyChecking=no root@172.234.217.219 'chmod 644 /var/www/willow/*'
```

- [ ] **Step 3: Run E2E tests**

```bash
npx playwright test --project=desktop-chrome
```

- [ ] **Step 4: Final commit if needed**

```bash
git add -A
git commit -m "fix: address lint and test issues from UX improvements"
```
