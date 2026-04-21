# Data dependencies rollup

**Date:** 2026-04-19
**Scope:** every spec in `docs/specs/2026-04-19-ui-design/` (UX target set)

Specs read for this rollup (excludes `README.md`, `audit.md`, and this file):

- `call-experience.md`
- `device-handoff.md`
- `discover.md`
- `ephemeral-channels.md`
- `foundation.md` *(no Data dependencies section; declares only tokens)*
- `governance.md`
- `layout-primitives.md`
- `letters-dms.md`
- `message-row.md`
- `composer.md`
- `reactions-pins.md`
- `files-inline.md`
- `onboarding.md`
- `profile-card.md`
- `settings-tweaks.md`
- `sync-queue.md`
- `thread-pane.md`
- `trust-verification.md`
- `whisper-mode.md`

## Purpose

This document is the single-pane-of-glass view of state-layer impact from
the target UX set. Parallel plans risk designing conflicting state
changes — duplicate `EventKind` variants for similar concerns, competing
signal shapes on `AppState`, inconsistent storage keys, or protocols
that clash on the wire. Writing these down in one place before
`writing-plans` runs lets plan authors sequence their work, catch
overlaps (e.g. `SetProfile` extensions proposed by both `profile-card.md`
and `settings-tweaks.md`), and choose shared primitives (e.g. SAS
derivation, per-identity settings doc) before they diverge.

## Executive summary

- **New `willow-state` event kinds:** **13** — `Whisper.Start`,
  `Whisper.Invite`, `Whisper.KeyDerive`, `Whisper.Leave`, `SetRoleColour`,
  `SetRoleDescription`, `DereferenceFile`, `ThreadStart`, `ThreadReply`,
  `ThreadJoin`, `ThreadLeave`, `ThreadArchive`, plus **4 Discover** events
  (`GrovePublish`, `GroveUnpublish`, `InviteRequest`, `InviteRequestCancel`)
  and **2 device events** (`LinkDevice`, `RevokeDevice`). If we count all
  items flagged *new EventKind*: **19**.
- **Events extended:** **3** — `SetProfile` (pronouns, bio, tagline, crest
  pattern, crest colour, pinned, elsewhere), `ChannelCreate` (optional
  `EphemeralConfig`), invite container (optional `role_id`, note) — the
  invite container is a `letters-dms`/`onboarding`-owned artefact, not a
  `willow-state` `EventKind` in the current shape.
- **New signals in `state.rs` (or equivalent client surface):** **~22** —
  including `typing`, `connection_state`, `active_thread_parent_id`,
  `thread_pane_mode`, `thread_unread_by_parent`, `peer_trust`,
  `channel_holders`, `trust_events`, `queue_depth`, `queue_per_peer`,
  `queue_inbound_per_peer`, `queue_peer_count`, `queue_oldest_at`,
  `queue_recent_arrivals`, `relay_status`, `device_online`,
  `call_pinned_peer`, `call_pinned_share`, `call_layout` (extended with
  `Grove`), and nicknames/local trust.
- **New storage shapes:** **~7** — local trust store (IndexedDB / native
  file), Letter record (peer-to-peer, per-device), outbound message queue
  at rest, per-identity settings doc (CRDT-ish, self-gossiped), local
  nickname map, `localStorage` `willow.tweaks` + `willow.call.layout.*`,
  local archive flag on letters, recent-quick-reactions cache.
- **New network protocols:** **6** — Discover topic (dedicated blake3
  topic), device presence signalling (per-identity gossip), device-handoff
  3-message protocol (`HandoffOffer`/`HandoffReady`/`HandoffAck`), per-
  letter sealed-session topic + membership event stream, whisper message
  envelope carrying whisper-id, inbound-queue hint in peer heartbeat
  (open question).
- **New client methods:** **~11** — `send_typing`, `whisper_send`,
  `trust_state`, `mark_verified`, `mark_unverified`, `begin_compare`,
  `open_thread`, `send_thread_reply`, `leave_thread`,
  `thread_participants`, grove publish/unpublish/request APIs.
- **Items flagged "new" with no clear owning spec:** **3**
  - *Peer-identity tombstone signal* — `letters-dms.md` expects a
    "peer no longer exists" signal but delegates to `willow-identity`.
    No spec owns the emission semantics.
  - *Device presence signalling* — `device-handoff.md` declares
    design-only; no spec owns the protocol write path.
  - *Inbound queue hint* — `sync-queue.md` lists
    `queue_inbound_per_peer` and flags an open question on whether it
    piggy-backs on peer heartbeats. No spec owns the heartbeat shape.

## 1. New willow-state events

Status legend: **new** = new `EventKind`; **extend** = new fields on an
existing `EventKind`.

| Event | Kind | Owning spec | Depends on (other events) | Authority required | Serialization concerns |
|---|---|---|---|---|---|
| `Whisper.Start` | new | `whisper-mode.md` §L381 | parent call/letter id; verified peer or target's `allow-unverified-whispers` | member of parent; trust-gate per target | whisper id hash; HLC; ParentHash required |
| `Whisper.Invite` | new | `whisper-mode.md` §L382 | `Whisper.Start` in tail | current whisper participant (must have own `KeyDerive` in tail) | HLC; ParentHash |
| `Whisper.KeyDerive` | new | `whisper-mode.md` §L383 | `Whisper.Invite` or `Whisper.Start` | self (always self-signed) | HLC; ParentHash; self-authorship only |
| `Whisper.Leave` | new | `whisper-mode.md` §L384 | `Whisper.KeyDerive` | self | HLC; dissolves whisper when last |
| `Whisper.Revoke` | new (*open question*) | `whisper-mode.md` §L522 (open question) | `Whisper.KeyDerive` | current whisper participant (proposed) | HLC; signature chain; see §7 |
| `SetRoleColour { role_id, colour }` | new | `governance.md` §L534 | `CreateRole` | `ManageRoles` (implied) | colour = foundation accent token name |
| `SetRoleDescription { role_id, description }` | new | `governance.md` §L536 | `CreateRole` | `ManageRoles` (implied) | cap not specified |
| `DereferenceFile { file_hash }` | new | `governance.md` §L545 | any `Message` with attachment | Owner / Administrator | best-effort; holders purge on next sync |
| `ThreadStart { parent, initial_participants }` | new | `thread-pane.md` §L371 | chat `Message` (parent) | `SendMessages` | emitted on first reply, not on open |
| `ThreadReply { parent, content: SealedContent, reply_to }` | new | `thread-pane.md` §L373 | `ThreadStart`; seals with **thread key**, not channel key | thread participant | `reply_to` scoped to thread |
| `ThreadJoin { parent, peer }` | new | `thread-pane.md` §L375 | `ThreadStart` | Owner / Admin / `ManageChannels` (**confirm during planning**) | HLC |
| `ThreadLeave { parent, peer }` | new | `thread-pane.md` §L377 | `ThreadStart` | self | HLC |
| `ThreadArchive { parent }` | new | `thread-pane.md` §L378 | `ThreadStart` | `ManageChannels` or thread starter | HLC; freezes composer |
| `LinkDevice { device_id, device_name, sub_key_pub }` | new | `settings-tweaks.md` §L479 | identity root key | self (identity owner) | sub-key of root Ed25519 identity |
| `RevokeDevice { device_id }` | new | `settings-tweaks.md` §L480 | `LinkDevice` | self (identity owner) | invalidates sub-key; no future seals to that device |
| `GrovePublish` | new | `discover.md` §L317 | grove exists | owner-signed | rides dedicated Discover topic; newest wins per (grove_id, owner) |
| `GroveUnpublish` | new | `discover.md` §L322 | `GrovePublish` | owner-signed | removes listing on next Discover sync |
| `InviteRequest` | new | `discover.md` §L324 | `GrovePublish` | requester-signed | delivered to the steward set per `governance.md` |
| `InviteRequestCancel` | new | `discover.md` §L327 | `InviteRequest` | requester-signed | field `request_id` |
| `SetProfile` (pronouns, bio, tagline, crest pattern, crest colour, pinned fragment, elsewhere) | extend | `profile-card.md` §L423-L438, `settings-tweaks.md` §L473-L477 | existing `SetProfile` | self | single event carrying optional field per update |
| `ChannelCreate` + `EphemeralConfig { duration_ms, created_at_hlc, custom }` | extend | `ephemeral-channels.md` §L388 | existing `CreateChannel` | `CreateChannel` | absence = regular channel; state may reject duration outside [1 min, 30 d] |
| `ChannelExtend { channel_id, added_ms }` | new | `ephemeral-channels.md` §L404 | ephemeral `ChannelCreate` | `ManageChannels` | at most once per channel; `added_ms ≤ original duration` |
| `ChannelExpiryTick` | new (**open question**) | `ephemeral-channels.md` §L395 | ephemeral channels | none | may be unnecessary if merge frontier HLC + duration suffices — flagged in §7 |
| `ReadMark { letter_id, peer, last_read_message_id }` | new | `letters-dms.md` §L585 | letter exists | self | per-letter opt-in; sent on the letter's own stream, not `willow-state` per se |
| Letter membership events (add / remove / self-leave / rename) | new | `letters-dms.md` §L583 | letter exists | depends on role; must be defined during planning | emitted into the letter's own signed message stream (**not** `ServerState`) |

Two events in the table above (`ReadMark` and letter-membership events)
are scoped to the letter stream, not `willow-state`. They still require
new type definitions in `willow-messaging`; callers should not assume
they show up in `ServerState::events`.

## 2. New / extended signals

Signals surfaced in `crates/web/src/state.rs` (or equivalent read-only
stream on `ClientHandle`). "existing" = signal already exists and is
reused unchanged. "extend" = existing signal gains new variants / new
fields. "new" = signal does not exist yet.

| Signal | Type | Status | Owning UX spec | Consumed by (specs) |
|---|---|---|---|---|
| `connection_status` | `ReadSignal<String>` | extend (add `"offline"` variant) | `sync-queue.md` §L452 | `layout-primitives.md`, `composer.md`, `letters-dms.md` |
| `peers` | `ReadSignal<Vec<(String, String, bool)>>` | existing | `sync-queue.md` §L455 | `layout-primitives.md`, `letters-dms.md`, `profile-card.md` |
| `typing(channel_id)` | `ReadSignal<Vec<PeerId>>` | new | `composer.md` | composer, message list |
| `connection_state()` | `ReadSignal<ConnectionState>` (`Connected`/`Degraded`/`Offline`) | new | `composer.md` | composer placeholder, meta line, offline tinting |
| `active_thread_parent_id` | `RwSignal<Option<String>>` | new | `thread-pane.md` §L346 | thread pane, stub renderer |
| `thread_pane_mode` | `RwSignal<ThreadPaneMode>` (`Closed`/`DesktopRail`/`MobileScreen`) | new | `thread-pane.md` §L348 | layout primitives |
| `thread_unread_by_parent` | `RwSignal<HashMap<String, usize>>` | new | `thread-pane.md` §L351 | thread stub |
| `members_pane_open` | existing (mutually exclusive with thread pane) | existing | `thread-pane.md` §L354 | layout |
| `peer_trust` | `ReadSignal<HashMap<String, PeerTrust>>` | new | `trust-verification.md` §L437 | profile card, letters row, call tile, whisper gate, handoff gate, ephemeral holder chip |
| `channel_holders` | per-channel `ChannelHolders` view | new | `trust-verification.md` §L466 | channel header holder pill |
| `trust_events` | `ReadSignal<Vec<TrustEvent>>` (streams `KeyRotated`, `SasMismatch`) | new | `trust-verification.md` §L474 | downgrade banner |
| `queue_depth` | `ReadSignal<usize>` | new | `sync-queue.md` §L461 | strip, screen |
| `queue_peer_count` | `ReadSignal<usize>` | new | `sync-queue.md` §L462 | strip |
| `queue_per_peer` | `ReadSignal<HashMap<PeerId, QueueSummary>>` | new | `sync-queue.md` §L463 | letter row pills, member row pills |
| `queue_inbound_per_peer` | `ReadSignal<HashMap<PeerId, usize>>` | new (may be zero) | `sync-queue.md` §L464 | letter row pills |
| `queue_oldest_at` | `ReadSignal<Option<HlcTime>>` | new | `sync-queue.md` §L465 | "oldest waiting" copy |
| `queue_recent_arrivals` | `ReadSignal<Vec<ArrivedSummary>>` | new | `sync-queue.md` §L466 | recent-arrivals section |
| `relay_status` | `ReadSignal<RelayStatus>` (`Reachable`/`Unreachable`/`NotConfigured`) | new | `sync-queue.md` §L467 | relay-aware chrome |
| `device_online` | `ReadSignal<bool>` | new | `sync-queue.md` §L468 | reconnection toast |
| `AppState.ui.call_pinned_peer` | `Option<PeerId>` | new (local) | `call-experience.md` §L433 | call tile grid |
| `AppState.ui.call_pinned_share` | `Option<PeerId>` | new (local, ephemeral per call) | `call-experience.md` §L435 | call layout manager |
| `AppState.ui.call_layout` | existing | extend with `Grove` variant | `call-experience.md` §L426 | call header, controls strip |
| voice speaking-time history | 5-min rolling window per peer in voice actor | new (local, not persisted, not networked) | `call-experience.md` §L437 | speaking-stats popover |
| `AppState.server.display_name` | existing read surface | existing | `onboarding.md` §L500 | onboarding step 2, profile |
| `LinkedDevices` | derived from `LinkDevice`/`RevokeDevice` | new | `device-handoff.md` §L343 | handoff device list, Settings → devices |
| device presence | `online`/`offline`/`last_seen`/`network_kind` per linked device | new (design-only; protocol deferred) | `device-handoff.md` §L348 | handoff row status |
| `voice_participants_map` + per-peer mute | existing | existing | `call-experience.md` §L423 | call tiles |
| `local_video_stream` + `remote_video_streams` | existing | existing | `call-experience.md` §L425 | call video tiles |
| `AppState.voice.speaking_peers` | existing | existing | `call-experience.md` §L422 | call speaker halo |
| `call-connection-quality` | `RTCPeerConnection.getStats()` sampled 1 Hz per peer | existing (underlying API) | `call-experience.md` §L428 | per-tile quality chip |
| `tweaks.cryptoVisibility` | string enum | new | `settings-tweaks.md` §L490 | `trust-verification.md` (holder-pill visibility), channel header |
| `tweaks.sidebarVariant` | string enum | new | `settings-tweaks.md` §L507 | `layout-primitives.md` |
| `tweaks.showWordmark` | bool | new | `settings-tweaks.md` §L507 | `layout-primitives.md` |

The `layout-primitives.md` spec explicitly declares no new signals — it
consumes `ServerState`, `willow-network`, `willow-identity`, and the
signals declared by other child specs (§L506–L525).

## 3. Storage / schema

Local device state that doesn't live in the willow-state event log.

| Shape | Purpose | Location (crate / file best guess) | Owning UX spec |
|---|---|---|---|
| Per-device trust store (append-only) | `PeerTrust` transitions (`Unknown` → `Verified` / `Unverified` / `DowngradedFromVerified`) | `willow-identity` or `willow-client`; IndexedDB on web, `.dev/trust/` on native | `trust-verification.md` §L457–L463 |
| Per-letter `Letter` record | id, participants, created-at, owner | `willow-messaging`; per-device local | `letters-dms.md` §L582 |
| Per-letter key material | sealed-session key; fresh key on every group membership change | `willow-crypto`; per-letter | `letters-dms.md` §L581 |
| Letter message storage | reuses `MessageStore` per-letter topic | `willow-messaging` (existing infra) | `letters-dms.md` §L584 |
| Local archive flag on letters | device-only, not synced | `willow-messaging` | `letters-dms.md` §L591 |
| Outbound message queue (encrypted at rest) | keyed by `(message_id, recipient_peer_id)`; drains on peer-reachable + ACK | `willow-messaging` (new primitives) | `sync-queue.md` §L486–L496 |
| `QueueView` bounded in-memory projection | publishes per-peer queue stats to client view system | `willow-client` | `sync-queue.md` §L493 |
| Per-identity settings doc (CRDT-ish or LWW) | `readReceiptsDefault`, `typingDefault`, `cryptoVisibilityDefault`, `allowUnverifiedWhispers`, `localSearchIndex`, notification toggles, quiet-hours schedule | new — location unspecified; gossips only between an identity's own devices | `settings-tweaks.md` §L484–L489 |
| `localStorage` `willow.tweaks` | per-device tweaks (accent, density, crypto-visibility override, call layout, sidebar, wordmark) | `crates/web` localStorage | `settings-tweaks.md` §L645, `layout-primitives.md` |
| `localStorage` `willow.call.layout.<channel_id>` | per-channel call layout persistence | `crates/web` localStorage | `call-experience.md` §L439 |
| Local nickname map (per peer) | never propagated, cap 32 chars | browser storage keyed by peer id | `profile-card.md` §L442 |
| Quick-reaction recency cache | channel-scoped recent reactions | `crates/web` or `willow-client` | `message-row.md` §L524, `reactions-pins.md` (open question) |
| Identity export passphrase | OS keyring on native; IndexedDB (non-sync, device-scoped) on web | `willow-identity` | `settings-tweaks.md` §L512 |
| Moderation report (local) | written into the `governance.md` reports list | existing | `discover.md` §L333 |
| `threads: HashMap<EventHash, ThreadState>` materialization | `participants`, `archived`, cached `reply_count` on `ServerState` | `willow-state` materialize | `thread-pane.md` §L381 |
| `grant_paths: BTreeMap<EndpointId, Vec<EndpointId>>` | cached chain-of-trust projection | `willow-state` `ServerState` | `governance.md` §L551 |
| File index (projection over `Message` events with attachments) | derived `FileRef` index | `willow-state` projection | `governance.md` §L543 |

## 4. Network / protocol

Multi-step exchanges, new iroh topics, and new wire-level envelopes.

| Protocol | Purpose | Owning UX spec | Depends on |
|---|---|---|---|
| Discover topic (blake3-hashed via `network::topics`) | gossip `GrovePublish`/`GroveUnpublish`/`InviteRequest`/`InviteRequestCancel` events; users subscribe only while Discover is mounted | `discover.md` §L336 | `willow-network` topic registry |
| Device presence signalling | per-identity gossip of `online/offline/last_seen/network_kind` per linked device; not part of `ServerState` | `device-handoff.md` §L348 (design-only) | `LinkDevice` / `RevokeDevice`; open question: dedicated identity-private topic vs. relay piggy-back |
| Device-handoff 3-message protocol | `HandoffOffer` (with re-seal material), `HandoffReady` (target has installed keys), `HandoffAck` / `HandoffDecline` / `HandoffTimeout` | `device-handoff.md` §L352 | re-seal material wrapped in identity's device-to-device seal; detailed wire format deferred to a protocol spec |
| Per-letter sealed-session topic | each letter carries its own topic + signed message stream | `letters-dms.md` §L582–L584 | `willow-network` topic + `willow-crypto` sealed session |
| Letter membership event stream | add / remove / self-leave / rename messages embedded in letter stream | `letters-dms.md` §L583 | per-letter topic |
| Whisper message envelope | reuses `Message` infra but whisper id is carried in envelope so clients render with correct participant set | `whisper-mode.md` §L406–L409 | whisper key as seal key |
| Typing ping | `TypingPing` ephemeral gossip, rate-limited 3 s, not persisted | `composer.md` | channel topic (reused) |
| Profile gossip tiering | verified peers receive profile event before wider grove | `settings-tweaks.md` §L611 | existing trust tiering in `willow-network` |
| Inbound queue hint | optional field in peer last-seen heartbeat so viewer knows "N waiting for you from X" | `sync-queue.md` §L618 (open question) | heartbeat wire shape |
| Thread key derivation | thread key distinct from channel key; replies seal with thread key | `thread-pane.md` §L373 | `willow-crypto` key derivation |

## 5. Client methods (new APIs)

| Method | Signature hint | Owning UX spec |
|---|---|---|
| `send_typing(channel_id)` | rate-limited (3 s) typing ping | `composer.md` |
| `whisper_send(channel_id, body, reply_to)` | compose-surface extension; owned by `whisper-mode.md`, referenced by composer | `composer.md`, `whisper-mode.md` |
| `trust_state(peer_id)` | returns `PeerTrust` | `trust-verification.md` §L461 |
| `mark_verified(peer_id, session_key, words)` | records SAS-verified transition | `trust-verification.md` §L461 |
| `mark_unverified(peer_id, reason)` | records mismatch / downgrade | `trust-verification.md` §L461 |
| `begin_compare(peer_id) -> ComparePreview { you, them }` | opens the compare-fingerprints flow | `trust-verification.md` §L461 |
| `open_thread(parent: EventHash) -> ThreadHandle` | filtered reply stream + participant set | `thread-pane.md` §L359 |
| `send_thread_reply(parent, content)` | reply sealed with thread key | `thread-pane.md` §L361 |
| `leave_thread(parent)` | emits `ThreadLeave` | `thread-pane.md` §L362 |
| `thread_participants(parent) -> Vec<PeerId>` | participant lookup | `thread-pane.md` §L363 |
| Grove publish / unpublish / request / cancel request APIs | emit the 4 Discover events and manage local subscription | `discover.md` §L317–L328 |
| `DisplayMessage` projection `pinned: bool` | derived from channel pin events | `message-row.md` §L394 |
| `DisplayMessage.whisper: bool` | derived from whisper `EventKind` | `message-row.md` §L398 |
| `DisplayMessage.queue_note: QueueNote` | enum `None`/`LateArrival`/`Pending`, derived from `MessageStore` delivery + online state | `message-row.md` §L400 |
| `DisplayMessage.mentions: Vec<PeerId>` | explicit mention list | `message-row.md` §L404 |
| `DisplayMessage.thread_count`, `thread_last_at_ms`, `thread_participant_ids` | thread stub data | `thread-pane.md` §L391 |
| Read-receipt toggle API | per-letter opt-in; emits `ReadMark` | `letters-dms.md` §L585 |

## 6. Identity / profile field additions

| Field | Persistence (local / gossiped) | Owning UX spec |
|---|---|---|
| `pronouns` (≤ 32) | gossiped via `SetProfile` extension | `profile-card.md` §L428, `settings-tweaks.md` §L473 |
| `bio` (`profile-card.md` ≤ 240, `settings-tweaks.md` ≤ 280 — **conflict, see §7**) | gossiped via `SetProfile` extension | `profile-card.md` §L429, `settings-tweaks.md` §L473 |
| `tagline` (`profile-card.md` ≤ 80, `settings-tweaks.md` ≤ 60 — **conflict, see §7**) | gossiped via `SetProfile` extension | `profile-card.md` §L430, `settings-tweaks.md` §L473 |
| `crest pattern` (`profile-card.md` enum `{Fronds, Rings, Leaf}`, `settings-tweaks.md` "enum of 6" — **conflict, see §7**) | gossiped via `SetProfile` extension | `profile-card.md` §L431, `settings-tweaks.md` §L474 |
| `crest colour` / accent (`profile-card.md` RGB hex ≤ 7 chars + palette validator; `settings-tweaks.md` "enum of 7" — **conflict, see §7**) | gossiped via `SetProfile` extension | `profile-card.md` §L432, `settings-tweaks.md` §L474 |
| `pinned fragment` (`profile-card.md` `Option<Pinned { kind, body }>` body ≤ 280; `settings-tweaks.md` string ≤ 120 — **conflict, see §7**) | gossiped via `SetProfile` extension | `profile-card.md` §L433, `settings-tweaks.md` §L475 |
| `elsewhere` (`profile-card.md` `Vec<String>` up to 4 × 48; `settings-tweaks.md` up to 6 × 40 — **conflict, see §7**) | gossiped via `SetProfile` extension | `profile-card.md` §L435, `settings-tweaks.md` §L476 |
| nickname (local only, ≤ 32) | local browser storage per peer id | `profile-card.md` §L442 |
| device linked (`device_id`, `device_name`, `sub_key_pub`) | gossiped via `LinkDevice` | `settings-tweaks.md` §L479, `device-handoff.md` |
| peer-identity tombstone signal | announcement tombstone or local eviction after sustained unreachability | `letters-dms.md` §L590 (no owner) |

## 7. Conflicts / open questions

1. **Profile-field caps disagree.** `profile-card.md` and
   `settings-tweaks.md` both declare the new profile fields with
   *different* caps and enum widths:
   - bio: 240 (profile-card §L429) vs 280 (settings-tweaks §L473)
   - tagline: 80 (profile-card §L430) vs 60 (settings-tweaks §L473)
   - crest pattern enum: 3 variants (profile-card §L431) vs 6 (settings-tweaks §L474)
   - crest colour: hex string + accent-palette validator (profile-card §L432) vs enum of 7 (settings-tweaks §L474)
   - pinned fragment: `Pinned` struct with kind enum, body ≤ 280 (profile-card §L433) vs string ≤ 120 (settings-tweaks §L475)
   - elsewhere: 4 × 48 chars (profile-card §L435) vs 6 × 40 chars (settings-tweaks §L476)

   *Recommendation:* resolve during the profile-card plan;
   `settings-tweaks.md` consumes the field *definitions* from
   `profile-card.md`. Pin the canonical shape in the plan for
   `profile-card.md` and treat `settings-tweaks.md`'s settings form as
   the editor UI for whatever shape lands.

2. **Whisper revocation event.** `whisper-mode.md` §L522 asks whether
   `Whisper.Leave` is sufficient for a revoked peer who is offline, or
   whether a separate `Whisper.Revoke` is needed (signed by other
   whisperers on the victim's behalf). **Recommendation:** add
   `Whisper.Revoke` to the plan so an offline, non-cooperating peer
   can be cryptographically removed without waiting for their own
   `Leave`.

3. **`ChannelExpiryTick` event or pure derivation.** `ephemeral-channels.md`
   §L395/§L546 asks the state team whether an explicit tick event is
   required, or whether materialize can derive expiry from
   `ChannelCreate.created_at_hlc + duration_ms` against the HLC merge
   frontier. **Recommendation:** prefer pure derivation; add a tick
   only if merge determinism cannot be satisfied otherwise.

4. **Thread-join authority.** `thread-pane.md` §L375 tentatively sets
   `ThreadJoin` authority to `ManageChannels` / admin but explicitly
   says *confirm during planning*. Letters-style threads may want
   "any participant can invite". **Recommendation:** default to
   starter + `ManageChannels`; revisit after first usage.

5. **Inbound queue count source.** `sync-queue.md` §L618 asks whether
   `queue_inbound_per_peer` rides on the peer heartbeat or is omitted
   until peer returns. Depends on the heartbeat-shape decision owned by
   `willow-network`. **Recommendation:** allow the signal to be zero
   when unknown; treat heartbeat extension as a follow-up.

6. **Device presence topic scope.** `device-handoff.md` §L471 asks
   whether device presence propagates via a dedicated identity-private
   topic or via the relay. **Recommendation:** dedicated identity
   topic — relay-dependence undermines a trust-sensitive feature.

7. **Letter membership vs `willow-state`.** `letters-dms.md` says
   letter membership events are emitted into the letter's **own**
   signed stream, not `ServerState`. This is coherent with the letter
   model, but the pattern is new — the plan needs to define how
   membership events are stored, merged, and surfaced. Parallel
   thought: `ReadMark` is also not a `willow-state` event. **Recommendation:**
   write a small design memo before the letters plan lands.

8. **Peer-identity tombstone.** `letters-dms.md` §L590 calls for a
   "peer no longer exists" signal, described as a tombstone on
   announcement or local eviction after sustained unreachability. No
   spec owns the emission semantics. **Recommendation:** the letters
   plan needs to pick a definition (local-only eviction is simpler and
   avoids signalling "deletion" over the wire). Cross-check with
   `willow-identity` maintainers.

9. **Call-layout `Grove` variant breakage.** `call-experience.md` §L426
   extends the existing `AppState.ui.call_layout` enum with `Grove`.
   Any code exhaustively matching the enum must be updated. Small, but
   a coordination point with the messaging/thread split.

10. **Settings-tweaks vs profile-card ownership of `SetProfile`.**
    Both specs claim ownership of the `SetProfile` extension shape.
    The profile-card spec has the per-field design (caps, validators);
    the settings-tweaks spec has the edit UI and gossip-tiering
    requirement. **Recommendation:** `profile-card.md` owns the event
    shape; `settings-tweaks.md` owns the editor surface; treat the
    "enum of 6 crests" mismatch as a copy/paste bug in
    `settings-tweaks.md`.

11. **Per-identity settings doc shape.** `settings-tweaks.md` §L484
    calls for a CRDT or LWW document keyed on identity, gossiped
    between an identity's own devices only. There is no owning crate
    yet. **Recommendation:** design this before the settings plan
    lands; it is a new gossip surface that needs its own protocol spec
    (similar scale to the discover topic).

12. **Quick-reaction recency scope.** `message-row.md` §L524
    (tracked via reactions-pins.md, which does not yet exist)
    designates quick-reaction recency as channel-scoped. If
    `reactions-pins.md` ships with a different scope, the
    `DisplayMessage` projection needs to know.

## 8. Suggested sequencing

The parent tier DAG applies. The sequencing below calls out
**data-layer** constraints in addition to the UX-tier order.

**Tier A — foundational data primitives (must ship first).** Three
primitives gate most downstream work:

1. **SAS derivation + local trust store** (`trust-verification.md`).
   Whisper gating, device-handoff gating, ephemeral-holder warning
   chips, Discover verified badge, profile-card badge, and onboarding
   step 4 all consume `peer_trust`. Ship the pure `willow_crypto::sas`
   module, the trust store schema, and the `trust_state` client API
   before anything that reads them.
2. **`SetProfile` extension** (`profile-card.md` owning shape). Until
   the field definitions land, `settings-tweaks.md` has nothing to
   edit and `profile-card.md` has nothing to render. Freeze caps and
   enum widths here.
3. **Outbound queue primitives + `connection_state`** (`sync-queue.md`).
   `composer.md` states, letters row pills, and the offline
   banner in `layout-primitives.md` all read these. Also extend
   `connection_status` with `"offline"`.

**Tier B — shared state that several UX specs consume.** Once Tier A
is in place:

4. **`LinkDevice` / `RevokeDevice` events** (`settings-tweaks.md`) and
   device-presence protocol design (`device-handoff.md`).
   These are prerequisites for the handoff surface and the Settings →
   devices list. Presence protocol can be scoped smaller (one bit per
   device) to unblock.
5. **Thread events** (`thread-pane.md`). `DisplayMessage.thread_*`
   fields need the `threads` materialization shape before stub
   rendering works. Thread-key derivation is a small new crypto path
   (sibling of channel-key derivation) — align with Tier A.
6. **Whisper events** (`whisper-mode.md`). Depends on trust-verification
   (Tier A) because the `Whisper.Start` authority check consults
   `peer_trust`.

**Tier C — feature-scoped surfaces that can parallelise.** Once Tier B
lands:

7. **Governance extensions** (`SetRoleColour`, `SetRoleDescription`,
   `DereferenceFile`, chain-of-trust projection, file index).
   Coordinates with `onboarding.md` for the invite-container
   extensions (optional role_id, note).
8. **Discover events + Discover topic** (`discover.md`). Independent
   of the other Tier C items but needs the owner-verification badge
   from Tier A.
9. **Ephemeral channels** (`ephemeral-channels.md`). Needs Tier A's
   trust store only indirectly (holder unverified chip). The
   `ChannelExpiryTick` question should be resolved before plan-writing.
10. **Letters / DMs** (`letters-dms.md`). Needs Tier A
    (`peer_trust`, queue primitives). Letter membership events and
    the per-identity peer-tombstone signal must be specified in this
    plan. `ReadMark` events belong to the letter stream, not
    `ServerState`.

**Tier D — pure UI rollup.** `layout-primitives.md`, `message-row.md`,
`composer.md`, `reactions-pins.md`, `files-inline.md`,
`call-experience.md`, `profile-card.md`, and
`onboarding.md` consume everything above. These can ship alongside
their child feature plans; they declare no new events of their own
(except the three `messaging` projections).

**Cross-cutting call-outs.**

- The **per-identity settings doc** (see §7.11) is a new protocol
  surface. Even though `settings-tweaks.md` owns the editor, the
  protocol itself should be specced like the discover topic: which
  topic id, which wire shape, which CRDT semantics.
- **`SetProfile` extension** vs. **`LinkDevice` / `RevokeDevice`** both
  modify identity-adjacent state. They must agree on the distinction
  between "per-peer public profile" and "per-identity private
  settings".
- **Thread key derivation** vs. **ephemeral channel key derivation**
  are parallel small crypto paths. Share code paths where possible
  inside `willow-crypto` to avoid two near-identical implementations.
- **`Whisper.Revoke`** should land with the other Whisper events, not
  as a follow-up — otherwise whisper offline revocation remains
  unsolved.
