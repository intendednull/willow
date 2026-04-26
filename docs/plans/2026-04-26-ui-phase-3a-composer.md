# UI Phase 3a — Composer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development + superpowers:test-driven-development. Every task = one commit; tick the checkbox in the same commit.

**Goal:** Ship `docs/specs/2026-04-19-ui-design/composer.md` in full — replace the legacy single-line `<input>` ChatInput with the spec'd compose surface (desktop pill + mobile pill), reply preview bar, edit bar, autogrow textarea, full keybinding set (⌘↵, Shift+Enter, Esc unwind, ↑ to edit last own message), mention autocomplete on `@`, offline tinting, per-channel-kind placeholder copy, typing indicator row above the composer, and ARIA labels for every interactive element.

**Architecture:** One shared `<Composer>` parent component reads from spec'd derivations on `ClientHandle` (`typing(channel_id)`, `connection_state()`) plus existing mutations (`send_message`, `edit_message`, `send_typing_indicator`). Two style variants — desktop (`pill-large`) and mobile (`pill-small`) — share the same input logic; the wrapper picks the variant by `data-shell` from `mobile_shell`. A new `<MentionAutocomplete>` popover anchors above the textarea and is driven by `composer_mentions::Suggestions` (a small per-channel filter helper added to `willow-client`). Reply / edit bars are visual-only refinements of existing local `RwSignal<Option<DisplayMessage>>` state in the chat view; we do **not** introduce new global signals. Typing-ping transport is already complete (`WireMessage::TypingIndicator` + `Client::typing_in` + 3s throttle) — this plan only consumes it from the new composer.

**Tech Stack:** Leptos 0.7, WASM (wasm-pack), willow-client (1 new view derivation, 1 mention-suggestions helper, 1 last-own-message accessor), willow-web (composer module rewrite, mention popover, typing indicator row). Foundation tokens only (no new hex). No new EventKinds; the composer is pure UI on top of already-shipped state + transport. `just test-browser` and `just test-e2e-*` run in CI.

**Branch:** `ui/phase-3a-composer`. Commits `ui(phase-3a): <imperative>` (code) and `docs(plan): phase 3a — composer implementation plan` (initial plan commit only). Lands as one PR; the plan ships in the same PR as the implementation.

---

## Scope

**In:**
- Replace `crates/web/src/components/input.rs` with a spec-compliant `<Composer>`.
- Autogrow `<textarea>`: `min-height: 1.45em`, grows by `scrollHeight` up to 8 lines then scrolls.
- Desktop pill (`--bg-2` on `--line`, radius 12, `--shadow-1`, 8/10 outer padding) with attach + emoji + send button row.
- Mobile pill (radius 22, single-line default, 6/8/6/12 padding, circular 34×34 send).
- Send button label flips to `save` when in edit mode (no separate action row).
- Meta row: `lock` + `sealed with grove-keys` + `ear` + `hold shift to whisper` (desktop) / `lock` + `sealed to {N} peers in grove` + `tap ear to whisper` (mobile).
- Per-channel-kind placeholder copy: `message #{channel} — encrypted to {N} peers` / `message {name}` / `offline — messages queue until reconnect` / `choose a channel to start`.
- Keybindings (desktop): Enter sends, Shift+Enter newline, Ctrl/⌘+Enter force-send, Tab inserts 2 spaces, Esc unwinds edit → reply → blur, ArrowUp on empty enters edit on last own message.
- Keybindings (mobile): Enter inserts newline; send button is the only submit path. `@` still opens mention autocomplete.
- Mention autocomplete popover on `@` at word boundary — peer list filtered by prefix on handle / first segment / display name; arrow keys navigate; Enter/Tab insert handle; Esc dismisses; max 8 visible rows; scroll above; `@channel` row visible only with `ManageChannels`.
- Offline state: amber tint (`color-mix(in oklab, var(--amber) 10%, var(--bg-2))`), meta line becomes `hourglass` + `offline · queuing messages`, send still works.
- Reply preview bar: 2px `--moss-2` left rule, `replying to` label, parent author + truncated body preview, click → scroll-to + 180ms `willow-pop-in` flash on parent.
- Edit bar: `editing message · esc to cancel` hint above composer; pre-fills body with selection; `(edited)` already rendered in `message.rs:729` — keep.
- Typing indicator row above the composer: 3-dot `willowPulse` (staggered 0/200/400ms), label per copy table (1/2/3/4+ forms), `aria-live="polite"` debounced to once per 5s, hidden when empty.
- ARIA labels: `cancel reply` / `cancel edit` / `send` / `attach file` / `open emoji picker` (cancel on `@channel` row reads "everyone in this channel · notifies all members").
- Reduced-motion: `willowPulse` becomes a static dot (foundation already gates this).
- Browser tests for: autogrow line counts, send/cancel/edit/reply keybindings, ArrowUp-edit-last, Tab-2-spaces, mention autocomplete (open/filter/insert/dismiss), `@channel` permission gate, offline tint flip on `connection_state`, typing indicator pluralization (1/2/3/4+), `aria-live` debounce, reply preview click-scroll wiring.

**Out (hand-off):**
- Drag-and-drop / paste-to-upload / attach-button file dialog → `files-inline.md` (Phase 3b). The attach button renders, fires a stub `on:click` that dispatches an existing TODO event for that phase to wire up.
- Emoji picker popover when opened from the smile button → `reactions-pins.md` (Phase 3c). The smile button renders, fires the same stub.
- Whisper send (Shift held during send + ear button on mobile) → `whisper-mode.md` (Phase 4). Composer renders the whisper hint and ear button; the actual `whisper_send` route is left as a TODO callback in this plan, gated on Phase 4.
- Empty-channel illustration / "say hi?" copy → owned by `message-row.md` (already shipped).
- Edit-history surfacing (`open question` in spec) — defer; `(edited)` is enough for v1.
- `@channel` confirmation modal (`open question` in spec) — defer; `ManageChannels`-only is enough for v1. Add a TODO to revisit after `governance.md`.
- Typing-ping transport — **already shipped**. The spec's "open question" is stale. We consume `Client::typing_in` directly.

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/client/src/views.rs` | modify | Add `mention_candidates(channel_id) -> Vec<MentionCandidate>` derivation: returns peers in the current channel with display name, handle, status dot. Add `last_own_message(channel_id) -> Option<DisplayMessage>` accessor for ArrowUp-edit. |
| `crates/client/src/lib.rs` | modify | Re-export `MentionCandidate`. |
| `crates/client/src/mentions.rs` | modify | Add `Suggestions::filter(query, candidates) -> Vec<MentionCandidate>` — prefix match on handle / first display-name segment / full display name, dedupe, max 8. Pure function. 5 unit tests. |
| `crates/client/src/tests/composer_views.rs` | **new** | Tests: `mention_candidates_includes_channel_peers` / `mention_candidates_excludes_self` / `last_own_message_returns_most_recent_in_channel` / `last_own_message_none_when_no_own_messages` / `mention_filter_prefix_handle` / `mention_filter_prefix_display` / `mention_filter_caps_at_8`. |
| `crates/client/src/tests/mod.rs` | modify | Declare `composer_views` module. |
| `crates/web/src/components/composer/mod.rs` | **new** | `pub mod composer; pub mod meta_row; pub mod reply_bar; pub mod edit_bar; pub mod typing_indicator; pub mod mention_autocomplete; pub mod placeholders; pub use composer::*;`. |
| `crates/web/src/components/composer/composer.rs` | **new** | `<Composer>` parent. Owns `(input_text, set_input_text)`, `(mention_query, set_mention_query)`, `(autocomplete_open, set_autocomplete_open)`. Reads `replying_to`, `editing` from props (existing `RwSignal<Option<DisplayMessage>>`). Reads `connection_state` + `current_channel` + `channel_kind` from `AppState`. Renders typing-indicator row → optional reply bar → optional edit bar → autogrow textarea + button row → meta row. Wires keydown handler with full unwinding semantics. Picks pill variant via `data-shell` body attr (already set by `mobile_shell`). |
| `crates/web/src/components/composer/meta_row.rs` | **new** | `<MetaRow>` — desktop variant renders `lock · sealed with grove-keys · ear · hold shift to whisper` plus optional `{name} is whispering` status with 3-dot `willowPulse` in `--whisper`. Mobile variant renders `lock · sealed to {N} peers in grove · tap ear to whisper`. Switches to amber `hourglass · offline · queuing messages` on `ConnectionState::Offline`. |
| `crates/web/src/components/composer/reply_bar.rs` | **new** | `<ReplyBar>` — 2px `--moss-2` left rule + `replying to` label + parent author italic + truncated 1-line body preview + cancel text button. Click on the preview body fires a `scroll_to_message(parent_id)` callback wired in the chat view to scroll + flash via existing `willow-pop-in` keyframe. ARIA `cancel reply`. |
| `crates/web/src/components/composer/edit_bar.rs` | **new** | `<EditBar>` — `editing message · esc to cancel` hint (`--ink-3`, hint size). ARIA `cancel edit`. Send button label flip is owned by parent. |
| `crates/web/src/components/composer/typing_indicator.rs` | **new** | `<TypingIndicator>` — reads `client.typing_in(channel_id)` via existing accessor; renders 3-dot `willowPulse` (staggered 0/200/400ms) + label per copy table. Hidden when `typing_in` returns empty. `aria-live="polite"` with a 5s debounce in a local `use_debounce` (foundation pattern from notifications). |
| `crates/web/src/components/composer/mention_autocomplete.rs` | **new** | `<MentionAutocomplete>` popover. Anchored to the textarea via inline `top` / `left` from a measure-on-`@` `Effect`. `--bg-1` on `--line`, radius 10, `--shadow-2`, max 8 rows visible. Each row: avatar (20px), display name, handle (mono), status dot. Arrow keys move selection (with wrap), Enter/Tab insert handle pill at the `@` position, Esc dismisses. `@channel` row prepended only if local peer has `ManageChannels`. Exposes `on_select(handle: String)` callback wired in the parent to splice into the textarea. |
| `crates/web/src/components/composer/placeholders.rs` | **new** | `placeholder_for(channel_kind, channel_name, recipient_name, peer_count, connection: ConnectionState) -> &str` — pure function returning the spec's placeholder strings. 4 unit tests in the same file under `#[cfg(test)] mod tests`. |
| `crates/web/src/components/input.rs` | **delete** | Replaced by composer module. Remove the legacy `<ChatInput>`. Update imports in callers below. |
| `crates/web/src/components/chat_view.rs` | modify | Replace `<ChatInput …>` with `<Composer …>`. Add `scroll_to_message` callback closing over the message-list ref + `flash_message_id` signal. Wire the existing `replying_to` / `editing` signals through unchanged. |
| `crates/web/src/components/mod.rs` | modify | Replace `pub use input::*;` with `pub use composer::*;`. |
| `crates/web/style.css` | modify | Append `.composer`, `.composer--desktop`, `.composer--mobile`, `.composer__textarea`, `.composer__buttons`, `.composer__meta`, `.composer--offline`, `.composer__reply-bar`, `.composer__edit-bar`, `.composer__typing-indicator`, `.composer__typing-dot`, `.mention-popover`, `.mention-popover__row`, `.mention-popover__row--selected`, `.mention-popover__row--channel`, `@keyframes willow-pulse-dot`. Foundation tokens only — `--bg-0`, `--bg-1`, `--bg-2`, `--line`, `--moss-1`, `--moss-2`, `--moss-4`, `--ink-0`, `--ink-1`, `--ink-2`, `--ink-3`, `--ink-4`, `--whisper`, `--amber`, `--shadow-1`, `--shadow-2`, `--motion-fast`. |
| `crates/web/tests/browser.rs` | modify | Append `mod phase_3a_composer { … }` — ~16 tests (see Acceptance gates). |

## Acceptance gates

> Each gate maps to one or more browser/client/state tests. A task is complete when (a) the production code lands, (b) the corresponding test(s) pass, (c) the spec checkbox is ticked in the same commit.

- [ ] **AG-1.** `<Composer>` mounts with autogrow textarea (1 line min, 8 lines max, then scrolls). Test: type 1 / 4 / 8 / 12 lines, assert `scrollHeight` matches expected.
- [ ] **AG-2.** Enter sends; Shift+Enter inserts newline; Ctrl/⌘+Enter force-sends. Test: simulate each combo, assert send callback fires (or doesn't) + textarea state.
- [ ] **AG-3.** Tab in textarea inserts 2 spaces with no focus move. Test: simulate Tab keydown, assert content + active element unchanged.
- [ ] **AG-4.** ArrowUp on an empty textarea enters edit mode for the most recent own message in the current channel. Test: send a message, focus composer, simulate ArrowUp, assert `editing` signal flipped to that message.
- [ ] **AG-5.** Esc unwinds in order: cancel edit → cancel reply → blur. Test: with both edit and reply active, simulate three Esc presses, assert state after each.
- [ ] **AG-6.** Reply preview bar renders with the spec layout (left rule + author + preview + cancel) and clicking the preview body fires `scroll_to_message`. Test: mount with `replying_to = Some(msg)`, simulate click on preview body, assert callback fired with `msg.id`.
- [ ] **AG-7.** Edit bar shows `editing message · esc to cancel` and the send button label is `save` while editing; submitting routes to the edit callback. Test: mount with `editing = Some(msg)`, assert send button text + edit callback wiring.
- [ ] **AG-8.** Mention autocomplete opens on `@` at a word boundary, filters by prefix on handle / display, supports arrow + Enter/Tab to insert, Esc dismisses. Test: type `@a`, assert popover open + filtered list, simulate arrow + Enter, assert handle inserted at the `@` position with the `@` consumed.
- [ ] **AG-9.** `@channel` row appears only when local peer has `ManageChannels`. Test: with and without permission, assert row presence.
- [ ] **AG-10.** Connection state `Offline` flips composer to amber tint and meta to `hourglass · offline · queuing messages`; send still routes to the queue path. Test: drive `connection_state` to `Offline`, assert class + meta copy + send still calls callback.
- [ ] **AG-11.** Per-channel-kind placeholder copy renders the right string for text / letter / offline / no-channel. Test: each combo, assert textarea `placeholder` attribute. (Pure-function unit tests in `placeholders.rs` cover the table; one browser test asserts the wiring.)
- [ ] **AG-12.** Typing indicator row renders the right form for 1 / 2 / 3 / 4+ peers driven by `Client::typing_in`. Test: drive the underlying actor state, assert text content for each count.
- [ ] **AG-13.** Typing indicator announces via `aria-live="polite"` and debounces to at most once per 5 s. Test: drive 3 rapid changes within 5 s, assert only the first announces.
- [ ] **AG-14.** ARIA labels match the spec table (`cancel reply` / `cancel edit` / `send` / `attach file` / `open emoji picker`). Test: mount, assert each `aria-label` attribute.
- [ ] **AG-15.** `prefers-reduced-motion: reduce` collapses `willowPulse` to a static dot. Test: set the media query, assert `--motion-fast` token replaces the keyframe (foundation already implements this — the test verifies the composer participates).
- [ ] **AG-16.** Spec acceptance criteria checkboxes in `composer.md` are ticked at the end of the plan (final task).

## Tasks

> Each task is one self-contained commit. Subagent contract: implement the production change(s), add the listed test(s), make them pass, run `just check` (or the targeted subset called out), tick the matching `[ ]` checkbox in this plan, then commit. No multi-task batches.

### Phase A — client derivations (foundation for the UI)

- [x] **T1.** Add `MentionCandidate { peer_id, display_name, handle, presence }` to `willow-client::views`. Add `mention_candidates(channel_id) -> Vec<MentionCandidate>` view selector. Includes peers in the current channel; excludes the local peer. Resolves display + handle + presence from the existing `Profiles` + `presence` derivations. **Tests:** `mention_candidates_includes_channel_peers`, `mention_candidates_excludes_self`. **Verify:** `just test-client`.

- [x] **T2.** Add `last_own_message(channel_id) -> Option<DisplayMessage>` accessor in `willow-client::views`. Returns the most recent message authored by the local peer in the given channel, or `None`. **Tests:** `last_own_message_returns_most_recent_in_channel`, `last_own_message_none_when_no_own_messages`. **Verify:** `just test-client`.

- [x] **T3.** Add `mentions::Suggestions::filter(query, candidates) -> Vec<MentionCandidate>` — prefix match on `handle`, first `display_name` segment, full `display_name`; dedupe by `peer_id`; max 8 entries. Pure function. **Tests:** `mention_filter_prefix_handle`, `mention_filter_prefix_display`, `mention_filter_caps_at_8`, `mention_filter_dedupes_overlapping_matches`, `mention_filter_empty_query_returns_all_capped`. **Verify:** `just test-client`.

### Phase B — composer module skeleton

- [x] **T4.** Create `crates/web/src/components/composer/` module + `placeholders.rs` with `placeholder_for(...)`. Pure function with 4 unit tests (text / letter / offline / no-channel). Wire into `components/mod.rs`. **Verify:** `cargo test -p willow-web`.

- [x] **T5.** Add `<Composer>` shell component (no behavior yet) — autogrow textarea + send button only, picks `--desktop` / `--mobile` variant from `data-shell` body attribute. Replace `<ChatInput>` callsite in `chat_view.rs`. Delete `crates/web/src/components/input.rs`. **Browser test:** `composer_mounts_with_autogrow_textarea` (renders, asserts initial `min-height`, types 12 lines, asserts max 8 visible + scrolls). **Verify:** `just test-browser` for the new test only.

### Phase C — keybindings + reply/edit bars

- [ ] **T6.** Implement full keydown handler: Enter / Shift+Enter / Ctrl|⌘+Enter / Tab / Esc unwind / ArrowUp-edit-last. **Browser tests:** AG-2, AG-3, AG-4, AG-5. **Verify:** `just test-browser` for the suite.

- [ ] **T7.** Implement `<ReplyBar>` per spec layout. Wire `scroll_to_message` callback in `chat_view.rs` to scroll + flash the parent via `willow-pop-in` (foundation keyframe already exists). **Browser test:** AG-6. **Verify:** `just test-browser`.

- [ ] **T8.** Implement `<EditBar>` per spec layout. Add send-button label flip in parent (`save` while editing; `send` otherwise). **Browser test:** AG-7. **Verify:** `just test-browser`.

### Phase D — meta row, offline tint, placeholders

- [ ] **T9.** Implement `<MetaRow>` desktop + mobile variants reading from `connection_state` + `peer_count` + `current_channel`. **Browser test:** `meta_row_renders_desktop_meta`, `meta_row_renders_mobile_meta` (use `mount_test_with_shell`). **Verify:** `just test-browser`.

- [ ] **T10.** Implement offline tint flip + placeholder copy wiring from `placeholder_for`. **Browser tests:** AG-10, AG-11. **Verify:** `just test-browser`.

### Phase E — typing indicator

- [ ] **T11.** Implement `<TypingIndicator>` row above the composer. Reads `client.typing_in(channel_id)` (already shipped in `joining.rs:322`). Pluralization via match on `len()`. **Browser test:** AG-12. **Verify:** `just test-browser`.

- [ ] **T12.** Wire `aria-live="polite"` with 5s debounce. **Browser test:** AG-13. **Verify:** `just test-browser`.

### Phase F — mention autocomplete

- [ ] **T13.** Implement `<MentionAutocomplete>` popover with anchored positioning, 8-row cap, and arrow/Enter/Tab/Esc handling. **Browser test:** AG-8. **Verify:** `just test-browser`.

- [ ] **T14.** Wire `@channel` row gated on `ManageChannels`. **Browser test:** AG-9. **Verify:** `just test-browser`.

### Phase G — accessibility, motion, polish

- [ ] **T15.** Audit ARIA labels per spec table. **Browser test:** AG-14. **Verify:** `just test-browser`.

- [ ] **T16.** Verify reduced-motion participation. **Browser test:** AG-15. **Verify:** `just test-browser`.

### Phase H — close-out

- [ ] **T17.** Tick the spec acceptance criteria in `docs/specs/2026-04-19-ui-design/composer.md` based on the gates that landed. Tick this plan's checkboxes. Update spec status from `draft` → `implementing` (if not already). Resolve the spec's "Open questions" — typing-ping transport is already shipped; mark answered with a pointer to `WireMessage::TypingIndicator`. Edit-history surfacing → `defer to v2`. `@channel` confirmation → `defer to v2 post-governance`. **Verify:** spec + plan diff inspected; `just check` clean.

## Ambiguity decisions

These are decisions made in the plan that the spec leaves open or under-specified. Recorded so future readers know why we chose what we chose.

1. **Typing-ping transport.** Spec marks this as an open question requiring `willow-state` / `willow-network` design review. The transport is already shipped: `WireMessage::TypingIndicator` in `crates/common/src/wire.rs:30-33`, send path in `crates/client/src/joining.rs:299-320` (3s throttle), receive path in `crates/client/src/listeners.rs:317-330` (`network_meta.typing_peers` map with 4s TTL). No new state event needed. The plan consumes the existing `Client::typing_in(channel)` accessor unchanged.
2. **`@channel` confirmation modal.** Spec proposes a confirmation step above 20 members, defers final decision. v1 ships permission-only gating on `ManageChannels`; the threshold confirmation is parked behind a TODO referencing `governance.md`. Avoids speculative scope.
3. **Edit history surfacing.** Spec asks whether prior edit versions should be reachable; defers. v1 keeps just `(edited)` (already implemented in `message.rs:729`). Data is already preserved in the event log so a later revision can opt in.
4. **Mobile whisper send action.** The ear button renders and dispatches a `whisper_pending` callback that is a TODO until `whisper-mode.md` (Phase 4) lands. Composer ships visually correct without functional whisper send.
5. **Attach + emoji buttons.** Render and dispatch placeholder click callbacks. The dialog/picker components are owned by `files-inline.md` / `reactions-pins.md` (Phases 3b / 3c). The composer pre-wires the click hooks so those phases drop in without re-shaping props.
6. **Reduced-motion handling for typing dots.** Spec says `willowPulse` becomes static. We collapse to a single static `--ink-3` dot rather than three; equivalent affordance, cleaner static look. Foundation tokens already gate this via `prefers-reduced-motion: reduce` in `style.css`.
7. **Mention autocomplete ranking.** Spec lists prefix-match on handle / first segment / display name without a tiebreak rule. Implementation: handle-prefix > first-segment-prefix > display-name-prefix > everything else, then alphabetical by handle. Stable across renders.

## Test plan

- **State tests:** none. No new EventKinds.
- **Client tests:** `mention_candidates_includes_channel_peers`, `mention_candidates_excludes_self`, `last_own_message_returns_most_recent_in_channel`, `last_own_message_none_when_no_own_messages`, `mention_filter_prefix_handle`, `mention_filter_prefix_display`, `mention_filter_caps_at_8`, `mention_filter_dedupes_overlapping_matches`, `mention_filter_empty_query_returns_all_capped` (in `crates/client/src/tests/composer_views.rs`).
- **Web unit tests:** 4 `placeholder_for` tests (in `placeholders.rs`).
- **Browser tests:** AG-1 through AG-15 (16 tests in `mod phase_3a_composer`). Use `mount_test_with_shell(TestShell::Desktop)` for desktop-pill assertions, `Mobile` for mobile-pill. Use `tick().await` after every signal mutation.
- **E2E:** none added in this phase. Mention autocomplete cross-peer is covered by existing single-client browser tests; multi-peer typing indicators are out of scope for v1 (the existing `multi_peer_sync.rs:437` test covers wire delivery).

## Verification commands

Subagents must run the targeted subset for their task; the final task runs the full set:

```bash
just test-client                      # T1, T2, T3
just test-browser                     # T5–T16 (each task runs the suite or a filter)
cargo test -p willow-web              # T4 unit tests
just check                            # T17 close-out (full sweep)
```

## Out of scope (explicitly deferred)

- Drag-and-drop / paste-to-upload / attach-button file dialog → Phase 3b (`files-inline.md`).
- Emoji picker popover → Phase 3c (`reactions-pins.md`).
- Whisper send wire-up → Phase 4 (`whisper-mode.md`).
- Edit-history surfacing → defer to v2.
- `@channel` confirmation modal → defer to v2 post-governance.
- Multi-peer typing indicator E2E → existing `multi_peer_sync.rs:437` is sufficient.

## Status ladder transitions

- Spec `composer.md` moves `draft` → `implementing` on T17 commit (or earlier if the human reviewer prefers a separate spec-status bump commit).
- Spec moves `implementing` → `shipped` on PR merge (commit hash recorded in spec front-matter).
- This plan moves `draft` → `shipped` on PR merge.
