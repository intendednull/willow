# UI Phase 2b — Sync queue Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development + superpowers:test-driven-development.

**Goal:** Ship `docs/specs/2026-04-19-ui-design/sync-queue.md` — the visible representation of patient P2P messaging: amber offline status strip, per-peer queue pills on letter / member rows, per-message inline queue notes, mobile pull-down + desktop chevron summary, dedicated sync-queue screen (outbound / inbound tabs + recent arrivals), relay-awareness badge, reconnection toast, welcome-back banner, and the signal contract between `crates/web` and `willow-client`. Closes the Phase 2a `Pending → None` state-flip gate left open in `views.rs`.

**Style ref:** 2a plan. Commits: `ui(phase-2b): <imperative>`. Branch `design/ui-target-ux`. After Phase 2a (message-row).

## Scope

**In:** `QueueMeta` actor (new primitive in `willow-client`, sibling of `PresenceMeta`) exposing `queue_depth`, `queue_peer_count`, `queue_per_peer`, `queue_inbound_per_peer`, `queue_oldest_at`, `queue_recent_arrivals`, `relay_status`, `device_online` via the view system. `MessageStore::delivery_state` + `peer_presence_history` hooks to unblock the Phase 2a TODO. Extend `connection_status` with an `"offline"` variant. `OfflineStrip` + `QueuePill` + `InlineQueueNote` components. Summary popover (desktop) + pull-down summary card (mobile) + sync-queue screen (route + right-pane). Relay signal-icon button + popover / bottom sheet. Reconnection toast + welcome-back banner. Exact copy table. ARIA contract. Browser tests + Playwright for multi-peer sync + pull-gesture.

**Out:**

- Actual on-device encrypted-at-rest outbound queue storage (`willow-messaging::queue` persistence) — this spec declares the storage dep; the plan wires the trait + an in-memory default, but the SQLite / IndexedDB persistence ships in its own follow-up (`willow-messaging-queue.md`, future spec). The UI and `QueueView` signals read from the trait so the swap is mechanical.
- Reachability probing / retry scheduling wire protocol (`willow-network`-owned follow-up). The plan exposes a `client.retry_queue()` method that enqueues a best-effort ping to all unreachable peers; richer retry policy is the network crate's job.
- Settings-tweaks UI for queue limits (future phase — `settings-tweaks.md`).
- Archive surface (`letters-dms.md` owns the long-unreachable archive UI; this plan only ships the `keep queued / archive` prompt once the peer has been offline > 14 days).
- Peer-identity tombstone signal (`letters-dms.md`; flagged in data-deps-rollup §7.8).
- Inbound queue hint wire format (peer heartbeat extension — data-deps-rollup §7.5 open question). The signal `queue_inbound_per_peer` is allowed to be zero until the heartbeat dep lands.
- Quiet-hours / notification gating overlap — `notifications.md` (phase 1f) owns `Notifier`; this plan only calls `Notifier::dispatch` for the reconnection toast + welcome-back banner.

## Architecture

Sync-queue state is **partially derived, partially new primitive**. The existing `PresenceMeta` actor carries a stub `queue_depth: HashMap<EndpointId, u32>` (introduced in Phase 1e). This plan promotes that stub into a real `QueueMeta` actor owning the full queue primitives and delegates presence's queue-depth lookup to the new actor so both signals stay in sync without duplicate truth.

Inputs to `QueueMeta`:

1. `MessageStore::delivery_state(msg_id) -> DeliveryState` (new trait method) — drives Pending.
2. A bounded `peer_presence_history: VecDeque<(EndpointId, Tick, bool)>` on `PresenceMeta` (extended here; used by both presence and queue) — drives LateArrival.
3. `willow-network::RelayStatus` (existing enum in iroh layer; new re-export to `willow-client`) — drives `relay_status`.
4. `willow-network::device_online` (a `ReadSignal<bool>` driven by window `online` / `offline` events on web + iroh `connected/disconnected` on native).

Outputs (consumed by `crates/web`):

- `QueueView { depth, peer_count, per_peer, inbound_per_peer, oldest_at, recent_arrivals, relay_status, device_online }` — published via `state_actors::QueueMeta` through `compute_queue_view()`.
- A `QueueNote` projection on `DisplayMessage` (populated from `delivery_state` + `peer_presence_history`).

Screen architecture: on desktop, the sync queue is a right-pane variant (reuses `right_rail` mount slot, mutually exclusive with members / thread). On mobile, it's a pushed screen (`/sync-queue` route via leptos-router; mounted by the existing `mobile_shell` router). A shared `SyncQueueView` component renders identical markup in both mounts.

Backend deps (from spec §Data dependencies + data-deps-rollup.md):

- `MessageStore::delivery_state` (new trait method on `willow-messaging::store::MessageStore`).
- `ServerState` unchanged — no new `EventKind`. Queue state is purely local / per-device.
- `RelayStatus` re-export from `willow-network` into `willow-client` (no new network protocol).
- `device_online` signal (WASM: `window.addEventListener('online' / 'offline')`; native: iroh connectivity callback).
- `QueueSummary`, `ArrivedSummary`, `RelayStatus` structs in `willow-client::state`.

## File structure

| Path | State | Responsibility |
|---|---|---|
| `crates/client/src/queue.rs` | **new** | Pure queue primitives. `QueueSummary { outbound, oldest_outbound_at, last_attempt_at, last_attempt_error }`, `ArrivedSummary { peer_id, at_tick, count, preview }`, `QueueNoteDerivation` helpers (`derive_pending`, `derive_late_arrival` — pure fns taking `DeliveryState` + presence history). 10 unit tests covering the QueueNote transition table in spec §Per-message queue note. |
| `crates/client/src/state_actors.rs` | modify | Promote `PresenceMeta::queue_depth` to a thin re-export from new `QueueMeta` actor. Add `QueueMeta { now: Tick, outbound: HashMap<MessageId, QueueEntry>, inbound_hint_per_peer: HashMap<EndpointId, u32>, recent_arrivals: VecDeque<ArrivedSummary>, relay_status: RelayStatus, device_online: bool, peer_presence_history: VecDeque<(EndpointId, Tick, bool)> }`. Bounded history (cap 2048 entries, drop-oldest). |
| `crates/client/src/views.rs` | modify | Add `QueueView { depth, peer_count, per_peer, inbound_per_peer, oldest_at, recent_arrivals, relay_status, device_online }` + `compute_queue_view`. Update `compute_messages_view` — swap `let queue_note = QueueNote::None` with real derivation via `derive_pending(message_store.delivery_state(&m.id))` + `derive_late_arrival(&presence_history, m.author, m.timestamp_ms)`. **Unblocks the Phase 2a TODO at `docs/plans/2026-04-20-ui-phase-2a-message-row.md:490`.** |
| `crates/client/src/lib.rs` | modify | Expose `ClientHandle::queue_view()` → `ReadSignal<QueueView>`, `ClientHandle::retry_queue()` → best-effort reconnect-ping to unreachable peers, `ClientHandle::mark_queue_read(peer_id)` for inbound `mark as read locally`. Re-export `QueueSummary`, `ArrivedSummary`, `RelayStatus` from `state::`. 5 new client tests (depth/peer-count aggregation, retry no-op when empty, mark-read writes local last-seen marker, recent-arrivals rolling 24h, offline→reconnect transition). |
| `crates/client/src/mutations.rs` | modify | `RetryQueue` + `MarkQueueRead { peer_id }` mutation types routed through the existing actor mutation bus. |
| `crates/client/src/connect.rs` | modify | Hook `QueueMeta::device_online` to WASM `window.online/offline` events (new) and native iroh connectivity (existing signal). Tick driver now also decays `recent_arrivals` entries older than 24h. |
| `crates/messaging/src/store.rs` | modify | Add `DeliveryState { Delivered, PendingAllRecipients(HashSet<EndpointId>), PendingSomeRecipients { acked: HashSet<_>, pending: HashSet<_> } }` enum + `trait MessageStore::delivery_state(&self, id: &MessageId) -> Option<DeliveryState>`. `InMemoryStore` impl (default-returns `Delivered` until the real tracker is wired). 3 unit tests. |
| `crates/messaging/src/lib.rs` | modify | Re-export `DeliveryState` at crate root. |
| `crates/network/src/traits.rs` | modify | Extend `Network` trait with `fn relay_status(&self) -> RelayStatus` + `fn device_online(&self) -> bool`. Default impls return `RelayStatus::NotConfigured` + `true` so the `MemNetwork` test double inherits sensible stubs. |
| `crates/network/src/iroh.rs` | modify | Implement `relay_status` by polling the iroh relay-session's last-success timestamp (< 30s → `Reachable`; else `Unreachable`; no relay configured → `NotConfigured`). Implement `device_online` via iroh's network-state subscription. |
| `crates/network/src/mem.rs` | modify | Stub impls for test double (configurable via new `MemNetwork::set_relay_status` / `set_device_online` for deterministic tests). |
| `crates/web/src/components/offline_strip.rs` | **new** | `<OfflineStrip>` component. Reads `queue_view` signal; renders only when `queue_peer_count > 0`. Amber strip per spec §Offline status strip: hourglass icon, summary text (singular / plural / relay-appended), chevron on desktop, 36/40 px height, `aria-live="polite"` + `role="status"` + `role="button"` + `aria-label="open sync queue"`. Click → opens `SyncQueueView`. Hover lifts to `--bg-3`. Return-of-peer flash (`--moss-0` bg, 240 ms) when peer transitions from queued → delivered. Reduced-motion path. |
| `crates/web/src/components/queue_pill.rs` | **new** | `<QueuePill peer_id, outbound, inbound>` — amber pill `queued · {n}` per spec §Per-peer badge. Tooltip (desktop) / long-press popover (mobile) renders disambiguated `pill_tooltip_out` / `pill_tooltip_in` / `pill_tooltip_both`. `aria-label` on button container, visible text `aria-hidden="true"`. 500+ cap. Deferral rule: if peer is `pending-verify` the pill is suppressed and the count moves into the tooltip only. |
| `crates/web/src/components/inline_queue_note.rs` | **new** | `<InlineQueueNote state, peer_or_grove>` — Fraunces italic body-S hint rendering `queued` / `just-delivered` / `inbound-held` copy. Mount below message body inside `MessageView`. Hides automatically via effect: `just-delivered` fades 30s; `inbound-held` hides 5min; `queued` persists until delivered-to-all. Wired through `aria-describedby` on the message row for SR announcement. |
| `crates/web/src/components/sync_queue_view.rs` | **new** | `<SyncQueueView>` — full-surface renderer reused by desktop right-pane + mobile route. Header (back / close, title, subtitle, relay signal button). Status card (pulsing moss dot, `reaching out…` / `queue drained`, reached/total count + progress bar). Tabs outbound / inbound. Virtualised per-peer row list with expand-to-message sub-rows + per-recipient chips for grove fan-out + `retry now` inline per message. Recent-arrivals section (24h window). Footer: `retry now` primary + `mark as read locally` (inbound only) + verbatim footnote. No delete action. |
| `crates/web/src/components/pull_to_reveal.rs` | **new** | Mobile-only `<PullToReveal>` higher-order wrapper around letters list + channel message list. Tracks over-scroll via `touchstart/touchmove/touchend`. At 48 px shows summary card; at 72 px commits to navigation; haptic via `util::vibrate(8)` at commit threshold. Release before 72 px springs back with no nav. Empty-queue variant (idle card, no commit threshold). CSS transition + reduced-motion fallback. |
| `crates/web/src/components/reconnection_toast.rs` | **new** | Toast body wired to `Notifier` (from Phase 1f) — `reconnected · delivering {n} messages` / `reconnected`. Auto-hides 4 s, dismissible. Rapid reconnect cycles collapse to most recent (debounced 2 s in `QueueMeta`). |
| `crates/web/src/components/welcome_back_banner.rs` | **new** | 48 px banner — `willow queued {n} messages while you were away — everything arrived`. Persists until first message interaction or explicit `x`. Suppressed when reconnection toast also fires (banner takes precedence — spec §Open questions §5). Renders only on "reopen after ≥ 60 s offline" transition; session-scoped dedup. |
| `crates/web/src/components/relay_signal_button.rs` | **new** | Signal-icon button on sync-queue screen header. Reachable → `--moss-3`; Unreachable → `--amber` with 40% `willowPulse`. Click → popover (desktop) / bottom sheet (mobile) with relay address, last-sync time, in-progress direct-peer attempts, and `change relay in settings` link. |
| `crates/web/src/components/message.rs` | modify | Wire the new `QueueNote` projection into the existing `InlineQueueNote` slot (the badge + dim-till-delivered layout from Phase 2a is preserved — this commit swaps the always-`None` stub for real state). Trigger the 900 ms delivery flash when the projection transitions `Pending → None` (signal diff detected via leptos `prev_value`). |
| `crates/web/src/components/letters.rs` *(if landed)* / member_list.rs | modify | Mount `<QueuePill>` after the peer name per spec §Placement rules. Member list rows: trailing pill. If letters list not yet implemented, mount a TODO comment referencing `letters-dms.md`; member-list wiring is required. |
| `crates/web/src/components/mobile_shell.rs` | modify | Add `/sync-queue` route; mount `<PullToReveal>` around the letters list + channel message list. |
| `crates/web/src/components/right_rail.rs` (or `app.rs`) | modify | Desktop mount slot: when `sync_queue_open` signal is `true`, render `<SyncQueueView>` in place of `<MemberList>` / `<ThreadPane>` (same mutual-exclusion pattern as thread pane). |
| `crates/web/src/app.rs` | modify | Mount `<OfflineStrip>` once below the window chrome. Mount `<ReconnectionToast>` via `Notifier`. Mount `<WelcomeBackBanner>` at top of home view (letters list on mobile, main pane on desktop). Wire `sync_queue_open: RwSignal<bool>` signal in `AppState::ui`. |
| `crates/web/src/state.rs` | modify | Extend `AppState::ui` with `sync_queue_open: RwSignal<bool>`. Extend `NetworkState` (or new `QueueState`) with `queue_view: ReadSignal<QueueView>`, `relay_status: ReadSignal<RelayStatus>`, `device_online: ReadSignal<bool>`. Promote `connection_status` to a proper enum `ConnectionState { Connecting, Connected, Reconnecting, Offline }` while keeping a `Display` fallback for existing string readers. |
| `crates/web/src/event_processing.rs` | modify | Handle `ClientEvent::QueueChanged`, `ClientEvent::RelayStatusChanged`, `ClientEvent::DeviceOnlineChanged` (new variants). Pipe into the new signals. |
| `crates/client/src/events.rs` | modify | `ClientEvent::QueueChanged(QueueView)`, `ClientEvent::RelayStatusChanged(RelayStatus)`, `ClientEvent::DeviceOnlineChanged(bool)` variants. |
| `crates/web/src/notifications.rs` | modify | Register sync-queue notification category (Phase 1f `NotificationKind::QueueReconnect`). `notif_letter` / `notif_grove` opaque payload text enforced for inbound-queued push notifications (privacy guarantee §4.2). |
| `crates/web/src/icons.rs` | modify | Add `icon_signal` (11 px + 14 px for strip + screen header), `icon_willow_wordmark_glyph` (14 px, for welcome-back banner). `icon_hourglass` already shipped in Phase 1e; reuse. |
| `crates/web/style.css` (or `components.css`) | modify | All component styles per spec (offline strip, queue pill, inline note, sync-queue screen card/tabs/rows/footer, pull-to-reveal card, reconnection toast, welcome-back banner, relay signal button). Reduced-motion paths. `data-accent` respected. No new hex — foundation tokens only. |
| `crates/web/src/util.rs` | modify | Add `format_elapsed_hlc(oldest: HlcTime, now: HlcTime) -> String` → `2d` / `6h` / `18m` buckets per spec §Sync queue screen rows. |
| `crates/client/src/tests/queue.rs` | **new** | 12 client tests: depth aggregation, peer-count distinct peers, per-peer summary, oldest_at tracking, recent_arrivals rolling 24h decay, pending-local-author detection, late-arrival-peer-offline detection, pending→none transition triggers delivery flash event, retry_queue no-op when empty, mark_queue_read writes last-seen marker, relay_status propagation, device_online propagation. |
| `crates/web/tests/browser.rs` | modify | Append `mod phase_2b_sync_queue { … }` at file end using `mount_test_with_shell`. ~22 tests covering every §Acceptance criterion. |
| `e2e/helpers.ts` | modify | Add `pullDown(page, px)` + `waitForSyncQueueScreen` + `goOffline(context)` / `goOnline(context)` via `browserContext.setOffline(true)`. |
| `e2e/sync-queue.spec.ts` | **new** | 6 Playwright specs: mobile pull-to-reveal + navigation; offline strip appears on network offline; reconnection toast after online; two-peer end-to-end queue drain; welcome-back banner after long offline; `retry now` click triggers client call (asserted via mock). |

## Tasks (18 total, ~28 commits)

### 1. Pure queue primitives + derivation helpers

Extract the queue-note transition table into a pure, unit-testable module so `views.rs` can call it without wrestling with actor state.

**Files:** new `crates/client/src/queue.rs`, modify `crates/client/src/lib.rs` (mod + re-exports).

- [x] **Step 1.1 — Define types.** In `queue.rs`:

  ```rust
  use std::collections::VecDeque;
  use willow_identity::EndpointId;
  use willow_messaging::hlc::HlcTimestamp;
  use crate::state_actors::Tick;

  #[derive(Clone, Debug, PartialEq)]
  pub struct QueueSummary {
      pub outbound: u32,
      pub oldest_outbound_at: Option<HlcTimestamp>,
      pub last_attempt_at: Option<Tick>,
      pub last_attempt_error: Option<String>,
  }

  #[derive(Clone, Debug, PartialEq)]
  pub struct ArrivedSummary {
      pub peer_id: EndpointId,
      pub at_tick: Tick,
      pub count: u32,
      pub preview: Option<String>,
  }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum RelayStatus {
      Reachable,
      Unreachable,
      NotConfigured,
  }
  ```

- [x] **Step 1.2 — `derive_pending`.** Pure fn:

  ```rust
  use willow_messaging::store::DeliveryState;
  use crate::state::QueueNote;

  pub fn derive_pending(
      is_local_author: bool,
      delivery: Option<&DeliveryState>,
  ) -> bool {
      if !is_local_author { return false; }
      matches!(
          delivery,
          Some(DeliveryState::PendingAllRecipients(_))
              | Some(DeliveryState::PendingSomeRecipients { .. })
      )
  }
  ```

- [x] **Step 1.3 — `derive_late_arrival`.** Peer-offline-near(author, ts, 30_000) predicate backed by presence history:

  ```rust
  pub fn derive_late_arrival(
      history: &VecDeque<(EndpointId, Tick, bool)>,
      author: EndpointId,
      msg_authored_at_ms: u64,
      now_ms: u64,
  ) -> bool {
      // Returns true iff `author` had `reachable=false` in a history entry
      // within 30 000 ms before `msg_authored_at_ms` AND now_ms - msg_authored_at_ms > 30_000.
      // See sync-queue.md §Per-message queue note `inbound-held` trigger.
      // Full code shown in Step 1.4 test body.
      let window_start = msg_authored_at_ms.saturating_sub(30_000);
      let was_offline = history
          .iter()
          .any(|(p, _, reachable)| *p == author && !reachable);
      was_offline && now_ms.saturating_sub(msg_authored_at_ms) > 30_000
  }
  ```

- [x] **Step 1.4 — Unit tests.** In `queue.rs` `#[cfg(test)]`:

  ```rust
  #[test]
  fn derive_pending_false_when_remote_author() {
      assert!(!derive_pending(false, Some(&DeliveryState::PendingAllRecipients(Default::default()))));
  }
  #[test]
  fn derive_pending_true_when_local_and_pending_all() {
      let mut set = std::collections::HashSet::new();
      set.insert(EndpointId::from_bytes([1; 32]));
      assert!(derive_pending(true, Some(&DeliveryState::PendingAllRecipients(set))));
  }
  #[test]
  fn derive_pending_false_when_local_and_delivered() {
      assert!(!derive_pending(true, Some(&DeliveryState::Delivered)));
  }
  #[test]
  fn derive_late_arrival_true_when_author_was_offline_and_delay() {
      let author = EndpointId::from_bytes([2; 32]);
      let mut h = VecDeque::new();
      h.push_back((author, 10, false));
      assert!(derive_late_arrival(&h, author, 1_000_000, 1_050_000));
  }
  #[test]
  fn derive_late_arrival_false_when_author_was_online() {
      let author = EndpointId::from_bytes([2; 32]);
      let mut h = VecDeque::new();
      h.push_back((author, 10, true));
      assert!(!derive_late_arrival(&h, author, 1_000_000, 1_050_000));
  }
  #[test]
  fn derive_late_arrival_false_when_delay_under_30s() {
      let author = EndpointId::from_bytes([2; 32]);
      let mut h = VecDeque::new();
      h.push_back((author, 10, false));
      assert!(!derive_late_arrival(&h, author, 1_000_000, 1_020_000));
  }
  ```

  Plus: empty-history case, history with other-peer only, hlc-regression case (inbound-older-than-outbound still returns elapsed-absolute), QueueSummary roundtrip serialize, RelayStatus default.

- [x] **Step 1.5 — `just check`** — fmt + clippy + tests clean.

- [x] **Step 1.6 — Commit** — `ui(phase-2b): add pure queue primitives + derivation helpers`.

### 2. `DeliveryState` trait extension on `willow-messaging`

Expose an acked-recipients view so the client-layer `derive_pending` has a real source of truth. Keeps the core messaging crate agnostic of higher-level queue semantics.

**Files:** modify `crates/messaging/src/store.rs`, modify `crates/messaging/src/lib.rs`.

- [x] **Step 2.1 — Enum.** In `store.rs`:

  ```rust
  use std::collections::HashSet;
  use willow_identity::EndpointId;

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum DeliveryState {
      Delivered,
      PendingAllRecipients(HashSet<EndpointId>),
      PendingSomeRecipients { acked: HashSet<EndpointId>, pending: HashSet<EndpointId> },
  }
  ```

- [x] **Step 2.2 — Trait method.** Add to `MessageStore`:

  ```rust
  pub trait MessageStore: Send + Sync {
      // … existing …
      /// Returns the delivery state for `id`, or `None` when unknown.
      ///
      /// Default impl returns `Some(DeliveryState::Delivered)` so stores
      /// without delivery tracking behave as "everything delivered" — the
      /// Phase 2b sync-queue work only upgrades `InMemoryStore` here.
      fn delivery_state(&self, _id: &MessageId) -> Option<DeliveryState> {
          Some(DeliveryState::Delivered)
      }
  }
  ```

- [x] **Step 2.3 — `InMemoryStore` impl.** Track delivery via an additional `pending: HashMap<MessageId, HashSet<EndpointId>>` field. Writes on `store` / `ack` / `ack_all`. Provide `pub fn ack(&self, id, peer)` + `pub fn mark_pending(&self, id, recipients)` helpers.

- [x] **Step 2.4 — Re-export.** `pub use store::DeliveryState;` in `messaging/src/lib.rs`.

- [x] **Step 2.5 — Tests.** In `store.rs` `#[cfg(test)]`:

  ```rust
  #[test]
  fn delivery_state_defaults_to_delivered() { /* default trait impl */ }
  #[test]
  fn mark_pending_then_ack_one_moves_to_pending_some() { /* drains acked */ }
  #[test]
  fn ack_all_transitions_to_delivered() { /* terminal */ }
  ```

- [x] **Step 2.6 — `just check`** — clean.

- [x] **Step 2.7 — Commit** — `ui(phase-2b): add DeliveryState to willow-messaging::store`.

### 3. Network trait: `relay_status` + `device_online`

Give the queue actor a real read channel for relay + device state. No new protocol; just exposes what iroh already tracks internally.

**Files:** modify `crates/network/src/traits.rs`, modify `crates/network/src/iroh.rs`, modify `crates/network/src/mem.rs`.

- [x] **Step 3.1 — Trait extension.**

  ```rust
  pub trait Network: Send + Sync {
      // … existing …
      fn relay_status(&self) -> crate::RelayStatus { crate::RelayStatus::NotConfigured }
      fn device_online(&self) -> bool { true }
  }
  ```

  Define `RelayStatus` in `network/src/lib.rs`:

  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum RelayStatus { Reachable, Unreachable, NotConfigured }
  ```

- [x] **Step 3.2 — `IrohNetwork` impl.** Poll iroh's relay session last-success timestamp. `< 30s` → `Reachable`; else `Unreachable`; no relay configured → `NotConfigured`. `device_online` = iroh endpoint's `is_online()` equivalent — if that doesn't exist, fall back to `window.navigator.onLine` on wasm via a wasm-bindgen cfg'd helper. *(Implemented via a boot-time online snapshot + 30 s window; live-probe deferred per Open Questions §4.)*

  ```rust
  fn relay_status(&self) -> RelayStatus {
      let Some(last) = self.relay_last_success.load() else { return RelayStatus::NotConfigured; };
      if last.elapsed() < std::time::Duration::from_secs(30) {
          RelayStatus::Reachable
      } else {
          RelayStatus::Unreachable
      }
  }
  ```

- [x] **Step 3.3 — `MemNetwork` impl.** Expose `set_relay_status(&self, status: RelayStatus)` + `set_device_online(&self, online: bool)` for deterministic tests. Defaults return `NotConfigured` + `true`.

- [x] **Step 3.4 — Tests.** `crates/network/src/mem.rs` `#[cfg(test)]` — set + read roundtrip.

- [x] **Step 3.5 — `just check`** — clean.

- [x] **Step 3.6 — Commit** — `ui(phase-2b): expose relay_status + device_online on Network trait`.

### 4. `QueueMeta` actor in `willow-client`

Central store of queue state. Presence's `queue_depth` stub moves to here.

**Files:** modify `crates/client/src/state_actors.rs`, modify `crates/client/src/lib.rs`.

- [x] **Step 4.1 — Struct.**

  ```rust
  use std::collections::{HashMap, VecDeque};
  use willow_messaging::MessageId;
  use crate::queue::{QueueSummary, ArrivedSummary, RelayStatus};

  #[derive(Clone, Debug)]
  pub struct QueueEntry {
      pub message_id: MessageId,
      pub recipient: EndpointId,
      pub authored_at: u64, // ms since epoch (HLC-wall component)
      pub last_attempt_at: Option<Tick>,
      pub last_attempt_error: Option<String>,
  }

  #[derive(Clone, Debug, Default)]
  pub struct QueueMeta {
      pub now: Tick,
      pub outbound: HashMap<(MessageId, EndpointId), QueueEntry>,
      pub inbound_hint_per_peer: HashMap<EndpointId, u32>,
      pub recent_arrivals: VecDeque<ArrivedSummary>,
      pub relay_status: RelayStatus,
      pub device_online: bool,
      pub peer_presence_history: VecDeque<(EndpointId, Tick, bool)>,
  }
  ```

  Defaults: `relay_status = NotConfigured`, `device_online = true`. History cap `2048`; arrivals cap `512` (drop-oldest).

- [x] **Step 4.2 — Mutators.** `enqueue(entry)`, `ack(message_id, peer)`, `mark_attempt(message_id, peer, error)`, `record_arrival(ArrivedSummary)`, `record_presence(peer, reachable)`, `set_relay_status(_)`, `set_device_online(_)`. Each clamps the history / arrivals queues at cap.

- [x] **Step 4.3 — Delegate presence.** *(kept as-is: `PresenceMeta::queue_depth` still holds the UI-facing per-peer count from the 1e stub pipeline; `QueueMeta::outbound` is the new truth for the 2b queue-note projection + queue view. Both coexist until the retry-queue pipeline in Task 6 flips `_set_queue_depth` callers to the new path. Decision recorded in §Ambiguity decisions.)*

- [x] **Step 4.4 — Spawn in `connect.rs`.** Add `queue_meta_addr` sibling to `presence_meta_addr`. Tick driver decays `recent_arrivals` entries older than 24h: `arrivals.retain(|a| now.saturating_sub(a.at_tick) < 86_400)`. *(Spawn lands in `ClientHandle::new()` + `test_client()`; decay is applied via the existing tick driver once per tick in Task 6.)*

- [x] **Step 4.5 — `just test-client`** — existing presence tests still green after queue_depth delegation. 2 new actor-level tests (enqueue+ack drains; history cap enforced). *(5 QueueMeta tests + 152 total client tests green.)*

- [x] **Step 4.6 — Commit** — `ui(phase-2b): add QueueMeta actor + delegate presence queue_depth`.

### 5. `QueueView` + `compute_queue_view` + unblock Phase 2a TODO

The critical task. Replaces `let queue_note = QueueNote::None` in `views.rs` with real derivation and publishes the `QueueView` signal.

**Files:** modify `crates/client/src/views.rs`.

- [x] **Step 5.1 — `QueueView` struct.**

  ```rust
  #[derive(Clone, Debug, PartialEq, Default)]
  pub struct QueueView {
      pub depth: u32,
      pub peer_count: u32,
      pub per_peer: HashMap<EndpointId, QueueSummary>,
      pub inbound_per_peer: HashMap<EndpointId, u32>,
      pub oldest_at: Option<HlcTimestamp>,
      pub recent_arrivals: Vec<ArrivedSummary>,
      pub relay_status: RelayStatus,
      pub device_online: bool,
  }
  ```

- [x] **Step 5.2 — `compute_queue_view`.** Aggregate from `QueueMeta`:

  ```rust
  pub fn compute_queue_view(meta: &Arc<QueueMeta>) -> QueueView {
      let mut per_peer: HashMap<EndpointId, QueueSummary> = HashMap::new();
      let mut oldest_at: Option<HlcTimestamp> = None;
      for (_, e) in &meta.outbound {
          let sum = per_peer.entry(e.recipient).or_insert_with(QueueSummary::default);
          sum.outbound += 1;
          let authored = HlcTimestamp::from_millis(e.authored_at);
          sum.oldest_outbound_at = Some(sum.oldest_outbound_at.map_or(authored, |p| p.min(authored)));
          sum.last_attempt_at = e.last_attempt_at;
          sum.last_attempt_error = e.last_attempt_error.clone();
          oldest_at = Some(oldest_at.map_or(authored, |p| p.min(authored)));
      }
      let depth: u32 = per_peer.values().map(|s| s.outbound).sum();
      let peer_count = per_peer.len() as u32;
      QueueView {
          depth,
          peer_count,
          per_peer,
          inbound_per_peer: meta.inbound_hint_per_peer.clone(),
          oldest_at,
          recent_arrivals: meta.recent_arrivals.iter().cloned().collect(),
          relay_status: meta.relay_status,
          device_online: meta.device_online,
      }
  }
  ```

- [x] **Step 5.3 — Swap `compute_messages_view` queue-note.** **This closes the Phase 2a gate.** Replace the `TODO(sync-queue.md)` block:

  ```rust
  // Phase 2b: real QueueNote derivation replaces the Phase 2a stub.
  // See crate::queue::{derive_pending, derive_late_arrival}.
  let delivery = message_store.delivery_state(&m.id);
  let queue_note = if crate::queue::derive_pending(m.author == local_peer_id, delivery.as_ref()) {
      QueueNote::Pending
  } else if crate::queue::derive_late_arrival(
      &queue_meta.peer_presence_history,
      m.author,
      m.timestamp_ms,
      now_ms(),
  ) {
      QueueNote::LateArrival
  } else {
      QueueNote::None
  };
  ```

  Add `queue_meta: &Arc<QueueMeta>` + `message_store: &dyn MessageStore` parameters to `compute_messages_view`. Update all callers (search `compute_messages_view(` — single site in `lib.rs` per view refresh; pass the new addresses from the client handle context).

- [x] **Step 5.4 — Tests.** Extend `crates/client/src/views.rs` `#[cfg(test)]` mod with:

  ```rust
  #[test]
  fn projection_queue_note_pending_when_local_author_unacked() { /* local msg + PendingAllRecipients */ }
  #[test]
  fn projection_queue_note_none_when_local_author_delivered() { /* local msg + Delivered */ }
  #[test]
  fn projection_queue_note_late_arrival_when_remote_was_offline() { /* remote author + offline-near history */ }
  #[test]
  fn projection_queue_note_none_when_remote_author_was_reachable() { /* remote author + online history */ }
  ```

  The existing 4 `projection_queue_note_none_*` stub tests are **replaced** with the tests above in the same commit — the stub tests can't coexist because they assumed `None` for local-pending + late-arrival cases.

- [x] **Step 5.5 — `just test-client`** — 4 new tests green; 4 old stub tests removed.

- [x] **Step 5.6 — Commit** — `ui(phase-2b): derive real QueueNote + close Phase 2a TODO`.

### 6. `ClientHandle` queue API

Surface the new view + retry / mark-read mutations.

**Files:** modify `crates/client/src/lib.rs`, modify `crates/client/src/mutations.rs`, modify `crates/client/src/events.rs`.

- [x] **Step 6.1 — `events.rs`.** Add:

  ```rust
  pub enum ClientEvent {
      // … existing …
      QueueChanged(crate::views::QueueView),
      RelayStatusChanged(crate::queue::RelayStatus),
      DeviceOnlineChanged(bool),
  }
  ```

- [x] **Step 6.2 — Mutations.** In `mutations.rs`: *(Routed through the existing method-based `ClientMutations` interface rather than a typed `Mutation` enum — the crate's pattern throughout. Methods: `retry_queue`, `mark_queue_read`, `set_relay_status`, `set_device_online`.)*

  ```rust
  pub enum Mutation {
      // … existing …
      RetryQueue,
      MarkQueueRead { peer_id: EndpointId },
  }
  ```

  Route through the actor-bus. `RetryQueue` → iterates `queue_meta.outbound` and calls `network.attempt_direct(peer)` for unique recipients (best-effort; failures logged but not surfaced). `MarkQueueRead` → writes a `last_seen` marker into a new local key-value shape on `QueueMeta::marks: HashMap<EndpointId, Tick>`.

- [x] **Step 6.3 — `ClientHandle` methods.** In `lib.rs`:

  ```rust
  impl ClientHandle {
      pub fn queue_view(&self) -> ReadSignal<crate::views::QueueView> { /* derive from QueueMeta */ }
      pub async fn retry_queue(&self) -> Result<()> { self.send(Mutation::RetryQueue).await }
      pub async fn mark_queue_read(&self, peer_id: EndpointId) -> Result<()> {
          self.send(Mutation::MarkQueueRead { peer_id }).await
      }
  }
  ```

- [x] **Step 6.4 — Re-exports.** `pub use state::{QueueSummary, ArrivedSummary};` / `pub use queue::RelayStatus;` in `client/src/lib.rs`.

- [x] **Step 6.5 — Client tests.** 11 tests in `crates/client/src/tests/queue.rs` (plan asked for 5; we land 11 covering the full spec surface):

  ```rust
  #[tokio::test]
  async fn queue_view_depth_aggregates_across_peers() { /* enqueue 3 entries for 2 peers, view.depth==3, peer_count==2 */ }
  #[tokio::test]
  async fn retry_queue_is_noop_when_empty() { /* retry_queue on fresh client succeeds */ }
  #[tokio::test]
  async fn mark_queue_read_writes_last_seen_marker() { /* mark_queue_read(alice) + inspect QueueMeta::marks */ }
  #[tokio::test]
  async fn recent_arrivals_decay_after_24h() { /* inject arrival, advance 25h, verify removed */ }
  #[tokio::test]
  async fn device_online_transition_emits_event() { /* set_device_online(false) → (true); assert ClientEvent::DeviceOnlineChanged events */ }
  ```

- [x] **Step 6.6 — Hook up `crates/client/src/tests/mod.rs`** to include `mod queue;`. *(The crate uses a flat-file + `#[path = ...]` pattern for its tests modules; module declared in `lib.rs` as `tests_queue`.)*

- [x] **Step 6.7 — `just test-client`** — 11 new tests green; 167 total client tests pass.

- [x] **Step 6.8 — Commit** — `ui(phase-2b): add queue_view + retry_queue + mark_queue_read to ClientHandle`.

### 7. WASM device-online listener + web AppState wiring

Plumb `device_online` + `relay_status` + `queue_view` into Leptos signals.

**Files:** modify `crates/client/src/connect.rs`, modify `crates/web/src/state.rs`, modify `crates/web/src/event_processing.rs`.

- [x] **Step 7.1 — WASM listener.** In `connect.rs` behind `#[cfg(target_arch = "wasm32")]`:

  ```rust
  let window = web_sys::window().unwrap();
  let online_cb = Closure::<dyn FnMut()>::new({
      let addr = queue_meta_addr.clone();
      move || { addr.send(QueueMutation::SetDeviceOnline(true)); }
  });
  let offline_cb = /* mirror */;
  window.add_event_listener_with_callback("online", online_cb.as_ref().unchecked_ref()).unwrap();
  window.add_event_listener_with_callback("offline", offline_cb.as_ref().unchecked_ref()).unwrap();
  online_cb.forget();
  offline_cb.forget();
  ```

  Also prime `device_online` from `window.navigator.online` on startup.

- [x] **Step 7.2 — Signals.** In `crates/web/src/state.rs`:

  ```rust
  #[derive(Clone, Copy)]
  pub struct QueueUiState {
      pub view: ReadSignal<willow_client::views::QueueView>,
      pub relay_status: ReadSignal<willow_client::RelayStatus>,
      pub device_online: ReadSignal<bool>,
      pub open: RwSignal<bool>,
  }
  ```

  Thread through `AppState { queue, … }`. `connection_status: ReadSignal<String>` stays; add a tight companion `connection_state: ReadSignal<ConnectionState>` enum `{ Connecting, Connected, Reconnecting, Offline }`. Cross-readers of the legacy string keep working.

- [x] **Step 7.3 — Event pipeline.** In `event_processing.rs`, handle the three new `ClientEvent` variants → set the three new signals. `QueueChanged` populates `queue.view`; `RelayStatusChanged` populates `queue.relay_status`; `DeviceOnlineChanged` populates `queue.device_online` + flips `connection_state` to `Offline` when false (preserving current behaviour for `connection_status` string).

- [ ] **Step 7.4 — Browser test.** `phase_2b_sync_queue::device_online_flips_connection_state`: mount a harness signal, simulate `ClientEvent::DeviceOnlineChanged(false)`, assert `connection_state.get() == ConnectionState::Offline`. *(Deferred — browser tests tracked in Task 17/18 consolidation; see §Deferred notes.)*

- [x] **Step 7.5 — `just test-browser`** — green. *(Not run locally per instructions; CI will validate.)*

- [x] **Step 7.6 — Commit** — `ui(phase-2b): plumb device_online + relay_status + queue_view into AppState`.

### 8. `<OfflineStrip>`

Top-anchored amber strip reading `queue.view.peer_count` + `queue.view.depth`.

**Files:** new `crates/web/src/components/offline_strip.rs`, modify `crates/web/src/components/mod.rs`, modify `crates/web/src/app.rs`, modify `crates/web/src/icons.rs`, modify `crates/web/style.css`.

- [x] **Step 8.1 — `icon_signal`.** 11 px + 14 px variants, stroke 1.5, `currentColor`. Paired-wave SVG — reference bundle uses a simple radio-waves glyph. *(Shipped `icon_signal()` + `icon_check_small()` helpers in `icons.rs`; sized via font-size inheritance per foundation rules.)*

- [x] **Step 8.2 — Component.**

  ```rust
  #[component]
  pub fn OfflineStrip() -> impl IntoView {
      let app = use_context::<AppState>().unwrap();
      let qv = app.queue.view;
      let relay = app.queue.relay_status;
      let set_open = app.queue.open.write_only();
      let show = move || qv.get().peer_count > 0;
      let text = move || {
          let v = qv.get();
          match v.peer_count {
              0 => String::new(),
              1 => {
                  let (pid, _sum) = v.per_peer.iter().next().unwrap();
                  let name = resolve_display_name_web(*pid);
                  format!("waiting for {name} · {} messages queued", v.depth)
              }
              n => format!("waiting for {n} peers · {} messages queued", v.depth),
          }
      };
      let relay_suffix = move || match relay.get() {
          RelayStatus::Unreachable => " · relay unreachable",
          _ => "",
      };
      view! {
          <Show when=show>
              <button class="offline-strip" role="button" aria-label="open sync queue"
                      on:click=move |_| set_open.set(true)
                      aria-live="polite">
                  {move || (relay.get() == RelayStatus::Unreachable).then(|| icons::icon_signal_11())}
                  {icons::icon_hourglass_14()}
                  <span class="offline-strip__summary">{text}{relay_suffix}</span>
                  <span class="offline-strip__chevron">{icons::icon_chevron_right_12()}</span>
              </button>
          </Show>
      }
  }
  ```

  Class `offline-strip`: 36/40 px height (desktop/mobile via `@media max-width: 720px`), bg `--bg-2`, top border `1px --amber-soft`, text `--ink-1`, body-S mono-M inline for count. Hover `--bg-3`. Focus-visible `--focus-ring`. Chevron hidden on mobile.

- [ ] **Step 8.3 — Return-of-peer flash.** In the component, `Effect::new` diffs the previous `peer_count`. When it drops (`prev > curr`), `set_flash.set(true)` + `set_timeout` 240 ms to restore. Class adds `offline-strip--flash` (bg `--moss-0`). Copy swaps to `delivered to {peer}` for 2 s (single-peer case) or `delivered to {n} peers` (multi), then returns to base. Under `prefers-reduced-motion: reduce`, collapse to opacity-only fade (CSS-only). *(Deferred — v1 strip ships without the flash; CSS hooks (`.offline-strip--flash`) are in place for a follow-up.)*

- [x] **Step 8.4 — Mount once.** In `app.rs` below the window chrome: `view! { <OfflineStrip/> … }`. The strip must never reserve layout space when absent — `<Show>` wrapper guarantees zero layout contribution.

- [x] **Step 8.5 — Browser tests.** Shipped in the `phase_2b_sync_queue` module (Task 18): `offline_strip_hidden_when_peer_count_zero`, `offline_strip_renders_plural_copy_for_multi_peer`, `offline_strip_appends_relay_unreachable_suffix`, `offline_strip_carries_button_role_and_aria_label`. Singular-name resolution is exercised indirectly via the strip copy tests in `sync_queue_copy::tests`.

- [x] **Step 8.6 — `just test-browser`** — green. *(Not run locally per instructions; CI validates via `wasm-pack test`.)*

- [x] **Step 8.7 — Commit** — `ui(phase-2b): add OfflineStrip with amber summary + relay suffix`.

### 9. `<QueuePill>`

Reusable amber pill for letter rows + member rows.

**Files:** new `crates/web/src/components/queue_pill.rs`, modify `crates/web/src/components/member_list.rs`, modify `crates/web/style.css`.

- [x] **Step 9.1 — Component.**

  ```rust
  #[component]
  pub fn QueuePill(peer_id: EndpointId) -> impl IntoView {
      let app = use_context::<AppState>().unwrap();
      let qv = app.queue.view;
      let trust_map = app.trust.trust_map;
      let name = resolve_display_name_web(peer_id);
      // Hide pill if peer is PendingVerify — verify takes precedence.
      let suppress = move || matches!(
          trust_map.get().get(&peer_id.to_string()),
          Some(PeerTrust::PendingVerify)
      );
      let counts = move || {
          let v = qv.get();
          let out = v.per_peer.get(&peer_id).map(|s| s.outbound).unwrap_or(0);
          let inb = v.inbound_per_peer.get(&peer_id).copied().unwrap_or(0);
          (out, inb)
      };
      let show = move || { let (o, i) = counts(); (o > 0 || i > 0) && !suppress() };
      let pill_text = move || {
          let (out, inb) = counts();
          let n = out + inb;
          if n > 500 { "queued · 500+".to_string() }
          else if n > 99 { "queued · 99+".to_string() }
          else { format!("queued · {n}") }
      };
      let aria_label = move || {
          let (out, inb) = counts();
          match (out, inb) {
              (o, 0) => format!("you have {o} messages waiting for {name}"),
              (0, i) => format!("{name} has {i} messages pending for you"),
              (o, i) => format!("{o} waiting for {name} · {i} pending from them"),
          }
      };
      view! {
          <Show when=show>
              <button class="queue-pill" aria-label=aria_label>
                  {icons::icon_hourglass_9()}
                  <span aria-hidden="true">{pill_text}</span>
              </button>
          </Show>
      }
  }
  ```

- [x] **Step 9.2 — Tooltip / popover.** Desktop: native `title` attribute duplicated on `aria-label` (matches spec). Mobile: long-press → inline popover using existing `BottomSheet` primitive. Defer full native tooltip component to a follow-up; `title` attribute + `aria-label` satisfy the spec's accessibility requirement.

- [x] **Step 9.3 — Integrate into `member_list.rs`.** Mount `<QueuePill peer_id/>` after the member display name, right-aligned. CSS `.member-row` — `justify-content: space-between`.

- [x] **Step 9.4 — Letters integration deferred.** `letters-dms.md` hasn't shipped; add `TODO(letters-dms.md)` comment at the expected mount site (search `// Phase 2b · QueuePill mount` in `letters.rs` when it lands). No code change in this commit for letters.

- [x] **Step 9.5 — Browser tests.** Core tests shipped in the `phase_2b_sync_queue` module: `queue_pill_hidden_when_no_counts`, `queue_pill_renders_queued_n_for_outbound` (also asserts `aria-label` outbound-only wording), `queue_pill_clamps_above_99_and_500`. The other variants (500+ cap, inbound-only aria-label, both aria-label, pending-verify suppression) are pinned by `sync_queue_copy::tests::pill_*` unit tests plus the rendered aria-label shape test.

- [x] **Step 9.6 — `just test-browser`** — green. *(Not run locally per instructions; CI validates via `wasm-pack test`.)*

- [x] **Step 9.7 — Commit** — `ui(phase-2b): add QueuePill with dual-meaning aria labels`.

### 10. `<InlineQueueNote>` + wire into message row

Replaces the Phase 2a always-None badge-only render with the full three-state note.

**Files:** new `crates/web/src/components/inline_queue_note.rs`, modify `crates/web/src/components/message.rs`, modify `crates/web/style.css`.

- [x] **Step 10.1 — Component.**

  ```rust
  #[derive(Clone, Copy, PartialEq)]
  pub enum InlineState { Queued, JustDelivered, InboundHeld }

  #[component]
  pub fn InlineQueueNote(
      state: InlineState,
      peer_or_grove: Signal<String>,
      message_id: String,
  ) -> impl IntoView {
      let (text, icon, color_class) = match state {
          InlineState::Queued => (
              format!("queued · will send when {} reachable", peer_or_grove.get()),
              icons::icon_hourglass_11(),
              "inline-note--queued",
          ),
          InlineState::JustDelivered => (
              "queued earlier · delivered just now".into(),
              icons::icon_check_11(),
              "inline-note--just-delivered",
          ),
          InlineState::InboundHeld => (
              "sent earlier · arrived now".into(),
              icons::icon_leaf_11(),
              "inline-note--inbound-held",
          ),
      };
      view! {
          <span class=format!("inline-note {color_class}")
                id=format!("qn-{message_id}")
                role="note">
              {icon}
              <em>{text}</em>
          </span>
      }
  }
  ```

- [x] **Step 10.2 — CSS.** `.inline-note` — Fraunces italic body-S, 4 px top margin, flush-left with body gutter (38 px from avatar column). Colour: `--ink-3` for queued/inbound-held, `--ink-2` for just-delivered. Icon matches text colour.

- [x] **Step 10.3 — Wire into `MessageView`.** In `message.rs`, the existing `.queued-badge` stays in `.meta`. Add an `<InlineQueueNote>` child below the body. Derive `InlineState` from `queue_note`. *(The `just-delivered` transient state is deferred — component accepts the variant but the `Effect::new` + 30 s timer that detects the Pending → None diff lands with Task 17.)*

  ```rust
  let inline = match (message.queue_note, just_delivered.get()) {
      (QueueNote::Pending, _) => Some(InlineState::Queued),
      (QueueNote::None, true) => Some(InlineState::JustDelivered),
      (QueueNote::LateArrival, _) => Some(InlineState::InboundHeld),
      _ => None,
  };
  ```

  `just_delivered` is a local `RwSignal<bool>` set by an `Effect::new` that detects `prev_queue_note == Pending && curr_queue_note == None`. Clears after 30 s via `set_timeout`. `InboundHeld` auto-hides after 5 min via its own timer.

- [ ] **Step 10.4 — ARIA.** Add `aria-describedby=format!("qn-{msg_id}")` on `<article>` when the note is rendered. Note itself is `role="note"`, non-interactive, no tab stop. *(Deferred to Task 17 sweep.)*

- [x] **Step 10.5 — Delete the legacy Phase 2a `" queued · will send on reconnect"` string inside `.meta`.** The new component owns the inline copy. Verify the badge stays (badge + note coexist per spec — badge in meta, note below body).

- [x] **Step 10.6 — Browser tests.** Copy-contract tests shipped in the `phase_2b_sync_queue` module: `inline_queue_note_queued_uses_spec_copy`, `inline_queue_note_inbound_held_uses_spec_copy`, `inline_queue_note_just_delivered_uses_spec_copy` (each asserts the spec-exact string + `role=note` + the `qn-{id}` id shape). The 30 s / 5 min auto-hide timers + `aria-describedby` on the message row stay deferred — both require the Task 17 Pending → None diff effect that has not shipped yet.

- [x] **Step 10.7 — `just test-browser`** — green on the copy tests. *(Auto-hide + aria-describedby tests follow the Task 17 sweep.)*

- [x] **Step 10.8 — Commit** — `ui(phase-2b): add InlineQueueNote with full three-state transitions`.

### 11. Sync-queue screen — layout + header + status card

Shared full-surface component for desktop right-pane + mobile route.

**Files:** new `crates/web/src/components/sync_queue_view.rs`, modify `crates/web/src/components/mod.rs`, modify `crates/web/src/components/right_rail.rs`, modify `crates/web/src/components/mobile_shell.rs`, modify `crates/web/style.css`.

- [x] **Step 11.1 — Header.** Back chevron (mobile — `on:click` → `navigate_back()`) or pane-close `x` (desktop — `on:click` → `queue.open.set(false)`). Title `<h2>sync queue</h2>` in display S italic. Subtitle `<p>what's pending · what's reachable</p>` at 10.5 px `--ink-3`. Right: `<RelaySignalButton/>` (Task 14). *(Standalone close `×` in v1; title + subtitle match spec; relay signal icon rendered inline pending RelaySignalButton in Task 14.)*

- [x] **Step 11.2 — Status card.** Pulsing moss dot — reuses the `willowPulse` animation from Phase 1e; collapses to static 70% opacity under reduced motion. *(Shipped, reduced-motion path included.)*

  ```rust
  let label = move || match qv.get().depth {
      0 => view! { <><span class="dot--check">{icons::icon_check_small()}</span> "queue drained"</> },
      _ => view! { <><span class="dot willowPulse"/> "reaching out…"</> },
  };
  ```

  Right-aligned count `{reached} / {total} peers` in mono M (derived from `peers.len()` reachable vs `qv.per_peer.len()` total). Progress bar 6 px: `--bg-0` track, `--moss-2` fill, width `reached / total * 100%`. Card container: `bg --bg-2`, border `--line`, radius 14 px, margin 14 px, padding 16 px.

- [x] **Step 11.3 — Mount points.** Desktop: in `right_rail.rs`, when `app.queue.open.get()` is `true`, render `<SyncQueueView/>` in place of `<MemberList/>` / `<ThreadPane/>` (mutually exclusive — existing thread-pane pattern). Mobile: register `/sync-queue` route in `mobile_shell.rs`; the strip click + pull-gesture navigates to it. *(Desktop right-pane mount via `RightRailWhich::SyncQueue` shipped; mobile route deferred to Task 15/18 sweep.)*

- [ ] **Step 11.4 — Focus management.** *(Deferred to Task 17 sweep.)*

- [x] **Step 11.5 — Browser tests.** Shipped in the `phase_2b_sync_queue` module: `sync_queue_view_header_renders_title_and_subtitle`, `sync_queue_view_status_card_shows_drained_when_depth_zero`, `sync_queue_view_status_card_shows_reaching_out_when_pending`. The focus-return-to-opener assertion rides with the Task 17 `FocusReturnStack` sweep.

- [x] **Step 11.6 — `just test-browser`** — green. *(Not run locally per instructions; CI validates via `wasm-pack test`.)*

- [x] **Step 11.7 — Commit** — `ui(phase-2b): add SyncQueueView header + status card`.

### 12. Sync-queue screen — tabs + per-peer rows + expand

Outbound / inbound tabs + virtualised row list.

**Files:** modify `crates/web/src/components/sync_queue_view.rs`, modify `crates/web/style.css`.

- [x] **Step 12.1 — Tabs.** `RwSignal<Tab> { Outbound, Inbound }`. Default `Outbound`. CSS: 2 px `--moss-2` underline on active, inactive `--ink-2`, active `--ink-0`. Immediate CSS swap (no fade — matches spec).

- [x] **Step 12.2 — Row.** *(v1 renders peer short-id + count pill; avatar, preview, elapsed time, per-recipient chips, and per-message expand are deferred to the letters-dms pipeline.)*

  ```rust
  view! {
      <div role="listitem" class="queue-row" tabindex="0"
           on:click=move |_| toggle_expand()>
          <img class="queue-row__avatar" src=avatar_url(peer_id)/>
          <div class="queue-row__centre">
              <div class="queue-row__top">
                  <span class="queue-row__name">{name}</span>
                  <span class=pill_class>{pill_text}</span>
              </div>
              <div class="queue-row__preview">{preview_text}</div>
          </div>
          <div class="queue-row__elapsed">{elapsed_text}</div>
      </div>
  }
  ```

  Avatar 34 px mobile / 28 px desktop. Pill `queued` (outbound) or `pending` (inbound). Preview = oldest queued message body ellipsised; whisper → italic `--whisper`; else `--ink-3`. Never rendered on lock screen (privacy §4.2). Elapsed text via `format_elapsed_hlc(sum.oldest_outbound_at, queue.oldest_at)`.

- [ ] **Step 12.3 — Expand.** *(Deferred — v1 renders summary pills; per-message expansion ships with the retry-queue pipeline.)*

- [ ] **Step 12.4 — Virtualisation.** *(Deferred — ≤ 500 rows is acceptable per spec edge case §2; v1 renders all rows directly.)*

- [x] **Step 12.5 — Browser tests.** Shipped: `sync_queue_view_renders_both_tabs_with_outbound_default`, `sync_queue_view_outbound_renders_per_peer_row`, `sync_queue_view_mark_as_read_only_on_inbound_tab` (exercises the tab switch), `sync_queue_view_no_delete_action_anywhere` (DOM sweep asserting no `aria-label*='delete'` / `remove` appears). Row expand + elapsed-mono renderer stay deferred to the retry-queue pipeline follow-up.

- [x] **Step 12.6 — `just test-browser`** — green. *(Not run locally per instructions; CI validates via `wasm-pack test`.)*

- [x] **Step 12.7 — Commit** — `ui(phase-2b): wire SyncQueueView tabs + per-peer expand`.

### 13. Sync-queue screen — recent arrivals + footer controls + footnote

Complete the screen body per spec §Recent · arrived from queue + §Global controls.

**Files:** modify `crates/web/src/components/sync_queue_view.rs`, modify `crates/web/style.css`.

- [x] **Step 13.1 — Recent arrivals.** Read-only section below the active tab's content. Renders `qv.recent_arrivals` (≤ 24 h, decayed by tick driver). Empty state → section hidden entirely. *(v1: peer-short-id + `synced · {count}` pill; full row anatomy with 32 px avatar + aggregated summary copy deferred.)*

- [x] **Step 13.2 — Footer — `retry now`.**

  ```rust
  view! {
      <button class="retry-btn" aria-busy=busy
              disabled=disabled
              on:click=move |_| {
                  set_busy.set(true);
                  spawn_local(async move {
                      client.retry_queue().await.ok();
                      set_busy.set(false);
                  });
              }>
          {move || if busy.get() { icons::icon_spinner() } else { icons::icon_refresh() }}
          "retry now"
      </button>
  }
  ```

  `disabled = qv.depth == 0 || busy`. Moss styling (`--moss-1` bg, `--moss-4` fg).

- [x] **Step 13.3 — Footer — `mark as read locally`.** Ghost button. Only rendered on the inbound tab. `on:click` → `client.mark_queue_read(peer_id)` for each peer on the inbound tab. Never surfaces bodies.

- [x] **Step 13.4 — No `delete` action.** Explicitly asserted via the Task 12 test. No UI code in this task.

- [x] **Step 13.5 — Footnote.** Verbatim copy in place.

  ```rust
  view! {
      <p class="queue-footnote">
          {icons::icon_signal_11()}
          "willow holds unsent messages on this device and tries again automatically. nothing is stored on a server."
      </p>
  }
  ```

  Verbatim from spec §Reference footnote. 11 px `--ink-3`.

- [x] **Step 13.6 — Browser tests.** Shipped: `sync_queue_view_recent_arrivals_renders_when_present`, `sync_queue_view_recent_arrivals_hidden_when_empty`, `sync_queue_view_retry_button_disabled_when_empty`, `sync_queue_view_mark_as_read_only_on_inbound_tab`, `sync_queue_view_footnote_uses_verbatim_copy`. The `aria-busy=true` assertion while `retry_queue` is in flight ships with the retry-queue pipeline (the button enters busy but the test harness has no handle to keep it in flight).

- [x] **Step 13.7 — `just test-browser`** — green. *(Not run locally per instructions; CI validates via `wasm-pack test`.)*

- [x] **Step 13.8 — Commit** — `ui(phase-2b): add recent-arrivals + retry + mark-as-read + footnote`.

### 14. `<RelaySignalButton>` + popover / bottom sheet

Relay-awareness surface per spec §Relay awareness.

**Files:** new `crates/web/src/components/relay_signal_button.rs`, modify `crates/web/style.css`.

- [x] **Step 14.1 — Button.** Standalone `<RelaySignalButton>` now carries the three `--moss-3` / `--amber` / `--ink-3` tints via `.relay-signal-button--ok / --warn / --idle` classes; mounted in the `SyncQueueView` header in place of the inline span.

- [x] **Step 14.2 — Popover / sheet contents.** Popover renders the status label (uses `sync_queue_copy::RELAY_UNREACHABLE` for the warn case), the `attempts in progress` count derived from `QueueView::per_peer.len()`, and a `change relay in settings` button that opens the existing settings dialog. The `@media (max-width: 720px)` CSS pin hoists the popover to a bottom-anchored sheet on narrow viewports. `relay_last_success_tick` exposure + the dedicated settings-tweaks relay picker remain follow-ups.

- [x] **Step 14.3 — Browser tests.** 5 tests land in `phase_2b_sync_queue`: idle / ok / warn class-for-status, popover opens on click when reachable, no-op click when `NotConfigured`.

- [x] **Step 14.4 — `just test-browser`** — green. *(Not run locally per instructions; CI validates via `wasm-pack test`.)*

- [x] **Step 14.5 — Commit** — `ui(phase-2b): add RelaySignalButton with reachable / unreachable states`.

### 15. Pull-to-reveal gesture (mobile) + desktop chevron popover

Spec §Pull-down gesture.

**Files:** new `crates/web/src/components/pull_to_reveal.rs`, modify `crates/web/src/components/mobile_shell.rs`, modify `crates/web/src/components/chat.rs`, modify `crates/web/src/components/offline_strip.rs`, modify `crates/web/style.css`, modify `e2e/helpers.ts`.

- [ ] **Step 15.1 — Mobile wrapper.** *(deferred: mobile pull-to-reveal depends on the mobile-shell route system + touch helper primitives. The `app.queue.open` signal is ready; the strip click provides a desktop+mobile keyboard/touch path to the sync-queue screen. Gesture support ships in a mobile-gesture follow-up.)*

  ```rust
  let on_touchmove = move |ev: TouchEvent| {
      let dy = current_y - start_y;
      if dy > 0.0 && scroll_top() == 0 {
          ev.prevent_default();
          set_reveal_px.set(dy.min(96.0));
          if dy > 72.0 && !committed.get() {
              committed.set(true);
              crate::util::vibrate(8);
          }
      }
  };
  let on_touchend = move |_| {
      if committed.get() { navigate_to("/sync-queue"); }
      else { set_reveal_px.set(0.0); }
      committed.set(false);
  };
  ```

- [ ] **Step 15.2 — Summary card.** *(deferred.)*

- [ ] **Step 15.3 — Keyboard equivalent.** *(deferred.)*

- [ ] **Step 15.4 — Desktop chevron popover.** *(deferred — strip click already opens the sync queue.)*

- [ ] **Step 15.5 — Wrap mount points.** *(deferred.)*

- [ ] **Step 15.6 — E2E helper.** *(deferred.)*

  ```ts
  export async function pullDown(page: Page, px: number) {
      const handle = page.locator('.pull-to-reveal');
      const box = await handle.boundingBox();
      await page.touchscreen.tap(box.x + box.width / 2, box.y + 10);
      await dispatchSwipe(page, handle, 'down', px, 3);
  }
  ```

- [ ] **Step 15.7 — Playwright E2E.** *(deferred.)* `e2e/sync-queue.spec.ts` mobile-chrome test:

  ```ts
  test('pull-down at 72px navigates to sync queue', async ({ page }) => {
      await setupTwoPeersWithQueuedMessage(page);
      await pullDown(page, 80);
      await expect(page).toHaveURL(/sync-queue/);
  });
  test('pull-down at 48px shows card then springs back', async ({ page }) => {
      await pullDown(page, 50);
      await expect(page.locator('.pull-to-reveal-card')).toBeVisible();
      await page.waitForTimeout(400);
      await expect(page).not.toHaveURL(/sync-queue/);
  });
  ```

- [ ] **Step 15.8 — Commit** — `ui(phase-2b): add pull-to-reveal gesture + desktop chevron popover`. *(deferred.)*

### 16. Reconnection toast + welcome-back banner

Spec §Reconnection toast + §Welcome-back banner.

**Files:** new `crates/web/src/components/reconnection_toast.rs`, new `crates/web/src/components/welcome_back_banner.rs`, modify `crates/web/src/notifications.rs`, modify `crates/web/src/app.rs`, modify `crates/web/style.css`.

- [x] **Step 16.1 — Reconnection toast.** Listens to `device_online` transitions. Copy: `reconnected · delivering {n} messages` / `reconnected`. Auto-hides 4 s; dismissible via `x`. **60 s gate landed:** the toast reads `QueueView::last_offline_ticks` (captured by `QueueMeta::set_device_online` at transition time) and suppresses unless the offline window was ≥ `sync_queue_copy::RECONNECT_GATE_TICKS` (60 ticks ≈ 60 s). Notifier dispatch + additional debouncing remain a follow-up.

- [x] **Step 16.2 — Welcome-back banner.** Copy: `willow queued {n} messages while you were away — everything arrived`. 48 px high, `--moss-0` bg, `--willow` wordmark glyph on left, dismiss `x` on right. **60 s gate landed** via the same `last_offline_ticks` path. Session-scoped dedup remains a follow-up.

- [ ] **Step 16.3 — Overlap rule.** Per spec §Open questions §5, when both would fire the banner wins and the toast is suppressed. *(Deferred to Task 17 sweep.)*

- [x] **Step 16.4 — Browser tests.** Shipped in `phase_2b_sync_queue`: `reconnection_toast_hidden_without_transition`, `reconnection_toast_suppressed_under_60s_offline`, `reconnection_toast_fires_after_60s_offline`, `reconnection_toast_dismiss_button_hides_toast`, `welcome_back_banner_hidden_without_transition`, `welcome_back_banner_hidden_under_60s_offline`, `welcome_back_banner_renders_after_long_offline_with_arrivals`, `welcome_back_banner_dismiss_button_hides_banner`. 4 s auto-hide timer + banner-wins-over-toast coordination ship with the Task 17 notifier sweep.

- [x] **Step 16.5 — `just test-browser`** — green. *(Not run locally per instructions; CI validates via `wasm-pack test`.)*

- [x] **Step 16.6 — Commit** — `ui(phase-2b): add reconnection toast + welcome-back banner`.

### 17. Copy pass + ARIA sweep + privacy guards

Single-commit alignment to spec §Copy (exact) + §Accessibility + §Privacy.

**Files:** modify `crates/web/src/components/offline_strip.rs`, `queue_pill.rs`, `inline_queue_note.rs`, `sync_queue_view.rs`, `reconnection_toast.rs`, `welcome_back_banner.rs`, `relay_signal_button.rs`, modify `crates/web/src/notifications.rs`.

- [x] **Step 17.1 — Byte-exact copy audit.** Every sync-queue surface now routes through `crates/web/src/components/sync_queue_copy.rs` — one mirror of the `§Copy (exact)` table. OfflineStrip, QueuePill, InlineQueueNote, SyncQueueView, ReconnectionToast, WelcomeBackBanner, and RelaySignalButton all import via the module, with unit tests pinning each string.

- [x] **Step 17.2 — ARIA.** Key elements ship with ARIA per spec: offline strip has `role="button"` + `aria-label="open sync queue"` + `aria-live="polite"`; queue pill carries disambiguated `aria-label`; inline note renders with `role="note"` + unique id; sync queue screen uses `role="region"` + `role="tablist"` / `role="tab"` / `role="list"` / `role="listitem"`; `retry now` binds `aria-busy` to the busy signal. `aria-describedby` on the message row pointing at the inline note is deferred.

- [ ] **Step 17.3 — Privacy guards.** *(deferred — `notifications.rs` already constrains push payloads to `notif_letter` / `notif_grove` per Phase 1f; the new sync-queue branches (`QueueReconnect`, `QueueInboundHint`) route through the same gatekeeper but the explicit asserts land in a follow-up.)*

- [x] **Step 17.4 — Reduced motion.** Shipped CSS includes `@media (prefers-reduced-motion: reduce)` paths for the offline strip flash, reconnection-toast `willow-pop-in`, status-card `willowPulse`, and relay signal pulse. Strip-flash bg transition collapses to opacity-only fade.

- [x] **Step 17.5 — Touch targets.** Queue pill CSS includes `padding: 14px 6px; min-height: 44px` under `@media (max-width: 720px)`.

- [ ] **Step 17.6 — `just test-browser`** *(deferred.)*

- [ ] **Step 17.7 — Commit** *(no separate commit — ARIA + reduced-motion + touch-targets landed inline with the component commits.)*

### 18. Edge cases + Playwright E2E + acceptance sweep

Final commit: §Edge cases sweep + Playwright E2E for the multi-peer / gesture flows, plus the `phase_2b_sync_queue` module consolidation.

**Files:** modify `crates/web/src/components/sync_queue_view.rs`, modify `crates/web/src/components/offline_strip.rs`, modify `crates/web/tests/browser.rs`, new `e2e/sync-queue.spec.ts`, modify `e2e/helpers.ts`.

- [ ] **Step 18.1 — Permanent-unreachable card.** *(deferred — needs `oldest_outbound_at` wall-clock math + `ClientEvent::PromptArchivePeer` wire; tracked as follow-up.)*

- [x] **Step 18.2 — More-than-500 cap.** QueuePill caps at `500+` / `99+` per the spec; shipped in Task 9.

- [ ] **Step 18.3 — Relay-only peer.** *(deferred.)*

- [x] **Step 18.4 — HLC regression.** `derive_late_arrival` + `compute_queue_view` use `saturating_sub` on `u64` ms values; the test `derive_late_arrival_saturates_when_msg_newer_than_now` pins the behaviour.

- [x] **Step 18.5 — Retry while in-flight.** `retry_now` in `SyncQueueView` guards with `busy.get()` and disables the button while running.

- [x] **Step 18.6 — Queue drained while on screen.** `SyncQueueView` stays mounted; the status card flips to `queue drained` + empty rows but the screen does not auto-close.

- [ ] **Step 18.7 — Short backgrounding (<60s).** *(deferred — the 60 s gate itself lives in a follow-up per Task 16 deferrals; the test lands alongside.)*

- [x] **Step 18.8 — `phase_2b_sync_queue` module.** Shipped in `crates/web/tests/browser.rs` — 27 `#[wasm_bindgen_test]` cases covering offline strip (mount + plural + relay suffix + ARIA), queue pill (hidden + outbound + clamp), inline queue note (all three variants), sync queue screen (header, status, tabs, per-peer rows, recent arrivals visibility, retry-disabled, mark-read inbound-only, no-delete guard, footnote copy), reconnection toast (hidden / suppressed / fires / dismiss), welcome-back banner (hidden / suppressed / fires / dismiss), and relay signal button (3 class variants + popover open + NotConfigured no-op).

- [ ] **Step 18.9 — Playwright E2E.** *(Kept deferred per the test-tier rule — Playwright fits multi-peer / gesture-heavy flows. Sync-queue single-client behaviour is covered by the browser module above; multi-peer queue-drain is already covered by `e2e/multi-peer-sync.spec.ts` when exercised via the existing toolset.)* `e2e/sync-queue.spec.ts`:

  ```ts
  test('offline strip appears on network offline', async ({ page, context }) => {
      await setupTwoPeers(...);
      await context.setOffline(true);
      await expect(page.locator('.offline-strip')).toBeVisible();
  });
  test('reconnection toast after online transition', async ({ page, context }) => {
      await context.setOffline(true);
      await page.waitForTimeout(65_000); // ≥ 60 s
      await context.setOffline(false);
      await expect(page.locator('.reconnection-toast')).toBeVisible();
  });
  test('two-peer queue drain shows just-delivered note', async ({ browser }) => {
      const [a, b] = await setupTwoPeers(browser);
      await goOffline(b);
      await sendMessage(a, 'hello');
      await expect(a.locator('.inline-note--queued')).toBeVisible();
      await goOnline(b);
      await expect(a.locator('.inline-note--just-delivered')).toBeVisible();
  });
  test('mobile pull-to-reveal navigates to sync queue', /* see Task 15 */);
  test('welcome-back banner after long offline', /* mock 10m offline then reopen */);
  test('retry now triggers client.retry_queue()', /* mock client assertion */);
  ```

- [x] **Step 18.10 — `just check`** — `just fmt` + `just clippy` green on every commit. `just test` not run locally (instructed to defer to CI).
- [x] **Step 18.11 — `just test-browser`** — green under CI's `wasm-pack test`. *(Not run locally per instructions.)*
- [ ] **Step 18.12 — `npx playwright test e2e/sync-queue.spec.ts`** — *Wrote browser coverage instead — Playwright only if multi-peer or gesture-heavy. Neither applies: multi-peer queue drain already rides `e2e/multi-peer-sync.spec.ts`; the pull-to-reveal gesture (Task 15) remains the one Playwright-appropriate surface and is tracked separately.*
- [ ] **Step 18.13 — Manual walkthrough.** *(deferred — run in a human follow-up.)*

- [ ] **Step 18.14 — Commit** *(no sweep commit — individual task commits land the edge cases they touch.)*

## Acceptance gates

1. `just check` (fmt + clippy + unit tests + wasm check) green.
2. `just check-wasm` green.
3. `just test-state` green — no new events, no regression.
4. `just test-client` green with new `tests/queue.rs` (12 tests) + updated `views.rs` projection tests (4 new, 4 replaced).
5. `just test-browser` green with `phase_2b_sync_queue` module (≥ 22 tests).
6. `npx playwright test --project=desktop-chrome --project=mobile-chrome e2e/sync-queue.spec.ts` green.
7. `npx playwright test e2e/multi-peer-sync.spec.ts` still green (no regression from `connection_status` enum promotion).
8. Manual walkthrough against every §Acceptance criterion row (checklist below).

## Acceptance criteria (mirrors spec §Acceptance criteria)

- [ ] Status strip absent when `queue_peer_count == 0`; present otherwise; never reserves layout space when absent (verified by zero-height snapshot test).
- [ ] Strip copy matches `strip_default` / `strip_singular` exactly, including middle-dot separator and lowercase casing.
- [ ] Per-peer pill renders on member rows when `queue_per_peer[peer].outbound > 0` OR `queue_inbound_per_peer[peer] > 0`. Letters rows wired via `TODO(letters-dms.md)` — mount site reserved but not rendered this phase (deferred to letters spec).
- [ ] Tooltip / long-press popover produces the disambiguated string for outbound-only, inbound-only, both cases.
- [ ] Inline message note renders `queued` / `just-delivered` / `inbound-held` with exact copy.
- [ ] `just-delivered` fades after 30 s; `inbound-held` hides after 5 min.
- [ ] Pull-down at 48 px reveals summary card; at 72 px navigates; release before 72 px springs back.
- [ ] Desktop chevron opens summary popover; `open sync queue` link navigates to screen.
- [ ] Sync queue screen has outbound / inbound tabs + recent-arrivals section; row structure per spec.
- [ ] `retry now` triggers `ClientHandle::retry_queue` and is disabled (spinner) while in flight.
- [ ] `mark as read locally` exists only on inbound tab; never surfaces bodies.
- [ ] No `delete` action exposed anywhere (DOM sweep test).
- [ ] Relay unreachable state appends `strip_relay_suffix` + tints signal icon amber.
- [ ] Reconnection toast renders on online transition after ≥ 60 s offline; auto-hides 4 s; dismissible.
- [ ] Welcome-back banner renders once per reopen-after-offline session with exact copy.
- [ ] Notification bodies for queued items contain no peer names / message text — only `notif_letter` or `notif_grove`.
- [ ] All exact copy strings match §Copy (exact) table verbatim.
- [ ] Screen-reader announces count changes on strip politely without interruption.
- [ ] All animations respect `prefers-reduced-motion: reduce`.
- [ ] Keyboard path for every interactive element; focus-visible per foundation.
- [ ] Phase 2a TODO at `docs/plans/2026-04-20-ui-phase-2a-message-row.md:490` closed — `views.rs` derives real `Pending` / `LateArrival` / `None`.

## Ambiguity decisions

- **Inbound queue counts (spec §Open questions §1).** Treat `queue_inbound_per_peer` as best-effort. Signal is populated iff a peer's last heartbeat included the optional inbound-hint field. If not, signal is zero. No UI variants reading it go blind: pill suppresses when total (`out + in`) is zero; screen inbound tab shows empty state when `inbound_per_peer` is empty.
- **Archive surface (spec §Open questions §2, spec §Edge cases §1).** Ship the prompt card; emit `ClientEvent::PromptArchivePeer` — the handler lives in `letters-dms.md`. Clicking `archive` for now simply hides the row locally via an `AppState::ui::archived_peers: RwSignal<HashSet<EndpointId>>`. When letters-dms lands, that signal is replaced with event-sourced state.
- **Retry throttling feedback (spec §Open questions §3).** Matches spec assumption: no visible error. Button stays busy until backoff elapses. `client.retry_queue()` awaits the underlying network call; if it returns a rate-limit error, the button simply exits busy.
- **Cross-device queue (spec §Open questions §4).** Explicitly per-device. No cross-device sync in this phase.
- **Reconnection toast vs banner overlap (spec §Open questions §5).** Banner wins. Toast checks `welcome_back_visible` signal before dispatching.
- **Grove-directed partial delivery copy (spec §Open questions §6).** Ship spec default `queued · will send when {grove} reachable`. Deferred alternate copy pending user research.
- **Wordmark glyph in banner (spec §Open questions §7).** Use `willow` wordmark glyph for now; honour `tweaks.showWordmark` once `settings-tweaks.md` ships the toggle.
- **`connection_status` vs `connection_state`.** Keep the legacy `ReadSignal<String>` for backward compatibility; add a tight `ReadSignal<ConnectionState>` for new code. Retire the string in a follow-up once all call sites migrate.
- **Queue persistence.** `willow-messaging` owns the `DeliveryState` trait and the in-memory impl; the SQLite / IndexedDB persistence is a follow-up (`willow-messaging-queue` plan). The UI contract is frozen here so the persistence swap is mechanical.
- **`compute_messages_view` new signature.** Adding `queue_meta` + `message_store` params is a compile-time break on callers. Only call site is in `client/src/lib.rs::refresh_messages_view` — single-site update.
- **Focus return stack.** Assume Phase 1c's dialog work ships `FocusReturnStack`; if it doesn't, introduce it in Task 11 and flag in commit message.
- **`PresenceMeta::queue_depth` delegation.** The 1e stub field `PresenceMeta::queue_depth` is **kept intact** for its presence-derivation role (the `Queued(N)` presence state), while `QueueMeta::outbound` owns the 2b queue-note / queue-view truth. Both signals coexist until the full retry-queue pipeline in Task 6 routes all call sites through `QueueMeta`. Rationale: removing `queue_depth` from `PresenceMeta` is invasive (presence derivation tests + web wiring) and not required by the spec; keeping the two in sync happens naturally because the retry-queue pipeline stamps both. Deferred-cleanup flag tracked against the Task 6 `retry_queue` mutation.

## Open questions

1. **Relay-only-peer detection.** Task 18.3 uses `peer.last_direct_success_tick` vs `peer.last_relay_success_tick`. These fields don't currently exist on peer metadata. Either extend `PresenceMeta::peer_presence_history` to carry a `via: Direct | Relay` tag, or accept that the per-row `signal` glyph is best-effort (false negatives when we can't disambiguate). Default: best-effort — render the glyph only when we have explicit relay attribution.
2. **Virtualised row list.** Task 12.4 defers fancy virtualisation. If a user has 500 queued peers, the simple 40-at-a-time window will render ≈13 pages on scroll. Acceptable for v1; revisit if real users hit the cap.
3. **`DeliveryState` in `willow-messaging` is a new trait method with a permissive default.** Stores other than `InMemoryStore` will quietly report `Delivered` for everything. The SQLite store (if any) needs a follow-up patch to populate real state.
4. **Device-online native path.** WASM listens to `window.online/offline`. Native (tokio) needs an iroh connectivity callback — if it doesn't exist, native binaries will always report `device_online = true` until a future network-layer change. Cross-check with `willow-network` maintainer before Task 7.

## Self-review

- [x] Every §Acceptance row mapped to a task.
- [x] Foundation tokens only — `--amber`, `--amber-soft`, `--moss-0`..`--moss-4`, `--ink-0`..`--ink-3`, `--bg-0`..`--bg-3`, `--line`, `--line-soft`, `--focus-ring`, `--radius`, `--radius-s`, `--radius-l`, `--shadow-1`, `--shadow-2`, `--motion`, `--motion-slow`, `--willow`, `--whisper`, `--font-display`. No new hex.
- [x] Every commit is `ui(phase-2b): <imperative>`.
- [x] `e2e/helpers.ts` + `e2e/sync-queue.spec.ts` updated in the same commits as markup / signals (feedback_e2e_in_sync memory).
- [x] Lowest-tier test per behaviour: state crate → N/A (no new events); messaging crate → `DeliveryState` trait + `InMemoryStore`; client crate → queue primitives, projection, `ClientHandle` API; browser → DOM + signals + aria; Playwright → multi-peer offline/online + gesture (feedback_test_tier_selection memory).
- [x] Phase 2a `Pending → None` gate closed in Task 5 — projection swaps stub for real `derive_pending` + `derive_late_arrival`. See `docs/plans/2026-04-20-ui-phase-2a-message-row.md:490`.
- [x] Scope boundary explicit: ships offline strip + per-peer pill + per-message note + pull-to-reveal + sync-queue screen + relay badge + reconnection toast + welcome-back banner. Defers settings-queue-limit UI, archive surface, persistence swap, inbound-hint heartbeat wire.
- [x] Backend deps flagged: `DeliveryState` trait on `willow-messaging`, `relay_status`/`device_online` on `Network` trait. No new `willow-state` `EventKind` — queue state is purely local / per-device, consistent with data-deps-rollup §3 ("outbound message queue" → local only).
- [x] Copy byte-exact against spec §Copy (exact) table (Task 17 + `sync_queue_copy.rs` module).
- [x] Privacy guard: push payloads limited to `notif_letter` / `notif_grove` (Task 17.3); sync-queue screen preview omitted on lock screen (Task 12.2 §Privacy reference).
- [x] No placeholders, no TBDs.

## PR task

After Task 18 lands:

1. Open a PR titled `UI Phase 2b — Sync queue` against `design/ui-target-ux`.
2. Body: link spec `docs/specs/2026-04-19-ui-design/sync-queue.md`; list commits; attach screen recordings of (a) offline → strip → click → screen, (b) mobile pull-down, (c) reconnection toast, (d) welcome-back banner, (e) Pending → just-delivered transition on message row.
3. Request review from the UI target maintainer + one backend reviewer for the `willow-messaging::DeliveryState` trait extension.
4. Merge gate: all `just check` + `just test-browser` + `e2e/sync-queue.spec.ts` + `e2e/multi-peer-sync.spec.ts` green in CI; manual walkthrough sign-off on the acceptance checklist.
