# UI Phase 1f — Notifications Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development + superpowers:test-driven-development.

**Goal:** `docs/specs/2026-04-19-ui-design/notifications.md` — in-app toast stack, unread badges (channel rows + grove tiles + mobile tab bar), OS push payload contract, chime sound, per-surface mute overrides.

**Style ref:** `docs/plans/2026-04-19-ui-phase-0-foundation.md`. Commits: `ui(phase-1): <imperative>`. Branch `design/ui-target-ux`. After 1a/1b/1c/1d/1e.

## Scope

**In:** toast primitive + stack + aria-live routing, unread-badge derivation + rendering, `MuteChannel` + `MuteGrove` event kinds (per-identity, not admin-gated), service-worker push handler + payload contract, chime player, Notifier service (gating + coalescing), context-menu mute + settings placeholder.

**Out:** settings-tweaks full notifications panel (Phase 1g), VAPID wire format / relay-side push dispatch (follow-up), whisper/handoff/ephemeral sender emission (their phases call `Notifier::dispatch`).

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/web/src/components/toast.rs` | **new** | Portal-mount at `#toast-root`, 3 visible cap + overflow pill, willow-pop-in enter / opacity exit, dedup_key replacement, hover-pause, keyboard (`Ctrl+Alt+N` focus newest, `Esc` dismiss, `Tab` cycle), aria-live routing (status vs alert). `Toast` struct + builder API. |
| `crates/web/src/notifications.rs` | **new** | `Notifier` service — dispatch decision point. Gates by own-event / category / global-mute / quiet-hours / per-surface-mute / focus. 20s coalescing per surface. `NotificationKind` enum. `Category {Msg, Mention, Letter, EphemeralExpiry, WhisperInvite, Handoff}`. |
| `crates/web/src/audio.rs` | **new** | `ChimePlayer` wrapping `HtmlAudioElement`, preload `/willow-chime.webm`, max queue depth 1. |
| `crates/web/willow-chime.webm` | **new** | Placeholder 400ms silent opus (generate via ffmpeg once). Replace with mastered asset later. |
| `crates/web/index.html` | modify | `<link data-trunk rel="copy-file" href="willow-chime.webm">` + `<div id="toast-root" aria-live="polite" aria-relevant="additions">` as last body child. |
| `crates/web/sw.js` | modify | Rewrite: install/activate unchanged, add `push` handler + `notificationclick` handler. Payload contract: `{ wake: 1, ref: <base64url-ciphertext>, cat: "msg|mention|letter|ephemeral-expiry|whisper-invite|handoff" }`. No author/body/grove/peer fields. If focused client → postMessage; else `registration.showNotification` with opaque title. |
| `crates/web/src/main.rs` | modify | Register service worker; inject `Notifier`, `ChimePlayer`, upgraded `UnreadStatsView` into context. `navigator.serviceWorker.onmessage` → Notifier dispatch. |
| `crates/web/src/app.rs` | modify | Mount `<ToastStackView>` once. Wire visibilitychange → document.title `(N) willow` prefix. Register `keydown` for `Ctrl+Alt+N` / `Esc`. |
| `crates/web/src/components/unread_badge.rs` | **new** | `<UnreadBadge stats: UnreadStats, dot: bool>`. Priority: whisper > mentioned > announce_only > muted > default. `99+` threshold. Mentioned `@` prefix. Outlined muted variant. aria-label. |
| `crates/web/src/components/sidebar.rs` (channel rows) | modify | Prop type `ReadSignal<HashMap<SurfaceId, UnreadStats>>`. Render `<UnreadBadge>`. Context-menu entry `"mute channel" / "unmute channel"` calls `client.mutate_channel_mute`. |
| `crates/web/src/components/server_list.rs` (grove rail) | modify | Aggregate via `UnreadView::for_server(&channels)` → `<UnreadBadge>` on each tile. |
| Mobile tab bar (from 1b) | modify | 6×6 dot when unfocused; pill when focused/long-pressed. |
| `crates/state/src/event.rs` | modify | Add `MuteChannel { channel_id, muted }` + `MuteGrove { muted }` variants. Not admin-gated. |
| `crates/state/src/types.rs` | modify | `MuteState { channels: HashSet<String>, grove_muted: bool }`. |
| `crates/state/src/server.rs` | modify | `ServerState::mute_state: HashMap<EndpointId, MuteState>`. |
| `crates/state/src/materialize.rs` | modify | Handle `MuteChannel` + `MuteGrove` in apply_event. |
| `crates/state/src/tests.rs` | modify | 4 tests: roundtrip channel, roundtrip grove, mute-unmute idempotent, not-admin-gated. |
| `crates/client/src/views.rs` | modify | `UnreadStats { count, mentioned, whisper, announce_only, muted }` + `SurfaceId` enum. `UnreadView { stats: HashMap<SurfaceId, UnreadStats> }` + `counts()` back-compat shim. `for_server` aggregator. `compute_unread_view` derives from registry + mute_state + messages. |
| `crates/client/src/mutations.rs` | modify | `mutate_channel_mute` + `mutate_grove_mute`. |
| `crates/client/src/events.rs` | modify | `ClientEvent::MuteChanged { scope: MuteScope, muted }`. |
| `crates/web/foundation.css` | modify | Toast + unread-badge + tab-bar-dot CSS. Reduced-motion path. |
| `crates/web/style.css` | modify | Delete legacy `.channel-item .unread-badge` amber rule. |
| `crates/web/src/components/mod.rs` | modify | Register new modules. |
| `crates/web/tests/browser.rs` | modify | `notifications` module: badge 99+/variants/muted aria; toast polite+alert live region; dedup replaces; overflow pill beyond 3. |

## Tasks (15)

1. Upgrade `UnreadView` in willow-client to `UnreadStats` per-surface + back-compat `counts()` shim. Update callers via shim.
2. Add `MuteChannel`/`MuteGrove` EventKind + `MuteState` + apply handlers. 4 state tests.
3. `mutate_channel_mute`/`mutate_grove_mute` + `ClientEvent::MuteChanged`. 3 client tests.
4. Toast primitive + portal mount + stack cap + dedup + auto-dismiss.
5. Severity variants (info/ok/warn/err) + builder API + icon + aria-role routing (status vs alert).
6. `UnreadBadge` component + priority variants + 99+ threshold.
7. Plumb badges into channel rows / grove tiles / mobile tab bar. Delete legacy amber rule.
8. Chime asset + ChimePlayer + queue-depth-1.
9. Notifier service — dispatch + gating + 20s coalescing + own-event suppress + focus gate + permission-denied sticky toast.
10. Toast keyboard — Ctrl+Alt+N focus newest / Esc dismiss / Tab cycle / Enter activate / hover-pause.
11. Service-worker push handler + permission prompt after first send + postMessage bridge.
12. Per-surface mute UI — channel-row context-menu + settings mute-grove toggle placeholder.
13. A11y sweep — aria-live, sr-only badge label, document.title `(N) willow` prefix, reduced-motion overrides for badge.
14. Browser tests `notifications` module (7 tests).
15. `just check` + visual smoke + Phase 1f PR.

## Ambiguity decisions

- **Chime asset placeholder** — ship 400ms silent opus. Replace with mastered later.
- **Mention detection** — last-500-messages substring heuristic on local peer short-id. Full parsing in message-row.md.
- **Tab-bar badge** — if 1b hasn't landed, leave `TODO(phase-1f)` comment.
- **MuteState shape** — per-identity `HashMap<EndpointId, MuteState>` on ServerState.
- **Two variants vs one `MuteSurface`** — Two concrete `MuteChannel`/`MuteGrove` variants for simpler pattern-match.
- **VAPID deferred** — service-worker scaffold only.
- **Session quiet-hours-announced reset** — page load = session.
- **Permission prompt** — after first local `MessageReceived { is_local: true }` per session.

## Acceptance gates

1. `just check` green.
2. `just test-state` green (4 new mute tests).
3. `just test-client` green (3 new).
4. `just test-browser` green (7 new in `notifications` module).
5. Visual: background-channel message → toast + chime + badge increment; mute channel → silent, outlined pill, count still increments; `99+` at >99; reduced-motion emulation fades instead of slides; tab title `(N) willow` while hidden, strips after 1s visible; permission-denied sticky toast verbatim `"willow works better with notifications — settings lets you pick what's loud"`; Ctrl+Alt+N focuses newest; coalesced dedup replaces within 20s.

## Self-review

- [x] Every §Acceptance criterion mapped (29 items).
- [x] Privacy: OS push payload never leaks text.
- [x] aria-live routing polite/alert matches severity.
- [x] 44×44 touch targets on mobile toast actions.
- [x] Every task → commit `ui(phase-1): <imperative>`.
- [x] Copy lowercase / no exclamation marks.
