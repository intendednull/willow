# Whisper mode — private side-channels inside calls and letters

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:**
[`foundation.md`](foundation.md),
[`layout-primitives.md`](layout-primitives.md),
[`call-experience.md`](call-experience.md),
[`letters-dms.md`](letters-dms.md),
[`messaging.md`](messaging.md),
[`trust-verification.md`](trust-verification.md)

## Purpose

A *whisper* is an encrypted side-channel that branches off from a main
voice call or a letter. A subset of the participants derive a fresh key
and continue in private; the surrounding call or letter keeps running
for everyone else. Whisper content and whisper membership are invisible
to non-whisperers — they see only that *some* whisper exists, if the
surface shows a pill at all.

The design goal is to make whisper feel calmer than the main call, not
louder. Whisper is always violet (`--whisper`), always paired with the
`ear` icon, never shouts. The user should feel the temperature drop —
italic serif labels, soft left rules, a slower rhythm — rather than
feel the UI "alarm" them into a secure mode.

Whisper is the product's intimate register. It is the only surface
where the key material is ephemeral, the participant set is hidden
from bystanders, and leaving is cryptographically clean (nothing is
saved beyond what you already saw).

## Scope

In scope:

- the **whisper pill** in the call header (desktop + mobile),
- the **whisper controls** popover (desktop) / bottom sheet (mobile),
- whisper-marked **letter rows** in the letters list,
- whisper-marked **messages** inside a letter or thread (hand-off
  from `messaging.md` which defers whisper rendering to here),
- **presence + profile status** "whispering",
- **activation flow** inside a call and at letter creation,
- **teardown flow** (leave whisper, last-whisperer leaves),
- **trust gating** (verified-only by default),
- **edge cases** (drop-offline, network partition, revocation),
- **accessibility** (SR labels, reduced motion, non-colour cues),
- the **new `willow-state` events** whisper depends on.

Out of scope, deferred to follow-ups:

- hiding the whisper pill entirely from non-participants (grove-level
  privacy option — future),
- whisper-merge protocol when a network partition heals (crypto spec
  lives elsewhere; the UI surfaces a plain "network split" state),
- whisper-in-thread (threads are already sealed to thread
  participants; whisper-in-thread is not currently requested).

## Whisper pill — call header

The whisper pill is the ambient indicator that a whisper is active
inside a call. It is visible to *all* call participants, whisperers and
non-whisperers alike: non-whisperers see that a whisper *exists*,
nothing more. Content, participant names, and participant count are
never leaked by the pill unless the current user is in the whisper.

Render rules:

- Only renders when at least one whisper is active in the call.
- Background `color-mix(in oklab, var(--whisper) 14%, var(--bg-1))`,
  border `1px solid color-mix(in oklab, var(--whisper) 40%,
  var(--line))`, colour `var(--whisper)`.
- `--font-mono`, `font-size: 11`; padding `3px 10px`;
  `border-radius: 999`.
- Content: `Icons.Ear` (size 10, stroke 1.8) + label.
- Label for *in*-whisper viewers: `whisper · {names}`, joining other
  whisperers with `, ` up to 2 names, then `+N`. Self is never
  included.
  - 1 peer: `whisper · ori`
  - 2 peers: `whisper · ori, juno`
  - 3+ peers: `whisper · ori, juno +2`
- Label for bystanders: `whisper` — no names, no count.
- Clickable; opens the whisper controls popover/sheet. Bystanders get
  a dimmed version with "start a whisper" as the only affordance.
- Placement: desktop top call toolbar (right of channel title, left
  of now-speaking ticker); mobile above the participant grid,
  `alignSelf: flex-start`, inset 14 px.
- Motion: `willow-pop-in` on first activation; quiet after.

## Whisper controls — start, invite, leave

The whisper controls surface is a **popover on desktop** and a
**bottom sheet on mobile**. The visual and copy vocabulary match
across both; only the container primitive differs (see
`layout-primitives.md` for popover vs. sheet).

### Header

- Ear icon (size 14 desktop / 17 mobile) + display-italic serif title:
  - `whisper with {names}` when in a whisper (desktop size 15,
    mobile size 17),
  - `start a whisper` when not yet whispering,
  - colour: `var(--whisper)`, family: `--font-display`, italic.
- Right side: mono duration clock (`mm:ss`) in `--ink-3` when in a
  whisper. No clock when starting.
- Gradient fade on the container:
  `linear-gradient(180deg, color-mix(in oklab, var(--whisper) 14%,
  var(--bg-1)), var(--bg-1) 90%)`. Mobile uses 16% mix.

### Description

Below the header, one soft-italic line in `--ink-2`, size 12:

> side-channel inside the grove. its own ephemeral key; nobody else
> on the call can hear it.

### Starting a whisper

When no whisper is active:

1. Primary panel lists the current call members, each row showing the
   avatar, handle (mono), verified badge if applicable.
2. Each row has a round radio/check on the right for multi-select.
3. The picker allows *single-participant* whispers — "whisper with ori"
   is valid.
4. Unverified peers: if the **allow whispers from unverified peers**
   setting (see Trust gating) is OFF, unverified rows are disabled and
   show the copy "verify first" in `--amber` next to the radio.
5. CTA at the bottom: violet-bordered pill button `start whisper`.
6. On click, emit `Whisper.Start` (new event, see Data dependencies)
   and transition into the active whisper view.

### In an active whisper

- One row per other whisperer: avatar + handle + a "speaking softly…"
  italic serif caption (size 10.5, `--whisper`) when that peer is
  active, with a violet `Pulse` indicator on the right.
- Two equal-width actions at the bottom:
  - `invite someone` — `--bg-2` background, `--line` border,
    `--ink-1` text. Opens a sub-picker.
  - `leave whisper` — transparent background, violet border
    `color-mix(in oklab, var(--whisper) 40%, var(--line))`, text
    `--whisper`. Single-tap; no confirm prompt.
- Buttons: `padding: 8px 10px`, `border-radius: 8`, `font-size: 12`.

### Inviting more

When `invite someone` is tapped:

1. Sub-picker slides in (desktop) / pushes (mobile, via
   `--motion-slow`). Member list, single-select or multi-select.
2. On confirm, emit `Whisper.Invite` for each invitee. The invitees
   see a **consent prompt** before any key material is derived:
   - Title: `{name} wants to whisper with you`.
   - Body: "join the whisper? keys are separate from the main call."
   - CTAs: `join` (primary, violet) and `not now` (neutral).
3. Only after the invitee accepts does their device derive the whisper
   key via `Whisper.KeyDerive`. Declining is silent — the inviter sees
   no negative confirmation; after ~10 s the invitee's name greys out
   in the picker with a muted "didn't join" label, clearable.

### Leaving a whisper

- The `leave whisper` button is a single tap.
- Toast slides up (desktop top-right / mobile bottom-centre):
  > whisper ended — nothing is saved beyond what you already saw.
- Main call audio re-surfaces for the leaver; the whisper continues
  for the remaining whisperers. If the leaver was the last whisperer,
  the pill disappears from the call header for everyone.
- The leaver's device burns the whisper key locally. If they are
  re-invited later, a fresh `Whisper.KeyDerive` happens.

## Whisper-marked letter

A letter (1:1 or small group, see `letters-dms.md`) may itself be a
whisper letter — the *entire letter's* content is under a whisper key
distinct from the ordinary letter key. This is activated at letter
creation, not retrofitted, because persistent side-channels need key
material from the first message.

Render rules in the letters list (desktop sidebar + mobile tab):

- Row layout identical to non-whisper letters — whisper is a marker,
  not a different component.
- After the verified/unverified badge, render `Icons.Ear` size 11,
  stroke 1.6, colour `var(--whisper)`.
- Preview text stays in `--ink-3` / `--ink-1`; no italic override.
- Desktop hover background
  `color-mix(in oklab, var(--whisper) 6%, var(--bg-2))` (replacing
  the default `--bg-2`). No tint at rest.
- Mobile: active row uses the same whisper tint at 8% mix.

Verification is orthogonal to whisper: a whisper letter with an
unverified peer still shows the amber dashed `?` badge; verified
peers still show the moss `VerifiedBadge`.

Letter creation (whisper variant):

1. "new letter" flow, pick peer(s).
2. Composer shows a **lock icon menu** ("sealed with peer-keys ▾").
   Options:
   - `seal with peer-keys` (default).
   - `seal with whisper key` — selecting this changes the menu glyph
     to the ear, updates the label to `sealed with whisper key`, and
     adds a soft violet left rule to the composer border.
3. On send, emit `Whisper.Start` with the letter participants as the
   whisper set. The first message seals with the derived key.

Mid-letter whisper start in an already-running letter is **not
supported in v1**. The lock menu shows `seal with whisper key` as
disabled with the hint `whisper at letter start only`.

## Whisper-marked message — hand-off from `messaging.md`

`messaging.md` defers the full whisper-message rendering to this file.
The rules below are the canonical description of how a whisper-marked
message renders.

Visible rules apply **only when the current viewer is a whisper
participant**. To non-participants the message does not exist.

- Row background:
  `color-mix(in oklab, var(--whisper) 10%, transparent)`.
- Row left rule: `border-left: 3px solid var(--whisper)`, full row
  height, butted against the avatar column. Overrides the mention
  highlight's 2 px amber rule when both apply (whisper wins; mention
  background still layers on top via `color-mix`).
- Body text: `color: var(--ink-2)`, `font-style: italic` (using
  `--font-ui` italic, not Fraunces — keep body consistent).
- Author row renders as usual; after the timestamp, a small
  `Icons.Ear` (size 10, stroke 1.6, `var(--whisper)`) plus the
  `WhisperBadge` pill from `shared/messaging.jsx` on the first
  message in a run. Compact continuation rows still show the small
  ear next to the inline timestamp.
- Hover toolbar: the "whisper reply" ear is dimmed inside an
  existing whisper; it re-activates on non-whisper messages.
- Day separators are shared across whisper/non-whisper and are never
  violet.

On mobile: same visuals; long-press action sheet drops the "whisper
reply" row when already in a whisper. Whispered text is plain — the
left rule and ear badge do all the signalling.

## Whisper presence / status

Whisper shows up as a first-class user status, alongside online / busy
/ dnd / away / offline:

- Status key: `'whisper'`.
- Presence dot colour: `var(--whisper)`.
- Dot animation: soft `willowPulse` (ambient, 1200 ms) — same as
  online but violet.
- Status label in profile card and hover tooltip: `whispering`.
- In the letter row, the status dot on the peer avatar picks up the
  violet tint when the peer is currently in a whisper (even if it's
  *your* whisper — privacy: seeing a peer's whisper status never
  discloses *which* whisper).
- Self-presence: when the local user is in a whisper, the sidebar
  user card status reads `whispering` in `--whisper`. No mention of
  participants (redundant with the whisper pill) — just the status.
- Presence is derived, not explicit — users do not manually set
  "whisper" status. It is implied by being in a `Whisper.*` event
  stream.

In `data.jsx` the reference bundle models this as `status: 'whisper'`
on a user and `whisper: true` on DM rows and messages. The runtime
client will derive these from `Whisper.Start` / `Whisper.Leave` state,
not store them as user-set fields.

## Activation + teardown lifecycle

### Start from inside a call

1. User taps the whisper pill (or the `ear` control-bar button).
2. Whisper controls popover / sheet opens.
3. User picks one or more call members. `start whisper` emits
   `Whisper.Start`.
4. Invitees get a consent prompt. On accept,
   `Whisper.KeyDerive` emits; the pill + controls both transition to
   "whisper with {names}" for the whisperers.
5. Toast (violet, display-italic title + body line):
   > whispering with ori
   >
   > keys are separate from the main call.

### Start at letter creation

1. "new letter" flow, pick peers.
2. Composer lock menu → `seal with whisper key`.
3. First send emits `Whisper.Start` + `Whisper.KeyDerive` and encrypts
   the first message.
4. Letter row in the list appears with the violet ear marker from the
   first render.

### Teardown — leave whisper

1. `leave whisper` tap burns the local whisper key.
2. Toast:
   > whisper ended — nothing is saved beyond what you already saw.
3. Main call / main letter re-surfaces.
4. Remaining whisperers see a neutral-toned system message in the
   whisper body:
   > {name} left the whisper.
5. If the leaver was the last whisperer, the pill disappears for
   everyone and the whisper events cease.

### Teardown — whisper ends (last whisperer leaves)

When the last whisperer emits `Whisper.Leave`, the whisper state
resolves. All participants' devices burn keys. No "whisper ended"
broadcast is sent *outside* the whisper — to bystanders, the pill
simply disappears, same as if it had never been there.

## Trust gating

Whisper activation is gated by trust state. The rule is:

- By default, only **verified peers** (see `trust-verification.md`)
  can start a whisper with you. Invites from unverified peers are
  silently dropped with a sidebar notification:
  > {name} tried to whisper — verify first.
  The notification is tap-to-open-verify-flow.
- A per-user toggle in the Tweaks panel (see `settings-tweaks.md`)
  labelled **allow whispers from unverified peers** governs this.
  OFF by default. When ON, unverified peers may still initiate; the
  consent prompt on the recipient side then shows an amber
  unverified-caller warning.
- Initiating a whisper from your side with an unverified peer also
  obeys the toggle: if OFF, the unverified rows in the picker are
  disabled and show "verify first".

Whisper and verification are independent — a *whisper* can itself be
verified (SAS exchange inside the whisper) or unverified, and that
state is orthogonal to the main peer's verification state. The
verification badge shown on a whisper message/row reflects the main
peer's state; a separate sub-badge may appear in the whisper controls
header in a follow-up spec.

## Copy — exact strings

All whisper copy is lowercase (per foundation voice). Exact strings
used by this spec:

- `whisper`
- `whispering`
- `start a whisper`
- `whisper · {names}` (pill label, when in whisper)
- `whispering with {names}` (controls header, when in whisper)
- `whisper with {names}` (desktop controls header, singular/list form)
- `leave whisper`
- `whisper ended`
- `whisper ended — nothing is saved beyond what you already saw.`
  (teardown toast)
- `keys are separate from the main call.` (activation toast body)
- `side-channel inside the grove. its own ephemeral key; nobody else
  on the call can hear it.` (controls description)
- `speaking softly…` (italic serif, in-controls speaking indicator)
- `{name} left the whisper.` (system message in the whisper body)
- `{name} wants to whisper with you` (consent prompt title)
- `join the whisper? keys are separate from the main call.` (consent
  body)
- `join` / `not now` (consent CTAs)
- `{name} tried to whisper — verify first.` (gated-invite notification)
- `allow whispers from unverified peers` (Tweaks label)
- `verify first` (picker-row hint for unverified peers)
- `invite someone` (controls action)
- `sealed with whisper key` (composer lock-menu label, letter variant)
- `whisper at letter start only` (disabled hint mid-letter)

SR-only phrasings and live-region announcements are listed under
Accessibility below.

## Data dependencies

Whisper introduces four new `willow-state` events. **Flag: new.** The
state crate does not yet implement these; they must be added before
this spec ships.

| Event | Purpose |
|-------|---------|
| `Whisper.Start` | Creates a whisper context. Payload: parent call or letter id, initial participant PeerId set, whisper id (hash). Author must satisfy the trust-gating rule (verified, or target has `allow-unverified-whispers` set). |
| `Whisper.Invite` | Adds a candidate peer to a whisper. Consent-pending state; not effective until the target's device emits `Whisper.KeyDerive`. |
| `Whisper.KeyDerive` | Records that a peer derived the whisper key. Treated as "joined" from the UI's perspective. Emits on activation and re-joins. |
| `Whisper.Leave` | Removes a peer from a whisper and signals their device to burn the local key. When all participants have left, the whisper dissolves. |

All four events must be added to the `EventKind` enum in
`crates/state/src/event.rs` with corresponding `apply()` handlers
in `crates/state/src/materialize.rs`, per the "Adding a new EventKind"
guidance in `CLAUDE.md`.

Permission checks:

- `Whisper.Start`: author must be a member of the parent (call member
  or letter participant) and must satisfy trust gating for *every*
  initial participant. No new `Permission` variant is required — the
  gate is a property of the target peer's `allow-unverified-whispers`
  setting, which is a per-user preference, not a `Permission`.
- `Whisper.Invite`: author must be a current whisper participant
  (i.e. have a `Whisper.KeyDerive` in the current whisper's event
  tail).
- `Whisper.KeyDerive`: author is always self; event is self-signed
  and its presence is what marks a peer as "joined".
- `Whisper.Leave`: author is always self.

Whisper message content itself is *not* new state — whispered
messages reuse the existing `Message` infrastructure (`willow-
messaging`) with the whisper key used as the seal key. The whisper
id is carried in the message envelope so clients render it with the
correct participant set.

## Edge cases

### Whisperer drops offline

- Their presence dot in the whisper controls greys out; caption
  changes to `offline — reconnecting` (`--ink-3`, italic).
- Their last `Whisper.KeyDerive` still holds on other devices; they
  rejoin silently on reconnect if they still hold the key.
- If they burned their key (explicit leave, device wipe), reconnect
  triggers a fresh consent prompt.
- The whisper continues; only their tile dims.

### Network partition splits the whisper

- Each subset stays sealed to itself — all hold the same key, but
  neither subset can reach the other.
- The whisper controls show an amber advisory pill under the header:
  > network split — some whisperers are not reachable.
- Bystanders on the main call are unaffected.
- Merging two whisper event logs after partition heal is a crypto
  problem; the UI just surfaces the advisory. Deferred.

### User revoked from whisper

- Performed by a current whisperer via a more-actions menu on a peer
  row in the controls (`remove from whisper`, `--err`).
- `Whisper.Leave` fires on behalf of the revoked peer (see Open
  questions — may need a separate `Whisper.Revoke`).
- Revoked peer's device burns the key; from the next tick they see
  the main call / letter re-surface. No toast.
- Pill count decrements for remaining whisperers; bystanders see no
  change.

### Inviter leaves before invitee accepts

- Consent prompt remains valid while at least one other whisperer
  stays. If the whisper dissolves first, the prompt reads
  `whisper no longer available.` with a single dismiss.

## Accessibility

- **Whisper pill**: `aria-label` is
  `whisper with {names}, click to manage` when in whisper, or
  `whisper active in this call` for bystanders.
- **Ear icon** accompanies every violet surface so colour-blind users
  have a non-colour signal. Violet is *never* the only indicator.
- **Live region announcements** (polite):
  - on join: `you are now whispering with {names}.`
  - on leave: `you left the whisper.`
  - on network split: `network split — some whisperers are not
    reachable.`
  - on end: `the whisper has ended.`
- **Focus order** inside the whisper controls: header → description
  → member list → primary action → secondary action. All elements
  reachable via Tab; Escape closes the popover / sheet and returns
  focus to the pill.
- **Touch targets** ≥ 44 × 44 CSS px on mobile for every whisper
  action (pill, sheet buttons, member picker rows).
- **Reduced motion**: `willow-pop-in` collapses to opacity-only for
  the pill entrance and controls surface. `willowPulse` on the
  presence dot becomes static opacity.
- **Screen-reader text** on icon-only controls:
  - Ear control-bar button: `start a whisper` / `whisper controls`.
  - "whisper reply" in the hover toolbar: `whisper reply`.
  - Violet dot in the letters list: SR text `whispering`.
- **Keyboard-only path** for every interaction: picker rows accept
  Enter/Space; `leave whisper` is Tab-reachable; the consent prompt's
  `join` / `not now` CTAs are default-focusable.

## Acceptance criteria

- [ ] Whisper pill renders in the call header whenever at least one
      whisper is active in the current call.
- [ ] Bystanders see a content-less `whisper` pill (no names, no
      count); whisperers see `whisper · {names}` with truncation
      rule applied.
- [ ] Whisper controls surface opens as a popover on desktop and a
      bottom sheet on mobile, both using the gradient described in
      Whisper controls.
- [ ] Starting a whisper emits `Whisper.Start`; invitees see a
      consent prompt; accepting emits `Whisper.KeyDerive`.
- [ ] Leaving a whisper emits `Whisper.Leave`; the toast copy reads
      `whisper ended — nothing is saved beyond what you already saw.`
- [ ] Last-whisperer-leaves dissolves the whisper and removes the
      pill for all participants.
- [ ] Letter rows with `whisper` state render the violet ear marker
      after the verified/unverified badge; hover tint uses
      `color-mix(in oklab, var(--whisper) 6%, var(--bg-2))`.
- [ ] Whisper-marked messages render with a 3 px violet left rule,
      `color-mix(in oklab, var(--whisper) 10%, transparent)`
      background, muted italic body, and an ear icon next to the
      timestamp.
- [ ] Presence dot is violet with `willowPulse` when a peer is in a
      whisper; label reads `whispering`.
- [ ] Trust gate: unverified peers cannot initiate by default; the
      "allow whispers from unverified peers" toggle lives in Tweaks
      and defaults OFF.
- [ ] `Whisper.Start`, `Whisper.Invite`, `Whisper.KeyDerive`, and
      `Whisper.Leave` are added to `EventKind` with tests for
      permission rejection, dedup, and application.
- [ ] Network-split advisory pill renders inside the whisper controls
      when partition detected.
- [ ] All SR labels listed under Accessibility are present on their
      controls.
- [ ] Reduced-motion variant collapses `willow-pop-in` and
      `willowPulse` to opacity-only.
- [ ] `--whisper` is never overridden by an accent variant (guarded
      in `foundation.css`).

## Open questions

- **Revocation event kind.** Is `Whisper.Leave` sufficient when the
  revoked peer is offline, or do we need a separate `Whisper.Revoke`
  that other whisperers sign on the victim's behalf and the victim's
  device honours on next sync? Proposed answer: add `Whisper.Revoke`
  if the state crate cannot otherwise express "this peer is no
  longer in the whisper from time T even though their device hasn't
  emitted `Leave` yet". Defer until we write the plan.
- **Grove-level hide-the-pill-from-bystanders option.** Some groves
  may want to hide the whisper pill from non-participants entirely
  so they cannot even infer that *a* whisper is happening. Deferred
  to a future spec; the architecture must not preclude it.
- **Whisper-in-thread.** Threads are already sealed to thread
  participants; whisper-in-thread would be a side-channel inside a
  side-channel. Not requested in v1.
- **Mid-letter whisper start.** Currently disallowed because
  persistent side-channels need key material from letter creation.
  If a follow-up adds mid-letter whisper, the composer lock-menu
  `seal with whisper key` affordance becomes enabled and the spec
  needs a "switch from regular to whisper mid-letter" flow.
- **Verification of the whisper itself.** Should the whisper controls
  expose an in-whisper SAS flow to verify the whisper key
  independently of the main peer? Likely yes, as a follow-up
  integrated with `trust-verification.md`; not in this spec.
- **Whisper history retention.** "nothing is saved beyond what you
  already saw" is a UX promise; confirm with `willow-messaging`
  storage semantics before plan-writing.
- **Consent-prompt timeout.** 10 s to grey out a non-responding
  invitee is a guess; may need to be longer on mobile. Revisit
  during the plan.
