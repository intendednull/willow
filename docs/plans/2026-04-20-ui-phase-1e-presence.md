# UI Phase 1e — Presence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development + superpowers:test-driven-development.

**Goal:** `docs/specs/2026-04-19-ui-design/presence.md` — 7-state catalog (here/away/whispering/in a call/queued·N/gone/invisible), `StatusDot` + `PeerStatusLabel` atoms on every peer surface, self-presence override menu.

**Style ref:** `docs/plans/2026-04-19-ui-phase-0-foundation.md`. Commits: `ui(phase-1): <imperative>`. Branch `design/ui-target-ux`. After 1a/1b/1c/1d.

## Scope

**In:** pure `PresenceState` enum + `derive_peer_presence` + `derive_self_presence`, `PresenceMeta` actor, `PresenceView` derived, `ClientHandle::set_self_presence / observe_peer_presence`, `StatusDot` + `PeerStatusLabel` atoms, integration on member rows / me strip / grove drawer me strip / profile-card stub / message author / participant tile, presence menu, settings scaffold.

**Out (deferred stubs):** `PresenceHeartbeat` EventKind (stubbed via 1s tick), `SetInvisible` signal, settings-tweaks threshold UI. HLC swap: mechanical replacement for tick counter.

## Architecture

Presence is **derived, not event-sourced** in 1e. Inputs: `willow-network` reachability + `willow-client` voice membership + whisper registry stub + sync-queue depth stub + local override. `PresenceMeta` actor holds tick counter, last_seen, queue_depth, whispering_with, invisible_to_me, self_override, idle/gone thresholds. Tick driver advances 1/s and updates last_seen for online peers.

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/client/src/presence.rs` | **new** | `PresenceState` enum (7 + Unknown). `PresenceOverride` enum (Auto/Away/Gone/Invisible). `Tick = u64`. `PresenceInputs`. Pure `derive_peer_presence(inputs)` + `derive_self_presence(override, reachable, in_call, whispering)`. Precedence: invisible > whispering > in_call > queued > gone > away > here. 8 unit tests. |
| `crates/client/src/state_actors.rs` | modify | Add `PresenceMeta` struct (tick, last_seen, queue_depth, whispering_with, invisible_to_me, self_override, idle_ticks=360, gone_ticks=172_800). |
| `crates/client/src/views.rs` | modify | Add `PresenceView { per_peer: HashMap<EndpointId, PresenceState>, self_state: PresenceState }` + `compute_presence_view`. |
| `crates/client/src/lib.rs` | modify | Spawn `presence_meta_addr`. Add `ClientHandle::observe_peer_presence(peer_id)`, `set_self_presence(override_)`, stub helpers `_set_whispering_with`, `_set_queue_depth`. 2 client tests (round-trip + default=Here). |
| `crates/client/src/mutations.rs` | modify | `SetSelfPresence` message routing. |
| `crates/client/src/connect.rs` | modify | Tick-driver task: 1s sleep, advance tick + update last_seen for current peers. Use `tokio::spawn` (native) / `wasm_bindgen_futures::spawn_local` (wasm). |
| `crates/web/src/components/status_dot.rs` | **new** | `<StatusDot state, size, border, ambient>`. Sizes: Profile(13), Row(9), Rail(10), MeStrip(8), Author(9), CallTile(14). Border surface Bg0/Bg1. Renders filled disk (here/away/gone), ring (in-a-call), hourglass glyph (queued), ear glyph (whispering), nothing (invisible). aria-label = `"status: {label}"`. Pulses on `here`/`whispering` when `ambient=true`. |
| `crates/web/src/components/peer_status_label.rs` | **new** | `<PeerStatusLabel state, show_dot>`. Icon (ear/hourglass) + optional dot + text. `queued · N` with mono 12px `--amber` count. Invisible → null. |
| `crates/web/src/components/presence_menu.rs` | **new** | `<PresenceMenu open, on_close>`. Menu with entries auto/away/gone/invisible. Calls `set_self_presence`. role="menu". |
| `crates/web/src/icons.rs` | modify | Add `icon_ear()` + `icon_hourglass()` 11×11 at stroke 1.5 (keep full-size icon_hourglass from 1a intact). |
| `crates/web/foundation.css` | modify | Append `.status-dot--*`, `.peer-status-label--*`, `.presence-pulse`, `.peer-status-label__count` (mono 12px amber). Don't recolour on accent swap. |
| `crates/web/src/components/member_list.rs` | modify | Swap ad-hoc `.status-dot` div for `<StatusDot size=Rail border=Bg1>` + tooltip wrapper with `<PeerStatusLabel>`. |
| `crates/web/src/components/sidebar.rs` (or post-1a equivalent) | modify | Me-strip: swap ad-hoc `.status-dot` for `<StatusDot size=MeStrip>` + `<PeerStatusLabel state=self show_dot=false>` + chevron triggering `<PresenceMenu>`. `aria-live="polite"` on trigger announces self-state changes. |
| Grove drawer me strip (post-1b) | modify | Same swap. |
| `crates/web/src/components/participant_tile.rs` | modify | Swap for `<StatusDot size=CallTile border=Bg0>`. |
| `crates/web/src/components/message.rs` | modify | Author avatar: add 9px `<StatusDot size=Author border=Bg0>`. |
| Profile-card stub (from 1b) | modify | Compose `<StatusDot size=Profile ambient=false>` + `<PeerStatusLabel>`. |
| `crates/web/src/components/settings.rs` | modify | Add Presence tab/section stub with same 4 override buttons. |
| `crates/web/src/components/mod.rs` | modify | Register new modules. |
| `crates/web/tests/browser.rs` | modify | `presence_atom` module: 8 tests (per-state render, ear icon, hourglass icon, count, aria labels, invisible renders nothing, pulse class on here/whispering, reduced-motion disables pulse animation). |

## Tasks (14)

1. `crates/client/src/presence.rs` pure derivation + 8 unit tests.
2. `PresenceMeta` actor in `state_actors.rs` + `ClientHandle` wiring.
3. `PresenceView` + `compute_presence_view` + `observe_peer_presence` / `set_self_presence`. 2 client tests.
4. `StatusDot` atom + foundation.css styles + icon_ear + icon_hourglass (11×11 variant).
5. `PeerStatusLabel` atom + CSS.
6. Integrate StatusDot into member rows + me strip + grove drawer me strip.
7. Integrate StatusDot + PeerStatusLabel into profile-card stub.
8. `PresenceMenu` + me-strip trigger + settings Presence section.
9. Participant tile ring for `in a call`.
10. Stub helpers `_set_whispering_with` / `_set_queue_depth` for later phases.
11. Tick driver in `connect.rs` (WASM-safe via cfg). `queued_then_gone_after_threshold` test.
12. Reduced-motion verification (CSS already handles via foundation pulse override).
13. Browser tests `presence_atom` module (8 tests).
14. `just check` + visual smoke + Phase 1e PR.

## Ambiguity decisions

- **Heartbeat events deferred** — tick driver + last_seen updates stub at the seam. Plan intro flags for later phase.
- **Invisibility deferred** — `invisible_to_me` set stays empty. Self-override Invisible hides self-dot.
- **Tick vs HLC** — `u64` tick, 1 tick = 1 second. `saturating_sub` semantics transfer.
- **Sync-queue depth** — stubbed mutator only; real queue ships in `sync-queue.md`.
- **Profile-card stub location** — grep for Phase 1b profile-card component in Task 7.
- **Settings thresholds** — hardcoded defaults 360/172800. Settings form in future phase.

## Acceptance gates

1. `just check` green.
2. `cargo test -p willow-client presence` green (8 tests).
3. `just test-browser` green with `presence_atom` module.
4. Visual: member rows / me strip / message authors / call tiles all show new dot; override menu has auto/away/gone/invisible; ring on in-call; reduced-motion freezes pulse but colour transitions still work; `data-accent="ember"` doesn't recolour presence dots.

## Self-review

- [x] Every §Acceptance row mapped.
- [x] 7 colour-independent cues verified.
- [x] Touch target ≥ 44×44 on me-strip trigger.
- [x] Reduced-motion path covered.
- [x] Commits `ui(phase-1): <imperative>`.
- [x] No new EventKind (stubs only).
