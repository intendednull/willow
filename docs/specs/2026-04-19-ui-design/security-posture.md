# Security posture — threat model, invariants, classifications

**Parent:** [README.md](README.md)
**Dependencies:** none (this is a reference, not a spec)
**Status:** draft
**Consumed by:** every security-critical and privacy-sensitive spec in this
folder. Every other child spec cross-references the invariants below from
its own Accessibility and Acceptance sections.

## Purpose

The sibling specs in this folder describe user flows — what a peer sees,
where a pill sits, what a button says. This document describes the *axis
those flows are measured against*. It names the threat model Willow assumes,
classifies every spec by its security posture, and enumerates the invariants
each security-critical spec must preserve.

It has no UX of its own. Its only job is to be the single answer to the
question *"is this spec change safe?"*. When a future spec or change
appears to conflict with §2 (invariants) or §5 (red-lines), this document
is the veto. When a claim in the invariants below is not yet reflected in
the spec it cites, §8 ("open issues") lists the gap — future revisions
either bring the spec into line or amend the invariant.

Willow is a peer-to-peer, event-sourced client. Authority lives in
cryptographic keys; state lives in signed events; trust lives in locally-
held beliefs about who those keys belong to. The UX has to make those
properties legible without making them noisy. That balance is the whole
product; this document is its contract.

## 1. Actors + trust boundaries

Seven actors. Each is cited by name throughout the invariants and the
adversary table.

### 1.1 Local device (fully trusted)

The device holding the user's Ed25519 identity keypair — the root of the
user's authority. Willow treats the local device as fully trusted by its
owner: it renders decrypted message bodies, holds channel keys in memory,
derives SAS fingerprints, and signs outbound events. There is no UX that
models the local device as adversarial; if the local device is
compromised, every ceiling above it collapses. See `settings-tweaks.md`
for the identity + devices surface that manages the keypair and
`onboarding.md` for identity creation.

### 1.2 Other devices linked to the same identity (trusted after linking)

A second device joined via the explicit linking ceremony (see
`settings-tweaks.md § identity + devices`) becomes a peer of the local
device under the same identity. Linked devices receive handoffs
(`device-handoff.md`), share the identity's event stream, and can
themselves emit signed events authored by the identity. Linking is a
ceremony, not an inference — two devices never become linked by
coincidence, network proximity, or a majority vote of peers.

### 1.3 Verified peer (SAS-verified)

A peer whose identity key has been compared out-of-band via the six-word
SAS flow in `trust-verification.md`, resulting in a locally-pinned
`PeerTrust::Verified { pinned_key }`. Verified peers unlock whisper
(`whisper-mode.md`), handoff targets involving them, ephemeral channel
membership negotiation, and the reassurance copy "this cannot be silently
downgraded by an attacker" (`trust-verification.md § Confirm.matchBody`).

### 1.4 Unverified peer

A peer whose identity key has not been (or has been but failed) SAS-
compared, or whose previously-verified key has rotated. The UX still
renders their messages and allows plaintext conversation inside groves
the user has opted into, but gates security-ceremony flows (whisper,
handoff, ephemeral holder promotion) per `trust-verification.md §
Compare → they don't match`. An unverified peer is a peer whose MITM
status is *undecided*, not a peer known to be hostile.

### 1.5 Arbitrary network (fully untrusted)

Public internet, wifi at the coffee shop, cellular carrier, the iroh
relay's transit. Willow assumes a global passive adversary observing
every byte of metadata: connection timing, peer reachability, message
sizes, directory lookups. The UX never suggests network traffic is
private; it communicates that *content* is sealed end-to-end and that
metadata leakage is a known, bounded surface. No UX in this folder may
claim "willow hides who you talk to".

### 1.6 Relay (role tag only — no default authority)

The iroh relay is a regular peer with a role tag — a convenience node
for bridging peers behind NAT and for offline history buffering. It
holds no more authority than any other peer absent an explicit
`GrantPermission(SyncProvider)` event authored by the grove owner (see
`willow-state` authority model in `CLAUDE.md`). The settings footer in
`settings-tweaks.md § network` states this plainly: *"traffic through
the relay is still end-to-end encrypted. the relay only sees
ciphertext."* A compromised relay is an untrusted peer signing attacker-
controlled events (§4.2 below).

### 1.7 Owner of a grove (root of trust for that grove)

The creator of a grove holds implicit all-permissions and is the root of
the grove's permission chain. Every admin, every role, every permission
grant traces back to an owner signature via `GrantPermission` events (see
`governance.md` and `willow-state` authority model). The owner key is
rotated or transferred only via an explicit transfer flow; `governance.md`
treats owner change as a first-class, audited event, never an inference.
A grove with no owner is the `Orphan admin` edge case in
`governance.md § Edge cases` — its event log is preserved, but no
further admin actions can be taken.

## 2. Invariants the UX must never break

Each item is a short assertion, cited by spec. If a cited spec does not
yet reflect the invariant, see §8 (open issues) — do not silently ship.

- **I-1.** Fingerprints displayed to the user are derived from the
  current session key plus both peer identities, symmetrically
  (`sas_words(session_key, a, b) == sas_words(session_key, b, a)`). No
  surface derives a fingerprint from identity alone. Source:
  `trust-verification.md § Data dependencies → SAS word derivation`.
- **I-2.** The "they match" decision in the SAS flow persists a
  *local-only* trust annotation. It is never gossiped to other peers,
  never broadcast as a `willow-state` event, and is not authoritative
  for other devices under the same identity. Source:
  `trust-verification.md § Data dependencies → Trust state`
  (`PeerTrust` backed by an append-only **local** trust store, not a
  willow-state event).
- **I-3.** Whisper keys are distinct from main-call keys. Whisper
  content is invisible to non-whisperers including in sidebar previews,
  participant counts, and bystander-visible pills. Source:
  `whisper-mode.md § Purpose` and
  `whisper-mode.md § Whisper pill — call header` (bystanders see the
  pill's existence, never names or count).
- **I-4.** Ephemeral channel destruction is unrecoverable. "Keys
  burned" copy is literal: the per-device derived key is scrubbed, no
  "restore" affordance exists, no owner override exists. Source:
  `ephemeral-channels.md § Destruction lifecycle` and the exact copy
  `conversations cannot be recovered`.
- **I-5.** Handoff only targets devices linked to the same identity;
  only those devices appear in the handoff list. There is no "other"
  section, no unknown-device row, no ad-hoc peer device. Source:
  `device-handoff.md § Device-list source` and the invariant copy
  `only devices linked to your identity can accept a handoff.`.
- **I-6.** The local search index is on-device. Queries never leave the
  device, never hit the relay, never produce telemetry. Source:
  `local-search.md § Purpose` ("queries never leave the device").
- **I-7.** OS push payloads carry no plaintext content by default.
  Sender display names and message bodies are not included in the
  system notification body. Source: `sync-queue.md § Privacy` item 2
  and `device-handoff.md § In-progress states` (handoff target push is
  content-free). The opt-in preview path is not yet specified — see §8.
- **I-8.** "Invisible" presence is mutual by design: making yourself
  invisible to a peer hides that peer's presence from you as well.
  Source: `presence.md § State catalog` row `invisible` and
  `presence.md § Self-presence` ("`invisible` is mutual").
- **I-9.** Identity export requires a passphrase the user sets locally.
  Willow never transmits that passphrase; the keyfile is encrypted
  at rest with it. Source: `settings-tweaks.md § Identity + devices →
  export identity` and the helper copy
  `this passphrase only protects your keyfile; willow never sees it.`.
- **I-10.** Whisper gating depends on `PeerTrust` state: unverified
  peers cannot initiate a whisper by default, and a downgraded-from-
  verified peer has whisper and handoff paused until re-compared.
  Source: `whisper-mode.md § Trust gating` and
  `trust-verification.md § Downgrade / re-verify prompts`.
- **I-11.** Verification badges are mandatory on every peer-identifying
  surface (profile card, letter row, members list, message author on
  first-of-run, call participant tile, governance member row). No
  surface may soften, hide, or conditionally suppress the badge.
  Source: `trust-verification.md § Badges → Placement rules by surface`.
- **I-12.** Key rotation or SAS mismatch immediately demotes a peer to
  `Unverified` and raises the `Downgrade / re-verify` banner until the
  user acts. The banner cannot be permanently dismissed without re-
  comparing; a 24 h snooze is the only escape hatch. Source:
  `trust-verification.md § Downgrade / re-verify prompts → Rules`.
- **I-13.** Sync-queue contents are stored encrypted at rest on-device.
  The UI never reads plaintext from the queue except to render it
  inside the running app. Source: `sync-queue.md § Privacy` item 1.
- **I-14.** The relay footer copy in `settings-tweaks.md § network`
  is load-bearing: *"traffic through the relay is still end-to-end
  encrypted. the relay only sees ciphertext."* Any spec that implies
  relay plaintext access violates the threat model.
- **I-15.** Copy nowhere implies that willow (the product / the network
  / the server) sees plaintext. The only actor that sees plaintext is
  the device holding the keys. Source: the build-string footer in
  `settings-tweaks.md § About` — `willow · {version} · peer-to-peer ·
  no server knows your plaintext`.
- **I-16.** Offline is never framed as *failed* in user-visible copy.
  The offline state is "queued", "waiting", "will send when the peer is
  reachable". Source: `README.md § Copy voice` and
  `sync-queue.md § Copy` (`strip_default`, `welcome-back banner` copy).
- **I-17.** Ephemeral extension is bounded: at most one extension per
  channel, capped at the original duration, gated by `ManageChannels`.
  Finality is the point; extension is a delay, not a cancellation.
  Source: `ephemeral-channels.md § Extension`.
- **I-18.** Ephemeral channels never appear in Discover or public grove
  directories, regardless of grove discovery settings. Source:
  `ephemeral-channels.md § Discovery policy`.
- **I-19.** The governance event log surfaces *that* events happened,
  never the plaintext of message bodies. Source:
  `governance.md § Event log` ("surfaces *that* events happened — not
  bodies. Content is sealed.").
- **I-20.** Events whose ed25519 signature fails verification are
  omitted from views; they do not render as "unknown-author" rows or
  similar. Source: `governance.md § Event log` (signature verification
  failure omits the row) and `governance.md § Edge cases → Signature
  verification failure in export` (a meta line declares how many were
  omitted, never their content).

## 3. Per-spec security classification

Every spec file in this folder is listed. The classification is:

- **security-critical** — UX drift would change security properties.
  A small copy change here can enable or mask an attack; these specs
  get the strictest review bar.
- **privacy-sensitive** — UX drift leaks identity or content without
  breaking crypto. Review should still reference §2; the failure mode
  is exposure, not compromise.
- **cosmetic** — UX drift does not affect security. Review is purely
  for design quality and accessibility; this doc does not apply.

Classifications are *at a spec granularity*. A cosmetic spec may contain
a single security-critical clause (e.g. `message-row.md` defers whisper
rendering); when that's the case, the delegating spec inherits its
parent's classification for that subsection only.

| Spec | Classification | Notes |
|------|----------------|-------|
| `README.md` | cosmetic | Parent index; routes specs but makes no security claim of its own. The dependency graph here is load-bearing for review order. |
| `foundation.md` | cosmetic | Tokens, typography, motion. No security surface. Accessibility claims (focus-ring, reduced-motion) are consumed by every security-critical spec. |
| `layout-primitives.md` | cosmetic | Chrome and containers. No security surface; it hosts surfaces defined by other specs. |
| `trust-verification.md` | **security-critical** | Owns SAS grid, badge state machine, compare flow, downgrade banner, holder pill. Drift here changes what "verified" means. Every copy string is security UI; paraphrasing changes what the user thinks they're protecting. |
| `whisper-mode.md` | **security-critical** | Owns whisper key lifecycle, trust gating, consent prompt, leave-and-burn UX. Drift can make whisper content leak to bystanders or persist past teardown. |
| `ephemeral-channels.md` | **security-critical** | Owns timer, burn, extension cap, "keys burned" card. Drift can convert a literal burn into a promise-only deletion. |
| `device-handoff.md` | **security-critical** | Owns linked-device list, re-seal flow, trust gate ("only devices linked to your identity"). Drift can extend the linked-device boundary to an attacker device. |
| `onboarding.md` | **security-critical** | Owns identity creation, first SAS ceremony, recovery. First-run is where the user's mental model sets; a skipped SAS ceremony is a permanently unverified peer. |
| `settings-tweaks.md` | **security-critical** (identity section), privacy-sensitive elsewhere | Identity + devices, export, link, revoke, passphrase, relay override, crypto-visibility default. The Tweaks panel proper is cosmetic; the Settings → Account/Identity/Network sections are critical. |
| `local-search.md` | **security-critical** | Owns the on-device-only search invariant (I-6). A future revision that proposes "search remotely" breaks the model. |
| `presence.md` | **security-critical** | Owns the mutual-invisibility rule (I-8). Drift converts a symmetric hide into an observable leak. |
| `governance.md` | **security-critical** (authority section + event log) / privacy-sensitive (member rows) | Owns permission-grant chain, authority surfacing, signed-event log. Drift hides admin changes or surfaces event bodies. |
| `profile-card.md` | privacy-sensitive | Owns what gossips out: bio, tagline, crest, pinned fragment, pronouns, elsewhere list. All are self-authored, signed, and propagated per the table in `profile-card.md § Edit profile`. Drift expands the propagation surface. See §8 open issue on scope gating. |
| `letters-dms.md` | privacy-sensitive | Owns letter list rows, search entry, verified markers, queued markers. Drift reveals peer identity in queue UI or previews. |
| `sync-queue.md` | privacy-sensitive | Owns queue strip, offline copy, lock-screen payloads (I-7, I-13, I-16). Drift leaks peer identity in notifications or reframes offline as failure. |
| `discover.md` | privacy-sensitive | Owns public grove directory, invite surfaces. Drift can list ephemeral channels (I-18 violation) or leak grove membership without opt-in. |
| `call-experience.md` | cosmetic (layout) / security-critical (whisper + handoff integration points) | Layout, grid/focus/spotlight, controls strip are cosmetic. The slots for the whisper pill and the handoff button delegate to `whisper-mode.md` / `device-handoff.md`; that delegation is critical. |
| `message-row.md` | cosmetic (row rendering) / security-critical (whisper hand-off, mention rendering) | Row styling, density, day separators, author grouping, mention pills, and queue notes are cosmetic. Whisper-message rendering is explicitly handed off to `whisper-mode.md`; the hand-off itself is critical. |
| `composer.md` | cosmetic | Compose textarea, reply/edit bars, mention autocomplete, typing indicator. Placeholder copy per channel kind (text/voice/ephemeral/thread) inherits ephemeral warnings from `ephemeral-channels.md`. |
| `reactions-pins.md` | cosmetic | Reaction picker, pins surface. No security surface. |
| `thread-pane.md` | cosmetic | Thread layout. Thread sealing is inherited from parent channel key (noted in `ephemeral-channels.md § Edge cases`); no decision lives here. |
| `audit.md` | cosmetic | Meta-document comparing specs to the reference bundle. Not shipped UX. |

## 4. Adversary scenarios

Illustrative, not exhaustive. Each scenario names the expected UX
behaviour; deviations from the expected behaviour are bugs, not design
choices.

### 4.1 Network observer (passive global adversary)

Adversary reads every packet on the wire. Can see connection timing,
peer reachability, message sizes, gossip fanout.

**UX expectation.** No UX impact; `willow-crypto` handles content
confidentiality and `willow-identity` handles authentication. Copy
never claims metadata privacy. The `settings-tweaks.md § network`
footer is accurate: *"the relay only sees ciphertext."*

**Specific constraint.** The sync-queue strip (`sync-queue.md`) must
not render peer-identifying URLs or relay addresses in a way that
becomes a shoulder-surf leak. The existing copy limits itself to
display names and counts (`strip_default`, `strip_singular`), which
is correct. Relay address rendering is scoped to the sync-queue screen
popover (`sync-queue.md § Sync queue screen`), not the always-visible
strip.

### 4.2 Compromised relay

Relay operator signs attacker-controlled events and injects them into
the gossip stream, or alters the set of peers it routes to.

**UX expectation.** Because the relay is a regular peer (§1.6), its
events either carry the right signature (from a peer the relay
impersonates — impossible without the peer's key) or the wrong one
(omitted from views per I-20). The holder pill
(`trust-verification.md § Holder pill`) for any channel the relay
holds drops when the relay's `SyncProvider` grant is revoked. An
unverified owner badge appears on the grove if the relay is somehow
claiming owner authority without a valid chain. The governance event
log (`governance.md § Event log`) shows the anomaly — an out-of-order
grant, a new admin, a revoked permission — as a signed entry
authored by whoever actually signed it.

**Copy precedent.** The relay footer in `settings-tweaks.md § network`
and the "relay is a regular peer" framing in `CLAUDE.md § Trust Model`.

### 4.3 Compromised peer device

One of the peer's devices (not the user's) is controlled by an attacker.
The attacker can sign events with that device's sub-key.

**UX expectation.** When the peer rotates their top-level identity key
(legitimate remediation), every verifier receives a `KeyRotated`
`TrustEvent` and fires the downgrade banner
(`trust-verification.md § Downgrade / re-verify prompts`). The peer's
badge flips to `unverified`. Whisper and handoff with that peer become
unavailable until the user re-compares fingerprints (I-10). The banner
copy is literal: *"this peer's key rotated or a fingerprint check
failed. whisper and device handoff are paused until you compare
again."*

Before key rotation, if the attacker signs a message that triggers a
SAS mismatch on a later compare attempt, the same banner fires via
`SasMismatch`. The UI does not auto-block the peer; auto-blocking
punishes the common case (misread words, wrong device). See
`trust-verification.md § Edge cases → Compare → they don't match`.

### 4.4 Lost own device (seized / stolen)

The user's device is out of their control. Attacker may brute-force
the keyfile passphrase or exploit an unlocked device.

**UX expectation.** Recovery flow in `settings-tweaks.md § Identity
+ devices → export identity` and `onboarding.md § recovery` requires
the 12-word mnemonic plus the per-device passphrase. Remote-revoke
flow marks the lost device untrusted:
`settings-tweaks.md § Identity + devices → linked devices` offers
`forget willow·desk? its sub-key stops being trusted in seconds. your
identity lives on your other devices.` A revoked device is removed
from every surface that enumerates linked devices, including the
handoff list (I-5) — the handoff target list hides revoked devices
without any "recover this device" affordance.

If the lost device was the user's *only* device, the UX hard-blocks
the forget action: `settings-tweaks.md § Identity + devices`
dialog titled `this is your only device` with the copy `you'll lose
access to your identity — export first.`. No escape hatch. This
forces the user into export before an irrecoverable state.

### 4.5 Malicious owner

The owner of a grove abuses their admin powers (mass kicks, permission
revokes aimed at silencing dissent, role reassignments).

**UX expectation.** The event log (`governance.md § Event log`)
surfaces every authority change as a signed entry, authored by the
owner. Other admins can still read the log, still emit
`GrantPermission` / governance events subject to their own
permissions, and still raise objections via governance (see
`governance.md § governance tab`). The chain-of-trust projection
(`governance.md § Data dependencies` — `chain-of-trust projection`)
makes the authority path legible: every admin's permission traces
back through grants, visibly, so misuse is observable. There is no
"global moderation" surface that overrides the owner (§5 red-line):
Willow is peer-to-peer, and moderation is per-user (block, leave,
start a new grove).

### 4.6 Ephemeral coercion

The user is compelled (social, legal, threat-based) to reveal the
contents of an ephemeral channel after the timer has expired.

**UX expectation.** Once burned, keys are gone (I-4). There is no
retrieval UI, no "just this once" escape hatch, no owner override.
The destruction flow has no override of any kind; `ephemeral-
channels.md § Destruction lifecycle` describes a one-way transition
from countdown to the `keys burned` card and no other terminus. The
red-line list below (§5) prohibits any future spec from introducing
a recovery affordance.

### 4.7 Notification leakage (bystander shoulder-surf)

A phone sits face-up on a café table; the lock screen is visible to
anyone nearby. A push notification arrives.

**UX expectation.** By default, the OS push payload is content-free
(I-7). Lock-screen / system notification body is limited to
`a letter is waiting` (for letters) or `a message in {grove}` (for
groves) — no sender name, no body. A content preview in the
notification is an *explicit* opt-in the user must toggle on; the
opt-in decrypts locally and writes the plaintext into the payload
the OS displays. The opt-in UX itself is not yet specified — see §8.

## 5. Red-line UX prohibitions

No spec, present or future, may propose any of the following. These
are prohibited even when the proposal would improve a single UX
metric; the threat model ranks above the usability gain. Adding one
requires amending this document first, with explicit reviewer sign-
off on the reasoning.

- **P-1.** A "recover ephemeral channel" flow. The burn is final. No
  owner override, no "just this once" escape, no "contact support".
  Rationale: I-4, §4.6.
- **P-2.** A "silently upgrade unverified peer to verified after N
  messages exchanged" flow. Verification is a ceremony with an
  explicit user decision; inferring trust from message count breaks
  I-2.
- **P-3.** A default-on "include message preview in OS push
  notifications" setting. The default is content-free (I-7); preview
  is an explicit user opt-in.
- **P-4.** A "server-side search" feature, or any search that sends
  the query off-device. Rationale: I-6.
- **P-5.** A "global moderation" surface — Willow-wide bans, cross-
  grove suspensions, platform-level takedowns. Willow is peer-to-
  peer; moderation is per-user (block, leave, start a new grove) or
  per-grove (kick, permission revoke via `governance.md`).
- **P-6.** A handoff that skips the linked-device requirement. No
  "handoff to a friend's device", no "handoff to a browser you're
  about to sign into", no ad-hoc target. Rationale: I-5, §4.4.
- **P-7.** Copy that treats offline as *failed*. "Failed to send",
  "error — not delivered", "can't reach peer". The patient vocabulary
  in `sync-queue.md` is mandatory. Rationale: I-16.
- **P-8.** Copy that implies willow (product / network / server) sees
  plaintext. No "we analysed your messages", no "willow delivered
  your letter" framed as if willow inspected it, no ambiguous "your
  data" that elides who holds it. Rationale: I-15.
- **P-9.** A "trust-on-first-use upgrade" where pending-verify peers
  are auto-promoted if no SAS attempt has been made after N hours.
  Trust is a user decision, not a timeout. The pending-verify badge
  does the work (`trust-verification.md § Open questions` item 7).
- **P-10.** Any UI that hides the verification badge on a peer-
  identifying surface "to reduce noise". Rationale: I-11.
- **P-11.** A relay override UI that does not reconnect. The
  `settings-tweaks.md § network` relay URL override must trigger a
  reconnect and surface success / failure; a silent, unreachable
  relay setting is a silent downgrade.

## 6. How to use this document

This document is a reference. Its consumers are other spec authors, a
reviewer evaluating a proposed change, and any future audit.

### 6.1 For spec authors

- When writing or revising a spec, read §2 (invariants) and §5 (red-
  lines). Every invariant that touches your surface must appear, by
  reference, in the spec's Accessibility or Acceptance sections.
- If an invariant in §2 is *not* reflected in the spec it cites, the
  gap is listed in §8; resolving the gap is a prerequisite for
  shipping the spec.
- Cite by filename, not by section — sections are still churning.
  `(see trust-verification.md)` is acceptable; `(see
  trust-verification.md#downgrade)` risks staleness.

### 6.2 For reviewers

- Check that every new UX surface that identifies a peer carries a
  badge (I-11).
- Check that every new UX surface that renders encrypted content
  does not add a storage / transit path the invariants don't cover
  (I-13).
- Check that any new copy about network state does not frame offline
  as failure (I-16) or imply willow sees plaintext (I-15).
- Check that any new authority-surfacing UX traces back to a signed
  event (I-19, I-20) and does not add a "we think this is correct"
  inference.

### 6.3 For the veto

When a proposed spec change appears to conflict with §2 or §5, this
document is the veto: the change is rejected, or §2/§5 is amended
first. The amendment itself requires the process in §7.

## 7. Amendment process

Willow does not currently have a formal spec-review process; CLAUDE.md
records only the `audit.md` comparison against the reference bundle.
This document proposes a lightweight addition: changes to §1 through
§5 require a **second human reviewer** sign-off, noted in the commit
message, before merging. The first reviewer is the author; the second
reviewer is anyone else who has read this document end-to-end.

Changes to §3 (per-spec classification) should accompany the spec
change they track; a spec moving from `cosmetic` to `privacy-
sensitive` is an amendment, not a correction. Changes to §8 (open
issues) are routine — close them as the referenced specs catch up.

The intent is not bureaucratic overhead. It is to make every
invariant change visible to a second pair of eyes, because one
reviewer writing both the spec and the invariant change cannot
self-audit.

## 8. Open issues

Entries here are claims in §2 or §4 that conflict with, or are not
yet reflected in, the specs they cite. Each either resolves by
updating the spec or by amending the invariant. Do not silently
invent; do not silently drop.

- **O-1. Profile-field gossip is not scope-gated.** I-claim "Profile
  fields (bio, tagline, crest, pinned fragment) gossip to verified
  peers first; unverified peers see them only with explicit opt-in"
  was drafted for the invariants section of this document but is
  **not** yet reflected in `profile-card.md § Edit profile`. That
  spec's propagation table currently marks every new-state field as
  unconditionally "yes — propagates to grove peers". The invariant is
  therefore aspirational, not enforced. Resolution: either
  `profile-card.md` gains a "gated by verification" row on each
  propagating field, or this invariant is removed. Tracking: needs
  clarification before onboarding (`onboarding.md § step 3`) lands,
  since the first-grove flow exposes profile fields to peers who are
  pending-verify.
- **O-2. Notifications spec does not exist.** I-7 cites an OS push
  payload invariant; the underlying copy exists in
  `sync-queue.md § Privacy` and `device-handoff.md § In-progress
  states`, but there is no dedicated `notifications.md` defining the
  opt-in preview flow. `settings-tweaks.md § Notifications` covers
  per-surface switches and quiet hours; it does not currently include
  a "show preview in lock-screen" toggle. Resolution: either add a
  row to `settings-tweaks.md § Notifications` for the preview opt-in,
  or write a dedicated `notifications.md` spec. Until then, the
  invariant "opt-in preview decrypts locally" is not UX-testable.
- **O-3. Governance event log export is client-side but unsigned.**
  I-claim "Event log export is signed by the exporting device's
  keypair so receivers can verify provenance" does not match the
  current `governance.md § Data dependencies` entry, which states
  *"event log JSONL export — purely client-side; PGP-signed export
  deferred."* Resolution: the current spec's position is weaker than
  the invariant. Either `governance.md` promotes signed export from
  deferred to v1 (and the invariant stands), or the invariant is
  revised to match reality (unsigned export, with an advisory banner
  that receivers cannot verify provenance). The latter is less
  defensible in §4.5 (malicious owner) scenarios, where a user
  exporting the log to a third party wants provenance.
- **O-4. Whisper history retention is stated as UX promise only.**
  `whisper-mode.md § Open questions` item 5 asks "confirm with
  `willow-messaging` storage semantics before plan-writing." The
  teardown copy *"whisper ended — nothing is saved beyond what you
  already saw."* assumes no disk persistence of whisper message
  bodies past the session. If `willow-messaging` stores whisper
  bodies in the local encrypted-at-rest queue, the copy is
  misleading. Resolution: confirm storage semantics; if bodies are
  persisted, either the copy changes or the persistence is excised
  from the whisper path. Until resolved, I-3 is true for *over-the-
  wire* confidentiality but ambiguous for *at-rest* persistence.
- **O-5. Device linking ceremony is not yet fully specified.**
  §1.2 (linked devices) and I-5 (handoff only to linked devices)
  depend on a linking ceremony described in passing in
  `settings-tweaks.md § Identity + devices → linked devices`. The
  exact flow (QR-scan, mnemonic-cross-sign, out-of-band verification)
  is not pinned down, and `settings-tweaks.md § Open questions`
  item 8 flags identity-settings sync between own devices as
  unresolved. Resolution: until the linking UX is specified, "linked
  devices" is a shape of trust without a rigorous on-ramp. The
  adversary scenario in §4.4 (lost own device) is describable but
  not fully testable against a flow.
- **O-6. Verified-peer downgrade grace period is open.**
  `trust-verification.md § Open questions` item 8 proposes a 60 s
  auto-re-enable of whisper/handoff after `KeyRotated`. I-10 and
  I-12 assume no grace period. Resolution: the current "leaning no"
  is consistent with the invariants; if the grace period ships, the
  invariants need to relax to "paused until re-verified *or* the
  configured grace window expires". The safer default is to keep the
  door closed.
- **O-7. Ephemeral clock-skew edge case and offline-at-burn copy.**
  `ephemeral-channels.md § Edge cases → Participant offline at
  destruction` surfaces the copy `keys burned on all online devices
  — will burn on others when they reconnect`. I-4 says destruction is
  unrecoverable; this is still true per-device, but the observable
  state across the peer set is momentarily divergent (some devices
  have burned, some haven't). The invariant as stated is correct
  per-device but does not capture the multi-device convergence
  window. Resolution: clarify in §2 that I-4 is a *per-device*
  property reached eventually on every participating device, not a
  network-instantaneous property. Non-critical but worth an
  invariant-text refinement.
- **O-8. "Compromised peer device" scenario depends on a KeyRotated
  event stream that is proposed, not yet built.**
  `trust-verification.md § Data dependencies → Key rotation` lists
  `views.trust_events: ReadSignal<Vec<TrustEvent>>` as **to add**.
  The downgrade UX in §4.3 depends on this stream. Resolution: the
  scenario is accurate as a *target* behaviour; the client
  implementation of the `TrustEvent` stream is a prerequisite for the
  invariant being testable.

---

## Appendix A — glossary of citations

Every invariant and scenario in this document cites a spec by
filename. The specs live in `/mnt/storage/projects/willow/docs/specs/
2026-04-19-ui-design/`. For each cited filename, the owning concept:

- `trust-verification.md` — SAS, badges, compare flow, downgrade
  banner, holder pill.
- `whisper-mode.md` — whisper side-channels, trust gating, teardown.
- `ephemeral-channels.md` — finite-lifespan channels, burn, extension.
- `device-handoff.md` — move an in-progress call between own devices.
- `onboarding.md` — first-run identity, first SAS, first grove.
- `settings-tweaks.md` — account, identity, devices, privacy,
  notifications, network, appearance, about + the Tweaks panel.
- `local-search.md` — on-device search primitive.
- `presence.md` — peer status atom (`StatusDot`, `PeerStatusLabel`),
  mutual invisibility.
- `governance.md` — authority, management, event log.
- `profile-card.md` — enriched profile with crest, bio, pinned
  fragment.
- `letters-dms.md` — peer + group letters, verified markers.
- `sync-queue.md` — offline queue, lock-screen notifications.
- `discover.md` — public grove directory.
- `call-experience.md` — grove call layout, controls strip.
- `message-row.md` — row styling, density, day separators, author
  grouping, mentions, code, queue notes, scroll anchor.
- `composer.md` — compose textarea, reply / edit bars, autocomplete,
  typing indicator.
- `reactions-pins.md` — reactions strip, emoji picker, pins action
  and panel.
- `files-inline.md` — file cards, inline images, voice notes, upload
  dialog, drag-and-drop.
- `thread-pane.md` — thread layout.
- `foundation.md` — tokens, typography, motion, copy voice.
- `layout-primitives.md` — desktop three-pane, mobile tab chrome,
  bottom sheets, popovers.
- `audit.md` — spec-vs-bundle meta document.
- `README.md` — parent index + dependency graph.
