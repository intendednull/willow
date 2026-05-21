# Reactions and pins — emoji reactions, picker, pinned panel

**Parent:** [README.md](README.md)
**Status:** landed (phase 3c PR #634 foundation + 3c.2 PR #635 picker wireup + 3c.3 close-out)
**Dependencies:** [`foundation.md`](foundation.md),
[`layout-primitives.md`](layout-primitives.md),
[`message-row.md`](message-row.md)

## Purpose

Reactions and pins are both per-message actions that augment a row
without changing its body. This spec owns the reactions strip, the
add-reaction affordance, the reactor tooltip / list, the full emoji
picker popover, the pin action (permission-gated), the pinned panel
entry point, and the pinned panel contents. The in-row pin *marker*
(thin amber left rule + `pinned` badge) is owned by
[`message-row.md`](message-row.md); this spec references it.

## Scope

**In scope.** Reactions strip (flex row of emoji chips) with counts and
reactor state. Add-reaction affordance: desktop hover chip and the
mobile action-sheet entry. Emoji picker popover (grid, recent, search,
categories). Reactor tooltip (desktop hover) and reactor list card
(mobile tap-and-hold). Pins: the pin action, who can pin (permission-
gated), the pinned-panel entry point in the channel header (right-rail
slot), the pinned-panel contents (list of pinned messages, jump-to,
unpin).

**Out of scope (hand-off).** Row-level pin marker (thin amber left
rule + `pinned` badge) → [`message-row.md`](message-row.md). The
quick-reactions row in the hover toolbar and the long-press action
sheet — those UI containers live in [`message-row.md`](message-row.md);
the quick-reaction *contents* are specified here. Emoji picker invoked
from the composer's emoji button shares this component; the composer
binding lives in [`composer.md`](composer.md).

## Reactions

### Reaction strip

Rendered below the body when `message.reactions` is non-empty:

- Flex row, `5px` gap, `flex-wrap: wrap`, `margin-top: 6px`.
- Each pill: inline-flex, `5px` gap, `2px 8px` padding, radius `999px`,
  `--bg-2` on `--line`, `--ink-1` text. 12 px desktop, 11 px mobile.
  Contents: emoji glyph + count (`--ink-2`, weight 500).
- Clicking toggles the local user's reaction (`client.react(id, emoji)`).
- If the local user has reacted, the pill uses `--moss-1` border and
  `color-mix(in oklab, var(--moss-2) 18%, var(--bg-2))` background.
- Hover (desktop) shows a tooltip: `mira, ori, kes reacted`; >3
  reactors uses `first two, and N others`. Mobile tap-and-hold on the
  pill exposes the same list as a small card.

### Add-reaction chip

On desktop it appears at the tail of the reaction strip on row hover:
dashed `--line` border, transparent background, `--ink-3`, containing
a `plus` + `smile` icon. Click opens the emoji picker. Hidden on
mobile — the add action is in the action sheet (mobile row sheet
container owned by [`message-row.md`](message-row.md)).

### Quick reactions (desktop hover toolbar / mobile sheet)

Quick reactions default to `👍 ❤️ 🍃 💚 👀`. Override with the five
most recent reactions used in *this channel*. The toolbar / sheet
*containers* are owned by [`message-row.md`](message-row.md); the list
contents and toggling behavior live here. Clicking a quick-reaction
glyph applies the same `client.react(id, emoji)` toggle as the strip.

### Reactions on deleted messages

Allowed in v1 (see Open questions).

## Emoji picker

Popover, 320 × 360, `--bg-1` on `--line`, radius 10 px, `--shadow-2`.
Top search input (mono placeholder `search emoji`). Categories
scrollable: recent (reuses the quick-react recency), smileys, nature,
food, travel, objects, symbols. Arrow keys move selection, `Enter`
inserts, `Escape` closes.

The same component is reused for:

- Add-reaction from the row's add-chip or action sheet.
- The `smile` (more reactions) button in the hover toolbar (container
  owned by [`message-row.md`](message-row.md)).
- The composer's `smile` emoji IconBtn (binding owned by
  [`composer.md`](composer.md)).

## Pins

### Permission + action

Pinning is permission-gated on `ManageChannels`. The pin menu item is
greyed (no-op) for users without the permission, with tooltip
`only stewards can pin here`.

Invocation paths:

- **Desktop.** Overflow menu (`pin` / `unpin`) from the hover toolbar
  (container owned by [`message-row.md`](message-row.md)).
- **Mobile.** Action sheet entry (`pin` / `unpin`) (container owned by
  [`message-row.md`](message-row.md)).
- **Keyboard.** `P` on a focused row when permitted (keybinding owned
  by [`message-row.md`](message-row.md)).

### Header entry point

The channel header's pin IconBtn (owned by
[`layout-primitives.md`](layout-primitives.md)) tints `--amber` when
the channel has pinned messages, with a mono superscript count.
Clicking opens the pinned panel.

### Pinned panel contents

The pinned panel is the right-rail / overlay slot:

- Vertically stacked list of pinned messages in pin-time order,
  newest first.
- Each entry: mini message card — avatar (24 px), display name (body
  weight 500), timestamp (`--ink-3`, 11 px), body preview (2 lines
  max, ellipsis), optional `pinned by {name} · {when}` footer row
  (`--ink-3`, 10 px, mono `when`). The footer is omitted when the
  pinner's `PinMetadata` has not yet materialized for this peer
  (rare; resolves on the next profile sync). Schema for
  `PinMetadata` lives in
  [`../2026-05-21-pinned-message-metadata-design.md`](../2026-05-21-pinned-message-metadata-design.md).
- Entry actions (right-aligned on hover / always on mobile): `jump to`
  (scrolls the channel list to the parent message and flashes it via
  the same 180 ms `willow-pop-in` as reply-jump), `unpin`
  (permission-gated; same grey + tooltip behaviour as the pin action).
- Empty state: `font-display` italic, 13 px, `--ink-3` —
  `nothing pinned yet.`

## Row marker

See [`message-row.md`](message-row.md) for the row-level pin visual
(1 px thin `--amber` left rule + `pinned` badge in the author meta
row; pinned rows always break a run).

## Copy (exact strings)

Lowercase unless proper noun. This is the source of truth for
translation work for the strings owned here.

### Reactor tooltip
- 3 or fewer reactors: `mira, ori, kes reacted`
- More than 3 reactors: `first two, and N others`

### Pins
- Permission-denied tooltip on pin action: `only stewards can pin here`
- Pinned-panel empty state: `nothing pinned yet.`

## Data dependencies

### Existing `DisplayMessage` fields

`reactions: Vec<(String, Vec<PeerId>)>`.

### Existing methods

`react`, `pin_message`, `unpin_message`.

### New surface area

The channel's pinned-messages projection (ordered list of pinned
message IDs + pinner + pin-time) needs to surface from
`willow-state::ServerState` through `willow-client` so the panel can
render without re-scanning the full message store.

## Edge cases

- **Huge reactor list.** Tooltip truncates to `first two, and N others`
  past 3 reactors; the full list is accessible via the reactor list
  card (mobile) or hover tooltip details reveal.
- **Reaction to a deleted message.** Allowed in v1 (see Open questions).
- **Pin by a peer who later has permission revoked.** The pin persists
  (it is a prior authorized event). Future unpin uses current
  permission check, not the original pinner identity.

## Accessibility

### ARIA labels

| Element | Label |
|---|---|
| toolbar react (each) | `react with {emoji}` |
| reaction pill | `{emoji} reacted by {count} — toggle your reaction` |
| add-reaction chip | `add reaction` |
| pin IconBtn (header) | `pinned messages ({count})` |
| pinned panel jump-to | `jump to pinned message` |
| pinned panel unpin | `unpin message` |

### Keyboard path

- `+` or `:` opens the emoji picker on a focused row (keybinding
  defined in [`message-row.md`](message-row.md), binds to the
  add-reaction action here).
- Inside the picker: arrow keys move selection, `Enter` inserts,
  `Escape` closes.

### Color-independent cues

Reactions: emoji + numeric count. Pins: `pin` icon + rule + text.
Color is never the sole signifier.

### Screen reader flow

- Reactions announce as `{emoji} {count}` for others; for the local
  user, `{emoji} {count}, including you`.

## Acceptance criteria

- [x] Reaction pills render with count; click toggles the local
      user's reaction; hover shows reactor list (desktop); add-chip
      appears on row hover (desktop); mobile adds via the action
      sheet. *(`<ReactionStrip>` + `reactor_tooltip` + `<AddReactionChip>`
      — phase 3c.3.)*
- [x] Quick-reaction list feeds both the desktop hover toolbar and
      the mobile action sheet; defaults to `👍 ❤️ 🍃 💚 👀` until a
      channel-scoped recency list supersedes it. *(LRU on `ChatMeta`
      + `ReactionRecency` context — phase 3c + 3c.3.)*
- [x] Emoji picker opens at 320 × 360, with search, categories,
      and recents; arrow keys + `Enter` insert; `Escape` closes.
      *(`<EmojiPicker>` — phase 3c + 3c.2 callsite wiring.)*
- [x] Pin action is greyed for users without `ManageChannels`, with
      the tooltip in §Copy; permitted users can pin and unpin from
      both the desktop overflow menu and the mobile action sheet.
      *(`local_can_manage_channels` + per-entry unpin — phase 3c + 3c.3.)*
- [x] Header pin IconBtn shows a superscript count when > 0 and tints
      amber; click opens the pinned panel. *(`<MainPaneHeader>` +
      `pinned_count` prop — phase 3c.)*
- [x] Pinned panel lists pinned messages newest-first, each entry
      shows a `pinned by {name} · {when}` footer (omitted only when
      pinner metadata is not yet materialized), jump-to scrolls and
      flashes the parent, unpin honours the permission check.
      *(`<PinnedPanel>` rewrite — phase 3c; footer + `PinMetadata`
      state-schema change — phase 3c close-out.)*
- [x] Every interactive element has an ARIA label per §Accessibility.
      *(`add reaction`, `download {filename}`, `react with {emoji}`,
      `{emoji} reacted by {count} — toggle your reaction`,
      `pinned messages ({count})`, `jump to pinned message`,
      `unpin message`, `close pinned panel` — phase 3c + 3c.3.)*

## Open questions

- **Quick-reaction recency scope.** Channel-scoped in v1. Revisit
  after use data: users may expect reaction muscle memory across
  groves. (Cross-listed in [`message-row.md`](message-row.md) because
  the list drives both row-level containers.)
- **Reactions on deleted messages.** Allowed in v1. Confirm, or lock
  reactions post-delete.
