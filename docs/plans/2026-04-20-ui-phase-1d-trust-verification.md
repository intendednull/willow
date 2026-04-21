# UI Phase 1d — Trust Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development + superpowers:test-driven-development per task.

**Goal:** `docs/specs/2026-04-19-ui-design/trust-verification.md` — SAS fingerprint grid, verified/unverified/pending/new-peer badges on every peer surface, `add a friend` compare flow (desktop modal + mobile sheet), long-press SAS on mobile, holder pill on channel header, downgrade banner, `crypto_visibility` modes.

**Style ref:** `docs/plans/2026-04-19-ui-phase-0-foundation.md`. Commits: `ui(phase-1): <imperative>`.

**Branch:** `design/ui-target-ux`. Lands after 1a/1b/1c.

## Scope

**In:** trust-verification.md in full — SAS derivation, WebTrustStore, badges, compare dialog, long-press + keyboard, holder pill + list, downgrade banner, crypto_visibility tweak.

**Out:** onboarding first-SAS (Phase 5), profile-card chrome internals, whisper/handoff gating (later), cross-locale word lists, shared-trust EventKind, `not sure` CTA (flagged off).

## Architecture

Trust is **local per-device belief** — no new `EventKind`. `willow-crypto::sas` produces deterministic symmetric 6-word fingerprints via blake3 + domain-separation tag. `WebTrustStore` holds `PeerTrust` in `localStorage`. `ClientHandle::verify_peer / mark_unverified / begin_compare` shims (wasm32-only) delegate to the store. Holder count is a derived signal over existing `ServerState::channel_keys`.

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/crypto/src/sas.rs` | **new** | `sas_words(session_key, a, b) -> [String; 6]` via blake3 + DS_TAG `b"willow-sas-v1"`. Symmetric in (a,b). WASM-safe. |
| `crates/crypto/src/sas_wordlist.rs` | **new** | 2048-entry Willow word list `pub static WORDS: [&str; 2048]`. Alphabetised, lowercase ASCII. `SAS_WORDLIST_HASH`. |
| `crates/crypto/src/lib.rs` | modify | Declare modules + re-export `sas::{sas_words, SasError}`. |
| `crates/crypto/Cargo.toml` | modify | Add `blake3 = { workspace = true }`. |
| `crates/client/src/trust.rs` | **new** | `TrustStore` trait + `PeerTrust` enum + `UnverifiedReason`. |
| `crates/web/src/trust_store.rs` | **new** | `WebTrustStore` impl of `TrustStore` trait. localStorage-backed. Serde round-trip. |
| `crates/web/src/state.rs` | modify | Add `TrustState` bucket + `CryptoVisibility` enum. `compare_target: Option<String>` signal. Initialize from `WebTrustStore::load()`. |
| `crates/web/src/components/sas.rs` | **new** | `FingerprintLabel` + `FingerprintGrid` atoms. Sizes sm/md. Variants You/Peer/Matched/Mismatch. `SAS_COPY` byte-exact constants. |
| `crates/web/src/components/trust_badge.rs` | **new** | `<TrustBadge peer_id, size>`. Five states: verified disk/pill, unverified dashed-ring disk/pill, pending-verify `?` + compare chip, new peer label, downgraded (same as unverified). Each carries exact `aria-label`. |
| `crates/web/src/components/add_friend.rs` | **new** | `<AddFriendDialog open: Option<String>, on_close>`. Desktop modal + mobile bottom sheet (media query). Screens: Compare → ConfirmMatch / ConfirmMismatch. Mobile peer card top per spec §Screen 1. |
| `crates/web/src/components/long_press.rs` | **new** | `<LongPressAvatar peer_id, children, on_trigger>`. 350ms pointerdown+hold. Growth ring. Keyboard Enter equivalent. Haptic vibrate(8). |
| `crates/web/src/components/holder_pill.rs` | **new** | `<HolderPill channel_id>` + `<HolderList>`. Reads `app_state.chat.holder_counts` (derive from `ServerState::channel_keys` len). Respects `crypto_visibility`. |
| `crates/web/src/components/downgrade_banner.rs` | **new** | `<DowngradeBanner peer_id, on_compare>`. Renders when `PeerTrust::DowngradedFromVerified`. Dashed `--warn` border. `dismiss for now` → 24h `localStorage` key `willow.downgrade-dismiss.<peer_id>`. |
| `crates/web/src/components/member_list.rs` | modify | Mount `<TrustBadge peer_id size=Disk12>` after display name. Keep legacy Trust/Untrust admin buttons unchanged. |
| `crates/web/src/components/sidebar.rs` (or wherever me strip lives post-1a) | modify | Add `<TrustBadge>` adjacent to own avatar. |
| `crates/web/src/components/message.rs` | modify | Add `<TrustBadge size=Disk12>` on first-of-run author rows. |
| `crates/web/src/components/participant_tile.rs` | modify | Top-left corner `<TrustBadge size=Disk12>` with `.trust-badge--tile-corner` background `color-mix(bg-0 60%, transparent)`. |
| `crates/web/src/components/chat.rs` | modify | Mount `<HolderPill>` in channel header flush-right. Mount `<DowngradeBanner>` at top of peer letter view. |
| `crates/web/src/components/mod.rs` | modify | Register new modules. |
| `crates/web/src/app.rs` | modify | Construct `Arc<WebTrustStore>` at boot; inject into ClientHandle via new `with_trust_store`. Mount `<AddFriendDialog open=compare_target on_close>` once at root. Add `<div id="trust-live-region" class="sr-only" aria-live="assertive">`. |
| `crates/client/src/lib.rs` | modify | Add `trust_store: Option<Arc<dyn TrustStore>>` field. Add `#[cfg(target_arch="wasm32")]` shim methods `verify_peer/mark_unverified/begin_compare/trust_state`. `ComparePreview { you, them }` struct. |
| `crates/web/src/icons.rs` | modify | Add icon_shield, icon_key (icon_check already exists from earlier work). |
| `crates/web/style.css` | modify | Append `.sas-grid`, `.sas-cell`, `.sas-label`, `.trust-badge--*`, `.trust-chip`, `.add-friend__*`, `.holder-pill`, `.holder-list`, `.downgrade-banner`, `.sas-press-ring`, `.crypto-strip`, `.sr-only`. Foundation tokens only. |
| `crates/web/tests/browser.rs` | modify | `trust_verification` module: grid 6 numbered cells, badge aria-labels, keyboard Enter activation, matched/mismatch variants render, copy table byte-exact. |
| `e2e/helpers.ts` | modify | Add `openCompareFingerprints`, `markFingerprintsMatch`, `longPressAvatar`. |
| `e2e/permissions.spec.ts` | modify | Add 3 cases: compare-match → verified badge; compare-mismatch → stays unverified + messaging works; mobile long-press opens sheet. |

## Tasks (14)

1. SAS word list (2048 entries) + `sas::sas_words` + 7 tests (determinism, symmetry, key-change, peer-change, wordlist size, ASCII-lowercase, stable vector). Capture stable-vector hex in last step.
2. `crates/client/src/trust.rs` — `TrustStore` trait + `PeerTrust` enum + `UnverifiedReason`.
3. `WebTrustStore` impl of TrustStore trait (localStorage-backed on wasm32; in-memory HashMap on native). 3 round-trip tests.
4. `AppState::trust` bucket + `CryptoVisibility` + `compare_target` signal. Load snapshot at boot; store version signal increments on write.
5. `FingerprintGrid` + `FingerprintLabel` atoms + `SAS_COPY` module with byte-exact constants.
6. `AddFriendDialog` single-flow (Compare / ConfirmMatch / ConfirmMismatch screens). Desktop modal + mobile sheet CSS. Focus trap. Aria-live announce on match/mismatch.
7. `LongPressAvatar` primitive (350ms hold + keyboard). Ring growth animation, reduced-motion fallback.
8. `ClientHandle` WASM shims (`verify_peer`, `mark_unverified`, `begin_compare`, `trust_state`). Trust-store injection via `with_trust_store(handle, Arc<dyn TrustStore>)`.
9. `TrustBadge` component with 5 visual states. Mount on member rows + me strip + message first-of-run + participant tile.
10. `HolderPill` + `HolderList` + `crypto_visibility` subtle/default/explicit branching.
11. `DowngradeBanner` with 24h dismiss + mount on peer letter.
12. Browser tests `trust_verification` module.
13. E2E tests (3 new cases in `permissions.spec.ts` + helpers).
14. `just check` + visual smoke + Phase 1d PR.

## Ambiguity decisions

- **Session-key seed** (pending backend): `blake3(local_pub || remote_pub || DS_TAG)`. Swap to real per-DM key in follow-up. UI unblocked.
- **No new EventKind**: trust is local per spec §Open Questions #3.
- **Word list**: Willow-specific 2048 (spec OQ #1 recommendation). `DS_TAG = "willow-sas-v1"`.
- **`not sure` CTA**: flag `V1_ALLOW_UNSURE_CTA = false`, not rendered.
- **Holder-list rotation timestamp**: em-dash placeholder until backend exposes `rotation_at`.
- **Self compare dialog**: when `open == own_pid`, render only `you` card + single `close` CTA (no match/mismatch).

## Acceptance gates

1. `just check` green.
2. `cargo test -p willow-crypto sas` green (7 tests).
3. `just test-browser` green with `trust_verification` module.
4. `npx playwright test --project=desktop-chrome --project=mobile-chrome e2e/permissions.spec.ts` green.
5. Visual: badge on every peer surface; compare dialog modal on desktop + sheet on mobile; long-press ≥ 350ms opens sheet; match flips badge everywhere in same render; holder pill shows `{n} holders`; subtle mode hides when count==member count; downgrade banner dashed warn border + 24h dismiss; reduced-motion collapses ring + grid + dialog to opacity; aria-live announces on match/mismatch.

## Self-review

- [x] Every §Acceptance row mapped.
- [x] Copy byte-exact from spec Copy table; test enforces.
- [x] Touch targets ≥ 44×44 on mobile; `.trust-chip` padded via `@media (max-width: 720px)`.
- [x] Reduced-motion path on every transition.
- [x] Commits `ui(phase-1): <imperative>`.
- [x] No new EventKind.
