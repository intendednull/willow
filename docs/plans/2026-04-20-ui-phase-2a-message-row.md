# UI Phase 2a — Message row Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development + superpowers:test-driven-development.

**Goal:** Ship `docs/specs/2026-04-19-ui-design/message-row.md` — row anatomy/grouping/day separators, mentions (parsing + pills + self-mention highlight), inline + fenced code, pinned marker, queue notes (LateArrival + Pending), whisper hand-off placeholder, empty/loading states, jump-to-latest pill, swipe-left quote-reply (swipe-right already ships), long-press action sheet compliance, hover toolbar anatomy, spec copy + ARIA contract.

**Style ref:** 1a/1b/1f plans. Commits: `ui(phase-2): <imperative>`. Branch `design/ui-target-ux`. After Phase 1 (shell + a11y + trust + presence + notifications).

## Scope

**In:** Row anatomy + density-aware padding / run-break rules (whisper / pin / queueNote) / collapsed-row hover timestamp / day separators / mention parsing + `MentionPill` + self-mention row highlight / inline backtick pill + fenced `<pre>` with copy button / pinned marker + `pinned` badge / queue-note derivation + hints + dim-till-delivered / whisper placeholder (always-false gate) / empty + loading states + jump-to-latest pill / swipe-left quote-reply gesture / long-press action sheet copy alignment / hover toolbar rendering / exact copy pass / ARIA label table + keyboard path / 500-char wrap + RTL + zero-width + fallback edge cases.

**Out:** Reactions strip / add-reaction chip / emoji picker (`reactions-pins.md`). File cards, images, upload flow (`files-inline.md`). Composer, reply-preview bar, edit bar, typing, mention autocomplete (`composer.md`). Thread pane interior (`thread-pane.md`). Whisper body styling + `WhisperStart` EventKind (`whisper-mode.md`; this phase gates a placeholder). Pinned panel (`reactions-pins.md`). Sync-queue screen (`sync-queue.md`). Profile popover (`profile-card.md`).

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/web/src/components/message.rs` | modify | Rewrite body segment pipeline to run mentions → inline code → urls. Add `MentionPill`, inline-code pill, fenced-code `<pre>` + copy button, pinned marker, queue-note hint, whisper placeholder, collapsed-row hover timestamp, swipe-left quote-reply handler, hover toolbar markup (icons only). Replace ad-hoc `"Reply" / "Edit" / …"` strings with exact spec copy. |
| `crates/web/src/components/chat.rs` | modify | Add day-separator insertion, jump-to-latest pill + unread-count signal, 120px auto-scroll gate, loading skeleton rows, empty-channel illustration + copy, cleared-after-deletion variant, downstream `on_reply`/`on_edit` plumbing for composer quote-reply. |
| `crates/web/src/components/message_row/mention.rs` | **new** | `parse_mentions(body, channel_peers, local_peer) -> Vec<Segment>` + `MentionPill` component (peer / self variants). Regex `/@([a-z][a-z0-9._-]*)/gi`. Resolver: exact handle → first handle segment → display-name → `@you`. |
| `crates/web/src/components/message_row/code.rs` | **new** | `parse_code_segments(body) -> Vec<Seg>` splitting on ``` fences and `` ` `` inline spans. `InlineCodePill` + `FencedCodeBlock` (with copy IconBtn — `check` flash 900ms). |
| `crates/web/src/components/message_row/day_separator.rs` | **new** | `day_bucket(ts_ms) -> DayBucket` enum + `<DaySeparator bucket>` component rendering `— today —` / `— yesterday —` / `— friday · 14 april —` / `— friday · 14 april · 2025 —`. |
| `crates/web/src/components/message_row/jump_latest.rs` | **new** | `<JumpToLatestPill on_click, new_count>` — chevron + `jump to latest` + ` · {N} new`. Mounted inside `MessageList` container; hides when within 120px of bottom. |
| `crates/web/src/components/message_row/mod.rs` | **new** | Submodule re-exports (`mention`, `code`, `day_separator`, `jump_latest`). |
| `crates/web/src/components/mod.rs` | modify | Register `message_row` submodule; re-export `MentionPill`, `JumpToLatestPill`, `DaySeparator`. |
| `crates/web/src/icons.rs` | modify | Add `icon_hourglass` (if 1a didn't already), `icon_pin_small` (1px-rule marker paired badge uses existing `icon_pin`), `icon_copy`, `icon_check_small` (24×24 stroke-1.5), `icon_leaf` (48×48 empty-state SVG). |
| `crates/web/components.css` (or `style.css`) | modify | Add `.message` density via `--msg-pad`; `.message--run` (collapsed) with `1px 24px` padding + empty avatar column hover timestamp; `.message--pinned`, `.message--whisper`, `.message--queue`, `.message--mention` row rules; `.mention-pill`, `.mention-pill--self`; `.code-inline`, `.code-fenced`, `.code-copy-btn`; `.queue-note`; `.pinned-badge`, `.whisper-badge`, `.queued-badge`; `.day-separator`; `.jump-to-latest`; `.chat-empty`, `.chat-cleared`, `.chat-skeleton`; `.message-hover-toolbar`. Consume foundation tokens only. |
| `crates/client/src/state.rs` | modify | Extend `DisplayMessage` with `pinned: bool`, `whisper: bool` (always `false` this phase), `queue_note: QueueNote`, `mentions: Vec<EndpointId>`. |
| `crates/client/src/lib.rs` | modify | Re-export `QueueNote`. |
| `crates/client/src/views.rs` | modify | Populate new `DisplayMessage` fields when projecting: `pinned` from `ServerState::pinned_messages`, `queue_note` from `MessageStore` delivery-state + peer online-status-at-author, `mentions` from body parse over channel peers at projection time. |
| `crates/client/src/tests/display_message.rs` (or inline `tests` mod) | **new** | Unit tests: pinned projection, LateArrival detection, Pending detection, mention resolver (peer / self / unresolved). |
| `crates/web/tests/browser.rs` | modify | Append `mod phase_2a_message_row { … }` at file end using `mount_test_with_shell`. ~18 tests covering every §Acceptance item. |
| `e2e/helpers.ts` | modify | Add `swipeLeft(row)` helper (mirror of existing `swipeRight`). Add `.jump-to-latest` selector helper. |
| `e2e/mobile-actions.spec.ts` | modify | Assert action-sheet copy (`reply`, `reply in thread`, `add reaction`, `pin`/`unpin`, `copy text`, `edit`, `delete`, `cancel`) + swipe-down 80px dismiss + velocity-dismiss path. |

## Acceptance gates

1. `just check` (fmt + clippy + unit tests + wasm check) green.
2. `just check-wasm` green.
3. `just test-state` + `just test-client` green (new `DisplayMessage` field + mention resolver + queue-note tests).
4. `just test-browser` green with the new `phase_2a_message_row` module (~18 tests).
5. `npx playwright test --project=desktop-chrome --project=mobile-chrome e2e/mobile-actions.spec.ts` green; helpers updated in same commit as markup.
6. Manual walkthrough:
   - Two runs from same author within 5 min collapse; whisper / pin / queue-note break runs; collapsed row reveals mono `HH:MM` in avatar column on hover.
   - Day separators render between local-date boundaries with em-dash decor and correct copy tier (today / yesterday / weekday · dd mmm / …· yyyy).
   - `@handle` body tokens render as moss-pill; self-mention tokens render as amber `@you`; self-mention row carries amber left-rule + 8% tint; sidebar mention counter increments.
   - Inline `` `code` `` renders as mono pill; triple-backtick fence renders as `<pre>` with copy button that flashes `check` for 900ms.
   - Pinned message carries 1px amber left rule + `pinned` badge; always breaks a run.
   - Pending message opacity 0.7 + `queued · will send on reconnect` hint; on delivery fades to 1 + `sent` flash 900ms. LateArrival message shows `sent earlier · arrived now` + `queued` badge.
   - Whisper placeholder (forced via always-false gate) — spec rules are renderable when a future `WhisperStart` event flips the gate.
   - Empty channel shows leaf SVG + `this channel is quiet. say hi?`; after-delete shows `cleared — nothing here yet.`; loading shows five shimmer skeletons; reduced-motion collapses to static `--bg-2` rectangles.
   - Scroll up > 120px hides auto-scroll; a `jump to latest · {N} new` pill appears; click smooth-scrolls and clears the count.
   - Swipe-left on a row (dx > 60 px, dx > 1.2× dy) populates composer `replying_to` without opening the thread. Swipe-right still opens the thread.
   - Long-press sheet shows spec copy verbatim; swipe-down 80px or velocity > 200 px/s dismisses; haptic fires on open.
   - Hover toolbar fades in over 120 ms on desktop mouseenter; opacity-only under reduced motion; ARIA labels match spec table.
   - 500-char single-word body wraps without horizontal scroll; RTL paragraph picks its own base direction; empty body renders `empty message`; missing display name falls back to handle.

## Tasks (16 total, ~22 commits)

### 1. Row anatomy + grouping + density-aware padding

Tighten the existing `.message` rules to consume `--msg-pad`, add run-break rules for whisper / pin / queueNote, expose a mono hover timestamp in the collapsed-row avatar column.

**Files:** modify `crates/web/src/components/message.rs`, `crates/web/src/components/chat.rs`, `crates/web/style.css` (or new `components.css`).

- [ ] **Step 1.1 — Promote run-break predicate.** In `MessageList`, expand the grouping predicate to break runs when `prev.whisper || prev.pinned || prev.queue_note != None || msg.whisper || msg.pinned || msg.queue_note != None`. Preserve existing 5-minute + same-author rule.

  ```rust
  let break_run = prev.author_peer_id != msg.author_peer_id
      || msg.timestamp_ms.saturating_sub(prev.timestamp_ms) > 300_000
      || prev.whisper || prev.pinned || prev.queue_note != QueueNote::None
      || msg.whisper || msg.pinned || msg.queue_note != QueueNote::None;
  let show_header = i == 0 || break_run;
  ```

- [ ] **Step 1.2 — Density-aware padding.** Replace hard-coded `.message` / `.message.grouped` padding with `padding: var(--msg-pad)` + `padding-block: 1px` on `.message--run`. Keep `--msg-pad` density variants from `foundation.css` (no new vars).

- [ ] **Step 1.3 — Collapsed-row hover timestamp.** In `MessageView`, when `!show_header`, render `<span class="run-hover-ts">` inside the empty avatar column with the pre-formatted `HH:MM`. CSS reveals it on `.message--run:hover` only.

- [ ] **Step 1.4 — `just check-wasm`** — expect clean build.

- [ ] **Step 1.5 — Commit** — `ui(phase-2): tighten run-break rules + density-aware message padding`.

### 2. Day separators

Emit `<DaySeparator>` between messages whose local dates differ. No existing separator renderer.

**Files:** new `crates/web/src/components/message_row/day_separator.rs`, new `crates/web/src/components/message_row/mod.rs`, modify `crates/web/src/components/chat.rs`, modify `crates/web/src/components/mod.rs`, modify `crates/web/style.css`.

- [ ] **Step 2.1 — `DayBucket` enum + formatter.** `Today | Yesterday | ThisYear { weekday, day, month } | Older { weekday, day, month, year }`. Use `js_sys::Date` for local-timezone bucketing in WASM.

  ```rust
  pub enum DayBucket { Today, Yesterday, ThisYear(String), Older(String) }
  pub fn day_bucket(ts_ms: u64) -> DayBucket { /* Date::new_0() offsets */ }
  ```

- [ ] **Step 2.2 — `<DaySeparator>` component.** Markup: `<div class="day-separator"><span class="rule"/> <em>— {label} —</em> <span class="rule"/></div>`. CSS per spec §Day separator (flex 1 rules, `font-display italic 11px --ink-3 uppercase letter-spacing 1.2px`).

- [ ] **Step 2.3 — Insertion in `MessageList`.** During the enumerate pass, when `day_bucket(msg.ts) != day_bucket(prev.ts)`, push a `<DaySeparator>` view before the row. For the first message of the list, emit one too.

- [ ] **Step 2.4 — `just test-browser`** (day-separator renders `today` / `yesterday` / dated label given fixture timestamps). Expect green.

- [ ] **Step 2.5 — Commit** — `ui(phase-2): add day separators between local-date boundaries`.

### 3. Mention parsing + pills

Resolve `@handle` tokens against channel peers and render coloured pills. Prepares the signal feed for self-mention row highlight (Task 4).

**Files:** new `crates/web/src/components/message_row/mention.rs`, modify `crates/web/src/components/message.rs`, modify `crates/web/style.css`.

- [ ] **Step 3.1 — Parser.** `parse_mentions(body, peers: &[PeerRef], local_peer: &EndpointId) -> Vec<Segment>` where `Segment` is `Text(String) | Mention { label, peer_id, is_self }`. Regex `@([a-z][a-z0-9._-]*)` (case-insensitive). Resolve in order: exact handle → first segment of handle → display-name → literal `@you` → local peer. Unresolved → fall through to `Text`.

- [ ] **Step 3.2 — `<MentionPill>`.** Variants `Peer` (moss colour-mix bg, moss-3 fg, moss-1 border) and `Self_` (amber 28% bg, amber fg, amber-soft border). Opens profile popover on click (wire to `use_context::<AppState>()::ui::open_profile` if present; else no-op + `TODO(profile-card.md)`).

  ```rust
  view! {
      <button class=move || if is_self { "mention-pill mention-pill--self" } else { "mention-pill" }
              aria-label=format!("mention {label}")>
          "@" {label}
      </button>
  }
  ```

- [ ] **Step 3.3 — Wire segment pipeline.** In `MessageView`, run mentions → inline code → urls (order matters: mentions must run first so `@mira` inside backticks stays code; inline-code protects against mention regex inside fences). Keep existing URL handler as the tail stage.

- [ ] **Step 3.4 — Unit tests.** In the new file's `#[cfg(test)]`: exact handle match, first-segment match, display-name match, `@you` self-alias, unresolved fallthrough, long-handle truncation (>32 chars → `first 28 + …`).

- [ ] **Step 3.5 — `just check`** — expect clean fmt/clippy/tests.

- [ ] **Step 3.6 — Commit** — `ui(phase-2): parse @handle mentions + render MentionPill`.

### 4. Self-mention row highlight + sidebar signal

Wire `messageMentionsMe` as the source of truth for both the row highlight and the sidebar unread-mentions counter.

**Files:** modify `crates/client/src/state.rs` (add `mentions: Vec<EndpointId>`), modify `crates/client/src/views.rs` (populate `mentions` during projection), modify `crates/web/src/components/message.rs`, modify `crates/web/style.css`.

- [x] **Step 4.1 — Extend `DisplayMessage`.** Add `pub mentions: Vec<EndpointId>` (default `vec![]`). Projection populates via `parse_mentions` (shared module usable from both client + web).

  **Decision:** put the parser in `willow-client` (not `willow-web`) so projection can populate it; `<MentionPill>` stays in web. Mention parser is text-only, no WASM-specific deps.

- [x] **Step 4.2 — `messageMentionsMe`.** Helper `pub fn mentions_me(m: &DisplayMessage, local: &EndpointId) -> bool` — `m.mentions.contains(local) || body parser finds any segment with is_self==true`.

- [x] **Step 4.3 — Row class.** In `MessageView`, append `message--mention` when `mentions_me`. CSS: `background: color-mix(in oklab, var(--amber) 8%, transparent); box-shadow: inset 2px 0 0 var(--amber);`.

- [x] **Step 4.4 — Sidebar counter source.** `UnreadStats.mentioned` now derives from `mentions_me` over the tail-slice of each channel (capped at 500 per spec). 1f's stub is swapped in-place; the public shape (`mentioned: bool`) is unchanged so sidebar / grove-tile renderers keep reading it via the existing path.

- [x] **Step 4.5 — `just test-client`** — 4 new projection tests + 3 new `mentions_me` unit tests all green.

- [x] **Step 4.6 — Commit** — `ui(phase-2): highlight self-mention rows + feed sidebar mention counter`.

### 5. Inline + fenced code

Replace the current "url-only" body pipeline with mentions → code → urls.

**Files:** new `crates/web/src/components/message_row/code.rs`, modify `crates/web/src/components/message.rs`, modify `crates/web/style.css`, modify `crates/web/src/icons.rs`.

- [x] **Step 5.1 — `parse_code_segments`.** Split body on triple-backtick fences (`^```(?<lang>\w+)?\n…\n```$` on a line, non-greedy), then on single backticks (non-multiline) within remaining plain-text segments.

- [x] **Step 5.2 — `<InlineCodePill>` + `<FencedCodeBlock>`.** Inline: `<code class="code-inline">{text}</code>`. Fenced: `<pre class="code-fenced">…<button class="code-copy-btn">` — copy routes through the shared `crate::util::copy_to_clipboard` helper (clipboard API + textarea fallback, same surface as invite-code copy); on success swap icon to `icon_check` for 900 ms using leptos `set_timeout`. Re-uses existing `icons::icon_copy` / `icons::icon_check` — no new icon glyphs.

  ```rust
  view! {
      <pre class="code-fenced">
          <button class="code-copy-btn" on:click=copy aria-label="copy code">
              { move || if copied.get() { icons::icon_check() } else { icons::icon_copy() } }
          </button>
          <code>{text}</code>
      </pre>
  }
  ```

- [x] **Step 5.3 — Wire in `MessageView`.** Replace the existing `extract_urls` direct call with the new pipeline (mentions from task 3 → code → urls). Reply-preview stays plain-text (renders via `format!("> {preview}")`, no parser pipeline) — guarded by `reply_preview_stays_plain_text` browser test.

- [x] **Step 5.4 — `just test-browser`** — 4 new tests: inline-backtick pill renders, fenced-block + copy button renders, mixed inline+fenced in one body, reply-preview stays plain text. Plus 10 parser unit tests in `code.rs` (`parse_no_backticks`, `parse_inline_only`, `parse_fenced_only`, `parse_fenced_with_lang`, `parse_unmatched_backtick`, `parse_unmatched_fence`, `parse_triple_backtick_inline_is_not_fence`, `parse_inline_does_not_span_newline`, `parse_mixed_inline_and_fenced`, `parse_fence_with_junk_after_lang_falls_back_to_text`). All green.

- [ ] **Step 5.5 — Commit** — `ui(phase-2): render inline + fenced code with copy button`.

### 6. Pinned marker

Add `pinned: bool` to `DisplayMessage` and render a 1px amber left rule + badge. Pinning *action* + panel are owned by `reactions-pins.md` (already partly in place); this phase just reads the projection.

**Files:** modify `crates/client/src/state.rs`, modify `crates/client/src/views.rs`, modify `crates/web/src/components/message.rs`, modify `crates/web/style.css`.

- [x] **Step 6.1 — Extend `DisplayMessage`.** Added `pub pinned: bool`; the projection in `views::compute_messages_view` stamps each message with `pinned = events.channels[cid].pinned_messages.contains(&msg.id)`.

- [x] **Step 6.2 — Row class + badge.** `.message--pinned` appended via the pinned branch of `msg_class`; CSS at `crates/web/style.css` uses `inset 1px 0 0 var(--amber)` per spec "pin is a quiet mark". The `.meta` row renders `<span class="pinned-badge" aria-label="pinned">{icon_pin()} pinned</span>` when `pinned` (first-of-run only; pinned rows always break a run via the chat.rs predicate). A `.message--pinned.message--mention` override lets the 2 px mention rule win the left-rule stack.

- [x] **Step 6.3 — Client test.** Three projection tests in `crates/client/src/views.rs` `tests` mod (`projection_pinned_false_when_not_pinned`, `projection_pinned_true_when_channel_lists_message`, `projection_pinned_flips_back_false_on_unpin`). Two browser tests in `phase_2a_message_row` (`row_has_pinned_class_when_pinned`, `row_has_no_pinned_class_when_unpinned`).

- [x] **Step 6.4 — `just test-client`** — 7 `views::tests` green (3 new + 4 pre-existing).

- [x] **Step 6.5 — Commit** — `ui(phase-2): render pinned marker + badge on DisplayMessage.pinned`.

### 7. Queue notes (LateArrival + Pending)

Add `queue_note: QueueNote` to `DisplayMessage` and render hints / badges / opacity / delivery flash.

**Files:** modify `crates/client/src/state.rs`, modify `crates/client/src/views.rs`, modify `crates/web/src/components/message.rs`, modify `crates/web/style.css`.

- [ ] **Step 7.1 — `QueueNote` enum.** In `willow-client`:

  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum QueueNote { None, LateArrival, Pending }
  ```

  Re-export from `lib.rs`.

- [ ] **Step 7.2 — Projection.** Derive during `views::compute_messages`:
  - `Pending` when `message.is_local && message_store.delivery_state(&msg.id) == NotAcked`.
  - `LateArrival` when `!message.is_local && peer_was_offline_at(msg.author_peer_id, msg.timestamp_ms)` — use `ServerState::peer_presence_history` if available; else fallback to "author peer was unreachable within 30s of authoring" (cross-reference `sync-queue.md` dependency note — flag inline `TODO(sync-queue.md)` if the presence-history store isn't landed yet).
  - `None` otherwise.

- [ ] **Step 7.3 — Row rendering.** For `LateArrival`: inline hint `<span class="queue-note late">{icon_hourglass()} "sent earlier · arrived now"</span>` under the body. For `Pending`: hint `queued · will send on reconnect`, row `opacity: 0.7`. Both get the `queued` badge in `.meta`.

- [ ] **Step 7.4 — Delivery flash.** When a `Pending` transitions to `None` (delivery ack), drive a 900 ms `check + sent` flash: local `RwSignal<bool>` on the row, set true in an `Effect::new` that watches `queue_note`; `set_timeout` resets after 900 ms. Opacity fades to 1 over 180 ms via transition.

- [ ] **Step 7.5 — Client test.** `crates/client/src/tests/display_message.rs` — Pending for local unacked; LateArrival when peer was offline; flips to None on ack.

- [ ] **Step 7.6 — `just test-client` + `just test-browser`** — expect green.

- [ ] **Step 7.7 — Commit** — `ui(phase-2): derive + render queue_note hints with delivery flash`.

### 8. Whisper hand-off placeholder

Reserve layout + styling surface so the later `whisper-mode.md` phase only has to flip the gate. Add `whisper: bool` but hard-code `false`.

**Files:** modify `crates/client/src/state.rs`, modify `crates/web/src/components/message.rs`, modify `crates/web/style.css`.

- [x] **Step 8.1 — Field + always-false gate.** `pub whisper: bool` added to `DisplayMessage` with `TODO(whisper-mode.md)` comment in `views::compute_messages_view`. All three construction sites (projection, `mentions.rs` test helper, `browser.rs` `make_msg`) updated.

- [x] **Step 8.2 — Row styling.** `.message.message--whisper` CSS appended to `crates/web/style.css` per spec §Whisper hand-off: violet 2 px left rule + 8% tinted bg, italic body on `--ink-2`. Run-break predicate in `chat.rs` now breaks runs on `prev.whisper || msg.whisper` — Task 1's last deferred rule is fully wired.

- [x] **Step 8.3 — `whisper` badge.** `<span class="whisper-badge" aria-label="whisper">{icon_ear()} whisper</span>` renders in `.meta` when `is_whisper`. `icon_ear` already existed.

- [x] **Step 8.4 — Browser test.** Three tests added to `phase_2a_message_row` (`row_has_whisper_class_when_whisper`, `whisper_badge_has_aria_label`, `row_has_no_whisper_class_by_default`) using inline `DisplayMessage { whisper: true, ..make_msg(..) }` construction — no feature flag needed since fields are public.

- [x] **Step 8.5 — Commit** — `ui(phase-2): reserve whisper row styling behind always-false gate`.

### 9. Empty / loading states

Replace the current `"No messages yet. Say hello!"` + spinner with the spec's leaf illustration + copy; add cleared-after-deletion; add 5-skeleton shimmer.

**Files:** modify `crates/web/src/components/chat.rs`, modify `crates/web/src/icons.rs`, modify `crates/web/style.css`.

- [x] **Step 9.1 — Leaf icon.** `icon_leaf()` — 48×48, stroke 1.5, `currentColor=var(--willow)`. Placeholder path if asset not ready; flag `TODO(illustration)` if the designer hasn't shipped the final SVG.

- [x] **Step 9.2 — Empty variants.** In `MessageList`, split the current empty branch into three:
  - Never-had-messages → leaf + `this channel is quiet. say hi?` + subtext `messages here are sealed to everyone in the grove.`.
  - All-deleted (messages was non-empty last tick, now empty) → `cleared — nothing here yet.`. Track via a local `prev_len` signal.
  - Loading → five `<div class="chat-skeleton-row">` with 32px circle + 2 shimmer bars; uses `shimmer` keyframes from foundation. Reduced-motion → static `--bg-2` rectangles.

- [x] **Step 9.3 — Cross-fade.** First real message cross-fades skeletons out over 180ms via `opacity` transition; no-op under reduced motion.

- [x] **Step 9.4 — `just test-browser`** — 3 new tests (empty-never, cleared, loading skeleton count = 5). Expect green.

- [x] **Step 9.5 — Commit** — `ui(phase-2): add spec-compliant empty/cleared/loading states`.

### 10. Jump-to-latest pill

Replace the current `.scroll-to-bottom` "New messages" button with the spec `jump to latest · {N} new` pill and the 120px auto-scroll gate.

**Files:** new `crates/web/src/components/message_row/jump_latest.rs`, modify `crates/web/src/components/chat.rs`, modify `crates/web/style.css`.

- [x] **Step 10.1 — Auto-scroll gate.** In `MessageList`, tighten the "was near bottom" check from 200px → 120px per spec. Track `new_count` as the number of messages arrived since the user last scrolled to bottom.

  ```rust
  let was_at_bottom = (scroll_height - scroll_top - client_height) < 120.0;
  if was_at_bottom { set_new_count.set(0); }
  else if is_new { set_new_count.update(|n| *n += delta); }
  ```

- [x] **Step 10.2 — `<JumpToLatestPill>` component.** Renders `<button class="jump-to-latest" aria-label="jump to latest messages">{icon_chevron_down()} "jump to latest" {move || if n.get() > 0 { format!(" · {} new", n.get()) }}</button>`. Smooth-scroll on click; clears `new_count`.

- [x] **Step 10.3 — Positioning.** CSS: desktop `bottom: 16px; right: 16px`; mobile `bottom: 80px; right: 12px` (above tab bar / composer). Auto-hide when within 120px.

- [x] **Step 10.4 — `just test-browser`** — 4 pill-level tests (aria label, count visible, count hidden, click callback) plus the repurposed `jump_to_latest_pill_hidden_when_near_bottom` mount/unmount test. 212 browser tests pass (up from 209 pre-Task-10). Smooth-scroll uses `scrollIntoView({ behavior: 'smooth' })` on the list's last child because the `ScrollToOptions` web-sys feature provokes a wasm-bindgen-test startup crash in this workspace's pinned toolchain.

- [x] **Step 10.5 — Commit** — `ui(phase-2): add jump-to-latest pill with 120px auto-scroll gate`.

### 11. Swipe-left quote-reply

Add a second horizontal swipe gesture distinct from the existing swipe-right-opens-thread.

**Files:** modify `crates/web/src/components/message.rs`, modify `e2e/helpers.ts`.

- [x] **Step 11.1 — Gesture handlers.** `MessageView` now tracks `(dx, dy)` across touchstart/move/end via shared `Rc<Cell<(f64, f64)>>` + `Rc<Cell<bool>>` capture flag. Horizontal-dominance gate (`dx.abs() > 1.2 * dy.abs() && dx.abs() > 8.0`) keeps vertical scroll winning until the row captures. On release: `dx > 60` → new optional `on_open_thread` callback (no-op if unwired, reserving the thread-pane surface); `dx < -60` → existing `on_click` callback (reply path already wired at both `app.rs` and `mobile_shell.rs` call-sites to populate `chat.set_replying_to`). A `first_touch(ev)` helper via `js_sys::Reflect` tolerates the synthetic `Event`s dispatched by the browser-test harness (avoids a `TypeError` on `ev.touches()`).

- [x] **Step 11.2 — Snap-back.** `.message` transition extended to include `transform 200ms ease-out`; `.message.is-dragging` disables the transition so the translate tracks the finger 1:1. `@media (prefers-reduced-motion: reduce)` drops `transform` from the transition list (instant state change per spec). `touch-action: pan-y` on `.message` prevents native horizontal panning from stealing the gesture.

- [x] **Step 11.3 — E2E helper.** `swipeLeft(page, row)` + `swipeRight(page, row)` added to `e2e/helpers.ts` (shared `dispatchSwipe` fires touchstart → 3× touchmove → touchend with four waypoints so the dominance gate trips). New mobile-chrome Playwright case in `e2e/mobile-actions.spec.ts` asserts `.reply-bar` becomes visible after `swipeLeft` on the first message row. `npx playwright test --project=mobile-chrome e2e/mobile-actions.spec.ts` → 3 passed.

- [x] **Step 11.4 — Commit** — `ui(phase-2): add swipe-left quote-reply gesture`.

### 12. Hover toolbar rendering

Render the desktop-only floating toolbar at top-right of the row on mouseenter. Actions delegate to already-owned callbacks (reply, thread, reactions, more); this commit just lays out the buttons and wires the two we already own.

**Files:** modify `crates/web/src/components/message.rs`, modify `crates/web/style.css`, modify `crates/web/src/icons.rs`.

- [x] **Step 12.1 — Markup.** Replaced the single `.message-actions` `…` trigger with a `<div class="message-hover-toolbar" role="toolbar" aria-label="message actions">` wrapping: 5 quick-reaction placeholder `.toolbar-btn--quick-react` buttons (emoji literals until `reactions-pins.md` lands), a `.toolbar-divider`, then `smile` / `thread` / `ear` / `more-horizontal` buttons. The `more-horizontal` button retained the `.action-trigger` class so its existing `show_dropdown` toggle still opens the overflow menu; the dropdown contents (Reply / Pin / React / Edit / Delete / Download) stayed unchanged. The ear (whisper reply) is a layout-only placeholder behind a `TODO(whisper-mode.md)` comment — click is a no-op. Thread button is wired through the existing `on_open_thread` Callback from Task 11.

  ```rust
  view! {
      <div class="message-hover-toolbar">
          // 5 quick reactions (delegate — owned by reactions-pins.md; render
          // placeholder IconBtns now so layout is stable).
          <For each=move || quick_reactions.get() key=|e| e.clone() let:emoji>
              <button class="toolbar-btn" aria-label=format!("react with {emoji}")
                      on:click=move |_| react_cb.run((msg.clone(), emoji.clone()))>
                  {emoji.clone()}
              </button>
          </For>
          <span class="toolbar-divider"/>
          <button class="toolbar-btn" aria-label="more reactions">{icons::icon_smile()}</button>
          <button class="toolbar-btn" aria-label="start thread">{icons::icon_thread()}</button>
          <button class="toolbar-btn" aria-label="whisper reply">{icons::icon_ear()}</button>
          <button class="toolbar-btn" aria-label="more actions">{icons::icon_more_horizontal()}</button>
      </div>
  }
  ```

- [x] **Step 12.2 — CSS.** Appended in `crates/web/style.css` under `/* ── Phase 2a Task 12 · Desktop hover toolbar ──*/`. `.message-hover-toolbar` is `inline-flex` with `gap: 2px; padding: 3px; background: var(--bg-1); border: 1px solid var(--line); border-radius: 10px; box-shadow: var(--shadow-2)`; fades in via `opacity` + `pointer-events` on `.message:hover` / `.message:focus-within` over `var(--motion-fast, 120ms)`. `.toolbar-btn` is a 26 × 26 transparent button with `--ink-2` → `--ink-1` on hover; `.toolbar-divider` is a 1 px `--line` strip. `@media (prefers-reduced-motion: reduce)` drops the transition to `none`; `@media (max-width: 720px)` hides the toolbar (mobile long-press sheet owns that surface). The outer `.message-actions` is now a positioning-only anchor (its `display: none` was removed) so the toolbar inside controls visibility via opacity — this keeps the dropdown's absolute positioning stable and avoids layout thrash on hover.

- [x] **Step 12.3 — Wire reply + thread.** Thread button routes through the `on_open_thread` Callback added in Task 11 (same surface the swipe-right gesture uses). Quick-reaction buttons route through the existing `on_react` callback immediately. Reply is still served by the dropdown's Reply item (via `more-horizontal` → `show_dropdown`) plus the swipe-left gesture — a dedicated reply toolbar button is out-of-scope for this task (the spec's toolbar list covers quick-reacts / smile / thread / ear / more-horizontal and never a reply chevron).

- [x] **Step 12.4 — `just test-browser`** — 3 new tests in `phase_2a_message_row`: `hover_toolbar_renders_all_buttons` (asserts `role=toolbar`, 5 quick-reaction buttons with `react with {emoji}` aria-labels, divider, and the 4 trailing buttons with exact `aria-label` text), `hover_toolbar_more_actions_toggles_dropdown` (click → `.message-dropdown` mounts), `hover_toolbar_quick_react_fires_callback` (click first quick-react → `on_react` receives (msg, emoji) pair). Captured via `RwSignal` because leptos `Callback` requires `Send + Sync`. 215 browser tests pass (up from 212).

- [x] **Step 12.5 — Commit** — `ui(phase-2): render desktop hover toolbar anatomy`.

### 13. Long-press action sheet copy + dismissal

Align the existing mobile long-press sheet with the spec's exact copy list and dismissal rules. The 500ms + haptic + swipe-down path is already in place.

**Files:** modify `crates/web/src/components/message.rs`, modify `e2e/mobile-actions.spec.ts`.

- [x] **Step 13.1 — Copy alignment.** All sheet items now render the lowercase spec copy — `reply`, `reply in thread`, `add reaction`, `pin`/`unpin` (via `pin_label.to_lowercase()`), `copy text`, `edit`, `delete`, `cancel` — in the spec's order with the quick-emoji row at the top (capped at 6 slots per spec). `reply in thread` always renders (falls back to a no-op when `on_open_thread` is unwired, per `thread-pane.md` TODO). `add reaction` is a stand-in that keeps the sheet open until `reactions-pins.md` lands the full picker. `copy text` routes through the shared `crate::util::copy_to_clipboard` helper.

- [x] **Step 13.2 — Dismissal path.** `on_sheet_touchend` now uses `drag >= 80.0 || velocity > 200.0` (tightened from `>`) with a spec-citation comment. Overlay tap and sheet drag are unchanged — the overlay's own click handler dismisses, and `transition: none` during drag keeps the finger tracking 1:1.

- [x] **Step 13.3 — E2E case.** `e2e/mobile-actions.spec.ts` adds two tests: `action sheet renders spec copy verbatim` (asserts every label is visible with an anchored-regex match) and `fast downward swipe dismisses by velocity` (60 px over ~60 ms real wall-time → ~1000 px/s, past the 200 px/s threshold but below the 80 px distance threshold, so the dismiss must fire from the velocity branch). `e2e/helpers.ts::messageAction` now does a case-insensitive `hasText` match so existing call-sites passing `'Reply'`/`'Edit'`/`'Delete'` still work. `npx playwright test e2e/mobile-actions.spec.ts --project=mobile-chrome` → 5 passed.

- [x] **Step 13.4 — Commit** — `ui(phase-2): align action-sheet copy + dismissal with spec`.

### 14. Copy pass (exact strings)

Single-commit string pass aligning every user-visible string owned by the message row with the spec §Copy table.

**Files:** modify `crates/web/src/components/message.rs`, modify `crates/web/src/components/chat.rs`.

- [ ] **Step 14.1 — Delete confirm.** Replace the current `"Delete Message"` / `"Are you sure you want to delete this message?"` / `"Delete"` / `"Cancel"` with:
  - Title: `withdraw message?`
  - Body: `this removes it from every peer's view. it was already read by some.`
  - Confirm: `withdraw`
  - Cancel: `keep`

- [ ] **Step 14.2 — Edited suffix.** Confirm `(edited)` (already in place).

- [ ] **Step 14.3 — Deleted placeholder.** Replace current `class="body deleted"` text path — when `message.deleted`, render `this message was withdrawn` in `--ink-3` italic.

- [ ] **Step 14.4 — Empty-body fallback.** When `message.body.trim().is_empty() && !message.deleted`, render `empty message` (per spec edge case).

- [ ] **Step 14.5 — Unknown peer fallback.** In projection, when display name + handle are both missing, use `unknown peer` in `--ink-3` italic.

- [ ] **Step 14.6 — `just test-browser`** — 1 test asserting delete-confirm copy table byte-exact.

- [ ] **Step 14.7 — Commit** — `ui(phase-2): align message-row copy with spec table`.

### 15. Accessibility — ARIA + keyboard

Wire the spec's ARIA label table, keyboard path, and screen-reader single-unit announce format.

**Files:** modify `crates/web/src/components/message.rs`, modify `crates/web/src/components/chat.rs`, modify `crates/web/style.css`.

- [x] **Step 15.1 — ARIA label table.** `MessageView` now renders the row as
  `<article role="article" aria-label="message from {name} at {HH:MM}" tabindex="-1">`
  with the timestamp reused from `willow_client::util::format_timestamp` so the
  ARIA string never drifts from the visible `.meta` time. `.author` is a real
  `<button class="author author-btn" aria-label="{name} — open profile">` with
  UA button chrome stripped by `.author-btn` CSS so the visual is unchanged.
  Avatar button and thread-stub labels are deferred: the message row does not
  render a distinct avatar element today (avatar column is CSS-only) and the
  thread stub lives in `thread-pane.md`, which hasn't landed. The
  jump-to-latest pill already carries `aria-label="jump to latest messages"`
  from Task 10.

- [x] **Step 15.2 — Keyboard path.** `MessageList` tracks `focused_idx: RwSignal<usize>`
  and installs a list-level `on:keydown` handler: ArrowUp/Down + Home/End move
  focus (clamped to `[0, len)`); Enter clicks the focused row's `.action-trigger`
  so the overflow dropdown opens; Escape fires `on_focus_composer`; `R` → `on_message_click`,
  `P` → `on_pin`, `E` → `on_edit` (gated on `is_local`), `Delete`/`Backspace` →
  `on_delete` (gated on `is_local`), `C` copies the body via `crate::util::copy_to_clipboard`,
  `+`/`:` fire the thumbs-up quick-reaction through `on_react` as a stand-in
  until `reactions-pins.md` lands the full picker. `T` consumes the keystroke
  so literal `t` doesn't leak to the composer (thread-pane is deferred).

- [x] **Step 15.3 — `aria-live`.** `.message-list` now carries
  `role="log"` + `aria-live="polite"` + `aria-label="channel messages"` +
  `tabindex="0"` so the list is the single Tab stop, arriving messages
  announce while focused, and the log region is named.

- [x] **Step 15.4 — Color-independent cues.** Audit confirms all four cues
  have a non-colour signifier: `.mention-pill` already carries `font-weight: 500`
  (Task 3); whisper has violet left-rule + `icon_ear` + italic body (Task 8);
  queued has `icon_hourglass` + inline text (Task 7); pinned has 1px amber
  left-rule + `icon_pin` + `pinned` label (Task 6). No new icons needed.

- [x] **Step 15.5 — `just test-browser`** — 7 new tests added to
  `phase_2a_message_row`: `message_row_has_article_role_and_aria_label`,
  `author_button_has_open_profile_aria_label`,
  `message_list_container_has_log_role_and_aria_live`,
  `arrow_down_advances_focus_across_rows`, `arrow_up_at_top_stays_at_top`,
  `escape_fires_on_focus_composer_callback`, `r_key_fires_reply_callback_on_focused_row`.
  226 browser tests pass (up from 219).

- [x] **Step 15.6 — Commit** — `ui(phase-2): wire message-row ARIA contract + keyboard path`.

### 16. Edge cases + browser test coverage

Consolidate the §Edge cases sweep + fill out the `phase_2a_message_row` browser module.

**Files:** modify `crates/web/src/components/message.rs`, modify `crates/web/style.css`, modify `crates/web/tests/browser.rs`.

- [ ] **Step 16.1 — 500-char single-word wrap.** CSS: `.message .body { word-break: break-word; overflow-wrap: anywhere; min-width: 0; }`.

- [ ] **Step 16.2 — RTL.** `.message .body { unicode-bidi: plaintext; direction: auto; }`.

- [ ] **Step 16.3 — Zero-width chars.** No-op (Leptos renders as text, not `innerHTML`). Mention regex: add a strip step `body.chars().filter(|c| !matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}')).collect::<String>()` before resolving.

- [ ] **Step 16.4 — Long-handle mention truncation.** In `MentionPill`, when `label.len() > 32`, render `first 28 + …` with full handle in `title`.

- [ ] **Step 16.5 — Edit after 24h.** Confirm `(edited)` is the only marker; no timeline. Already covered by Step 14.2.

- [ ] **Step 16.6 — `phase_2a_message_row` module.** Append to `crates/web/tests/browser.rs`:

  ```rust
  mod phase_2a_message_row {
      use super::*;
      // ~18 tests covering every §Acceptance row:
      // - row_anatomy_renders_avatar_name_handle_timestamp
      // - run_collapses_same_author_within_5min
      // - run_breaks_on_whisper_pin_queue_note
      // - collapsed_row_shows_hover_timestamp
      // - day_separator_today_yesterday_weekday_older
      // - mention_pill_peer_variant
      // - mention_pill_self_variant
      // - self_mention_row_amber_rule
      // - inline_code_backtick_pill
      // - fenced_code_pre_with_copy
      // - pinned_marker_1px_rule_and_badge
      // - queue_note_pending_opacity_hint
      // - queue_note_late_arrival_hint
      // - whisper_placeholder_violet_rule
      // - empty_leaf_and_copy
      // - cleared_after_deletion_copy
      // - loading_5_skeletons
      // - jump_to_latest_appears_above_120px
      // - keyboard_arrowup_moves_focus
      // - delete_confirm_copy_byte_exact
  }
  ```

- [ ] **Step 16.7 — `just test-browser`** — full `phase_2a_message_row` module green.

- [ ] **Step 16.8 — Commit** — `ui(phase-2): add phase_2a_message_row browser coverage + edge cases`.

## Ambiguity decisions

- **Mention parser home.** Lives in `willow-client` (not `willow-web`) so projection can populate `DisplayMessage::mentions`. Text-only, no wasm deps.
- **Queue-note LateArrival source.** Uses presence-history on `ServerState` if available; otherwise a 30s-of-authoring unreachable fallback with a `TODO(sync-queue.md)` comment. Block-ready enough to ship the UI contract.
- **Whisper gate.** `whisper: bool` on `DisplayMessage` is hard-coded `false` until `whisper-mode.md` lands a `WhisperStart` EventKind. Styling is reserved.
- **Reactions in hover toolbar.** The 5 quick-reaction slots render placeholder IconBtns reading `quick_reactions.get()` — populated by `reactions-pins.md` in a later phase. This commit wires the layout + the click handler against the existing `on_react` callback.
- **Day-separator bucketing.** Local-timezone via `js_sys::Date`. Native `cargo test -p willow-client` can't test bucketing (JS APIs absent); bucketing test lives in the wasm-pack browser module.
- **Profile popover click.** `MentionPill` click is a no-op until `profile-card.md` lands. `TODO(profile-card.md)` comment on the click handler.
- **Empty-body fallback vs. deleted.** Deleted uses `this message was withdrawn`; empty but not-deleted (migration edge case) uses `empty message`.
- **Sidebar mention counter overlap with 1f.** 1f's `UnreadStats.mentioned` was a substring heuristic. This phase swaps it to `mentions_me(m, local)`; no API break because 1f's public surface (the `mentioned: u32` field) stays.

## Acceptance criteria (mirrors spec §Acceptance criteria)

- [ ] Message row renders avatar (32 px desktop, 36 px mobile), display name (Fraunces 15 px, italic for `you`), mono handle, `11 px --ink-3` timestamp, and body with density-aware padding.
- [ ] Consecutive same-author messages within 5 min collapse into a run: avatar hidden, meta row hidden, padding tightened, hover reveals a mono timestamp in the avatar column.
- [ ] Whisper, pinned, and queueNote always break a run.
- [ ] Day separators render between messages from different local dates using copy in §Copy.
- [ ] `@mention` tokens become pills in the correct variant; `messageMentionsMe` matches either `mentions[]` or parsed body; self-mention rows carry the amber left rule + background.
- [ ] Hover toolbar (desktop) appears on mouseenter, offers five quick reactions, thread, whisper, more; all buttons carry the ARIA labels in §Accessibility.
- [ ] Long-press ≥ 500 ms opens the bottom action sheet; swipe-down at 80 px *or* velocity > 200 px/s dismisses; haptic fires on open.
- [ ] Pinned messages render with a 1 px amber left rule and a `pinned` badge.
- [ ] Fenced code renders in mono with `--bg-0` + `--line` border; a copy button appears on hover (desktop).
- [ ] Queue notes render the inline hint + badge; pending messages dim to 0.7 opacity until delivered; delivery flashes `sent`.
- [ ] Whisper rows carry the violet left rule, tinted background, and whisper badge (full styling in `whisper-mode.md`).
- [ ] Empty channel shows the leaf illustration and the copy in §Copy.
- [ ] Scroll anchoring: auto-scroll only when within 120 px of bottom; otherwise a `jump to latest` pill with unread count appears.
- [ ] Every interactive element has an ARIA label per §Accessibility.
- [ ] Every interaction has a keyboard path; reduced motion collapses animations per foundation.
- [ ] 500-char single-word messages wrap without breaking layout.

## Self-review

- [x] Every §Acceptance row mapped to a task.
- [x] Foundation tokens only — `--amber`, `--amber-soft`, `--whisper`, `--moss-*`, `--msg-pad`, `--motion-fast`, `--shadow-2`. No new hex.
- [x] Every commit is `ui(phase-2): <imperative>`.
- [x] `e2e/helpers.ts` + `e2e/mobile-actions.spec.ts` updated in same commits as markup rename (feedback_e2e_in_sync memory).
- [x] Lowest-tier test per behaviour: state crate → N/A (no new events); client crate → mentions + queue-note + pinned projection; browser → DOM + signals; Playwright → gestures + sheet dismiss (feedback_test_tier_selection memory).
- [x] Whisper remains an explicit always-false gate; no quiet shipping of un-specced behaviour.
- [x] No placeholders, no TBDs.
