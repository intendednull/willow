# UI Phase 2c — Profile Card Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development + superpowers:test-driven-development. Every task = one commit; tick the checkbox in the same commit.

**Goal:** Ship `docs/specs/2026-04-19-ui-design/profile-card.md` in full — one shared content component, desktop popover + mobile bottom-sheet wrappers, a global profile-card controller, avatar event-bus entry from every avatar surface, all 17 peer-view fields (crest banner, verification badge, avatar, presence dot, display name, pronouns pill, handle + `you call them …`, status pill, bio, tagline, pinned fragment, shared groves, elsewhere, `since`, fingerprint, primary action row, secondary row), self-view variant, crest banner (three procedural SVG patterns, seeded by peer id), private-nickname inline editor (local-only), badge-tap hand-off to the compare-fingerprints flow owned by `trust-verification.md`.

**Architecture:** One shared `<ProfileCardContent>` leaf renders the 17 fields reading from a merged `ProfileView` struct. Two wrappers (`<ProfilePopover>` desktop, `<ProfileSheet>` mobile) consume the same leaf and share a global controller (`use_profile_controller`). A thin `crates/web/src/profile/bus.rs` helper dispatches `ProfileEvent::Open{user_id, anchor}` / `ProfileEvent::Close` window-level CustomEvents, letting every avatar surface call `open_profile(&id, Some(el))` without knowing which wrapper is mounted. Profile data lives on `ProfileView` — merged from `willow-state::Profile` (now extended with `pronouns / bio / tagline / crest_pattern / crest_color / pinned / elsewhere / since`), `willow-identity`, live presence + queue signals, `ServerState` grove intersection, and local `nickname_store`. A new `EventKind::UpdateProfile` carries an optional delta of every grove-propagated field (author permission = self-authorship only).

**Tech Stack:** Leptos 0.7, WASM (wasm-pack), willow-state (new EventKind), willow-client (new ClientMutations method + view derivations + NicknameStore helper), willow-web (profile module, two new components). Foundation tokens only (no new hex). No `just test-browser` / `just test-e2e-*` locally — CI runs them.

**Branch:** `phase-2c/profile-card`. Commits `ui(phase-2c): <imperative>` (code) and `docs(plan): phase 2c — profile-card implementation plan` (initial plan commit only). Lands as one PR; the plan ships in the same PR as the implementation.

---

## Scope

**In:**
- `willow-state::EventKind::UpdateProfile` + extended `Profile` fields (pronouns, bio, tagline, crest_pattern, crest_color, pinned, elsewhere, since) + caps validation + `apply()` implementation + permission table entry (`None` — author authorship is sufficient) + dedup + 6 state-machine tests.
- `ClientMutations::update_profile_fields` API that builds an `UpdateProfile` event and broadcasts it.
- `ProfileView` derivation + `ClientViewHandle::profile_view(peer_id) -> Signal<ProfileView>` selector and `shared_groves(peer_id)` grove-intersection helper + `since_hint(peer_id)` soft-time formatter.
- Local-only `NicknameStore` (localStorage-backed on wasm32, HashMap on native) + `set / get / clear` API. NO event kind. Stored alongside trust store.
- `crates/web/src/profile/` module: `bus.rs` (open/close/controller), `crest.rs` (three procedural SVG patterns), `nickname_store.rs` (WebNicknameStore impl).
- `<ProfileCardContent>` leaf: 17 peer-view fields in render order + self-view variant.
- `<ProfilePopover>` desktop wrapper: anchor positioning w/ flip/clamp, 320 px fixed width, internal scroll cap, dismissal paths (Escape / outside / close / nav dispatch).
- `<ProfileSheet>` mobile wrapper: scrim + bottom slide, safe-area-inset-bottom, scrim/back/escape dismissal.
- Global `use_profile_controller` hook: subscribes to window CustomEvents, maintains `ProfileState`, debounces double-opens, cross-fades peer swap, returns a `Signal<Option<ProfileState>>`.
- Avatar click wiring: every avatar surface (grove rail, channel sidebar, message row first-of-run, thread pane mount point, members pane, letters list, call tile) dispatches `open_profile`.
- Crest banner: three deterministic SVG patterns (`fronds`, `rings`, `leaf`) seeded by peer id + vertical gradient + horizontal ink-wash + default fallbacks.
- Badge (verified / unverified / pending-verify) tap → `begin_compare(peer_id)` into `AppState::trust::compare_target` → existing `<AddFriendDialog>` takes over (no reimpl).
- Private-nickname inline editor with save-on-enter / cancel-on-escape / blur-saves / empty-clears semantics, 32-char cap.
- Exact `PROFILE_COPY` strings from spec §Copy wired into a lookup module.
- Accessibility: `role="dialog"` + `aria-label="profile — <name>"` + focus move-on-open + focus return-on-close + Escape close + SR order enforcement + screen-reader variant for banner badge.
- Browser tests for wrappers, leaf rendering, crest variants, nickname editor, badge click handoff, event-bus controller, avatar click entry from message row.

**Out:**
- Compare-fingerprints screen (`trust-verification.md` — already shipped).
- Settings Profile tab layout (`settings-tweaks.md`). The `edit profile` button fires an existing `set_settings_tab(Profile)` + `set_show_settings(true)` — tab internals stay where they live.
- Block-list management, letter composition, call tile (those surfaces merely call `open_profile`).
- Nickname propagation (v1 local-only per spec Open Questions).
- Whisper start (`whisper-mode.md`) — the `whisper` action button is wired but dispatches a TODO until that phase lands.
- Sheet drag-to-dismiss (spec §Open questions — v2).

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/state/src/types.rs` | modify | Extend `Profile` with `pronouns: Option<String>`, `bio: Option<String>`, `tagline: Option<String>`, `crest_pattern: Option<CrestPattern>`, `crest_color: Option<String>`, `pinned: Option<PinnedFragment>`, `elsewhere: Vec<String>`, `since: Option<String>`. Add `CrestPattern` enum (`Fronds | Rings | Leaf`) + `PinnedFragment { kind: PinnedKind, body: String }` + `PinnedKind` enum (`Quote | Fragment`). All new fields `#[serde(default)]` to keep wire-compatibility with old events. |
| `crates/state/src/event.rs` | modify | Add `EventKind::UpdateProfile { display_name: Option<String>, pronouns: Option<String>, bio: Option<String>, tagline: Option<String>, crest_pattern: Option<Option<CrestPattern>>, crest_color: Option<Option<String>>, pinned: Option<Option<PinnedFragment>>, elsewhere: Option<Vec<String>>, since: Option<Option<String>> }`. Doc-comment: each `Some` = update; each `None` = no change; for nullable fields the inner `Option` distinguishes "clear" from "unchanged". |
| `crates/state/src/materialize.rs` | modify | Add `UpdateProfile` branch in `apply_event` — overlays non-`None` fields onto the author's `Profile` (upserting a bare `Profile { peer_id, display_name: "" }` if missing). No permission check beyond self-authorship (matches `SetProfile`). Length caps enforced at construction — `apply_event` truncates defensively. Mirror `display_name` write into `members[author].display_name` for consistency with legacy `SetProfile`. |
| `crates/state/src/tests.rs` | modify | Add 6 tests: `update_profile_merges_fields` / `update_profile_clears_field_with_inner_none` / `update_profile_preserves_missing_fields` / `update_profile_dedup_is_idempotent` / `update_profile_non_author_rejected_when_not_self` / `update_profile_caps_enforced_on_apply`. |
| `crates/client/src/mutations.rs` | modify | Add `ClientMutations::update_profile_fields(&self, delta: ProfileDelta) -> anyhow::Result<()>` that builds the new `EventKind::UpdateProfile`, applies locally, and broadcasts. Delta is the same shape as the EventKind payload but client-friendly (`ProfileDelta` newtype in `mutations.rs`). |
| `crates/client/src/views.rs` | modify | Add `ProfileView { peer_id, handle, display_name, pronouns, bio, tagline, crest_pattern, crest_color, pinned, elsewhere, since, fingerprint_short, fingerprint_full, is_self }` + `ProfilesView::view_of(&self, peer_id) -> ProfileView` accessor. Also add `ServerRegistryView::shared_groves(&self, local, other)` helper that intersects grove membership. |
| `crates/client/src/lib.rs` | modify | Re-export new types: `ProfileView`, `ProfileDelta`, `CrestPattern`, `PinnedFragment`, `PinnedKind`, `NicknameStore` trait, `NicknameStoreHandle`. |
| `crates/client/src/nickname.rs` | **new** | `NicknameStore` trait (`get(peer_id) -> Option<String>` / `set(peer_id, name)` / `clear(peer_id)` / `version() -> u64`). `MemNicknameStore` impl for native tests. `NicknameStoreHandle = Arc<dyn NicknameStore + Send + Sync>`. 32-char cap enforced in `set`. 4 round-trip tests. |
| `crates/client/src/tests/profile_view.rs` | **new** | 10 client tests: `profile_view_reads_updated_fields` / `profile_view_defaults_crest_to_leaf_moss` / `shared_groves_intersect_memberships` / `shared_groves_empty_when_disjoint` / `since_hint_format_spring_yr_2` / `nickname_store_set_get_clear` / `nickname_store_caps_at_32_chars` / `nickname_store_version_bumps_on_set` / `profile_view_self_flag_true_when_local` / `update_profile_broadcasts_event`. |
| `crates/client/src/tests/mod.rs` (create if missing) | modify | Declare the new `profile_view` module. |
| `crates/web/src/trust_store.rs` | keep | Reference pattern for the nickname store below. |
| `crates/web/src/profile/mod.rs` | **new** | `pub mod bus; pub mod controller; pub mod nickname_store; pub mod crest;` re-exports. |
| `crates/web/src/profile/bus.rs` | **new** | `open_profile(user_id: &str, anchor: Option<HtmlElement>)` dispatches `window.dispatchEvent(new CustomEvent('willow:profile:open', { detail }))`. `close_profile()` dispatches `'willow:profile:close'`. Serializes `detail` via JS (no `to_jsvalue`). Export `PROFILE_OPEN_EVENT` / `PROFILE_CLOSE_EVENT` string constants. |
| `crates/web/src/profile/controller.rs` | **new** | `use_profile_controller() -> (ReadSignal<Option<ProfileState>>, WriteSignal<Option<ProfileState>>)`. Installs window listeners for the two events, resolves the target user via `ClientHandle::views().profiles.view_of(peer_id)`, debounces double-opens within 16 ms, dedupes on `user_id` (update anchor only), handles fade-swap between different ids. Owns a window-level `Escape` keydown listener. |
| `crates/web/src/profile/nickname_store.rs` | **new** | `WebNicknameStore` impl of `willow_client::NicknameStore`. localStorage key `willow.profile.nickname.<peer_id>`. Boots with `load()` that rehydrates a `HashMap` from the storage prefix. Version counter bumps on each mutation. |
| `crates/web/src/profile/crest.rs` | **new** | `render_crest(pattern: CrestPattern, color: &str, peer_id: &str) -> ElementView` — returns a deterministic `<svg viewBox="0 0 320 92">` for the three patterns. Seeded by blake3(peer_id) so the same peer always gets the same layout. Vertical gradient (0.55 → 0.18 → 0) + horizontal ink-wash (`--bg-0` at 0 → 0.22 → 0). `aria-hidden="true"` on the root. `crest_defaults(profile) -> (CrestPattern, String)` falls back to `Leaf` / `--moss-2` when either field is `None`. |
| `crates/web/src/profile/copy.rs` | **new** | `PROFILE_COPY` module — `pub const MESSAGE: &str = "message";` / `CALL: "start call";` / `WHISPER: "whisper";` / …every string from spec §Copy verbatim. Also `ROLE_LABEL` + `STATUS_LABEL` constants. |
| `crates/web/src/components/profile_card.rs` | modify | Replace the 68-line `ProfileCardStub` with the real `<ProfileCardContent>` leaf (accepts `view: ProfileView`, `variant: ProfileVariant { Peer, Self_ }`, closes callback). Keep the existing `ProfileCardStub` name as a deprecated re-export for one release so `presence.md`-surface callers continue to compile; add `#[deprecated = "…"]` and a TODO to remove after any remaining callsite migrates. All 17 fields + self-view flags rendered inside. |
| `crates/web/src/components/profile_popover.rs` | **new** | Desktop wrapper. Reads the controller signal; when `Some(state)` and the viewport is desktop (media query probed via `data-shell` set by `mobile_shell`), measures anchor rect in an `Effect`, picks position (right / flip-left / clamp), sets inline `top`/`left`, renders `<ProfileCardContent>`. Owns its own Escape, outside-click listener (attached one tick post-open), and fires the close on navigating actions. |
| `crates/web/src/components/profile_sheet.rs` | **new** | Mobile wrapper. Reads the controller signal; when `Some(state)` and viewport is mobile, renders a scrim + translateY-in sheet holding `<ProfileCardContent>`. Pushes a transient `history.state` on open, pops on close. Scrim tap + back + Escape dismiss. Respects `env(safe-area-inset-bottom)`. |
| `crates/web/src/components/mod.rs` | modify | Register `profile_popover`, `profile_sheet`. `pub use profile_popover::*; pub use profile_sheet::*;`. |
| `crates/web/src/components/message.rs` | modify | Author-button `aria-label="{name} — open profile"` already exists; replace the existing no-op `on:click` with `open_profile(&msg.author_peer_id.to_string(), Some(ev.current_target()))`. |
| `crates/web/src/components/member_list.rs` | modify | Avatar + name row — add a click handler dispatching `open_profile`. |
| `crates/web/src/components/grove_rail.rs` | modify | Grove-rail peer avatars dispatch `open_profile`. |
| `crates/web/src/components/channel_sidebar.rs` | modify | Channel-sidebar "Me" strip + peer rows dispatch `open_profile`. |
| `crates/web/src/components/participant_tile.rs` | modify | Call-tile avatar dispatches `open_profile`. |
| `crates/web/src/app.rs` | modify | Mount `<ProfilePopover/>` + `<ProfileSheet/>` once at the root (alongside `<AddFriendDialog/>`). Construct + provide `NicknameStoreHandle` in context. Inject into `ClientHandle` via a new `with_nickname_store` builder (or read directly from context — see Ambiguity decisions). |
| `crates/web/src/state.rs` | modify | Add `profile: ProfileUiState` bucket with `open: ReadSignal<Option<ProfileState>>` + `set_open`. |
| `crates/web/src/lib.rs` | modify | `pub mod profile;` export. |
| `crates/web/style.css` | modify | Append `.profile-popover`, `.profile-sheet`, `.profile-sheet__scrim`, `.profile-card`, `.profile-card__banner`, `.profile-card__badge`, `.profile-card__avatar`, `.profile-card__presence`, `.profile-card__name`, `.profile-card__pronouns`, `.profile-card__handle`, `.profile-card__nickname`, `.profile-card__status`, `.profile-card__bio`, `.profile-card__tagline`, `.profile-card__pinned`, `.profile-card__chips`, `.profile-card__meta`, `.profile-card__actions-primary`, `.profile-card__actions-secondary`, `.profile-card--self`, `.nickname-editor`. Foundation tokens only — `--moss-2`, `--moss-3`, `--amber`, `--warn`, `--ink-1`, `--ink-2`, `--ink-3`, `--bg-0`, `--bg-1`, `--bg-2`, `--line-soft`, `--line`, `--motion-fast`, `--shadow-2`. |
| `crates/web/tests/browser.rs` | modify | Append `mod phase_2c_profile_card { … }` — ~14 tests (see Acceptance gates). |

## Acceptance gates

1. `just fmt` + `just clippy` zero warnings.
2. `cargo test -p willow-state` — 6 new `update_profile` tests + existing suite green.
3. `cargo test -p willow-client` — 10 new `profile_view` tests + existing suite green.
4. `just check-wasm` green.
5. `just test-browser` (CI) green — new `phase_2c_profile_card` module ~14 tests plus no regressions.
6. Manual walkthrough (reserved for post-merge):
   - Click any avatar (grove rail / channel sidebar / message row first-of-run / members pane / participant tile) on desktop → popover opens to the right of the avatar with the correct ProfileView data.
   - Click outside the popover → closes without dismissing underlying view; click `copy fingerprint` → clipboard contains the full 6-word form and popover stays open.
   - Click the same avatar a second time → re-positions without remount or re-animation.
   - Click a different avatar while one is open → cross-fades to the new user without replaying the entry animation.
   - Resize to mobile (or load in mobile-chrome) → same event opens the bottom sheet with scrim + translateY-in; back gesture / scrim tap dismisses.
   - Badge: verified variant shows filled moss check + tooltip `verified peer`; clicking `unverified` badge opens the compare dialog; the card closes on the dispatch.
   - Self view: opens via the "me" strip; shows `edit profile` full-width button; clicking it opens Settings with tab = Profile; secondary row replaced with `this is you · <3 fingerprint words>` caption.
   - Private nickname: `set nickname` → inline editor; Enter saves, Escape cancels, blur saves, empty clears. Reload → nickname persists. Never emitted on a broadcast event.
   - Crest: three patterns render for three different peers with different `crest_pattern` fields; same peer re-opened renders the same layout deterministically.
   - Missing crest: new peer with no `UpdateProfile` ever sent renders leaf + `--moss-2` default banner.
   - Accessibility: focus moves to first action on open; Escape returns focus to the anchor; reduced-motion collapses pop-in / sheet-slide to opacity fade.

## Tasks (14 total, ~18 commits)

### 1. State — extend `Profile` + `CrestPattern` / `PinnedFragment` types

Extend `willow-state` with the new fields so downstream crates can reference them before the event kind lands. Serde `#[serde(default)]` keeps existing serialized events readable.

**Files:** modify `crates/state/src/types.rs`, modify `crates/state/src/tests.rs`.

- [x] **Step 1.1 — Add `CrestPattern` enum.**

  ```rust
  /// Procedural crest patterns. Deterministic SVG seeded by peer id.
  ///
  /// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md` §Crest banner.
  #[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
  pub enum CrestPattern {
      Fronds,
      Rings,
      Leaf,
  }

  impl Default for CrestPattern {
      fn default() -> Self {
          // Spec §Missing / default: `leaf` is the fallback pattern.
          CrestPattern::Leaf
      }
  }
  ```

- [x] **Step 1.2 — Add `PinnedFragment` + `PinnedKind`.**

  ```rust
  #[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
  pub enum PinnedKind {
      Quote,
      Fragment,
  }

  #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
  pub struct PinnedFragment {
      pub kind: PinnedKind,
      pub body: String,
  }
  ```

- [x] **Step 1.3 — Extend `Profile`.** Replace the existing `Profile` struct with:

  ```rust
  /// A peer's display profile.
  ///
  /// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md` §Data dependencies.
  /// All new fields (pronouns, bio, tagline, crest_*, pinned, elsewhere, since)
  /// are `#[serde(default)]` so events serialized before this change still
  /// deserialize without schema migration.
  #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
  pub struct Profile {
      pub peer_id: EndpointId,
      pub display_name: String,
      #[serde(default)]
      pub pronouns: Option<String>,
      #[serde(default)]
      pub bio: Option<String>,
      #[serde(default)]
      pub tagline: Option<String>,
      #[serde(default)]
      pub crest_pattern: Option<CrestPattern>,
      /// RGB hex (including leading `#`). Cap 7 chars. Validator rejects
      /// anything else on the apply path.
      #[serde(default)]
      pub crest_color: Option<String>,
      #[serde(default)]
      pub pinned: Option<PinnedFragment>,
      /// Non-identifying freeform labels. Cap 4 × 48 chars.
      #[serde(default)]
      pub elsewhere: Vec<String>,
      #[serde(default)]
      pub since: Option<String>,
  }
  ```

  Note: `Profile` was previously missing `Default`. Add `Default` via the derive — this requires `EndpointId: Default` which it already is (`EndpointId([0u8; 32])`).

- [x] **Step 1.4 — Unit tests.** In `crates/state/src/types.rs` `#[cfg(test)] mod tests` (add if missing):

  ```rust
  #[test]
  fn profile_default_has_empty_optional_fields() {
      let p = Profile::default();
      assert!(p.pronouns.is_none());
      assert!(p.bio.is_none());
      assert!(p.tagline.is_none());
      assert!(p.crest_pattern.is_none());
      assert!(p.crest_color.is_none());
      assert!(p.pinned.is_none());
      assert!(p.elsewhere.is_empty());
      assert!(p.since.is_none());
  }

  #[test]
  fn profile_serde_back_compat() {
      // Events serialized before this change only carry peer_id + display_name.
      let json = r#"{"peer_id":"0000000000000000000000000000000000000000000000000000000000000000","display_name":"mira"}"#;
      let p: Profile = serde_json::from_str(json).unwrap();
      assert_eq!(p.display_name, "mira");
      assert!(p.pronouns.is_none());
  }

  #[test]
  fn crest_pattern_default_is_leaf() {
      assert_eq!(CrestPattern::default(), CrestPattern::Leaf);
  }
  ```

  (If the crate doesn't use `serde_json` in tests yet, add it to `[dev-dependencies]` of `crates/state/Cargo.toml` — it is already a workspace dep.)

- [x] **Step 1.5 — Run state tests.**

  ```bash
  cargo test -p willow-state
  ```

  Expected: all existing tests plus 3 new `profile_*` / `crest_pattern_*` tests green.

- [x] **Step 1.6 — Commit.**

  ```bash
  git add crates/state/src/types.rs crates/state/Cargo.toml
  git commit -m "ui(phase-2c): extend Profile with pronouns/bio/crest/elsewhere fields"
  ```

### 2. State — add `EventKind::UpdateProfile` + materialize + 6 tests

Define the event kind carrying a full delta, wire `apply()` to overlay the delta onto the author's Profile (creating one if missing), enforce caps, dedupe.

**Files:** modify `crates/state/src/event.rs`, modify `crates/state/src/materialize.rs`, modify `crates/state/src/tests.rs`.

- [x] **Step 2.1 — Add `EventKind::UpdateProfile`.** In `crates/state/src/event.rs` under `// -- Identity --`:

  ```rust
  /// Update one or more profile fields in-place.
  ///
  /// Each outer `Option` is "unchanged when `None`", "overwrite when
  /// `Some`". For nullable fields (`crest_pattern`, `crest_color`,
  /// `pinned`, `since`), the inner `Option` carries "clear when
  /// `None`", "set when `Some(value)`".
  ///
  /// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
  /// §Data dependencies.
  UpdateProfile {
      display_name: Option<String>,
      pronouns: Option<Option<String>>,
      bio: Option<Option<String>>,
      tagline: Option<Option<String>>,
      crest_pattern: Option<Option<crate::types::CrestPattern>>,
      crest_color: Option<Option<String>>,
      pinned: Option<Option<crate::types::PinnedFragment>>,
      elsewhere: Option<Vec<String>>,
      since: Option<Option<String>>,
  },
  ```

  Note: display_name stays a plain `Option<String>` because the existing contract is "empty string = never set", not nullable. The event is author-gated: any member can author `UpdateProfile` for their own `Profile` (the author field of the event IS the peer id being updated).

- [x] **Step 2.2 — Add caps constants.** In `crates/state/src/types.rs`:

  ```rust
  /// Per-field caps enforced by `apply_event(UpdateProfile)`.
  ///
  /// Values above the cap are silently truncated on apply rather than
  /// rejecting the event — so misbehaving clients cannot DoS the DAG.
  pub const PROFILE_CAP_PRONOUNS: usize = 32;
  pub const PROFILE_CAP_BIO: usize = 240;
  pub const PROFILE_CAP_TAGLINE: usize = 80;
  pub const PROFILE_CAP_CREST_COLOR: usize = 7;
  pub const PROFILE_CAP_PINNED_BODY: usize = 280;
  pub const PROFILE_CAP_ELSEWHERE_ENTRY: usize = 48;
  pub const PROFILE_CAP_ELSEWHERE_LEN: usize = 4;
  pub const PROFILE_CAP_SINCE: usize = 32;
  pub const PROFILE_CAP_NICKNAME: usize = 32;
  ```

- [x] **Step 2.3 — Implement in `apply_event`.** In `crates/state/src/materialize.rs` immediately after the `SetProfile` branch:

  ```rust
  EventKind::UpdateProfile {
      display_name,
      pronouns,
      bio,
      tagline,
      crest_pattern,
      crest_color,
      pinned,
      elsewhere,
      since,
  } => {
      let entry = state.profiles.entry(event.author).or_insert_with(|| Profile {
          peer_id: event.author,
          ..Profile::default()
      });
      if let Some(name) = display_name {
          entry.display_name = name.clone();
          if let Some(member) = state.members.get_mut(&event.author) {
              member.display_name = Some(name.clone());
          }
      }
      if let Some(v) = pronouns {
          entry.pronouns = v.as_ref().map(|s| truncate(s, crate::types::PROFILE_CAP_PRONOUNS));
      }
      if let Some(v) = bio {
          entry.bio = v.as_ref().map(|s| truncate(s, crate::types::PROFILE_CAP_BIO));
      }
      if let Some(v) = tagline {
          entry.tagline = v.as_ref().map(|s| truncate(s, crate::types::PROFILE_CAP_TAGLINE));
      }
      if let Some(v) = crest_pattern {
          entry.crest_pattern = *v;
      }
      if let Some(v) = crest_color {
          entry.crest_color = v.as_ref().and_then(|s| {
              let t = truncate(s, crate::types::PROFILE_CAP_CREST_COLOR);
              if t.starts_with('#') { Some(t) } else { None }
          });
      }
      if let Some(v) = pinned {
          entry.pinned = v.as_ref().map(|p| crate::types::PinnedFragment {
              kind: p.kind,
              body: truncate(&p.body, crate::types::PROFILE_CAP_PINNED_BODY),
          });
      }
      if let Some(v) = elsewhere {
          entry.elsewhere = v
              .iter()
              .take(crate::types::PROFILE_CAP_ELSEWHERE_LEN)
              .map(|s| truncate(s, crate::types::PROFILE_CAP_ELSEWHERE_ENTRY))
              .collect();
      }
      if let Some(v) = since {
          entry.since = v.as_ref().map(|s| truncate(s, crate::types::PROFILE_CAP_SINCE));
      }
  }
  ```

  Add a private `fn truncate(s: &str, cap: usize) -> String` helper to the top of `materialize.rs` (UTF-8-safe — walks char boundaries; existing codebase has no equivalent).

- [x] **Step 2.4 — Permission table.** In `required_permission(kind)` in `crates/state/src/materialize.rs`, add `EventKind::UpdateProfile { .. } => None` right next to `EventKind::SetProfile { .. } => None`. Same contract: self-authorship is sufficient.

- [x] **Step 2.5 — State tests.** Append to `crates/state/src/tests.rs`:

  ```rust
  #[test]
  fn update_profile_merges_fields() {
      let alice = Identity::generate();
      let mut st = ServerState::default();
      st.profiles.insert(alice.endpoint_id(), Profile {
          peer_id: alice.endpoint_id(),
          display_name: "alice".into(),
          ..Profile::default()
      });
      let e = make_event(&alice, &st, EventKind::UpdateProfile {
          display_name: None,
          pronouns: Some(Some("she/her".into())),
          bio: Some(Some("gardener".into())),
          tagline: None,
          crest_pattern: Some(Some(CrestPattern::Fronds)),
          crest_color: Some(Some("#6b8e4e".into())),
          pinned: None,
          elsewhere: Some(vec!["west coast".into()]),
          since: Some(Some("spring · yr 2".into())),
      });
      let r = apply(&mut st, &e);
      assert!(matches!(r, ApplyResult::Ok));
      let p = &st.profiles[&alice.endpoint_id()];
      assert_eq!(p.display_name, "alice");
      assert_eq!(p.pronouns.as_deref(), Some("she/her"));
      assert_eq!(p.bio.as_deref(), Some("gardener"));
      assert_eq!(p.crest_pattern, Some(CrestPattern::Fronds));
      assert_eq!(p.crest_color.as_deref(), Some("#6b8e4e"));
      assert_eq!(p.elsewhere, vec!["west coast".to_string()]);
      assert_eq!(p.since.as_deref(), Some("spring · yr 2"));
  }

  #[test]
  fn update_profile_clears_field_with_inner_none() {
      let alice = Identity::generate();
      let mut st = ServerState::default();
      st.profiles.insert(alice.endpoint_id(), Profile {
          peer_id: alice.endpoint_id(),
          display_name: "alice".into(),
          bio: Some("old bio".into()),
          ..Profile::default()
      });
      let e = make_event(&alice, &st, EventKind::UpdateProfile {
          display_name: None,
          pronouns: None,
          bio: Some(None),
          tagline: None,
          crest_pattern: None,
          crest_color: None,
          pinned: None,
          elsewhere: None,
          since: None,
      });
      apply(&mut st, &e);
      assert!(st.profiles[&alice.endpoint_id()].bio.is_none());
  }

  #[test]
  fn update_profile_preserves_missing_fields() {
      let alice = Identity::generate();
      let mut st = ServerState::default();
      st.profiles.insert(alice.endpoint_id(), Profile {
          peer_id: alice.endpoint_id(),
          display_name: "alice".into(),
          bio: Some("hello".into()),
          pronouns: Some("she/her".into()),
          ..Profile::default()
      });
      let e = make_event(&alice, &st, EventKind::UpdateProfile {
          display_name: None,
          pronouns: None,
          bio: None,
          tagline: Some(Some("tending the moss".into())),
          crest_pattern: None,
          crest_color: None,
          pinned: None,
          elsewhere: None,
          since: None,
      });
      apply(&mut st, &e);
      let p = &st.profiles[&alice.endpoint_id()];
      assert_eq!(p.bio.as_deref(), Some("hello"));
      assert_eq!(p.pronouns.as_deref(), Some("she/her"));
      assert_eq!(p.tagline.as_deref(), Some("tending the moss"));
  }

  #[test]
  fn update_profile_dedup_is_idempotent() {
      let alice = Identity::generate();
      let mut st = ServerState::default();
      let e = make_event(&alice, &st, EventKind::UpdateProfile {
          display_name: Some("alice".into()),
          pronouns: Some(Some("she/her".into())),
          bio: None, tagline: None, crest_pattern: None, crest_color: None,
          pinned: None, elsewhere: None, since: None,
      });
      let r1 = apply(&mut st, &e);
      let r2 = apply(&mut st, &e);
      assert!(matches!(r1, ApplyResult::Ok));
      // Second apply is a dedup'd no-op (apply_event's outer guard
      // drops re-applied hashes — see `applied_hashes`).
      assert!(matches!(r2, ApplyResult::Ok | ApplyResult::Duplicate));
  }

  #[test]
  fn update_profile_caps_enforced_on_apply() {
      let alice = Identity::generate();
      let mut st = ServerState::default();
      let long_bio = "a".repeat(500);
      let e = make_event(&alice, &st, EventKind::UpdateProfile {
          display_name: None,
          pronouns: None,
          bio: Some(Some(long_bio)),
          tagline: None, crest_pattern: None, crest_color: None,
          pinned: None, elsewhere: None, since: None,
      });
      apply(&mut st, &e);
      let p = &st.profiles[&alice.endpoint_id()];
      assert_eq!(p.bio.as_ref().map(|s| s.len()), Some(PROFILE_CAP_BIO));
  }

  #[test]
  fn update_profile_creates_profile_if_missing() {
      let alice = Identity::generate();
      let mut st = ServerState::default();
      // alice has no Profile entry yet.
      assert!(!st.profiles.contains_key(&alice.endpoint_id()));
      let e = make_event(&alice, &st, EventKind::UpdateProfile {
          display_name: None,
          pronouns: Some(Some("they/them".into())),
          bio: None, tagline: None, crest_pattern: None, crest_color: None,
          pinned: None, elsewhere: None, since: None,
      });
      apply(&mut st, &e);
      let p = st.profiles.get(&alice.endpoint_id()).expect("profile upserted");
      assert_eq!(p.pronouns.as_deref(), Some("they/them"));
      assert_eq!(p.display_name, ""); // never set
  }
  ```

  Imports: add `use crate::types::{Profile, CrestPattern, PROFILE_CAP_BIO};` at the top of the test module if not already present (the `tests.rs` file typically wildcards from `super::*`).

- [x] **Step 2.6 — Run state tests.**

  ```bash
  cargo test -p willow-state
  ```

  Expected: 6 new tests green + existing suite green.

- [x] **Step 2.7 — Commit.**

  ```bash
  git add crates/state/
  git commit -m "ui(phase-2c): add EventKind::UpdateProfile + materialize + caps"
  ```

### 3. Client — `NicknameStore` trait + `MemNicknameStore`

Add a dependency-injected, local-only nickname store to the client crate. Matches the shape of `TrustStore` so the web crate can provide a localStorage-backed impl without ceremony. Spec §Private nickname requires local-only, never-propagated storage.

**Files:** new `crates/client/src/nickname.rs`, modify `crates/client/src/lib.rs`, modify `crates/client/src/tests/mod.rs` (create if missing).

- [x] **Step 3.1 — Define trait + handle + in-memory impl.** New `crates/client/src/nickname.rs`:

  ```rust
  //! Local-only peer nicknames.
  //!
  //! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
  //! §Private nickname. Nicknames never propagate — they live alongside
  //! the trust store in browser localStorage. This crate owns the trait;
  //! the web crate ships the `WebNicknameStore` impl.

  use std::collections::HashMap;
  use std::sync::{Arc, RwLock};

  /// Cap on nickname length in characters. Spec §Private nickname.
  pub const NICKNAME_CAP: usize = 32;

  /// Trait for an opaque, local-only per-peer nickname store.
  ///
  /// Implementations MUST persist writes durably within the lifetime of
  /// the session (e.g. localStorage on web, on-disk file natively). The
  /// `version` counter increments on every successful mutation so
  /// reactive UIs can bump a signal.
  pub trait NicknameStore {
      /// Return the stored nickname for `peer_id`, or `None`.
      fn get(&self, peer_id: &str) -> Option<String>;
      /// Persist `value` (truncated to [`NICKNAME_CAP`]). Pass empty to clear.
      fn set(&self, peer_id: &str, value: &str);
      /// Remove the entry for `peer_id`. Equivalent to `set(peer_id, "")`.
      fn clear(&self, peer_id: &str);
      /// Current version counter — bumps on every mutation.
      fn version(&self) -> u64;
      /// Full snapshot as `(peer_id, nickname)` pairs.
      fn snapshot(&self) -> Vec<(String, String)>;
  }

  pub type NicknameStoreHandle = Arc<dyn NicknameStore + Send + Sync>;

  /// In-memory implementation for tests + native builds.
  #[derive(Default)]
  pub struct MemNicknameStore {
      inner: RwLock<HashMap<String, String>>,
      version: RwLock<u64>,
  }

  impl NicknameStore for MemNicknameStore {
      fn get(&self, peer_id: &str) -> Option<String> {
          self.inner.read().ok()?.get(peer_id).cloned()
      }
      fn set(&self, peer_id: &str, value: &str) {
          let trimmed: String = value.chars().take(NICKNAME_CAP).collect();
          if trimmed.is_empty() {
              self.clear(peer_id);
              return;
          }
          if let Ok(mut guard) = self.inner.write() {
              guard.insert(peer_id.to_string(), trimmed);
          }
          if let Ok(mut v) = self.version.write() {
              *v += 1;
          }
      }
      fn clear(&self, peer_id: &str) {
          if let Ok(mut guard) = self.inner.write() {
              guard.remove(peer_id);
          }
          if let Ok(mut v) = self.version.write() {
              *v += 1;
          }
      }
      fn version(&self) -> u64 {
          self.version.read().map(|g| *g).unwrap_or(0)
      }
      fn snapshot(&self) -> Vec<(String, String)> {
          self.inner
              .read()
              .map(|g| g.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
              .unwrap_or_default()
      }
  }

  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn mem_store_set_and_get_round_trip() {
          let s = MemNicknameStore::default();
          s.set("alice", "mira");
          assert_eq!(s.get("alice").as_deref(), Some("mira"));
      }

      #[test]
      fn mem_store_clear_removes_entry() {
          let s = MemNicknameStore::default();
          s.set("alice", "mira");
          s.clear("alice");
          assert_eq!(s.get("alice"), None);
      }

      #[test]
      fn mem_store_version_bumps_on_mutation() {
          let s = MemNicknameStore::default();
          let v0 = s.version();
          s.set("alice", "mira");
          let v1 = s.version();
          s.clear("alice");
          let v2 = s.version();
          assert!(v1 > v0);
          assert!(v2 > v1);
      }

      #[test]
      fn mem_store_caps_at_nickname_cap_chars() {
          let s = MemNicknameStore::default();
          let long = "a".repeat(100);
          s.set("alice", &long);
          assert_eq!(s.get("alice").unwrap().chars().count(), NICKNAME_CAP);
      }
  }
  ```

- [x] **Step 3.2 — Re-export from `lib.rs`.** In `crates/client/src/lib.rs`:

  ```rust
  pub mod nickname;
  pub use nickname::{MemNicknameStore, NicknameStore, NicknameStoreHandle, NICKNAME_CAP};
  ```

- [x] **Step 3.3 — Run client tests.**

  ```bash
  cargo test -p willow-client nickname
  ```

  Expected: 4 new tests green.

- [x] **Step 3.4 — Commit.**

  ```bash
  git add crates/client/src/nickname.rs crates/client/src/lib.rs
  git commit -m "ui(phase-2c): add local-only NicknameStore trait + MemNicknameStore"
  ```

### 4. Client — `ProfileView` + `shared_groves` + `update_profile_fields` mutation

Expose a materialized `ProfileView` the UI consumes directly, plus the `ClientMutations` entrypoint that builds and broadcasts an `UpdateProfile` event.

**Files:** modify `crates/client/src/views.rs`, modify `crates/client/src/mutations.rs`, modify `crates/client/src/lib.rs`, new `crates/client/src/tests/profile_view.rs`.

- [ ] **Step 4.1 — Define `ProfileView` + `ProfileDelta`.** Append to `crates/client/src/views.rs`:

  ```rust
  /// Merged profile payload the UI renders.
  ///
  /// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
  /// §Data dependencies. Aggregates fields from `willow-state::Profile`,
  /// `willow-identity`, and derived client-side helpers into one shape
  /// so the UI never knows about the source tables.
  #[derive(Clone, Debug, PartialEq, Eq, Default)]
  pub struct ProfileView {
      pub peer_id: String,
      pub handle: String,
      pub display_name: String,
      pub pronouns: Option<String>,
      pub bio: Option<String>,
      pub tagline: Option<String>,
      pub crest_pattern: Option<willow_state::CrestPattern>,
      pub crest_color: Option<String>,
      pub pinned: Option<willow_state::PinnedFragment>,
      pub elsewhere: Vec<String>,
      pub since: Option<String>,
      /// Short form (3 words) of the 6-word fingerprint.
      pub fingerprint_short: String,
      /// Full 6-word fingerprint.
      pub fingerprint_full: String,
      /// True if this is the local peer's own profile.
      pub is_self: bool,
  }

  /// Delta passed to [`ClientMutations::update_profile_fields`].
  ///
  /// Outer `Option` = "unchanged"; inner `Option` (for nullable fields) =
  /// "clear". Matches the on-wire `EventKind::UpdateProfile` shape.
  #[derive(Clone, Debug, Default, PartialEq, Eq)]
  pub struct ProfileDelta {
      pub display_name: Option<String>,
      pub pronouns: Option<Option<String>>,
      pub bio: Option<Option<String>>,
      pub tagline: Option<Option<String>>,
      pub crest_pattern: Option<Option<willow_state::CrestPattern>>,
      pub crest_color: Option<Option<String>>,
      pub pinned: Option<Option<willow_state::PinnedFragment>>,
      pub elsewhere: Option<Vec<String>>,
      pub since: Option<Option<String>>,
  }
  ```

- [ ] **Step 4.2 — `profile_view` selector.** The existing `ProfilesView { names: HashMap<EndpointId, String> }` carries only display names today. We keep it as the name index and add a second view that wraps a read-through to `ServerRegistryView`'s `active().state.profiles` map.

  In `crates/client/src/views.rs`, extend `ClientViewHandle` with a helper method:

  ```rust
  impl ClientViewHandle {
      /// Build the merged `ProfileView` for `peer_id` asynchronously.
      ///
      /// The UI invokes this on each `open_profile` dispatch. Returns a
      /// default `ProfileView { peer_id, is_self: local == peer, .. }`
      /// when the registry has no entry for that peer.
      pub async fn profile_view_of(
          &self,
          peer_id: &willow_identity::EndpointId,
          local: &willow_identity::EndpointId,
      ) -> ProfileView {
          let pid = *peer_id;
          let local = *local;
          let name_snap = willow_actor::state::select(&self.profiles_addr(), move |p| {
              p.names.get(&pid).cloned()
          }).await;
          let profile_snap = willow_actor::state::select(&self.server_registry_addr(), move |reg| {
              reg.active()
                  .and_then(|e| e.state.profiles.get(&pid).cloned())
          }).await;
          let handle = willow_identity::handle(&pid);
          let fp = willow_identity::fingerprint_words(&pid);
          let short = fp.iter().take(3).cloned().collect::<Vec<_>>().join(" · ");
          let full = fp.join(" · ");
          let display_name = profile_snap
              .as_ref()
              .map(|p| p.display_name.clone())
              .or(name_snap)
              .unwrap_or_else(|| handle.clone());
          ProfileView {
              peer_id: pid.to_string(),
              handle,
              display_name,
              pronouns: profile_snap.as_ref().and_then(|p| p.pronouns.clone()),
              bio: profile_snap.as_ref().and_then(|p| p.bio.clone()),
              tagline: profile_snap.as_ref().and_then(|p| p.tagline.clone()),
              crest_pattern: profile_snap.as_ref().and_then(|p| p.crest_pattern),
              crest_color: profile_snap.as_ref().and_then(|p| p.crest_color.clone()),
              pinned: profile_snap.as_ref().and_then(|p| p.pinned.clone()),
              elsewhere: profile_snap.as_ref().map(|p| p.elsewhere.clone()).unwrap_or_default(),
              since: profile_snap.as_ref().and_then(|p| p.since.clone()),
              fingerprint_short: short,
              fingerprint_full: full,
              is_self: pid == local,
          }
      }
  }
  ```

  If `ClientViewHandle` doesn't already expose `profiles_addr()` / `server_registry_addr()`, thread the existing internal addrs through a `pub(crate) fn` accessor. If the required `willow_identity::handle` / `willow_identity::fingerprint_words` helpers don't exist yet, add them as thin wrappers over `EndpointId::to_string()` (handle = first 8 hex chars lowercased) and over the existing 6-word-mnemonic machinery used by `trust-verification.md` (already present in `willow-crypto`). See Ambiguity decisions.

- [ ] **Step 4.3 — `shared_groves` helper.** In `views.rs`:

  ```rust
  impl crate::views::ServerRegistryView {
      /// Return the set of grove names where both peers are members.
      ///
      /// Spec §Data dependencies: shared groves = intersection of grove
      /// memberships.
      pub fn shared_groves(
          &self,
          local: &willow_identity::EndpointId,
          other: &willow_identity::EndpointId,
      ) -> Vec<String> {
          let mut out = Vec::new();
          for entry in self.servers.values() {
              if entry.state.members.contains_key(local)
                  && entry.state.members.contains_key(other)
              {
                  out.push(entry.name.clone());
              }
          }
          out.sort();
          out
      }
  }
  ```

- [ ] **Step 4.4 — `since_hint` soft-time formatter.** Append a free function to `views.rs`:

  ```rust
  /// Format a wall-clock ms timestamp as a soft-time hint.
  ///
  /// Spec §Data dependencies — the `since` field renders as
  /// `"spring · yr 2"`-style text (not `YYYY-MM-DD`). Bucket by season
  /// + "yr N" offset from the earliest event in the grove.
  pub fn since_hint(earliest_ms: u64, now_ms: u64) -> String {
      let season_idx = ((earliest_ms / 86_400_000) % 365 / 91).min(3);
      let season = ["spring", "summer", "fall", "winter"][season_idx as usize];
      let years_ago = ((now_ms.saturating_sub(earliest_ms)) / (365 * 86_400_000)).max(1);
      format!("{season} · yr {years_ago}")
  }
  ```

- [ ] **Step 4.5 — `update_profile_fields` mutation.** Append to `crates/client/src/mutations.rs`:

  ```rust
  impl<N: willow_network::Network> ClientMutations<N> {
      /// Build + apply + broadcast an `EventKind::UpdateProfile`.
      ///
      /// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
      /// §Editing — self. The Settings Profile tab calls into this; the
      /// popover never inlines edits itself.
      pub async fn update_profile_fields(
          &self,
          delta: crate::views::ProfileDelta,
      ) -> anyhow::Result<()> {
          let event = self
              .mutation_handle
              .build_event(willow_state::EventKind::UpdateProfile {
                  display_name: delta.display_name,
                  pronouns: delta.pronouns,
                  bio: delta.bio,
                  tagline: delta.tagline,
                  crest_pattern: delta.crest_pattern,
                  crest_color: delta.crest_color,
                  pinned: delta.pinned,
                  elsewhere: delta.elsewhere,
                  since: delta.since,
              })
              .await?;
          self.mutation_handle.apply_event(&event).await;
          self.mutation_handle.broadcast_event(&event);
          Ok(())
      }
  }
  ```

- [ ] **Step 4.6 — Re-exports.** In `crates/client/src/lib.rs`:

  ```rust
  pub use views::{ProfileDelta, ProfileView, since_hint};
  pub use willow_state::{CrestPattern, PinnedFragment, PinnedKind};
  ```

- [ ] **Step 4.7 — Client tests.** New `crates/client/src/tests/profile_view.rs`:

  ```rust
  //! Tests for `ProfileView` derivation + `shared_groves` + `since_hint`.
  //!
  //! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`.

  use crate::{test_client, ProfileDelta, ProfileView};
  use willow_state::{CrestPattern, PinnedFragment, PinnedKind};

  #[tokio::test]
  async fn profile_view_reads_updated_fields() {
      let (client, _loop) = test_client();
      // Seed a server so the local peer has an authored chain.
      client.bootstrap_server("demo").await.unwrap();
      client.mutations().update_profile_fields(ProfileDelta {
          display_name: Some("mira".into()),
          pronouns: Some(Some("she/her".into())),
          bio: Some(Some("gardener".into())),
          tagline: Some(Some("tending the moss".into())),
          crest_pattern: Some(Some(CrestPattern::Fronds)),
          crest_color: Some(Some("#6b8e4e".into())),
          pinned: Some(Some(PinnedFragment {
              kind: PinnedKind::Quote,
              body: "quiet is a kind of music".into(),
          })),
          elsewhere: Some(vec!["coast · west".into()]),
          since: Some(Some("spring · yr 2".into())),
      }).await.unwrap();
      let local = client.identity().endpoint_id();
      let v = client.views().profile_view_of(&local, &local).await;
      assert_eq!(v.display_name, "mira");
      assert_eq!(v.pronouns.as_deref(), Some("she/her"));
      assert_eq!(v.bio.as_deref(), Some("gardener"));
      assert_eq!(v.tagline.as_deref(), Some("tending the moss"));
      assert_eq!(v.crest_pattern, Some(CrestPattern::Fronds));
      assert_eq!(v.crest_color.as_deref(), Some("#6b8e4e"));
      assert_eq!(v.elsewhere, vec!["coast · west".to_string()]);
      assert_eq!(v.since.as_deref(), Some("spring · yr 2"));
      assert!(v.is_self);
  }

  #[tokio::test]
  async fn profile_view_defaults_crest_to_none_for_missing_fields() {
      let (client, _loop) = test_client();
      client.bootstrap_server("demo").await.unwrap();
      let local = client.identity().endpoint_id();
      // Never called update_profile_fields — all optional fields absent.
      let v = client.views().profile_view_of(&local, &local).await;
      assert!(v.crest_pattern.is_none());
      assert!(v.crest_color.is_none());
      // UI falls back to Leaf / --moss-2 at render time.
  }

  #[tokio::test]
  async fn shared_groves_intersect_memberships() {
      use crate::test_client;
      let (alice, _l1) = test_client();
      alice.bootstrap_server("demo").await.unwrap();
      let local = alice.identity().endpoint_id();
      // Same peer in both — intersection contains the only server.
      let names = alice.views().server_registry_snapshot().await.shared_groves(&local, &local);
      assert_eq!(names.len(), 1);
      assert_eq!(names[0], "demo");
  }

  #[tokio::test]
  async fn shared_groves_empty_when_disjoint() {
      use crate::test_client;
      let (alice, _l1) = test_client();
      alice.bootstrap_server("demo").await.unwrap();
      let local = alice.identity().endpoint_id();
      let phantom = willow_identity::Identity::generate().endpoint_id();
      let names = alice.views().server_registry_snapshot().await.shared_groves(&local, &phantom);
      assert!(names.is_empty());
  }

  #[test]
  fn since_hint_format_spring_yr_2() {
      let earliest = 1_714_000_000_000u64; // mid-2024
      let now = earliest + 2 * 365 * 86_400_000;
      let s = crate::since_hint(earliest, now);
      assert!(s.starts_with("spring") || s.starts_with("summer") || s.starts_with("fall") || s.starts_with("winter"));
      assert!(s.contains("yr 2"));
  }

  #[test]
  fn update_profile_delta_default_is_noop_shape() {
      let d = ProfileDelta::default();
      assert!(d.display_name.is_none());
      assert!(d.pronouns.is_none());
      assert!(d.elsewhere.is_none());
  }
  ```

  (`server_registry_snapshot()` may not exist yet. If not, add a `pub async fn server_registry_snapshot(&self) -> ServerRegistryView` helper in `ClientViewHandle` that clones the actor state.)

- [ ] **Step 4.8 — Declare test module.** In `crates/client/src/tests/mod.rs` (create if missing; mark `#[cfg(test)]` in `lib.rs`):

  ```rust
  #[cfg(test)]
  mod profile_view;
  ```

  Ensure `lib.rs` already has `#[cfg(test)] mod tests;` (it does — see `crates/client/src/tests/multi_peer_sync.rs`).

- [ ] **Step 4.9 — Run client tests.**

  ```bash
  cargo test -p willow-client profile
  ```

  Expected: 6 new tests green (the 4 MemNicknameStore tests from Task 3 plus 6 here). If helpers are missing, plug them until green.

- [ ] **Step 4.10 — Commit.**

  ```bash
  git add crates/client/
  git commit -m "ui(phase-2c): add ProfileView + ProfileDelta + update_profile_fields mutation"
  ```

### 5. Web — profile module skeleton (bus + controller + nickname store + copy)

Scaffold the `crates/web/src/profile/` submodule. No components yet — just the event bus, the controller signal, the WebNicknameStore impl, the exact `PROFILE_COPY` strings.

**Files:** new `crates/web/src/profile/mod.rs`, new `crates/web/src/profile/bus.rs`, new `crates/web/src/profile/controller.rs`, new `crates/web/src/profile/nickname_store.rs`, new `crates/web/src/profile/copy.rs`, modify `crates/web/src/lib.rs`.

- [ ] **Step 5.1 — Module registration.** New `crates/web/src/profile/mod.rs`:

  ```rust
  //! Profile-card wiring: event bus, controller, nickname store, copy.
  //!
  //! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md` §Event-bus API.

  pub mod bus;
  pub mod controller;
  pub mod copy;
  pub mod nickname_store;

  pub use bus::{close_profile, open_profile, PROFILE_CLOSE_EVENT, PROFILE_OPEN_EVENT};
  pub use controller::{use_profile_controller, ProfileState};
  pub use nickname_store::WebNicknameStore;
  ```

  In `crates/web/src/lib.rs`, add `pub mod profile;` near the other `pub mod` declarations.

- [ ] **Step 5.2 — Copy module.** New `crates/web/src/profile/copy.rs`:

  ```rust
  //! Exact strings from `profile-card.md` §Copy.
  //!
  //! All labels are lowercase per the foundation voice rule. Copy in
  //! this module is load-bearing for byte-exact tests.

  pub const MESSAGE: &str = "message";
  pub const CALL: &str = "start call";
  pub const WHISPER: &str = "whisper";
  pub const COPY_FINGERPRINT: &str = "copy fingerprint";
  pub const VERIFY: &str = "verify in person";
  pub const BLOCK: &str = "block";
  pub const EDIT_PROFILE: &str = "edit profile";
  pub const SET_NICKNAME: &str = "set nickname";
  pub const CHANGE_NICKNAME: &str = "change nickname";
  pub const UNVERIFIED_TOOLTIP: &str =
      "unverified — compare fingerprints before you trust this peer";
  pub const VERIFIED_TOOLTIP: &str = "verified peer";
  pub const PENDING_TOOLTIP: &str = "compare in progress · resume →";
  pub const SELF_CAPTION: &str = "this is you";
  pub const QUEUED_PREFIX: &str = "queued ·";
  pub const WHISPER_STATUS: &str = "whispering";
  pub const FINGERPRINT_LABEL: &str = "fingerprint";
  pub const SINCE_LABEL: &str = "in the grove since";
  pub const SHARED_GROVES_LABEL: &str = "you share";
  pub const KNOWN_AS_PREFIX: &str = "you call them";
  pub const PINNED_LABEL: &str = "pinned fragment";
  pub const ELSEWHERE_LABEL: &str = "elsewhere";
  pub const EMPTY_PINNED: &str = "no pinned fragment";
  ```

- [ ] **Step 5.3 — Event bus.** New `crates/web/src/profile/bus.rs`:

  ```rust
  //! Window-level CustomEvent bus for opening / closing the profile card.
  //!
  //! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md` §Event-bus API.
  //!
  //! Any avatar surface calls [`open_profile`] with the clicked user id
  //! and the anchor element. The global controller (mounted once at app
  //! root) subscribes to the window and decides which wrapper renders.

  use wasm_bindgen::prelude::*;
  use wasm_bindgen::JsCast;
  use web_sys::{CustomEvent, CustomEventInit, HtmlElement};

  pub const PROFILE_OPEN_EVENT: &str = "willow:profile:open";
  pub const PROFILE_CLOSE_EVENT: &str = "willow:profile:close";

  /// Dispatch a request to open the profile card for `user_id`.
  ///
  /// `anchor` is optional — the desktop popover needs it for positioning;
  /// the mobile sheet ignores it entirely. Safe to call from any
  /// component's click handler.
  pub fn open_profile(user_id: &str, anchor: Option<HtmlElement>) {
      let Some(win) = web_sys::window() else { return };
      let detail = js_sys::Object::new();
      js_sys::Reflect::set(&detail, &"user_id".into(), &JsValue::from_str(user_id)).ok();
      if let Some(a) = anchor {
          js_sys::Reflect::set(&detail, &"anchor".into(), a.as_ref()).ok();
      }
      let mut init = CustomEventInit::new();
      init.detail(&detail);
      let ev = CustomEvent::new_with_event_init_dict(PROFILE_OPEN_EVENT, &init).unwrap();
      win.dispatch_event(&ev).ok();
  }

  /// Dispatch a request to close the profile card.
  pub fn close_profile() {
      let Some(win) = web_sys::window() else { return };
      let ev = CustomEvent::new(PROFILE_CLOSE_EVENT).unwrap();
      win.dispatch_event(&ev).ok();
  }
  ```

- [ ] **Step 5.4 — Controller.** New `crates/web/src/profile/controller.rs`:

  ```rust
  //! Global controller signal for the profile card.
  //!
  //! Subscribes to `PROFILE_OPEN_EVENT` / `PROFILE_CLOSE_EVENT` at the
  //! window level and exposes a `ReadSignal<Option<ProfileState>>`
  //! drained by the two wrappers. Owns the Escape handler + anchor
  //! update path.

  use std::sync::Arc;

  use leptos::prelude::*;
  use wasm_bindgen::closure::Closure;
  use wasm_bindgen::JsCast;
  use web_sys::{CustomEvent, HtmlElement};
  use willow_client::views::ProfileView;

  use super::bus::{PROFILE_CLOSE_EVENT, PROFILE_OPEN_EVENT};

  /// Shape exposed to the two wrappers.
  #[derive(Clone)]
  pub struct ProfileState {
      pub view: Arc<ProfileView>,
      pub anchor: Option<send_wrapper::SendWrapper<HtmlElement>>,
  }

  impl PartialEq for ProfileState {
      fn eq(&self, other: &Self) -> bool {
          Arc::ptr_eq(&self.view, &other.view)
              || (self.view.peer_id == other.view.peer_id
                  && self.view.display_name == other.view.display_name)
      }
  }

  /// Hook returning the read + write handles on the controller signal.
  ///
  /// Callers MUST be mounted inside a component tree that holds
  /// [`crate::app::WebClientHandle`] in context (the controller needs
  /// `views().profile_view_of`).
  pub fn use_profile_controller() -> (
      ReadSignal<Option<ProfileState>>,
      WriteSignal<Option<ProfileState>>,
  ) {
      let app_state = use_context::<crate::state::AppState>().expect("AppState in context");
      let (read, write) = (app_state.profile.open, app_state.profile.set_open);
      install_listeners_once(write);
      (read, write)
  }

  /// Idempotent — calling twice is a no-op because the listeners live on
  /// the window and we key on a data attribute. In practice the app
  /// calls it once from `<App>`.
  fn install_listeners_once(set_open: WriteSignal<Option<ProfileState>>) {
      let Some(win) = web_sys::window() else { return };
      let body = match win.document().and_then(|d| d.body()) {
          Some(b) => b,
          None => return,
      };
      if body
          .get_attribute("data-profile-bus")
          .as_deref()
          == Some("mounted")
      {
          return;
      }
      body.set_attribute("data-profile-bus", "mounted").ok();

      // OPEN
      let handle = use_context::<crate::app::WebClientHandle>().expect("WebClientHandle in context");
      let handle_for_open = handle.clone();
      let set_open_for_open = set_open;
      let on_open = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
          let Ok(ce) = ev.dyn_into::<CustomEvent>() else { return };
          let detail = ce.detail();
          let Ok(user_id) =
              js_sys::Reflect::get(&detail, &"user_id".into()).map(|v| v.as_string())
          else { return };
          let Some(user_id) = user_id else { return };
          let anchor = js_sys::Reflect::get(&detail, &"anchor".into())
              .ok()
              .and_then(|v| v.dyn_into::<HtmlElement>().ok())
              .map(send_wrapper::SendWrapper::new);
          let Ok(peer_id) = user_id.parse::<willow_identity::EndpointId>() else { return };
          let local = handle_for_open.identity().endpoint_id();
          let handle_for_fut = handle_for_open.clone();
          let set_open_for_fut = set_open_for_open;
          leptos::task::spawn_local(async move {
              let view = handle_for_fut.views().profile_view_of(&peer_id, &local).await;
              set_open_for_fut.set(Some(ProfileState {
                  view: Arc::new(view),
                  anchor,
              }));
          });
      });
      win.add_event_listener_with_callback(PROFILE_OPEN_EVENT, on_open.as_ref().unchecked_ref())
          .ok();
      on_open.forget();

      // CLOSE
      let set_open_for_close = set_open;
      let on_close = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
          set_open_for_close.set(None);
      });
      win.add_event_listener_with_callback(PROFILE_CLOSE_EVENT, on_close.as_ref().unchecked_ref())
          .ok();
      on_close.forget();

      // ESCAPE
      let set_open_for_esc = set_open;
      let on_esc = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
          if let Ok(ke) = ev.dyn_into::<web_sys::KeyboardEvent>() {
              if ke.key() == "Escape" {
                  set_open_for_esc.set(None);
              }
          }
      });
      win.add_event_listener_with_callback("keydown", on_esc.as_ref().unchecked_ref())
          .ok();
      on_esc.forget();
  }
  ```

- [ ] **Step 5.5 — WebNicknameStore impl.** New `crates/web/src/profile/nickname_store.rs`:

  ```rust
  //! localStorage-backed [`NicknameStore`].
  //!
  //! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
  //! §Private nickname. Key format: `willow.profile.nickname.<peer_id>`.

  use std::collections::HashMap;
  use std::sync::RwLock;

  use willow_client::{NicknameStore, NICKNAME_CAP};

  const KEY_PREFIX: &str = "willow.profile.nickname.";

  /// localStorage-backed nickname store.
  #[derive(Default)]
  pub struct WebNicknameStore {
      cache: RwLock<HashMap<String, String>>,
      version: RwLock<u64>,
  }

  impl WebNicknameStore {
      /// Boot + hydrate from the `willow.profile.nickname.*` keys.
      pub fn load() -> Self {
          let store = Self::default();
          let Some(win) = web_sys::window() else { return store };
          let Ok(Some(ls)) = win.local_storage() else { return store };
          let len = ls.length().unwrap_or(0);
          let mut cache = HashMap::new();
          for i in 0..len {
              let Ok(Some(k)) = ls.key(i) else { continue };
              if let Some(pid) = k.strip_prefix(KEY_PREFIX) {
                  if let Ok(Some(v)) = ls.get_item(&k) {
                      cache.insert(pid.to_string(), v);
                  }
              }
          }
          *store.cache.write().unwrap() = cache;
          store
      }
  }

  impl NicknameStore for WebNicknameStore {
      fn get(&self, peer_id: &str) -> Option<String> {
          self.cache.read().ok()?.get(peer_id).cloned()
      }
      fn set(&self, peer_id: &str, value: &str) {
          let trimmed: String = value.chars().take(NICKNAME_CAP).collect();
          if trimmed.is_empty() {
              self.clear(peer_id);
              return;
          }
          self.cache
              .write()
              .unwrap()
              .insert(peer_id.to_string(), trimmed.clone());
          if let Some(win) = web_sys::window() {
              if let Ok(Some(ls)) = win.local_storage() {
                  ls.set_item(&format!("{KEY_PREFIX}{peer_id}"), &trimmed).ok();
              }
          }
          *self.version.write().unwrap() += 1;
      }
      fn clear(&self, peer_id: &str) {
          self.cache.write().unwrap().remove(peer_id);
          if let Some(win) = web_sys::window() {
              if let Ok(Some(ls)) = win.local_storage() {
                  ls.remove_item(&format!("{KEY_PREFIX}{peer_id}")).ok();
              }
          }
          *self.version.write().unwrap() += 1;
      }
      fn version(&self) -> u64 {
          *self.version.read().unwrap()
      }
      fn snapshot(&self) -> Vec<(String, String)> {
          self.cache
              .read()
              .map(|g| g.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
              .unwrap_or_default()
      }
  }
  ```

- [ ] **Step 5.6 — App state slot.** In `crates/web/src/state.rs` add the `ProfileUiState` bucket:

  ```rust
  #[derive(Clone, Copy)]
  pub struct ProfileUiState {
      pub open: ReadSignal<Option<crate::profile::ProfileState>>,
      pub set_open: WriteSignal<Option<crate::profile::ProfileState>>,
  }
  ```

  Wire a new pair in `create_signals` (beside `presence`), attach it to `AppState`, and add a `ProfileWriteSignals` bucket mirroring the pattern. Also initialise a `WebNicknameStore` arc and return it on `InitialSignals`.

- [ ] **Step 5.7 — `just check-wasm`.**

  ```bash
  just check-wasm
  ```

  Expected: clean. No warnings.

- [ ] **Step 5.8 — Commit.**

  ```bash
  git add crates/web/src/profile/ crates/web/src/state.rs crates/web/src/lib.rs
  git commit -m "ui(phase-2c): add profile event bus + controller + WebNicknameStore"
  ```

### 6. Web — crest module (three procedural SVG patterns)

Add `render_crest(pattern, color, peer_id)` producing deterministic `<svg>` for each of the three patterns. Seeded by `blake3(peer_id)` so same peer always gets the same layout.

**Files:** new `crates/web/src/profile/crest.rs`, modify `crates/web/src/profile/mod.rs`.

- [ ] **Step 6.1 — Crest module.** New `crates/web/src/profile/crest.rs`:

  ```rust
  //! Procedural crest banner SVGs.
  //!
  //! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
  //! §Crest banner. Three deterministic patterns seeded by peer id.

  use leptos::either::EitherOf3;
  use leptos::prelude::*;
  use willow_state::CrestPattern;

  const MOSS_2_FALLBACK: &str = "var(--moss-2)";

  /// Resolve `(pattern, color)` with spec defaults.
  pub fn crest_defaults(
      pattern: Option<CrestPattern>,
      color: Option<&str>,
  ) -> (CrestPattern, String) {
      (
          pattern.unwrap_or(CrestPattern::Leaf),
          color
              .filter(|s| s.starts_with('#') && s.len() == 7)
              .map(|s| s.to_string())
              .unwrap_or_else(|| MOSS_2_FALLBACK.to_string()),
      )
  }

  /// Seeded PRNG over the peer id.
  fn seed_rng(peer_id: &str) -> [u8; 32] {
      let mut h = blake3::Hasher::new();
      h.update(b"willow-crest-v1");
      h.update(peer_id.as_bytes());
      *h.finalize().as_bytes()
  }

  fn roll(seed: &[u8; 32], idx: usize, modulus: u32) -> u32 {
      let off = idx * 4 % (seed.len() - 4);
      let x = u32::from_le_bytes([seed[off], seed[off + 1], seed[off + 2], seed[off + 3]]);
      x % modulus
  }

  /// Render the crest banner SVG for `peer_id`.
  #[component]
  pub fn CrestBanner(
      #[prop(into)] pattern: Signal<Option<CrestPattern>>,
      #[prop(into)] color: Signal<Option<String>>,
      #[prop(into)] peer_id: Signal<String>,
  ) -> impl IntoView {
      let svg = move || {
          let (p, c) = crest_defaults(pattern.get(), color.get().as_deref());
          let pid = peer_id.get();
          let seed = seed_rng(&pid);
          match p {
              CrestPattern::Fronds => EitherOf3::A(fronds(&seed, &c)),
              CrestPattern::Rings => EitherOf3::B(rings(&seed, &c)),
              CrestPattern::Leaf => EitherOf3::C(leaf(&seed, &c)),
          }
      };
      view! {
          <div class="profile-card__banner" aria-hidden="true">
              {svg}
          </div>
      }
  }

  fn fronds(seed: &[u8; 32], color: &str) -> impl IntoView {
      let strokes = (0..14).map(|i| {
          let x = 12 + i * 22;
          let sway = (roll(seed, i as usize, 20) as i32) - 10;
          view! {
              <path
                  d=format!("M {x},92 Q {mx},58 {tx},8", mx = x as i32 + sway, tx = x as i32 + sway / 2)
                  stroke=color.to_string()
                  stroke-width="1.5"
                  fill="none"
                  opacity="0.55"
              />
          }
      }).collect_view();
      view! {
          <svg viewBox="0 0 320 92" preserveAspectRatio="none" class="profile-card__crest">
              {banner_washes(color)}
              {strokes}
          </svg>
      }
  }

  fn rings(seed: &[u8; 32], color: &str) -> impl IntoView {
      let scattered = (0..6).map(|i| {
          let cx = 24 + roll(seed, i as usize, 270);
          let cy = 16 + roll(seed, (i + 30) as usize, 60);
          let r = 8 + roll(seed, (i + 60) as usize, 16);
          view! {
              <circle cx=cx.to_string() cy=cy.to_string() r=r.to_string()
                      stroke=color.to_string() stroke-width="1.5" fill="none" opacity="0.5"/>
          }
      }).collect_view();
      view! {
          <svg viewBox="0 0 320 92" preserveAspectRatio="none" class="profile-card__crest">
              {banner_washes(color)}
              {scattered}
              <circle cx="160" cy="46" r="18" stroke=color.to_string() stroke-width="1.5" fill="none" opacity="0.7"/>
              <circle cx="160" cy="46" r="10" stroke=color.to_string() stroke-width="1.5" fill="none" opacity="0.9"/>
          </svg>
      }
  }

  fn leaf(seed: &[u8; 32], color: &str) -> impl IntoView {
      let leaves = (0..9).map(|i| {
          let x = 28 + i * 32;
          let y_off = (roll(seed, i as usize, 8) as i32) + 26;
          view! {
              <path d=format!("M {x},{y} q 8,-14 16,0 q -8,14 -16,0 z", x = x, y = y_off)
                    fill=color.to_string()
                    opacity="0.55"/>
          }
      }).collect_view();
      view! {
          <svg viewBox="0 0 320 92" preserveAspectRatio="none" class="profile-card__crest">
              {banner_washes(color)}
              <path d="M 0,52 Q 160,12 320,52" stroke=color.to_string() stroke-width="1.5"
                    fill="none" opacity="0.35"/>
              {leaves}
          </svg>
      }
  }

  fn banner_washes(color: &str) -> impl IntoView {
      // Vertical gradient behind the pattern + horizontal ink wash over it.
      view! {
          <defs>
              <linearGradient id="crest-v" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0" stop-color=color.to_string() stop-opacity="0.55"/>
                  <stop offset="0.6" stop-color=color.to_string() stop-opacity="0.18"/>
                  <stop offset="1" stop-color=color.to_string() stop-opacity="0"/>
              </linearGradient>
              <linearGradient id="crest-h" x1="0" y1="0" x2="1" y2="0">
                  <stop offset="0" stop-color="var(--bg-0)" stop-opacity="0"/>
                  <stop offset="0.5" stop-color="var(--bg-0)" stop-opacity="0.22"/>
                  <stop offset="1" stop-color="var(--bg-0)" stop-opacity="0"/>
              </linearGradient>
          </defs>
          <rect width="320" height="92" fill="url(#crest-v)"/>
          <rect width="320" height="92" fill="url(#crest-h)"/>
      }
  }
  ```

- [ ] **Step 6.2 — Register in module.** Extend `crates/web/src/profile/mod.rs` with `pub mod crest; pub use crest::CrestBanner;`.

- [ ] **Step 6.3 — Add `blake3` workspace dep.** If `crates/web/Cargo.toml` doesn't already carry `blake3`, add `blake3 = { workspace = true }`. (The trust-verification phase added blake3 to `willow-crypto`; the web crate may need its own line.)

- [ ] **Step 6.4 — `just check-wasm`.**

  ```bash
  just check-wasm
  ```

  Expected: clean.

- [ ] **Step 6.5 — Commit.**

  ```bash
  git add crates/web/src/profile/ crates/web/Cargo.toml
  git commit -m "ui(phase-2c): add procedural crest banner SVG (fronds/rings/leaf)"
  ```

### 7. Web — `<ProfileCardContent>` leaf (peer view: fields 1–17)

Real implementation of the 17 fields. Replaces the stub in `profile_card.rs`. Mounts the crest banner, verification badge, avatar, presence dot, display name, pronouns, handle + nickname, status pill, bio, tagline, pinned fragment, shared-groves chips, elsewhere chips, since meta row, fingerprint meta row, primary action row, secondary row.

**Files:** modify `crates/web/src/components/profile_card.rs`, modify `crates/web/style.css`.

- [ ] **Step 7.1 — Variant enum.** In `profile_card.rs`:

  ```rust
  #[derive(Clone, Copy, PartialEq, Eq)]
  pub enum ProfileVariant {
      Peer,
      Self_,
  }
  ```

- [ ] **Step 7.2 — New `ProfileCardContent` component.** Replace `ProfileCardStub` body (keep the name as a `#[deprecated]` re-export):

  ```rust
  /// 17-field profile card content. Used inside both the desktop popover
  /// and the mobile bottom sheet.
  ///
  /// Spec: `docs/specs/2026-04-19-ui-design/profile-card.md` §Field
  /// inventory.
  #[component]
  pub fn ProfileCardContent(
      #[prop(into)] view: Signal<std::sync::Arc<willow_client::ProfileView>>,
      #[prop(into, default = ProfileVariant::Peer)]
      variant: ProfileVariant,
      /// Fired on close (close-button click on desktop).
      #[prop(into)]
      on_close: Callback<()>,
  ) -> impl IntoView {
      let app_state = use_context::<AppState>().unwrap();
      let write = use_context::<AppWriteSignals>().unwrap();
      // ... compute presence, trust state, shared groves, nickname,
      // primary actions, secondary actions ...
      view! {
          <div class="profile-card" class:profile-card--self=move || variant == ProfileVariant::Self_
               role="dialog" aria-label=move || format!("profile — {}", view.get().display_name)>
              <crate::profile::crest::CrestBanner
                  pattern=Signal::derive(move || view.get().crest_pattern)
                  color=Signal::derive(move || view.get().crest_color.clone())
                  peer_id=Signal::derive(move || view.get().peer_id.clone())/>
              // Verification badge on banner (top-left).
              // ... renders <TrustBadge size=Pill/> wired to begin_compare on click.
              // Desktop close button (top-right) — hidden on mobile via CSS.
              <button class="profile-card__close" aria-label="close profile"
                      on:click=move |_| on_close.run(())>
                  {crate::icons::icon_close()}
              </button>
              // Avatar + presence
              <div class="profile-card__avatar" style=move || format!("background: {}", super::peer_color(&view.get().peer_id))>
                  // ... initial + presence StatusDot ...
              </div>
              // Display name + pronouns pill + handle/nickname + status pill
              <div class="profile-card__name">{move || view.get().display_name.clone()}</div>
              // ... (remaining fields per spec §Field inventory in render order) ...
              // Primary action row (peer view): message · start call · whisper · more
              // Primary action row (self view): edit profile (full-width)
              // Secondary row (peer view): copy fingerprint · set/change nickname · block
              // Secondary row (self view): "this is you · <3 words>" caption
          </div>
      }
  }
  ```

  Implementation detail: the 17-field checklist is lengthy; write each section top-to-bottom following spec field order. Each field uses the exact copy string from `crate::profile::copy`. Hidden fields (pronouns/bio/tagline/pinned/elsewhere/since) are `{move || view.get().foo.clone().map(|v| view!{<div class="profile-card__foo">{v}</div>.into_any())}.unwrap_or_else(|| ().into_any())}` — the peer card never renders empty-state rows for unset fields (spec §Edge cases). The self card shows `no pinned fragment` when pinned is unset.

- [ ] **Step 7.3 — Primary + secondary rows.**

  ```rust
  // Peer variant primary row:
  // [message] [start call] [whisper] [more]
  // Peer variant secondary row:
  // copy fingerprint | set/change nickname | block

  // Self variant primary row:
  // [edit profile] (full-width, --moss-2 primary)
  // Self variant secondary caption:
  // "this is you · <first 3 fp words>"
  ```

  `message` button dispatches `close_profile()` + `set_current_channel` to the 1:1 letters channel (if already exposed; otherwise TODO comment). `start call` TODO-gated on `call-experience.md` — renders but dispatches a no-op. `whisper` TODO-gated on `whisper-mode.md` — same. `more` opens a small menu (block, report, mute) — out of scope for v1, render the button with aria-label and a TODO. `copy fingerprint` uses the existing `crate::util::copy_to_clipboard` + does **not** close the card. `block` TODO-gated on governance.md.

  `edit profile` (self): calls `write.ui.set_settings_tab.set(SettingsTab::Profile)` + `write.ui.set_show_settings.set(true)` + `on_close.run(())`.

- [ ] **Step 7.4 — Badge click handoff.** The verification badge uses the existing `<TrustBadge>` component. Wrap it so that click calls `write.trust.set_compare_target.set(Some(view.get().peer_id.clone()))` + `on_close.run(())`. The existing `<AddFriendDialog>` reads `compare_target` and takes over from there — we do NOT reimplement the compare flow.

- [ ] **Step 7.5 — Stub deprecation.** Keep the old `ProfileCardStub` symbol as a `#[deprecated = "use ProfileCardContent"]` thin wrapper that constructs a minimal `ProfileView` and renders the new component — so phase-1e presence surfaces keep working.

- [ ] **Step 7.6 — CSS skeleton.** Append `crates/web/style.css`:

  ```css
  /* ── Phase 2c · Profile card content ───────────────────────────── */
  .profile-card {
      position: relative;
      background: var(--bg-1);
      border-radius: 12px;
      overflow: hidden;
      font-family: var(--sans);
      color: var(--ink-1);
      display: flex;
      flex-direction: column;
  }
  .profile-card__banner {
      position: relative;
      height: 72px;
      overflow: hidden;
  }
  @media (max-width: 720px) {
      .profile-card__banner { height: 92px; }
  }
  .profile-card__close {
      position: absolute;
      top: 6px; right: 6px;
      background: color-mix(in oklab, var(--bg-0) 60%, transparent);
      backdrop-filter: blur(8px);
      border: 1px solid var(--line-soft);
      border-radius: 999px;
      width: 28px; height: 28px;
      display: inline-flex; align-items: center; justify-content: center;
      color: var(--ink-2);
  }
  @media (max-width: 720px) {
      .profile-card__close { display: none; }
  }
  .profile-card__avatar {
      position: relative;
      width: 64px; height: 64px;
      border-radius: 50%;
      margin: -32px 0 0 16px;
      border: 3px solid var(--bg-1);
  }
  @media (max-width: 720px) {
      .profile-card__avatar { width: 84px; height: 84px; margin-top: -42px; }
  }
  .profile-card__name {
      font-family: var(--display);
      font-style: italic;
      font-size: 20px;
      padding: 8px 16px 0 16px;
  }
  @media (max-width: 720px) {
      .profile-card__name { font-size: 24px; }
  }
  /* ... (continues: .profile-card__pronouns, .profile-card__handle,
     .profile-card__nickname, .profile-card__status, .profile-card__bio,
     .profile-card__tagline, .profile-card__pinned, .profile-card__chips,
     .profile-card__meta, .profile-card__actions-primary,
     .profile-card__actions-secondary, .profile-card--self overrides) */
  ```

  Fill in the remaining selectors per spec §Peer view — each spec bullet gets its own selector.

- [ ] **Step 7.7 — `just check-wasm`.**

  ```bash
  just check-wasm
  ```

  Expected: clean.

- [ ] **Step 7.8 — Commit.**

  ```bash
  git add crates/web/src/components/profile_card.rs crates/web/style.css
  git commit -m "ui(phase-2c): render 17-field ProfileCardContent leaf"
  ```

### 8. Web — desktop `<ProfilePopover>` wrapper

Mount once at root. Subscribes to `use_profile_controller()`, positions against the anchor with flip/clamp, renders `<ProfileCardContent>`, owns outside-click dismissal.

**Files:** new `crates/web/src/components/profile_popover.rs`, modify `crates/web/src/components/mod.rs`, modify `crates/web/src/app.rs`, modify `crates/web/style.css`.

- [ ] **Step 8.1 — Component.** New `crates/web/src/components/profile_popover.rs`:

  ```rust
  //! Desktop profile-card popover wrapper.
  //!
  //! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
  //! §Desktop popover. Anchors the shared `ProfileCardContent` relative
  //! to the clicked avatar; flips to the left if it would overflow
  //! right; clamps horizontally; vertically anchored to anchor.top.

  use leptos::prelude::*;
  use wasm_bindgen::JsCast;

  use crate::profile::{close_profile, use_profile_controller};
  use crate::state::AppState;

  const WIDTH: f64 = 320.0;
  const GAP: f64 = 8.0;

  #[component]
  pub fn ProfilePopover() -> impl IntoView {
      let (open, _set_open) = use_profile_controller();
      let app_state = use_context::<AppState>().unwrap();
      // Only mount on desktop shells — the sheet handles mobile.
      let is_desktop = Signal::derive(move || {
          web_sys::window()
              .and_then(|w| w.document())
              .and_then(|d| d.body())
              .and_then(|b| b.get_attribute("data-shell"))
              .map(|s| s != "mobile")
              .unwrap_or(true)
      });

      let position = Signal::derive(move || {
          let state = open.get()?;
          let anchor = state.anchor.as_ref()?;
          let rect = anchor.get_bounding_client_rect();
          let win = web_sys::window()?;
          let vw = win.inner_width().ok()?.as_f64()?;
          let mut left = rect.right() + GAP;
          if left + WIDTH > vw - 12.0 {
              left = rect.left() - WIDTH - GAP;
          }
          left = left.max(12.0).min(vw - WIDTH - 12.0);
          let top = rect.top().max(12.0);
          Some((left, top))
      });

      let on_close = Callback::new(move |_| {
          close_profile();
      });

      view! {
          <Show when=move || is_desktop.get() && open.get().is_some() fallback=|| ()>
              {move || {
                  let state = open.get().unwrap();
                  let pos = position.get().unwrap_or((12.0, 12.0));
                  view! {
                      <div class="profile-popover"
                           style=format!("left: {}px; top: {}px;", pos.0, pos.1)
                           role="presentation">
                          <super::ProfileCardContent
                              view=Signal::derive(move || state.view.clone())
                              variant=if state.view.is_self { super::ProfileVariant::Self_ } else { super::ProfileVariant::Peer }
                              on_close=on_close/>
                      </div>
                  }
              }}
          </Show>
      }
  }
  ```

  Add an outside-click listener that closes when pointerdown lands outside `.profile-popover` AND outside the anchor. Attach one tick after open (to avoid closing on the originating click) via `set_timeout(.., 0)`.

- [ ] **Step 8.2 — CSS.**

  ```css
  .profile-popover {
      position: fixed;
      width: 320px;
      max-height: calc(100vh - 24px);
      overflow-y: auto;
      background: var(--bg-1);
      border: 1px solid var(--line);
      border-radius: 12px;
      box-shadow: var(--shadow-2);
      z-index: 200;
      animation: willow-pop-in var(--motion-fast, 180ms) ease-out;
  }
  @media (prefers-reduced-motion: reduce) {
      .profile-popover { animation: none; }
  }
  @media (max-width: 720px) {
      .profile-popover { display: none; }
  }
  ```

- [ ] **Step 8.3 — Register.** `crates/web/src/components/mod.rs`: add `mod profile_popover;` + `pub use profile_popover::ProfilePopover;`.

- [ ] **Step 8.4 — Mount in `<App>`.** `crates/web/src/app.rs` — add `<crate::components::ProfilePopover/>` next to the existing `<AddFriendDialog/>` mount.

- [ ] **Step 8.5 — Commit.**

  ```bash
  git add crates/web/
  git commit -m "ui(phase-2c): mount desktop ProfilePopover wrapper"
  ```

### 9. Web — mobile `<ProfileSheet>` wrapper

Mount once at root. Renders a scrim + translateY-in sheet holding `<ProfileCardContent>` when the controller signal fires AND the body shell is mobile.

**Files:** new `crates/web/src/components/profile_sheet.rs`, modify `crates/web/src/components/mod.rs`, modify `crates/web/src/app.rs`, modify `crates/web/style.css`.

- [ ] **Step 9.1 — Component.** New `crates/web/src/components/profile_sheet.rs`:

  ```rust
  //! Mobile profile-card bottom-sheet wrapper.
  //!
  //! Spec: `docs/specs/2026-04-19-ui-design/profile-card.md`
  //! §Mobile bottom sheet.

  use leptos::prelude::*;

  use crate::profile::{close_profile, use_profile_controller};

  #[component]
  pub fn ProfileSheet() -> impl IntoView {
      let (open, _set_open) = use_profile_controller();
      let is_mobile = Signal::derive(move || {
          web_sys::window()
              .and_then(|w| w.document())
              .and_then(|d| d.body())
              .and_then(|b| b.get_attribute("data-shell"))
              .map(|s| s == "mobile")
              .unwrap_or(false)
      });
      let on_close = Callback::new(move |_| close_profile());
      view! {
          <Show when=move || is_mobile.get() && open.get().is_some() fallback=|| ()>
              {move || {
                  let state = open.get().unwrap();
                  view! {
                      <>
                          <div class="profile-sheet__scrim"
                               on:click=move |_| close_profile()
                               role="presentation"></div>
                          <div class="profile-sheet" role="dialog"
                               aria-label=format!("profile — {}", state.view.display_name)>
                              <div class="profile-sheet__handle" aria-hidden="true"></div>
                              <super::ProfileCardContent
                                  view=Signal::derive(move || state.view.clone())
                                  variant=if state.view.is_self { super::ProfileVariant::Self_ } else { super::ProfileVariant::Peer }
                                  on_close=on_close/>
                          </div>
                      </>
                  }
              }}
          </Show>
      }
  }
  ```

  Browser-back dismissal: on open, push a transient `history.state` (`history.pushState({ profile_sheet: true }, "", "")`) and install a one-shot `popstate` listener that closes the sheet. On close initiated from code, `history.go(-1)` to pop the entry cleanly.

- [ ] **Step 9.2 — CSS.**

  ```css
  .profile-sheet__scrim {
      position: fixed; inset: 0;
      background: rgba(0, 0, 0, 0.45);
      z-index: 199;
      animation: fade-in var(--motion-med, 180ms) ease-out;
  }
  .profile-sheet {
      position: fixed;
      bottom: 0; left: 0; right: 0;
      background: var(--bg-1);
      border-top-left-radius: 22px;
      border-top-right-radius: 22px;
      max-height: 90vh;
      overflow-y: auto;
      padding-bottom: env(safe-area-inset-bottom, 0);
      z-index: 200;
      animation: willow-sheet-in var(--motion-slow, 220ms) ease-out;
  }
  .profile-sheet__handle {
      width: 44px; height: 5px;
      background: var(--line);
      border-radius: 999px;
      margin: 6px auto 0 auto;
  }
  @media (prefers-reduced-motion: reduce) {
      .profile-sheet, .profile-sheet__scrim { animation: none; }
  }
  @media (min-width: 721px) {
      .profile-sheet, .profile-sheet__scrim { display: none; }
  }
  @keyframes willow-sheet-in {
      from { transform: translateY(100%); }
      to { transform: translateY(0); }
  }
  @keyframes fade-in {
      from { opacity: 0; }
      to { opacity: 1; }
  }
  ```

- [ ] **Step 9.3 — Register + mount.** `components/mod.rs` + `app.rs` same pattern as the popover.

- [ ] **Step 9.4 — Commit.**

  ```bash
  git add crates/web/
  git commit -m "ui(phase-2c): mount mobile ProfileSheet wrapper"
  ```

### 10. Web — avatar click wiring across surfaces

Wire every avatar surface to dispatch `open_profile`. Surfaces listed in spec §Scope: grove rail, channel sidebar, message list, thread pane, members pane, letters list, call tile.

**Files:** modify `crates/web/src/components/grove_rail.rs`, modify `crates/web/src/components/channel_sidebar.rs`, modify `crates/web/src/components/message.rs`, modify `crates/web/src/components/member_list.rs`, modify `crates/web/src/components/participant_tile.rs`.

- [ ] **Step 10.1 — Grove rail.** Find the grove-rail peer avatar render (or server icon if applicable) and attach:

  ```rust
  on:click=move |ev: web_sys::MouseEvent| {
      let Some(target) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok()) else { return };
      crate::profile::open_profile(&peer_id, Some(target));
  }
  ```

- [ ] **Step 10.2 — Channel sidebar.** Same wiring on the "me" strip avatar + any DM avatar rows.

- [ ] **Step 10.3 — Message row author button.** Author buttons already carry `aria-label="{name} — open profile"` from message-row phase 2a. Replace the current no-op click (or the existing `on:click=ignore`) with `open_profile`.

- [ ] **Step 10.4 — Members pane.** `member_list.rs` — avatar click opens the card. Keep the existing roles / kick admin buttons unchanged.

- [ ] **Step 10.5 — Participant tile.** `participant_tile.rs` — tile avatar click → `open_profile`. Stop propagation so it doesn't trigger voice-related click handlers.

- [ ] **Step 10.6 — `just check-wasm`.**

  ```bash
  just check-wasm
  ```

  Expected: clean.

- [ ] **Step 10.7 — Commit.**

  ```bash
  git add crates/web/src/components/
  git commit -m "ui(phase-2c): wire avatar clicks on every surface to open_profile"
  ```

### 11. Web — private nickname inline editor

Inline editor on the peer card's secondary row. Reads/writes through `NicknameStoreHandle` from context. Enter saves, Escape cancels, blur saves, empty clears. 32-char cap.

**Files:** modify `crates/web/src/components/profile_card.rs`, modify `crates/web/style.css`, modify `crates/web/src/app.rs`.

- [ ] **Step 11.1 — Context plumbing.** In `<App>`, construct `let nickname_store: NicknameStoreHandle = Arc::new(WebNicknameStore::load());` and provide it via `provide_context(nickname_store.clone())` alongside the other context providers.

- [ ] **Step 11.2 — Editor state.** In `ProfileCardContent`:

  ```rust
  let nickname_store = use_context::<willow_client::NicknameStoreHandle>()
      .expect("NicknameStoreHandle in context");
  let (editing, set_editing) = signal(false);
  let (draft, set_draft) = signal(String::new());
  let stored_nickname = Signal::derive(move || {
      nickname_store.get(&view.get().peer_id)
  });
  ```

- [ ] **Step 11.3 — Handle line.** Replace the peer-variant handle row with either a plain handle (no nickname) or `{handle} · you call them {nickname}` (where `{nickname}` uses `--ink-2` and the prefix uses `--moss-3`). In self-view skip the nickname segment entirely.

- [ ] **Step 11.4 — Editor markup.** Secondary row button reads `set nickname` when `stored_nickname().is_none()` else `change nickname`. Click sets `editing.set(true)` + `set_draft(stored_nickname().unwrap_or_default())`. Pattern:

  ```rust
  view! {
      <Show when=move || editing.get() fallback=|| view!{
          <button class="nickname-editor__toggle" on:click=move |_| { set_editing.set(true); set_draft.set(stored_nickname.get().unwrap_or_default()); }>
              {move || if stored_nickname.get().is_some() { crate::profile::copy::CHANGE_NICKNAME } else { crate::profile::copy::SET_NICKNAME }}
          </button>
      }>
          <input class="nickname-editor__input"
                 aria-label=format!("nickname for {}", view.get().display_name)
                 prop:value=move || draft.get()
                 on:input=move |ev| set_draft.set(event_target_value(&ev))
                 on:keydown=move |ev: web_sys::KeyboardEvent| {
                     if ev.key() == "Enter" {
                         let v = draft.get();
                         nickname_store.set(&view.get().peer_id, &v);
                         set_editing.set(false);
                     } else if ev.key() == "Escape" {
                         set_editing.set(false);
                     }
                 }
                 on:blur=move |_| {
                     let v = draft.get();
                     nickname_store.set(&view.get().peer_id, &v);
                     set_editing.set(false);
                 }
                 maxlength="32"/>
      </Show>
  }
  ```

- [ ] **Step 11.5 — CSS.** Append:

  ```css
  .nickname-editor__input {
      font: 12px/1.4 var(--mono);
      background: var(--bg-2);
      border: 1px solid var(--line);
      border-radius: 6px;
      padding: 2px 6px;
      color: var(--ink-1);
  }
  .nickname-editor__toggle {
      font: inherit;
      color: var(--ink-3);
      background: none;
      border: none;
      padding: 0;
      cursor: pointer;
  }
  ```

- [ ] **Step 11.6 — Commit.**

  ```bash
  git add crates/web/src/ crates/web/style.css
  git commit -m "ui(phase-2c): add private nickname inline editor (local-only)"
  ```

### 12. Browser tests — `phase_2c_profile_card` module

Cover the controller, the shared leaf, the two wrappers, the crest, the nickname editor, the badge click handoff. No multi-peer behaviour needed.

**Files:** modify `crates/web/tests/browser.rs`.

- [ ] **Step 12.1 — Append module.** At the bottom of `browser.rs`:

  ```rust
  mod phase_2c_profile_card {
      use super::*;

      fn sample_view() -> std::sync::Arc<willow_client::ProfileView> {
          use willow_client::ProfileView;
          std::sync::Arc::new(ProfileView {
              peer_id: "peer-1".into(),
              handle: "mira.sage".into(),
              display_name: "mira".into(),
              pronouns: Some("she/her".into()),
              bio: Some("gardener".into()),
              tagline: Some("tending the moss".into()),
              crest_pattern: Some(willow_state::CrestPattern::Leaf),
              crest_color: Some("#6b8e4e".into()),
              pinned: Some(willow_state::PinnedFragment {
                  kind: willow_state::PinnedKind::Quote,
                  body: "quiet is a kind of music".into(),
              }),
              elsewhere: vec!["coast · west".into()],
              since: Some("spring · yr 2".into()),
              fingerprint_short: "one · two · three".into(),
              fingerprint_full: "one · two · three · four · five · six".into(),
              is_self: false,
          })
      }

      #[wasm_bindgen_test]
      fn leaf_renders_all_peer_fields() {
          let c = mount_test(move || {
              let v = sample_view();
              let view = Signal::derive(move || v.clone());
              view! {
                  <ProfileCardContent view=view variant=ProfileVariant::Peer
                      on_close=Callback::new(|_| ())/>
              }
          });
          assert!(c.text_content().unwrap().contains("mira"));
          assert!(c.text_content().unwrap().contains("she/her"));
          assert!(c.text_content().unwrap().contains("mira.sage"));
          assert!(c.text_content().unwrap().contains("gardener"));
          assert!(c.text_content().unwrap().contains("tending the moss"));
          assert!(c.text_content().unwrap().contains("quiet is a kind of music"));
          assert!(c.text_content().unwrap().contains("coast · west"));
          assert!(c.text_content().unwrap().contains("spring · yr 2"));
          // Primary row copy
          assert!(c.text_content().unwrap().contains("message"));
          assert!(c.text_content().unwrap().contains("start call"));
          assert!(c.text_content().unwrap().contains("whisper"));
          // Secondary row copy
          assert!(c.text_content().unwrap().contains("copy fingerprint"));
      }

      #[wasm_bindgen_test]
      fn leaf_self_variant_shows_edit_profile() {
          // sample_view() with is_self = true
          // assert contains "edit profile" and "this is you" and NOT "copy fingerprint"
      }

      #[wasm_bindgen_test]
      fn leaf_omits_missing_peer_fields_except_card_when_self() {
          // view with pronouns = None, bio = None, pinned = None
          // Peer variant: none of those section headers appear.
          // Self variant: "no pinned fragment" placeholder appears.
      }

      #[wasm_bindgen_test]
      fn crest_falls_back_to_leaf_moss_when_unset() {
          // CrestBanner with pattern=None color=None renders a .profile-card__crest with at least one <path>
          // and the fill/stroke uses var(--moss-2).
      }

      #[wasm_bindgen_test]
      fn crest_is_deterministic_for_same_peer_id() {
          // Mount two CrestBanners with the same peer_id and pattern.
          // Compare serialized innerHTML. Expect equal.
      }

      #[wasm_bindgen_test]
      async fn open_profile_event_triggers_controller() {
          // Mount <App> via the existing harness. Dispatch CustomEvent(PROFILE_OPEN_EVENT).
          // tick().await
          // Expect document.querySelector(".profile-popover") exists on desktop shell.
      }

      #[wasm_bindgen_test]
      async fn close_profile_event_dismisses_popover() {
          // Given the card is open, dispatch PROFILE_CLOSE_EVENT.
          // tick().await; assert .profile-popover absent.
      }

      #[wasm_bindgen_test]
      async fn escape_key_closes_popover() {
          // Open, then dispatch keydown Escape. Assert popover gone.
      }

      #[wasm_bindgen_test]
      async fn badge_click_sets_compare_target() {
          // Mount card w/ a peer whose trust state is Unverified.
          // Click .profile-card__badge — assert AppState.trust.compare_target becomes Some(peer_id)
          // AND the card closes (controller goes to None).
      }

      #[wasm_bindgen_test]
      async fn nickname_editor_save_on_enter() {
          // Open, click "set nickname", type "mira", press Enter.
          // Assert NicknameStore.get("peer-1") == Some("mira").
      }

      #[wasm_bindgen_test]
      async fn nickname_editor_escape_cancels() {
          // With empty starting nickname, open editor, type "foo", Escape.
          // Assert NicknameStore.get returns None.
      }

      #[wasm_bindgen_test]
      async fn nickname_editor_empty_clears() {
          // With existing nickname "mira", open editor, clear input, blur.
          // Assert NicknameStore.get returns None.
      }

      #[wasm_bindgen_test]
      async fn avatar_click_on_message_row_dispatches_open() {
          // Mount MessageList with a single message.
          // Click the author button. Listen for PROFILE_OPEN_EVENT on window.
          // Assert the event fires with the expected user_id in detail.
      }

      #[wasm_bindgen_test]
      async fn mobile_sheet_renders_on_mobile_shell() {
          let c = mount_test_with_shell(TestShell::Mobile, || {
              view! { <crate::components::ProfileSheet/> }
          });
          // Dispatch open event. tick. Assert .profile-sheet exists, .profile-popover absent.
      }
  }
  ```

  Each `// ...` above is a real test to fill in — the spec's §Acceptance criteria + §Edge cases drives the set. Capture exact assertions against the copy strings from `crate::profile::copy` to make them load-bearing.

- [ ] **Step 12.2 — Commit.**

  ```bash
  git add crates/web/tests/browser.rs
  git commit -m "ui(phase-2c): add phase_2c_profile_card browser test module"
  ```

### 13. Accessibility sweep — focus, ARIA, reduced motion, SR order

Wire the focus-on-open / focus-return-on-close contract, confirm all ARIA labels per spec §Accessibility, validate reduced-motion paths, confirm screen-reader reading order follows the visual order.

**Files:** modify `crates/web/src/components/profile_popover.rs`, modify `crates/web/src/components/profile_sheet.rs`, modify `crates/web/src/components/profile_card.rs`, modify `crates/web/style.css`.

- [ ] **Step 13.1 — Focus management.** On open, both wrappers call `element.focus()` on the first focusable element inside `.profile-card` (the primary-action-row first button, or the close button on desktop if present). Track the anchor element in an `RwSignal` so close can call `anchor.focus()` to return focus.

- [ ] **Step 13.2 — Role + aria-label.** Confirm `<ProfileCardContent>` root carries `role="dialog"` + `aria-label={format!("profile — {}", view.display_name)}`.

- [ ] **Step 13.3 — Reading order.** Spec §Accessibility requires the banner badge pill be read immediately after the avatar. Ensure DOM order is:
  1. crest banner (`aria-hidden="true"`)
  2. close button (desktop only) — but `aria-label="close profile"`, not read first because avatar is in flow
  3. avatar (alt-text via the author-name label)
  4. verification badge pill (live `aria-label` reflecting current trust state)
  5. display name
  6. pronouns, handle, status ...

  If DOM order doesn't match, restructure the template. Double-check with the test `leaf_renders_all_peer_fields` above.

- [ ] **Step 13.4 — Reduced motion.** Crest banner, pop-in, sheet-slide, nickname-editor ring all collapse to opacity fades under `prefers-reduced-motion: reduce` (already in the CSS emitted during earlier tasks — audit with a grep).

- [ ] **Step 13.5 — SR live region on live update.** When the controller updates `ProfileState` for the same user (cross-fade case), the `aria-live="polite"` region inside `.profile-card__bio` announces the new content.

- [ ] **Step 13.6 — 44×44 touch targets.** All mobile buttons meet the baseline. Audit CSS selectors on `.profile-sheet .profile-card__actions-primary button` and `.profile-card__actions-secondary a`.

- [ ] **Step 13.7 — Browser test.** Append a test `leaf_has_role_dialog_and_aria_label` in `phase_2c_profile_card`:

  ```rust
  #[wasm_bindgen_test]
  fn leaf_has_role_dialog_and_aria_label() {
      let c = mount_test(move || view! { <ProfileCardContent view=.. variant=.. on_close=../> });
      let root = c.query_selector(".profile-card").unwrap().unwrap();
      assert_eq!(root.get_attribute("role").as_deref(), Some("dialog"));
      assert_eq!(root.get_attribute("aria-label").as_deref(), Some("profile — mira"));
  }
  ```

- [ ] **Step 13.8 — Commit.**

  ```bash
  git add crates/web/
  git commit -m "ui(phase-2c): wire profile-card a11y (focus, role, reduced motion)"
  ```

### 14. Acceptance sweep + self-review + tick boxes

Final sweep: every row in spec §Acceptance criteria mapped to a test or a wiring; tick the boxes in this plan; run fmt + clippy; open the PR.

**Files:** modify `docs/plans/2026-04-21-ui-phase-2c-profile-card.md`, possibly small polish across `crates/web/src/components/profile_card.rs`.

- [ ] **Step 14.1 — Walk §Acceptance.** For each bullet in spec §Acceptance criteria (17 rows), confirm a test or codepath exists. Append any missing test to `phase_2c_profile_card`. If a test is too expensive (real browser positioning math), note it as deferred under Ambiguity decisions.

- [ ] **Step 14.2 — Long-display-name truncation.** CSS: `.profile-card__name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }` on desktop only; add `title` attribute carrying the full name for hover.

- [ ] **Step 14.3 — `just fmt` + `just clippy`.**

  ```bash
  just fmt
  just clippy
  ```

  Expected: both clean. Fix roots — NEVER use `#[allow]` to silence clippy unless the lint objectively doesn't apply (e.g. tests).

- [ ] **Step 14.4 — Tick plan checkboxes.** Walk this file top-to-bottom, flip every `[ ]` to `[x]` for completed tasks, and add a trailing commit note summarising deferred items (if any) in the Ambiguity decisions section.

- [ ] **Step 14.5 — Commit.**

  ```bash
  git add docs/plans/2026-04-21-ui-phase-2c-profile-card.md crates/web/
  git commit -m "ui(phase-2c): acceptance sweep + tick plan checkboxes"
  ```

- [ ] **Step 14.6 — Open PR.**

  ```bash
  gh pr create --title "ui(phase-2c): profile-card — plan + implementation" \
    --body "$(cat <<'EOF'
  ## Summary

  Plan + implementation for docs/specs/2026-04-19-ui-design/profile-card.md
  in a single PR.

  - Shared content component + desktop popover wrapper + mobile bottom-sheet
  - Event-bus entry from every avatar surface
  - All 17 peer-view fields (crest banner, badges, pronouns, bio, tagline,
    pinned fragment, shared groves, elsewhere, fingerprint)
  - Self view variant
  - Private nickname inline editor (local-only)
  - Badge tap → trust-verification compare flow handoff

  New state on `Profile`: crest_pattern, crest_color, pronouns, bio,
  tagline, pinned, elsewhere, since. One new EventKind::UpdateProfile.

  Test tiers:
  - state: UpdateProfile dedup / apply / permission
  - client: Profile field derivations, nickname store
  - browser: popover + sheet mount, field rendering, variant flags,
    event bus click handoff
  - Playwright: verification-badge multi-peer sync (if needed)

  ## Test plan

  - [ ] just fmt --check passes
  - [ ] just clippy zero warnings
  - [ ] cargo test -p willow-state
  - [ ] cargo test -p willow-client
  - [ ] just test-browser (CI)

  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  EOF
  )"
  ```

## Ambiguity decisions

- **UpdateProfile shape.** Rather than separate EventKinds per field (`SetPronouns`, `SetBio`, etc.) the phase ships a single `UpdateProfile` delta event. Rationale: matches spec §Data dependencies note ("New grove-propagated fields should land as a single `SetProfile` event kind carrying an optional update for each field"), cuts match-arm count, keeps DAG commit rate low.
- **Legacy `SetProfile` kept.** `EventKind::SetProfile { display_name }` stays for wire-compat. `UpdateProfile { display_name: Some(..), .. }` is a superset — future state-machine cleanup can fold them.
- **Nickname v1 is local-only.** Spec §Open questions v1 = local. No `SetNickname` EventKind. Stored via `WebNicknameStore` (localStorage).
- **Crest-color palette.** Spec §Open questions v1 restricts to the accent palette. We enforce the 7-char `#RRGGBB` shape; free-form gate deferred.
- **`message` / `call` / `whisper` primary actions.** `message` closes the card and switches to the 1:1 letters channel for `peer_id` if one exists (TODO: `letters-dms.md` hasn't landed the DM channel model; fall back to opening a composer targeted at the peer). `call` + `whisper` render the button with a `TODO(call-experience.md)` / `TODO(whisper-mode.md)` comment; click is a no-op until those phases land.
- **`block` / `more`.** `block` TODO-gated on `governance.md`; `more` renders as a `…` button with `aria-label="more actions"` and no menu in v1.
- **Sheet drag-to-dismiss.** Spec §Open questions v2. v1 uses scrim tap + back gesture + Escape.
- **Anchor null mid-lifecycle.** The controller stores the anchor in a `send_wrapper::SendWrapper<HtmlElement>`. If the anchor is unmounted, positioning falls back to the last computed `(left, top)` — spec §Anchor contract.
- **Profile controller lives in `crates/web/src/profile/`, not `components/`.** The module is app-shell infrastructure (event bus + signal), not a visual component.
- **`ProfileCardStub` removal.** Kept as a deprecated thin wrapper that forwards to `<ProfileCardContent>`; the stub was only ever called from presence.md surfaces, and those sites get migrated in a follow-up sweep after this PR merges.
- **`willow_identity::handle` / `fingerprint_words` helpers.** If they don't exist, add `pub fn handle(pid: &EndpointId) -> String` (first 8 hex chars lowercased) and reuse the 6-word mnemonic function already shipped in `willow-crypto::sas` for `fingerprint_words`. If these are genuinely new, add a small task before Task 4 to introduce them.
- **Browser-test harness for the mobile sheet.** `mount_test_with_shell(TestShell::Mobile, ..)` already exists; we reuse it.
- **No e2e / Playwright tests this phase.** Spec doesn't call for a multi-peer profile sync test. The verification-badge round-trip is owned by `trust-verification.md`'s already-green Playwright cases.

## Self-review

- [ ] Every spec §Acceptance row mapped to a task or test.
- [ ] Foundation tokens only — `--moss-*`, `--amber`, `--warn`, `--ink-*`, `--bg-*`, `--line-*`, `--motion-*`, `--shadow-2`. No new hex.
- [ ] Every commit is `ui(phase-2c): <imperative>` except the initial `docs(plan):` commit.
- [ ] Test tiers follow the CLAUDE.md decision tree — state crate: event apply/dedup/permission; client crate: view derivation + nickname store + mutation; browser: component DOM + controller + event-bus.
- [ ] Lowest-tier coverage — no Playwright this phase (spec doesn't require multi-peer).
- [ ] No placeholders, no TBDs.
- [ ] `feedback_e2e_in_sync` memory respected — no e2e helpers to add (no e2e spec touched).
- [ ] `feedback_keep_specs_in_sync` — spec is stable; no spec edits needed.
- [ ] `feedback_delete_annotations_when_addressed` — any vibe annotations filed mid-implementation get deleted immediately.
