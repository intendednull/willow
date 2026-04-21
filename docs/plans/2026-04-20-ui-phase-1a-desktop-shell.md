# UI Phase 1a — Desktop Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship desktop portion of `docs/specs/2026-04-19-ui-design/layout-primitives.md` — three-pane shell (68 / 232 / flex / 280 px), grove rail, channel sidebar, main pane header, right rail, narrow-desktop collapse, desktop empty states.

**Style reference:** `docs/plans/2026-04-19-ui-phase-0-foundation.md` — match exactly (File structure table, checkbox steps, exact paths/code/commands, Acceptance gates, Self-review).

**Spec:** `docs/specs/2026-04-19-ui-design/layout-primitives.md` (desktop sections only).

**Branch:** `design/ui-target-ux`. Commit style: `ui(phase-1): <imperative>`.

## Scope

**In:** desktop Viewport / Panel stack / Grove rail (+ kb/aria) / Channel sidebar (grove header, four groups, row variants, me strip, net status footer) / Main pane header (six-button action bar) / Right rail (one-of-three mount with 120 ms cross-fade) / Desktop transitions / Narrow-desktop collapse (<960, <840) / Desktop empty states / Copy.

**Out:** mobile shell (1b), command palette (1c), message / composer / profile-card internals, new state events.

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/web/src/app.rs` | modify | `.app` flex → `.app-shell` CSS grid. Mount GroveRail + ChannelSidebar + MainPaneHeader + RightRail. |
| `crates/web/src/components/grove_rail.rs` | **new** | Replaces server_list.rs. Letters tile, divider, grove tiles (four states via `data-state`), new-grove, discover, spacer, settings. Roving kb. |
| `crates/web/src/components/server_list.rs` | delete | Superseded. |
| `crates/web/src/components/channel_sidebar.rs` | **new** | Replaces sidebar.rs. Grove header + four groups (commons/voice/ephemeral/archives) + row variants + me strip + net status footer. |
| `crates/web/src/components/sidebar.rs` | delete | Superseded. |
| `crates/web/src/components/main_pane_header.rs` | **new** | Replaces ChannelHeader in chat.rs. Kind icon, italic title, topic, six action buttons in fixed order (members/pinned/thread/phone/search/more), one-of-three active logic. `RightRailWhich` enum exported. |
| `crates/web/src/components/chat.rs` | modify | Drop ChannelHeader. Keep MessageList. |
| `crates/web/src/components/right_rail.rs` | **new** | `<RightRail which>` wraps MemberList / PinnedPanel / thread-stub. 180 ms transform slide on mount; 120 ms cross-fade swap. |
| `crates/web/src/components/mod.rs` | modify | Wire new modules, drop server_list + sidebar. |
| `crates/web/src/icons.rs` | modify | Add icon_inbox, icon_compass, icon_phone, icon_user, icon_chevron_down, icon_chevron_right, icon_thread, icon_lock, icon_hourglass, icon_volume_1. Stroke 1.5, viewBox 24. |
| `crates/web/components.css` | **new** | All new shell styles. Loaded after style.css. Consume foundation tokens by name — no new hex. |
| `crates/web/index.html` | modify | Add `<link data-trunk rel="css" href="components.css" />` after style.css. |
| `crates/web/tests/browser.rs` | modify | `desktop_shell` module: grove-rail nav landmark, channel-sidebar nav landmark, main-header banner + 6 buttons in order, right-rail one-of-three. |
| `e2e/helpers.ts` | modify | Selectors: `.sidebar` → `.channel-sidebar, .sidebar`; `.member-list-wrapper` → `.right-rail[data-open="true"] .member-list`. Keep `.server-gear-btn` compat class on grove header chevron. `openMemberList` clicks `.action-btn[aria-label="members"]` on desktop. |

## Tasks (14 total)

1. Scaffold `.app-shell` grid wrapper + create components.css + wire trunk link. Keep legacy children; CSS targets both old + new class names via `.app-shell > .server-rail, .grove-rail` pattern.
2. Add 10 new icons to icons.rs with stroke 1.5 / viewBox 24 / Lucide-style paths.
3. Create GroveRail component with letters / divider / grove tiles (`data-state` variants) / new-grove / discover / spacer / settings. Include context menu + leave confirm migrated from server_list.rs.
4. Add roving ↓↑/Home/End keyboard on grove-rail nav; swap call site in app.rs; delete server_list.rs.
5. Create ChannelSidebar shell: grove header (glyph tile + italic name + grove chip tooltip + status row + tagline + chevron), scroll region frame, me-strip shell, net-status footer. Migrate channel-create input + delete-confirm from sidebar.rs.
6. Channel groups: classify channels into commons/voice/ephemeral/archives (voice via ChannelKind; ephemeral via `_ephemeral-` prefix; archives via `_archive-` prefix; commons default). CSS-uppercase labels, chevron toggle, hide-when-empty.
7. Channel row variants: text (idle/unread/current), voice (with listener pulse chip), ephemeral (whisper timer placeholder), muted. Full me-strip + net-status copy.
8. Swap Sidebar → ChannelSidebar in app.rs; delete sidebar.rs; drop legacy mobile overlays (return in 1b plan).
9. Create MainPaneHeader with six-button action bar + RightRailWhich enum. Delete ChannelHeader from chat.rs. Swap in app.rs.
10. Create RightRail wrapper; mount one of MemberList/PinnedPanel/thread-stub via `which` prop. 180 ms slide + 120 ms cross-fade. Remove legacy `.member-list-wrapper` + inline pinned mount from app.rs.
11. Desktop empty states: zero-channels centred italic copy in main pane; zero-members copy in right rail members view.
12. Edge cases: title attrs on ellipsised names (grove header, channel rows) for long-name tooltips; confirm rail scroll + settings pinned; emoji names via `system-ui` fallback.
13. Browser tests for desktop shell + e2e helper updates. `wasm-pack test --headless --firefox crates/web` green.
14. `just check` + manual smoke + push + `gh pr create --title "ui(phase-1a): desktop shell"`.

## Ambiguity decisions

- **Ephemeral / archive channels:** no first-class ChannelKind yet → name-prefix heuristic (`_ephemeral-` / `_archive-`). Swap to event-backed when kinds ship.
- **Muted channel state:** no state flag yet → `.channel-item--muted` CSS defined but not emitted until later phase.
- **Per-grove accent:** spec open question → all grove tiles use `--moss-2` for now; `[data-accent]` foundation still works globally.
- **Listener chip count:** read `app_state.voice.voice_participants_map` via context inside voice-row renderer.
- **`which` prop type:** `ReadSignal<RightRailWhich>` preferred; component accepts `Signal` alternative.
- **Call-active action-bar merge:** out of scope — six buttons unconditional; `on_call_click` hook for call-experience plan.
- **With-unread grove tile state:** CSS in place; data binding (per-grove unread aggregate) is follow-up when ServerState exposes it.
- **`e2e/helpers.ts` mobile branch:** `.member-list-wrapper` kept for mobile path until 1b; desktop branch rewired to `.right-rail[data-open="true"] .member-list`.

## Acceptance gates

1. `just check` green.
2. `wasm-pack test --headless --firefox crates/web` green (new `desktop_shell` module passes).
3. `npx playwright test --project=desktop-chrome` passes basic-flow + any specs touching sidebar/chat.
4. Visual: three-pane 68/232/flex/280 at ≥1280px; narrow overlay <960; 60px sidebar collapse <840; one-of-three right rail; reduced-motion path opacity-only.

## Self-review

- [x] Every desktop §Acceptance row mapped to a task.
- [x] Every foundation token consumed by name, no hex.
- [x] Every commit is `ui(phase-1): <imperative>`.
- [x] `e2e/helpers.ts` selectors updated in same commit as markup rename (feedback_e2e_in_sync memory).
- [x] No placeholders.
