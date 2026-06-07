# Issue #354 — search index rebuilt from scratch on every message-list change

**Status:** landed (PR #507, commit `e07d974`, 2026-05-02) — `bootstrap.rs` with `hydrate_index` / `index_message` / `reindex_message` shipped alongside the dropped Effect in `app.rs:436` and the new `bootstrap_tests` module in `crates/client/src/search/tests.rs:1048`.

## Problem

`crates/web/src/app.rs:419` Effect reads `messages_sig.get()`
(current-channel messages) and calls `search.rebuild(indexable)` on every
change. Every send/receive/edit destroys + rebuilds the index
synchronously on the WASM main thread. Switching channels also wipes the
index for the previous channel.

## Approach

1. **Drop the rebuild Effect.** Remove the `messages_sig`-driven
   indexer at `app.rs:377-425` entirely. The index will be a global,
   long-lived structure built incrementally.

2. **New willow-client helpers (`crates/client/src/search/bootstrap.rs`).**

   - `pub async fn hydrate_index(&handle, &search, grove_id)` — walks
     every channel via `client.channels()` + `client.messages(name)`,
     builds an `IndexableMessage` per row using
     `IndexableMessage::from_display_message`, and calls
     `search.insert(...)` (idempotent on `message_id`). Used once at
     bootstrap.

   - `pub async fn index_message(&handle, &search, channel_id,
     message_id, grove_id)` — looks up channel name from id, fetches
     the matching message, builds an `IndexableMessage`, and calls
     `search.insert(...)`. Used by the incremental path on
     `MessageReceived`.

   - `pub async fn reindex_message(&handle, &search, channel_id,
     message_id, grove_id)` — calls `search.remove_message` first so
     the new body wins, then re-inserts. Used by the incremental path
     on `MessageEdited`.

   Why a new module: the bootstrap and incremental paths are pure
   client-tier logic (no DOM). Putting them in willow-client makes
   them testable with `test_client()` instead of needing a browser
   harness.

3. **New web-crate indexer task (`crates/web/src/app.rs`).** Replace the
   dropped Effect with a single `wasm_bindgen_futures::spawn_local`
   task that:

   - Subscribes to `client.subscribe_events()`.
   - Reads the active grove from `app_state.server.active_server_id`.
   - Calls `hydrate_index(...)` once on entry. (Storage-loaded events
     are already materialized synchronously in `ClientHandle::new`, so
     anything on disk is hydrated immediately. Live events arriving
     before `hydrate_index` returns are picked up by the subscriber
     loop — `insert` is idempotent, so a doubled hit is a no-op.)
   - Loops on `event_rx.recv()` and dispatches:
     - `MessageReceived { channel, message_id, .. }` → `index_message`
     - `MessageEdited { channel, message_id, .. }` → `reindex_message`
     - `MessageDeleted { channel, message_id }` → `search.remove_message`
     - `ChannelDeleted(channel_id)` → `search.remove_channel`

   The grove signal is read fresh on each event so a future
   multi-grove client picks up the right id.

## Tests

- **client-tier (`crates/client/src/search/tests.rs`)**: new
  `bootstrap_tests` module:
  - `hydrate_index_inserts_all_channels` — create 2 channels, send N
    messages to each via `test_client`, call `hydrate_index`, assert
    `message_count == 2N` and querying picks up both channels.
  - `hydrate_index_idempotent` — call twice, count stays the same.
  - `index_message_inserts_one` — assert single-message insert lands.
  - `reindex_message_replaces_body` — edit a message body, assert old
    body no longer matches but new body does.

- **web browser-tier (`crates/web/tests/browser.rs`)** — N/A. The
  rebuild Effect lives in `App()` which is wired to a live
  `ClientHandle<IrohNetwork>`. We can't realistically mount the full
  app under wasm-pack without a fake network. The bootstrap +
  incremental-update behaviour is fully covered by client-tier tests
  using `test_client()`. Skipping the DOM tier here is consistent with
  the test-tier decision tree in `CLAUDE.md`: "Default to lowest tier
  covering behaviour."

## Files touched (target ≤ 5)

1. `crates/client/src/search/bootstrap.rs` — new
2. `crates/client/src/search/mod.rs` — wire module
3. `crates/client/src/search/tests.rs` — bootstrap tests
4. `crates/web/src/app.rs` — drop rebuild Effect, spawn indexer task

## Trade-offs

- **Edits require channel-id → name lookup.** Each `MessageEdited`
  triggers a `state_snapshot()` to find the channel name. Cheap; the
  snapshot is an `Arc::clone`. Considered caching — premature.
- **Rejected: keep the Effect, gate on first run.** Less robust —
  signal-driven first-run still has the wrong subscription target
  (only current channel) and doesn't handle edits.
- **Rejected: poll for channels on a timer.** Wastes battery, racy.
