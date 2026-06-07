# UI Phase 3c — Reactions & Pins Implementation Plan

**Date:** 2026-05-08
**Status:** landed (foundation PR #634 + picker wireup PR #635 + initial close-out PR #637 + cascade close-out PRs #643 visibility wiring, #644 pinned-by footer + `PinMetadata`, #646 plan/spec/index close-out, #647 same-channel react-recency refresh, #648 DOM coverage tests)
**Spec:** [`docs/specs/2026-04-19-ui-design/reactions-pins.md`](../specs/2026-04-19-ui-design/reactions-pins.md)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development + superpowers:test-driven-development. Every task = one commit; tick the checkbox in the same commit.

**Goal:** Ship `docs/specs/2026-04-19-ui-design/reactions-pins.md` — replace the placeholder hover-toolbar emoji shelf with a real emoji picker popover, polish the reactions strip with the spec'd reactor tooltip + add-reaction hover chip + channel-scoped recency, and bring the pinned-panel + header-pin IconBtn up to spec (amber tint + superscript count + spec'd entry layout + per-entry unpin gated on `ManageChannels`).

**Architecture:** All authority + state already exists in `willow-state` — `EventKind::Reaction` / `PinMessage` / `UnpinMessage` plus `Channel::pinned_messages` and `ChatMessage::reactions`. This phase is pure UI on top of already-shipped state. The big new component is `<EmojiPicker>` — a 320 × 360 popover with search + categories + recents that reuses for the row's add-reaction chip, the row's `smile` more-reactions toolbar button, and the composer's emoji IconBtn (binding owned by `composer.md`, lands here too as a wire-up). Channel-scoped reaction recency is a new derived signal: a 5-deep LRU keyed by channel id, fed by every successful `react()` call. The pinned-panel polish is mostly visual + adding the per-entry unpin button gated on `ManageChannels`. The header pin IconBtn picks up an amber tint + mono superscript count from the existing `pinned_message_ids(channel)` view.

**Tech Stack:** Rust + Leptos 0.8 (WASM), `willow-client` (1 new view derivation for channel-scoped reaction recency), `willow-web` (`<EmojiPicker>` + `<ReactorTooltip>` + `<AddReactionChip>` components, pinned-panel rewrite, header pin-button enhancements). Foundation tokens only (no new hex). No new EventKinds; the phase is pure UI on top of already-shipped state. `just test-state` (already-passing reaction / pin tests are the foundation), `just test-client`, `just test-browser`, `just test-e2e-*` run in CI.

**Branch:** `ui/phase-3c-reactions-pins`. Commits `ui(phase-3c): <imperative>` (code), `client(phase-3c): <imperative>` for the recency derivation, `docs(plan): phase 3c — reactions & pins implementation plan` for this plan. Lands as one PR; the plan ships in the same PR as the implementation. The first commit also stamps `files-inline.md` + the phase-3b plan as `landed (e98ed26, 2026-05-08)` (carry-over from the phase-3b merge that protected-main blocked from a direct push).

---

## Scope

**In:**
- New `<EmojiPicker>` popover: 320 × 360 on `--bg-1` border `--line` radius 10 `--shadow-2`. Top mono `search emoji` input. Scrollable category sections (recent / smileys / nature / food / travel / objects / symbols). Arrow keys move selection, `Enter` inserts, `Escape` closes.
- Reusable `<EmojiPicker>` invocations:
  - Row add-reaction chip (this PR — new component).
  - Row hover toolbar `smile` more-reactions button (this PR — replaces the placeholder shelf).
  - Composer emoji IconBtn (this PR — wires the existing `on_emoji` stub into the same picker).
- `<AddReactionChip>` desktop-only hover affordance: dashed `--line` border, `plus` + `smile` icons, `--ink-3`. Click opens `<EmojiPicker>`. Hidden on mobile (action sheet entry already exists).
- `<ReactorTooltip>` desktop hover on a reaction pill: `mira, ori, kes reacted` for ≤ 3 reactors, `first two, and N others` past 3. Mobile tap-and-hold exposes the same list as a small card.
- Reaction-strip polish: spec'd geometry (`5px` gap, `2px 8px` padding, radius 999, `--bg-2` on `--line`); local-user pill uses `--moss-1` border + tinted bg; chip-style toggle on click (already wired through `on_react`).
- Channel-scoped reaction recency: `client.recent_reactions(channel_id) -> [String; 5]` LRU keyed by channel id, fed by every successful `react()` call. Falls back to spec default `👍 ❤️ 🍃 💚 👀` until 5 distinct emojis have been reacted in the channel. Used by the row hover toolbar + mobile action sheet quick-react row + `<EmojiPicker>` "recent" category.
- Pinned-panel rewrite to spec: entry layout = avatar (24 px) + display name (body weight 500) + timestamp (`--ink-3`, 11 px) + 2-line body preview ellipsis + optional `pinned by {name} · {when}` footer. Right-aligned per-entry actions on hover / always on mobile: `jump to` (existing wiring) + `unpin` (new, permission-gated). Empty state copy `nothing pinned yet.` (italic, `--ink-3`, 13 px). Header copy unchanged.
- Header pin IconBtn: tints `--amber` when `pinned_message_ids(channel).len() > 0`; mono superscript count overlay when count > 0. Existing toggle behaviour unchanged.
- Permission gating: pin / unpin actions render greyed (no-op) for users without `ManageChannels`, with `aria-disabled="true"` and tooltip `only stewards can pin here`. Applies to the row pin / unpin entry (overflow menu / action sheet) AND the pinned-panel per-entry unpin button.
- ARIA labels per spec table: `react with {emoji}` on quick-react buttons; `{emoji} reacted by {count} — toggle your reaction` on reaction pills; `add reaction` on the add-chip; `pinned messages ({count})` on the header IconBtn; `jump to pinned message` + `unpin message` on per-entry buttons.
- Browser tests for: reaction-strip render + toggle, add-chip click → picker open, picker arrow + enter insert, picker escape close, picker search filter, reactor tooltip 1-2-3-4+ pluralisation, channel-scoped recency LRU, pinned-panel render with all entry fields, pinned-panel unpin gated on permission, header pin IconBtn amber tint + superscript count, ARIA label coverage.

**Out (hand-off / explicit defer):**
- Cross-grove reaction-recency persistence — channel-scoped only in v1 per spec §Open questions.
- Reactions on deleted messages — already allowed in v1 per spec §Open questions; no code change needed (the existing materialize handler already permits it).
- Pinned-panel cross-channel "all pinned in this server" view — out of scope per spec.
- The composer emoji IconBtn shortcode autocomplete (`:thumbsup:` → 👍 expansion) — owned by `composer.md` §Mention autocomplete, deferred to a follow-up that polishes both autocompletes together.

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/client/src/actions.rs` | modify | Add `ClientHandle::recent_reactions(channel) -> Vec<String>` (delegates to `ChatMeta::recent_reactions`); `ClientHandle::react` now calls `note_reaction` after the mutation succeeds. |
| `crates/client/src/state_actors.rs` | modify | Add `ReactionRecency { per_channel: HashMap<String, VecDeque<String>> }` actor + `note(channel_id, emoji)` + `recent(channel_id) -> Vec<String>` (cap 5). |
| `crates/web/src/components/emoji_picker/mod.rs` | **new** | `pub mod picker; pub mod categories; pub use picker::EmojiPicker;` plus the static category data table (smileys / nature / food / travel / objects / symbols). |
| `crates/web/src/components/emoji_picker/picker.rs` | **new** | `<EmojiPicker open recent on_select on_close>` — 320 × 360 popover with mono search input + category-scrolled grid. Handles arrow / Enter / Escape keyboard. Reads `recent` from props (caller threads in `client.recent_reactions(channel_id)`). |
| `crates/web/src/components/emoji_picker/categories.rs` | **new** | Static `EMOJI_CATEGORIES: &[(&str, &[&str])]` data + a small `search(query, recent) -> Vec<&str>` helper used by the picker's filter. Pure-function unit tests. |
| `crates/web/src/components/reactions/mod.rs` | **new** | `pub mod strip; pub mod tooltip; pub mod add_chip;` plus `pub use` re-exports. The reactions surfaces currently inlined in `message.rs` move here. |
| `crates/web/src/components/reactions/strip.rs` | **new** | `<ReactionStrip>` — flex row of `<ReactionPill>` + `<AddReactionChip>` on hover. Spec'd geometry + local-user style. Reads `pills` + `local_peer_id` + `on_react` from props. |
| `crates/web/src/components/reactions/tooltip.rs` | **new** | `<ReactorTooltip>` — desktop hover `title` + mobile press-and-hold popover. `1-2-3 reactors`, `first two, and N others` past 3. |
| `crates/web/src/components/reactions/add_chip.rs` | **new** | `<AddReactionChip>` — desktop-only dashed-border chip with `plus` + `smile`. Click opens `<EmojiPicker>` via a callback. |
| `crates/web/src/components/message.rs` | modify | Replace the inline reactions-strip render at line 1515 with `<ReactionStrip>`. Replace the placeholder quick-react shelf at line 1083 with a `<EmojiPicker>` invocation that uses `client.recent_reactions(channel_id)` for the "recent" category. Wire the `<AddReactionChip>` into the strip on desktop-only. |
| `crates/web/src/components/pinned.rs` | rewrite | Match spec entry layout (avatar + display name + timestamp + 2-line preview + optional `pinned by {name} · {when}` footer). Add per-entry `unpin` button (permission-gated). Empty state copy `nothing pinned yet.` Header copy unchanged. ARIA labels per spec. |
| `crates/web/src/components/main_pane_header.rs` | modify | Header pin IconBtn picks up `--amber` tint when `pinned_message_ids(channel).len() > 0` and renders a mono superscript count overlay when > 0. ARIA `pinned messages ({count})`. |
| `crates/web/src/components/composer/composer.rs` | modify | Wire the existing `smile` emoji IconBtn click handler to open the shared `<EmojiPicker>` (currently a TODO callback per phase-3a §Out of scope). |
| `crates/web/src/components/chat.rs` | modify | Permission-gate the pin / unpin keyboard binding (`P` on focused row) on `ManageChannels` — currently fires for everyone. |
| `crates/web/style.css` | modify | Append `.emoji-picker`, `.emoji-picker__search`, `.emoji-picker__category-header`, `.emoji-picker__cell`, `.emoji-picker__cell--selected`, `.reactions-strip`, `.reaction-pill`, `.reaction-pill--reacted`, `.add-reaction-chip`, `.pinned-entry`, `.pinned-entry__avatar`, `.pinned-entry__meta`, `.pinned-entry__body`, `.pinned-entry__footer`, `.pinned-entry__actions`, `.pinned-empty`, `.action-btn--lit`, `.action-btn__count--pin`. Foundation tokens only — `--bg-1`, `--bg-2`, `--line`, `--moss-1`, `--moss-2`, `--ink-0`, `--ink-1`, `--ink-2`, `--ink-3`, `--amber`, `--shadow-2`. |
| `crates/web/tests/browser.rs` | modify | Append `mod phase_3c_reactions_pins { … }` — ~14 tests covering AG-1 through AG-13. |
| `docs/specs/2026-04-19-ui-design/files-inline.md` | modify | Stamp `**Status:** implementing → landed (e98ed26, 2026-05-08)`. (Carry-over from phase 3b — main is protected and the merge happened upstream.) |
| `docs/plans/2026-05-08-ui-phase-3b-files-inline.md` | modify | Add `**Status:** landed (e98ed26, 2026-05-08)` — carry-over. |

## Acceptance gates

> Each gate maps to one or more browser/state/client tests. A task is complete when (a) the production code lands, (b) the corresponding test(s) pass, (c) the spec checkbox is ticked in the same commit.

- [x] **AG-1.** Reaction strip renders pills with spec geometry; clicking a pill toggles via `on_react`. Local-user pill picks up `--moss-1` border + tinted bg.
- [x] **AG-2.** `<AddReactionChip>` appears on row hover (desktop-only); click opens `<EmojiPicker>`. Hidden on mobile.
- [x] **AG-3.** `<EmojiPicker>` opens at 320 × 360, with search input + category headers + recents row + grid cells. Arrow keys move highlight; `Enter` inserts via `on_select`; `Escape` closes via `on_close`.
- [x] **AG-4.** `<EmojiPicker>` search filter narrows the grid to matching glyphs.
- [x] **AG-5.** `<ReactorTooltip>` shows the spec copy: `mira, ori, kes reacted` for ≤ 3 reactors, `first two, and N others` past 3.
- [x] **AG-6.** `client.recent_reactions(channel_id)` returns the 5 most-recent reactions in the channel, MRU-first; falls back to the spec default until the LRU fills. Per-channel — different channels keep separate recency.
- [x] **AG-7.** Pinned panel renders entries with avatar + display name + timestamp + 2-line preview + optional `pinned by {name} · {when}` footer. Empty state copy is byte-exact `nothing pinned yet.`. *(Footer + `PinMetadata` state-schema change landed in the phase-3c close-out PR — see `docs/specs/2026-05-21-pinned-message-metadata-design.md`.)*
- [x] **AG-8.** Pinned panel `unpin` button is permission-gated: greyed + `aria-disabled="true"` + tooltip `only stewards can pin here` when local peer lacks `ManageChannels`.
- [x] **AG-9.** Header pin IconBtn tints `--amber` when channel has pins; superscript count overlay shows the count.
- [x] **AG-10.** Pin / unpin keyboard binding (`P`) on a focused row is permission-gated.
- [x] **AG-11.** Composer emoji IconBtn opens the same `<EmojiPicker>` as the row's add-reaction chip.
- [x] **AG-12.** ARIA labels match the spec table for every interactive element added in this phase.
- [x] **AG-13.** Spec `reactions-pins.md` acceptance criteria checkboxes are ticked at the end of the plan (final task).

## Tasks

> Each task is one self-contained commit. Subagent contract: implement the production change(s), add the listed test(s), make them pass, run `just check` (or the targeted subset called out), tick the matching `[ ]` checkbox in this plan, then commit. No multi-task batches.

### Phase A — carry-over stamp + recency derivation

- [x] **T0.** Stamp `files-inline.md` and the phase-3b plan as `landed (e98ed26, 2026-05-08)` (carry-over the dropped main commit). Commit `docs(phase-3b): mark files-inline spec + plan as landed (carry-over)`. **Verify:** spec/plan diffs inspected.

- [x] **T1.** Add per-channel reaction recency LRU + `ClientHandle::recent_reactions(channel) -> Vec<String>` (cap 5, falls back to spec default `👍 ❤️ 🍃 💚 👀`). Implemented as a new `reaction_recency: HashMap<String, VecDeque<String>>` field on the existing `state_actors::ChatMeta` (rather than spinning up a fresh actor) — keeps the wiring footprint to two functions on `ChatMeta` (`note_reaction` + `recent_reactions`) instead of threading a new `StateActor` through `SourceState`. `ClientHandle::react` now calls `note_reaction` after the underlying mutation succeeds; existing callers (`(&channel, &message_id, &emoji)`) keep working. **Tests:** `recent_reactions_starts_with_spec_default`, `recent_reactions_lru_caps_at_5`, `recent_reactions_per_channel_isolation`. **Verify:** `cargo test -p willow-client recent_reactions`.

### Phase B — emoji picker

- [x] **T2.** Create `crates/web/src/components/emoji_picker/` module + static `EMOJI_CATEGORIES` data (smileys / nature / food / travel / objects / symbols, ~ a few hundred glyphs) + `search(query, recent) -> Vec<&str>` helper. Pure function, 4 unit tests: empty query lists recents-first, category prefix match (`na` → nature only), case-insensitivity, dedupe against recents. Wire into `components/mod.rs`. **Verify:** `cargo test -p willow-web --lib emoji_picker`.

- [x] **T3.** Implement `<EmojiPicker>` popover (320 × 360 layout via CSS in T7+). Search input + glyph grid + category labels strip. ArrowLeft / ArrowRight move highlight (with wrap), Enter / Tab insert via `on_select`, Escape closes via `on_close`. The component is render-only — callers own visibility via `<Show>`. **Browser tests:** AG-3 / AG-4 land alongside T7 wiring (when the picker has a real callsite to mount under). **Verify:** `cargo check --workspace --all-targets` clean; pure-function search helper covered by T2 tests.

### Phase C — reactions strip

- [x] **T4.** Create `crates/web/src/components/reactions/` module + extract `<ReactionStrip>` from the inline render in `message.rs:1515`. Spec geometry + local-user pill style. **Browser test:** AG-1. **Verify:** `just test-browser`.

- [x] **T5.** Implement `<AddReactionChip>` desktop-only hover affordance. Click opens `<EmojiPicker>` via the shared callback. **Browser test:** AG-2. **Verify:** `just test-browser`.

- [x] **T6.** Implement `<ReactorTooltip>` (1 / 2 / 3 / 4+ pluralisation per spec). Hover-on-pill + mobile press-and-hold. **Browser test:** AG-5. **Verify:** `just test-browser`.

- [x] **T7.** Wire row hover toolbar `smile` more-reactions button + composer emoji IconBtn to open the shared `<EmojiPicker>`. Replace the inline placeholder shelf in `message.rs:1083`. **Browser test:** AG-11. **Verify:** `just test-browser`.

### Phase D — pinned-panel rewrite

- [x] **T8.** Rewrite `<PinnedPanel>` to spec entry layout. New `<article class="pinned-entry">` markup with avatar (24 px first-letter circle on `--bg-2`) + author (`--ink-0` weight 500) + relative timestamp (`--ink-3` 11 px) + 2-line ellipsis body preview + per-entry actions row. Empty state copy is byte-exact `nothing pinned yet.` (italic, `--ink-3`, 13 px) per spec §Pinned panel contents. CSS for `.pinned-entry__*` + `.pinned-empty` appended to `style.css` using foundation tokens only. **Browser test:** AG-7 lands alongside the rest of the phase-3c browser-tier sweep in T12. **Verify:** `just check` clean.

- [x] **T9.** Add per-entry `unpin` button to `<PinnedPanel>` permission-gated on `ManageChannels`. New `can_unpin: Signal<bool>` + `on_unpin: Callback<String>` props (both optional so existing callers compile unchanged). When `can_unpin == false`: `disabled` + `aria-disabled="true"` + tooltip `only stewards can pin here` per spec §Permission + action; click handler is a no-op. **Browser test:** AG-8 lands alongside T12. **Verify:** `just check` clean.

### Phase E — header pin button + permission gate

- [x] **T10.** Header pin IconBtn picks up `--amber` tint via the new `.action-btn--lit` class + a mono `.action-btn__count--pin` overlay when the channel has pins. New optional `pinned_count: Signal<usize>` prop on `<MainPaneHeader>`; callers thread it from `client.pinned_message_ids(channel).len()`. ARIA label flips between `pinned messages` (no pins) and `pinned messages ({count})` (with pins) per spec §Header entry point. **Browser test:** AG-9 lands alongside T12. **Verify:** `just check` clean.

- [x] **T11.** Permission-gate the row `P` keyboard binding on `ManageChannels`. **Browser test:** AG-10. **Verify:** `just test-browser`.

### Phase F — accessibility, polish

- [x] **T12.** Audit ARIA labels per the spec table. **Browser test:** AG-12. **Verify:** `just test-browser`.

### Phase G — close-out

- [x] **T13.** Tick the spec acceptance criteria in `docs/specs/2026-04-19-ui-design/reactions-pins.md`. Tick this plan's checkboxes. Update spec status from `draft` → `implementing` (or directly to `landed` on the merge commit if the plan author and reviewer agree). **Verify:** spec + plan diff inspected; `just check` clean.

## Ambiguity decisions

These are decisions made in the plan that the spec leaves open or under-specified.

1. **Recency LRU lifetime.** In-memory only for v1 — does not persist across app restarts. Persisting would mean a new sync-state row for every (channel, peer) pair which is more cost than benefit at this stage. The 5 recent reactions a user sees are recent within the *current session*; the spec's open question about cross-grove persistence remains open.
2. **Category data source.** Inline static `&[(&str, &[&str])]` array shipped alongside the picker. ~ a few hundred glyphs covering the spec's 6 named categories. Avoids a runtime fetch + the dependency creep of a full Unicode emoji table; the picker is search-first so coverage gaps surface only when a user knows a glyph by name and can't find it via search.
3. **Search ranking.** Prefix match on the emoji's primary name (e.g. `thumbs-up`, `red-heart`); ties broken by category order. No fuzzy match in v1 — keeps the picker fast and predictable.
4. **Composer emoji IconBtn binding.** Opens the same `<EmojiPicker>` instance and inserts the picked glyph at the current caret position via the existing composer textarea ref. The shortcode autocomplete (`:thumbsup:`) is a separate composer feature owned by `composer.md` and stays parked.
5. **Reactor tooltip on mobile.** Spec describes "tap and hold on the pill exposes the same list as a small card". v1 ships the desktop `title` attribute path immediately; the press-and-hold card lands as a follow-up to keep this PR scoped — the same `<ReactorTooltip>` component will mount in both contexts.
6. **Recency refresh granularity.** *(Resolved in PR #647.)* The web layer's `<ReactionRecencyProvider>` is a `LocalResource` keyed on the active channel signal AND a `RecencyRefreshTick` context value. `make_react_handler` bumps the tick after every successful `react()` so the resource re-fires immediately on same-channel reacts, not just on channel-revisit. See `crates/web/src/reaction_recency.rs` (context + `bump_recency_tick`), `crates/web/src/handlers.rs:163` (bump site), and `crates/web/src/app.rs:208` (LocalResource subscribing to the tick).

## Test plan

- **State tests:** none. No new EventKinds.
- **Client tests:** `recent_reactions_starts_with_spec_default`, `recent_reactions_lru_caps_at_5`, `recent_reactions_per_channel_isolation`.
- **Web unit tests:** `emoji_picker::categories::search` decision-table (4 cases).
- **Browser tests:** AG-1 through AG-12 (~14 tests in `mod phase_3c_reactions_pins`).
- **E2E:** none added in this phase. Reactions / pin sync are already covered by `multi_peer_sync.rs` + `permissions.rs` at the e2e tier.

## Verification commands

Subagents must run the targeted subset for their task; the final task runs the full set:

```bash
cargo test -p willow-client recent_reactions   # T1
cargo test -p willow-web --lib emoji_picker    # T2
just test-browser                              # T3–T12 (each task runs the suite or a filter)
just check                                     # T13 close-out (full sweep)
```

## Out of scope (explicitly deferred)

- Cross-grove reaction-recency persistence — channel-scoped LRU only in v1.
- Pinned-panel cross-channel view — out of scope.
- Composer shortcode autocomplete (`:thumbsup:`) — owned by `composer.md`, deferred.
- Fuzzy emoji search — prefix match only in v1.
- Reactor tooltip on mobile (press-and-hold card) — desktop hover only in v1; mobile path lands as a follow-up.

## Status ladder transitions

- Spec `reactions-pins.md` moves `draft` → `implementing` on T1 (or T0 if the human reviewer prefers a separate spec-status bump commit).
- Spec moves `implementing` → `landed` on PR merge (commit hash recorded in spec front-matter).
- This plan moves `draft` → `landed` on PR merge.
