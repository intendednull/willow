# UI Phase 1b — Mobile Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Use superpowers:test-driven-development within each task.

**Goal:** Mobile portion of `docs/specs/2026-04-19-ui-design/layout-primitives.md` — tab bar, top bar, grove drawer, full-screen pushes, bottom sheets, gestures, 721px breakpoint.

**Style ref:** `docs/plans/2026-04-19-ui-phase-0-foundation.md`. Match: agentic-workers header, File table, checkbox steps, exact paths/code/commands, Acceptance gates, Self-review, PR task. Commits: `ui(phase-1): <imperative>`.

**Spec:** `docs/specs/2026-04-19-ui-design/layout-primitives.md` — §Mobile layout through §Breakpoint switch.

**Branch:** `design/ui-target-ux`. Assumes 1a desktop shell landed.

## Scope

**In:** Viewport + breakpoint + safe-area / Tab bar (iOS blur, Android pill, 4 tabs, badges) / Top bar (primary + pushed) / Grove drawer (280px, backdrop, rail+body+me strip, gestures) / Full-screen pushes (translateX, back gesture) / Bottom sheet primitive / Gesture table / Mobile transitions / Mobile empty states.

**Out:** desktop (1a), command palette (1c), content internals, new events.

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/web/src/components/mobile_shell.rs` | **new** | Top-level mobile shell. MobileTab enum + MobilePush enum + push stack + drawer signal. Renders top bar / body / tab bar / drawer / sheet. |
| `crates/web/src/components/tab_bar.rs` | **new** | 4-tab bottom bar. Reads active, emits clicks, renders badges. iOS blur via `[data-platform="ios"]` on root; Android pill via `[data-platform="android"]`. |
| `crates/web/src/components/grove_drawer.rs` | **new** | 280px slide overlay. Backdrop + transform slide + swipe-left close + Escape close. Internal: 64px rail column + 216px body column + me-strip footer. |
| `crates/web/src/components/bottom_sheet.rs` | **new** | Reusable primitive. Handle, translateY anim, backdrop, dismiss on backdrop-tap/swipe-down/Escape. Takes children. |
| `crates/web/src/components/mod.rs` | modify | Register new modules. |
| `crates/web/src/app.rs` | modify | Wrap existing `.app` in `.shell-desktop`; mount `<MobileShell>` under `.shell-mobile` sibling. Media query hides inactive. Set `data-platform` on `#app-root` from UA once at boot. |
| `crates/web/src/icons.rs` | modify | Add icon_compass, icon_user, icon_thread if not present in 1a. |
| `crates/web/style.css` | modify | Add mobile-shell CSS + shell toggle `@media (max-width: 720px)`. Delete legacy `@media (max-width:640px)` + `(max-width:900px)` hamburger blocks. Keep `.mobile-action-sheet` block (reused by message-row long-press). |
| `crates/web/foundation.css` | modify | Append `--shell: desktop/mobile` CSS custom property toggle at 720px. |
| `e2e/helpers.ts` | modify | Rewrite `openSidebar` to tap `.mobile-top-bar .top-slot-left` on mobile. Add `switchTab(page, tabId)`. Update `.sidebar.open` → `.grove-drawer.open`. |
| `e2e/mobile.spec.ts` | modify | Replace hamburger assertions with tab-bar + drawer assertions (see Task 10). |
| `e2e/multi-peer-mobile.spec.ts` | modify | Swap selector assertions to `.grove-drawer.open`. |

## Tasks (11 total)

1. **Shell split** — Wrap `.app` in `.shell-desktop`, add `.shell-mobile` with `<MobileShell>` sibling. Media query toggles at 721px. Set `data-platform="ios|android|web"` on `#app-root` once at boot from UA. Delete legacy hamburger media blocks from style.css.
2. **`mobile_shell.rs` skeleton** — MobileTab {Home, Letters, Discover, You}. MobilePush {Channel(String), Thread(String), Call, Onboarding}. Local signals: `active_tab`, `push_stack`, `drawer_open`. Empty body placeholder; render stub tab bar.
3. **`tab_bar.rs`** — `<TabBar active, badges>`. 4 tabs w/ icons + labels. Badge 14×14 top-right w/ `--err` bg. iOS blur background branch; Android pill on active via `[data-platform="android"] .tab.active::before`.
4. **Top bar** — Inside MobileShell: left slot (grove glyph if root, back chevron if pushed), title (italic display, current push name or server name), subtitle (peer count), right slot (search button wired to `set_show_palette`).
5. **Grove drawer** — `<GroveDrawer open, ...>`. 280px `translateX(-100%) → 0` over 240ms. Backdrop fade. Internal: rail column (grove tiles) + body column (wordmark + grove list + me strip). Close on backdrop tap / swipe-left / Escape / grove select.
6. **Full-screen pushes** — Render push stack via `<For>`. Each push: `position: absolute; inset: 0; translateX` keyframe 240ms. Tab bar hidden when `push_stack.is_empty() == false`. Wire channel click → `push_stack.push(Channel(name))` + existing `on_channel_click`. Back chevron pops stack.
7. **`bottom_sheet.rs`** — `<BottomSheet open, label, children>`. Handle + translateY anim + Escape listener. Keep existing `.mobile-action-sheet` block (message-row owns its internals) — this is new reusable primitive for profile-sheet / confirm-sheet / etc.
8. **Gestures** — Edge-swipe open drawer (pointerdown x ≤ 20, pointerup dx > 60). Swipe-left close drawer (dx < -60). Swipe-down close sheet (dy > 120). All reduced-motion: transform → opacity fade same duration.
9. **Mobile empty states** — Zero groves home: `no groves yet — start one` + CTA. Zero channels: `this grove is quiet. say hi?` + hint `add a channel from the grove menu.` Letters/Discover/You are placeholder empty-states pending later specs.
10. **E2E + browser tests** — Update `e2e/helpers.ts` (`openSidebar` → tap top-slot-left). Update `e2e/mobile.spec.ts` (tab-bar visible on primary, hidden on push; drawer opens/closes). Update `e2e/multi-peer-mobile.spec.ts` selectors. Browser test: `mobile_shell` module asserts tab bar 4 tabs + `aria-label="primary"` + `data-platform="ios"` swaps bg.
11. `just check` + visual smoke + `gh pr create --title "ui(phase-1b): mobile shell"`.

## Ambiguity decisions

- **Home tab badge aggregates per-grove unread** (sum of `app_state.server.unread`).
- **Push animation via keyframe** (not transition — newly-mounted elements don't trigger transitions).
- **`.shell-desktop` uses `display: contents`** to preserve existing flex.
- **Legacy `.mobile-nav-toggle`** stays (desktop narrow collapse <840 uses it); mobile shell overrides it entirely at ≤720.
- **Platform UA sniff one-shot at boot** — no reactive re-detection.
- **Gesture closures `forget()`** consistent with existing `app.rs` patterns.
- **Real chat stack in `MobilePush::Channel`**: replace placeholder `<div class="push-placeholder">` with `<ChannelHeader>` + `<MessageList>` + `<ChatInput>` wiring in same task.
- **`.mobile-action-sheet` stays** — new `BottomSheet` is parallel primitive; migrate later.

## Acceptance gates

1. `just check` + `just check-wasm` green.
2. `just test-browser` — `mobile_shell` module green.
3. `npx playwright test --project=mobile-chrome e2e/mobile.spec.ts e2e/mobile-actions.spec.ts e2e/multi-peer-mobile.spec.ts` green.
4. Visual: 721px boundary toggles shells; iOS viewport → blurred tab bar; Android viewport → active pill; pushes hide tab bar; drawer + sheet + drawer backdrop all work; reduced-motion collapses slides to fades.

## Self-review

- [x] Every spec §Acceptance row mobile-related mapped.
- [x] Foundation tokens only; safe-area-inset vars.
- [x] Touch targets ≥ 44×44.
- [x] Reduced-motion path for drawer + sheet + push.
- [x] E2E helpers updated same commit as markup.
- [x] No placeholders.
