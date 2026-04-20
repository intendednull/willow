# User flows

**Date:** 2026-04-19
**Status:** draft
**Purpose:** Stitch individual UX specs into canonical end-to-end
journeys. Specs describe screens and components; users experience
*flows* that cross multiple specs. This document names each canonical
journey, traces the sequence of specs the user passes through, and
flags hand-offs not already documented in the relevant spec.

Each flow is a canonical sequence across UX specs. Deviations should be
documented as a variant of the nearest canonical flow rather than a
separate flow. Copy gates are verbatim from the owning spec and must
not be altered without a copy change to that spec.

**Spec-split note.** The former monolithic messaging spec has been
fully split into `message-row.md`, `composer.md`, `reactions-pins.md`,
and `files-inline.md`. This document uses those files directly; no
references to the old spec remain.

## Flow index

| # | Flow | Primary specs crossed |
|---|------|------------------------|
| 1 | first run | onboarding, trust-verification, letters-dms, discover |
| 2 | verify a peer (SAS) | trust-verification, profile-card, letters-dms, onboarding |
| 3 | start a whisper in a call | call-experience, whisper-mode, trust-verification |
| 4 | hand off a call to another device | call-experience, device-handoff, whisper-mode, settings-tweaks |
| 5 | join a grove via letter of introduction | letters-dms, governance, discover, onboarding |
| 6 | create an ephemeral channel | layout-primitives, ephemeral-channels, governance |
| 7 | send message while offline | messaging, sync-queue, letters-dms |
| 8 | start a letter (new DM) | letters-dms, profile-card, trust-verification |
| 9 | escalate 1:1 letter to group | letters-dms, messaging |
| 10 | reply in a thread | messaging, thread-pane |
| 11 | pin a message | messaging, layout-primitives, governance |
| 12 | change accent / density | settings-tweaks, foundation |
| 13 | report a grove in discover | discover, governance |
| 14 | receive key-rotation downgrade | trust-verification, letters-dms, profile-card |
| 15 | share a file in a letter | messaging, letters-dms |

## Glossary of triggers

Reused across flows. Each trigger names the input primitive and the
spec that owns the threshold.

- **tap** — mobile touch activation; desktop click.
- **long-press** — mobile hold. Message rows ≥ 500 ms (`message-row.md`);
  peer avatar for SAS ≥ 350 ms (`trust-verification.md`); onboarding
  hold-to-confirm ≥ 900 ms (`onboarding.md`).
- **hover** — desktop pointer-over. Reveals hover toolbar
  (`message-row.md`) and tooltips on badges / chips.
- **key** — keyboard. `Enter` activates, `Esc` dismisses, `⌘⇧H` /
  `Ctrl+Shift+H` triggers handoff while a call is focused
  (`device-handoff.md`).
- **swipe** — mobile drag. Swipe-right on a message row opens thread
  reply; swipe-left quote-replies (`message-row.md`); pull-down on the
  letters list reveals the sync-queue summary (`sync-queue.md`).
- **system event** — network reachability change, HLC timestamp tick,
  key-rotation notice received over gossip.

---

## 1. First run

**Goal.** Fresh user creates identity, sees fingerprint, optionally
pairs with a peer, creates or joins a first grove, lands in chat.

**Entry.** Fresh install, no stored identity. Boots into `onboarding.md`
§ Step 1 · Welcome.

**Sequence.**
1. **Welcome** (`onboarding.md` § Step 1). Trigger: tap `begin` →
   step 2. Alt: `i already have willow` → recovery branch (not
   canonical).
2. **Identity** (§ Step 2). User fills display name, pronouns, crest,
   accent. Trigger: tap `continue` → generates Ed25519 keypair via
   `willow-identity`, advances to step 3.
3. **Your six words** (§ Step 3). Fingerprint card renders atoms from
   `trust-verification.md` § SAS fingerprint grid. Trigger: tap
   `i'll remember` → step 4. Alt: `skip for now` → settings-gear amber
   dot registered; still advances.
4. **Pair with a peer** (§ Step 4). Two branches:
   - `i have an invite` → paste `willow://…` URL or scan QR → parsed
     invite handed to `letters-dms.md` join path → SAS ceremony from
     `trust-verification.md` § Compare-fingerprints flow renders
     inside the onboarding shell (Flow 2 as a sub-flow).
   - `show me mine` → generates invite via `letters-dms.md`; QR +
     URL shared; on peer connect, same SAS.
   On match, `verified peer` toast fades in; advances to step 5 with
   invite's grove pre-selected. Alt: `skip — find a peer later` →
   step 5, with sidebar nudge `you have no verified peers yet — pair`.
5. **First grove** (§ Step 5). Three cards:
   - A `plant grove` → emits `CreateServer`, advances.
   - B `step into <grove name>` → delegates to
     `letters-dms.md` / `discover.md` existing `JoinPage` flow.
   - C `browse discover` → opens `discover.md` inside the onboarding
     shell; returns on success.
6. **Final tour** (§ Step 6). Four teaching cards (groves, whispers,
   ephemerals, queued delivery). Trigger: `enter willow` → routes to
   the grove via `layout-primitives.md`. Alt: `skip tour`.

**Success.** Onboarding complete; first channel rendered.

**Abort / alternate paths.**
- Step 1 `i already have willow` → § Recovery flow (welcome → recover
  → grove → tour); skips steps 2–4.
- SAS mismatch in step 4: non-fatal amber card; retry or `skip this
  peer`.
- Step 5 skip only exposed on recovery when identity already has ≥ 1
  grove.

**Copy gates.**
- Welcome tagline: `a grove of your own — small group chat that lives
  on your devices, not on a server.`
- Privacy foot: `no account · no signup · nothing leaves this device
  without your action`
- Identity heading: `who are you, here?`
- Step 3 label (from `trust-verification.md`):
  `your fingerprint — read this aloud`
- Step 4 Branch A placeholder: `paste willow://… here`
- Step 5 Card A body: `a grove is a small space shared by a handful of
  people. you'll own this one — you decide who joins.`
- Tour CTA: `enter willow`

**Inter-spec hand-offs.**
- Step 3 → `trust-verification.md` grid + label atoms. Named in both.
  **OK.**
- Step 4 → `letters-dms.md` (invite payload) → `trust-verification.md`
  § Compare-fingerprints flow. Named in both. **OK.**
- Step 5 Card B → `letters-dms.md` / `discover.md` existing `JoinPage`
  flow. The grove-preview surface (title, member count, inviter
  badge) is named *only* in `onboarding.md` Card B. **GAP.**
- Step 6 → first grove via `layout-primitives.md` is implied but
  not named in a sentence. **GAP (soft).**

---

## 2. Verify a peer (SAS)

**Goal.** Turn an unverified peer verified; on mismatch keep them
unverified with whisper and handoff closed.

**Entry.** Four documented entry points:
1. Profile-card peer variant secondary row → tap `compare
   fingerprints` (slot reserved in `profile-card.md` § Field inventory
   peer-view row 17; `trust-verification.md` § Entry points).
2. Letter row chip `compare →` (desktop) or long-press avatar (mobile,
   ≥ 350 ms) — `trust-verification.md` § Long-press SAS on mobile.
3. Onboarding step 3 or step 4 — `onboarding.md` forwards into the
   flow.
4. Downgrade banner `compare now` — Flow 14.

**Sequence.**
1. Dialog opens (`trust-verification.md` § Compare-fingerprints flow).
   Desktop centred modal; mobile bottom sheet rising at 240 ms.
2. **Screen 1 — compare.** Two fingerprint grids (peer card first on
   mobile); reassurance footer; CTAs `they match` / `they don't
   match` (`SAS.matchCta` / `SAS.noMatchCta`).
3. On `they match` → **Screen 2 — confirm match.** Emits
   `client.mark_verified(peer_id)`. Default focus on `done`;
   secondary `undo` available via toast for 10 s.
4. On `they don't match` → **Screen 2b — confirm mismatch.** Emits
   `client.mark_unverified(peer_id, reason: SasMismatch)`. CTA
   `compare again` loops to Screen 1; `close` dismisses.

**Success.** Verified state propagates to every peer-identifying
surface listed in § Badges § Placement rules by surface. Whisper
gating (Flow 3) and handoff whisper preservation (Flow 4) unlock.

**Abort / alternate paths.**
- `Esc` closes; on Screens 2/2b `Esc` persists the decision (not
  `undo`). No background-click dismiss on desktop. Non-blocking —
  messaging continues behind the dialog.
- `not sure` CTA reserved for `V1_ALLOW_UNSURE_CTA` feature flag.

**Copy gates.**
- `SAS.title`: `add a friend`
- `SAS.matchCta`: `they match` · `SAS.noMatchCta`: `they don't match`
- `Confirm.matchTitle`: `verified.`
- `Confirm.matchBody`: `verified peer — this cannot be silently
  downgraded by an attacker. their key is pinned; if it ever changes
  you'll be asked to verify again.`
- `Confirm.mismatchTitle`: `marked not verified.`
- `Confirm.mismatchBody`: `marked not-verified — we will keep this
  peer unverified until you compare again. you can still send
  messages, but whisper and device handoff stay closed until the
  fingerprints match.`

**Inter-spec hand-offs.**
- Profile card row 17 ↔ `trust-verification.md` § Entry points. **OK.**
- Letter row chip ↔ `letters-dms.md` pendingVerify +
  `trust-verification.md` § Entry points. **OK.**
- Onboarding ↔ `trust-verification.md` § Entry points 3. **OK.**

---

## 3. Start a whisper in a call

**Goal.** Open whisper controls inside an active call, pick
participants, activate a violet side-channel with a derived key, then
tear down.

**Entry.** Inside a call (`call-experience.md` § Controls strip).

**Sequence.**
1. Tap the whisper (`ear`) button in call controls
   (`call-experience.md` § Whisper integration point 1). Opens
   popover (desktop) or sheet (mobile) from `whisper-mode.md` §
   Whisper controls.
2. Controls render header `start a whisper`, description, picker of
   call members (§ Starting a whisper).
3. **Trust gating** (`whisper-mode.md` § Trust gating). Unverified
   rows disabled with `verify first` unless the Tweaks toggle
   `allow whispers from unverified peers` is ON
   (`settings-tweaks.md` § Tweaks panel).
4. User selects participants; taps `start whisper` → emits
   `Whisper.Start`.
5. Invitees see a **consent prompt** (§ Inviting more). Body: `join
   the whisper? keys are separate from the main call.` Only on
   accept does their device emit `Whisper.KeyDerive`.
6. **Derived-key toast** on activation: `whispering with ori` / body
   `keys are separate from the main call.` (§ Start from inside a
   call step 5).
7. **Whisper pill** appears in call header (`call-experience.md` §
   Whisper integration point 2); per-tile ear icon + violet ring for
   co-whisperers only.
8. **Teardown.** Tap `leave whisper` → burns local key. Toast:
   `whisper ended — nothing is saved beyond what you already saw.`
   If last whisperer leaves, pill disappears for everyone.

**Success.** Whisper-marked messages render with violet left rule +
ear badge from `whisper-mode.md` § Whisper-marked message.

**Abort / alternate paths.**
- Invitee declines → silent; ~10 s later their name greys with
  `didn't join`.
- Unverified inbound whisper with toggle OFF → sidebar notification
  `{name} tried to whisper — verify first.` (tap opens Flow 2).
- Call ends before teardown → whisper dissolves; keys burned.

**Copy gates.**
- Controls description: `side-channel inside the grove. its own
  ephemeral key; nobody else on the call can hear it.`
- Consent title: `{name} wants to whisper with you` / body `join the
  whisper? keys are separate from the main call.` / CTAs `join` /
  `not now`.
- Teardown toast: `whisper ended — nothing is saved beyond what you
  already saw.`
- Speaking indicator: `speaking softly…`

**Inter-spec hand-offs.**
- Header pill slot in `call-experience.md` ↔ pill rendering in
  `whisper-mode.md` § Whisper pill — call header. **OK.**
- Tweaks toggle ↔ `settings-tweaks.md` label. **OK.**
- Whisper-marked message row rendering ↔ `message-row.md` § Whisper
  hand-off explicitly defers to `whisper-mode.md`. **OK.**
- Status on profile card (`whispering`) ↔ `profile-card.md` status
  pill row 8. **OK.**

---

## 4. Hand off a call to another device

**Goal.** Move an active call from source to a linked target device;
keys re-seal; source disconnects; other participants see no change.

**Entry.** In-call. Entry points (`device-handoff.md` § Entry points):
controls-strip "handoff" button; header overflow `…` → `move this
call`; Settings → devices row with `move active call to this device`;
`⌘⇧H` / `Ctrl+Shift+H`.

**Sequence.**
1. Popover (desktop) or sheet (mobile) opens anchored to the call
   controls (`call-experience.md` § Handoff integration reserves the
   anchor). Title `move this call`, subtitle `keys re-seal
   automatically on the new device. no re-join.`
2. Device list renders (`device-handoff.md` § Device list). Current
   device tagged `here`, disabled. Other linked devices follow in
   most-recently-online order. Offline rows disabled at 0.5 opacity.
3. Tap an online row; primary CTA `hand off` enables.
4. **Phase 2 `re-sealing`.** Button label swaps to mono
   `re-sealing keys on {device}…`; 2 px progress bar shimmers; other
   rows fade to 0.4.
5. **Phase 3 `ready`.** Source line: `ready — complete the handoff on
   {device} to continue`. Target gets a push notification and in-app
   `pick up the call?` prompt (secondary `not now`).
6. **Phase 4 `committed`.** Target accepts → source popover closes;
   toast `call moved to {device}` auto-dismisses in 3 s; source tears
   down cleanly. Other participants see no change.
7. Whisper preserved: title gains a violet ear glyph + second
   subtitle `whisper keys will move with the call.` Both main-call
   and whisper seals re-derive on target.

**Success.** Target device is in the call; source cleanly
disconnected.

**Abort / alternate paths.**
- Dismiss during `re-sealing` or `ready` → toast `handoff cancelled.`
- No response (20 s): `no response from {device}. try again or keep
  this call here.`
- Declined: `{device} declined the handoff. call remains here.`
- Dropped after `ready`: `{device} dropped — call remains here.`
- Source crash: target prompt becomes `{peer} dropped — you can pick
  up now.`; continuity holds (keys already re-sealed).
- Only one linked device: handoff button disabled with tooltip `add
  another device in settings to hand off calls.`
- Target can't derive whisper keys: `{device} can't seal this
  whisper. choose a different device or end the whisper first.`

**Copy gates.**
- Title: `move this call`
- Subtitle: `keys re-seal automatically on the new device. no re-join.`
- Progress: `re-sealing keys on {device}…`
- Ready: `ready — complete the handoff on {device} to continue`
- Committed toast: `call moved to {device}`
- Whisper addendum: `whisper keys will move with the call.`

**Inter-spec hand-offs.**
- Controls anchor ↔ `call-experience.md` § Handoff integration.
  **OK.**
- Device list source ↔ `settings-tweaks.md` § Identity + devices.
  Named in both. **OK.**
- Whisper preservation: `device-handoff.md` § Whisper interaction
  owns the source side. `whisper-mode.md` does not describe the
  target's post-handoff whisper controls. **GAP (soft).**

---

## 5. Join a grove via letter of introduction

**Goal.** Receive a letter of introduction, preview the grove, join,
land in the grove home.

**Entry.** Inbound letter from a grove steward delivered as a normal
letter (`letters-dms.md`), whose body carries the invite payload
surfaced via `discover.md` § Invitation response — hand-off to
letters. (The outbound creation side is owned by `governance.md` §
Manage → Invites.)

**Sequence.**
1. Letter appears in the letters list (`letters-dms.md` § Letters
   list). Row shows inviter + trust badge.
2. Tap row → letter thread view (§ Letter thread view). Invite embed
   has inline `join now` CTA (`discover.md` § Invitation response).
3. Tap `join now` → grove-preview surface (name, tagline, member
   count, inviter avatar + verified badge — per `onboarding.md` §
   Card B, the only spec that defines this compact preview shape).
4. Tap `join <grove name>` → delegates to the `letters-dms.md` /
   `discover.md` existing `JoinPage` flow.
5. Grove appears on the grove rail (`layout-primitives.md`); first
   `text` channel selected and rendered.

**Success.** Member of grove; first channel visible; composer meta
reads `sealed with grove-keys`.

**Abort / alternate paths.**
- Attached role (`governance.md` § Manage → Invites) fires
  `AssignRole` on accept. Unknown role: `this role no longer exists.
  accepting this letter still joins the grove but without a role.`
- Invite revoked / exhausted / expired → `join now` disabled; terminal
  states per `governance.md` (`revoked {time}`, `exhausted`,
  `expired`).
- Decline or no-response window → letter notice `no response yet from
  {grove}.`

**Copy gates.**
- Discover→letters explainer: `your request goes to this grove's
  steward. you'll see a response in letters.`
- Governance invite info: `invites are human-readable fingerprints.
  paste one from someone you trust, or share yours by voice.`
- Create success meta: `this letter was signed by {self}. when they
  arrive, compare fingerprints.`
- Unknown role: `this role no longer exists. accepting this letter
  still joins the grove but without a role.`

**Inter-spec hand-offs.**
- Steward invite creation (`governance.md`) → carried via
  `letters-dms.md` → accept via `discover.md` / `letters-dms.md`
  `JoinPage`. **GAP**: the inbound letter-of-introduction body
  *rendering inside the letter thread* (embed shape, `join now`
  layout) is not owned in any spec.
- Grove preview surface: defined only in `onboarding.md` Card B.
  **GAP**: lift into `letters-dms.md` or a shared join-preview spec.

---

## 6. Create an ephemeral channel

**Goal.** A steward creates a channel whose keys burn at a chosen
time. Channel appears in the sidebar with a timer; countdown advances;
keys burn.

**Entry.** Sidebar `+` (desktop) → new-channel popover; mobile FAB →
same bottom sheet (`ephemeral-channels.md` § Creation flow entries).

**Sequence.**
1. Open new-channel dialog; kind selector offers `text`, `voice`,
   `ephemeral`.
2. Enter name (lower-kebab-case, max 32, mono input font for
   ephemeral).
3. Pick kind `ephemeral`; duration reveals five presets (`1h`, `6h`,
   `24h`, `3d`, `7d`) and `custom…` tile.
4. Warning block renders (§ Creation flow step 4): dashed-amber
   callout + key-forge callout.
5. Tap `open #{name}` → emits ephemeral-channel creation event;
   routes in.
6. **Sidebar row** under the `ephemeral` group label; amber pill shows
   time remaining per § Sidebar row format rules.
7. **Header banner** reads `ephemeral — keys will be burned in {time}`.
8. **Countdown phases** (§ Destruction lifecycle): `normal` → `warn`
   (≤ 1 h) → `near` (≤ 10 min, banner dashed `--amber-soft`) →
   `pulse` (≤ 5 min) → `final minute` (toast `this channel will burn
   in 1 minute — say goodbye`) → `burn` (= 0).
9. **Burn** — row removed; if user is inside, pane fades; centred card
   `keys burned` / `conversations cannot be recovered`; leaf-fall
   burst (omitted reduced-motion); after 3.5 s or tap → grove home.

**Success.** Channel lived its lifespan; keys burned; no trace.

**Abort / alternate paths.**
- Without `ManageChannels`: ephemeral option disabled with tooltip
  `only grove stewards can create ephemeral channels`.
- One-time extension (§ Extension). Affordance hidden after first
  extension; later attempts show `already extended — cannot extend
  again`.
- Offline at burn: `keys burned on all online devices — will burn on
  others when they reconnect`.

**Copy gates.**
- Creation warning: `this channel will self-destruct in {duration} —
  keys burned on every device.`
- Key callout: `a single-use key is forged now and burned when the
  timer ends. messages can't be recovered.`
- Banner normal: `ephemeral — keys will be burned in {time}`
- Final-minute toast: `this channel will burn in 1 minute — say goodbye`
- Burn card title: `keys burned` / body `conversations cannot be
  recovered`

**Inter-spec hand-offs.**
- `+` primitive owned by `layout-primitives.md`;
  `ephemeral-channels.md` § Creation flow names the anchor. **OK.**
- `ManageChannels` defined in `governance.md`; referenced here. **OK.**
- Discovery exclusion: `ephemeral-channels.md` forbids appearance in
  `discover.md` but `discover.md` does not mirror the rule. **GAP
  (soft).**

---

## 7. Send message while offline

**Goal.** Send while offline; queue note appears; on reconnect a toast
announces delivery; queue note updates to "delivered just now".

**Entry.** Any conversation — grove channel (`message-row.md`) or letter
thread (`letters-dms.md`).

**Sequence.**
1. Device offline (or at least one recipient unreachable). Compose
   surface softens to amber mix (`message-row.md` § Offline state). Meta
   prepends `offline · queuing messages`. Placeholder becomes `offline
   — messages queue until reconnect`. Letters variant: `we'll wait ·
   no peer reachable`.
2. User types and sends.
3. **Status strip** appears at top of app chrome (`sync-queue.md` §
   Offline status strip): `waiting for {n} peers · {m} messages
   queued`.
4. **Per-message queue note** (§ Per-message queue note, `queued`
   state; `message-row.md` § Queue notes → `Pending`): `queued · will
   send on reconnect`. Row opacity drops to 0.7.
5. **Per-peer badge** on letter row / member row (§ Per-peer badge):
   `queued · {n}` amber pill.
6. **System event** — device reconnects. **Reconnection toast**
   (`sync-queue.md` § Reconnection toast): `reconnected · delivering
   {n} messages`.
7. On final-recipient delivery: opacity fades back to 1 over 180 ms;
   pill flashes `check + sent` for 900 ms; queue note updates to
   `queued earlier · delivered just now` (fades in 30 s).
8. Strip text flashes `delivered to {peer}` for 2 s, then strip hides.

**Success.** Queue drained; strip gone; notes cleared or in
`just-delivered` form.

**Abort / alternate paths.**
- Partial delivery: per-message note stays `queued` until all
  recipients received (tooltip has per-peer breakdown).
- Relay unreachable: strip appends ` · relay unreachable` and prepends
  a signal icon.
- Re-open after offline with messages arrived: **welcome-back banner**
  `willow queued {n} messages while you were away — everything
  arrived` (§ Welcome-back banner).

**Copy gates.**
- Offline placeholder: `offline — messages queue until reconnect`
- Offline meta: `offline · queuing messages`
- Strip default: `waiting for {n} peers · {m} messages queued`
- Pending note: `queued · will send on reconnect`
- Just-delivered note: `queued earlier · delivered just now`
- Inbound held note: `sent earlier · arrived now`
- Reconnection toast: `reconnected · delivering {n} messages`
- Welcome-back banner: `willow queued {n} messages while you were
  away — everything arrived`

**Inter-spec hand-offs.**
- `message-row.md` § Queue notes delegates full UX to `sync-queue.md`;
  copy matches. **OK.**
- Letter queued chip owned by `sync-queue.md` § Per-peer badge;
  `letters-dms.md` § Inline chips names the same pill. **OK.**
- Chrome slot for the status strip: `sync-queue.md` places it "top of
  app chrome" but `layout-primitives.md` does not reserve it. Chrome
  stacking order with downgrade banner + welcome-back banner is not
  defined. **GAP.**

---

## 8. Start a letter (new DM)

**Goal.** Pick a peer (by name, handle, or six-word fingerprint);
thread view opens with `letter started · say hi`.

**Entry.** Three entry points (`letters-dms.md` § Starting a letter):
desktop footer `start a letter · by fingerprint`; list-pane `+`;
profile-card `write a letter` (`profile-card.md` primary row).

**Sequence.**
1. Compose opens — modal popover (desktop) or pushed full screen
   (mobile). Header: `write a letter to…` / sub: `by name, handle, or
   six-word fingerprint`.
2. Search across known letters, trusted fingerprint book, and
   verbatim six-word fingerprints.
3. Pick one or more recipients. ≥ 2 morphs silently to group; group-
   name field appears.
4. **Whisper toggle** `start as whisper` — disabled unless every
   recipient is verified. Disabled hint: `whisper requires verified
   peers — compare fingerprints first`. Branch to Flow 2 first if
   needed.
5. Tap `start letter` (Cmd/Ctrl-Enter) → letter created; compose
   closes; thread view opens.
6. Confirmation meta at top of thread: `letter started · say hi`.

**Success.** Row in letters list under `peer letters` or `group
letters`; composer ready; `direct · no relay` chip (or `sealed` if
bridged).

**Abort / alternate paths.**
- Whisper variant (`whisper-mode.md` § Start at letter creation):
  emits `Whisper.Start` + `Whisper.KeyDerive`; letter row shows
  violet ear from first render. Mid-letter whisper start not
  supported in v1 (`whisper at letter start only`).
- Blocked peer: `{peer.name} isn't accepting letters right now`.
- Esc / back cancels.

**Copy gates.**
- Compose header: `write a letter to…` / sub `by name, handle, or
  six-word fingerprint`
- Whisper disabled: `whisper requires verified peers — compare
  fingerprints first`
- CTA: `start letter`
- Confirmation meta: `letter started · say hi`
- Empty-thread sub-copy: `{name} will see this when their client
  wakes up.`

**Inter-spec hand-offs.**
- `profile-card.md` primary action row → `letters-dms.md` compose.
  Both name the hand-off. **OK.**
- Whisper toggle ↔ `whisper-mode.md` § Start at letter creation. **OK.**
- "Trusted fingerprint book" is named in `letters-dms.md` compose but
  no spec defines its storage model, entry surface, or editing.
  **GAP.**

---

## 9. Escalate 1:1 letter to group

**Goal.** Add a third peer to a 1:1 letter. Warning explains new
sealed key; conversion divider renders; letter row migrates from
peer to group.

**Entry.** Inside a 1:1 letter. Overflow → `letter settings` →
`add peer` (desktop), or long-press header → same (mobile).
(`letters-dms.md` § Converting 1:1 → group.)

**Sequence.**
1. Tap `add peer`; recipient picker reuses compose (Flow 8).
2. Pick the new peer; confirm.
3. **Warning modal** (both existing peers see it — remote copy
   triggered by the membership event):
   > adding someone creates a new sealed key — old messages stay
   > private, future ones are shared with {new peer}
4. Tap `continue` → new letter keys derived. Letter reclassified as
   group; row migrates from `peer letters` to `group letters`;
   unread counts retained.
5. **Conversion divider** renders at change point: `new sealed key ·
   older messages stay private`. New peer sees only post-conversion
   content.

**Success.** Letter is a group with ≥ 3 participants; group
participants strip visible beneath header.

**Abort / alternate paths.**
- Cancel at warning → no conversion.
- Owner-only remove: `remove {name}? they keep their copy of old
  messages but stop receiving new ones.` Divider `{name} removed by
  {owner.name}`.
- Self-leave: `leave this letter? you'll keep old messages locally
  but stop receiving new ones.` Divider `{name} left the letter`.

**Copy gates.**
- Conversion warning: `adding someone creates a new sealed key — old
  messages stay private, future ones are shared with {new peer}`
- Conversion divider: `new sealed key · older messages stay private`
- Remove divider: `{name} removed by {owner.name}`
- Leave divider: `{name} left the letter`

**Inter-spec hand-offs.**
- Recipient picker reuse: `letters-dms.md` names it. **OK.**
- Remote warning echo: `letters-dms.md` says the warning is shown on
  *both* peers' screens triggered by the membership event, but no
  spec defines how the remote confirmation is delivered or whether
  it is blocking vs informational on the remote side. **GAP.**

---

## 10. Reply in a thread

**Goal.** Open a thread from a channel message, reply, see the
sealed-footer reminder.

**Entry.** Inside a grove channel, on an existing message row.

**Sequence.**
1. Trigger (desktop): hover row → hover toolbar (`message-row.md` §
   Hover toolbar) → tap `thread` icon. Mobile: long-press row
   (≥ 500 ms) → action sheet → `reply in thread`. Alt: swipe-right
   on row.
2. `reply in thread` fires. If thread exists, open it; else open
   empty thread with this message as parent (`thread-pane.md` §
   Create). Thread key derived lazily on first reply.
3. **Thread pane** opens — desktop right rail (mutually exclusive
   with members pane); mobile full-screen push.
4. Pane renders (§ Vertical sections): header `thread` + subtitle
   `from #<channel> · <n> replies`; parent card; reply list;
   participants; sealed footer `sealed to thread participants
   ({n}) — not the whole grove`; composer with placeholder `reply
   to thread…`.
5. User sends reply; first reply derives the thread key.
6. **Thread stub** renders on the parent in the channel
   (`thread-pane.md` § Thread stub; `files-inline.md` § Inline artefacts
   → Thread stub): icon + `<N> replies` + `last reply <relative>` +
   participant dots.

**Success.** Reply persisted; stub visible; sealed footer shown.

**Abort / alternate paths.**
- `leave thread` from overflow. Confirm `leave thread?` / `leaving
  stops you receiving new keys for this thread. replies already
  visible stay visible.`
- Archived parent: composer disabled; subtitle suffix `archived`.
- Deleted parent: `this thread's parent was removed — replies remain
  visible to participants`.
- Esc (desktop) / back (mobile) closes pane.

**Copy gates.**
- Channel action: `reply in thread`
- Composer placeholder: `reply to thread…`
- Sealed footer: `sealed to thread participants ({n}) — not the whole
  grove`
- Sealed footer (archived): `archived — participants can still read`
- Leave title: `leave thread?`
- Empty state: `no replies yet — write the first`

**Inter-spec hand-offs.**
- Message row → thread: `message-row.md` § Hover toolbar / § Long-press
  action sheet ↔ `thread-pane.md` § Create. **OK.**
- Thread stub render: both `thread-pane.md` § Thread stub and
  `files-inline.md` § Inline artefacts name it. **OK.**
- Members-pane mutual exclusion: flagged in `thread-pane.md`; pane
  itself owned by `layout-primitives.md`. **OK.**

---

## 11. Pin a message

**Goal.** Pin a message; row shows a quiet amber marker; header pin
icon tints with a count; pinned panel reachable from header.

**Entry.** Inside a grove channel, on a message row.

**Sequence.**
1. Desktop: hover → overflow `…` → `pin`. Mobile: long-press row →
   action sheet → `pin` (or `unpin`).
2. Permission check (`reactions-pins.md` § Pins): `ManageChannels`.
   Without: menu item greyed with tooltip `only stewards can pin
   here`; no-op.
3. With permission: emit pin event. **Row marker** renders (§ Pins):
   1 px amber left rule + `pinned` badge in the author meta row.
   Run is broken at pin.
4. **Header pin icon** (owned by `layout-primitives.md`) tints
   `--amber` with a mono superscript count.
5. Tap the pin icon → opens the pinned panel.

**Success.** Pinned state persisted; row marker visible; header count
updated.

**Abort / alternate paths.**
- Unpin → reverses; count decrements; tint clears at 0.
- No permission: tooltip only.

**Copy gates.**
- Overflow: `pin` / `unpin`
- Tooltip: `only stewards can pin here`
- Badge: `pinned`

**Inter-spec hand-offs.**
- Row marker: `reactions-pins.md` § Pins. **OK.**
- Header icon tint: `layout-primitives.md` owns icon button;
  `reactions-pins.md` § Header entry point names the tint. **OK.**
- Pinned-messages **panel interior** is deferred by `reactions-pins.md`
  (`panel interior not owned here`) and not owned by
  `layout-primitives.md`. The not-yet-written `reactions-pins.md`
  (referenced by `message-row.md`) is the natural home. **GAP.**

---

## 12. Change accent / density

**Goal.** Pick an accent (or density); immediate preview; persisted.

**Entry.** Desktop: circular "tweak" button in the wordmark corner.
Mobile: Settings → appearance (§ Entry points).

**Sequence.**
1. Open Tweaks (`settings-tweaks.md` § Tweaks panel § Surface).
   Desktop: 320 px popover bottom-right. Mobile: bottom sheet with
   handle.
2. Header: `tweaks` + description `live preview. changes apply
   instantly and persist on this device.`
3. Tap an **accent** chip (`moss · willow · amber · dusk · cedar ·
   lichen · ember`). Writes `--moss-*` and `--willow` on
   `documentElement.style` from `foundation.md` accent variant table.
   Whisper violet untouched.
4. Tap a **density** chip (`cozy · balanced · dense`). Writes
   `density-*` class on `#app-root`; only `--msg-pad` changes.
5. Persistence: localStorage key `willow.tweaks` (JSON). Applied
   pre-paint on boot via inline `<script>` in `index.html` to avoid
   FOUC.

**Success.** Accent and density visible immediately; persisted
per-device.

**Abort / alternate paths.**
- Outside click (desktop) / backdrop tap / swipe-down / back gesture
  (mobile) / `Esc` closes. No save button.
- Missing / malformed localStorage → defaults (moss / balanced /
  default / grove / stems / wordmark on).

**Copy gates.**
- Header: `tweaks`
- Description: `live preview. changes apply instantly and persist on
  this device.`
- Rows (labels): `accent`, `density`, `crypto visibility`,
  `call layout`, `sidebar`, `show wordmark`.

**Inter-spec hand-offs.**
- Accent variants: `foundation.md` § Accent variants ↔
  `settings-tweaks.md` § Controls. **OK.**
- `crypto visibility` consumed by `trust-verification.md`,
  `whisper-mode.md`, `message-row.md`; `call layout` by
  `call-experience.md`; `sidebar` by `layout-primitives.md`. All
  named. **OK.**

---

## 13. Report a grove in discover

**Goal.** Flag a discover card; local-only moderation record; card
gains a reminder marker. No global moderation.

**Entry.** Discover grid card header `more-horizontal`
(`discover.md` § Safety / reporting).

**Sequence.**
1. Tap `more-horizontal` → menu shows `report · this grove posted
   harmful content`.
2. Tap → report form (popover desktop, sheet mobile): header `report
   this grove`; required 500-char reason textarea; footer `cancel` /
   `submit report` (`--err` primary); explainer (verbatim):
   > willow has no global moderation. this goes to your own
   > moderation list — it stays with you, helps you remember, and is
   > visible only to you in governance → reports.
3. Tap `submit report` → emits a signed local-only record. Form
   closes.
4. Record lands in **governance → reports** (`governance.md`).
5. Reported card gains subtle `--ink-3` outline + `shield` icon in
   header as a reminder.

**Success.** Local moderation list entry created; card marked for
reporter only.

**Abort / alternate paths.**
- Cancel → no record, no marker.
- Card can be reported again; each report is a new entry.

**Copy gates.**
- Menu item: `report · this grove posted harmful content`
- Form header: `report this grove`
- Explainer: `willow has no global moderation. …`
- Primary CTA: `submit report`

**Inter-spec hand-offs.**
- Discover → governance: `discover.md` § Safety / reporting names
  `governance → reports`. `governance.md` currently has `roles &
  permissions`, `invites`, `files` tabs (plus Event log); a
  `reports` tab is not defined. **GAP.**

---

## 14. Receive key-rotation downgrade

**Goal.** On key rotation or SAS mismatch the peer is marked
unverified; banner demands re-verification; whisper and handoff pause
until re-verified.

**Entry.** System event: key-rotation notice received over gossip, or
a later SAS that mismatched a previously verified fingerprint
(`trust-verification.md` § Downgrade / re-verify prompts).

**Sequence.**
1. Client marks peer unverified. Every peer surface flips
   (verified → unverified badge per § Badges § Placement rules).
2. **Downgrade banner** renders at the top of the peer's letter and
   on the peer's profile card below the crest (§ Visual). Dashed
   amber border, no shadow.
3. Title `keys changed — verify again`. Body: `this peer's key
   rotated or a fingerprint check failed. whisper and device handoff
   are paused until you compare again.`
4. CTAs: `compare now` (primary) / `dismiss for now` (secondary).
5. Tap `compare now` → Flow 2 opens for this peer.
6. Match → peer returns to verified; banner auto-dismisses; whisper
   and handoff re-unlock.
7. Mismatch → peer stays unverified; banner remains; gates stay
   paused.

**Success.** Peer re-verified; banner gone; whisper and handoff
restored.

**Abort / alternate paths.**
- `dismiss for now` hides banner for 24 h only (§ Rules). Unverified
  badge on every surface remains.
- Cannot be permanently dismissed without comparing.
- Idempotent: repeat rotations re-render the same banner with updated
  copy.

**Copy gates.**
- `Downgrade.title`: `keys changed — verify again`
- `Downgrade.body`: `this peer's key rotated or a fingerprint check
  failed. whisper and device handoff are paused until you compare
  again.`
- `Downgrade.cta`: `compare now`
- `Downgrade.dismiss`: `dismiss for now`

**Inter-spec hand-offs.**
- Banner placement on profile card: `profile-card.md` reserves "below
  the crest"; `trust-verification.md` places it there. **OK.**
- Whisper gating: `whisper-mode.md` § Trust gating honours unverified
  state. **OK.**
- Rendering order when a peer has both a downgrade banner and a
  queued-state chip simultaneously (letter row + profile card) is not
  defined anywhere. **GAP.**

---

## 15. Share a file in a letter

**Goal.** Attach a file in the composer; upload progresses; file
arrives as a card in the thread; peer opens it.

**Entry.** Composer inside a letter thread (`letters-dms.md`) or a
channel (`composer.md`).

**Sequence.**
1. Tap composer attach (`plus`) IconBtn (`composer.md` § Composer).
2. **Upload dialog** — opens a native file picker (in-app surface not
   specified). **GAP**: no spec owns the upload surface, drag-drop,
   preview-before-send, per-upload progress, cancel, retry, failure,
   or size caps.
3. On send, file renders as a **file card** (`files-inline.md` § Inline
   artefacts § File card): mime icon, filename, size + mime hint,
   `download` IconBtn. Max 420 px desktop / 100% mobile.
4. Files > 10 MB: `large · downloads on click` warning badge.
   Images > 4 MB degrade to file card.
5. Peer taps `download` / card → file opens. **GAP**: open-file
   behaviour (new tab vs in-app preview vs system default vs
   download-to-disk) is not specified.

**Success.** File delivered as a card; peer can download.

**Abort / alternate paths.**
- Inline image caption: `filename · size · e2e encrypted`.
- Failed image load falls back to file card.
- Offline behaviour for files is not specified (Flow 7 conditions).
  **GAP.**

**Copy gates.**
- Image caption: `filename · size · e2e encrypted`
- Large-file badge: `large · downloads on click`
- Attach ARIA: `attach file`

**Inter-spec hand-offs.**
- Composer attach → upload: `composer.md` names the button; upload
  surface is unowned between picker and inline render. **GAP.**
- Letter composer reuses `composer.md` composer: named in
  `letters-dms.md`. **OK.**
- File card rendering: `files-inline.md`. **OK.**
- File-open behaviour: unowned. **GAP.**
- Governance → Files (`governance.md` § Manage → Files) covers grove
  files only; letter files have no management surface. **GAP (soft).**

---

## Gaps / unowned hand-offs

Hand-offs below are not fully owned on both sides. Treat each as a
spec bug to fix in a follow-up pass.

1. **First-grove / join preview surface** (Flows 1, 5). Compact
   preview of an invited grove (name, tagline, member count, inviter
   avatar + verified badge) is defined only in `onboarding.md` §
   Step 5 Card B. `letters-dms.md` and `discover.md` delegate to "the
   existing `JoinPage` flow" without owning the preview.

2. **Inbound letter-of-introduction body** (Flow 5). Neither
   `letters-dms.md` nor `discover.md` describes how the inbound
   letter-of-introduction renders inside a letter thread — the embed
   shape, the `join now` CTA layout, what sits above the embed.
   `discover.md` § Invitation response names the hand-off in one
   sentence only.

3. **Whisper target post-handoff** (Flow 4). `device-handoff.md` §
   Whisper interaction owns the source side. `whisper-mode.md` does
   not describe what the target device's whisper controls look like
   immediately post-commit, nor restates that whisper keys travel
   with the move.

4. **Chrome-stacking order** (Flow 7). `sync-queue.md` places the
   status strip "top of app chrome" but `layout-primitives.md` does
   not reserve the slot. Stacking order of status strip + downgrade
   banner + welcome-back banner is not defined.

5. **Trusted fingerprint book** (Flow 8). `letters-dms.md` § Compose
   surface mentions "trusted fingerprint book" but no spec defines
   its storage model, entry surface, editing, or removal.
   `trust-verification.md` § Data dependencies does not own it.

6. **Conversion warning remote echo** (Flow 9). `letters-dms.md` §
   Converting 1:1 → group says the warning is shown to both peers
   with the remote copy "triggered by the membership event". No spec
   defines the remote delivery mechanism or whether the remote side
   is blocking or informational.

7. **Pinned-messages panel interior** (Flow 11). `reactions-pins.md` §
   Pins defers it; `layout-primitives.md` owns the header icon but
   not the panel. The not-yet-written `reactions-pins.md` (referenced
   by `message-row.md`) is the natural home.

8. **Discover exclusion of ephemerals** (Flow 6).
   `ephemeral-channels.md` § Discovery policy forbids ephemerals
   from appearing in Discover, but `discover.md` does not mirror
   the rule.

9. **Governance → Reports tab** (Flow 13). `discover.md` § Safety /
   reporting routes reports to `governance → reports`, but
   `governance.md` defines only `roles & permissions`, `invites`,
   `files` tabs (plus Event log). A `reports` tab is missing.

10. **Downgrade + queued stacking** (Flow 14). Neither
    `trust-verification.md` nor `sync-queue.md` defines rendering
    order when a peer has both a downgrade banner and queued state
    simultaneously (letter row + profile card).

11. **File-upload surface** (Flow 15). `composer.md` § Composer
    names the attach button; no spec owns the upload dialog,
    progress, cancel, retry, failure copy, or size caps. The
    not-yet-written `files-inline.md` (referenced by
    `message-row.md`) is the natural home.

12. **File-open behaviour** (Flow 15). The file card renders a
    `download` IconBtn but no spec defines tap-open behaviour (new
    tab, in-app preview, system default, disk download).

13. **Letter-files management surface** (Flow 15 soft).
    `governance.md` § Manage → Files covers grove files only. There
    is no per-letter files list / search / revoke surface.

14. **Onboarding → first-grove routing** (Flow 1 soft).
    `onboarding.md` § Step 6 routes into a grove via
    `layout-primitives.md`, but the grove-rail → first-channel entry
    from onboarding is not named in `layout-primitives.md`.
