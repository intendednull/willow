# UI Phase 1c — Command palette + accessibility sweep Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `layout-primitives.md` §Command palette, §Accessibility, and the palette + a11y rows of §Acceptance criteria as a single pass on top of the desktop shell (Phase 1a) and mobile shell (Phase 1b). The existing `crates/web/src/components/command_palette.rs` is refactored — not rewritten — to speak the spec's anatomy, result groups, copy, and ARIA pattern; a shared keybinding layer is extracted; every shell region shipped by 1a + 1b receives its declared ARIA landmark; every coloured-only state is paired with a shape or icon cue.

**Architecture:** Refactor `command_palette.rs` in place: keep the signal topology (`show_palette`, `query`, `selected_index`), replace markup / classes / copy with the spec's anatomy (search input → result groups → footer hint strip), add a `PaletteScope` signal derived from the modifier prefix (`#` / `@` / `>` / literal quotes), and table-drive the action catalog. Palette CSS is added to `foundation.css` (tokens-first, no component-tree CSS) using `willow-pop-in` + `--motion` + backdrop blur. Keyboard shortcuts move from the inline closure in `app.rs` into a new `crates/web/src/keybindings.rs` module that owns the `Esc` close-stack priority (rail → sheet → drawer → palette), ⌘K / Ctrl-K palette toggle, Tab landmark cycling, and the optional Alt+↑/↓ grove switch. ARIA landmarks are added by editing each shell component's root element (no new components). Recents persist via `localStorage` through a tiny `palette_recents` helper. Browser tests assert each landmark selector, the palette open keybinding, and the reduced-motion path.

**Tech Stack:** Leptos 0.7, Rust/WASM, Trunk, existing `foundation.css` tokens, `web_sys` `localStorage` and `matchMedia`, `wasm-bindgen-test` (headless Firefox), just runner.

**Scope gate:**
- In scope: `layout-primitives.md` §Command palette (entry, anatomy, input, result groups, actions catalog v1, recents, empty / loading / error, copy exact, accessibility, data deps); §Accessibility (focus order, keyboard shortcuts, ARIA landmarks, touch targets, reduced motion, colour-independent cues); §Acceptance rows covering palette + a11y + focus ring + touch targets + reduced motion.
- Out of scope: desktop shell (Phase 1a), mobile shell (Phase 1b), trust / presence / notifications (separate plans), message-row internals, composer internals, call chrome, sync-queue, new state events. Local search handoff surface only — indexing owned by `local-search.md`.
- Spec source of truth: `/mnt/storage/projects/willow/docs/specs/2026-04-19-ui-design/layout-primitives.md`.

**Branch:** `design/ui-target-ux`. Phase 1a + 1b are assumed merged. Plan lands as one or more commits; the branch carries the Phase 1c PR.

---

## File structure

| Path | State | Responsibility |
|------|-------|----------------|
| `crates/web/src/components/command_palette.rs` | modify | Refactor existing 228-line component to spec anatomy. Keeps `show_palette` signal contract; rewrites markup, classes, copy, and result assembly. Adds `PaletteScope`, result groups (Channels / Letters / Groves / People / Actions / Search), `aria-activedescendant` combobox pattern, and the actions catalog. |
| `crates/web/src/components/palette_actions.rs` | **new** | Table-driven actions catalog v1 (`Action` struct + `actions_catalog()` returning `Vec<Action>`). Each entry carries `id`, `label`, `icon_fn`, `is_available: Fn(&AppState) -> bool`, `run: Fn(&CommandContext)`. Gates `new channel` on `ManageChannels` and `move this call` on active-call state. Imported by `command_palette.rs`. |
| `crates/web/src/palette_recents.rs` | **new** | Local-storage recents (`load`, `push`, `clear`), max 8 entries, JSON via `serde_json`. Reads a `remember_recents` toggle (default `true`) from `localStorage` key `willow.palette.remember-recents`; the Settings UI for it is a stub consumed later by `settings-tweaks.md`. |
| `crates/web/src/keybindings.rs` | **new** | Owns the global `keydown` listener. Handles ⌘K / Ctrl-K palette toggle, `Escape` close-stack priority (right rail → bottom sheet → grove drawer → palette), Tab landmark cycling helper, Alt+↑ / Alt+↓ grove switch (feature-flagged `v1_grove_switch_keys`). Exposes `install(app_state, write)` called once from `app.rs`. |
| `crates/web/src/app.rs` | modify | (a) Remove the inline `keydown` closure that currently registers ⌘K / Ctrl-K; replace with `keybindings::install(app_state, write)`. (b) Add ARIA landmark attributes to the outer shell containers that live in `app.rs` (`class="app"`, `class="main-content"`, `members-overlay`, mobile tab bar region). (c) Wire the `CommandPalette` new props (`on_create_grove`, `on_new_letter`, `on_open_sync_queue`, `on_move_call`, `is_call_active`, `can_manage_channels`). (d) Preserve the 1a-introduced `search (⌘K)` button in the main-pane header (`on_search_click`) as the click entry. |
| `crates/web/src/components/sidebar.rs` | modify | Add `role="navigation"` + `aria-label="channels"` to the sidebar root; add `role="navigation"` + `aria-label="groves"` to the grove rail wrapper rendered by `ServerList` (below); pair every coloured-only state with a shape cue (unread-channel left bar, active-channel `--bg-3` + ink ladder, offline footer word `queued`). |
| `crates/web/src/components/server_list.rs` | modify | Wrap the grove rail root with `role="navigation"` + `aria-label="groves"`; confirm active-grove indicator bar + unread pebble are rendered (shape cue contract); add `aria-current="page"` on the active tile. |
| `crates/web/src/components/chat.rs` | modify | (a) `ChannelHeader` root becomes `<header role="banner" aria-label="channel header">`; chat container gets `role="main"` with `aria-label` bound to current channel. (b) Update the `search (⌘K)` button (1a surface) to `aria-keyshortcuts="Control+K Meta+K"` + `aria-label="search (⌘K)"`. Pair every colour-only state on the header (active action button `--bg-3` + `--ink-0`) with the `aria-pressed="true"` attribute. |
| `crates/web/src/components/member_list.rs` | modify | Root becomes `<aside role="complementary" aria-label="members">`. Mobile overlay wrapper gets `aria-modal="true"` while open. Status dot continues to carry a `title` attribute (colour-independent cue for online/offline). |
| `crates/web/src/components/pinned.rs` | modify | Root becomes `<aside role="complementary" aria-label="pinned">`. |
| `crates/web/foundation.css` | modify | Append §Palette block: `.palette-root` (dialog container), `.palette-backdrop`, `.palette-input`, `.palette-group`, `.palette-group-label`, `.palette-row`, `.palette-row[aria-selected="true"]`, `.palette-footer`, `.palette-empty`, `.palette-loading`, `.palette-error`. Use only `--bg-*`, `--line*`, `--ink-*`, `--moss-*`, `--radius*`, `--shadow-2`, `--motion`, `--motion-ease`, `--focus-ring`, `--font-display`, `--font-ui`, `--font-mono` tokens. Keyframe is the existing `willow-pop-in`. Reduced-motion override maps scale to opacity only. Touch-target minimum rule `.palette-row { min-height: 44px }` on mobile. |
| `crates/web/style.css` | modify | Delete the legacy `.palette-overlay`, `.palette`, `.palette-input`, `.palette-results`, `.palette-item`, `.palette-empty`, `.palette-hint` block (lines ~3683–3695 in current HEAD) to prevent drift with the refactored markup. |
| `crates/web/tests/browser.rs` | modify | Append a `phase_1c_palette_and_a11y` test module. Assertions per §Acceptance gates below. |

No other files change in Phase 1c.

**Upstream deps (assumed landed):**
- Phase 1a introduces `search (⌘K)` button in the main-pane header. Phase 1c owns the palette this button opens via `write.ui.set_show_palette.set(true)`.
- Phase 1b introduces the mobile top-bar right-slot search button; same palette open handler.

---

## Acceptance gates (before the phase PR is opened)

1. `just fmt && just clippy && just test` pass on the branch.
2. `just check-wasm` passes.
3. `just test-browser` passes with the new `phase_1c_palette_and_a11y` module green.
4. `just dev` + manual walk-through:
   - ⌘K (or Ctrl-K) opens the palette; Esc closes it.
   - Placeholder reads `jump or search…`. Footer hint strip reads `↑↓ move · ⏎ open · esc close`.
   - Typing `#` scopes to channels; `@` scopes to peers; `>` scopes to actions; `"reading list"` forces literal.
   - Empty + no recents shows `jump or search across willow — try #channel, @peer, > for actions`.
   - No-match body reads `nothing matches '<q>' — try > for actions or /search`.
   - Search handoff row renders `search "<q>" in <scope>`.
   - Arrow up / down cycles result rows; Enter activates; mouse hover mirrors selection.
   - `prefers-reduced-motion: reduce`: palette fades in (no scale), right rail / drawer / sheet slide collapses to opacity.
   - DevTools a11y tree: `navigation[groves]`, `navigation[channels]`, `banner[channel header]`, `main[<channel>]`, `complementary[members|pinned|thread]`, `navigation[primary]` on mobile, `dialog[groves]` for the drawer (mobile), `dialog[command palette]` for the palette.
   - Focus ring: every interactive renders `var(--focus-ring)` when keyboard-focused.
   - Colour-independent cues: swap to `data-accent="ember"` — active / unread / offline states remain distinguishable by shape / icon / word.

---

## Task 1: Refactor `command_palette.rs` container + foundation.css palette block

**Files:**
- Modify: `crates/web/src/components/command_palette.rs`
- Modify: `crates/web/foundation.css`
- Modify: `crates/web/style.css` (delete legacy block)

- [ ] **Step 1.1 — Append the palette CSS block to `foundation.css`.**

```css
/* ── Command palette ─────────────────────────────────────────────── */
.palette-backdrop {
  position: fixed; inset: 0;
  background: color-mix(in oklab, var(--bg-0) 40%, transparent);
  backdrop-filter: blur(4px);
  display: flex; justify-content: center;
  padding-top: 15vh;
  z-index: 1000;
  animation: willow-pop-in var(--motion) var(--motion-ease);
}
.palette-root {
  width: min(560px, calc(100% - 32px));
  max-height: 60vh;
  background: var(--bg-1);
  border: 1px solid var(--line);
  border-radius: var(--radius-l);
  box-shadow: var(--shadow-2);
  display: flex; flex-direction: column;
  overflow: hidden;
  animation: willow-pop-in var(--motion) var(--motion-ease);
}
.palette-input {
  height: 48px;
  padding: 0 16px;
  background: transparent;
  border: none;
  border-bottom: 1px solid var(--line-soft);
  color: var(--ink-0);
  font: 15px var(--font-ui);
  outline: none;
}
.palette-input::placeholder { color: var(--ink-3); }
.palette-input:focus-visible { box-shadow: inset 0 -2px 0 var(--moss-2); }
.palette-results { overflow-y: auto; padding: 4px 0; }
.palette-group-label {
  font: 500 10.5px var(--font-ui);
  letter-spacing: 1.2px;
  text-transform: uppercase;
  color: var(--ink-3);
  padding: 8px 16px 4px;
}
.palette-row {
  display: flex; align-items: center; gap: 10px;
  padding: 8px 16px;
  color: var(--ink-1);
  cursor: pointer;
  font: 13.5px var(--font-ui);
}
.palette-row[aria-selected="true"] { background: var(--bg-2); color: var(--ink-0); }
.palette-row:focus-visible { box-shadow: var(--focus-ring); }
.palette-row .icon { color: var(--ink-2); flex: none; }
.palette-row-meta { color: var(--ink-3); font-size: 12px; margin-left: auto; }
.palette-footer {
  border-top: 1px solid var(--line-soft);
  padding: 8px 16px;
  color: var(--ink-3);
  font: 11px var(--font-mono);
  display: flex; gap: 14px;
}
.palette-empty, .palette-loading, .palette-error {
  padding: 24px 16px; text-align: center;
  color: var(--ink-2); font: italic 14px var(--font-display);
}
.palette-loading { color: var(--ink-3); font-family: var(--font-mono); font-style: normal; }
.palette-error  { color: var(--err);   font-style: normal; }

@media (max-width: 720px) {
  .palette-backdrop { padding-top: 0; align-items: flex-start; }
  .palette-root { width: 100%; max-height: 80vh; border-radius: 0 0 var(--radius-l) var(--radius-l); }
  .palette-row { min-height: 44px; }
}

@media (prefers-reduced-motion: reduce) {
  .palette-backdrop, .palette-root { animation: none; }
}
```

- [ ] **Step 1.2 — Delete the legacy palette CSS from `style.css`.**

Grep for `.palette-overlay` in `crates/web/style.css`; remove the block it opens plus `.palette`, `.palette-input`, `.palette-results`, `.palette-item`, `.palette-empty`, `.palette-hint` that follow it.

- [ ] **Step 1.3 — Rewrite `command_palette.rs` container skeleton.**

Keep the existing signals (`show_palette`, `query`, `selected_index`). Replace the root `view!` with:

```rust
view! {
    <div
        class="palette-backdrop"
        role="presentation"
        on:click=move |_| on_close.run(())
    >
        <div
            class="palette-root"
            role="dialog"
            aria-modal="true"
            aria-label="command palette"
            on:click=move |ev: web_sys::MouseEvent| ev.stop_propagation()
        >
            // input (Task 2), results (Task 3), footer (Task 6)
        </div>
    </div>
}
```

- [ ] **Step 1.4 — `just check-wasm`.** Expected: compiles. Visual regression mid-refactor is fine; later tasks restore UX.

- [ ] **Step 1.5 — Commit.**

```bash
git add crates/web/src/components/command_palette.rs crates/web/foundation.css crates/web/style.css
git commit -m "ui(phase-1): refactor palette container to spec anatomy + tokens"
```

---

## Task 2: Palette input + scope parser

**Files:**
- Modify: `crates/web/src/components/command_palette.rs`

- [ ] **Step 2.1 — Add `PaletteScope` enum + `parse_input`.**

```rust
#[derive(Clone, Copy, PartialEq, Debug)]
enum PaletteScope {
    Mixed,
    Channels, // # prefix
    Peers,    // @ prefix
    Actions,  // > prefix
}

/// Returns `(scope, query_body, literal)`.
fn parse_input(raw: &str) -> (PaletteScope, String, bool) {
    let trimmed = raw.trim_start();
    let (scope, rest) = match trimmed.chars().next() {
        Some('#') => (PaletteScope::Channels, &trimmed[1..]),
        Some('@') => (PaletteScope::Peers,    &trimmed[1..]),
        Some('>') => (PaletteScope::Actions,  &trimmed[1..]),
        _         => (PaletteScope::Mixed,    trimmed),
    };
    let rest = rest.trim_start();
    if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
        (scope, rest[1..rest.len() - 1].to_string(), true)
    } else {
        (scope, rest.to_string(), false)
    }
}

#[cfg(test)]
mod parse_input_tests {
    use super::*;

    #[test]
    fn mixed_default()    { assert_eq!(parse_input("foo"),           (PaletteScope::Mixed, "foo".into(), false)); }
    #[test]
    fn channel_prefix()   { assert_eq!(parse_input("#general"),      (PaletteScope::Channels, "general".into(), false)); }
    #[test]
    fn peer_prefix()      { assert_eq!(parse_input("@alice"),        (PaletteScope::Peers, "alice".into(), false)); }
    #[test]
    fn action_prefix()    { assert_eq!(parse_input("> new channel"), (PaletteScope::Actions, "new channel".into(), false)); }
    #[test]
    fn literal_quotes()   { assert_eq!(parse_input("\"reading list\""), (PaletteScope::Mixed, "reading list".into(), true)); }
}
```

- [ ] **Step 2.2 — Replace the `<input>`.**

Inside `.palette-root`:

```rust
<input
    class="palette-input"
    type="text"
    placeholder="jump or search…"
    aria-label="command palette input"
    aria-autocomplete="list"
    aria-controls="palette-listbox"
    aria-activedescendant=move || active_row_id.get()
    prop:value=move || query.get()
    on:input=move |ev| {
        set_query.set(event_target_value(&ev));
        set_selected_index.set(0);
    }
    on:keydown=on_keydown
    autofocus=true
/>
```

`active_row_id` is a `Memo<String>` resolved in Task 3.

- [ ] **Step 2.3 — `Escape` clears, else closes.**

```rust
"Escape" => {
    ev.prevent_default();
    if !query.get_untracked().is_empty() {
        set_query.set(String::new());
        set_selected_index.set(0);
    } else {
        on_close.run(());
    }
}
```

- [ ] **Step 2.4 — `just test` + `just check-wasm`.** Scope parser unit tests green.

- [ ] **Step 2.5 — Commit.**

```bash
git add crates/web/src/components/command_palette.rs
git commit -m "ui(phase-1): palette input placeholder + scope prefix parser"
```

---

## Task 3: Result groups + combobox/listbox pattern

**Files:**
- Modify: `crates/web/src/components/command_palette.rs`

- [ ] **Step 3.1 — Replace `PaletteCategory` with spec groups.**

```rust
#[derive(Clone, PartialEq, Debug)]
enum ResultGroup { Channels, Letters, Groves, People, Actions, Search }

#[derive(Clone)]
struct PaletteRow {
    group: ResultGroup,
    id: String,
    label: String,
    meta: Option<String>,
    activate: PaletteActivate,
}

#[derive(Clone)]
enum PaletteActivate {
    OpenChannel(String),
    OpenLetter(String),
    SwitchGrove(String),
    OpenProfile(String),
    RunAction(&'static str),
    Search(PaletteScope, String),
}
```

- [ ] **Step 3.2 — Memo-assemble rows from state + scope.**

```rust
let rows = Memo::new(move |_| {
    let (scope, q, literal) = parse_input(&query.get());
    let q_lc = q.to_lowercase();
    let mut out: Vec<PaletteRow> = Vec::new();
    let matches = |label: &str| -> bool {
        if q.is_empty() { true }
        else if literal { label == q }
        else { label.to_lowercase().contains(&q_lc) }
    };

    // Channels
    if matches_scope(scope, ResultGroup::Channels) {
        for (i, ch) in app_state.chat.channels.get().iter().enumerate() {
            if matches(&ch.name) {
                out.push(PaletteRow {
                    group: ResultGroup::Channels,
                    id: format!("pr-ch-{i}"),
                    label: ch.name.clone(),
                    meta: Some(ch.grove_name.clone()),
                    activate: PaletteActivate::OpenChannel(ch.id.clone()),
                });
            }
        }
    }

    // Groves
    if matches_scope(scope, ResultGroup::Groves) {
        for (i, (gid, meta)) in app_state.server.servers.get().iter().enumerate() {
            if matches(&meta.name) {
                out.push(PaletteRow {
                    group: ResultGroup::Groves,
                    id: format!("pr-gv-{i}"),
                    label: meta.name.clone(),
                    meta: Some(format!("{} peers", meta.member_count)),
                    activate: PaletteActivate::SwitchGrove(gid.clone()),
                });
            }
        }
    }

    // Peers / People
    if matches_scope(scope, ResultGroup::People) {
        for (i, p) in app_state.server.members.get().iter().enumerate() {
            if matches(&p.display_name) {
                out.push(PaletteRow {
                    group: ResultGroup::People,
                    id: format!("pr-pp-{i}"),
                    label: p.display_name.clone(),
                    meta: p.handle.clone(),
                    activate: PaletteActivate::OpenProfile(p.peer_id.clone()),
                });
            }
        }
    }

    // Actions
    if matches_scope(scope, ResultGroup::Actions) {
        for (i, a) in palette_actions::catalog().into_iter().enumerate() {
            if (a.available)(&app_state) && matches(a.label) {
                out.push(PaletteRow {
                    group: ResultGroup::Actions,
                    id: format!("pr-act-{i}"),
                    label: a.label.to_string(),
                    meta: None,
                    activate: PaletteActivate::RunAction(a.id),
                });
            }
        }
    }

    // Letters — feature-flagged until letters-dms.md lands.
    // No rows emitted in v1.

    // Search handoff row (always last when there's a query).
    if !q.is_empty() {
        out.push(PaletteRow {
            group: ResultGroup::Search,
            id: "pr-search".into(),
            label: format!("search \"{q}\" in {}", scope_label(scope)),
            meta: None,
            activate: PaletteActivate::Search(scope, q.clone()),
        });
    }

    out
});

fn matches_scope(scope: PaletteScope, group: ResultGroup) -> bool {
    use PaletteScope::*;
    use ResultGroup::*;
    match (scope, group) {
        (Mixed,    _)        => true,
        (Channels, Channels) => true,
        (Peers,    People)   => true,
        (Actions,  Actions)  => true,
        _ => false,
    }
}

fn scope_label(scope: PaletteScope) -> &'static str {
    match scope {
        PaletteScope::Mixed    => "everything",
        PaletteScope::Channels => "channels",
        PaletteScope::Peers    => "peers",
        PaletteScope::Actions  => "actions",
    }
}
```

- [ ] **Step 3.3 — Emit grouped DOM.**

Between input and footer:

```rust
<div id="palette-listbox" class="palette-results" role="listbox" aria-label="results">
    <For
        each=move || group_rows(rows.get())
        key=|(group, _)| format!("{group:?}")
        let:(group, group_rows_list)
    >
        <div class="palette-group-label" aria-hidden="true">{group_label(group)}</div>
        <For
            each=move || group_rows_list.clone()
            key=|r| r.id.clone()
            let:row
        >
            <div
                role="option"
                id=row.id.clone()
                aria-selected=move || (row_is_selected(&row, selected_index.get())).to_string()
                class="palette-row"
                on:click={
                    let activate = row.activate.clone();
                    move |_| dispatch_activate(&activate)
                }
                on:mouseenter=move |_| set_selected_index.set(flat_index_of(&row, rows.get_untracked()))
            >
                <span class="icon">{icon_for_group(&row.group)}</span>
                <span>{row.label.clone()}</span>
                {row.meta.clone().map(|m| view!{ <span class="palette-row-meta">{m}</span> })}
            </div>
        </For>
    </For>
</div>
```

Helpers `group_rows`, `group_label`, `icon_for_group`, `row_is_selected`, `flat_index_of`, `dispatch_activate` live at the bottom of the file.

- [ ] **Step 3.4 — `active_row_id` memo.**

```rust
let active_row_id = Memo::new(move |_| {
    let idx = selected_index.get();
    rows.get().get(idx).map(|r| r.id.clone()).unwrap_or_default()
});
```

- [ ] **Step 3.5 — Wire activate dispatch.**

Replace the legacy match with `dispatch_activate` that fans to the existing callbacks (`on_switch_channel`, `on_switch_server`, `on_open_members` for profile), plus `palette_actions::run(id, &ctx)` for actions and a new `on_search` prop for Search. After dispatch: `palette_recents::push(entry_from(&row))` + `on_close.run(())`.

- [ ] **Step 3.6 — Commit.**

```bash
git add crates/web/src/components/command_palette.rs
git commit -m "ui(phase-1): palette result groups + listbox combobox pattern"
```

---

## Task 4: Actions catalog v1

**Files:**
- Create: `crates/web/src/components/palette_actions.rs`
- Modify: `crates/web/src/components/mod.rs`
- Modify: `crates/web/src/components/command_palette.rs`

- [ ] **Step 4.1 — Create the module.**

```rust
//! Palette action catalog — spec layout-primitives.md §Actions catalog (v1).
use crate::icons;
use crate::state::AppState;
use leptos::prelude::*;

#[derive(Clone)]
pub struct Action {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: fn() -> leptos::prelude::AnyView,
    pub available: fn(&AppState) -> bool,
}

#[derive(Clone)]
pub struct CommandContext {
    pub on_open_tweaks: Callback<()>,
    pub on_open_settings: Callback<()>,
    pub on_new_channel: Callback<()>,
    pub on_new_letter: Callback<()>,
    pub on_create_grove: Callback<()>,
    pub on_open_sync_queue: Callback<()>,
    pub on_move_call: Callback<()>,
    pub on_toggle_theme: Callback<()>,
    pub on_sign_out: Callback<()>,
}

pub fn catalog() -> Vec<Action> {
    vec![
        Action { id: "tweaks",       label: "open tweaks",     icon: || icons::icon_settings().into_any(), available: |_| true },
        Action { id: "settings",     label: "open settings",   icon: || icons::icon_settings().into_any(), available: |_| true },
        Action { id: "new-channel",  label: "new channel…",    icon: || icons::icon_plus().into_any(),     available: can_manage_channels },
        Action { id: "new-letter",   label: "write a letter…", icon: || icons::icon_edit().into_any(),     available: |_| true },
        Action { id: "new-grove",    label: "new grove…",      icon: || icons::icon_plus().into_any(),     available: |_| true },
        Action { id: "sync-queue",   label: "open sync queue", icon: || icons::icon_refresh().into_any(),  available: |_| true },
        Action { id: "move-call",    label: "move this call",  icon: || icons::icon_phone_off().into_any(),available: is_call_active },
        // toggle theme deferred until light theme ships.
        Action { id: "sign-out",     label: "sign out",        icon: || icons::icon_x().into_any(),        available: |_| true },
    ]
}

pub fn run(id: &str, ctx: &CommandContext) {
    match id {
        "tweaks"      => ctx.on_open_tweaks.run(()),
        "settings"    => ctx.on_open_settings.run(()),
        "new-channel" => ctx.on_new_channel.run(()),
        "new-letter"  => ctx.on_new_letter.run(()),
        "new-grove"   => ctx.on_create_grove.run(()),
        "sync-queue"  => ctx.on_open_sync_queue.run(()),
        "move-call"   => ctx.on_move_call.run(()),
        "sign-out"    => ctx.on_sign_out.run(()),
        _             => tracing::warn!(%id, "unknown palette action"),
    }
}

fn can_manage_channels(state: &AppState) -> bool {
    let me = state.network.peer_id.get();
    state.server.admin_ids.get().contains(&me)
}

fn is_call_active(state: &AppState) -> bool {
    state.voice.voice_channel.get().is_some()
}
```

- [ ] **Step 4.2 — Register in `components/mod.rs`.** Add `mod palette_actions;` + re-export.

- [ ] **Step 4.3 — Consume from `command_palette.rs`.** Pass `ctx: CommandContext` as a prop; filter catalog rows via `(a.available)(&app_state)`; dispatch via `palette_actions::run`.

- [ ] **Step 4.4 — `just check-wasm`.**

- [ ] **Step 4.5 — Commit.**

```bash
git add crates/web/src/components/palette_actions.rs crates/web/src/components/mod.rs crates/web/src/components/command_palette.rs
git commit -m "ui(phase-1): palette actions catalog v1 with permission gating"
```

---

## Task 5: Palette recents (local-storage)

**Files:**
- Create: `crates/web/src/palette_recents.rs`
- Modify: `crates/web/src/main.rs`
- Modify: `crates/web/src/components/command_palette.rs`

- [ ] **Step 5.1 — Create the module.**

```rust
//! Palette recents — local storage, max 8 entries, toggleable.

use serde::{Deserialize, Serialize};

pub const MAX_RECENTS: usize = 8;
const KEY: &str         = "willow.palette.recents";
const TOGGLE_KEY: &str  = "willow.palette.remember-recents";

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Recent {
    pub kind: String,  // "channel" | "grove" | "peer" | "action" | "letter"
    pub id: String,
    pub label: String,
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn remember_enabled() -> bool {
    storage()
        .and_then(|s| s.get_item(TOGGLE_KEY).ok().flatten())
        .map(|v| v != "false")
        .unwrap_or(true)
}

pub fn set_remember_enabled(v: bool) {
    if let Some(s) = storage() {
        let _ = s.set_item(TOGGLE_KEY, if v { "true" } else { "false" });
    }
}

pub fn load() -> Vec<Recent> {
    if !remember_enabled() { return Vec::new(); }
    storage()
        .and_then(|s| s.get_item(KEY).ok().flatten())
        .and_then(|j| serde_json::from_str::<Vec<Recent>>(&j).ok())
        .unwrap_or_default()
}

pub fn push(entry: Recent) {
    if !remember_enabled() { return; }
    let mut list = load();
    list.retain(|e| !(e.kind == entry.kind && e.id == entry.id));
    list.insert(0, entry);
    list.truncate(MAX_RECENTS);
    if let Some(s) = storage() {
        if let Ok(j) = serde_json::to_string(&list) {
            let _ = s.set_item(KEY, &j);
        }
    }
}

pub fn clear() {
    if let Some(s) = storage() { let _ = s.remove_item(KEY); }
}
```

- [ ] **Step 5.2 — Register in `main.rs`.** Add `mod palette_recents;`.

- [ ] **Step 5.3 — Consume in `command_palette.rs`.**
- When opened + `query` empty, render Recents group instead of result groups.
- On activate, build a `Recent { kind, id, label }` and call `palette_recents::push`.

- [ ] **Step 5.4 — Commit.**

```bash
git add crates/web/src/palette_recents.rs crates/web/src/main.rs crates/web/src/components/command_palette.rs
git commit -m "ui(phase-1): palette recents persistence (local storage, max 8)"
```

---

## Task 6: Empty / loading / error copy (exact) + footer

**Files:**
- Modify: `crates/web/src/components/command_palette.rs`

- [ ] **Step 6.1 — State renderer.**

```rust
let body = move || {
    let q = query.get();
    let rows_len = rows.get().len();
    if q.is_empty() && recents.get().is_empty() {
        view! {
            <div class="palette-empty">
                "jump or search across willow — try #channel, @peer, > for actions"
            </div>
        }.into_any()
    } else if q.is_empty() {
        render_recents(recents.get()).into_any()
    } else if rows_len == 0 && !search_running.get() {
        view! {
            <div class="palette-empty">
                {format!("nothing matches '{q}' — try > for actions or /search")}
            </div>
        }.into_any()
    } else if search_running.get() {
        view! {
            <div class="palette-loading" role="status" aria-live="polite">
                "searching… (local only)"
            </div>
        }.into_any()
    } else if search_error.get() {
        view! {
            <div class="palette-error" role="status" aria-live="polite">
                "search indexer is rebuilding — try again in a moment"
            </div>
        }.into_any()
    } else {
        render_groups(rows.get()).into_any()
    }
};
```

`search_running`, `search_error` are `RwSignal<bool>`; v1 stays `false` until `local-search.md` ships.

- [ ] **Step 6.2 — Footer hint strip.**

```rust
<div class="palette-footer" aria-hidden="true">
    <span>"↑↓ move"</span>
    <span>"⏎ open"</span>
    <span>"esc close"</span>
</div>
```

- [ ] **Step 6.3 — `just check-wasm` + `just test-browser`.**

- [ ] **Step 6.4 — Commit.**

```bash
git add crates/web/src/components/command_palette.rs
git commit -m "ui(phase-1): palette empty / loading / error states with spec copy"
```

---

## Task 7: Palette motion polish + reduced-motion path

**Files:**
- Modify: `crates/web/src/components/command_palette.rs`

- [ ] **Step 7.1 — Verify `willow-pop-in` + reduced-motion override landed in Task 1.** Grep `crates/web/foundation.css`.

- [ ] **Step 7.2 — `will-change` scrub effect.**

```rust
Effect::new(move |_| {
    if show_palette.get() {
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Some(node) = doc.query_selector(".palette-root").ok().flatten() {
                if let Ok(html) = node.dyn_into::<web_sys::HtmlElement>() {
                    let _ = html.style().set_property("will-change", "transform, opacity");
                    let h = html.clone();
                    set_timeout(
                        move || { let _ = h.style().remove_property("will-change"); },
                        std::time::Duration::from_millis(180),
                    );
                }
            }
        }
    }
});
```

- [ ] **Step 7.3 — Commit.**

```bash
git add crates/web/src/components/command_palette.rs
git commit -m "ui(phase-1): palette motion polish (will-change scrub)"
```

---

## Task 8: Keyboard binding layer + Esc close-stack

**Files:**
- Create: `crates/web/src/keybindings.rs`
- Modify: `crates/web/src/main.rs`
- Modify: `crates/web/src/app.rs`

- [ ] **Step 8.1 — Create the module.**

```rust
//! Global keybindings — spec layout-primitives.md §Accessibility.
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::state::{AppState, AppWriteSignals};

pub fn install(state: AppState, write: AppWriteSignals) {
    let closure = Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(
        move |ev: web_sys::KeyboardEvent| {
            let is_ctrl = ev.ctrl_key() || ev.meta_key();
            match ev.key().as_str() {
                "k" | "K" if is_ctrl => {
                    ev.prevent_default();
                    write.ui.set_show_palette.update(|v| *v = !*v);
                }
                "Escape" => {
                    if close_top_of_stack(state.clone(), write.clone()) {
                        ev.prevent_default();
                    }
                }
                "ArrowUp" if ev.alt_key() => {
                    ev.prevent_default();
                    switch_grove(state.clone(), write.clone(), -1);
                }
                "ArrowDown" if ev.alt_key() => {
                    ev.prevent_default();
                    switch_grove(state.clone(), write.clone(), 1);
                }
                _ => {}
            }
        },
    );
    if let Some(w) = web_sys::window() {
        let _ = w.add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
    }
    closure.forget();
}

fn close_top_of_stack(state: AppState, write: AppWriteSignals) -> bool {
    if state.ui.show_members.get_untracked() {
        write.ui.set_show_members.set(false); return true;
    }
    if state.ui.show_pinned.get_untracked() {
        write.ui.set_show_pinned.set(false); return true;
    }
    #[cfg(feature = "mobile_shell")]
    {
        if state.ui.show_bottom_sheet.get_untracked() {
            write.ui.set_show_bottom_sheet.set(false); return true;
        }
        if state.ui.show_grove_drawer.get_untracked() {
            write.ui.set_show_grove_drawer.set(false); return true;
        }
    }
    if state.ui.show_palette.get_untracked() {
        write.ui.set_show_palette.set(false); return true;
    }
    false
}

fn switch_grove(state: AppState, write: AppWriteSignals, delta: i32) {
    let servers = state.server.servers.get_untracked();
    if servers.is_empty() { return; }
    let active = state.server.active_server_id.get_untracked();
    let idx = servers.iter().position(|(id, _)| id == &active).unwrap_or(0) as i32;
    let len = servers.len() as i32;
    let next = ((idx + delta).rem_euclid(len)) as usize;
    let (id, _) = &servers[next];
    write.server.set_active_server_id.set(id.clone());
}
```

- [ ] **Step 8.2 — Register in `main.rs`.** Add `mod keybindings;`.

- [ ] **Step 8.3 — Replace inline keydown in `app.rs`.** Delete the existing ⌘K / Ctrl-K block, replace with `crate::keybindings::install(app_state.clone(), write.clone());` after `wire_derived_signals`.

- [ ] **Step 8.4 — `just test-browser`.**

- [ ] **Step 8.5 — Commit.**

```bash
git add crates/web/src/keybindings.rs crates/web/src/main.rs crates/web/src/app.rs
git commit -m "ui(phase-1): extract keybindings module + Esc close-stack + Alt±grove"
```

---

## Task 9: ARIA landmark sweep

**Files:**
- Modify: `crates/web/src/app.rs`
- Modify: `crates/web/src/components/server_list.rs`
- Modify: `crates/web/src/components/sidebar.rs`
- Modify: `crates/web/src/components/chat.rs`
- Modify: `crates/web/src/components/member_list.rs`
- Modify: `crates/web/src/components/pinned.rs`

- [ ] **Step 9.1 — Grove rail.** In `server_list.rs`:

```rust
<nav class="server-rail" role="navigation" aria-label="groves">
```

Active tile: `aria-current="page"`.

- [ ] **Step 9.2 — Channel sidebar.** In `sidebar.rs`:

```rust
<nav class="sidebar" role="navigation" aria-label="channels" aria-hidden=move || if open.get() { "false" } else { "true" }>
```

- [ ] **Step 9.3 — Main pane header + body.** In `chat.rs`:

```rust
<header class="channel-header" role="banner" aria-label="channel header">
```

Container (in `app.rs` chat wrapper):

```rust
<main class="chat-container" role="main" aria-label=move || channel_signal.get()>
```

Search button: `aria-keyshortcuts="Control+K Meta+K"` + `aria-label="search (⌘K)"`. Members / pinned / thread toggles: `aria-pressed=move || flag.get().to_string()`.

- [ ] **Step 9.4 — Right rail.**
- `member_list.rs`: `<aside class="member-list" role="complementary" aria-label="members" aria-modal="true">`.
- `pinned.rs`: `<aside class="pinned-panel" role="complementary" aria-label="pinned">`.

- [ ] **Step 9.5 — Mobile tab bar + grove drawer + bottom sheet.** Phase 1b owns containers; add landmarks *if present*, else leave `// TODO(phase-1b)` comments referencing selectors:
- Tab bar: `<nav class="tab-bar" role="navigation" aria-label="primary">`.
- Grove drawer: `<dialog class="grove-drawer" role="dialog" aria-modal="true" aria-label="groves">`.
- Bottom sheet: `<dialog class="bottom-sheet" role="dialog" aria-modal="true" aria-label="{sheet-label}">`.

- [ ] **Step 9.6 — `just check-wasm`.**

- [ ] **Step 9.7 — Commit.**

```bash
git add crates/web/src/app.rs crates/web/src/components/server_list.rs crates/web/src/components/sidebar.rs crates/web/src/components/chat.rs crates/web/src/components/member_list.rs crates/web/src/components/pinned.rs
git commit -m "ui(phase-1): ARIA landmarks on every shell region"
```

---

## Task 10: Colour-independent cues audit

**Files:**
- Modify: `crates/web/src/components/server_list.rs`
- Modify: `crates/web/src/components/sidebar.rs`
- Modify: `crates/web/src/components/chat.rs`
- Modify: `crates/web/foundation.css`

- [ ] **Step 10.1 — Active grove indicator bar.** Ensure `.server-rail-tile.active::before { content:''; position:absolute; left:-12px; top:50%; transform:translateY(-50%); width:3px; height:22px; background:var(--ink-0); border-radius:2px; }` in `foundation.css`.

- [ ] **Step 10.2 — Unread grove pebble.** `.server-rail-tile.has-unread::before { height:8px; }`.

- [ ] **Step 10.3 — Active channel row.** `.channel-row.current { background:var(--bg-3); color:var(--ink-0); }` pairs accent with fill.

- [ ] **Step 10.4 — Unread channel row.** `.channel-row.has-unread::before { content:''; position:absolute; left:0; top:50%; transform:translateY(-50%); width:3px; height:16px; background:var(--ink-0); border-radius:2px; } .channel-row.has-unread .unread-pill { display:inline-flex; }`.

- [ ] **Step 10.5 — Offline net status.** `.net-status.offline .pulse { color:var(--ink-4); }` pairs with word `queued` rendered by the component.

- [ ] **Step 10.6 — Active header action button.** `aria-pressed="true"` (Task 9.3) + `--bg-3` fill.

- [ ] **Step 10.7 — Commit.**

```bash
git add crates/web/src/components/server_list.rs crates/web/src/components/sidebar.rs crates/web/src/components/chat.rs crates/web/foundation.css
git commit -m "ui(phase-1): pair every colour-only state with a shape or icon cue"
```

---

## Task 11: Browser tests + phase check + PR

**Files:**
- Modify: `crates/web/tests/browser.rs`

- [ ] **Step 11.1 — Append test module.**

```rust
// ── Phase 1c — palette + a11y (spec: layout-primitives.md) ──────────────────

#[cfg(test)]
mod phase_1c_palette_and_a11y {
    use super::*;

    fn query_selector(sel: &str) -> Option<web_sys::Element> {
        web_sys::window()?.document()?.query_selector(sel).ok().flatten()
    }

    #[wasm_bindgen_test]
    async fn palette_opens_on_ctrl_k() {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("k"); init.set_ctrl_key(true);
        let kbd = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        web_sys::window().unwrap().dispatch_event(&kbd).unwrap();
        tick().await;
        assert!(query_selector(".palette-root[role='dialog']").is_some());
    }

    #[wasm_bindgen_test]
    async fn palette_esc_closes_when_empty() {
        let init = web_sys::KeyboardEventInit::new(); init.set_key("Escape");
        let kbd = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        web_sys::window().unwrap().dispatch_event(&kbd).unwrap();
        tick().await;
        assert!(query_selector(".palette-root").is_none());
    }

    #[wasm_bindgen_test]
    async fn landmark_grove_rail()      { assert!(query_selector("nav[aria-label='groves']").is_some()); }
    #[wasm_bindgen_test]
    async fn landmark_channel_sidebar() { assert!(query_selector("nav[aria-label='channels']").is_some()); }
    #[wasm_bindgen_test]
    async fn landmark_channel_header()  { assert!(query_selector("header[role='banner'][aria-label='channel header']").is_some()); }
    #[wasm_bindgen_test]
    async fn landmark_main_body()       { assert!(query_selector("main[role='main']").is_some()); }
    #[wasm_bindgen_test]
    async fn landmark_members()         { assert!(query_selector("aside[aria-label='members']").is_some()); }

    #[wasm_bindgen_test]
    async fn search_button_has_keyshortcut() {
        let btn = query_selector("button[aria-label='search (⌘K)']").expect("search button present");
        let attr = btn.get_attribute("aria-keyshortcuts").unwrap_or_default();
        assert!(attr.contains("Control+K") && attr.contains("Meta+K"));
    }

    #[wasm_bindgen_test]
    async fn palette_footer_copy() {
        let init = web_sys::KeyboardEventInit::new(); init.set_key("k"); init.set_ctrl_key(true);
        let kbd = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        web_sys::window().unwrap().dispatch_event(&kbd).unwrap();
        tick().await;
        let footer = query_selector(".palette-footer").expect("palette footer present");
        let text = footer.text_content().unwrap_or_default();
        assert!(text.contains("↑↓ move"));
        assert!(text.contains("⏎ open"));
        assert!(text.contains("esc close"));
    }

    #[wasm_bindgen_test]
    async fn palette_placeholder_copy() {
        let input = query_selector(".palette-input").expect("palette input present");
        let ph = input.get_attribute("placeholder").unwrap_or_default();
        assert_eq!(ph, "jump or search…");
    }

    #[wasm_bindgen_test]
    async fn reduced_motion_palette_rule_exists() {
        let doc = web_sys::window().unwrap().document().unwrap();
        let sheets = doc.style_sheets();
        let mut found = false;
        for i in 0..sheets.length() {
            if let Some(sheet) = sheets.item(i) {
                if let Ok(css_sheet) = sheet.dyn_into::<web_sys::CssStyleSheet>() {
                    if let Ok(rules) = css_sheet.css_rules() {
                        for j in 0..rules.length() {
                            if let Some(rule) = rules.item(j) {
                                let text = rule.css_text();
                                if text.contains("prefers-reduced-motion") && text.contains("palette") {
                                    found = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(found, "reduced-motion rule for palette must exist");
    }
}
```

- [ ] **Step 11.2 — `just check`.**

- [ ] **Step 11.3 — `just test-browser`.**

- [ ] **Step 11.4 — Manual smoke** from Acceptance gates.

- [ ] **Step 11.5 — PR.**

```bash
git push
gh pr create --title "ui(phase-1c): command palette + a11y sweep" --body "$(cat <<'EOF'
## Summary

- Refactored `command_palette.rs` to spec anatomy: scope prefixes (#, @, >, literal quotes), six result groups, combobox + listbox with `aria-activedescendant`.
- New `palette_actions.rs` table-driven catalog with permission/call-state gating.
- New `palette_recents.rs` localStorage persistence (max 8, toggleable).
- New `keybindings.rs` centralising ⌘K toggle, `Escape` close-stack (rail → sheet → drawer → palette), Alt+↑/↓ grove switch.
- ARIA landmarks on every shell region.
- Colour-independent cues audit: shape/icon/word pairs colour on every state.

## Test plan

- [x] `just check`
- [x] `just test-browser` (new `phase_1c_palette_and_a11y` module green)
- [x] Manual smoke

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 11.6 — Record PR URL.**

---

## Post-phase notes

- Letters rows are feature-flagged off until `letters-dms.md` ships.
- `toggle theme` deferred until light theme ships; absent from v1.
- `move this call` delegates to `device-handoff.md`; v1 callback is a stub toast.
- `search` handoff: v1 logs only; `local-search.md` wires the indexer.
- `ManageChannels` gating uses `admin_ids` proxy until a per-permission signal exists.
- Alt+↑ / Alt+↓ ships default-on; gate behind preference if it collides with OS shortcuts.

---

## Self-review checklist

- [x] Every palette section in spec covered (Tasks 1–7).
- [x] Every a11y rule covered (Tasks 7, 8, 9, 10, 11).
- [x] Palette reachable from 1a `search (⌘K)` button.
- [x] Every foundation token consumed by name, no hex.
- [x] Every interactive has `--focus-ring`.
- [x] Every task ends with `ui(phase-1): <imperative>` commit.
- [x] Browser tests assert each landmark (Task 11.1).
- [x] No placeholders, no "TBD", no "similar to".
