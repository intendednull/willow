# Audit — UX specs vs reference design bundle

Reviewed: 2026-04-19
Reviewer: fresh audit agent
Specs: `docs/specs/2026-04-19-ui-design/` (parent + foundation + 15 children)
Bundle: `docs/reference-designs/2026-04-19-willow-design-bundle.tar.gz`
(extracted at `/tmp/willow-design/files/willow/`)

## Verdict

**approve-with-fixes.** Screen coverage, novel-mechanic coverage, and
token discipline are strong. The parent README's dependency-graph tree
is out of sync with child declarations (major), a few bundle copy
strings are not carried forward verbatim (minor), and swipe-to-reply
is named as a novel mechanic in the chat transcript but is not spec'd
distinct from swipe-to-thread (minor).

## 1. Screen coverage (desktop + mobile)

### Mobile screens (`src/mobile/m_*.jsx`)

| Bundle file | Spec coverage | Status |
|---|---|---|
| `m_app.jsx` (shell + tour) | `layout-primitives.md` §Mobile shell (bottom tab bar) | covered |
| `m_home.jsx` (grove+channels list) | `layout-primitives.md` §Home screen; `messaging.md` channel list feeds this | covered |
| `m_chat.jsx` (channel chat + swipe-to-thread + pull-down sync) | `messaging.md` §Message row — mobile (swipe-right-to-thread L138), `sync-queue.md` §Pull-to-reveal | covered |
| `m_thread.jsx` | `thread-pane.md` §Mobile — full-screen push | covered |
| `m_call.jsx` (+ whisper sheet, handoff sheet) | `call-experience.md` §Mobile, `whisper-mode.md`, `device-handoff.md` | covered |
| `m_dms.jsx` (peer + group letters, swipe actions) | `letters-dms.md` §Mobile, §Swipe actions L403 | covered |
| `m_discovery.jsx` | `discover.md` §Mobile | covered |
| `m_onboarding.jsx` (long-press SAS verify) | `onboarding.md`, `trust-verification.md` §Long-press SAS on mobile L351 | covered |
| `m_settings.jsx` | `settings-tweaks.md` §Mobile L396 | covered |
| `m_ephemeral.jsx` (lock-screen timer + duration picker) | `ephemeral-channels.md` §Creation flow L199-225, §Lock-screen timer | covered |
| `m_sync.jsx` (dedicated sync queue screen) | `sync-queue.md` §Dedicated screen | covered |
| `m_profile_sheet.jsx` (bottom-sheet wrapper) | `profile-card.md` §Mobile bottom sheet L163 | covered |
| `m_primitives.jsx` | implicit in `layout-primitives.md`; every child consumes its atoms | covered |
| `android-frame.jsx` / `ios-frame.jsx` (status bar, tab bar chrome) | `layout-primitives.md` §Platform chrome L285-303 | covered |

No mobile screen is missing a spec owner.

### Desktop screens (`src/*.jsx`)

| Bundle file | Spec coverage | Status |
|---|---|---|
| `app.jsx` (shell, rail, tweaks wiring) | `layout-primitives.md` + `settings-tweaks.md` (Tweaks wiring) | covered |
| `chat.jsx` | `messaging.md` (body, hover toolbar, author, reactions) | covered |
| `sidebar.jsx` (grove rail + channel sidebar + grove tagline) | `layout-primitives.md` §Grove rail L57-130, §Channel sidebar | covered |
| `thread.jsx` | `thread-pane.md` §Desktop right-rail | covered |
| `members.jsx` | `layout-primitives.md` §Right rail — members L125, L477 — but not a standalone spec. No dedicated behaviour spec (steward tag, status text, queued counter). | partial — no dedicated owner |
| `call.jsx` | `call-experience.md` | covered |
| `dms.jsx` | `letters-dms.md` | covered |
| `discovery.jsx` | `discover.md` | covered |
| `onboarding.jsx` | `onboarding.md` | covered |
| `settings.jsx` | `settings-tweaks.md` §Settings account | covered |
| `governance.jsx` | `governance.md` | covered |
| `roles_invites_files.jsx` | `governance.md` §Manage — roles + invites + files (refs to roles/invites/files, 34 matches) | covered |
| `eventlog.jsx` | `governance.md` §Event log L173, L347, L426, L445 | covered |
| `profile_popover.jsx` | `profile-card.md` §Desktop popover | covered |
| `tweaks.jsx` | `settings-tweaks.md` §Tweaks panel | covered |
| `primitives.jsx` (Avatar, PeerName, Tag, StatusDot, Wordmark) | implicit across specs; `layout-primitives.md` + `profile-card.md` (PeerName clickable handoff); `trust-verification.md` (VerifiedBadge) | covered |
| `data.jsx` (ChannelKind incl. `ephemeral`) | `ephemeral-channels.md` (kinds: text / voice / ephemeral listed in parent README terminology) | covered |
| `icons.jsx` | `foundation.md` §Iconography L175-193 enumerates the set | covered |

Only *partial*: the members rail has no dedicated spec. Its behaviour
is folded into `layout-primitives.md` and `profile-card.md`, which is
defensible, but bundle `members.jsx` includes member-row structure,
steward tag, and queued-count meta that are not named as a coherent
unit in any single child.

## 2. Shared atoms coverage

| Bundle atom | Owning spec | Status |
|---|---|---|
| `shared/messaging.jsx` (mentions, reactions, badges) | `messaging.md` §Hover toolbar L221, §Mentions, §Reactions | covered |
| `shared/sas.jsx` (6-word grid + SAS_COPY) | `trust-verification.md` §Fingerprint grid L88-91, §Labels L103, §Copy L383-387 | covered (with minor copy drift — see §4) |
| `shared/call_atoms.jsx` (ScreenShareContent, WhisperPill, HANDOFF_DEVICES, SPEAKING_STATS) | `call-experience.md` (screen-share + speaking stats) + `whisper-mode.md` (whisper pill) + `device-handoff.md` (device list) | covered |
| `shared/thread_atoms.jsx` (ThreadParentCard, ThreadReplyList, THREAD_COPY) | `thread-pane.md` §Parent card, §Reply list, §Sealed footer | covered |
| `shared/profile_card.jsx` (ProfileCardContent, ProfileCrest, PROFILE_COPY, event bus) | `profile-card.md` §Content L28-40, §Crest | covered |

No shared atom is orphaned.

## 3. Novel mechanics coverage

From the chat transcript (`chats/chat1.md`) the user explicitly called
out these novel mechanics.

| Mechanic | Spec | Status |
|---|---|---|
| SAS / fingerprint short-codes | `trust-verification.md` §Fingerprint grid + §Labels | covered |
| Offline sync queue (see what's pending) | `sync-queue.md` | covered |
| Local-first search across history | `letters-dms.md` L33 (list search) | partial — named only; no dedicated §Search copy, hotkey, scope (across-letters-and-channels) behaviour. No equivalent in `messaging.md`. |
| Ephemeral / self-destructing channels | `ephemeral-channels.md` | covered |
| Per-channel crypto settings (who holds keys) | `trust-verification.md` §Holder pill L313, §Crypto strip, §ChannelHolders view L456 | covered |
| Device-to-device call handoff | `device-handoff.md` | covered |
| Whisper mode | `whisper-mode.md` | covered |
| Swipe-to-reply / swipe-to-thread (mobile) | `messaging.md` §Mobile L138 — swipe-right-to-thread covered | partial — swipe-to-reply is listed alongside swipe-to-thread in the chat transcript novel-mobile list; only swipe-to-thread is spec'd. `letters-dms.md` spec'd right-to-left swipe on letter rows for archive / mute (§L403) but not a swipe-to-reply on a message row. |
| Long-press SAS verify (mobile) | `trust-verification.md` §Long-press SAS on mobile L351 | covered |
| Pull-down offline sync queue (mobile) | `sync-queue.md` §Pull-to-reveal L193 | covered |
| Lock-screen-style ephemeral timer (mobile) | `ephemeral-channels.md` §Mobile lock-screen clock L170; L230 creation warn copy | covered |
| Verified-peer badges | `trust-verification.md` §Badges + `profile-card.md` | covered |
| Queued flag + queueNote (per-peer + per-message) | `sync-queue.md` + `messaging.md` §Badges (`synced from queue`, `queued`) L515 | covered |

Only one novel-mechanic gap: **local-first search** is named but not
given dedicated copy, scope (letters-wide? channel-scoped?), hotkey
(desktop), or empty/loading-state behaviour.

## 4. Copy drift (sampled)

| String in bundle | Source | Spec | Verbatim? |
|---|---|---|---|
| `compare six words on two screens. if they match, no one can impersonate either of you in this conversation, ever.` | `shared/sas.jsx` `SAS_COPY.intro` | `trust-verification.md` L379 (`SAS.intro`) | verbatim |
| `your fingerprint — read this aloud` | `shared/sas.jsx` `FingerprintLabel which=you` | `trust-verification.md` L88, L103, L386 | verbatim |
| `their fingerprint — do these match?` | `shared/sas.jsx` `FingerprintLabel which=peer` | `trust-verification.md` L89, L104, L387 | verbatim |
| `they match` | `SAS_COPY.matchCta` | `trust-verification.md` L167, L383 | verbatim |
| `they don't match` | `SAS_COPY.noMatchCta` | `trust-verification.md` L169, L384 | verbatim |
| `add a friend` | `SAS_COPY.title` + `onboarding.jsx` Step 3 label | nowhere in spec (`add a friend` string absent) | **drift — dropped**. Onboarding step name is not preserved and the compare-dialog title has been renamed to `compare fingerprints` / `compare now`. The three-word title is load-bearing in the bundle ("add a friend" is the whole social frame of SAS) and should at minimum be documented as intentionally reworded. |
| `move this call` | `shared/call_atoms.jsx` `HANDOFF_COPY.title` | `device-handoff.md` L63, L115, L304 | verbatim |
| `keys re-seal automatically on the new device. no re-join.` | `HANDOFF_COPY.subtitle` | `device-handoff.md` L67, L118, L209, L305 | verbatim |
| `sealed to thread participants (${n}) — not the whole grove` | `shared/thread_atoms.jsx` `THREAD_COPY.sealedFooter` | `thread-pane.md` L220, L321, L460 renders as `sealed to <N> thread participants — not the whole grove` | **drift — word order**. Bundle puts count in parens after `participants`; spec moves count before `thread participants`. Same semantics, different rhythm. |
| `reply to thread…` | `THREAD_COPY.composePlaceholder` | `thread-pane.md` — not found verbatim (composer placeholder implied but not quoted) | **drift — absent**. Spec describes the composer but does not quote this placeholder. |
| `whisper · ori` / `whisper · ori, juno +2` | `WhisperPill` + dynamic label | `whisper-mode.md` L79-81 | verbatim |
| `synced from queue` / `queued` | `shared/messaging.jsx` `QueuedBadge` | `messaging.md` L515; `sync-queue.md` passim | verbatim |
| `reaching out…` | `m_sync.jsx` banner | `sync-queue.md` L240, L426 (`screen_card_label`) | verbatim |
| `until keys burn` | `m_ephemeral.jsx` lock-screen meta | `ephemeral-channels.md` L170 `"until keys burn"` | verbatim |
| `a single-use key is forged now and burned when the timer ends. messages can't be recovered.` | `m_ephemeral.jsx` callout | `ephemeral-channels.md` — key-forge callout is described but this exact sentence is not preserved | **drift — absent**. The creation warning in spec L230/L353 uses `this channel will self-destruct in {duration} — keys burned on every device.` which is a *different* sentence. The `forged / burned` framing is lost. |
| `not a server — held between us` | `sidebar.jsx` grove tagline (added in chat round 3) | `layout-primitives.md` / parent README — not found verbatim | **drift — absent**. Grove header tagline is a deliberate teammate-requested reinforcement of the metaphor; it belongs somewhere. |
| `in the grove since` / `you share` / `you call them` / `pinned fragment` / `elsewhere` / `this is you` | `shared/profile_card.jsx` `PROFILE_COPY` | `profile-card.md` (search confirmed presence of `pinned fragment`, `shared groves`, `elsewhere`, `since`, `known as`, `self`) | covered; exact tokens adopted but worded as section labels — verbatim. |

**Net:** 4 mild drifts. None are structural, but each drops an
intentionally-chosen piece of bundle voice.

## 5. Token discipline

Grep for hex values outside `foundation.md` turned up:

| Spec | Hex | Reading |
|---|---|---|
| `letters-dms.md:107`, `letters-dms.md:627` | `#14130f` | Call-back to `--bg-0` as the *foreground* ink on the amber unread pill. This is the documented foreground-on-accent choice established in `app.jsx` and `call_atoms.jsx` (see `primitives.jsx` Avatar `color: '#14130f'`). Acceptable as explicit ink-on-accent. |
| `trust-verification.md:167`, `339`; `onboarding.md:99`; `messaging.md:368`; `call-experience.md:129`, `216`; `layout-primitives.md:102`, `103`, `162`, `293` | `#14130f` | Same pattern — foreground ink on `--moss-2` / `--err` / `--amber` buttons. Consistent. |
| `README.md:64` | `#a88fc9` | Parenthetical reference to whisper token value. Load-bearing identity (whisper is not accent-swappable). Acceptable as in-prose documentation. |

No spec redefines a token. No ad-hoc colour invented. Pass.

Token discipline is cleaner than I expected — the `#14130f` repetition
is a light code-smell (it could be exposed as `--ink-on-accent`), but
that is a foundation-level nit, not a spec-level defect.

## 6. Dependency graph consistency

The parent README's dependency tree (L126-143) is *not* a faithful
union of child declarations. The tree reads as a topologically-sorted
subset; children declare richer DAG edges.

| Child | Child declares | Parent tree edge | Mismatch |
|---|---|---|---|
| `call-experience.md` | foundation, layout-primitives, whisper-mode, device-handoff, messaging | call-experience → whisper-mode, device-handoff (children of call) | inverted: child says it *depends on* whisper-mode + device-handoff; parent shows the reverse. Also child depends on messaging; parent omits. |
| `device-handoff.md` | foundation, call-experience, settings-tweaks | under call-experience | settings-tweaks dependency missing from parent. |
| `whisper-mode.md` | foundation, layout-primitives, call-experience, letters-dms, messaging, trust-verification | under call-experience | letters-dms, messaging, trust-verification deps all missing from parent. |
| `discover.md` | foundation, layout-primitives, letters-dms, governance, trust-verification | `(depends on foundation only)` | **False.** Parent claims foundation-only; child declares 5 deps. |
| `governance.md` | foundation, layout-primitives, profile-card, trust-verification | `(depends on layout-primitives)` | profile-card, trust-verification missing from parent. |
| `onboarding.md` | foundation, trust-verification, profile-card, letters-dms, discover | `(foundation + trust-verification)` | profile-card, letters-dms, discover missing from parent. |
| `settings-tweaks.md` | foundation, layout-primitives, profile-card, trust-verification, device-handoff, whisper-mode | `(depends on foundation)` | **False.** Parent claims foundation-only; child declares 6 deps. |
| `ephemeral-channels.md` | foundation, layout-primitives | parent: `(layout-primitives + messaging)` | parent adds `messaging` that child does not list. |
| `profile-card.md` | foundation, layout-primitives | parent: profile-card → trust-verification | direction inverted; trust-verification declares profile-card as a *consumer surface only*. |
| `trust-verification.md` | foundation, profile-card | parent: profile-card → trust-verification | consistent in direction but parent lists profile-card as parent of trust-verification, while trust-verification spec calls profile-card "only a surface consumer for the badge" — edge is over-stated. |

These are not blocking issues (no cyclic dependencies), but **the
parent graph is misleading**. It should either be rebuilt as a full
DAG or marked as "informal topological view; see child headers for the
true dependency set."

## 7. Internal contradictions

No substantive contradictions found on spot-check of the four
cross-cutting claims:

- **Terminology.** Parent glossary (`grove`, `letter`, `whisper`,
  `fingerprint`, `letter of introduction`, `queued`, `handoff`,
  `ephemeral`) is used consistently across every child. Grep for
  `server` (bundle legacy term) outside quoted/grep examples turned
  up no instances in user-visible child copy.
- **Whisper gating on trust.** `whisper-mode.md` L316-328 says whisper
  is gated to verified peers by default, with a Tweaks toggle
  `allow whispers from unverified peers` (off by default).
  `trust-verification.md` L396 says "whisper and device handoff stay
  closed until the fingerprints match." Consistent.
- **Ephemeral expiry range.** `ephemeral-channels.md` uses
  `1h / 6h / 24h / 3d / 7d` (L199-225, L521), matching the
  `m_ephemeral.jsx` preset array `[1, 6, 24, 72, 168]`.
- **Handoff device list ownership.** `device-handoff.md` says only
  devices linked to the same identity can accept (L17, L315).
  `call_atoms.jsx` backs this with a fixed three-device list all
  prefixed `willow · …`. Consistent.

One **small contradiction**: `profile-card.md` declares
`layout-primitives.md` as a dependency (L4), but
`trust-verification.md` declares `profile-card.md` is *only* a
consumer of the badge — then `profile-card.md` itself §Consumed-by
lists `trust-verification.md` as a consumer of the profile card. So
the two specs view each other as downstream consumers of the other.
That is circular only in documentation, not in implementation
(trust-verification just *renders* its SAS grid inside the
profile-card popover shell), but the framing should be cleaned up.

## 8. Invented UX

Mostly faithful. A few extensions that go beyond the bundle, with my
read on whether they are "edge-case" (fine) or "structural" (flag):

| Invention | Spec | Verdict |
|---|---|---|
| `set nickname` inline editor on profile card (peer view) | `profile-card.md` §Scope | **Grounded.** Bundle's `PROFILE_COPY.editNickname` = `set nickname` and the third secondary action. Spec upgrades it from a button label to an editor flow — edge extension, fine. |
| `downgrade / re-verify` banner on key change | `trust-verification.md` L397-399 (`Downgrade.title = keys changed — verify again`) | **Edge extension.** Bundle has no explicit "keys changed" banner, but the verified/unverified pair in data and `PROFILE_COPY.unverified` copy naturally imply this flow. Fine. |
| `event log JSONL export` | `governance.md` L445, L554 | **Edge extension.** Bundle has an event log view (`eventlog.jsx`) but no export; spec extends this with a signed-JSONL export. Within scope. |
| `letter of introduction sheet` with seal + policy fields | `governance.md` L275 | **Grounded.** Bundle's `roles_invites_files.jsx` has an invites tab; extending to a seal/policy form is natural. |
| Expired-invite copy — `this letter of introduction has expired — ask the…` | `onboarding.md` L524, L559, L656 | **Edge extension.** Bundle does not show an expired-invite state; the copy matches the grove voice. Fine. |
| Density variants (`cozy / balanced / dense`) | `foundation.md` §Density; `settings-tweaks.md` | **Grounded.** `chat1.md` captures density as a questions_v2 answer (`balanced`). Extending to three modes is a light extension. |
| Members pane as a first-class surface with no standalone spec but explicit behaviour in `layout-primitives.md`, `profile-card.md`, `thread-pane.md` | — | **Split ownership.** Not invented; just under-owned. See §1. |

No whole new screens invented. No new mechanics invented. The specs
are disciplined on scope.

## 9. Accessibility coverage

All 15 child specs plus foundation and README contain an
`## Accessibility` section (grep confirmed every filename under
`docs/specs/2026-04-19-ui-design/` hits the header).

Spot-checks for the three required notes (keyboard, labels,
reduced-motion):

- `foundation.md` §Accessibility L301-315 — all three named.
- `messaging.md` keyboard + `aria-label` list + `prefers-reduced-motion`
  notes present.
- `layout-primitives.md` L596 "All interactive elements ≥ 44 × 44 CSS
  px"; drawer & sheet keyboard equivalents; reduced-motion clause.
- `trust-verification.md` L533-578 explicit `they match` / `they
  don't match` buttons, focus trap, `role="dialog"`, dialog label.
- `device-handoff.md` L424-438 aria-labels quoted; reduced-motion in
  L496.
- `ephemeral-channels.md`, `sync-queue.md`, `profile-card.md`,
  `whisper-mode.md`, `call-experience.md`, `letters-dms.md`,
  `thread-pane.md`, `discover.md`, `onboarding.md`,
  `settings-tweaks.md`, `governance.md` — all hit the header; not
  exhaustively spot-checked for quality but headers + labels are
  present. No spec without it.

Pass.

## 10. Recommended fixes

### Blocker
(none)

### Major

1. **Rebuild the parent dependency graph.** README L126-143 is not a
   faithful DAG of child declarations. Either (a) expand the tree to
   a DAG enumerating every edge declared by a child, or (b) label
   the current figure as "informal topological order; see each
   child's header for its real dependency set." The most egregious
   mismatches are `discover` (parent says foundation-only; child
   declares 5 deps), `settings-tweaks` (parent says foundation-only;
   child declares 6), and `call-experience` (direction of whisper
   and handoff dependencies is inverted).

### Minor

2. **Preserve `add a friend`.** The bundle uses `add a friend` as
   (a) the onboarding step-3 label and (b) the SAS dialog title.
   `trust-verification.md` and `onboarding.md` both rename it to
   `compare fingerprints` / `compare now`. If the rename is
   intentional, add a note; if it is oversight, restore the string.
2. **Restore the grove tagline.** `sidebar.jsx` carries an italic
   tagline `not a server — held between us` beneath the grove peer
   count. Neither `layout-primitives.md` (grove header) nor the
   parent README quotes it. Add to `layout-primitives.md` §Channel
   sidebar.
3. **Align the thread sealed-footer string.** Bundle
   `THREAD_COPY.sealedFooter(n)` renders `sealed to thread
   participants (n) — not the whole grove`. `thread-pane.md`
   renders `sealed to <N> thread participants — not the whole
   grove`. Pick one and make it canonical in the Labels table and
   in every prose reference (L220, L321, L460).
4. **Quote `reply to thread…` as the thread composer placeholder** in
   `thread-pane.md` §Composer.
5. **Adopt the `forged now and burned` framing** for the ephemeral
   creation warn card (or explicitly reject it). Bundle callout:
   `a single-use key is forged now and burned when the timer ends.
   messages can't be recovered.` Spec currently uses different
   wording.
6. **Add a dedicated sub-section on local-first search** to
   `letters-dms.md` or `messaging.md`. It is named in the novel
   mechanics list but has no copy, scope, or keyboard shortcut.
7. **Document swipe-to-reply on message rows** if intended. The
   chat transcript calls out *Swipe-to-reply / swipe-to-thread* as
   co-equal mobile gestures; spec only covers swipe-right-to-thread.
   Either add a matching swipe-left-to-reply gesture to `messaging.md`
   §Mobile, or explicitly reject it and note that swipe-to-reply was
   collapsed into swipe-to-thread.
8. **Clarify profile-card ↔ trust-verification direction.**
   `profile-card.md` lists `trust-verification.md` as a consumer;
   `trust-verification.md` lists `profile-card.md` as a dep (profile
   is a consumer surface). They are not quite contradictory but
   they read as mutually-downstream. Settle on: trust-verification
   *exposes* the SAS + badge atoms; profile-card *renders* the badge
   and opens the compare flow.
9. **Members pane ownership.** Decide whether the members rail needs
   its own short spec or whether `layout-primitives.md` L125-130,
   L477 plus `profile-card.md` avatar-click contract are sufficient.
   `bundle/members.jsx` has specific detail (steward tag row,
   `<queued> queued` meta) that isn't quite nailed down in a single
   place.

### Nit

10. **Consider `--ink-on-accent` token.** Every button with
    `--moss-2` / `--amber` / `--err` background uses `#14130f` for
    foreground ink. It appears 10+ times across 7 specs. Promoting
    to a token in foundation would close the last "ad-hoc hex"
    channel, tighten accent-swap behaviour, and remove the only
    reason children currently quote hex values.
11. **Explicitly document `not a server` grove vocabulary.** The
    sentence in the chat transcript ("instead of discord servers we
    have willow groves") and the bundle's added sidebar tagline
    belong together. Parent README L79 already calls grove the
    user-visible term; one italicized tagline quote in
    `foundation.md` copy voice (or `layout-primitives.md` sidebar
    section) would pin it.
