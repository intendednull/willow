# Device handoff — move an in-progress call between your devices

**Parent:** [README.md](README.md)
**Dependencies:** [`foundation.md`](foundation.md), [`call-experience.md`](call-experience.md), [`settings-tweaks.md`](settings-tweaks.md)
**Status:** draft

## Purpose

A single identity may be attached to more than one device. *Handoff* lets a
user move an in-progress call — main call audio plus any active whisper —
from the device that joined the call to another of their own devices,
without the other participants seeing a drop, a rejoin, or any glitch in
the seal. Keys are re-derived on the target; the source leaves the call
only after the target is fully live.

Two jobs: **continuity** (no "x reconnected" from any participant's view)
and **trust** (only devices linked to the same identity can accept, and
the UI states this plainly every time).

## Scope

In scope: desktop popover, mobile bottom sheet, progress / failure
states, whisper-key transfer, screen-share pause/resume, the "move
active call here" pill in settings, dialog + listbox accessibility.

Out of scope: linking / revoking devices (owned by
[`settings-tweaks.md`](settings-tweaks.md); handoff only reads);
call join / leave and controls-strip layout (owned by
[`call-experience.md`](call-experience.md)); wire protocol for
re-seal (flagged as new in Data dependencies).

## Entry points

1. **In-call controls strip.** A "handoff" button (device icon; label
   "handoff" on desktop, bottom-dock button on mobile).
2. **In-call header overflow.** The call header's `…` menu contains
   "move this call" as its second item (after "call info").
3. **Settings → devices.** When another linked device is currently in a
   call, the device list row for *this* device shows a quiet moss-tinted
   pill: "move active call to this device". Tap opens the handoff sheet
   on the target-side of the flow.
4. **Keyboard.** Desktop has `⌘⇧H` / `Ctrl+Shift+H` while a call is
   focused. Documented in the help overlay, not in tooltips.

If the user has only one linked device, the handoff button is visible
but disabled, with tooltip copy: *"add another device in settings to
hand off calls."*

## Handoff popover (desktop)

Anchored above the "handoff" button in the call control strip. Uses
the `willow-pop-in` keyframe at `--motion` duration.

Container:

- Width: 320 px; `--bg-1` with `--line` border and `--shadow-2`.
- Radius: `--radius`.
- Padding: `16 px`.
- Focus trap: entire popover; `Escape` closes; outside click closes.

Header (stacked):

- Icon `device` at 14 px + title "move this call" in `--font-display`
  italic, 15 px, `--ink-0`. Matches the call popover vocabulary
  (whisper / stats).
- Subtitle line in `--ink-2`, 12 px, line-height 1.5:
  *"keys re-seal automatically on the new device. no re-join."*

Device list (vertical):

- The current device is pinned first, row disabled, tag "here" in
  `--moss-3` on `--line` border, label "this device" in `--ink-3`.
  It is present for orientation only; it is not a selection target.
- Remaining rows are the user's other linked devices, in
  most-recently-online order.
- Each row is a `<button>` styled as a list item:
  - Height ~44 px (meets the touch-target baseline even on desktop).
  - Background `--bg-2`; border `--line`; radius `--radius`.
  - Icon `device` 14 px on the left.
  - Device nickname in `--font-mono`, 12 px, `--ink-0` (e.g.
    `willow · phone`, `willow · cabin`).
  - Status in `--ink-3`, 10.5 px sans body: one of
    - `online · {network}` — e.g. `online · ethernet`, `online · wifi`,
      `online · cellular`.
    - `last seen {relative} · {network}` — e.g.
      `last seen 3m ago · wifi`.
    - `offline` — row is disabled, opacity 0.5, no selection.
  - Right edge: selection affordance — a 12 px empty circle that fills
    with `--moss-2` and inner check glyph when selected; for rows that
    aren't selected it shows a `chevron` glyph for affordance.
- Reference constant: `HANDOFF_DEVICES_DESKTOP` in the design bundle.

Primary / secondary buttons (bottom strip):

- Primary CTA: "hand off" — `--moss-2` fill, `--moss-4` text, radius
  `--radius-s`, disabled until a selection is made *and* that device
  is online. Disabled reason is shown under the button in
  `--ink-3` hint: "choose an online device".
- Secondary: "cancel" — ghost button, `--ink-2`.
- Keyboard: `Enter` commits when a valid device is selected; `Escape`
  cancels; `↑ ↓` navigates device rows; `Tab` leaves the popover.

With two linked devices the list is just two rows. Beyond six, the
list scrolls internally using the foundation `.scroll` style.

## Handoff sheet (mobile)

A standard bottom sheet per
[`layout-primitives.md`](layout-primitives.md#bottom-sheets):
full-width, top radius `--radius-l`, grabber bar, `--bg-1`,
`--shadow-2`, enters at `--motion-slow`.

Contents mirror the popover:

- Title row with `device` icon + "move this call" in Fraunces italic,
  17 px.
- Subtitle in `--ink-3`, 12 px:
  *"keys re-seal automatically on the new device. no re-join."*
- Device rows from `HANDOFF_DEVICES_MOBILE`, 11 px padding, radius
  `--radius`, tappable targets ≥ 44 × 44 px.
- Current device first, tagged "here", disabled at 0.65 opacity.
- Selected target row gains a `--moss-0` background tint, `--moss-3`
  border, and a filled `check` glyph on the right.
- Primary button at the bottom, full width: "hand off" — `--moss-2`
  fill, `--moss-4` text, 48 px tall. Disabled until a selection is
  made; disabled-copy rendered under the button same as desktop.
- Secondary dismiss: swipe down, backdrop tap, or a small `x` in the
  top-right of the sheet header.

Sheet content uses `.noscroll`; if the list doesn't fit, the sheet
expands to its tall variant (85 vh).

## In-progress states

The flow has four phases. The popover / sheet stays on-screen through
phases 1–3 and closes automatically only on success.

**Phase 1 — `selecting`.** User browsing the list. No progress UI.

**Phase 2 — `re-sealing`.** Immediately after "hand off" is tapped:

- The device row stays selected; its right-edge chevron is replaced
  by a small `willowPulse` dot in `--moss-3`.
- Primary button disables and its label changes to the progress line
  in `--font-mono`, 12 px:
  *"re-sealing keys on {device}…"*
  where `{device}` is the nickname (e.g. `willow · phone`).
- A 2 px-tall indeterminate progress bar in `--moss-2` appears under
  the button; it uses `shimmer` keyframe at 1200 ms.
- Other device rows fade to 0.4 opacity (non-interactive).

**Phase 3 — `ready`.** Keys are re-derived and negotiated; the call
media path on the target device is warm. The popover/sheet shows:

- A short status line in `--ink-1`, 12 px:
  *"ready — complete the handoff on {device} to continue"*
- Primary button label reverts to "hand off" but now also disabled
  (the commit is now local to the target device).
- On the *target* device, a push notification fires (content-free —
  title: "Willow", body: "pick up the call?"), plus an in-app prompt
  surface (a full-width banner on desktop call-center, a sheet on
  mobile) with primary "pick up the call?" and secondary "not now".

**Phase 4 — `committed`.** Target accepts. Source device:

- Popover / sheet closes with a short fade.
- A one-line toast in the bottom of the call window: "call moved to
  {device}" in `--ink-2`, auto-dismisses at 3 s.
- The call view on the source device tears down cleanly (no "you left
  the call" banner; this was a move, not a leave).
- No notice is shown to other participants — from their perspective,
  the user's tile stayed live.

If the user dismisses the popover during `re-sealing` or `ready`, the
handoff is cancelled before commit; the source stays in the call.
Copy for cancellation toast: "handoff cancelled."

## Failure states

Stated copy surfaces exactly. Each returns the user to `selecting`
phase with the device list still visible.

1. **No response (20 s timeout).** Row status flips to `--warn`:
   *"no response from {device}. try again or keep this call here."*
   Primary button returns to "hand off" and re-enables on re-select.
2. **Declined.** Inline copy in `--ink-2`: *"{device} declined the
   handoff. call remains here."* Row gets a dashed 1 px `--line`
   border for the session; selecting again is allowed.
3. **Dropped mid-handoff (after `ready`).** Aborts cleanly; source
   stays in the call: *"{device} dropped — call remains here."*
4. **Source crashes during handoff.** Target's live prompt updates
   to: *"{peer} dropped — you can pick up now."* Single "pick up"
   confirmation; keys are already re-sealed, so continuity holds.
5. **Only one linked device.** Handoff button disabled with tooltip
   *"add another device in settings to hand off calls."*
6. **Target can't derive whisper keys** (see *Whisper interaction*).
   Inline error, `--err`: *"{device} can't seal this whisper. choose
   a different device or end the whisper first."* Handoff blocked
   until another target is picked or the whisper is ended.

All failure copy is lowercase and non-pejorative per
[`foundation.md#copy-voice`](foundation.md#copy-voice). The word
"failed" never appears in the user-facing string.

## Security invariants + copy

Three invariants are always surfaced in the flow:

1. *"keys re-seal automatically on the new device. no re-join."* —
   subtitle of the popover / sheet. Explains why handoff is not
   observable to other participants.
2. *"only devices linked to your identity can accept a handoff."* —
   a quiet explainer line under the device list (below last row,
   `--ink-3`, 11 px). Reinforces that the list *cannot* contain a
   stranger.
3. *"you will disconnect from this device when the handoff
   completes."* — this surfaces as a micro-hint in `--ink-3` under
   the primary button once a target is selected (replaces the
   "choose an online device" hint).

No confirmation dialog is layered on top — the copy is enough; an
extra modal would be paternalistic.

## Whisper interaction

If the user is currently in a whisper, handoff transfers the whisper
keys to the target along with the main-call keys:

- The handoff popover / sheet title area gets a small violet `ear`
  glyph next to the title and a second subtitle line in
  `--whisper` tone (14 % on `--bg-1`, mono 11 px):
  *"whisper keys will move with the call."*
- On commit, both the main-call seal and the whisper seal are
  re-derived on the target.
- If the target device *can't* derive whisper keys — which shouldn't
  happen when the device is linked to the same identity, but is
  defensively checked — the pre-flight fails and the blocking copy
  is shown (see failure state 6).

Whisper state is preserved across the move: the violet pill remains
active on the target, and the whisper participants see no state
change. The "whispering" ring on the user's tile stays lit on every
participant's view for the duration.

## Screen-share interaction

If the user is screen-sharing at the time of handoff:

- During `re-sealing` and `ready`, the source *pauses* its screen
  share. Other participants see a single quiet notice in the call:
  *"screen share ended — handoff in progress"* in `--ink-3` body S,
  attached to the sharer's tile. It does *not* collapse the tile.
- On `committed`, the target resumes screen share *only if* it has a
  viable source (camera, capture, or window picker available). If
  it does, the share resumes with a fresh tile and a soft fade-in.
- If the target has no share source, the share ends. The call
  participants' tile returns to the normal avatar view with the
  same notice: *"screen share ended — handoff in progress"*, which
  then auto-dismisses 3 s after commit.
- Desktop source before handoff: an inline warn line appears above
  the primary button in `--amber`, 11 px:
  *"screen share will pause during the move."*
- Mobile source: same warn line in the sheet.

The notice copy is deliberately vague about which side ended the
share — the other participants don't need to know handoff occurred.

## Device-list source (hand off to settings-tweaks)

Handoff consumes a *read-only* view of the user's linked devices.
The list is produced and maintained in
[`settings-tweaks.md`](settings-tweaks.md). This spec does not define
the add / revoke flow; it only describes the shape of the view it
consumes.

Per-device fields this surface needs:

- `nickname: string` — e.g. `willow · phone`. Mono.
- `identity_fingerprint_short: string` — 3-word, used only by the
  settings surface; handoff does not display it.
- `presence: { state: 'online' | 'offline' | 'unknown',
                last_seen: Timestamp | null,
                network_kind: 'ethernet' | 'wifi' | 'cellular' |
                              'unknown' }` — drives the status line.
- `is_current: boolean` — marks the row as "here".
- `capabilities: { voice: bool, video: bool, share: bool,
                   whisper: bool }` — used to pre-flight errors
  (whisper / share).

Devices that are *not* in the linked list are not shown. Ever. There
is no "other" section, no unknown-device row, no "add a device
here" affordance inside the handoff surface. Adding a device is a
settings task; the handoff surface links to it at the very bottom,
in `--ink-3` body S:

*"manage devices →"* (navigates to settings → devices).

## Copy (exact)

Every string the user may see in this flow. Lowercase, no exclamation.

| Key | String |
|---|---|
| title | `move this call` |
| subtitle | `keys re-seal automatically on the new device. no re-join.` |
| primary CTA | `hand off` |
| secondary CTA | `cancel` |
| current-row tag | `here` |
| current-row meta | `this device` |
| status · online | `online · {network}` |
| status · last seen | `last seen {time} · {network}` |
| status · offline | `offline` |
| hint · no target | `choose an online device` |
| hint · ready to commit | `you will disconnect from this device when the handoff completes.` |
| trust explainer | `only devices linked to your identity can accept a handoff.` |
| screen-share warn (pre) | `screen share will pause during the move.` |
| whisper sub-line | `whisper keys will move with the call.` |
| progress | `re-sealing keys on {device}…` |
| ready | `ready — complete the handoff on {device} to continue` |
| target prompt | `pick up the call?` |
| target prompt (crash) | `{peer} dropped — you can pick up now` |
| toast · moved | `call moved to {device}` |
| toast · cancelled | `handoff cancelled.` |
| failure · timeout | `no response from {device}. try again or keep this call here.` |
| failure · declined | `{device} declined the handoff. call remains here.` |
| failure · mid-drop | `{device} dropped — call remains here.` |
| failure · whisper block | `{device} can't seal this whisper. choose a different device or end the whisper first.` |
| solo-device tooltip | `add another device in settings to hand off calls.` |
| participant notice (share) | `screen share ended — handoff in progress` |
| manage devices link | `manage devices →` |

Time tokens follow the foundation rule — "3m", "1h", "yesterday" — and
never render absolute timestamps in this surface.

## Data dependencies

This section flags anything that is **new** to the client state
machine. Handoff cannot ship without these landing in `willow-state`
or `willow-client` — they are called out here so the implementation
plan can depend on them explicitly.

- **Linked devices per identity (NEW).** The client needs a list of
  devices attached to the current identity, authored by the identity
  owner. Surface requirement here is read-only; the write path is
  owned by settings-tweaks. Tracked as `LinkedDevices` state derived
  from future `LinkDevice` / `RevokeDevice` events.
- **Device presence signalling (NEW).** The handoff list needs live
  `online | offline | last_seen + network_kind` per device. This is
  not a chat message and not part of `ServerState`; it's peer-to-peer
  presence within an identity, likely a lightweight gossip topic or
  direct-to-linked-devices side channel. Design-only here.
- **Handoff protocol (NEW).** Three new messages between source and
  target devices of the same identity: `HandoffOffer` (with re-seal
  material), `HandoffReady` (target has installed keys), and
  `HandoffAck | HandoffDecline | HandoffTimeout`. The re-seal
  material is itself wrapped in the identity's device-to-device
  seal. Protocol design lives in a separate spec; this UI spec only
  commits to the states it renders.
- **Call state reference.** The handoff surface reads the current
  call's `call_id`, `participants`, `whisper_state`, and
  `share_state`. These come from `call-experience.md` and are
  existing shapes.

No changes to chat, messaging, pins, or server state are required.

## Edge cases

1. **Target becomes unreachable during `re-sealing`.** Abort, surface
   the `no response` failure copy, return to `selecting`. Source
   keeps the call intact.
2. **Source crashes between `ready` and `committed`.** Target's
   prompt updates to `{peer} dropped — you can pick up now`. One
   confirmation; no second sheet; media warms and participants see
   no drop because keys and codecs are primed. If target does not
   confirm within 15 s the prompt dismisses and the participants
   see the normal dropped-peer state from the call.
3. **Only one device linked.** Handoff button is visible but
   disabled; tooltip explains. We show the button (rather than
   hiding) so the affordance is discoverable — "oh, I could link
   another device to do this".
4. **Multiple simultaneous handoff requests.** Not supported. If a
   handoff is in `re-sealing` or `ready`, the primary button is
   globally disabled and a hint shows: *"handoff already in
   progress."* Only one move at a time.
5. **Target device goes to sleep / app backgrounded.** The push
   notification is content-free and survives locked-screen; the
   in-app prompt is restored on foreground. Timeout is still 20 s
   measured from `ready`.
6. **User selects the current device.** Impossible — row is
   disabled. Keyboard navigation skips it.
7. **Accent variant is non-moss.** The selection highlight uses the
   active accent's `--moss-1` / `--moss-2` (renamed by the token
   swap). Violet whisper sub-line stays violet regardless.
8. **Reduced motion.** Progress bar becomes a static filled bar;
   `willowPulse` dots become static opacity; popover / sheet
   open/close remains opacity-only.

## Accessibility

- **Dialog semantics.** The popover and sheet both render as
  `role="dialog"` with `aria-modal="true"` and
  `aria-labelledby` pointing at the title. Focus traps inside;
  `Escape` closes and returns focus to the opening control.
- **Listbox for devices.** The device rows are a `role="listbox"`
  with `role="option"` children. `aria-selected` reflects the
  chosen target. `aria-disabled` on the "here" row and on offline
  rows. Arrow-key navigation moves selection; `Enter` commits.
- **Live region for progress.** A polite live region
  (`role="status"`, `aria-live="polite"`) announces:
  - *"re-sealing keys on {device}"* when phase goes `re-sealing`
  - *"handoff ready — complete on {device}"* when phase goes `ready`
  - *"call moved to {device}"* on commit
  - failure strings verbatim
- **Focus on phase change.** Focus does not jump on phase change;
  it stays on the primary button (now disabled). This keeps screen
  readers in place while the live region announces.
- **Target device prompt.** The "pick up the call?" banner / sheet
  is a `role="alertdialog"` (interrupting); the two buttons are in
  tab order. The banner's `aria-describedby` contains the identity
  fingerprint in 3-word short form for reassurance ("from your
  identity · willow · desk"). The fingerprint is only in the
  accessible description, not visually repeated.
- **Screen-reader labels on icon buttons.** The handoff button in
  the control strip has `aria-label="move this call"`. The list-row
  chevron is decorative (`aria-hidden`).
- **Keyboard path.** Every element is reachable without a pointer.
  On mobile, the equivalent of long-press is the Enter key on the
  focused overflow item.
- **Reduced motion.** All animations collapse to opacity; no
  transforms in `prefers-reduced-motion: reduce`.
- **Contrast.** All inline status text clears WCAG AA against its
  background: `--ink-3` on `--bg-2` verified ≥ 4.5:1; warn copy on
  the row uses `--warn` on `--bg-2` verified ≥ 4.5:1.

## Acceptance criteria

- [ ] Handoff button appears in the call control strip with
      `aria-label="move this call"` and is disabled with the tooltip
      copy when the user has only one linked device.
- [ ] Desktop popover and mobile sheet render the correct copy for
      title, subtitle, primary, secondary, and trust explainer.
- [ ] Device rows render nickname (mono), status (online /
      last-seen / offline with network kind), and the current
      device is pinned first with a "here" tag and disabled.
- [ ] Selecting an online target enables "hand off"; offline rows
      are disabled and not selectable.
- [ ] Four phases render distinctly: selecting, re-sealing, ready,
      committed. Progress bar + pulse indicator are visible in
      `re-sealing`.
- [ ] Target device receives a content-free push + in-app prompt
      with "pick up the call?"; on accept the source device tears
      down cleanly and shows the moved-toast.
- [ ] 20 s timeout triggers the "no response" copy; decline
      triggers the "declined" copy; both return to `selecting`.
- [ ] Whisper active: handoff transfers whisper keys; the
      sub-line appears; pre-flight failure blocks with the correct
      copy.
- [ ] Screen share active: warn copy on the source before commit;
      "screen share ended — handoff in progress" notice to
      participants; resume on target only if a source exists.
- [ ] Dialog semantics, listbox semantics, and live-region
      announcements pass an axe audit at zero violations.
- [ ] Reduced motion collapses progress animation and the sheet /
      popover enter/exit to opacity only.
- [ ] Settings → devices shows the quiet "move active call to this
      device" pill when another of the user's devices is mid-call.
- [ ] No string contains "failed", "error", or an exclamation mark.

## Open questions

1. **Same-identity presence scope.** Where does device presence
   propagate from — a dedicated identity-private gossip topic, or
   piggy-backed on the relay? If piggy-backed, presence accuracy
   depends on relay uptime, which is uncomfortable for a trust-
   sensitive feature. Flag for the protocol spec.
2. **Re-seal window length.** What is the maximum acceptable
   duration for `re-sealing`? Design assumes ≤ 2 s; if it exceeds
   3 s we should consider a more explicit progress affordance
   (percentage, step labels). Pending measurement.
3. **Multi-seat UX.** If the same identity is in two groves at once
   on different devices, should handoff offer to move all active
   calls, or just the focused one? Design assumes focused call
   only. Revisit once multi-call is shipped.
4. **Prompt visibility on the target.** Should the target device's
   prompt also suppress OS-level notifications for the originating
   grove during `ready` to avoid visual noise? Nice-to-have; defer.
5. **Device nicknames at creation.** Default nicknames are
   generated by settings-tweaks (`{identity} · {hint}`). If that
   spec introduces user-editable nicknames, this surface picks
   them up automatically. No work here.
6. **Call-recording interop.** If the grove is recording (future
   feature), does handoff pause the recording? Out of scope here;
   note for the recording spec.
7. **Accessibility live-region verbosity.** Announcing every phase
   change verbatim may be chatty for users on slow TTS. Consider a
   "concise" variant gated by `prefers-reduced-motion` or a future
   "quiet announcements" tweak. Defer.
