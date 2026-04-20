# Settings + Tweaks — account, device, privacy, notifications, and the live accent/density panel

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:** [`foundation.md`](foundation.md),
[`layout-primitives.md`](layout-primitives.md),
[`profile-card.md`](profile-card.md),
[`trust-verification.md`](trust-verification.md),
[`device-handoff.md`](device-handoff.md),
[`whisper-mode.md`](whisper-mode.md)

## Purpose

Settings and Tweaks are the two places a user configures Willow. They are
deliberately split:

- **Settings** is the *identity-scoped* surface. It edits fields that
  gossip with your profile (display name, pronouns, bio, crest, pinned
  fragment, elsewhere) and fields *true of this device* that need careful
  confirmation (linked devices, export identity, forget this device).
  Modal on desktop, full-screen on mobile; left-nav menu (desktop) or
  stacked sheet (mobile).
- **Tweaks** is the *device-scoped* visual panel. It writes only the
  runtime tokens declared in `foundation.md` (`--moss-0..4`, `--willow`,
  `--msg-pad`) plus a handful of behavioural flags (`crypto-visibility`,
  `sidebar-variant`, `call-layout`, `show-wordmark`). Persists per device
  in `localStorage`. Right-side popover on desktop; bottom sheet on
  mobile. Changes apply instantly — no save button.

Settings changes are *promises to other peers* and travel with your
identity. Tweaks changes are *preferences on this screen* and never
leave it.

## Scope

In scope: account profile, identity + devices, privacy, notifications,
network, appearance (embeds Tweaks), about; the Tweaks panel (accent,
density, crypto visibility, call layout, sidebar, show wordmark); entry
points (grove-rail gear, mobile "you" tab, wordmark-corner tweak
affordance, Settings → appearance on mobile); accessibility (menu
semantics, landmarks, explicit switches, radiogroups, focus-trapped
confirms for destructive flows).

Out of scope: grove-scoped settings (see [`governance.md`](governance.md));
per-channel crypto configuration (see
[`trust-verification.md`](trust-verification.md)); the iframe
`__edit_mode_*` protocol (kept only behind a `design-preview` feature
flag); remote accent sync across devices under one identity (open
question).

## Settings panel

### Surface

Desktop: centred modal, minimum 960 × 640 CSS px, `--bg-1` with `--line`
border and `--shadow-2`. Left nav is 232 px; remaining space is content.
Closes on `Esc` and on the `x` button in the top-right.

Mobile: full-screen screen pushed in from the right. Top bar per
`layout-primitives.md`: back chevron, title `settings`, subtitle
`you · {device-name}`. Rows at the top level; tapping a row pushes a
child screen.

### Left nav (desktop)

ARIA `menu` with grouped `menuitem` children. Groups use the foundation
meta label. Active item: `--bg-3` background, `--ink-0` text. Groups, in
order:

| Group         | Items |
|---------------|-------|
| **account**   | profile, identity + devices |
| **privacy**   | privacy, notifications |
| **app**       | network, appearance, about |

Each item: icon (14 px, `--moss-3` inactive, `--ink-0` active) + label.
Icons: `user`, `key`, `shield`, `bell`, `signal`, `spark`, `leaf`.

### 1. Profile

Title: `profile`. Subtitle: `profile changes sync to peers you're
verified with first.`

Fields, grouped in cards (`--bg-1`, `--line`, rows separated by
`--line-soft`):

- **display name** — single-line text, max 64 chars.
- **pronouns** — single-line text, max 24 chars. Placeholder: "she/her,
  they, he/him — anything you want".
- **private nickname visibility** — select: "everyone" / "verified peers
  only" / "no one". Default visibility of the per-peer nickname from
  `profile-card.md`. Default: "verified peers only".
- **bio** — textarea, 3 rows, max 280 chars. Fraunces italic placeholder:
  "a sentence or two about you".
- **tagline** — single-line, max 60 chars. Rendered beside display name
  in profile popover.
- **crest pattern** — radiogroup chips: the six patterns from
  `profile-card.md` (leaf, weave, wave, stipple, rings, grid). Chip shows
  a 28 × 28 preview swatch in `--moss-2` on `--bg-2`.
- **crest accent colour** — radiogroup of the seven accent names (moss,
  willow, amber, dusk, cedar, lichen, ember). Scoped to your crest only;
  does *not* rewrite app tokens. Lets verified peers distinguish you
  even when they share an app accent.
- **pinned fragment** — single-line, max 120 chars, Fraunces italic.
  Rendered as a quote in the profile popover.
- **elsewhere** — list of freeform strings (each ≤ 40 chars, up to 6
  rows). No URL validation; rendered as plain text. First-row placeholder:
  "signal, bandcamp, a handle somewhere, whatever you want".

Save button, bottom-right of the last card, label `save`. On press,
emits a new profile event (see Data dependencies) that gossips to
verified peers first, then the wider grove. Inline note below: "saved.
peers will see the update as they come online." Button disables until
fields change.

Closing the modal with unsaved input shows a confirm dialog: "discard
unsaved profile changes?"

### 2. Identity + devices

Title: `identity + devices`. Subtitle: "your keys were made on this
device and never leave it. use your fingerprint to verify peers in
person."

**your fingerprint**

- 6-word display grid: six cards, 6 cols on desktop, 3 × 2 on mobile.
  Each card: `--font-mono` 13 px, numeric label 1..6 above the word in
  `--ink-4` 9 px. Background `--bg-2`, border `--line`, radius `--radius`.
- Two buttons under the grid: `copy as text` (icon `link`) and `show qr`
  (icon `spark`). "show qr" opens the SAS dialog from
  `trust-verification.md`.
- Footer, right-aligned, mono 11 px, `--ink-3`:
  `ed25519 · x25519 · chacha20-poly1305`.

**linked devices**

Heading: `linked devices`. Device list rows:

- 34 × 34 icon box (`--bg-2`, `--line`, radius 10, `device` glyph in
  `--moss-3`).
- Name (mono, 12.5–13 px, `--ink-0`). Verified badge per `trust-verification.md`.
- Metadata (11 px, `--ink-3`): "macOS 15 · this device" or "Linux · added
  14 feb · last seen 2h ago".
- Action: *this device* is disabled with `active` in `--ink-3`; *other
  device* shows `revoke` in `--err`. Revoke opens a focus-trapped confirm
  dialog: "revoke {name}? this device's sub-key will be invalidated.
  messages it sent stay readable, but it cannot receive new sealed
  messages."

Below the list, a dashed-border row: icon `plus`, title "add a new
device", hint "scan a qr on the new device · keys stream over local
network". Clicking opens the handoff flow from `device-handoff.md`.

**export identity**

- `export mnemonic` — reveals a 12-word grid in mono. Confirm dialog:
  "write this down somewhere only you can reach. anyone with these words
  can become you."
- `export keyfile` — downloads an encrypted keyfile. If no local
  passphrase exists, a secondary dialog prompts: "set a passphrase for
  your keyfile" with helper: `this passphrase only protects your
  keyfile; willow never sees it.` Minimum 12 chars; zxcvbn strength
  hint shown.

**forget this device**

Button, `--err` outline, label `forget this device`. Confirm dialog:

- With other linked devices: "forget willow·desk? its sub-key stops
  being trusted in seconds. your identity lives on your other devices."
- If this is the *only* device: hard-blocked. Dialog titled "this is
  your only device" with copy `you'll lose access to your identity —
  export first.` Only buttons: "close" and outlined "export mnemonic".
  No escape hatch.

### 3. Privacy

Title: `privacy`. Subtitle: "defaults apply to new conversations. any
grove or letter can override these per-room."

| Row label                                      | Control | Default | Notes |
|------------------------------------------------|---------|---------|-------|
| read receipts default                          | switch  | off     | Per-letter override in profile popover. |
| typing indicators default                      | switch  | on      | Per-grove override in grove settings. |
| crypto visibility: subtle · default · explicit | radiogroup | default | Drives holder pills, key-rotation banners, SAS nudges per `trust-verification.md`. |
| allow whispers from unverified peers           | switch  | off     | Blocks whisper invitations from unverified peers. |
| local search index                             | switch  | on      | Indexes decrypted bodies locally; never leaves device. Off deletes the index. |

Crypto-visibility is a segmented chip control (mono 11 px, same as
Tweaks). Footer hint: "these defaults are stored with your profile and
apply on every device."

### 4. Notifications

Title: `notifications`. Subtitle: "per-surface, with a quiet-hours
schedule and a gentle chime."

Per-surface switches (all default on):

| letters | mentions | ephemeral expiration | whisper invitations | handoff requests |

Quiet-hours card:

- Switch `quiet hours` (default off).
- When on: start and end time pickers render inline; summary reads
  `quiet hours: {start}–{end}` (mono numbers, U+2013 en-dash).
- Weekday selector (toggle chips mon..sun).

Sound picker card:

- Switch `willow chime` (default on).
- When on: `preview` button plays current chime; radiogroup
  `choose sound` — "willow" (default) · "stone" · "silent-thud".

Footer link: "manage per-grove in grove settings" in `--moss-3`.

### 5. Network

Title: `network`. Subtitle: "willow prefers direct peer connections.
relays are only used when no hole-punch is possible."

- **relay URL override** — text input. Empty means default. Save
  triggers reconnect; inline note "reconnected to {relay}" in `--moss-3`
  on success, `--err` on failure.
- **connection quality sampling** — switch (default on). Measures RTT to
  peers; surfaces in profile popover.
- `allow direct peer connections only` — switch (default off). When on,
  relay is skipped even when hole-punch fails; conversations fall into
  the sync queue per `sync-queue.md`.

Footer: "traffic through the relay is still end-to-end encrypted. the
relay only sees ciphertext."

### 6. Appearance

Title: `appearance`. Subtitle: "these settings apply to this device.
they don't travel with your identity."

Embeds Tweaks inline — identical controls, identical persistence
(localStorage), identical instant preview. Renders as Settings card
rows (`--line-soft` separators) rather than the floating popover.

Each row: label (`--ink-0`, 13.5 px) + control + hint (`--ink-3`, 11.5
px). Hints:

- accent — changes the green. whisper violet stays the same.
- density — message spacing only.
- crypto visibility — where holder pills and rotation banners appear.
- call layout — starting layout for new calls.
- sidebar — channel list styling.
- show wordmark — tiny willow mark in the corner.

Footer link (desktop only): "find the same controls in the tweak corner".

### 7. About

Title: `about`. Rows:

- **version** — mono, `--ink-1`.
- **licence** — text "AGPL-3.0" + link "read the licence".
- **links** — no trackers: "source code", "homepage", "issues". Each
  opens in a new tab.
- `what's new` — opens a side-drawer with release notes.
- Build string, mono, `--ink-3`: "willow · {version} · peer-to-peer ·
  no server knows your plaintext".

## Tweaks panel

### Surface

Desktop: fixed popover, 320 × auto, positioned bottom-right of viewport
(`right: 16`, `bottom: 16`), `--bg-1`, `--line` border, `--radius-l`,
`--shadow-2`, 14 px internal padding.

Header row (display italic 15 px): `"tweaks"` with a `spark` icon on
the left and a mono `"willow"` label on the right in `--ink-3`.

Below the header, a 11 px `--ink-3` description line:
"live preview. changes apply instantly and persist on this device."

Mobile: bottom sheet. Height auto-fits content. Handle at the top.
Controls and copy identical to desktop.

### Controls

Each control is a row:

- Label (left, 12 px, `--ink-1`, lowercase).
- Control (right, segmented chips or radio swatches).
- Row separator: `--line-soft`.

All controls are radiogroups. Changes write immediately.

**accent** — radiogroup of 7 chips. Each chip is a colour swatch (24 ×
24 rounded square, filled with the variant's `--moss-2` colour) above a
mono label. Options, in this exact order:

> moss · willow · amber · dusk · cedar · lichen · ember

Selecting a variant rewrites the following properties on
`document.documentElement.style`:

- `--moss-0`
- `--moss-1`
- `--moss-2`
- `--moss-3`
- `--moss-4`
- `--willow`

Values come from the accent variant table in `foundation.md`. No other
tokens are touched. Whisper violet (`--whisper`) is untouched and must
remain distinguishable under every variant.

**density** — radiogroup of 3 chips.

> cozy · balanced · dense

Selecting writes a class on `#app-root`: `density-cozy`,
`density-balanced`, or `density-dense`. Each class maps to a
`--msg-pad` value per `foundation.md` density table. Other measurements
(sidebar, header, thread pane widths) stay constant.

**crypto visibility** — radiogroup of 3 chips.

> subtle · default · explicit

Stored in localStorage as `tweaks.cryptoVisibility`. Read by
`trust-verification.md`, `whisper-mode.md`, `message-row.md`, and
`composer.md` to decide
whether to render holder counts, key-rotation banners, SAS nudges, and
"sealed to N" footers. This spec does not describe *what* those surfaces
look like — only the persisted preference key.

**call layout** — radiogroup of 3 chips.

> grove · grid · focus

Default starting layout for new calls. Stored as `tweaks.callLayout`.
Read by `call-experience.md`.

**sidebar** — radiogroup of 3 chips.

> stems · reeds · letters

Visual-only variant for channel sidebar row styling. All variants render
the same information — only typography, spacing, and row adornments
differ (per `layout-primitives.md`). Stored as `tweaks.sidebarVariant`.
Default: `stems`.

**show wordmark** — switch (on / off).

When on, a tiny `willow` wordmark is rendered at the bottom-left corner
of the viewport (desktop) or hidden entirely on mobile regardless of the
setting. Stored as `tweaks.showWordmark`.

### Persistence

Tweaks are stored per-device in `localStorage` under a single key
`willow.tweaks` as a JSON object:

```json
{
  "accent": "moss",
  "density": "balanced",
  "cryptoVisibility": "default",
  "callLayout": "grove",
  "sidebarVariant": "stems",
  "showWordmark": true
}
```

On app boot, this object is read and applied before the first paint
(inline `<script>` in `index.html`, identical pattern to the design
prototype's `tweakable-defaults` element) to avoid FOUC. If the key is
missing or malformed, defaults from the table above are used.

Settings (sections 1–5, 7) persist per-identity. The profile-scoped
fields gossip with the next profile event. Device list changes emit
`LinkDevice` / `RevokeDevice` events (new EventKinds — see Data
dependencies). Privacy, notifications, and network preferences store in
a per-identity settings document that syncs alongside the profile but
is readable only by the identity's own devices.

### Instant preview

There is no save button in Tweaks. Every change applies within one
animation frame. Settings → Appearance (which embeds Tweaks) also has
no save button — Tweaks-style controls apply instantly regardless of
where they're rendered.

### Mobile vs desktop

| Aspect            | Desktop                                    | Mobile |
|-------------------|---------------------------------------------|--------|
| Entry             | Circular "tweak" button in the wordmark corner | Settings → appearance (section 6) |
| Surface           | Fixed popover bottom-right                  | Bottom sheet, handle, swipe-down to close |
| Width             | 320 px                                      | 100% viewport, max height 70 vh |
| Dismiss           | Click-away, `Esc`                           | Tap backdrop, swipe-down, back gesture |
| Wordmark corner   | Visible when `showWordmark` is on           | Always hidden (no room) |

## Entry points

- **Desktop Settings:** gear icon in the grove rail footer, bottom-left
  of viewport. Icon: `settings`, 17 px, `--ink-2` default, `--ink-0`
  on hover / active. `aria-label="settings"`.
- **Mobile Settings:** "you" tab in the bottom tab bar (per
  `layout-primitives.md` mobile tab set). Icon + label. Tapping opens
  the full-screen Settings screen.
- **Desktop Tweaks:** a small 32 × 32 circular button in the bottom-left
  corner of the viewport, overlapping the wordmark area. Icon: `spark`,
  14 px, `--ink-3`. Click toggles the popover.
- **Mobile Tweaks:** reachable via Settings → appearance only. There is
  no corner affordance on mobile.
- **Design-preview builds only:** iframe message protocol
  (`__activate_edit_mode` / `__deactivate_edit_mode` /
  `__edit_mode_set_keys`) gated behind a `design-preview` cargo feature.
  Not shipped to production; not covered by acceptance criteria.

## Mobile vs desktop

Beyond the Tweaks differences noted above, Settings itself differs:

| Aspect            | Desktop                                  | Mobile |
|-------------------|-------------------------------------------|--------|
| Surface           | Modal, ~960 × 640                          | Full-screen screen push |
| Nav               | Left sidebar menu, active-row highlight    | Top-level list; tapping pushes a child screen |
| Section layout    | Centred content pane, max-width 720 px      | Stacked sheet rows, 16 px padding |
| Save affordance   | Save button per section (profile)          | Sticky bottom "save" bar on profile screen |
| Close             | `x` button top-right, `Esc`                | Back chevron top-left |
| Destructive flows | Confirm dialog, focus-trapped              | Full-screen confirm sheet, focus-trapped |
| Quiet-hours pickers | Inline time pickers                       | Native time input, one at a time |

## Copy (exact)

The following strings must appear verbatim where indicated. All other
copy in this spec is normative but editable within the copy voice rules
in `foundation.md`.

- Titles: `settings`, `tweaks`.
- Profile subtitle: `profile changes sync to peers you're verified with
  first.`
- Fingerprint action: `show qr`.
- Devices heading: `linked devices`.
- Forget-device button: `forget this device`.
- Forget-only-device copy: `you'll lose access to your identity — export
  first.`
- Export passphrase helper: `this passphrase only protects your keyfile;
  willow never sees it.`
- Crypto-visibility row label: `crypto visibility: subtle · default ·
  explicit`.
- Whisper-unverified switch: `allow whispers from unverified peers`.
- Quiet-hours summary: `quiet hours: {start}–{end}` (en-dash, numbers
  in mono).
- Chime toggle: `willow chime`.
- Network strict-mode switch: `allow direct peer connections only`.
- About link: `what's new`.
- Tweak labels: `accent`, `density`, `crypto visibility`, `call layout`,
  `sidebar`, `show wordmark`.
- Accent option names, in order: `moss · willow · amber · dusk · cedar ·
  lichen · ember`.

## Data dependencies

### Existing

- **profile event** — the current `Profile` event (display name,
  avatar) is extended. The extension adds optional fields below.
- **EventStore gossip** — profile events gossip per-peer using existing
  machinery.

### New (scoped to this spec)

- **Profile fields** — pronouns (string), bio (string ≤ 280), tagline
  (string ≤ 60), crest pattern (enum of 6), crest accent (enum of 7),
  pinned fragment (string ≤ 120), elsewhere (Vec<String>, up to 6, each
  ≤ 40). These travel as additional fields on the existing profile
  event variant.
- **Device list + link event** — two new `EventKind` variants:
  `LinkDevice { device_id, device_name, sub_key_pub }` and
  `RevokeDevice { device_id }`. Devices are sub-keys of the root
  identity (see `device-handoff.md` for the existing handoff flow).
  Revocation invalidates the sub-key; future sealed messages to the
  identity do not include the revoked device.
- **Per-identity settings document** — a new small CRDT or
  last-writer-wins doc keyed on identity that stores:
  `readReceiptsDefault`, `typingDefault`, `cryptoVisibilityDefault`,
  `allowUnverifiedWhispers`, `localSearchIndex`, plus the notification
  sub-surface toggles and quiet-hours schedule. This gossips between
  an identity's own devices and is not exposed to peers.
- **Crypto-visibility preference** — `tweaks.cryptoVisibility` on this
  device *overrides* the per-identity default from Settings →
  Privacy. This is intentional: the device sets how loud crypto is on
  its own screen; the identity-scoped setting is the default for new
  devices.
- **Quiet-hours schedule** — stored in the per-identity settings doc:
  `{ enabled: bool, start: "HH:MM", end: "HH:MM", days: [mon..sun] }`.
  Consumed by the notification system.

### Consumed by other specs

- `profile-card.md` reads the new profile fields.
- `trust-verification.md` reads `cryptoVisibility` and
  `allowUnverifiedWhispers`.
- `whisper-mode.md` reads `allowUnverifiedWhispers`.
- `device-handoff.md` reads the linked devices list.
- `call-experience.md` reads `callLayout`.
- `layout-primitives.md` reads `sidebarVariant` and `showWordmark`.

## Edge cases

- **Export with no local passphrase set.** Prompt the user to set one.
  Copy: `"this passphrase only protects your keyfile; willow never sees
  it."` Passphrase minimum 12 chars. Once set, the passphrase is stored
  in an OS keyring on native builds and in `IndexedDB` (non-sync,
  device-scoped) on the web. It is never transmitted.
- **Forget the only device.** Hard-blocked. Confirm dialog cannot be
  dismissed into destructive action. Copy: `"you'll lose access to your
  identity — export first."` Only buttons: "close" and an outlined
  "export mnemonic" that routes to the export flow.
- **Relay URL override saved while offline.** The input accepts the
  value and queues a reconnect for when the network is reachable. A
  `--warn` note reads "saved — will reconnect when you're online".
- **Accent chosen while a colour-dependent surface is open** (e.g. a
  SAS dialog). Colours swap mid-dialog. This is acceptable; the SAS
  words themselves are not accent-dependent.
- **Density change while message list is scrolled.** Scroll anchor pins
  to the most-recently-visible message; `--msg-pad` swap must not push
  the reading position.
- **Tweaks applied before authentication.** Tweaks live in local storage
  under a fixed key; no identity is required. If the user signs out and
  in as a different identity on the same device, Tweaks persist. This
  is intentional — Tweaks describe the device, not the identity.
- **Settings opened while a profile gossip is in flight.** The form
  shows the locally-persisted values; when the in-flight save lands,
  inputs rebase onto the new server state unless the user has modified
  them (then we flag a conflict: "another device changed your profile
  — reload / keep mine").
- **`prefers-reduced-motion: reduce`.** The Tweaks popover open/close
  animation collapses to opacity. Chip selection highlight skips the
  slide and swaps instantly.
- **Remote accent sync across devices under one identity.** Deferred.
  Tracked under Open Questions. Today, accent is device-local only.

## Accessibility

- **Settings nav is a menu.** Root element has `role="menu"` with
  grouped `role="menuitem"` children. Each group has a visible label
  and `role="group"` with `aria-labelledby` pointing to the label
  element. Arrow keys move between items; `Home` / `End` jump to ends.
  `Enter` or `Space` activates.
- **Each section is a landmark.** Content pane renders each section
  inside `<section aria-labelledby="section-{id}-title">` with the
  title as `<h2 id="section-{id}-title">`. Screen readers announce the
  section as a landmark.
- **Toggles are explicit switches.** Rendered with `role="switch"` and
  `aria-checked`. The visual knob moves on state change but is decorative;
  the switch's accessible name is the row label. Hint text is announced
  via `aria-describedby`.
- **Accent chips are a radiogroup.** Container has `role="radiogroup"`
  with `aria-label="accent"`. Each chip has `role="radio"`,
  `aria-checked`, and an accessible name of the variant name ("moss",
  "willow", etc.). The coloured swatch is `aria-hidden="true"` — the
  name carries the meaning. Arrow keys move within the group; `Tab`
  moves out.
- **Density is a radiogroup** with the same pattern.
- **Crypto visibility, call layout, sidebar** are radiogroups with the
  same pattern.
- **Show wordmark** is a single switch.
- **Destructive confirm dialogs are modal and focus-trapped.** `Esc`
  closes (non-destructive). The destructive button is not the default
  focus; focus lands on the cancel button. The dialog has `role="dialog"`
  with `aria-modal="true"` and a visible title bound via
  `aria-labelledby`. Strong focus-trap: `Tab` cycles within the dialog,
  `Shift+Tab` also cycles. Clicking the backdrop closes only for
  non-destructive dialogs; destructive dialogs require an explicit
  button.
- **Export keyfile passphrase input** is `type="password"`. A "show"
  button toggles visibility; clicking it moves focus back to the input
  and announces "passphrase shown" / "passphrase hidden" via an
  `aria-live="polite"` region.
- **Quiet-hours time pickers** use native `<input type="time">` for
  correct keyboard + touch support. The weekday selector is a
  radiogroup per-day toggle set inside a fieldset with legend "days".
- **Screen-reader labels for icon-only buttons**: `aria-label="settings"`
  on the gear, `aria-label="tweaks"` on the corner spark, `aria-label=
  "close settings"` on the modal x.
- **Focus-visible** uses `--focus-ring` on every interactive element.
  Chips within a radiogroup render the focus ring on the currently
  focused chip, not the selected one.
- **Colour independence.** Accent chip's selected state uses a filled
  ring + checkmark glyph, not just a background shift. Switch on/off
  states use a visible knob position and a state icon (check vs empty
  circle) in addition to colour.
- **Reduced motion.** Modal enter animation collapses to opacity.
  Popover enter collapses to opacity. Switch knob slides are replaced
  with instant swap. `willowPulse` on the "tweak" corner affordance is
  replaced with a static opacity per `foundation.md`.

## Acceptance criteria

- [ ] Settings modal (desktop) opens from the grove-rail gear icon and
      from keyboard `,` when focus is in app chrome. Closes on `Esc`
      and on `x`.
- [ ] Mobile "settings" screen pushes from the right when the "you" tab
      is tapped and returns on back-gesture or back chevron.
- [ ] Left nav (desktop) renders three groups — account, privacy, app —
      with the items listed in section-order.
- [ ] All seven sections render with their required fields. Save
      button on profile emits a profile event with all new fields
      populated.
- [ ] Profile gossip to verified peers precedes gossip to the wider
      grove (semantic requirement on the profile event; verifiable via
      the existing trust tiering in `willow-network`).
- [ ] Fingerprint 6-word grid renders with number labels 1..6 and
      matches the mono style from `foundation.md`. "show qr" button
      opens the SAS dialog per `trust-verification.md`.
- [ ] Revoke-device confirm dialog is focus-trapped. Cancel is the
      default-focused button. Revoke button is `--err`-tinted and emits
      a `RevokeDevice` event.
- [ ] Forget-only-device hard-block shows the "you'll lose access"
      copy verbatim and offers only close / export-mnemonic.
- [ ] Export-keyfile prompts for passphrase-set when no passphrase
      exists on the device, with the verbatim helper copy.
- [ ] Privacy section toggles persist in the per-identity settings doc
      and gossip between own devices only.
- [ ] Crypto-visibility radiogroup row label matches verbatim: `crypto
      visibility: subtle · default · explicit`.
- [ ] Quiet-hours summary renders with `HH:MM`–`HH:MM` in mono and the
      en-dash character (U+2013), not a hyphen.
- [ ] Chime toggle label is `willow chime`.
- [ ] Network strict-mode switch label is `allow direct peer
      connections only`.
- [ ] About section exposes a `what's new` link.
- [ ] Tweaks popover (desktop) opens from the corner "tweak" button and
      closes on click-away and `Esc`.
- [ ] Tweaks bottom sheet (mobile) is reachable only via Settings →
      appearance.
- [ ] Accent change writes exactly `--moss-0`, `--moss-1`, `--moss-2`,
      `--moss-3`, `--moss-4`, and `--willow` on
      `document.documentElement.style` — no other tokens. Whisper
      violet stays visually distinct on all seven variants.
- [ ] Density change writes `density-{mode}` class on `#app-root` and
      the class maps to the right `--msg-pad` value. Scroll anchor is
      preserved across the swap.
- [ ] Tweaks persist per device in `localStorage` under
      `willow.tweaks`. Missing or malformed keys fall back to defaults
      without crashing.
- [ ] Settings → Appearance embeds the same controls as Tweaks and
      writes through to the same `localStorage` key, producing identical
      runtime effects.
- [ ] All radiogroups expose keyboard navigation (arrow keys within,
      `Tab` to move out). Every control has an accessible name matching
      its label.
- [ ] Destructive confirm dialogs (revoke device, forget device, discard
      profile changes) trap focus and close only on explicit button
      activation.
- [ ] Under `prefers-reduced-motion: reduce`, every open/close and knob
      slide collapses to opacity-only.

## Open questions

1. **Remote accent sync under one identity.** Should accent changes
   propagate to all devices attached to the same identity? The design
   bundle notes a future option ("propagated to all my devices"). If
   yes, this becomes an identity-scoped setting in the per-identity
   settings doc. If no, accent stays device-local. Deferred to v1.1.
2. **Per-grove accent override.** The parent `README.md` notes that
   grove-level accent is supported by the bundle. Who writes it —
   grove settings (`governance.md`) or here? Proposed: grove settings
   writes it; this spec does not expose the control. Confirm during
   review.
3. **Keyfile passphrase strength floor.** 12 characters is the current
   floor. Do we require a minimum zxcvbn score as well? Proposed: yes,
   score ≥ 2, but no hard reject — we show the score and let the user
   proceed. Confirm during review.
4. **Local search index wipe confirmation.** Turning off the toggle
   deletes the index. Is an "are you sure" dialog warranted? Proposed:
   a soft inline warning ("your local search index will be deleted")
   plus an undo that restores from re-index. No modal.
5. **Chime asset licensing.** The "willow / stone / silent-thud" set is
   placeholder. Final audio assets and their licences need human sign-off
   before ship.
6. **Notifications permission flow.** When notifications are first
   enabled on web, we need to request the browser's notification
   permission. This spec assumes the existing permission prompt pattern;
   if that pattern is not defined yet, it blocks implementation.
7. **Mobile Tweaks discovery.** With the corner affordance gone on
   mobile, how do we teach users that Settings → Appearance contains
   the same controls? Proposed: no extra UI — the subtitle on
   Appearance mentions "find the same controls in the tweak corner" on
   desktop and simply reads "these settings apply to this device. they
   don't travel with your identity." on mobile.
8. **Identity settings sync between own devices.** The per-identity
   settings doc requires a sync mechanism. The proposed CRDT vs
   last-writer-wins choice belongs in a separate state-layer spec.
   Open until that spec lands.
