# Foundation — tokens, typography, icons, motion, voice

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** none (this is the root)
**Consumed by:** every other child spec

Foundation defines the raw material of the Willow UI: CSS variables, type
scale, icon style, motion timings, density modes, accent variants,
terminology, copy voice, and the accessibility baseline. Every child spec
builds on the tokens and rules below.

If a later child spec needs a token not listed here, the foundation spec is
updated first; children do not define their own colour or typography tokens.

## CSS variable registry

All tokens ship on `:root` and are overridable by `[data-theme="..."]`. Light
theme is deferred — willow's v1 palette is dark-only, but the tokens are
named so a light theme can layer in later without renaming anything.

### Background (deep bark to lit panel)

| Variable     | Value     | Use |
|--------------|-----------|-----|
| `--bg-0`     | `#14130f` | Page background; deepest. Body uses this under radial gradients. |
| `--bg-1`     | `#1b1a15` | Primary panel (sidebar, thread pane, popovers). |
| `--bg-2`     | `#22211b` | Raised surface (cards, code blocks, input fields). |
| `--bg-3`     | `#2a2822` | Hover state for `--bg-1` surfaces. |
| `--bg-4`     | `#34322a` | Active / pressed state. |

### Line (separators, borders)

| Variable       | Value     | Use |
|----------------|-----------|-----|
| `--line`       | `#34322a` | Default border (panels, inputs, cards). 1 px. |
| `--line-soft`  | `#22211b` | Low-contrast separator (between groups inside the same panel). |

### Ink (text contrast ladder)

| Variable  | Value     | Use |
|-----------|-----------|-----|
| `--ink-0` | `#f1ede2` | Highest contrast: display headings, message bodies on focused read. |
| `--ink-1` | `#d9d3c2` | Body text default. |
| `--ink-2` | `#a8a290` | Secondary / meta (timestamps, author handles, tag labels). |
| `--ink-3` | `#7a7463` | Muted / hint (placeholders, descriptive labels). |
| `--ink-4` | `#504b3f` | Disabled / dividers that want to still read as text. |
| `--ink-on-accent` | `#14130f` | Foreground ink on filled accent / warn / err buttons. Replaces ad-hoc `#14130f` references. |

### Accent ladder (moss, default)

| Variable   | Value     | Use |
|------------|-----------|-----|
| `--moss-0` | `#2a3a28` | Deep accent surface (accent-tinted panel backgrounds). |
| `--moss-1` | `#425c3d` | Selection highlight, active pill backgrounds. |
| `--moss-2` | `#6a8d5e` | Primary interactive (buttons, unread badges). |
| `--moss-3` | `#93b582` | Hover / focus variant of primary. |
| `--moss-4` | `#c3d8b2` | Foreground on deep accent (icons on accent fill). |
| `--willow` | `#b8c67a` | Slightly yellower green used for wordmark, presence accents, own avatar tint. |

### Bark (amber warmth)

| Variable       | Value     | Use |
|----------------|-----------|-----|
| `--amber`      | `#c99b55` | Warm accent (queued states, warnings that aren't errors). |
| `--amber-soft` | `#7a5a2e` | Deep amber for borders and tag backgrounds. |

### Whisper (private side-channel)

| Variable    | Value     | Use |
|-------------|-----------|-----|
| `--whisper` | `#a88fc9` | Violet — *only* for whisper mode surfaces (pill, message background, presence dot). Not a general accent. |

### Semantic

| Variable | Value     | Use |
|----------|-----------|-----|
| `--ok`   | `#8fb36a` | Success. |
| `--warn` | `#d6a54a` | Warning (expiring ephemeral, unverified intent). |
| `--err`  | `#c97a5a` | Error, destructive confirmation. |

### Radius

| Variable      | Value | Use |
|---------------|-------|-----|
| `--radius-s`  | `6px` | Tags, small pills, inline chips. |
| `--radius`    | `10px`| Panels, cards, inputs. |
| `--radius-l`  | `16px`| Large surfaces, bottom sheets, full-card popovers. |

### Shadow

| Variable     | Value | Use |
|--------------|-------|-----|
| `--shadow-1` | `0 1px 0 rgba(255,255,255,0.02) inset, 0 1px 2px rgba(0,0,0,0.4)` | Subtle surface lift (cards, composer). |
| `--shadow-2` | `0 20px 50px -20px rgba(0,0,0,0.8), 0 4px 12px rgba(0,0,0,0.4)` | Popovers, sheets, modals. |

### Focus

| Variable       | Value | Use |
|----------------|-------|-----|
| `--focus-ring` | `0 0 0 2px var(--moss-1), 0 0 0 3px rgba(106, 141, 94, 0.6)` | Required on every focusable element. |

### Typography stacks

| Variable          | Value | Use |
|-------------------|-------|-----|
| `--font-display`  | `'Fraunces', Georgia, serif` | Display headings, channel titles, italic callouts, dialog headers. |
| `--font-ui`       | `'IBM Plex Sans', system-ui, sans-serif` | Default body, UI chrome. |
| `--font-mono`     | `'JetBrains Mono', ui-monospace, monospace` | Fingerprints, code blocks, crypto artefacts, timestamps when precision matters. |

## Body background

The root body fills `--bg-0` with two softly offset radial gradients so the
surface never reads flat:

```css
body {
  background:
    radial-gradient(1200px 600px at 10% -10%, rgba(106,141,94,0.07), transparent 60%),
    radial-gradient(900px 500px at 110% 110%, rgba(201,155,85,0.05), transparent 60%),
    var(--bg-0);
}
```

Mobile uses a slightly deeper base (`#0c0b08`) to compensate for the smaller
surface area and different ambient lighting assumptions. Gradients scale
wider on mobile (`1400px 700px at 20% 0%`).

## Accent variants

Accent is user-controllable via the Tweaks panel. Each variant rewrites the
`--moss-*` and `--willow` tokens only; other tokens stay fixed so the
overall aesthetic stays coherent across variants.

| Variant  | `--willow` | `--moss-2` | `--moss-3` | `--moss-4` | `--moss-1` | `--moss-0` |
|----------|------------|------------|------------|------------|------------|------------|
| `moss`   | `#b8c67a`  | `#6a8d5e`  | `#93b582`  | `#c3d8b2`  | `#425c3d`  | `#2a3a28`  |
| `willow` | `#b8c67a`  | `#a7b86a`  | `#c1d18a`  | `#e4ecb8`  | `#5a663a`  | `#353b22`  |
| `amber`  | `#c99b55`  | `#c99b55`  | `#e0b57a`  | `#f2d8a8`  | `#7a5a2e`  | `#3a2c18`  |
| `dusk`   | `#c2adda`  | `#a88fc9`  | `#c2adda`  | `#e0d4ef`  | `#5a4a72`  | `#2d2438`  |
| `cedar`  | `#d0a878`  | `#a47848`  | `#c99b55`  | `#e6c08a`  | `#5e3e20`  | `#2e1e10`  |
| `lichen` | `#89b6a5`  | `#5a9580`  | `#86c1ae`  | `#b8dccc`  | `#2e5b4c`  | `#18302a`  |
| `ember`  | `#d48a6a`  | `#b5644a`  | `#d98a68`  | `#f2b89e`  | `#6e3722`  | `#3a1a10`  |

*Whisper violet (`--whisper`) is not accent-swappable.* Whisper must always
be unmistakable regardless of accent setting.

Default accent is `moss`. Accent change is immediate (CSS-variable swap, no
reload), and persisted per-device.

## Typography scale

Each child spec selects from this scale; no ad-hoc font sizes.

| Role           | Family         | Size / weight / style |
|----------------|----------------|-----------------------|
| display XL     | Fraunces       | 54 px / 400 / italic — onboarding hero only |
| display L      | Fraunces       | 34 px / 400 / italic |
| display M      | Fraunces       | 22 px / 500 — dialog headers |
| display S      | Fraunces       | 17 px / 500 / italic — channel title in header |
| body L         | IBM Plex Sans  | 14.5 px / 400 |
| body (default) | IBM Plex Sans  | 14 px / 400 |
| body S         | IBM Plex Sans  | 13 px / 400 |
| meta           | IBM Plex Sans  | 11.5 px / 500 / uppercase (tracked +1.2) — section labels |
| hint           | IBM Plex Sans  | 10.5 px / 400 — tooltips, muted meta |
| mono L         | JetBrains Mono | 14 px / 500 — SAS grid words |
| mono M         | JetBrains Mono | 12 px / 400 — code in messages |
| mono S         | JetBrains Mono | 10.5 px / 400 — inline fingerprint short form |

Line-height is 1.45 for body, 1.25 for display, 1.3 for mono. Italic is the
Fraunces signature — use it deliberately (channel titles, day separators,
dialog headers, poetic callouts). Never italicize body.

## Iconography

- Source: custom SVG set, not an external library.
- viewBox: `24 24`.
- Stroke: 1.5 px default, 1.6 px for sidebar icons (slight optical weight
  compensation). Use `round` linecap and linejoin everywhere. Stroke uses
  `currentColor`.
- Fill: none by default; filled variants exist for Pin, Check, StatusDot.
- Baseline size: 14 px (inline), 17 px (header), 13 px (chips), 11 px
  (meta). The `.icon-sm / -md / -lg` classes from the current client are
  kept: `sm = 14`, `md = 18`, `lg = 22`.
- Naming: lowercase verbs or nouns, no library prefix. `tree`, `inbox`,
  `phone`, `shield`, `users`, `dashboard`, `compass`, `key`, `settings`,
  `hash`, `volume`, `hourglass`, `lock`, `fingerprint`, `ear`, `leaf`,
  `chevron`, `check`, `x`, `plus`, `search`, `signal`, `ghost`, `device`,
  `thread`, `pin`, `send`, `smile`, `paperclip`, `menu`, `more-horizontal`,
  `mic`, `mic-off`, `headphones`, `headphones-off`, `phone-off`, `video`,
  `video-off`, `monitor`, `grid`, `maximize`, `server`, `database`,
  `refresh`, `activity`, `sun`, `moon`, `reply`, `edit`, `trash`, `copy`.
- Direction-sensitive icons (chevron, arrow) ship one canonical direction
  and are flipped via `transform: rotate(...)` in CSS.

## Motion

- Primary easing: `cubic-bezier(0.2, 0.8, 0.2, 1)` — "ease-out willow".
- Durations:
  - `--motion-fast`: 120 ms — hover, focus, button press feedback.
  - `--motion`: 180 ms — popover open/close, tab change, theme swap.
  - `--motion-slow`: 240 ms — drawer slide, bottom sheet, screen push on
    mobile.
  - `--motion-ambient`: 1200 ms — `willowPulse`, presence dots, voice
    listeners.
- Named keyframes shipped in `foundation.css`:
  - `willowPulse`: opacity 0.3 ↔ 1, scale 0.8 ↔ 1.3, infinite.
  - `willow-pop-in`: opacity 0 → 1, translateY(-4 → 0).
  - `leafFall`: used only in ephemeral expiration warnings and onboarding.
    translateY(-12 vh → 120 vh), rotate(-8 → 24), opacity 0 → 1 → 0.
  - `shimmer`: skeleton loading only; background-position -200 → 200.
- `prefers-reduced-motion: reduce` collapses *every* animation to opacity
  only. Transforms are removed; `willowPulse` becomes static opacity.

## Density

Three modes, shipped as CSS class on `#app-root`:

| Mode     | `--msg-pad`  | Use |
|----------|--------------|-----|
| `cozy`   | `10px 24px`  | Fewer messages visible; more whitespace. |
| `balanced` (default) | `8px 24px`  | Default. |
| `dense`  | `4px 24px`   | Power-user screens, long-running grove catch-up. |

Density changes affect message padding only. Sidebar, header, pane widths
stay constant across modes.

## Backgrounds + panels

- Main chat pane: `--bg-0`. No panel border on the left; the channel sidebar
  provides the visual seam.
- Sidebar: `--bg-1` with `--line-soft` right border.
- Grove rail: `--bg-0` with `--line-soft` right border; no left border.
- Thread pane / members pane: `--bg-1` with `--line-soft` left border.
- Popovers / sheets: `--bg-1` with `--line` border and `--shadow-2`.
- Cards inside panels (thread parent card, file card, code block): `--bg-2`
  with `--line` border.

## Scrollbars

```css
.scroll { overflow-y: auto; scrollbar-width: thin; scrollbar-color: var(--bg-3) transparent; }
.scroll::-webkit-scrollbar { width: 8px; }       /* desktop */
.scroll::-webkit-scrollbar-thumb { background: var(--bg-3); border-radius: 4px; }
.scroll::-webkit-scrollbar-track { background: transparent; }
```

Mobile variant uses `width: 6px`. Scrollbars are always visible but quiet —
no auto-hide on desktop; hidden by default on mobile with `.noscroll` class
available for sheets that don't want one.

## Selection

```css
::selection { background: var(--moss-1); color: var(--ink-0); }
```

## Focus

All focusable elements use `--focus-ring` via `:focus-visible`. The ring is
drawn as a box-shadow so it respects rounded corners. Example:

```css
button:focus-visible,
input:focus-visible,
textarea:focus-visible { outline: none; box-shadow: var(--focus-ring); }
```

Never suppress focus on interactive elements. Composer and message bodies
do not trap focus.

## Copy voice

1. **Lowercase first letters** in chrome labels (tabs, buttons, tooltips,
   section headers). Proper nouns (grove names, people names) stay cased.
   Start of a *sentence inside a paragraph* is cased normally.
2. **Nature metaphor is intentional.** "Grove", "letter", "whisper",
   "sealed", "queued". Do not break the metaphor for a single affordance.
3. **Security copy teaches.** Don't just label the control; explain the
   property it protects. Example:
   > compare six words on two screens. if they match, no one can impersonate
   > either of you in this conversation, ever.
4. **Offline is patient, never broken.** "queued", "waiting for peers",
   "will send on reconnect" — never "failed", "disconnected", "error".
5. **Empty states have a voice.**
   > no letters yet — send the first
   > this grove is quiet. say hi?
   > no pinned fragments. pin something you want to keep.
6. **Time is soft.** "this morning", "yesterday", "3d", "1w". Exact
   timestamps appear only on hover / tap and in the mono column.
7. **Avoid corporate verbs.** "delete" / "leave" are fine; "remove from
   community" / "abandon server" are not. "invite" stays "invite".
   "letter of introduction" is the *noun*; the CTA is "invite".
8. **Exclamation marks: none.** Emphasis is from display italic, not
   punctuation.
9. **Grove, not server.** The user-visible term is always `grove`. The
   channel sidebar header tagline `not a server — held between us` is a
   load-bearing reinforcement of the p2p model — keep it when space allows.

## Terminology full map

See parent (`README.md`) for the top-level glossary. Child specs may *add*
terms but may not redefine existing ones.

## States (empty, loading, error, skeleton)

Every surface will render four extra states beyond "populated". Child
specs own their copy, but the visual pattern and the contract live here
so drift can't set in.

### Empty

For collections with no items. Use an italic Fraunces primary line and
a small muted Plex Sans follow-up. Never show an icon for an empty
state unless it is the only way to convey the concept (the sync-queue
welcome banner is one of the few licensed exceptions).

Shape:
```
{italic Fraunces headline, --ink-1, 17 px}
{Plex Sans hint, --ink-3, 13 px, max one line}
{optional primary action, only if the user has one clear next step}
```

Examples (owning spec):
- channel with no messages → "this channel is quiet. say hi?" (message-row.md)
- no letters → "no letters yet — send the first" (letters-dms.md)
- no pinned fragment → "no pinned fragment" (profile-card.md)

### Loading

- **Content loading (skeleton)** — use the `shimmer` keyframe defined
  under §Motion. Skeleton blocks are `--bg-2` with a 200 ms delayed
  start so that fast networks never flash. Max three rows of skeleton
  per surface — don't mimic dozens of rows.
- **Inline / streamed** — show a short mono italic `loading…` in
  `--ink-3` when replacing the whole surface would be disruptive.
- **Indeterminate progress** — a thin 2 px bar in `--moss-2` at the top
  of the surface, sliding from left to right with `willowPulse`-style
  opacity (not a spinner).

Reduced-motion path: static `--bg-2` blocks; no shimmer, no slide.

### Error

- Use the `--err` token for both fill and icon. Icon is always present
  (colour is never the only signifier).
- Structure: a small card or toast, never a full-pane error unless the
  whole surface is unreachable.
- Copy rules (carry forward the copy-voice §):
  - Never punish; never imply the user broke anything.
  - Offer the recovery action inline, not in a dialog.
  - "couldn't reach peer · will retry" beats "Error 502".
- Unreachable surface pattern (worst case):
  ```
  {italic Fraunces: "this room is quiet right now"}
  {Plex Sans --ink-3: "we can't reach peers for this grove. willow will keep trying."}
  {button: "retry"}  {button-link: "report"}
  ```

### Skeleton vs placeholder

"Skeleton" means a structural preview of the same layout about to
render. "Placeholder" means a neutral copy hint (usually italic, no
shimmer) shown while the user *has nothing yet*. Use skeleton for
"loading what exists"; use placeholder for "populate this to see
content".

### Per-state acceptance

Each child spec ships all four states before it can move out of draft:
- empty (listed)
- loading (skeleton + reduced-motion path)
- error (token, copy, recovery action)
- skeleton (or a note "not applicable — this surface has no list")

Children do not need to restate the above shapes; they only provide
their own copy.

## Accessibility baseline

- Contrast ratios meet WCAG AA on default theme. All body text combinations
  (ink on bg) are verified ≥ 4.5:1. Meta (`--ink-2` on `--bg-0`) is 4.65:1.
- Colour is never the only signifier. Every accent-tinted state pairs with
  an icon or shape cue (whisper = violet + ear; queued = amber + hourglass;
  unverified = amber + dashed ring; verified = moss + filled check).
- Focus-visible is required on every interactive element.
- Touch targets on mobile are ≥ 44 × 44 CSS px. Small icons are wrapped in
  larger hit boxes.
- Screen-reader labels are required on icon-only buttons. Each child spec
  lists its labels in a "Labels" section.
- Keyboard path exists for every interaction. Long-press on mobile has a
  keyboard equivalent (Enter on focused element, or context-menu key).
- Motion respects `prefers-reduced-motion: reduce`.

## Implementation notes

- Tokens ship in a single `foundation.css` that is imported first. Child
  component stylesheets reference tokens only; they never redefine values.
- Theme variants (accent swap) apply by writing to `documentElement.style`
  for the `--moss-*` and `--willow` custom properties only. Never touch
  `--ink-*`, `--bg-*`, or `--line*` at runtime.
- The Tweaks panel (`settings-tweaks.md`) is the only surface that writes
  tokens at runtime.
- Density classes (`density-cozy`, `density-balanced`, `density-dense`)
  apply to `#app-root` and swap `--msg-pad`.
- All font faces are loaded from Google Fonts with `display=swap`. Self-
  hosting is a future optimization and does not change the spec.

## Acceptance

- [ ] Every CSS variable above is defined in `foundation.css`.
- [ ] Every accent variant renders correctly (sampled visually).
- [ ] Reduced motion collapses all animations.
- [ ] Focus ring visible on all interactive elements under keyboard nav.
- [ ] Copy lint (future): detect pejorative offline strings, uppercase
      chrome labels, and stray exclamation marks.

## Open questions

- Should light theme ship in v1? Current answer: no, deferred. Tokens are
  named to allow it later.
- Self-hosted fonts (FOIT risk on slow connections)? Deferred.
- Per-grove accent override? The design bundle supports grove-level accent
  via `GROVES[].accent`. If we enable this, grove accent only applies to
  the grove's own surfaces (its rail tile, its header underline), not to
  the whole app.
