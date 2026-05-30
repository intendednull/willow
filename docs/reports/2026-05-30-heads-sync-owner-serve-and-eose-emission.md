# Heads-sync owner-serve gate + gossip EOSE emission

**Date:** 2026-05-30
**Status:** landed
**PR:** #664 (`worktree-relay-upgrade-bundle`)
**Specs touched:** `specs/2026-04-24-negentropy-sync.md`,
`specs/2026-04-24-history-sync-eose.md`
**Plan touched:** `plans/2026-05-28-relay-upgrade-bundle.md`

## Summary

PR4/PR5 of the "relay upgrade bundle" turned the Playwright E2E suite red
(it was green at PR3). Two independent bugs, both in the gossip sync path,
each made a different E2E spec time out. This report records the two root
causes, the fix chosen for each, the alternatives considered, and the one
residual nuance (`stream_generation` stability).

## Bug 1 — the serving gate excluded the server owner

### Root cause

The `SyncRequestV2` responder in `crates/client/src/listeners.rs` gated
serving on `ServerState::has_explicit_permission(&local, SyncProvider)`.
That predicate **deliberately ignores** the "admins/owner hold every
permission" rule (`crates/state/src/server.rs`: `has_permission` honors
`admins`; `ServerState::new` inserts the genesis author into `admins`). So
the server **owner** — the canonical source of her own server's state —
refused to serve a joining peer's `SyncRequestV2`. In the web E2E mesh
nobody holds an explicit `SyncProvider` grant, so joiners could never
backfill and timed out (`e2e/multi-peer-sync.spec.ts`,
`e2e/cross-browser-sync.spec.ts`).

### Why this is a bug, not the intended design

Pinned decision 4 of the bundle plan says "a peer serves a delta only if it
holds `SyncProvider`." The owner **does** hold `SyncProvider` — implicitly,
under the authority model (`docs/specs/2026-04-12-state-authority-and-mutations.md`:
owner = root of all permissions; admins inherit every permission). The
explicit-only reading was a deviation from the authority model, not a
faithful encoding of decision 4. It is also operationally fatal: a 2-peer
server where nobody has been explicitly granted the role is **unsyncable**
under it.

### Fix

Gate on `ServerState::is_sync_provider(&local)` (= `has_permission(SyncProvider)`),
which honors the owner/admins implicitly **and** explicit grants. A regular
member (neither owner/admin nor explicitly granted) still refuses, so the
gate stays meaningful. The receiver-side trust gate for the EOSE marker was
moved to the same predicate so a marker from the owner is trusted.

## Bug 2 — the EOSE marker was unobservable by any client

### Root cause

`WireMessage::HistorySyncComplete` was emitted **only** by workers, **only**
in response to a `WorkerRequest::Sync` (`crates/worker/src/actors/network.rs`).
Web clients backfill over **gossip** (`SyncRequestV2`) and never send a
`WorkerRequest::Sync` or subscribe to the worker reply path, so no marker
ever reached them. The PR5 EOSE feature was dead end-to-end for clients:
`e2e/history-sync.spec.ts` waited for a `HistorySynced` event that never
fired, even though the joiner's DAG converged.

### Fix

The gossip `SyncRequestV2` responder now also broadcasts a
`HistorySyncComplete` on SERVER_OPS after a successful serve, with:

- `topic_id` = the SERVER_OPS topic id (SyncRequestV2 flows on SERVER_OPS;
  identical to the worker's `ops_topic_id()`),
- `last_event_hash` = the hash of the last streamed event (captured before
  `delta` is moved into `pack_sync_batches`; `None` for an empty delta),
- `stream_generation` = a stable per-session value (see nuance below).

Because the receiver trust gate now honors the owner/admins (Bug 1 fix), an
owner-served backfill produces an observable `HistorySynced` whose
`provider` is the owner. This completes the feature the plan had deferred as
the "peer-to-peer marker."

## Alternatives considered

**Bug 1 — owner-serve.**

1. **Keep the strict explicit-only gate, auto-grant the owner `SyncProvider`
   at genesis.** Rejected: it pollutes the DAG with a self-grant the
   authority model already implies, duplicates the owner's authority in two
   places (the `admins` set *and* `peer_permissions`), and every server
   would carry a redundant grant event. The implicit rule already says the
   owner holds every permission; the gate should read it, not work around
   it.
2. **Legacy-fallback: let joiners fall back to the ungated 500-event
   `SyncRequest` path.** Rejected: the legacy path is slated for removal,
   re-entrenching it is backwards, and it is a heuristic dump (no per-author
   delta) — exactly what the heads protocol replaces.
3. **Re-scope the E2E test to grant a provider explicitly before asserting
   convergence.** Rejected: the test pins the real P2P UX (two members, no
   relay/worker, must still sync). Changing the test to dodge the bug would
   hide that a bare 2-peer server cannot sync — a real product regression.
4. **Chosen: gate on `is_sync_provider` (honor owner/admins implicitly).**
   Faithful to decision 4 (owner *does* hold `SyncProvider`), no DAG
   pollution, keeps the gate meaningful for regular members.

**Bug 2 — EOSE emission.**

1. **Make web clients use the worker ALPN `WorkerRequest::Sync` path so the
   existing worker emission reaches them.** Rejected: it abandons the gossip
   heads protocol the bundle just built for clients, requires every client
   to find and round-trip a worker, and breaks the no-worker P2P case
   entirely (the E2E mesh has no worker).
2. **Chosen: emit the marker from the gossip `SyncRequestV2` responder too.**
   Minimal, rides the exact path clients already use, and works in the
   no-worker case. The plan had deferred peer emission expecting workers to
   cover clients; worker-only emission turned out unobservable, so this is
   the completion of that deferred item, not new scope.

## Residual nuance — `stream_generation` stability

The marker's `stream_generation` is **not** generated fresh per serve. It is
a stable per-session value (`ListenerCtx::history_stream_generation`,
generated once per `ClientHandle::connect()` via
`uuid::Uuid::new_v4().as_u64_pair().0` — WASM-safe, no `std` RNG). The
receiver dedups on `(provider, stream_generation)`
(`NetworkMeta::record_history_marker`), so a stable value makes **repeated
serves to a reconnecting peer idempotent**: the owner can answer many
`SyncRequestV2`s in one session without spamming the joiner with a fresh
`HistorySynced` each time. A genuinely new session (reconnect → new
`connect()`) produces a new generation and correctly re-emits. This mirrors
the worker's per-run `stream_generation` (`rand::random()` once at actor
construction), adapted to the client's per-connect lifecycle.

## Tests

Client-tier (`crates/client/src/tests/heads_sync.rs`), driving
`process_received_message` directly over `MemNetwork`:

- `owner_without_explicit_grant_serves` — owner (auto-admin, no explicit
  grant) serves a `SyncRequestV2`; joiner gets the full chain. (Fails before
  the Bug 1 fix.)
- `regular_member_without_sync_provider_serves_nothing` — reconciled from
  the old `responder_without_sync_provider_serves_nothing`, which asserted
  the *owner herself* served nothing; that encoded the now-corrected
  deviation. The responder is now a *regular member* (replayed the owner's
  DAG into a fresh identity), who correctly still refuses.
- `owner_served_backfill_emits_history_synced` — owner serves a non-empty
  backfill over gossip, emits a `HistorySyncComplete`, and the receiving
  peer surfaces exactly one `ClientEvent::HistorySynced` whose provider is
  the owner. (Fails before the Bug 2 + receiver-gate fixes.)
- `responder_streams_full_chain_for_empty_heads` (pre-existing) — an
  explicit-grant holder still serves: guards against regressing the explicit
  path.

The five existing `tests_history_eose` cases still pass unchanged (the
provider identities there are explicitly granted, so the owner-honoring
change is a no-op for them; the untrusted `imposter` is still untrusted).
