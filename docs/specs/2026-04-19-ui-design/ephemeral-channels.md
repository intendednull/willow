# Ephemeral channels — non-permanent surfaces, idle-archive

**Parent:** [README.md](README.md)
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md)
**Related:** [`thread-pane.md`](thread-pane.md), [`whisper-mode.md`](whisper-mode.md) (share the auto-archive mechanic defined here)
**Status:** draft

## Purpose

Ephemerals are conversational surfaces that are **not part of a grove's
permanent structure**. They spawn ad-hoc, live for as long as the
conversation lives, and quietly auto-archive when activity dies down.
The archive is recoverable: any participant who returns and posts
revives the surface; an explicit "revive" affordance restores it from
the archives view; nothing is destroyed.

Ephemeral is a kind, not a retention policy. The defining property is
not destruction — it is **non-permanence in the grove's structure**.
Permanent channels live in the sidebar forever. Ephemerals appear when
they exist, fade to "dormant" when idle, slide into archives when the
inactivity threshold passes, and pop back when someone speaks again.

The ethos: low ceremony. A small group splits off for a side chat,
the chat winds down on its own, the sidebar doesn't fill with stale
rooms anyone has to clean up. If the conversation comes back, the
room comes back. No countdowns, no alarms, no destruction event.

## Scope

In scope (this spec):

- The **auto-archive mechanic** for non-permanent surfaces. Used by
  ephemeral channels (canonical), threads (see `thread-pane.md`), and
  whispers (see `whisper-mode.md`).
- Ephemeral channel kind: spawn flows (explicit + ad-hoc), sidebar
  representation, dormant-state styling, archive transition, revive.
- Copy strings, edge-case copy, accessibility.

Out of scope:

- Cryptographic destruction. Ephemerals retain their channel key; the
  archive is data-preserving. (A separate "destroy" affordance for
  permanent channels lives outside this feature; if a user wants real
  finality, that is a different action.)
- Thread + whisper spawn flows in detail — those specs own their UX.
  This spec defines only the auto-archive behaviour they inherit.
- Backend wire format for the auto-archive event (state team owns
  exact shape; this spec lists data dependencies only).

## Concepts

| term | meaning |
|------|---------|
| **permanent channel** | shows in sidebar always, lives until explicitly deleted by a steward |
| **ephemeral surface** | umbrella for non-permanent surfaces: ephemeral channel, thread, whisper |
| **active** | has had recent activity; renders in sidebar |
| **dormant** | no activity for a soft idle window; still in sidebar but de-emphasised |
| **archived** | crossed the auto-archive threshold; hidden from sidebar; reachable via archives surface |
| **revived** | a previously archived ephemeral has new activity; returns to sidebar in the active state |

## Inactivity ladder

A single signal drives every ephemeral surface: time since last
activity. **Activity is a new message only** (or thread reply, or
whisper utterance). The following do **not** count:

- Reactions on existing messages
- Pin / unpin on existing messages
- Typing indicators
- Presence changes
- Membership changes (joins, leaves, kicks)
- Edits to existing messages

The reasoning: ephemeral surfaces archive when conversation stops.
Reactions on a stale thread are residue; they should not artificially
extend the room's life. Only a new utterance counts as "this room is
still being used."

The state machine derives the ladder from the channel's
`last_activity_hlc` against the merge frontier's HLC:

| state    | trigger                          | UX                                                                      |
|----------|----------------------------------|-------------------------------------------------------------------------|
| active   | activity within 25 % of threshold | normal sidebar row                                                      |
| dormant  | activity in 25–100 % of threshold | sidebar row dimmed, time-since chip in `--ink-3`                        |
| archived | exceeds threshold                | row removed from sidebar; appears under archives → "auto-archived" group |
| revived  | new activity after archive       | row reappears in sidebar; archived entry retained as a record           |

Thresholds (defaults; configurable per-grove in `governance.md`):

| surface           | default threshold |
|-------------------|-------------------|
| ephemeral channel | 14 days           |
| thread            | 7 days            |
| whisper           | 24 hours          |

Per-channel override is allowed at creation (cap 90 days). Permanent
channels have no threshold — they never archive automatically.

The transitions are deterministic on every device because they are
HLC-derived. No countdown broadcast is needed.

## Sidebar treatment

### Active

Ephemerals share a sidebar group with permanent channels (no separate
"ephemeral" group — the kind is signalled by the trailing chip, not
group membership). Row anatomy matches a normal channel row plus a
small trailing chip:

- Trailing chip: `--radius-s`, 1 px border `--line-soft`, padding
  `2px 6px`, foreground `--ink-3`, font `--font-mono` 11 px.
- Chip text: a single token from the spec's small lexicon —
  `temp` for ephemeral channels, `thread` for threads, `whisper` for
  whispers. The chip token communicates "non-permanent" without using
  the word "ephemeral" in the UI (which is a metaphor, not user copy).

### Dormant

When the surface enters the dormant phase, the row dims:

- Channel name colour: `--ink-2` (one step muted from `--ink-1`).
- Trailing chip: gains a meta line "*N days ago*" (or `*N hours
  ago*`, `*N minutes ago*`) in `--ink-3`. The chip wraps to two
  lines on desktop sidebars wide enough to allow it.
- No badges, no glow. Dormant is calm, not alarming.

**Mobile compact form.** The mobile sidebar (≤ 640 px viewport) is
too narrow to carry the meta line without pushing rows to two lines
and breaking the channel-list rhythm. On mobile the dormant state
is signalled by:

- Dimmed name colour (`--ink-2`) — same as desktop.
- Kind chip remains (`temp` / `thread` / `whisper`) — no meta line
  appended.
- Long-pressing the row opens the standard row preview overlay
  (existing pattern from `mobile-shell.md`); the preview header
  carries the meta string `"last activity {N} {unit} ago"` so the
  user can still read it on demand.

This keeps the row at single-line height regardless of dormancy and
matches the existing mobile pattern of pushing extra metadata into
long-press surfaces rather than the row itself.

### Archived

Archived ephemerals are removed from their normal sidebar position
and listed inside the archives surface (existing — see `discover.md`
§Archives) under an "auto-archived" subgroup, ordered by
`archived_at` descending. The kind chip remains. A meta line under
the row reads "*archived after N days idle*" using the same humanised
phrasing as the dormant chip.

### Revived

When new activity arrives on an archived surface, the row returns to
the active sidebar in the normal position; the archives entry stays
as a record (so users can see "this channel was archived, then
revived on {date}" if they go looking). No toast, no banner — the
return is silent. The previous archive entry collapses into a small
"revived" badge inside the archives list.

## Spawn flows

Each ephemeral type spawns through its own surface; the auto-archive
behaviour is identical regardless of how the surface came into being.

### Ephemeral channel — explicit creation

Sidebar `+` (desktop) or mobile FAB → "new channel" picker. The kind
selector exposes three options: `text`, `voice`, `temp`. Selecting
`temp` reveals an inactivity-threshold field with the default
populated (14 days) and the cap enforced (90 days).

Steps:

1. **Name.** Standard channel-name input, lower-kebab-case, max 32 ch.
2. **Kind.** Three segmented chips: `text` (permanent), `voice`,
   `temp`. Selecting `temp` reveals:
3. **Idle threshold.** Slider or steppered field, range `1h – 90d`.
   Default 14 days. Helper copy: `"archives if no one posts for {N}.
   anyone can revive it by posting again."`
4. **Confirm.** Primary CTA: `start #{name}`. Hitting confirm creates
   the channel.

If the user lacks `CreateChannel`, the kind selector is disabled with
a hint tooltip (existing pattern).

### Ephemeral channel — ad-hoc spawn

A short-lived ephemeral is also creatable from any peer's profile
card or member list via `start temp channel…`. This path skips the
threshold field (uses default) and seeds the member list with the
user + the targeted peer. Useful for "two of us need a side room
right now" without ceremony.

### Thread

Threads spawn from a message via the existing thread-pane flow
(`thread-pane.md`). Threads inherit the auto-archive mechanic with a
7-day default threshold. The thread row in the side rail carries the
`thread` kind chip plus the same dormant / archived states defined
here.

### Whisper

Whispers spawn from a peer interaction (`whisper-mode.md`). Whispers
inherit the auto-archive mechanic with a 24-hour default. Archived
whispers **never** appear in the global grove archives surface — they
are peer-scoped, not grove-scoped, and surfacing them in a grove
view would leak the side-channel back into the public structure
they were created to escape. Archived whispers are listed only
inside the originating peer's profile card under a "recent whispers"
section. This applies even when the whisper occurred inside the
context of a grove channel: the artifact still belongs to the peers,
not the grove.

## Archive surface

The archives view (existing) gains a new subgroup ordering:

1. Auto-archived (newest first), grouped by kind chip.
2. Manually archived (existing).

Each row shows: kind chip, channel name, last-activity timestamp in
human phrasing, and a quiet `revive` link. Tapping the row opens the
channel in read-only review mode (composer hidden); the user can
read the conversation without un-archiving. Tapping `revive` brings
the channel back to the sidebar without posting a message — useful
for "I want to keep this around for now" without performing activity.

Read-only access is gated to **prior participants of the channel
only**. A grove steward who never joined the ephemeral cannot read
its archive — the archives surface lists only ephemerals the
viewing peer was a member of. This matches active-channel access
(you cannot read a private channel you are not in); archiving does
not lower the access bar. State enforces this via the existing
"is the actor a member of the channel?" check.

Revived channels reappear in the sidebar in the active state. The
revive itself is a state event (see Data dependencies).

## Copy

Use these literally; do not paraphrase.

| context                                                | string                                                                                       |
|--------------------------------------------------------|----------------------------------------------------------------------------------------------|
| Kind chip — channel                                    | `temp`                                                                                       |
| Kind chip — thread                                     | `thread`                                                                                     |
| Kind chip — whisper                                    | `whisper`                                                                                    |
| Dormant meta                                           | `{N} {unit} ago`                                                                             |
| Archived meta in archives view                         | `archived after {N} {unit} idle`                                                             |
| Creation helper                                        | `archives if no one posts for {N}. anyone can revive it by posting again.`                   |
| Creation confirm                                       | `start #{name}`                                                                              |
| Ad-hoc spawn entry                                     | `start temp channel…`                                                                        |
| Archives view subgroup                                 | `auto-archived`                                                                              |
| Revive link                                            | `revive`                                                                                     |
| Archived peer-whisper grouping (in profile card)       | `recent whispers`                                                                            |
| Read-only banner inside an archived channel            | `archived — read-only · post or tap revive to bring it back`                                 |
| Insufficient-permission tooltip on kind selector       | `you don't have permission to create channels in this grove`                                 |
| Per-channel idle override above cap                    | `temp channels archive within 90 days of inactivity`                                         |

`{N} {unit}` uses the same humanised phrasing pattern across the spec:
`5 minutes`, `2 hours`, `3 days`, `2 weeks`. No abbreviations in the
visible string (abbreviations are only for the trailing time chip in
the original message-row).

No exclamation marks anywhere. All copy lowercase except where the
copy itself contains proper nouns (none here).

## Data dependencies

Required from `willow-state`. Items marked **new** are new event
kinds; items marked **extend** reuse existing kinds with new fields.

- **extend `ChannelCreate`** — add an optional `EphemeralConfig`
  payload: `{ kind: EphemeralKind, idle_threshold_ms: u64 }` where
  `EphemeralKind ∈ {Channel, Thread, Whisper}`. Absence of the
  payload means a permanent channel. State team may reject
  `idle_threshold_ms` outside `[3_600_000, 90 * 24 * 3600 * 1000]`
  (1 hour to 90 days).
- **derived `last_activity_hlc`** — materialize tracks the latest
  message-emission HLC per channel; ephemeral surface state derives
  from this against the frontier HLC. No new event needed for the
  active → dormant → archived transitions: they are read-only
  derivations.
- **new `ChannelRevive`** — emitted when a user explicitly taps
  `revive` from the archives surface. Payload `{ channel_id }`.
  Updates `last_activity_hlc` to the event HLC so the materialize
  derivation flips back to active. Posting a normal message also
  achieves this implicitly without needing a `ChannelRevive` event.
- **read-only review** — the archives surface mounts a read-only
  view. No new event; the UI suppresses composer + write actions
  client-side. Posting from inside the archived channel is allowed
  (it acts as an implicit revive); the composer is hidden by default
  but can be expanded via the read-only banner.

Permissions:

- `CreateChannel` (existing) gates ephemeral channel creation. Same
  gate as permanent channels.
- No new permission for revive — any peer who could post in the
  channel originally can revive it. State enforces "is the actor a
  member of the channel?" the same way it does for `MessageEmit`.

Crypto:

- The channel key is **not burned**. Archive is a UI / state
  transition only. Existing per-channel key derivation is reused
  unchanged.

## Edge cases

**No-one ever posts again.** The channel sits in archives forever.
Storage cost is bounded — archives are pruned by the storage worker
under the same retention rules as permanent channels (see existing
storage retention configuration). Auto-archived channels do not get
special treatment for retention.

**Author of last message leaves the grove.** Idle clock keeps
ticking. Membership change is not activity. Once the threshold
passes, the channel auto-archives normally. Surviving members can
revive it.

**Per-grove threshold override.** Governance can set a grove-wide
default for each ephemeral kind (see `governance.md` §Grove
defaults). Per-channel overrides at creation respect the grove
default as the *cap* for that grove (so a grove that mandates
short-lived rooms cannot be subverted by a long per-channel
threshold). When the grove cap is **lowered after** a channel was
created with a longer threshold, the channel's effective threshold
clamps to the new cap on the next materialize derivation pass — no
grandfathering. The reasoning: governance changes are a deliberate
act and should take effect immediately; storing per-channel "exempt
from cap" flags would complicate state for a corner case nobody
asked for.

**Member count drops to zero on an ephemeral channel.** The channel
auto-archives on the next derivation pass regardless of idle
threshold. Empty rooms cannot have activity by definition.

**Threads inside an archived ephemeral channel.** When the parent
archives, threads under it archive as a side-effect (their
`last_activity_hlc` is bounded by the parent). Tapping `revive` on
the parent revives all threads simultaneously.

**Whispers between two peers, one of whom is offline.** The
24-hour clock keeps ticking based on HLC. The offline peer, on
reconnect, sees the whisper in their profile-card "recent whispers"
group rather than as an active surface. Posting in the whisper
(implicit revive) brings it back for both peers on next sync.

**Clock skew.** Same as elsewhere: HLCs drive transitions, not
wall-clock time, so two devices with drifted system clocks still
agree on when the channel archives.

**Per-channel idle threshold conflicts with grove cap.** UI clamps
the slider to the grove's cap; if the cap was set after the channel
was created with a longer threshold, the channel archives at the cap
on next derivation, not at the original threshold. A meta line in
the channel banner notes this: `"grove now caps idle at {N} {unit} —
will archive sooner than originally configured"`.

**Race: revive + archive at same HLC tick.** State machine treats a
`ChannelRevive` as later than a derivation transition at the same
HLC; explicit revives win over passive archive. Idempotency: a
duplicate `ChannelRevive` for an already-active channel is a no-op
(does not advance `last_activity_hlc`).

## Accessibility

- Kind chip carries `aria-label` `"non-permanent — {kind}"` so
  screen-reader users hear the metaphor explicitly (the visual chip
  text alone — `temp`, `thread`, `whisper` — is intentionally short
  for sighted users; the verbal label clarifies).
- Dormant state is announced once per surface entry, not on every
  render: `"{name} — last activity {N} {unit} ago"`. Sub-minute
  updates do not announce.
- Archive transition is **not** announced. The row simply leaves the
  sidebar; the archives surface gains an entry. Announcing
  archive-transition would be hostile (the user might be reading the
  channel right when it transitions; the transition itself is silent
  on every device).
- Revive transition is announced once on the device that triggers
  it: `"{name} — revived"` via `aria-live="polite"`.
- Read-only banner inside an archived channel is a `role="status"`
  region read once on entry.
- Colour is never the only cue: dormant uses both colour change and
  meta text; archived uses both removal-from-sidebar and presence in
  the archives view; the kind chip uses both colour and word.
- Keyboard path: every affordance — kind selector, threshold field,
  revive link, read-only banner button — is reachable via tab order
  with `--focus-ring` from `foundation.md`.

## Acceptance criteria

- [ ] Channel creation flow exposes a `temp` kind option with an
      inactivity-threshold field defaulting to 14 days, capped at 90.
- [ ] Sidebar rows render the kind chip (`temp` / `thread` /
      `whisper`) for ephemeral surfaces.
- [ ] Dormant state dims the row name to `--ink-2` and shows the
      "{N} {unit} ago" meta when activity is in the 25–100 % window.
- [ ] When `last_activity_hlc + idle_threshold_ms < frontier_hlc`,
      the channel archives: row leaves the active sidebar; archives
      surface gains a new entry under the `auto-archived` subgroup.
- [ ] Tapping `revive` on an archived row emits a `ChannelRevive`
      event and the channel returns to the active sidebar without
      posting a message.
- [ ] Posting a message inside an archived channel is allowed and
      acts as an implicit revive (`last_activity_hlc` advances).
- [ ] Threads inherit the mechanic with a 7-day default; whispers
      inherit it with a 24-hour default.
- [ ] Per-grove governance can override the per-kind default and the
      cap; per-channel overrides at creation respect the grove cap.
- [ ] Archived ephemerals retain their channel key; messages remain
      readable when the archive entry is opened in review mode.
- [ ] Read-only banner displays inside an archived channel and the
      composer is hidden by default.
- [ ] Screen-reader announcements fire on dormant entry and on
      revive, but never on auto-archive.

## Open questions

- **Cross-spec updates.** `thread-pane.md` and `whisper-mode.md`
  must each gain a §Auto-archive section that references this spec
  for the inheritance contract (kind chip, dormant/archived states,
  default thresholds, revive). Tracked separately; not blocking the
  Phase 2d plan but required before the thread / whisper plans are
  written in Phase 3.
- **Storage retention coupling.** Auto-archived channels share the
  same storage retention rules as permanent channels. If a future
  retention policy diverges (e.g. "auto-archived ephemerals expire
  faster"), this spec needs an addendum. Out of scope for v1.
