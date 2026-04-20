# Profile card — peer + self letterhead

**Parent:** [README.md](README.md)
**Dependencies:** [`foundation.md`](foundation.md), [`layout-primitives.md`](layout-primitives.md), [`trust-verification.md`](trust-verification.md)
**Status:** draft
**Consumed by:** [`letters-dms.md`](letters-dms.md), [`messaging.md`](messaging.md), [`settings-tweaks.md`](settings-tweaks.md)

**Ownership direction.** The badge visuals and the SAS grid come from
[`trust-verification.md`](trust-verification.md). This spec owns the
profile card container (desktop popover + mobile sheet), the crest
banner, the field layout, and the private-nickname editor. When a
badge on this card is tapped, control hands off to
`trust-verification.md`'s `add a friend` compare flow; this spec only
reserves the slot and the event-bus entry point.

## Purpose

The profile card is a peer's *letterhead* — a small, quiet document
that tells you who they are, what they've chosen to show, and how far
you've come in trusting them. Not a social-media profile. Literary,
trust-forward: the banner is a personal crest, the bio is plain on an
italic display name, the pinned fragment sits behind an accent-tinted
rule, and every fingerprint is mono and verifiable.

One profile-card surface exists in the app. Any avatar click (grove
rail, channel sidebar, message list, thread pane, members pane,
letters list, call tile) opens it. On desktop it is an anchored
popover; on mobile a bottom sheet. Both wrappers render the same
content in the same order.

Trust state (verified / unverified / pending) is surfaced *here* but
the compare-fingerprints flow it hands off to lives in
[`trust-verification.md`](trust-verification.md). Editing self fields
hands off to [`settings-tweaks.md`](settings-tweaks.md).

## Scope

- One shared content component with two wrappers (popover, sheet) and
  one global controller that wires the event bus to the wrapper
  mounted at the current breakpoint.
- Peer view and self view, from the same content with variant flags.
- Crest banner (procedural SVG, three patterns).
- Private-nickname inline editor (peer view).
- Badge display and click-through to the trust-verification flow.
- Data contract: which fields come from `willow-state`, which from
  `willow-identity`, which are local-only, and which are new.

Out of scope: the compare-fingerprints screen, the Settings Profile
tab layout, block-list management, letter composition.

## Field inventory

Fields are listed in render order, top-to-bottom, peer view. Self view
uses the same order except as noted under "Self view".

### Peer view

| # | Field | Source | Shape |
|---|-------|--------|-------|
| 1 | Crest banner | `crestPattern` + `crestColor` (new state — see *Data dependencies*). Defaults to `leaf` / `--moss-2` when absent. | Procedural SVG, 72 px tall on desktop, 92 px on mobile. |
| 2 | Verification badge (on banner) | Derived from `willow-identity` + SAS records (`trust-verification.md`). | Pill, top-left on banner. |
| 3 | Avatar | `willow-identity` profile. Overlaps banner; 3 px `--bg-1` border ring; 64 px on desktop, 84 px on mobile. | Circle. |
| 4 | Presence dot | Live network state + whisper signal. | 13 / 16 px circle, anchored bottom-right of avatar, boxed in `--bg-1`. |
| 5 | Display name | `willow-state::Profile.display_name` (existing). | Fraunces italic, display-S size (17 px) → bumped to 20 px here, 24 px on mobile. |
| 6 | Pronouns pill | `profile.pronouns` (new state). | Body-S in a `--line-soft` bordered capsule; omitted if unset. |
| 7 | Handle + "you call them …" line | Handle from `willow-identity`; nickname from local-only storage. | Mono M, `--ink-3`. Nickname segment tinted `--moss-3` "you call them" label + `--ink-2` body. |
| 8 | Status pill | Live presence: `here`, `away`, `whispering`, `gone`. Queued count overlays. | Inline flex with colour dot. |
| 9 | Bio | `profile.bio` (new state). | Body 13 px / 14.5 px (mobile), `--ink-1`, line-height 1.55. Not italic — the *display name* carries the italic voice. |
| 10 | Tagline | `profile.tagline` (new state). | Mono S, `--ink-3`, preceded by middle-dot. |
| 11 | Pinned fragment | `profile.pinned` (new state; shape `{kind: quote|fragment, body}`). | Accent-tinted left rule (2 px, `crestColor`), Fraunces italic body, `PINNED FRAGMENT` mono label. Quotes wrap in curly quotation marks; fragments render plain. |
| 12 | Shared groves | Derived by intersecting grove membership (existing `willow-state` — the set of groves the local peer and the target peer both belong to). | Chip row; each chip shows grove glyph on accent-coloured square + lowercased name. |
| 13 | Elsewhere | `profile.elsewhere: Vec<String>` (new state). Non-identifying freeform labels ("coast · west", "studio: the long room"). | Chip row; `ELSEWHERE` mono label. |
| 14 | Meta: since | `profile.since` string hint (new state) — e.g. `spring · yr 2`. Soft-time, not a timestamp. | Grid row: mono `SINCE` label, body `--ink-2` value. |
| 15 | Meta: fingerprint | `willow-identity` 6-word fingerprint, short form (3 words) in the meta row, full form on hover title / long-press. Colour follows verification state (`--moss-3` verified, `--warn` unverified). | Mono S, truncate-ellipsis. |
| 16 | Primary action row | `letter` · `call` · `whisper` · `more`. | Horizontal buttons on desktop; stacked vertical column on mobile. |
| 17 | Secondary row | `copy fingerprint` · `set nickname` / `change nickname` · `block`. | Flex-between link-style buttons above a `--line-soft` rule. |

### Self view

Same fields 1–15, with these changes:

- **Pronouns pill** — shown; clicking the identity block opens the
  Settings profile tab. No inline edit.
- **Nickname line** — absent. Nicknames are things *others* call them.
- **Primary action row** — replaced by a single full-width
  `edit profile` button (`--moss-2` primary) that opens the Settings
  Profile tab. No letter / call / whisper.
- **Secondary row** — replaced by a quiet centred caption:
  `this is you · <first three fingerprint words>`. No block, no
  nickname action, no copy-fingerprint.

### Role label

The card does not render a large role badge. Role surfaces in the
Members pane; the card is about the person, not their grove role.
Role labels `steward`, `member`, `guest` are defined here for cross-
spec reference only.

## Crest banner

The banner is a procedural SVG occupying the top 72 px of the desktop
card and 92 px of the mobile sheet. It is a *personal mark*, not a user-
uploaded cover photo: the only inputs are `(crestPattern, crestColor)`
which the user selects from the Tweaks panel.

### Patterns

Three deterministic SVG patterns, seeded by peer id so the same peer
always renders the same banner (visual-identity fingerprint, not a
cryptographic one):

- `fronds` — 14 vertical frond strokes with seeded sway; bowed
  understory curve.
- `rings` — six scattered circles + two concentric centre rings.
- `leaf` — long ogee sweep with nine pendant leaves; faint upper line.

### Colour + gradient

`crestColor` fills the pattern strokes. A vertical `crestColor`
gradient (0.55 → 0.18 → 0 opacity) sits behind the pattern, and a
horizontal ink wash (`--bg-0` at 0 → 0.22 → 0 opacity) sits over it
so the bottom edge fades toward the avatar. The banner reads as a
watermark — never competes with the identity block.

### Missing / default

- `crestPattern === nil` → default to `leaf`.
- `crestColor === nil` → default to `--moss-2`.

### Banner badges

The badge row renders *inside* the banner, top-left, and a close button
renders top-right (desktop only — mobile uses the sheet's own close
affordances).

- **Verified** — filled moss check on a `color-mix(--bg-0, 60%,
  transparent)` backdrop-blur pill. Label: `verified`. Colour:
  `--moss-3`.
- **Unverified** — outline shield icon, label: `unverified`, colour:
  `--warn`. Same blur pill backdrop.
- **Pending verify** — amber `?` glyph + compare-arrow chip. Label:
  `compare →`. Colour: `--warn`. Clicking opens the compare flow (see
  *Badges + trust surfacing*).

## Desktop popover

Single host component mounted once near the app root. Subscribes to
the profile event bus and renders the card when controller state is
non-null.

- **Width:** 320 px, fixed. **Height:** intrinsic, capped at
  `calc(100vh - 24px)` with internal scroll using the `.scroll` style
  from `foundation.md`.
- **Positioning:** default `anchor.right + 8px`; flip to
  `anchor.left - 320 - 8px` if it would overflow right; clamp
  horizontally to `[12, vw - 320 - 12]` if neither side fits.
  Vertically align `top` with `anchor.top`; shift up if bottom would
  overflow. Positioning runs pre-paint so the card never flashes at
  an offscreen placeholder coordinate.
- **Chrome:** `--bg-1` surface, `1px solid --line`, `border-radius:
  12px`, `--shadow-2`, z-index 200. Open animation: `willow-pop-in`
  (~180 ms); close animation: opacity-only.
- **Dismissal:** Escape; click outside (mousedown listener attached
  one tick after open so the originating click doesn't immediately
  close); close button in banner top-right; any navigating action
  (letter, call, whisper, edit profile, compare) closes after
  dispatch; copy-fingerprint does *not* close the card.
- **Re-anchor:** opening for the *same* user id with a different
  anchor re-positions without remount or re-animation.

## Mobile bottom sheet

Single host component mounted in the mobile app shell. Same event-bus
contract as the popover. Exactly one host renders per breakpoint;
`layout-primitives.md` picks which.

- **Dimensions:** full viewport width; intrinsic height up to `90vh`
  with internal scroll; top corners `22 px`.
- **Animation:** scrim fades to `rgba(0,0,0,0.45)` in 180 ms; sheet
  `translateY(100%) → 0` in 220 ms on ease-out-willow. Close reverses.
  Reduced-motion collapses the translate to an opacity fade.
- **Drag handle:** 44 × 5 px `--line` pill, 6 px below the top edge,
  centred. Decorative in v1 (not a drag target). Drag-to-dismiss is
  intentionally deferred.
- **Dismissal:** scrim tap; browser back (sheet pushes a transient
  history state on open, pops on close); Escape on tablets; any
  navigating action closes on dispatch.
- **Safe area:** `padding-bottom` respects
  `env(safe-area-inset-bottom)` so the action column clears the home
  indicator.
- **Layout deltas from desktop:** avatar 84 px; display name 24 px;
  presence dot 16 px; primary action row stacks as a full-width
  column with 12 px radius; secondary row stays horizontal but with
  ≥ 44 × 44 targets.

## Event-bus API

All avatar clicks across the app dispatch through one global bus. The
UI layer exposes three symbols.

### `open_profile(user_id, anchor_el)`

Dispatch a `ProfileEvent::Open { user_id, anchor }` custom event on the
window. `anchor_el` is an optional DOM reference — required on desktop
to position the popover, optional on mobile (the sheet ignores it).

In Leptos, this is a small helper in `crates/web/src/profile/bus.rs`
that wraps `web_sys::CustomEvent` dispatch and is callable from any
component.

### `close_profile()`

Dispatch a `ProfileEvent::Close` custom event on the window.

### Anchor contract

- The anchor is a `web_sys::HtmlElement` (or any element with
  `getBoundingClientRect`). On click, callers pass the clicked element:

  ```rust
  let on_click = move |ev: MouseEvent| {
      let target = ev.current_target().unwrap().dyn_into().unwrap();
      open_profile(&user_id, Some(target));
  };
  ```

- Callers must not detach or re-render the anchor while the card is
  open for the same user. The controller uses the anchor live for
  "click outside" detection; if the anchor is unmounted, outside-click
  detection falls back to "any click outside the card".
- If the anchor becomes null mid-lifecycle (e.g. message scrolled out
  of view on desktop), the card stays open at its last position until
  explicitly closed.

### Controller

`use_profile_controller() -> Signal<Option<ProfileState>>` — a Leptos
signal that the two host components subscribe to. The controller
listens on the window for the two events and maintains:

```
struct ProfileState {
    user: Arc<ProfileView>,   // merged peer data (see Data dependencies)
    anchor: Option<HtmlElement>,
}
```

The controller owns its `Escape` keydown listener. It debounces open
events so double-click on an avatar fires the card once.

### De-duplication

- `open_profile(u, a)` with a card already open:
  - Same user id → update anchor only; do not remount, no entry
    animation replay.
  - Different user id → close current, open new as a fade-swap (no
    slide/pop re-animation); sheet content cross-fades in ~100 ms.
- `close_profile()` while no card is open is a no-op.

## Badges + trust surfacing

The profile card is the primary surface where trust state is visible
on-demand. The badge displayed on the banner pill is derived from a
single source: the local peer's SAS verification record for the target
peer, as defined in `trust-verification.md`.

### State → badge mapping

| Trust state | Pill background | Icon | Label | Colour |
|-------------|-----------------|------|-------|--------|
| `verified` | blurred `--bg-0` 60% | filled check | `verified` | `--moss-3` |
| `unverified` (default for new peers) | blurred `--bg-0` 60% | outline shield (dashed ring variant) | `unverified` | `--warn` |
| `pending_verify` (SAS started, not completed) | blurred `--bg-0` 60% | amber `?` + arrow | `compare →` | `--warn` |

The unverified pill uses a *dashed ring* stroke on the shield icon to
satisfy the accessibility baseline in `foundation.md` (colour is never
the only signifier — amber pairs with dashed stroke; moss pairs with
filled check).

### Hand-off to `trust-verification.md`

- Clicking the **verified** pill: opens a small confirmation tooltip
  showing the full 6-word fingerprint and the date verified. No flow
  transition.
- Clicking the **unverified** pill: opens the compare-fingerprints
  flow (see `trust-verification.md`). On desktop, the flow opens as a
  centred dialog, replacing the popover; on mobile, the sheet content
  slides sideways to the compare view within the same sheet.
- Clicking the **pending-verify** pill: resumes the in-progress compare
  flow at its last step.

### Copy (banner)

- Verified title tooltip: `verified peer`
- Unverified title tooltip: `unverified — compare fingerprints before you trust this peer`
- Pending-verify title tooltip: `compare in progress · resume →`

The meta-row fingerprint tint (`--moss-3` or `--warn`) mirrors the
banner badge, so the card is colour-consistent top-to-bottom about
trust state.

## Private nickname

Nicknames are **local only** — not propagated, not in events, stored
in the client's browser storage keyed by peer id.

- **Display.** When set, the handle line reads
  `mira.sage · you call them Mira` (handle `--ink-3`, "you call them"
  `--moss-3`, nickname `--ink-2`). When unset, the handle stands alone.
- **Editor.** Secondary-row button reads `set nickname` when unset,
  `change nickname` when set. Click opens an inline editor in the
  handle line: the "you call them …" segment becomes a mono 12 px
  input (`--bg-2` on `--line`). Enter saves; Escape cancels; blur
  saves; empty save clears the nickname.
- **Storage.** Key `willow.profile.nickname.<peer_id>`; value UTF-8;
  cap 32 chars (client-enforced). Cleared local storage loses
  nicknames — correct, since they were never off-device.
- **Self view.** Never shows "you call them …". The self display name
  is how *others* see you; nicknames are how *you* see *others*.

## Editing — self

The self view's `edit profile` button opens the Settings panel Profile
tab. The Profile tab layout is specified in `settings-tweaks.md`; this
spec defines *what the card writes through*.

Editable fields (from the Settings Profile tab, not inline on the
card):

| Field | Scope | Propagates? |
|-------|-------|-------------|
| display name | existing `willow-state` | yes — `SetDisplayName` event |
| pronouns | new state | yes (new `SetPronouns` event or carried on `SetProfile`) |
| nickname-visible-to-others (bool) | new state | yes — but see *Open questions*: v1 **does not ship this**. Nicknames remain local-only. Field reserved for future. |
| bio | new state | yes |
| tagline | new state | yes |
| crest pattern | new state | yes |
| crest colour | new state | yes |
| pinned fragment | new state | yes |
| elsewhere list | new state | yes |

"Propagates" means the field is part of the grove-visible profile
event stream and reaches other peers. All propagated fields are
authored by the peer themselves and carry the peer's ed25519 signature
per `willow-identity`.

Clicking `edit profile` on the desktop popover: close the popover,
open the Settings panel with tab = Profile. On mobile: close the sheet,
navigate to the settings route.

## Copy

Exact strings. These are the authoritative versions; all other specs
and code that need these strings must import or mirror these values.

```
PROFILE_COPY {
    message:         "message"
    call:            "start call"
    whisper:         "whisper"
    copy_fingerprint:"copy fingerprint"
    verify:          "verify in person"
    block:           "block"
    edit_profile:    "edit profile"
    edit_nickname:   "set nickname"
    change_nickname: "change nickname"
    unverified:      "unverified — compare fingerprints before you trust this peer"
    verified:        "verified peer"
    self:            "this is you"
    queued_prefix:   "queued ·"
    whisper_status:  "whispering"
    fingerprint:     "fingerprint"
    since:           "in the grove since"
    shared_groves:   "you share"
    known_as:        "you call them"
    pinned:          "pinned fragment"
    elsewhere:       "elsewhere"
    empty_pinned:    "no pinned fragment"
}

ROLE_LABEL   { steward, member, guest }
STATUS_LABEL { here, away, whispering, gone }
```

Notes:

- All labels are lowercase per `foundation.md` voice rule.
- `here` / `away` / `whispering` / `gone` map to network states `online
  / idle / whisper / offline`. The user-visible term is *never* the
  code identifier.
- `queued ·` always precedes a count, e.g. `queued · 3`.
- `you call them` is followed by the nickname value, never colonised:
  `you call them Mira`, not `you call them: Mira`.
- Empty-pinned is only shown on the *self* card. The peer card simply
  omits the pinned block when no fragment is set.

## Data dependencies

Every field on the card, and where it comes from.

### Existing

- **Handle** — `willow-identity::PeerId` short form. Existing.
- **Display name** — `willow-state::ServerState::display_name(peer)`.
  Existing: set by `SetDisplayName` event.
- **Fingerprint (short + full)** — `willow-identity` 6-word mapping of
  the peer's public key. Existing.
- **Avatar** — current client uses a deterministic colour derived from
  peer id. Existing.
- **Status / presence** — live network state from `willow-network`.
  Existing. (The *label* mapping here → `here / away / whispering /
  gone` is new copy, not new state.)
- **Queued count** — `willow-client` sync queue per-peer count.
  Existing.
- **Shared groves** — intersection of grove memberships from
  `willow-state`. Existing data; new read path.
- **Since** — derived from the earliest event authored by the peer in
  the local grove event log. Existing data; new derivation. Rendered as
  soft-time (`spring · yr 2`) — soft-time mapping is a new utility.

### New state (grove-propagated)

These require new `EventKind` variants in `willow-state` (or extension
of a `SetProfile` meta-event). Each is authored by the peer themselves;
permission required is none beyond self-authorship.

- **Pronouns** — `String`, cap 32.
- **Bio** — `String`, cap 240.
- **Tagline** — `String`, cap 80.
- **Crest pattern** — enum `{ Fronds, Rings, Leaf }`.
- **Crest colour** — RGB hex string, cap 7 chars. Validator enforces
  the palette set (future: free-form with contrast check).
- **Pinned fragment** — `Option<Pinned { kind: Quote | Fragment,
  body: String }>`, body cap 280.
- **Elsewhere** — `Vec<String>`, cap 4 entries × 48 chars. Non-
  identifying freeform; no URL parsing, no link rendering. The client
  treats entries as plain text to avoid accidentally leaking
  identifiable side-channel accounts.

### New state (local-only)

- **Nickname** — local browser storage keyed by peer id. Never
  propagated. Cap 32.

### Derived client-side

- **Verification state** — from SAS records (see
  `trust-verification.md`).
- **Whisper flag on status pill** — from active whisper sessions (see
  `whisper-mode.md`).

### Summary table

| Field | Where | Status |
|-------|-------|--------|
| handle | `willow-identity` | existing |
| display_name | `willow-state` | existing |
| fingerprint | `willow-identity` | existing |
| avatar colour | derived from peer id | existing |
| status / presence | `willow-network` | existing |
| queued count | `willow-client` | existing |
| shared groves | `willow-state` | existing |
| since | `willow-state` (derived) | existing data, new derivation |
| verification | `trust-verification.md` | derived |
| pronouns | profile event | **new state** |
| bio | profile event | **new state** |
| tagline | profile event | **new state** |
| crest pattern | profile event | **new state** |
| crest colour | profile event | **new state** |
| pinned fragment | profile event | **new state** |
| elsewhere | profile event | **new state** |
| nickname | local storage | **new state** (local-only) |

New grove-propagated fields should land as a single `SetProfile`
event kind carrying an optional update for each field. The plan for
rolling out these events is deferred to the per-child implementation
plan (`docs/plans/…-profile-card.md`).

## Edge cases

- **Missing crest pattern / colour** — default `leaf` / `--moss-2`.
- **Missing bio, tagline, pinned, elsewhere, pronouns, since** — the
  section is omitted entirely. The peer card never renders empty-
  state rows for unset fields. The *self* card shows `no pinned
  fragment` as a quiet prompt when pinned is unset.
- **Super-long display name** — single-line truncate with ellipsis;
  full name in `title` on desktop hover / long-press on mobile. The
  pronouns pill wraps to line 2 if needed.
- **Super-long bio** — wraps freely; card scrolls internally above
  the viewport height cap.
- **No shared groves** — omit the shared-groves section entirely.
- **Unverified peer with nickname set** — badge stays unverified;
  nickname still renders in the handle line. Nickname does not imply
  trust.
- **Card opened for an unknown user id** — the controller drops the
  event; caller bug. Controller does not fetch.
- **Mobile sheet opened while another sheet is open** — profile wins;
  the layout sheet manager closes the other first.
- **Peer goes offline while card is open** — presence dot updates
  live; queued count appears on the status pill. No modal interrupt.
- **Peer updates their profile while card is open** — sections cross-
  fade in place over 120 ms; wrapper does not re-animate.
- **Peer leaves the shared grove** — chip row updates; card stays
  open; `block` remains available.

## Accessibility

- The popover and the sheet both render with `role="dialog"` and
  `aria-label="profile — <display_name>"`.
- On open, focus moves to the card (first focusable element in the
  primary action row, or the close button on desktop if present). On
  close, focus returns to the anchor element.
- `Escape` closes the card. On mobile, the browser back gesture also
  closes.
- The verification badge communicates state via:
  - icon shape (check for verified, dashed-ring shield for unverified,
    question mark for pending),
  - label text (`verified`, `unverified`, `compare →`),
  - colour (`--moss-3`, `--warn`, `--warn`).
  Colour is never load-bearing alone.
- Touch targets ≥ 44 × 44 CSS px on mobile for every button,
  including the small secondary-row links.
- The sheet respects `env(safe-area-inset-bottom)`.
- The crest SVG has `aria-hidden="true"` — it is decorative; the
  banner's meaning comes from the adjacent badge label.
- Nickname inline editor: the input has `aria-label="nickname for <name>"`;
  the cancel / save affordance is keyboard-only (Enter / Escape /
  blur).
- Reduced motion: sheet slide collapses to opacity fade; popover
  `willow-pop-in` collapses to opacity fade; section cross-fades on
  live update collapse to instant swap.
- Screen reader order follows the visual render order. The banner
  badge pill is read immediately after the avatar: `<display name>,
  <pronouns>, <handle>, you call them …, verified peer`.

## Acceptance criteria

- [ ] `open_profile(user_id, anchor)` anywhere in the web client
      produces the card on both desktop and mobile.
- [ ] Only one profile surface is mounted per breakpoint.
- [ ] Desktop popover positions against any anchor inside main pane,
      thread pane, members pane, letters list, and grove rail. Flips
      when overflowing right. Never offscreen.
- [ ] Mobile sheet opens from the bottom, respects safe-area-inset,
      and closes via back button, scrim tap, and Escape (tablets).
- [ ] Escape and click-outside dismiss without dismissing the
      underlying view.
- [ ] All three crest patterns render correctly; the same peer id
      produces the same banner deterministically.
- [ ] Missing crest falls back to leaf / moss; no crash.
- [ ] Verified / unverified / pending badges render their correct
      icon + label + colour with matching title-tooltip copy.
- [ ] Clicking an unverified or pending badge opens the compare-
      fingerprints flow.
- [ ] `copy fingerprint` copies the full 6-word form and does *not*
      close the card.
- [ ] `set nickname` / `change nickname` opens an inline editor;
      Enter saves, Escape cancels, blur saves, empty clears.
- [ ] Nicknames persist across reload, never emitted in an event.
- [ ] Self card shows `edit profile`, omits the secondary row, and
      shows `this is you · <3 fingerprint words>` as a quiet caption.
- [ ] `edit profile` opens the Settings Profile tab and closes the
      card.
- [ ] Accessibility baseline from `foundation.md` is met: focus ring,
      focus move on open, focus return on close, role=dialog, aria-
      label, reduced motion respected.
- [ ] Long display names truncate with ellipsis; full name on hover
      (desktop) or long-press (mobile).
- [ ] Re-opening for the same user id re-positions without remount;
      re-opening for a different user cross-fades without re-
      animating the wrapper.

## Open questions

- **Nicknames optionally propagate?** v1 local-only. A later opt-in
  could tell the peer "alex calls you alex·b". Deferred.
- **Crest colour freeform?** v1 restricts to the accent palette in
  `foundation.md` to avoid low-contrast banners. Freeform w/ contrast
  check deferred.
- **Role surface on the card?** v1 omits. Member pane shows role.
  Revisit if governance needs a jump-off.
- **Multiple pinned fragments?** v1 is one. Shape reserved for a list;
  only the first renders today.
- **Elsewhere entries linkable?** v1 is plain text only.
  `willow://` deep links are a future option; arbitrary URLs stay
  disallowed to avoid accidental identity leaks.
- **Sheet drag-to-dismiss?** v2. v1 uses scrim tap, back gesture,
  Escape.
- **Mini-badge on avatars app-wide?** Owned by
  `trust-verification.md`.
- **Live update animation?** v1 cross-fades changed sections; fall
  back to instant swap if distracting. Decision after first review.
