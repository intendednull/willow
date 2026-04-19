# Test Audit Report — Willow Project
**Date:** 2026-04-13
**Scope:** All crates in the workspace (14 crates, ~649 tests total)
**Method:** Independent audit of each crate's test suite, synthesized here

---

## Executive Summary

The Willow test suite is **uneven in a structurally dangerous way.** The lowest-level primitives (willow-actor, willow-crypto) are excellent, but coverage collapses precisely at the layers where security properties must be enforced: the client boundary, the relay's core routing logic, and the UI/state bridge. Three findings stand out above all others:

**1. The client layer (willow-client) is nearly untested where it matters most.** `mutations.rs`, `listeners.rs`, `views.rs`, and `derive_client_events` together represent the entire behavioral surface of the client API — and collectively have zero tests. The 69 existing client tests cover wire serialization utilities and emoji shortcodes, not the code that builds DAGs, checks permissions, or delivers events to the UI.

**2. Multiple crates test the wrong code path entirely.** `willow-common` and `willow-messaging` worker tests use `bincode::serialize` directly, but production uses `willow_transport::pack/unpack`. These tests provide false confidence: they can pass even when the wire protocol is broken. Similarly, `willow-web`'s 97 browser tests instantiate zero real Leptos components — they re-implement component logic inline, so a complete rewrite of any component would leave every test green.

**3. Several tests are structured so they cannot fail.** `node_disconnect_detected` in willow-network accepts timeout as success. `rename_server` tests in willow-agent assert only `!name.is_empty()` — a property that holds even if the rename fails. The multi-peer section of willow-agent's e2e creates two clients that never connect to each other. These tests provide zero regression value while implying coverage.

**Priority order for remediation:**
1. willow-client (security + behavioral correctness at the most important boundary)
2. willow-web / willow-agent (false confidence removal and real component testing)
3. willow-relay (gossip bridging — the relay's primary purpose has no test)
4. willow-network (IrohNetwork adapter layer entirely untested)
5. willow-state (member-vs-member message permissions, `accept: false` votes)

---

## Crate-by-Crate Scorecard

| Crate | Rating | Tests | Key Finding |
|---|---|---|---|
| willow-actor | 5/5 | 92 + 15 perf | Complete primitive coverage; best in project |
| willow-crypto | 5/5 | 46 | Best security testing; AEAD, ratchet, DoS, zeroization all covered |
| willow-state | 4/5 | 166 | Strong governance/permission coverage; member-vs-member message auth and `accept:false` votes missing |
| willow-identity | 4/5 | 22 | Core operations solid; no impersonation test |
| willow-storage | 4/5 | 21 | Good durability primitives; `sync_since` (primary peer catch-up path) barely tested |
| willow-replay | 4/5 | 16 | OOO buffering solid; Snapshot branch untested; `on_event` routing has zero tests |
| willow-worker | 4/5 | 36 | Real actor integration; SyncActor behavior entirely untested; no two-worker convergence test |
| willow-transport | 4/5 | 11 | Pack/unpack and size limits covered; version 0 rejection and boundary-exact size missing |
| willow-messaging | 3.5/5 | 25 | HLC ordering good; clock regression (the defining HLC property) untested; wrong codepath in worker tests |
| willow-common | 3/5 | 17 | Tamper detection works; 7/9 WireMessage variants have no serde test; serializer mismatch in tests |
| willow-agent | 3/5 | 52 | Scope enforcement present; notification_bridge (core MCP feature) entirely untested; false confidence multi-peer test |
| willow-network | 3/5 | 16 | MemNetwork semantics solid; IrohNetwork adapter layer untestable and untested |
| willow-web | 2.5/5 | 97 | Zero real components tested; tests re-implement logic inline; event_processing.rs and state_bridge.rs dark |
| willow-client | 2.5/5 | 69 | mutations.rs, listeners.rs, views.rs, derive_client_events — all zero coverage |
| willow-relay | 2/5 | 12 | topic_announce_listener (core logic) zero tested; gossip bridging (primary purpose) never tested |
| Playwright E2E | 3/5 | 68 | multi-peer-sync.spec.ts is genuinely good; ~35 tests should be unit tests; timeout-based waits throughout |

---

## Cross-Cutting Patterns

### 1. Testing the Wrong Code Path

This is the most pervasive structural problem. Tests in at least three crates exercise code paths that production never uses:

**Serializer mismatch (willow-common, willow-messaging):** Both crates' `worker_types.rs` tests call `bincode::serialize` / `bincode::deserialize` directly. Production code uses `willow_transport::pack` and `willow_transport::unpack`, which add framing, versioning, and size limits. A breakage in `willow_transport` that corrupts the wire format would leave these tests green.

**Inline component re-implementation (willow-web):** The 97 browser tests never call `view! { <ChatInput ... /> }` or any other real component. Instead, each test builds equivalent HTML inline using raw Leptos primitives. If `ChatInput` were completely rewritten with a bug, no test would catch it. Some tests copy-paste production functions into the test file and test the copy, not the original.

**Recommendation:** In willow-common and willow-messaging, replace `bincode::serialize` calls with `willow_transport::pack` / `willow_transport::unpack`. In willow-web, write at least one test per component that mounts the real component and exercises its behavior.

### 2. False Confidence Tests

Tests that cannot detect regressions are worse than no tests — they actively mislead.

**`node_disconnect_detected` (willow-network/src/mem.rs):** The test accepts both a disconnect notification and a timeout as valid outcomes. The assertion branch for "no disconnect received" passes. This test cannot fail.

**`rename_server` tests (willow-agent):** Both rename tests assert only that `result` is `!name.is_empty()`. A rename that silently fails, returns the old name, or returns any non-empty string passes this test.

**`kick_member_removes_from_server` (willow-agent):** Kicks self, then asserts the result is a non-empty string. The member list is never consulted.

**Multi-peer section of willow-agent e2e:** Creates two `ClientHandle` instances but never connects them through any network. Messages sent from client A can never reach client B. The test asserts properties of isolated local state, calling it "multi-peer."

**`sync_snapshot_fallback_when_peer_is_behind` (willow-replay):** Claims to test the `WorkerResponse::Snapshot` branch but the Snapshot variant is never triggered in the test body. The test passes by exercising a different code path than its name implies.

**`>= N` assertions on deterministic state (willow-state):** Stress tests asserting `assert!(state.messages.len() >= 1000)` on a deterministic, pure state machine. These should be exact equality assertions. The current form would pass even if half the messages were dropped.

**Recommendation:** Delete or rewrite `node_disconnect_detected`, both `rename_server` tests, `kick_member_removes_from_server`, and the multi-peer e2e section. Convert `>= N` state assertions to `== N`. Fix `sync_snapshot_fallback_when_peer_is_behind` to actually trigger the Snapshot branch.

### 3. Security-Critical Code with Zero Coverage

Permission enforcement is Willow's primary security boundary. Several critical permission checks are untested:

**Member-vs-member message editing (willow-state):** Existing tests verify that a stranger cannot edit a member's message. No test verifies that Member A cannot edit or delete Member B's messages. This is a distinct security property (peer privilege escalation) and is completely untested.

**`accept: false` votes (willow-state):** The negative vote path in governance has zero tests. Any bug in rejection logic — including a crash or silent accept — would go undetected.

**Permission enforcement at the client boundary (willow-client):** `mutations.rs` performs permission checks before building DAG events. This entire file has zero tests. A regression that stripped permission checks from client-side mutations would not be detected by any existing test.

**`listeners.rs` tamper detection (willow-client):** The listener handles 15+ incoming WireMessage variants and is supposed to enforce tamper detection on received messages. Zero tests exist for any of this logic.

### 4. Missing Integration Seams

**`test_client_on_hub` (willow-client):** A helper function exists specifically for multi-peer client tests, but it has zero callers anywhere in the codebase. The multi-peer behavioral surface of the client — the thing that makes Willow actually work — has never been tested through this seam.

**`IrohNetwork` (willow-network):** The production networking implementation cannot be constructed in tests without a real relay server or mDNS. The two integration tests that do test real gossip bypass `IrohNetwork` entirely and use raw iroh APIs. The `IrohTopicHandle` and `IrohTopicEvents` adapter layers, which translate between iroh primitives and Willow's trait interface, are entirely untested.

**`topic_announce_listener` (willow-relay):** The relay's core routing logic — topic validation, MAX_TOPICS cap enforcement, dynamic subscription management — has zero tests despite `MemNetwork` being available and usable from tests. This function is the relay's primary job.

### 5. Structural Duplication in Test Helpers

**willow-state:** The `test_dag()` genesis setup helper and associated scaffolding is duplicated across four files: `tests.rs`, `materialize.rs`, and at least two others. The test split between `tests.rs` (78 tests) and `materialize.rs` (28 tests) follows no principled boundary — both contain behavioral tests for the same state machine.

**willow-storage:** The `setup_identity_and_genesis` test helper accepts a `channel` parameter that is never used in any test. Dead parameter in shared test infrastructure.

---

## Critical Gaps by Risk

Ranked by: (security impact) × (probability of bug existing undetected)

### Priority 1 — Security Properties with Zero Coverage

1. **Member-vs-member message edit/delete in willow-state.** Peer privilege escalation. Add `test_member_cannot_edit_another_members_message` and `test_member_cannot_delete_another_members_message` to `crates/state/src/tests.rs`.

2. **`mutations.rs` permission checks in willow-client.** Every client-side mutation (send, edit, delete, kick, grant, revoke) goes through `mutations.rs`. Zero tests. Add tests using `test_client()` helper for each mutation, verifying both the success path and permission-denied rejection.

3. **`listeners.rs` tamper detection in willow-client.** Remote message handling with tamper detection. Zero tests. Use `test_client_on_hub` to send tampered WireMessage variants and assert they are rejected.

4. **`accept: false` votes in willow-state.** Negative governance votes. Zero tests. Add `test_governance_reject_vote` and `test_reject_vote_below_threshold` to `crates/state/src/tests.rs`.

### Priority 2 — Core Behavioral Paths with Zero Coverage

5. **`derive_client_events` in willow-client.** Maps every EventKind to a ClientEvent. Zero tests. Verify that each EventKind variant produces the expected ClientEvent output.

6. **`views.rs` in willow-client.** `compute_messages_view`, `compute_members_view`, etc. — the functions that compute what the UI displays. Zero tests.

7. **`topic_announce_listener` in willow-relay.** The relay's core routing logic. Zero tests. Use MemNetwork to test topic validation, subscription limits, and dynamic subscription.

8. **Gossip bridging in willow-relay.** The relay's primary purpose (bridging TCP and WebSocket peers) has no test. Add an integration test that sends a message on one connection type and receives it on another.

9. **Two-worker convergence in willow-worker.** State synchronization between two workers is the system's core value proposition. Zero tests verify that two workers actually converge. Use `test_client_on_hub` or equivalent to set up two workers sharing a MemNetwork and assert state convergence.

10. **`on_event` server routing in willow-replay.** Four cases (CreateServer, known prev, known author, fallback) with zero tests.

### Priority 3 — Reliability Gaps

11. **`sync_since` in willow-storage.** The primary data catch-up path for new peers. Virtually no direct coverage. Add a test that stores N events, then calls `sync_since` with various cursor positions and asserts correct results.

12. **Persist→drop→reopen durability in willow-storage.** The most fundamental durability regression test. Not present. Add it.

13. **HLC clock regression in willow-messaging.** HLC's defining property is handling backward-moving clocks. The wall clock is not injectable, making this hard but not impossible to test with a mock.

14. **SyncActor behavior in willow-worker.** Sync requests are never verified to be broadcast, nor is their content checked. Add tests that assert sync broadcast and head correctness.

15. **WorkerResponse::Snapshot branch in willow-replay.** Claimed to be tested by `sync_snapshot_fallback_when_peer_is_behind` but never actually triggered. Fix the test.

### Priority 4 — False Confidence Removal

16. Delete `node_disconnect_detected` (willow-network) or rewrite to use a channel with a definitive assert.
17. Delete or rewrite both `rename_server` tests in willow-agent.
18. Delete or rewrite `kick_member_removes_from_server` in willow-agent.
19. Delete the multi-peer section of willow-agent's e2e or connect the clients through MemNetwork.
20. Convert all `>= N` assertions in willow-state stress tests to `== N`.

---

## Per-Crate Details

### willow-state (`crates/state/src/`)
**Test files:** `tests.rs` (78 tests), `materialize.rs` (28 tests)

**Strengths:** Permission enforcement for strangers vs. members, last-admin guard, multi-admin vote threshold, vote ordering with dependency resolution, equivocation rejection (3 scenarios), pending buffer/OOO delivery, topo-sort determinism, sync round-trip, Issue #109 regression guard.

**Actions:**
- Add `test_member_cannot_edit_another_members_message`: create server, two members, Member A sends message, Member B attempts edit — assert rejection.
- Add `test_member_cannot_delete_another_members_message`: same setup for delete.
- Add `test_governance_reject_vote` and `test_reject_vote_below_threshold` for `accept: false` paths.
- Add `test_set_vote_threshold_cascade`: verify that changing threshold mid-vote applies to pending vote count.
- Convert all `assert!(x.len() >= N)` in stress tests to `assert_eq!(x.len(), N)`.
- Consolidate `test_dag()` helper into a single shared location (consider `crates/state/src/test_helpers.rs`) and remove duplication from 4 files.
- Evaluate whether the tests.rs / materialize.rs split has a principled boundary; if not, merge.

### willow-client (`crates/client/src/`)
**Test files:** `lib.rs` test module (69 tests)

**Strengths:** Wire serialization round-trips, emoji shortcode normalization, worker cache behavior.

**Actions:**
- `mutations.rs` — add one test per public mutation function using `test_client()`. For each: verify success path (correct EventKind emitted, correct ClientEvent received), and permission-denied path (non-owner/non-admin rejected).
- `listeners.rs` — activate `test_client_on_hub` (currently zero callers). Write tests that inject WireMessage variants from a second peer and assert: correct ClientEvent delivery for happy path, rejection for tampered messages, rejection for messages with wrong signer.
- `views.rs` — add tests for `compute_messages_view` (correct ordering, edit/delete application), `compute_members_view` (member join/kick/leave), and similar view functions.
- `derive_client_events` — parameterized test over every EventKind variant asserting correct ClientEvent output.
- Message mutations (edit, delete, react, pin, unpin) — zero tests. Add one behavioral test each.
- Replace all `tokio::time::sleep()` synchronization in existing tests with channel-based signaling or `tokio::time::timeout` with failure on timeout.

### willow-messaging (`crates/messaging/src/`)
**Test files:** `lib.rs` test module (25 tests)

**Strengths:** HLC ordering invariants, two-clock convergence, store ordering with scrambled timestamps, duplicate timestamp collision handling.

**Actions:**
- Make wall clock injectable (wrap `SystemTime::now()` behind a trait or closure) to enable deterministic HLC testing.
- Once injectable: add `test_hlc_clock_regression` (wall clock moves backward, HLC must advance), `test_hlc_receive_when_local_ahead` (local ts > remote ts, counter logic), `test_hlc_counter_overflow`.
- Fix `message_serde_round_trip`: add assertion that `decoded.hlc == original.hlc`.
- Fix `worker_types.rs` tests: replace `bincode::serialize` / `bincode::deserialize` with `willow_transport::pack` / `willow_transport::unpack`.

### willow-common (`crates/` — shared worker types)
**Test files:** `worker_types.rs` test module (17 tests)

**Strengths:** `tampered_data_fails_unpack` (security-critical), pack/unpack signing round-trip with EndpointId verification.

**Actions:**
- Replace `bincode::serialize` with `willow_transport::pack` in all tests.
- Add serde round-trip tests for the 7 untested WireMessage variants: `TypingIndicator`, `VoiceJoin`, `VoiceLeave`, `VoiceSignal`, `JoinRequest`, `JoinResponse`, `JoinDenied`, `TopicAnnounce`.
- Add `test_wrong_signer_attribution`: pack a message signed by Alice, assert that unpack reports Alice as author even if the payload claims Bob.
- Add `WorkerResponse::Snapshot` serde test.

### willow-storage (`crates/storage/src/`)
**Test files:** test module in `lib.rs` (21 tests)

**Strengths:** Dedup, pagination with cursor, batch atomicity, corruption tolerance, PRAGMA checks (WAL, synchronous=FULL, foreign keys on), migration idempotency.

**Actions:**
- Add `test_sync_since_returns_correct_events`: insert 20 events with known IDs, call `sync_since` at positions 0, 5, 10, 19 and assert exact event sets returned.
- Add `test_persist_drop_reopen_durability`: insert events, drop the storage object, reopen from same path, assert all events present with round-trip equality.
- Add round-trip equality assertions to existing tests: replace `assert_eq!(events.len(), N)` with element-wise equality against originals.
- Add `test_history_multi_author_cursor`: two authors, interleaved events, verify `history()` HeadsSummary cursor handles both authors correctly.
- Remove unused `channel` parameter from `setup_identity_and_genesis` test helper.

### willow-replay (`crates/replay/src/`)
**Test files:** test module (16 tests)

**Strengths:** OOO buffering and resolution, deeply OOO chain resolves, Issue #51 regression pin, multi-author sync delta.

**Actions:**
- Rewrite `sync_snapshot_fallback_when_peer_is_behind`: trace through the code to find what inputs actually trigger `WorkerResponse::Snapshot`, then construct those inputs. The test must assert that the Snapshot variant is received.
- Add `test_on_event_create_server`: send a CreateServer event to `on_event`, assert server is created in state.
- Add `test_on_event_unknown_prev_buffers`: send an event whose parent is unknown, assert it is buffered.
- Add `test_on_event_known_author_routing`: verify correct routing for events from known vs. unknown authors.
- Add `test_lru_eviction_order`: fill cache to capacity + 1, assert that the least-recently-used entry is the one evicted (not just that count == capacity).

### willow-network (`crates/network/src/`)
**Test files:** `mem.rs` test module, integration tests (16 tests)

**Strengths:** MemNetwork semantics (no self-loopback, topic isolation, neighbor events), topic ID stability (blake3 hash pinning).

**Actions:**
- Fix `node_disconnect_detected`: remove the timeout-as-success branch. The test must definitively assert a disconnect event is received within a tight deadline, or fail.
- Fix `broadcast_neighbors` divergence: if `neighbors_only: true` should deliver only to neighbors (not all subscribers), fix the MemNetwork implementation and add a test that distinguishes the two cases.
- Make `IrohNetwork` testable: extract address lookup behind a trait or inject a test relay address so `IrohNetwork` can be constructed without a live relay. This unblocks testing `IrohTopicHandle` and `IrohTopicEvents`.
- Add tests for `IrohTopicHandle::broadcast` and `IrohTopicEvents::next` via the trait interface.

### willow-relay (`crates/relay/src/`)
**Test files:** test module (12 tests)

**Strengths:** Bootstrap DoS hardening (semaphore cap, slowloris timeout, permit recovery), protocol conformance.

**Actions:**
- Add `test_topic_announce_listener_rejects_invalid_topic`: send a malformed TopicAnnounce, assert relay drops it without crashing.
- Add `test_topic_announce_listener_enforces_max_topics`: send MAX_TOPICS + 1 announces, assert only MAX_TOPICS subscriptions created.
- Add `test_topic_announce_listener_dynamic_subscription`: send announce for new topic, assert relay subscribes and begins forwarding.
- Add `test_gossip_bridge`: connect two peers on different transport types (TCP, WebSocket) via MemNetwork relay, send message from one, assert receipt on other.
- Add relay startup/shutdown test.

### willow-agent (`crates/agent/src/`)
**Test files:** test module and e2e.rs (52 tests)

**Strengths:** Scope enforcement security model, tool dispatch round-trips, scope integration tests.

**Actions:**
- Add `test_notification_bridge_forwards_client_event`: subscribe a mock MCP peer to the notification bridge, emit a ClientEvent from a test client, assert the MCP peer receives a notification.
- Add `test_call_tool_scope_rejection`: directly call `call_tool` with a tool that the agent's scope does not permit, assert rejection (distinct from `list_tools` scope filtering).
- Fix or delete both `rename_server` tests. If renaming is implemented: assert the server name in state equals the new name. If not: remove the tests.
- Fix or delete `kick_member_removes_from_server`: kick a different peer (not self), then assert that peer is absent from the member list.
- Fix the multi-peer e2e section: connect the two `ClientHandle` instances through a shared `MemNetwork`, then assert cross-peer message delivery.
- Add error-path tests for `call_tool`: invalid JSON args, missing required fields, unknown tool name.
- Add tests for the 11 untested resource URIs.

### willow-worker (`crates/worker/src/`)
**Test files:** test module (36 tests)

**Strengths:** Real actor system integration tests, real MemNetwork gossip (heartbeat, departure, event forwarding), NetworkActor pre-buffer race condition regression, `parse` function exhaustive coverage.

**Actions:**
- Add `test_sync_actor_broadcasts_heads`: after applying N events, trigger sync and assert a sync request containing correct heads is broadcast on the network topic.
- Add `test_two_worker_convergence`: create two workers sharing a MemNetwork, send a message from worker A, assert worker B's state contains that message. This is the most important missing test in this crate.
- Fix `network_actor_drains_immediately_without_ready_signal`: if this test depends on timing, rewrite it using a deterministic signal (e.g., a oneshot channel that fires when the drain completes) rather than a fixed sleep.

### willow-web (`crates/web/`)
**Test files:** `tests/browser.rs` (97 tests)

**Strengths:** URL extraction, image detection, timestamp formatting utilities, Issue #81 signal reactivity regression pin.

**Actions:**
- Add at least one test per real component that mounts the actual component using `mount_test(|| view! { <ComponentName ... /> })` and asserts on its rendered output or behavior.
- Priority components: `ChatInput` (submit behavior, input state), `MessageList` (renders messages, edit/delete visible on own messages), `Sidebar` (channel selection updates active channel signal).
- Add `AppState` tests: switching channels updates the active channel signal, unread counter increments on new messages.
- Add `event_processing.rs` tests: verify that incoming ClientEvents update the correct signals.
- Add `state_bridge.rs` tests: verify that state changes propagate through the bridge to the UI signals.
- Fix tests that copy-paste production functions: import and call the original, not the copy.
- Remove `#[allow(dead_code)]` from test helpers that exist only because the implementation they were meant to test doesn't exist yet.

### willow-identity (`crates/identity/src/`)
**Test files:** test module (22 tests)

**Strengths:** Key generation, pack/unpack round-trip, tamper detection, file persistence, Unix permissions (0600 enforcement), ZeroizeOnDrop.

**Actions:**
- Add `test_impersonation_rejected`: sign a message as Alice, manually rewrite the claimed author to Bob, assert `unpack` rejects it with an attribution error.
- Add `test_truncated_key_file_returns_error`: write a key file with fewer bytes than a valid key, assert `load` returns an error (not a panic).

### willow-crypto (`crates/crypto/src/`)
**Test files:** test module (46 tests)

**No changes needed.** This is the project's strongest test suite. Maintain it as the reference for how security-critical crates should be tested.

### willow-transport (`crates/transport/src/`)
**Test files:** test module (11 tests)

**Strengths:** Pack/unpack round-trips, 256KB size limit enforcement, MessageType discriminant pinning (wire stability), version mismatch rejection.

**Actions:**
- Add `test_valid_outer_invalid_inner`: pack a valid outer envelope containing a syntactically invalid inner payload, assert `unpack` returns the correct error variant.
- Add `test_version_zero_rejected`: craft a byte buffer with version field = 0 (old/reserved), assert rejection.
- Add `test_size_limit_exact_boundary`: construct a payload of exactly 256KB, assert it is accepted. Then 256KB + 1 byte, assert rejection.

### willow-actor (`crates/actor/src/`)
**No changes needed.** Complete primitive coverage with 92 behavioral tests and 15 performance benchmarks. This is the other reference implementation alongside willow-crypto.

### Playwright E2E (`e2e/`)
**Test files:** `multi-peer-sync.spec.ts`, `permissions.spec.ts`, `mobile.spec.ts`, and others (68 tests total)

**Strengths:** `multi-peer-sync.spec.ts` is the project's best integration test file — 12 genuine two-peer tests with real network assertions: edit sync, delete sync, reaction sync, typing indicator propagation, display names across peers, pre-join history delivery.

**Actions:**
- Migrate the ~35 single-peer UI tests that don't require a relay or second peer into `crates/web/tests/browser.rs` as Leptos browser tests. E2E test time and flakiness will drop significantly.
- Replace all `waitForTimeout(300–5000)` calls with event-driven waits: `page.waitForSelector`, `page.waitForFunction`, or custom polling on specific DOM state.
- Fix negative assertions: instead of `await page.waitForTimeout(5000); expect(...).not.toBeVisible()`, use `expect(...).not.toBeVisible({ timeout: 1000 })` or a shorter definitive check.
- Set `workers` to at least 2 (or `fullyParallel: true`) so tests run in parallel.
- Add `storageState` reset or IndexedDB clearing between tests to prevent state bleed.
- Add missing critical E2E scenarios:
  - Relay history recovery: peer A sends messages, peer B goes offline, peer A sends more, peer B reconnects, assert all messages visible.
  - 3-peer convergence: send from A, assert visible on B and C within timeout.
  - Kicked peer perspective: kicked peer's UI reflects kick event.
  - Relay-only path: disable QUIC direct connection, assert peers still sync through relay.
  - HLC ordering: two peers send messages concurrently, assert consistent ordering on both sides.

---

## What's Working

**willow-crypto** and **willow-actor** demonstrate what good crate-level testing looks like. willow-crypto covers every AEAD failure mode, wrong-key decryption, nonce uniqueness, forward secrecy ratchet state transitions, DoS bounds (Issue #110 regression guard), X25519 key exchange, RatchetCache eviction, memory zeroization, and epoch isolation. Every security property is named, isolated, and directly asserted. willow-actor achieves the same completeness for concurrency primitives: supervisor restart, broker pub/sub, FSM transitions, pool load balancing, debounce and throttle semantics, derived state, CoW, backpressure, and shutdown hooks are all tested independently.

**`multi-peer-sync.spec.ts`** is the correct template for E2E tests. It uses real two-peer network setup, makes concrete assertions on message content (not just existence), tests both sides of the sync, and covers non-obvious edge cases (pre-join history, display name resolution across peers). The remaining E2E test files should be brought up to this standard.

**willow-state permission enforcement** for the stranger-vs-member boundary, governance rules, and equivocation detection is solid. The Issue #109 regression guard is a model for how security bugs should be pinned in tests. The OOO delivery and topo-sort tests in willow-state and willow-replay show good instinct for testing distributed system edge cases.

**willow-worker's real actor integration tests** — particularly the NetworkActor pre-buffer race condition regression — demonstrate the right approach for actor-based systems: use real MemNetwork, real actors, and real message flows rather than mocking the actor system.

---

## Recommended Improvement Order

### Phase 1 — Remove False Confidence (1–2 days)

These changes reduce the misleading signal from tests that cannot fail. Do them first so the test suite accurately reflects actual coverage.

1. Delete or rewrite `node_disconnect_detected` in willow-network.
2. Delete or rewrite both `rename_server` tests in willow-agent.
3. Delete or rewrite `kick_member_removes_from_server` in willow-agent.
4. Delete the unreachable multi-peer section in willow-agent's e2e.
5. Rewrite `sync_snapshot_fallback_when_peer_is_behind` in willow-replay to actually trigger the Snapshot branch.
6. Convert all `>= N` assertions in willow-state stress tests to `== N`.
7. Fix the serializer mismatch in willow-common and willow-messaging: replace `bincode::serialize` with `willow_transport::pack`.

### Phase 2 — Security Gaps (3–5 days)

These are the highest-risk missing tests. Each protects a distinct security property.

1. Member-vs-member message edit/delete (willow-state).
2. `accept: false` vote path (willow-state).
3. `mutations.rs` permission checks for each mutation type (willow-client) — use `test_client()`.
4. `listeners.rs` tamper detection (willow-client) — activate `test_client_on_hub`.
5. Wrong-signer attribution (willow-common and willow-identity).
6. Impersonation rejection (willow-identity).

### Phase 3 — Core Behavioral Coverage (1–2 weeks)

These cover the system's primary behavioral surface. Prioritize by blast radius.

1. `derive_client_events` and `views.rs` in willow-client.
2. Message mutations (edit, delete, react, pin, unpin) in willow-client.
3. `topic_announce_listener` and gossip bridging in willow-relay.
4. Two-worker convergence in willow-worker.
5. `sync_since` direct coverage in willow-storage.
6. Persist→drop→reopen durability in willow-storage.
7. `on_event` routing cases in willow-replay.
8. `notification_bridge` in willow-agent.
9. Remaining WireMessage serde tests in willow-common.

### Phase 4 — Test Infrastructure (ongoing)

These are structural improvements that make future tests easier and more reliable.

1. Make IrohNetwork constructable in tests (injectable address lookup).
2. Make HLC wall clock injectable in willow-messaging.
3. Create a real AppState mock context for willow-web tests.
4. Consolidate `test_dag()` helper in willow-state into a single shared module.
5. Migrate ~35 single-peer Playwright tests into Leptos browser tests.
6. Replace `waitForTimeout` in all Playwright tests with event-driven waits.
7. Add real component mounts to willow-web browser tests (one per component).
8. Replace `tokio::time::sleep()` synchronization in willow-client tests with channel-based signaling.

### Phase 5 — Edge Cases and Completeness (ongoing)

1. HLC clock regression and counter overflow (willow-messaging).
2. SyncActor broadcast verification (willow-worker).
3. LRU eviction ordering (willow-replay).
4. Transport boundary-exact size and version-zero rejection (willow-transport).
5. Relay history recovery E2E test.
6. 3-peer convergence E2E test.
7. Kicked peer perspective E2E test.
8. HLC concurrent message ordering E2E test.

---

*This report was generated from 14 independent crate audits on 2026-04-13. Total test count: ~649 across all crates plus 68 Playwright E2E tests.*
