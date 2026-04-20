# Trust & verification — SAS, badges, compare flow

**Parent:** [README.md](README.md)
**Status:** draft
**Dependencies:**
- [`foundation.md`](foundation.md) — tokens, typography, motion, copy voice,
  accessibility baseline. All visual values below reference these tokens by
  name; no raw hex appears in this spec.
- [`profile-card.md`](profile-card.md) — *only* as a surface consumer for the
  verified / unverified / pending-verify badge. The SAS fingerprint grid,
  the compare-fingerprints flow, and the holder pill are independent and
  do not depend on the profile-card spec.

**Consumed by:**
- [`profile-card.md`](profile-card.md) renders the verified / unverified /
  pending-verify badge and hosts the `add a friend` dialog entry point.
  This spec owns the SAS grid atom, the badge visual language, and the
  badge state machine.

This spec is **security-critical**. Copy is reproduced *exactly* from the
reference bundle. Badges are **mandatory** on every peer-identifying
surface (profile card, letter row, members list, message author on the
first message of a run, call-participant tile, governance member row).
No surface may soften, hide, or conditionally suppress a badge.

## Purpose

The SAS (Short Authentication String) fingerprint is the Willow user's only
defence against a MITM on their identity key exchange. The UX has three
jobs:

1. **Make verification visible everywhere.** Every peer-identifying surface
   announces *verified* / *unverified* / *verification pending*. Colour is
   never the only signal; badges pair icon shape + screen-reader label.
2. **Make comparison possible without ceremony.** A user who sees an
   unverified badge is one tap / click / Enter away from the comparison
   screen from every surface.
3. **Teach the property, not the crypto.** Copy explains *what verification
   protects* ("no one can impersonate either of you in this conversation,
   ever"), not the mechanism. Lowercase, intimate, per `foundation.md`.

## Scope

In: the SAS fingerprint grid (component, variants, a11y); the compare-
fingerprints flow (entry points, screens, decisions); the three badge
states and their placement on every peer surface; the per-channel
holders pill; downgrade / re-verify banners; long-press SAS on mobile +
keyboard equivalent; exact copy strings.

Out: profile-card chrome (see `profile-card.md` — this spec only owns
the badge *on* the card); the first-SAS ceremony during onboarding (see
`onboarding.md`); whisper / handoff gating — those specs consume the
trust state defined here.

## SAS fingerprint grid — visual spec

The fingerprint grid is a pure, stateless component. It renders identically
on desktop and mobile onboarding, in the compare-fingerprints flow, inside
the profile sheet, and anywhere a full fingerprint is shown. No surface is
permitted to render its own grid; all callers use this component.

### Shape

- Layout: `display: grid; grid-template-columns: repeat(3, 1fr);` — 3
  columns × 2 rows of six words. (Reading order is left-to-right, top-
  to-bottom.)
- Cell count: always 6. Fewer is not a valid state; while the session
  key is deriving, render a `shimmer`-keyframe skeleton.
- Cell contents, in reading order:
  1. 1-indexed numeral (`1`..`6`) — `mono S` 10.5 px `md` / 9 px `sm`,
     colour `--ink-4`.
  2. Lowercase word — `mono L` 14 px `md` / 12 px `sm`, colour `--ink-0`.
- Cell chrome: `--bg-2` background; 1 px `--line` border; radius 8 px
  (proposed `--radius-cell` in `foundation.md`). Padding `10px 12px` on
  `md`, `8px` on `sm`. Cell gap 6 / 5 px; number-word gap 6 / 4 px.

### Size variants

| Variant | Use                             | Word type | Number type | Cell pad   |
|---------|---------------------------------|-----------|-------------|------------|
| `md`    | Desktop onboarding, desktop compare flow, desktop profile card expanded fingerprint | `mono L` 14 px | `mono S` 10 px | `10px 12px` |
| `sm`    | Mobile onboarding, mobile compare sheet, mobile profile sheet expanded fingerprint | `mono L` 12 px | `mono S` 9 px  | `8px`       |

No large variant; surfaces add whitespace, not scale. Arbitrary zoom
breaks the comparison rhythm.

### State tint variants

The grid has four tint states. State is driven by the caller via a
`variant` prop, not by internal logic.

| Variant         | Where it's used                         | Tint rule |
|-----------------|-----------------------------------------|-----------|
| `you`           | "your fingerprint — read this aloud"    | Left rule: 2 px `--moss-2`; cells keep `--bg-2`. Label above uses `--ink-2`; the secondary label strip above cells uses `--moss-2` text. |
| `peer`          | "their fingerprint — do these match?"   | Cells keep `--bg-2`; label above uses `--ink-2` (neutral until decided). |
| `matched`       | Applied *after* the user taps "they match" | Border on each cell becomes 1 px `--ok`; card border becomes 1 px `--moss-2`; a `Check` icon appears next to the section label. |
| `mismatch`      | Applied after "they don't match"        | Border on each cell becomes 1 px `--warn` (dashed, 1.5 px); card border becomes 1 px `--warn` dashed; a `Shield` icon with the `--warn` colour appears next to the section label. |

Tint changes animate `--motion` (180 ms, ease-out willow) on border-
colour and icon opacity only. Cell width / word position never animate —
a jumpy grid during a security decision is a UX failure.

### Section label above the grid

Rendered above every grid via a shared `FingerprintLabel` component.

| Variant | String |
|---------|--------|
| `you`   | `your fingerprint — read this aloud` |
| `peer`  | `their fingerprint — do these match?` |

Style: type role `meta` (`IBM Plex Sans` 11.5 px / 500 / uppercase,
letter-spacing +1.2), colour `--ink-3` on `md`, 10 px on `sm`. Margin-top
14 px on `md`, 12 px on `sm`. The em dash is intentional copy.

### Semantics (accessibility)

The grid is keyboard-inert but screen-reader-readable. Structure:

Grid uses `role="table"` with `aria-label="your six-word fingerprint"`;
rows are `role="row"`, cells `role="cell"` with
`aria-label="word {n}, {word}"` — number first so a screen-reader user
can cross-check position and word in lockstep with the visual reader.
The numeral inside each cell is `aria-hidden="true"` so it isn't re-read.

## Compare-fingerprints flow

A single flow covers every compare-fingerprints entry point. On desktop
it opens as a centred modal dialog; on mobile it opens as a full-height
bottom sheet that uses `--motion-slow` (240 ms) to rise from below.
The dialog is titled `add a friend` in the UI (verbatim from the
reference bundle's `SAS_COPY.title`); the internal name for the flow
is the *compare-fingerprints* flow, and this spec uses both terms
interchangeably where the context is clear.

### Entry points

All three of the following must exist and route to the same flow:

1. **Profile card — peer variant, secondary action row.** The label is
   `compare fingerprints` (no icon needed; inline with copy-fingerprint).
   See `profile-card.md` for the row; this spec only reserves the slot.
2. **Letter row — unverified peer.** Show a small chip at the right edge
   of the row with label `compare →`, background `--bg-2`, border
   `--line`, text `--warn`. Width is intrinsic; the chip is never the
   full width of the row. On mobile, tapping either the chip *or*
   long-pressing the avatar opens the same flow.
3. **Pending-verify peer in onboarding.** The onboarding flow (`onboarding.md`)
   forwards its "compare" CTA into this flow and consumes the decision.

Keyboard entry: every surface that shows the unverified or pending-verify
badge must accept `Enter` when the badge has keyboard focus. Focus opens
the compare-fingerprints dialog scoped to the peer whose badge was
focused.

### Screen 1 — compare

Two cards side-by-side on desktop (`grid-template-columns: 1fr 1fr;
gap: 20px`); stacked on mobile with the **peer card first** and own
card below. Rationale: mobile users hold the phone up to the other
person; peer words on top keeps both grids in the upper half above the
thumb.

**Card 1 — you:** avatar (36 `md` / 32 `sm`) + device label
(`you · willow·desk` / `you · willow·phone`) + meta
`SAS_COPY.youMeta`. `FingerprintLabel which="you"` +
`FingerprintGrid variant="you"`.

**Card 2 — them:** peer avatar + name + arrival-channel meta
(`SAS_COPY.peerMeta` = `arrived via nearby share`, or letter-of-
introduction / join-code / relay-bridge variant). `FingerprintLabel
which="peer"` + `FingerprintGrid variant="peer"` (becomes `matched` or
`mismatch` after the decision).

CTAs on the peer card:

- Primary `they match` — `--moss-2` bg, `#14130f` text, `Check` 14 px
  stroke 2.2. Full-width on mobile; inline on desktop.
- Secondary `they don't match` — transparent bg, `--line` border,
  `--ink-2` text. Full-width below primary on mobile.
- **Reserved slot** for `not sure` — rendered only when feature flag
  `V1_ALLOW_UNSURE_CTA` is set. See open questions; the slot exists so
  enabling it later doesn't rework the sheet.

**Reassurance footer**, below both cards: `Shield` icon (16 px, 1.6
stroke, `--ink-3`) + `SAS.reassurance` copy at 12 px `--ink-3`, line-
height 1.6, max-width 720 px desktop.

### Screen 2 — confirm match

Triggered by tapping `they match`. A full-screen confirm on mobile; an
in-place card replacement on desktop (the peer card becomes the confirm
card; the own card stays as reference).

- Title, `display M` italic: `verified.`
- Body, 14 px, `--ink-1`, max-width 520 px:
  > verified peer — this cannot be silently downgraded by an attacker.
  > their key is pinned; if it ever changes you'll be asked to verify
  > again.
- State change emitted to client: `client.mark_verified(peer_id)` (a
  new API — see "Data dependencies" below).
- CTA: `done` — closes the sheet. Default focus.
- Secondary: `undo` — reverts the peer to unverified. Present for 10
  seconds with a toast; after 10 s only the profile page can undo.

### Screen 2b — confirm mismatch

Triggered by tapping `they don't match`.

- Title, `display M` italic: `marked not verified.`
- Body, 14 px, `--ink-1`, max-width 520 px:
  > marked not-verified — we will keep this peer unverified until you
  > compare again. you can still send messages, but whisper and device
  > handoff stay closed until the fingerprints match.
- State change: `client.mark_unverified(peer_id, reason:
  SasMismatch)`; peer stays in the contact list and can still receive
  text but whisper / handoff / any future "ceremony-gated" flow is
  disabled. No auto-block.
- CTA: `compare again` — loops back to Screen 1 with fresh words.
- Secondary: `close` — dismisses.

**The flow never deletes messages, never unfriends the peer, and never
calls out to the network.** All state lives in the client's local trust
store (see "Data dependencies").

### Focus, escape, non-blocking

- `role="dialog"` + `aria-modal="true"` + focus trap. First focus on the
  primary CTA (`they match`). The secondary `they don't match` is never
  auto-focused — accidental Enter on a security decision is a failure.
- ESC dismisses from any screen. On Screens 2/2b, ESC = `done` (persists
  the decision); it is **not** equivalent to `undo`.
- No background-click dismiss on desktop.
- Non-blocking: ongoing messaging continues while the dialog is open.

## Badges

Three visual states, surfaced on **every** peer-identifying surface
listed under Scope. Implementers must audit new surfaces for badge
presence before shipping.

### verified

- Disk: filled `Check` 10 px, stroke 2.4, on a 14 px `--moss-1` disk.
- `aria-label="verified peer"`; tooltip `verified peer`.
- On profile card crest: pill variant — `3px 8px 3px 7px`, radius 999,
  bg `color-mix(in oklab, var(--bg-0) 60%, transparent)`, `--moss-3`
  text, `mono S`, `backdrop-filter: blur(6px)`; icon + word `verified`.

### unverified

- Disk: 14 px dashed 1.5 px `--warn` ring around a `?` glyph
  (`mono M`, `--warn`).
- `aria-label="unverified — compare fingerprints"`; tooltip
  `unverified — compare fingerprints before you trust this peer`.
- Profile crest pill: `Shield` icon + word `unverified`, `--warn`.

### pending-verify

- Glyph: `?` in `--warn` (no ring) + inline `compare →` chip to the
  right of the peer name. Chip: `3px 8px`, radius 999, bg `--bg-2`,
  border `--line`, text `--warn`, `mono S`, chevron `Arrow` 10 px.
- `aria-label` on the glyph:
  `verification pending — compare fingerprints`.
- Chip is a real `<button>` (focusable, Enter activates) — opens the
  compare-fingerprints flow.

### `new peer` edge case

A peer with no prior messages and no SAS attempt renders `new peer`
(label only, `--ink-3`, no icon) *instead of* the unverified amber, for
the first interaction only. Once any message is exchanged, the label
collapses to the normal unverified badge. This avoids the amber reading
as a warning against a peer the user has not yet made any decision about.

### Placement rules by surface

| Surface | Badge placement | Size |
|---------|-----------------|------|
| Profile card (hero crest) | Absolute, top: 10 px, left: 12 px, over crest banner | Pill |
| Profile card (compact header inside letter sheet) | Inline after the display name | Disk (14 px) |
| Letter row in sidebar | Right end of row, before timestamp | Disk (14 px) or pending chip |
| Members list row | After display name, before role chip | Disk (12 px) |
| Message author (first message of a run) | Directly after author display name | Disk (12 px) |
| Call participant tile | Top-left corner, 6 px inset | Disk (12 px) over a `color-mix(in oklab, var(--bg-0) 60%, transparent)` background |
| Governance member row | After handle | Disk (14 px) |
| Onboarding peer card | Top-right of card header | Pill |

No surface may silently omit the badge. If the surface has no space, the
surface is redesigned; the badge is not cut.

## Holder pill + visibility tweak

### Holder pill

Flush-right in the channel header, after the topic string. `<button>`
with `aria-label="{n} peers hold this channel's key — tap to see who"`.
Style: `3px 10px`, radius 999, bg `--bg-2`, border `--line`, text
`--ink-2`, type `meta`. Active state: `--moss-1` bg. Left icon `key`
(14 px, 1.6 stroke, `--ink-3`); text is `{n} holders`.

### Holder list

Desktop: popover under the pill (`--bg-1`, `--line`, `--shadow-2`).
Mobile: bottom sheet (radius `--radius-l`). Contents: section label
`who can read this channel` (`meta`, `--ink-3`); one row per holder
(28 px avatar · display name · badge disk · HLC rotation timestamp in
`mono S` `--ink-4`); footer row on `--bg-2` showing your own presence:
`you · holder since {t}`. Badge placement follows the peer-surface
rules.

### Per-grove crypto-visibility setting

The holder pill's *visibility* is controlled by the per-grove
`crypto-visibility` tweak (see `settings-tweaks.md` for the tweak itself;
the tweak is defined in foundation-adjacent scope and consumed here).
Values:

| Value      | Behaviour |
|------------|-----------|
| `subtle`   | Holder pill visible only when the count is *less than* the grove's member count (i.e. not every member holds the key). Otherwise hidden. |
| `default`  | Holder pill always visible on every channel header. |
| `explicit` | Holder pill visible, plus a one-line *crypto strip* below the header: `{n} holders · last rotated {t} · unverified peers: {m}` in `mono S` colour `--ink-3`. |

The tweak is per-grove, not global. Groves with sensitive content can
run `explicit`; casual groves can stay `subtle`. Default is `default`.

## Downgrade / re-verify prompts

When a peer's public key rotates (legitimate device change) or when a
later SAS attempt mismatches a previously verified fingerprint, the
client **marks the peer unverified** and surfaces a prominent banner.
This is a safety-critical UX moment: the banner is explicit, not subtle.

### Visual

Horizontal banner at the top of the peer's letter and on the peer's
profile card below the crest. Full content-width. Icon `Shield`
(`--warn`); title in `--ink-0`, body `--ink-1`. Background
`color-mix(in oklab, var(--warn) 12%, var(--bg-1))`; border 1 px
**dashed** `--warn`; radius `--radius`; padding `12px 14px`; no shadow
(this is a banner, not a popover).

### Copy

Title (`body L`, 14.5 px): `keys changed — verify again`.
Body (`body`, 14 px): `this peer's key rotated or a fingerprint check
failed. whisper and device handoff are paused until you compare again.`
Primary CTA right: `compare now` — `--moss-2` bg, `#14130f` text,
`body S` 13 px / 500. Secondary: `dismiss for now`, `--ink-2` muted
button.

### Rules

- Dismiss hides the banner for 24 h only; the unverified badge on every
  surface remains for the full duration of the unverified state.
- Cannot be permanently dismissed without comparing.
- Idempotent: repeat rotations don't stack banners; the one banner re-
  renders with updated copy.

## Long-press SAS on mobile

On mobile (letter row, members list, profile sheet) long-pressing a
peer avatar opens the compare-fingerprints sheet directly.

- Trigger: press-and-hold ≥ 350 ms on the avatar. Note: the onboarding
  card's 900 ms *hold-to-confirm-match* gesture is distinct and lives
  in `onboarding.md`.
- Feedback: a 2 px `--moss-2` ring grows opacity 0 → 1 over the hold
  duration. Release before threshold: ring fades (`--motion-fast`), no
  action. Release at threshold: ring briefly brightens to `--moss-3`,
  haptic tap (`navigator.vibrate?.(8)`), sheet rises.

### Keyboard equivalent

Every surface that accepts long-press also accepts focus + Enter on
the avatar *or* the badge — same dialog, no separate path.
`prefers-reduced-motion: reduce` drops the ring to opacity-only fade.

## Copy — exact strings

All strings below are reproduced verbatim from the reference bundle and
must not be altered in implementation without a copy change to this spec.
These strings are security UI, not marketing — rewording subtly changes
what the user thinks they're protecting.

| Key | String |
|-----|--------|
| `SAS.title` | `add a friend` |
| `SAS.intro` | `compare six words on two screens. if they match, no one can impersonate either of you in this conversation, ever.` |
| `SAS.reassurance` | `these six words come from your shared key. if someone tried to sit between you, at least one word would be different. verification gets stronger with repetition.` |
| `SAS.youMeta` | `just now · keys created` |
| `SAS.peerMeta` | `arrived via nearby share` |
| `SAS.matchCta` | `they match` |
| `SAS.noMatchCta` | `they don't match` |
| `SAS.unsureCta` (reserved) | `not sure` |
| `SAS.labelYou` | `your fingerprint — read this aloud` |
| `SAS.labelPeer` | `their fingerprint — do these match?` |
| `Badge.verified` | `verified peer` |
| `Badge.unverified` | `unverified — compare fingerprints before you trust this peer` |
| `Badge.pending` | `verification pending` |
| `Badge.pendingChip` | `compare →` |
| `Badge.newPeer` | `new peer` |
| `Confirm.matchTitle` | `verified.` |
| `Confirm.matchBody` | `verified peer — this cannot be silently downgraded by an attacker. their key is pinned; if it ever changes you'll be asked to verify again.` |
| `Confirm.mismatchTitle` | `marked not verified.` |
| `Confirm.mismatchBody` | `marked not-verified — we will keep this peer unverified until you compare again. you can still send messages, but whisper and device handoff stay closed until the fingerprints match.` |
| `Downgrade.title` | `keys changed — verify again` |
| `Downgrade.body` | `this peer's key rotated or a fingerprint check failed. whisper and device handoff are paused until you compare again.` |
| `Downgrade.cta` | `compare now` |
| `Downgrade.dismiss` | `dismiss for now` |
| `Holder.pill` | `{n} holders` |
| `Holder.title` | `who can read this channel` |
| `Holder.selfFooter` | `you · holder since {t}` |

No exclamation marks. All lowercase per `foundation.md`. Proper nouns (a
peer's display name) stay cased.

## Data dependencies

This spec depends on state and derivations that **do not yet exist** in
the current codebase. Everything below is flagged as either existing or
to-be-added so implementation plans can split the work.

### SAS word derivation — **to add**

Pure function `sas_words(session_key: &[u8], a: EndpointId, b:
EndpointId) -> [String; 6]` in a new `willow_crypto::sas` module.
blake3-hash the concat with a stable domain-separation tag; map 6 × 11-
bit windows of the hash to a canonical 2048-word list (see open
question). Must be symmetric (`a,b` == `b,a`), deterministic, WASM-safe
(no `std::time` / `std::thread`), and covered by round-trip vectors.
`willow-crypto` already has X25519 + ChaCha20-Poly1305; SAS is new work.

### Trust state — **to add**

New signal `peer_trust: ReadSignal<HashMap<String, PeerTrust>>` on
`AppState` (candidate: new `AppState::trust` bucket):

```rust
pub enum PeerTrust {
    Unknown,                  // never met, never verified
    PendingVerify,            // first contact, SAS not yet attempted
    Unverified,               // SAS mismatch or never completed
    Verified {
        at: DateTime<Utc>,
        pinned_key: PublicKey,
    },
    DowngradedFromVerified {
        previous_key: PublicKey,
        new_key: PublicKey,
        at: DateTime<Utc>,
    },
}
```

Backed by an append-only **local** trust store (per-device, **not** a
willow-state event — verification is a local belief; shared-trust is
out of scope). Client API additions:
`trust_state(peer_id)`, `mark_verified(peer_id, session_key, words)`,
`mark_unverified(peer_id, reason)`, `begin_compare(peer_id) ->
ComparePreview { you, them }`. Storage: IndexedDB on web, `.dev/trust/`
on native.

### Holder list — **to add**

New `views.channel_holders: ChannelHolders` — per-channel set of
`{peer_id, joined_key_at}`. The rotation timestamp exists inside
`willow-crypto`'s channel-key machine but is not surfaced to the web
state today.

### Key rotation — **to add**

`views.trust_events: ReadSignal<Vec<TrustEvent>>` streaming
`KeyRotated { peer_id, previous, new, at }` and `SasMismatch { peer_id,
at }`. The downgrade banner subscribes to this stream.

### What already exists

- `willow_identity::Identity::endpoint_id()` — Ed25519 public key; the
  identity half of SAS derivation.
- `willow_identity::unpack_profile` already rejects spoofed `peer_id`s
  via `PeerMismatch`. The badge UI does not duplicate that check; it
  trusts state to be consistent before render.
- Re-exported `iroh_base::PublicKey` is the pinned-key type for
  `PeerTrust::Verified`.
- `crates/web/src/state.rs` today has **no** `verified`,
  `pending_verify`, or `peer_trust` signals — adding them is a
  prerequisite for this spec.

## Edge cases

### Compare → they don't match

- Peer becomes `PeerTrust::Unverified`, reason `SasMismatch`. Unverified
  badge on every surface.
- **Messaging still works.** A mismatch can mean genuine MITM *or*
  misread words / wrong device / wrong person. Auto-blocking punishes
  the common case.
- **Whisper is gated.** Whisper pill disabled; tooltip `whisper needs a
  verified fingerprint`.
- **Device handoff is gated.** `move this call` refuses to hand off to
  an unverified peer's device.
- **Ephemeral channels with this peer as holder** gain a warning chip
  `unverified holder`; their holder pill promotes to `explicit`
  visibility until resolved.
- Re-running compare-fingerprints and succeeding clears everything above.

### New peer (never interacted)

First message from a peer whose trust is `Unknown` shows the `new peer`
label (not amber unverified). A `learn` link opens a compact tooltip
`new peer · compare fingerprints to verify`. The first SAS attempt
(success or fail) replaces the label with the normal badge everywhere.

### Peer with multiple devices

Out of scope for v1 — each `EndpointId` is its own trust subject.
Device linking is covered by `device-handoff.md`.

### i18n word-list collisions

Reference word list is English-only in v1. Cross-locale compare is
broken by a locale-sensitive list (Alice sees English, Bob sees German —
they never match even with a correct session key). **Deferred**; see
open questions.

### Screen-reader vs visual mismatch

The cell's `aria-label` always contains the full word. A truncated word
is never acceptable; if the word won't fit on a narrow screen, the
layout is redesigned.

### Copy-to-clipboard

The profile-card `copy fingerprint` action copies the dash-joined six-
word string (`willow-copper-reed-glade-slate-moth` format). It's a
separate action from compare-fingerprints and does not change trust.

## Accessibility

- **Dialog.** `role="dialog"`, `aria-modal="true"`, focus trap. First
  focus on the primary CTA. ESC dismisses.
- **Buttons, not links.** `they match`, `they don't match`, and the
  reserved `not sure` are `<button>` elements — never `<a href>`.
  Security decisions must not be triggered by navigation.
- **Grid semantics.** `role="table"` / `row` / `cell`; cell label is
  `word {n}, {word}`.
- **Badge labels.** Every badge carries an `aria-label` from the copy
  table. Icon-only badges never render without the label.
- **Colour independence.** Every state has a shape cue: verified =
  filled check on disk; unverified = dashed ring + `?`; pending-verify
  = `?` + chip; matched = solid `--ok` border; mismatch = dashed
  `--warn` border.
- **Focus-visible.** All interactive elements use `--focus-ring`.
- **Reduced motion.** State-tint transitions collapse to instant; long-
  press ring fades opacity only.
- **Touch targets.** Pending-verify chip, `compare →` chip, and holder
  pill are each ≥ 44 × 44 CSS px on mobile (visual border is smaller;
  hit box extends via padding).
- **Contrast.** `--warn` on `--bg-1` meets WCAG AA at ≥ 14 px / 500.
  Smaller warning text (10.5 px meta) always pairs with a larger title
  to satisfy the 3:1 non-text contrast rule for the shape cue.
- **Announcements.** On state change, an `aria-live="assertive"`
  region emits:
  - match → `verified peer {name}. compare fingerprints dialog closed.`
  - mismatch → `marked not verified. compare fingerprints dialog still
    open; choose compare again or close.`
  - downgrade → `keys changed for {name}. verify again.`

## Acceptance criteria

- [ ] `FingerprintGrid` and `FingerprintLabel` exist as reusable Leptos
      components in `crates/web/src/components/sas.rs`, accept `words`
      (`[String; 6]`), `size` (`md` / `sm`), and `variant`
      (`you` / `peer` / `matched` / `mismatch`).
- [ ] The grid renders a 3-column × 2-row table with 1-indexed numbers
      in `mono S` and lowercase words in `mono L`, using `--bg-2` cells
      and `--line` borders.
- [ ] Every badge state (`verified`, `unverified`, `pending-verify`,
      `new peer`) renders on every peer-identifying surface listed in
      the placement table, with the exact `aria-label` strings.
- [ ] The compare-fingerprints flow opens from the profile card, from
      the letter row chip, and from the onboarding pending-verify
      entry point, and reaches the same three screens (compare / match
      confirm / mismatch confirm).
- [ ] The flow is a `role="dialog"` with focus trap; `they don't match`
      is never auto-focused.
- [ ] Tapping `they match` marks the peer verified via the new client
      API, animates the peer grid to `matched` state, and transitions
      to the match-confirm screen.
- [ ] Tapping `they don't match` marks the peer unverified via the new
      client API, animates the grid to `mismatch` state, and offers
      `compare again` / `close`.
- [ ] Long-press on a mobile avatar (≥ 350 ms) opens the compare
      sheet; release before threshold cancels with visible ring fade.
- [ ] Keyboard focus + Enter on the badge opens the same flow.
- [ ] The holder pill renders on every channel header and respects the
      three `crypto-visibility` modes (`subtle`, `default`, `explicit`).
- [ ] After a simulated key rotation, the downgrade banner appears at
      the top of the peer's letter and profile card with the exact
      copy, a dashed `--warn` border, and a `compare now` CTA.
- [ ] All copy strings match the "Copy" table byte-for-byte; a copy-
      lint test catches drift in CI.
- [ ] `prefers-reduced-motion: reduce` collapses every state animation
      to opacity-only.
- [ ] `crates/web/tests/browser.rs` covers: grid rendering, badge
      placement, dialog open/close, match and mismatch paths, keyboard
      activation from badge focus.
- [ ] `just test-state` covers the new trust store transitions
      (`Unknown → PendingVerify → Verified / Unverified`,
      `Verified → DowngradedFromVerified`).

## Open questions

1. **Word list.** BIP-39 English (standardised, familiar to some users)
   vs a Willow-specific 2048-word list with forest vocabulary? The
   reference bundle's sample (`willow`, `copper`, `reed`, `glade`,
   `slate`, `moth`) is not BIP-39. Recommendation: ship a Willow list
   disjoint enough from BIP-39 that users never confuse the two, and
   publish it in the spec for auditability.
2. **SAS algorithm.** blake3 + domain-separation tag + 6 × 11-bit
   windows, or HKDF-SHA256 with `"willow sas v1"` info? Either is fine;
   the implementation plan picks one and adds test vectors.
3. **Trust store as event?** Local belief (per-device) is the safer
   default: one compromised device cannot unverify a peer on another.
   A future "shared trust" EventKind is out of scope for v1.
4. **`not sure` CTA.** Ship in v1 or defer? Shipping early defends
   against the "decision under pressure" failure where a user taps
   `they match` just to dismiss the dialog.
5. **i18n.** Cross-locale compare is broken by locale-sensitive word
   lists. Candidate fix: embed the locale ID in SAS derivation so
   mismatches surface as locale mismatches, not generic "not verified".
6. **Verified audio cue.** A soft `willow-pop-in` on success? Defer
   unless user testing asks for it — every sound has to earn its place
   in Willow's quiet voice.
7. **Pending-verify reminder.** Surface a reminder after N hours of no
   action? Tentative no — the badge does the work.
8. **Downgrade grace period.** After `KeyRotated`, auto-re-enable
   whisper / handoff for 60 s? Leaning no — safe default is to close
   the door and let the user re-open it via compare.
