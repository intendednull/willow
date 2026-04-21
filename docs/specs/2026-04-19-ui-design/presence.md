# Presence — the peer status atom

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md)
**Consumed by:** [`profile-card.md`](profile-card.md), [`letters-dms.md`](letters-dms.md), [`layout-primitives.md`](layout-primitives.md) (member rail + me-strip), [`whisper-mode.md`](whisper-mode.md), [`sync-queue.md`](sync-queue.md), [`call-experience.md`](call-experience.md)

## Purpose

Presence is the shared atom for rendering a peer's current status —
are they here, are they away, are they whispering, are they queued
for you, did they disappear. Before this spec, presence rendering
was scattered across `profile-card.md` (status pill),
`letters-dms.md` (row dot), `layout-primitives.md` (member rail +
me-strip), `whisper-mode.md` (violet dot), and `sync-queue.md`
(amber pill), each reaching for its own tokens and copy. This spec
consolidates those into one canonical state catalog and two atoms —
`StatusDot` and `PeerStatusLabel` — that every consuming surface
composes.

Presence is a *display primitive*. It does not own its data
sources (reachability from `willow-network`, whisper and call
sessions, sync-queue depth, and a new `PresenceHeartbeat` event)
and it does not own the controls that change self-presence (the
me-strip in `layout-primitives.md` and the menu in
`settings-tweaks.md`). It owns the *single authoritative mapping*
from `(reachable, idle, whispering, in-call, queue-depth,
invisible)` to a dot shape + colour + label + icon + accessible
name. A peer's presence is derived; it is never invented by a
surface. When two surfaces disagree about a peer's status, this
spec wins.

## Scope

**In scope.** The seven-state catalog (ids, labels, colours, shapes,
triggers); the `StatusDot` atom (filled / ring / pill variants); the
`PeerStatusLabel` atom; the ownership map that tells each consuming
spec which composition to render; self-presence controls (auto vs
manual, mutual-hide invisibility); transitions (heartbeat cadence,
idle / gone thresholds, reduced-motion behaviour); exact copy and
aria labels; data dependencies flagged existing vs new; collision
edge cases (whisper + call; queued both ways; clock skew; invisibility
mutuality).

**Out of scope.** Event-bus plumbing for heartbeat propagation
(implementation plan); the containing surface layouts (profile card,
letter row, member rail, call tile) — this spec supplies the atom
slot only; whisper activation UX and call join flows; the settings-
tweaks form that edits thresholds (values declared here, UI in
`settings-tweaks.md`); block-list interactions (blocked peers render
presence per normal rules).

## State catalog

Every peer the local client sees is in exactly one of these seven
states at any moment. Authoritative mapping; the first column is the
internal id, the second is the copy that reaches the user.

| id | label (user-visible) | dot colour | shape | icon prefix | who sets it |
|----|----------------------|------------|-------|-------------|-------------|
| `here` | `here` | `--moss-2` | filled circle | — | auto: peer reachable + recent heartbeat |
| `away` | `away` | `--ink-3` | filled circle | — | auto: no heartbeat for ≥ *idle threshold* (default 6 min; tunable) |
| `whispering` | `whispering` | `--whisper` | filled circle | `ear` | auto: peer is in an active whisper session (any whisper, not only with you) |
| `in a call` | `in a call` | `--moss-2` | **ring** (2 px stroke, no fill) | — | auto: peer is in a grove or letter voice call and *not* in whisper |
| `queued · N` | `queued · {n}` | `--amber` | filled circle + `--amber` pill wrap | `hourglass` | auto: peer unreachable AND this device has `n ≥ 1` messages queued for them |
| `gone` | `gone` | `--ink-4` | filled circle | — | auto: peer unreachable for ≥ *gone threshold* (default 48 h; tunable) |
| `invisible` | (hidden; no dot) | — | not rendered | — | manual, mutual: the peer has set themselves invisible to this client, or this client has set itself invisible to them |

Precedence (highest → lowest) when two triggers fire at once:
`invisible` > `whispering` > `in a call` > `queued · N` > `gone` >
`away` > `here`. Invisibility always wins (both sides vanish).
Whispering dominates call state. Queued only appears when unreachable;
if the peer comes back with queue > 0, the pill drains and the label
snaps to `here`. Gone applies only after the unreachable window
crosses the threshold.

The precedence table is load-bearing: consuming surfaces never
implement their own merge order. Compute once in the atom, render
everywhere.

## StatusDot atom

`StatusDot` is the small glyph placed at the bottom-right of an
avatar. Three shape variants; sizing depends on context. Every
variant renders the same 2 px "knock-out" border in the colour of the
immediate containing surface so the dot reads cleanly against both
panel and main backgrounds.

### Variants

| Variant | Description | Used for |
|---------|-------------|----------|
| `filled` | Solid circle; colour from the state catalog. | `here`, `away`, `whispering`, `queued · N`, `gone`. |
| `ring`   | 2 px stroke ring, no fill; 2 px inner radius retained. Colour `--moss-2`. | `in a call`. |
| `pill`   | Wraps the dot and the label text in a rounded `--amber` / `--whisper` tint capsule. | Inline letter-row chip for `queued · N` (amber pill) and whisper-marked letters where the row has no avatar focus. Never on avatars. |

### Sizing

`StatusDot` sits at the bottom-right corner of an avatar. Size
scales with avatar size and platform.

| Context | Avatar | Dot | Border | Border colour |
|---------|--------|-----|--------|---------------|
| profile-card banner (desktop / mobile) | 64 / 84 | 13 / 16 | 3 | `--bg-1` |
| letters-dms row (desktop / mobile) | 30 / 38 | 9 / 10 | 2 | `--bg-1` |
| member rail | 30 | 10 | 2 | `--bg-1` |
| me-strip footer | 26 | 8 | 2 | `--bg-1` |
| message-row author avatar | 36 | 9 | 2 | `--bg-0` |
| call participant tile | 72 | 14 | 2 | `--bg-0` |

Grove-rail tiles never carry a dot — groves are not peers.

The border colour is the token of the surface the dot sits on, not
a fixed value: panels (`--bg-1`) get a `--bg-1` border; main-pane
surfaces (`--bg-0`) get a `--bg-0` border. This keeps the dot
visually detached regardless of placement.

### Animation

`here` dots may render with `willowPulse` (opacity 0.7 ↔ 1.0, scale
0.95 ↔ 1.05, period 1200 ms) in list surfaces that want an ambient
"alive" cue — letters-dms rows and the member rail use it; the
profile card banner does not (it's a stopping surface). Pulse is
never applied to `away`, `gone`, or `invisible`. `whispering` uses a
tighter pulse (opacity 0.85 ↔ 1.0). Reduced motion collapses all
pulses to steady opacity per `foundation.md`.

## PeerStatusLabel atom

`PeerStatusLabel` is the short text rendering that accompanies the
dot when text is wanted. Most surfaces show it inline; the member
rail hides it in the row and exposes it via tooltip only.

**Composition.** `Label ::= [icon] [dot] <text>`

- `icon` — optional, 11 px, `currentColor` stroke. `whispering`
  uses ear; `queued · N` uses hourglass. Omitted for `here`,
  `away`, `in a call`, `gone`.
- `dot` — a small `StatusDot` at 8 px (filled) or the `ring`
  variant for `in a call`. Omitted when the label already renders
  next to an avatar dot (no double-render).
- `text` — the user-visible label from the state catalog.

**Typography.** Label text: `--font-ui`, body S (13 px) / 400,
`--ink-2` default. On the profile card status pill the text sits
on `--bg-2` with `--line-soft` border. The `N` in `queued · N` is
`--font-mono` 12 px `--amber`, echoing the sync-queue's mono-
timestamp grammar.

### When to render

| Surface | Render label? |
|---------|----------------|
| profile card | always (next to avatar, below display name) |
| letters-dms row | only if state ≠ `here` (visual quiet when present) |
| member rail | no inline text; tooltip on hover / focus reveals the label |
| me-strip footer | always (self label with manual-override menu) |
| call tile | tooltip only — avatar dot carries the state visually |
| thread pane header (1:1) | inline, right of handle, when state ≠ `here` |

Surfaces that show a trust badge alongside presence render
`PeerStatusLabel` after the badge — trust reads left, presence
reads right. Trust is *who*; presence is *when*.

## Ownership map

Each consuming surface delegates to this spec for the atom's render.
The table states the composition; the surface spec supplies its
container, padding, and interactions.

| Surface | Composition |
|---------|-------------|
| `profile-card.md` | avatar + `StatusDot` (13 / 16 px) + `PeerStatusLabel` — card status pill below the avatar block, replacing the bespoke mapping the profile-card spec currently declares. |
| `letters-dms.md` row | avatar + `StatusDot` (9 / 10 px) + `PeerStatusLabel` *iff state ≠ here*. Queued pill consumes the inline chip when rendered — no double render. |
| `layout-primitives.md` member rail | avatar + `StatusDot` (10 px) + tooltip `PeerStatusLabel`. Tooltip uses foundation tooltip styling; no visible row text. |
| `layout-primitives.md` me-strip | avatar + `StatusDot` (8 px) + `PeerStatusLabel` (self) + override menu. See *Self-presence*. |
| `whisper-mode.md` | derives `whispering` from the active-whisper signal; renders no independent dot. Whisper pill in the call header remains whisper-mode's concern; the peer-side dot is this spec's. |
| `sync-queue.md` | derives `queued · N` from per-peer queue depth; emits no independent dot. The status strip and per-peer pills are sync-queue's concern; the atom inside them comes from here. |
| `call-experience.md` tile | `StatusDot` `ring` when peer is on call but not whispering; `filled` `--whisper` if also whispering (whisper wins). |
| `message-row.md` author avatar | `StatusDot` (9 px, filled only). Live refresh on presence updates; hover tooltip shows the label. |

Surfaces never render a dot without consulting this atom and never
invent additional states ("just left", "do not disturb"). New states
are added to the catalog via a foundation-level update first.

## Self-presence + manual override

The local user is a special peer: you always see your own status,
you can override it, and what you override propagates — with the
exception of `invisible`, which is a signalled mutual hide.

**Default.** Self-presence is `auto`. The auto-derived state is
computed identically to the peer view from this client's own
heartbeat + whisper / call activity + queue depth (self-queue is
always 0 user-facing — you don't queue to yourself).

**Manual toggles.** The me-strip footer status label opens a menu
with exactly these entries, in order:

| Toggle | What it does |
|--------|--------------|
| `auto` | Return to auto-derived state. Default. |
| `away` | Force `away` regardless of heartbeat / activity. |
| `gone` | Force `gone`. Peers see you as long-absent even if reachable. |
| `invisible` | Stop heartbeats + emit invisibility signal. Mutual: peers you're invisible *to* are invisible *to you*. |

There is no manual `here`. "Here" is the absence of override; when
you come back to your keyboard, auto-here resumes.

Manual overrides are stored locally (per device) and do not gossip
— with one exception: `invisible` emits a signed hint that peers
use to suppress your presence in their render. Peers trust it as
they trust any other self-authored profile update. Invisibility is
best-effort; peers that haven't received the hint yet may still
show your last-known status until heartbeats lapse into `gone`.

**Me-strip rendering.** 26 px avatar + 8 px `StatusDot` (border
`--bg-1`) + `PeerStatusLabel` + chevron for the menu. In
`invisible` mode the dot still renders locally (so the user
remembers) and the label reads `invisible` with the mutual-hide
tooltip.

## Transitions + thresholds

Transitions are derived, not scheduled. The client listens to four
input streams:

- `PresenceHeartbeat` events from peers (new event — see *Data
  dependencies*).
- `willow-network` reachability state (per-peer; existing).
- Whisper session registry (per `whisper-mode.md`).
- Sync-queue depth (per `sync-queue.md`).

### Heartbeat cadence

Peers emit a `PresenceHeartbeat` every 60 s while online. A missed
heartbeat alone does not flip the state — the idle threshold
compares against the last heartbeat's HLC timestamp. Heartbeats are
signed and may carry an optional intent hint (`active` / `idle`).
When the hint is present the client trusts it; otherwise it falls
back to time-since-last-heartbeat.

### Thresholds

| Threshold | Default | Override | Applied to |
|-----------|---------|----------|------------|
| `idle` (here → away) | 6 minutes since last heartbeat | `settings-tweaks.md` (open question: should this be per-grove or global? See *Open questions*) | All peers, including self when auto |
| `gone` (unreachable → gone) | 48 hours without reachability | `settings-tweaks.md` | Peers only; self is never auto-`gone` |
| `queued-decay` (queued · N → here after reconnect) | as queue drains; no timer | n/a | Per-peer |

Thresholds are compared in HLC-time, not wall-clock, so a client with
a skewed clock does not flip every peer to `gone` on startup.

### Auto transitions

| From → To | Trigger |
|-----------|---------|
| `here → away` | No heartbeat for `idle` threshold, or peer hint flips to `idle`. |
| `away → here` | Fresh heartbeat with `active` hint (or no hint + activity marker). |
| `here ↔ whispering` | Whisper session starts / ends for this peer. |
| `here ↔ in a call` | Call session starts / ends (no whisper). |
| `in a call ↔ whispering` | Whisper overlaps an active call; resolves to whichever is currently active (whisper wins concurrent). |
| `* → queued · N` | Peer unreachable AND local queue depth ≥ 1. |
| `queued · N → here` | Peer reachable AND queue drains to 0. |
| `queued · N → gone` | Unreachable window exceeds `gone` threshold (queue depth does not reset it). |
| `here / away / in a call / whispering → gone` | Unreachable for `gone` threshold. |
| `any ↔ invisible` | Peer emits / revokes invisibility signal; local recomputes from last-known signals on revoke. |

### Motion

Dot colour changes cross-fade 180 ms (`--motion`). Label text swaps
fade-out → fade-in 180 ms inside a shared container so the dot does
not flicker. The `away` auto-transition does **not** animate under
`prefers-reduced-motion: reduce` — the colour snaps and pulse is
disabled. `queued · N` increments animate via 120 ms `willow-pop-in`;
reduced motion collapses to an instant swap.

### Self auto resumption

Manual overrides are sticky. Re-engaging with the client (keystroke,
scroll, tap, focus) does *not* flip back to `auto` automatically —
only explicit selection of `auto` does. A browser close / reopen
does reset the session to `auto`, so a user cannot accidentally stay
`gone` for weeks.

## Copy (exact)

All lowercase. No exclamation marks. Proper nouns (peer names) stay
as authored. These strings are authoritative; other specs must
import or mirror them.

**Labels.** `here` · `away` · `whispering` · `in a call` ·
`queued · {n}` (`99+` above 99) · `gone` · `invisible`
(self-only; peers never see this label).

**Manual-toggle menu.**

```
PRESENCE_MENU { auto: "auto", away: "away", gone: "gone", invisible: "invisible" }
```

### Tooltips

| Context | Tooltip |
|---------|---------|
| `gone` (hover / long-press) | `last seen {soft-time}` |
| `invisible` (self hover) | `you're hidden from peers — they're hidden from you` |
| `queued · N` (hover) | `{n} queued · will send when they reach you` |
| `whispering` (hover on peer dot) | `whispering` (not "whispering with …" — see `whisper-mode.md` privacy rule) |
| `in a call` (hover) | `in a call` |
| `away` (hover) | `away for {soft-time}` |

The `last seen {soft-time}` and `away for {soft-time}` values use
the foundation soft-time grammar: `a few minutes`, `this morning`,
`yesterday`, `3d`, `1w`, or `a while`. Never an exact clock time.

**Aria labels (icon-only contexts).** StatusDot without visible
text: `status: {label}`. Ring variant on call tile: `in a call`.
Pill variant with queued count: `{n} queued for you`. Me-strip menu
trigger: `change your status · currently {label}`.

## Data dependencies

Flagged **existing** (reuses current state / events / APIs) or **new**
(requires a new type, event kind, or API).

| Dependency | Status | Owner | Notes |
|------------|--------|-------|-------|
| Peer reachability | existing | `willow-network` | iroh connection state + gossip visibility. |
| Queue depth per peer | existing | `willow-client` / `sync-queue.md` | Already surfaced. |
| Whisper session state | existing | `whisper-mode.md` | "Peer is in ≥ 1 whisper session you know about." |
| Call session state | existing | `call-experience.md` | Peer participating in a voice call. |
| **`PresenceHeartbeat` event** | **new** | `willow-state` | New `EventKind`. `{ peer, hlc, hint: Option<{active|idle}> }`. Every 60 s while active or on state change. Self-authored. Latest per peer retained in materialized state; earlier ones evicted. |
| **Invisible signal** | **new** | `willow-identity` + `willow-state` | Signed "invisible-to-peers" hint — a new `EventKind` (`SetInvisible(bool)`) or a field on `SetProfile`. Mutual: receiving suppresses the peer's presence locally. |
| **Idle threshold (tunable)** | **new** | `settings-tweaks.md` | Per-device. Default 6 min. Range 2–30 min. |
| **Gone threshold (tunable)** | **new** | `settings-tweaks.md` | Per-device. Default 48 h. Range 1 h – 7 d. |
| Soft-time formatter | existing | `willow-web` utility | Shared with profile-card meta and letters-dms timestamps. |

Thresholds are declared here and edited in the Tweaks panel;
`settings-tweaks.md` owns the form UI and should reference this
spec for defaults and ranges.

### Heartbeat churn and event-log bloat

Naively persisted, heartbeats would swamp the event log (one per
minute per peer). The implementation plan must keep only the latest
heartbeat per peer in materialized state — not the full DAG — or
route heartbeats over a separate ephemeral gossip topic. User-facing
behaviour is identical either way.

## Edge cases

- **Peer whispering AND on a call with you.** Label resolves to
  `whispering`; dot is `--whisper`. The call-tile *ring* is
  replaced by a filled violet dot — whispering is the stronger
  privacy signal and the rarer state.
- **Peer whispering with a third party AND on a call with you.**
  Same resolution: `whispering` wins. The call UI already
  communicates they're present with you; the whisper state
  communicates their attention is split.
- **Peer queued for you AND queued from you.** Single `queued · N`
  using the outbound count only; tooltip reads
  `{out} from you · {in} for you · will settle on reconnect`.
- **Peer reachable but no heartbeats received yet.** Render `here`
  — reachability alone is a strong signal. `away` only kicks in
  after the first heartbeat lands and then goes stale. Avoids an
  "instantly away on connect" glitch.
- **Peer unreachable but queue is empty.** Render `away` before
  the gone threshold, `gone` after. Never `queued · 0`; the
  queued pill only appears with count ≥ 1.
- **Unreachable → reachable → drained → unreachable again.** The
  state machine handles each transition strictly per the auto
  table; no sticky "was queued" state. Surfaces that want a
  "just delivered" celebration use `sync-queue.md`'s own
  annotation, not this atom.
- **Clock skew between devices.** Heartbeats carry HLC; idle /
  gone thresholds compare HLC-time, not wall-clock. A 3-hour
  clock skew does not misclassify every peer as `gone`.
- **`invisible` is mutual.** If you are invisible to a peer, you
  cannot see *their* presence either — their dot is absent, their
  label empty. The self-tooltip surfaces this:
  `you're hidden from peers — they're hidden from you`.
  Invisibility is not a voyeurism vector. Lists still render the
  peer's row with a default avatar tint; only the dot + label
  are suppressed.
- **Peer was `gone`, suddenly reachable.** Jump straight to `here`
  (or `whispering` / `in a call` if applicable). Standard 180 ms
  cross-fade.
- **Self `gone` by manual override, then re-engages.** Stays
  `gone` (sticky). Must toggle back to `auto`.
- **Peer in a grove I'm not in.** Presence is only computed for
  peers you share at least one grove or letter with; cross-grove
  presence is never exposed (privacy leak vector).
- **Pending-verify trust state.** Trust and presence are
  orthogonal — unverified peers can be `here`, verified peers
  can be `gone`. Badge and dot are independent.
- **Blocked peer.** Presence renders per normal rules. Block
  affects message surfacing, not presence visibility (see Open
  questions).

## Accessibility

- Every `StatusDot` has an `aria-label` derived from the state
  catalog even when no visible text accompanies it. Example:
  `aria-label="status: here"`.
- Colour is never the only signifier: `whispering` has the ear
  icon, `queued · N` has the hourglass icon, `in a call` uses the
  *ring* shape (not a colour change), `gone` and `away` differ by
  the label text.
- The pulse animation on `here` / `whispering` dots collapses to
  steady opacity under `prefers-reduced-motion: reduce`. The idle
  → away transition does not animate under reduced motion — the
  colour snaps.
- Tooltips are keyboard-accessible: focusing the dot (via
  tab-through on surfaces that make it focusable, like the me-
  strip trigger) shows the tooltip; blur hides it. On surfaces
  where the dot is decorative (letter row, author avatar), the
  label is announced as part of the row's composed accessible
  name (see `letters-dms.md` row label composition).
- Screen readers hear state changes as they happen. The me-strip
  menu trigger has `aria-live="polite"` so flipping your own status
  announces as `status changed: gone`.
- Touch targets: the me-strip menu trigger is ≥ 44 × 44 CSS px on
  mobile. The peer dot itself is decorative and need not meet
  touch target minima (it is not independently actionable).
- Sufficient contrast: `--amber` on `--bg-0` (queued pill) and
  `--moss-2` on `--bg-1` (here dot) both meet WCAG AA non-text
  contrast (≥ 3:1). `--whisper` on `--bg-1` verified ≥ 3:1.
  `--ink-4` on `--bg-1` (gone dot) verified ≥ 3:1 against the
  border which provides the visual edge.

## Acceptance criteria

- [ ] Every peer render across the app (profile card, letter row,
      member rail, me-strip, call tile, message author avatar,
      thread header) goes through `StatusDot` and `PeerStatusLabel`.
- [ ] The seven states render with the exact colour / shape / icon
      combinations in the catalog. No surface invents additional
      states.
- [ ] Precedence resolves correctly: whisper wins over call; queued
      only when unreachable; invisibility hides both directions.
- [ ] Manual self-override menu offers exactly `auto`, `away`,
      `gone`, `invisible` — no `here` toggle. Browser close resets
      to `auto`.
- [ ] `PresenceHeartbeat` events emit at 60 s cadence while active
      and on state-change transitions. Materialized state retains
      only the latest per peer.
- [ ] Idle threshold (default 6 min) and gone threshold (default
      48 h) are read from settings-tweaks; out-of-range edits are
      clamped.
- [ ] Thresholds compare HLC-time, not wall-clock — clock skew
      does not misclassify peers.
- [ ] Invisibility propagates via a signed signal and suppresses
      rendering in both directions.
- [ ] All copy strings match exactly (labels, tooltips, menu,
      aria). No exclamation marks, no uppercase chrome.
- [ ] Reduced motion: pulse collapses; away transition snaps;
      queued-count increment collapses to instant swap.
- [ ] Accessibility: aria-label on every dot; icon / shape
      distinguishes colour-duplicate states; me-strip trigger
      ≥ 44 × 44; self state changes announce via
      `aria-live="polite"`.
- [ ] Surfaces with multiple adornments order them trust →
      presence → queued left-to-right.
- [ ] Presence changes cross-fade in place; the surrounding
      layout does not reflow.

## Open questions

- **Idle threshold: per-grove or global?** Default global; a
  per-grove override is conceivable for high-activity chat.
  Deferred to a later Tweaks iteration.
- **Gone threshold: raw duration or preset ladder?** UI choice
  (`an hour` / `a day` / `a week` vs free slider) deferred to
  `settings-tweaks.md`.
- **Heartbeats in the event log or ephemeral gossip?** Ephemeral
  is cleaner (no DAG bloat) but needs a separate transport
  channel. Decision tracked in the implementation plan.
- **Peer-authored intent string** (`"head down — back at 3"`)
  inside the label? Out of scope for v1; catalog is closed.
  Revisit post-review.
- **Blocked peers and presence:** suppress their dot (treat like
  mutual invisibility), or keep showing? v1 keeps showing. Open.
- **Manual `do not disturb` variant** distinct from `away` /
  `invisible`? Deferred; would extend the catalog.
- **Mobile backgrounding:** accelerate the idle timer when the
  app is backgrounded, or apply the normal threshold? v1 keeps
  normal threshold.
- **Cross-device self-presence:** when phone is active but
  laptop idle, which presence do peers see? v1: most-recent
  heartbeat wins, each device emits independently. Multi-device
  reconciliation deferred to `device-handoff.md`.
- **Queue-both-directions tooltip wording:** the
  `{out} from you · {in} for you` phrasing is awkward; review
  with copy voice before freezing.
- **`whispering` tooltip privacy:** the rule that the tooltip
  never names the whisper partner is re-affirmed from
  `whisper-mode.md`. Flagged here so future reviewers do not try
  to "improve" it.
