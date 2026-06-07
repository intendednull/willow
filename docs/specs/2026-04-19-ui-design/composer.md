# Composer — textarea, reply / edit bars, mentions, typing indicator

**Parent:** [README.md](README.md)
**Status:** landed (e46e4b3, 2026-05-08)
**Dependencies:** [`foundation.md`](foundation.md),
[`layout-primitives.md`](layout-primitives.md),
[`message-row.md`](message-row.md)
**Consumed by:** [`thread-pane.md`](thread-pane.md),
[`letters-dms.md`](letters-dms.md), [`call-experience.md`](call-experience.md)

## Purpose

The composer is the input surface beneath every message list. This spec
owns the compose textarea (desktop + mobile), the reply preview bar, the
edit bar, keybindings, offline state, mention autocomplete, and the
typing indicator above the composer. Empty-state copy *inside* the
channel ("say hi?") is owned by [`message-row.md`](message-row.md);
placeholder copy *inside* the composer for different channel kinds is
owned here.

## Scope

**In scope.** Composer layout on desktop and mobile. Autogrow textarea,
attach button, emoji button, send button. Reply preview bar (above the
composer) + edit bar + flow from the row actions (reply / edit). Send
keybindings (⌘↵/Ctrl+↵, Shift+Enter newline, Esc cancels reply/edit,
ArrowUp to edit last, etc.). Mention autocomplete popover. Offline
state styling of the compose surface. Typing indicator rendering + the
throttle rules ("mira is typing…", 2 names, "4 people typing…").
Placeholder copy per channel kind (text, voice, ephemeral, thread).

**Out of scope (hand-off).** File upload dialog, attach icon target
affordances beyond the button, drag-and-drop, paste-to-upload →
[`files-inline.md`](files-inline.md). Emoji picker popover when opened
from the compose surface's emoji button uses the same component as the
add-reaction picker; the component is owned by
[`reactions-pins.md`](reactions-pins.md). Empty channel state
illustration and "say hi?" copy → [`message-row.md`](message-row.md).

## Composer

Vertical stack: optional reply bar → optional edit bar → compose surface.

### Desktop compose surface

- `--bg-2` on `--line`, radius 12 px, `--shadow-1`, outer `8px 10px`,
  inner vertical flex `8px` gap.
- **Upper row.** Attach IconBtn (`plus`) → auto-grow textarea →
  reserved `gift` IconBtn (feature-flagged, v1 hides) → emoji IconBtn
  (`smile`) → send button.
- **Send button.** Pill, `6px 10px`, `--moss-1` background,
  `--moss-4` foreground, `send` icon + text `send`. Disabled when
  the trimmed textarea is empty.
- **Textarea.** Transparent, no border, `--ink-0`, 14 px.
  `min-height: 1.45em`, grows by `scrollHeight` up to 8 lines then
  scrolls. Placeholder: `message #{channel} — encrypted to {N} peers`;
  for letters, `message {name}`; offline, `offline — messages queue
  until reconnect`.
- **Meta row.** `lock` + `sealed with grove-keys` (`--ink-3`), `·`
  separator (`--ink-4`), `ear` + `hold shift to whisper` (`shift` in
  mono `--whisper`), flex spacer, optional `{name} is whispering`
  status with a 3-dot `willowPulse` in `--whisper`.

### Mobile compose surface

- `--bg-2` on `--line`, radius 22 px (pill), `6px 8px 6px 12px`.
- Attach button (`plus`, 18 px, `--ink-2`) → input (single-line
  default; multi-line on external Shift+Enter) → whisper button
  (`ear`, 18 px, `--whisper`) → circular 34 × 34 send button
  (`--moss-2` bg, `#14130f` fg, `send` icon; dims to `--ink-4` when
  empty).
- Meta row below: `lock` · `sealed to {N} peers in grove` · `tap ear
  to whisper`.

### Keyboard (desktop)

| Key | Action |
|---|---|
| `Enter` | Send. |
| `Shift + Enter` | Newline. |
| `Ctrl + Enter` / `Cmd + Enter` | Force-send (for users who prefer Enter as newline). |
| `Escape` | Unwinds in order: cancel edit → cancel reply → blur. |
| `Tab` inside textarea | Inserts two spaces (no focus move). |
| `ArrowUp` when textarea empty | Enters edit mode on most recent own message. |
| `@` | Opens mention autocomplete. |
| `:` | Opens emoji shortcode autocomplete. |

### Keyboard (mobile)

Same keyboard table as desktop. Mobile users on touchscreens primarily
submit via the send button, but for users on physical keyboards
(iPad + Magic Keyboard, foldables, Bluetooth setups) `Enter` sends and
`Shift+Enter` inserts a newline — matching the desktop convention so
muscle memory carries across shells. `@` still opens mention
autocomplete.

### Mention autocomplete

Triggered on `@` at a word boundary:

- Popover above the composer, `-8px` offset, aligned to the `@`.
- `--bg-1` on `--line`, radius 10 px, `--shadow-2`.
- List of peers in the current channel, filtered by prefix match on
  handle / first segment / display name. Each row: avatar (20 px),
  display name, handle (mono), status dot.
- Arrow keys move selection; `Enter` / `Tab` inserts the handle pill;
  `Escape` dismisses. Max 8 rows visible; scrolls above.
- Special row `@channel` (mentions all members) visible only with
  `ManageChannels`.

### Offline state

- Compose surface background softens to
  `color-mix(in oklab, var(--amber) 10%, var(--bg-2))`.
- Meta line prepends `hourglass` + `offline · queuing messages`.
- Send still works; messages enter the pending queue.

## Reply and edit

### Reply preview (above composer)

When reply is chosen:

- Bar above composer. `--bg-2` on `--line`, radius 10 px (top only,
  attaches to composer). `6px 12px` padding.
- Contents: 2 px `--moss-2` left rule, `replying to` label
  (`--ink-3`, hint size), parent author (display italic, `--ink-1`),
  truncated body preview (single line, `--ink-2`, ellipsis), flex
  spacer, `cancel` text button (`--ink-2`, hover `--ink-0`).
  `Escape` also cancels.
- Click the preview: scrolls the list to the parent and flashes it
  with a 180 ms `willow-pop-in`.

The sent reply carries `reply_to = parent_id` and a `reply_preview`
(~120 chars of parent body). The rendered reply shows a small preview
block above its own body, clickable to jump.

### Edit mode

Choosing edit on an own message:

- Composer pre-fills with the original body, text selected.
- Above the composer: `editing message · esc to cancel` (hint size,
  `--ink-3`).
- Send button label flips to `save`. Submit calls `edit_message`.
  Escape cancels.
- Edited messages show `(edited)` in `--ink-3` after the timestamp.

## Typing indicator

Thin row just above the composer:

- Padding `4px 24px` desktop, `8px 14px` mobile.
- 3-dot `willowPulse` (staggered 0 / 200 / 400 ms), `--ink-3`.
- Label: `font-display` italic, `--ink-2`.
- Copy:
  - 1: `{name} is writing…`
  - 2: `{name} and {name} are writing…`
  - 3: `{name}, {name}, and {name} are writing…`
  - 4+: `{count} people are writing…`
- Own typing is never shown to self. A peer is "typing" for 4 s after
  their last typing ping. The local client emits at most one ping per
  3 s while textarea is focused and non-empty.

## Copy (exact strings)

Lowercase unless proper noun. This is the source of truth for
translation work for the strings owned here.

### Composer placeholders
- Channel: `message #{channel} — encrypted to {N} peers`
- Letter: `message {name}`
- Offline: `offline — messages queue until reconnect`
- No channel selected: `choose a channel to start`

### Composer meta
- Default: `sealed with grove-keys`
- Offline: `offline · queuing messages`
- Whisper hint (desktop): `hold shift to whisper`
- Whisper hint (mobile): `tap ear to whisper`

### Reply bar
- Label: `replying to`
- Cancel: `cancel` (ARIA `cancel reply`)

### Edit bar
- Label: `editing message · esc to cancel`
- Send label during edit: `save`

### Typing
- `{name} is writing…`
- `{name} and {name} are writing…`
- `{name}, {name}, and {name} are writing…`
- `{count} people are writing…`

### Mention autocomplete
- Empty: `no peer by that handle in this channel`
- `@channel` row label: `everyone in this channel`
- `@channel` row hint: `notifies all members`

### Send error (rare, real failure not offline)
- `couldn't send — you're not permitted to post here.`

## Data dependencies

### Existing methods

`send_message`, `edit_message`.

### New methods required

- `send_typing(channel_id)` — rate-limited (3 s) typing ping.
- `whisper_send(channel_id, body, reply_to)` — defined in
  [`whisper-mode.md`](whisper-mode.md); referenced here as the
  compose-surface extension point.

### New reactive signals on `Client`

- `typing(channel_id) -> ReadSignal<Vec<PeerId>>` — peers who pinged
  `TypingPing` in the last 4 s. Ephemeral gossip, not persisted.
- `connection_state() -> ReadSignal<ConnectionState>` with variants
  `Connected | Degraded | Offline`. Drives composer placeholder, meta
  line, and offline tinting.

### Local (view-scope) state

Active reply target and active edit target are local `RwSignal<Option<DisplayMessage>>`
in the chat view, matching the current `input.rs` contract.

## Edge cases

- **Empty / whitespace-only body.** Cannot be sent (send disabled,
  Enter no-op).

## Accessibility

### ARIA labels

| Element | Label |
|---|---|
| reply bar cancel | `cancel reply` |
| edit bar cancel | `cancel edit` |
| send button | `send` |
| attach button | `attach file` |
| emoji button | `open emoji picker` |

### Focus moves

- On *reply*: focus → composer textarea.
- On *edit*: focus → composer textarea, caret at end.
- On *dismiss sheet / toolbar*: focus returns to the originating row
  (owned by [`message-row.md`](message-row.md)).
- Every focusable element uses `--focus-ring` from foundation.

### Motion

All animations respect `prefers-reduced-motion: reduce` per foundation:
`willowPulse` becomes a static opacity dot.

### Screen reader flow

- Typing indicator announced via `aria-live="polite"` with debouncing
  (at most once per 5 s).

## Acceptance criteria

- [x] Reply: choosing reply focuses the composer and shows a preview
      bar with parent author + excerpt + cancel; `Escape` cancels.
      Clicking the preview scrolls to the parent and flashes it.
- [x] Edit: choosing edit pre-fills the composer, shows the edit
      bar, submits via the edit path, and marks the message `(edited)`.
- [x] Composer autogrows up to 8 lines; `Enter` sends; `Shift+Enter`
      newlines; `Ctrl/Cmd+Enter` always sends; `Escape` unwinds
      edit → reply → blur.
- [x] Mention autocomplete opens on `@` with peer filter; arrow keys
      + `Enter`/`Tab` insert; `Escape` dismisses.
- [x] Offline state: composer applies amber tint; meta line becomes
      `offline · queuing messages`; pending messages show the queue
      hint.
- [x] Typing indicator shows the correct form for 1 / 2 / 3 / 4+
      typers, driven by a per-channel typing signal.
- [x] Every interactive element has an ARIA label per §Accessibility.

## Open questions

- **Edit history.** Should we surface prior edit versions (the event
  log has them)? Default: no in v1 — just `(edited)`. Data remains
  so a later version can.

  **Resolution (2026-04-26, Phase 3a):** defer to v2 — `(edited)` is
  rendered; the raw event log retains every version so a future
  opt-in surface (e.g. an "edit history" submenu in the row toolbar)
  can revisit without a data migration.

- **`@channel` confirmation.** Skip a confirm step below 20 members;
  show one above that threshold. Revisit after governance.

  **Resolution (2026-04-26, Phase 3a):** defer to v2 post-governance —
  v1 ships `ManageChannels`-only gating on the `@channel` row in the
  mention popover (see §Mention autocomplete and `composer.rs`
  `allow_channel_mention`). The threshold-confirmation modal is
  parked behind `governance.md`.

- **Typing ping transport.** A new ephemeral event type that does
  not enter the log needs design review in `willow-state` /
  `willow-network` before this spec's acceptance can be met.

  **Resolution (2026-04-26, Phase 3a):** **already shipped.** No new
  state event needed. Wire format is `WireMessage::TypingIndicator`
  in `crates/common/src/wire.rs`, send path is `Client::send_typing_indicator`
  with a 3 s throttle, receive path is `Client::typing_in(channel)`
  with a 4 s TTL on the in-memory `network_meta.typing_peers` map.
  The composer consumes `typing_in` directly.
